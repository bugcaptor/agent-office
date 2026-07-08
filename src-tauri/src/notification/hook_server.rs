// src-tauri/src/notification/hook_server.rs
//
// Local axum HTTP server that receives Claude Code hook POSTs (written by
// `HookSettingsWriter`) and forwards them into `NotificationHub::ingest_hook`.
// axum was chosen over a sync minimal-dependency server so it can reuse the
// tokio runtime Tauri already loads and hook into
// `RunEvent::ExitRequested` via `axum::serve(...).with_graceful_shutdown(rx)`.
// This module only owns request parsing/routing and its own graceful
// shutdown; binding-port retry and the app-quit -> shutdown_tx wiring live
// in the bootstrap code (`lib.rs`), not here.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Router,
};
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::notification::hub::NotificationHub;
use crate::types::{ActivityKind, NotificationSource};

#[derive(Deserialize)]
struct HookQuery {
    session: String,
    #[serde(default)]
    source: String,
}

async fn handle_hook(
    State(hub): State<Arc<NotificationHub>>,
    Query(q): Query<HookQuery>,
    body: Bytes, // curl --data-binary @- 의 원본 이벤트 JSON
) -> impl IntoResponse {
    // source 라우팅:
    //  - stop  → Stop 알림(기존)
    //  - prompt/tool → activity 신호(dedup/큐 우회)
    //  - hook 또는 빈 값 → Hook 알림(기존 기본값 보존)
    //  - 그 외 → 무시 + 경고(반쪽 데이터 방지). curl `|| true`가 실패를 삼키므로
    //    응답은 여전히 200으로 둔다.
    match q.source.as_str() {
        "stop" => hub.ingest_hook(&q.session, NotificationSource::Stop, &body),
        "prompt" => hub.ingest_activity_with_body(&q.session, ActivityKind::Prompt, &body),
        "tool" => hub.ingest_activity(&q.session, ActivityKind::Tool),
        "" | "hook" => hub.ingest_hook(&q.session, NotificationSource::Hook, &body),
        other => eprintln!("hook_server: ignoring unknown hook source '{other}' (session={})", q.session),
    }
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        r#"{"ok":true}"#,
    )
}

/// 127.0.0.1 랜덤 포트에 바인딩하고 (실제 포트, 서버 태스크 핸들) 반환.
/// 포트 0 = OS 할당으로 정적 충돌 원천 차단.
pub async fn serve(
    hub: Arc<NotificationHub>,
    shutdown_rx: oneshot::Receiver<()>,
) -> std::io::Result<(u16, JoinHandle<()>)> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let app = Router::new().route("/hook", post(handle_hook)).with_state(hub);
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    Ok((port, handle))
}

// `/hook` 외 경로·메서드는 axum이 자동 404/405. Electron 설계의 라우팅 의미와 동일.

/// 바인딩 실패 시 1회 재시도 — 그리고 **살아있는 서버와 짝이 맞는
/// shutdown sender**를 함께 반환한다.
///
/// `serve`는 sender drop도 shutdown 신호로 취급하므로(내부의 `let _ =
/// shutdown_rx.await;`는 send/드롭을 구분하지 않음), 각 시도마다 반드시 새
/// oneshot 쌍을 만들고 **성공한 그 시도의 sender만** 호출자에게 넘겨야 한다.
/// 첫 시도의 sender를 재시도된 서버와 짝지어 보관하면 (a) 재시도 sender가
/// 즉시 drop되어 새 서버가 뜨자마자 내려가고 (b) 보관된 sender는 이미 소비된
/// rx를 향해 send하는 유령 핸들이 된다.
///
/// bind 시도를 클로저로 주입받아 Tauri/실소켓 없이도 재시도 배선을 단위
/// 테스트할 수 있다. 프로덕션은 `|rx| serve(hub.clone(), rx)`를 넘긴다.
pub async fn serve_with_retry<F, Fut>(
    mut attempt: F,
) -> std::io::Result<(u16, oneshot::Sender<()>, JoinHandle<()>)>
where
    F: FnMut(oneshot::Receiver<()>) -> Fut,
    Fut: std::future::Future<Output = std::io::Result<(u16, JoinHandle<()>)>>,
{
    let (tx, rx) = oneshot::channel::<()>();
    match attempt(rx).await {
        Ok((port, handle)) => Ok((port, tx, handle)),
        Err(_) => {
            // tx(첫 시도분)는 여기서 drop되지만, 첫 시도는 서버를 못 띄웠으므로
            // 신호 갈 곳이 없어 무해하다. 재시도는 자기 몫의 새 쌍을 쓴다.
            let (tx2, rx2) = oneshot::channel::<()>();
            let (port, handle) = attempt(rx2).await?;
            Ok((port, tx2, handle))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::serve;
    use crate::notification::hub::NotificationHub;
    use crate::state::fake::RecordingEvents;
    use crate::state::SessionRegistry;
    use crate::types::{NotificationSource, SessionState};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::oneshot;

    /// Standard fixture: session "s1" mapped to agent "a1", registered as
    /// Running, wired to a fresh RecordingEvents + 3s dedup window.
    fn fixture() -> (Arc<NotificationHub>, Arc<RecordingEvents>) {
        let registry = Arc::new(SessionRegistry::new());
        registry.insert("s1", "a1", SessionState::Running);
        let events = Arc::new(RecordingEvents::default());
        let hub = Arc::new(NotificationHub::new(
            registry,
            events.clone(),
            Arc::new(crate::notification::hub::SystemClock),
            Duration::from_millis(3000),
        ));
        (hub, events)
    }

    /// Polls `pred` until it's true, panicking after a generous timeout
    /// instead of hanging forever if the server/hub wiring is broken.
    async fn wait_for<F: Fn() -> bool>(pred: F) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while !pred() {
            assert!(tokio::time::Instant::now() < deadline, "condition not met within timeout");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    // ---- T-F: binds an OS-assigned port, routes/parses `/hook`, graceful shutdown ----

    #[tokio::test]
    async fn binds_port_zero_and_returns_the_actual_assigned_port() {
        let (hub, _events) = fixture();
        let (tx, rx) = oneshot::channel();
        let (port, handle) = serve(hub, rx).await.unwrap();

        assert_ne!(port, 0, "serve() must resolve the OS-assigned port, not echo back 0");

        let _ = tx.send(());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn hook_post_with_stop_source_routes_and_parses_into_the_hub() {
        let (hub, _events) = fixture();
        let (tx, rx) = oneshot::channel();
        let (port, handle) = serve(hub.clone(), rx).await.unwrap();

        let resp = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/hook?session=s1&source=stop"))
            .body(r#"{"message":"done"}"#)
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());

        wait_for(|| !hub.pending("s1").is_empty()).await;
        let pending = hub.pending("s1");
        let ev = &pending[0];
        assert!(matches!(ev.source, NotificationSource::Stop));
        assert_eq!(ev.message, "done");
        assert_eq!(ev.session_id, "s1");
        assert_eq!(ev.agent_id, "a1");

        let _ = tx.send(());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn hook_post_without_source_query_param_defaults_to_hook_source() {
        let (hub, _events) = fixture();
        let (tx, rx) = oneshot::channel();
        let (port, handle) = serve(hub.clone(), rx).await.unwrap();

        reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/hook?session=s1"))
            .body(r#"{"message":"need input"}"#)
            .send()
            .await
            .unwrap();

        wait_for(|| !hub.pending("s1").is_empty()).await;
        assert!(matches!(hub.pending("s1")[0].source, NotificationSource::Hook));

        let _ = tx.send(());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_json_body_does_not_panic_and_falls_back_to_a_default_message() {
        let (hub, _events) = fixture();
        let (tx, rx) = oneshot::channel();
        let (port, handle) = serve(hub.clone(), rx).await.unwrap();

        let resp = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/hook?session=s1&source=hook"))
            .body("not valid json {{{")
            .send()
            .await
            .unwrap();
        // The handler must not panic/500 on unparseable bodies: extract_message
        // falls back to a default message rather than erroring.
        assert!(resp.status().is_success());

        wait_for(|| !hub.pending("s1").is_empty()).await;
        assert_eq!(hub.pending("s1").len(), 1);

        let _ = tx.send(());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn hook_post_for_unknown_session_is_discarded_harmlessly() {
        // NotificationHub::ingest discards hooks for sessions the
        // registry can't resolve. The HTTP layer must still 200 (curl's
        // `|| true` masks failures anyway) and must not panic.
        let (hub, events) = fixture();
        let (tx, rx) = oneshot::channel();
        let (port, handle) = serve(hub.clone(), rx).await.unwrap();

        let resp = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/hook?session=unknown-session&source=hook"))
            .body(r#"{"message":"need input"}"#)
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());

        // Give the server a beat to process, then assert nothing landed.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(hub.pending("unknown-session").is_empty());
        assert!(events.notifications().is_empty());

        let _ = tx.send(());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn missing_session_query_param_returns_a_client_error_without_panicking() {
        let (hub, _events) = fixture();
        let (tx, rx) = oneshot::channel();
        let (port, handle) = serve(hub, rx).await.unwrap();

        let resp = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/hook"))
            .body(r#"{"message":"need input"}"#)
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_client_error());

        let _ = tx.send(());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn unknown_route_gets_axum_default_404() {
        let (hub, _events) = fixture();
        let (tx, rx) = oneshot::channel();
        let (port, handle) = serve(hub, rx).await.unwrap();

        let resp = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/nope"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);

        let _ = tx.send(());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn hook_post_with_prompt_source_emits_activity_not_notification() {
        let (hub, events) = fixture();
        let (tx, rx) = oneshot::channel();
        let (port, handle) = serve(hub.clone(), rx).await.unwrap();

        let resp = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/hook?session=s1&source=prompt"))
            .body(r#"{"prompt":"hi"}"#)
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());

        wait_for(|| !events.activities().is_empty()).await;
        let acts = events.activities();
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].session_id, "s1");
        assert_eq!(acts[0].agent_id, "a1");
        // 알림 큐/방출은 오염되지 않았다.
        assert!(hub.pending("s1").is_empty());
        assert!(events.notifications().is_empty());

        let _ = tx.send(());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn hook_post_with_tool_source_emits_activity() {
        let (hub, events) = fixture();
        let (tx, rx) = oneshot::channel();
        let (port, handle) = serve(hub.clone(), rx).await.unwrap();

        reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/hook?session=s1&source=tool"))
            .body("") // body 없어도 activity는 성립
            .send()
            .await
            .unwrap();

        wait_for(|| !events.activities().is_empty()).await;
        assert_eq!(events.activities().len(), 1);
        assert!(events.notifications().is_empty());

        let _ = tx.send(());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn hook_post_with_unknown_source_is_ignored() {
        let (hub, events) = fixture();
        let (tx, rx) = oneshot::channel();
        let (port, handle) = serve(hub.clone(), rx).await.unwrap();

        let resp = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/hook?session=s1&source=bogus"))
            .body(r#"{"message":"x"}"#)
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success()); // 여전히 200(curl || true), 패닉 금지

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(events.activities().is_empty());
        assert!(events.notifications().is_empty());
        assert!(hub.pending("s1").is_empty());

        let _ = tx.send(());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_signal_stops_the_server_task_and_frees_the_port() {
        let (hub, _events) = fixture();
        let (tx, rx) = oneshot::channel();
        let (port, handle) = serve(hub, rx).await.unwrap();

        let _ = tx.send(());
        // Graceful shutdown must actually complete the spawned task, not
        // leave it running in the background forever.
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("server task did not shut down within timeout")
            .unwrap();

        // The listener must have been released: a fresh bind to the same
        // port should succeed once the server has actually stopped.
        let relisten = tokio::net::TcpListener::bind(("127.0.0.1", port)).await;
        assert!(relisten.is_ok(), "port {port} was not released after shutdown");
    }

    // ---- serve_with_retry: the returned sender must belong to the
    //      attempt whose server is actually alive ----

    use super::serve_with_retry;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn retry_helper_first_attempt_success_returns_a_sender_wired_to_the_live_server() {
        let (hub, _events) = fixture();
        let attempts = AtomicU32::new(0);

        let (port, tx, handle) = serve_with_retry(|rx| {
            attempts.fetch_add(1, Ordering::SeqCst);
            serve(hub.clone(), rx)
        })
        .await
        .unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 1, "no retry on first-attempt success");

        // Server is alive and reachable through the returned port...
        let resp = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/hook?session=s1&source=hook"))
            .body(r#"{"message":"alive"}"#)
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());

        // ...and the *returned* sender is the one that shuts it down.
        let _ = tx.send(());
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("returned sender did not shut down the live server")
            .unwrap();
    }

    #[tokio::test]
    async fn retry_helper_second_attempt_gets_its_own_live_shutdown_sender() {
        // Regression for the reviewed bug: the original bootstrap dropped the
        // retry attempt's sender immediately (killing the freshly-started
        // server, since `serve` treats sender-drop as a shutdown signal) and
        // stored the FIRST attempt's sender (whose rx was consumed by the
        // failed first call, i.e. a dangling handle). This test fails against
        // that wiring on both counts.
        let (hub, _events) = fixture();
        let attempts = AtomicU32::new(0);

        let (port, tx, handle) = serve_with_retry(|rx| {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            let hub = hub.clone();
            async move {
                if n == 0 {
                    // Simulated bind failure; rx (and the caller's first tx's
                    // counterpart) dies with this attempt.
                    drop(rx);
                    Err(std::io::Error::other("simulated bind failure"))
                } else {
                    serve(hub, rx).await
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 2, "exactly one retry");

        // The retried server must still be alive after bootstrap returns --
        // with the buggy wiring its (dropped) sender would have gracefully
        // shut it down already, making this request fail.
        let resp = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/hook?session=s1&source=stop"))
            .body(r#"{"message":"still alive after retry"}"#)
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "retried server must be alive, not shut down by a dropped sender");
        wait_for(|| !hub.pending("s1").is_empty()).await;

        // And the sender we were handed must be the retried server's own:
        // sending through it performs the graceful shutdown.
        let _ = tx.send(());
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("returned sender is not wired to the live (retried) server")
            .unwrap();
        let relisten = tokio::net::TcpListener::bind(("127.0.0.1", port)).await;
        assert!(relisten.is_ok(), "port {port} was not released after shutdown");
    }

    #[tokio::test]
    async fn retry_helper_gives_up_after_the_single_retry() {
        let attempts = AtomicU32::new(0);

        let result = serve_with_retry(|rx| {
            attempts.fetch_add(1, Ordering::SeqCst);
            drop(rx);
            async { Err::<(u16, tokio::task::JoinHandle<()>), _>(std::io::Error::other("still failing")) }
        })
        .await;

        assert!(result.is_err(), "second failure must propagate, not loop forever");
        assert_eq!(attempts.load(Ordering::SeqCst), 2, "exactly two attempts (one retry), no more");
    }
}
