use std::future::Future;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::{ObserverProvider, ObserverRuntime, RawObserverHook};

#[derive(Deserialize)]
struct HookQuery {
    session: String,
    provider: Option<String>,
    event: Option<String>,
    source: Option<String>,
    agent: Option<String>,
}

fn ok_response() -> impl IntoResponse {
    (
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        r#"{"ok":true}"#,
    )
}

/// agy() 셸 래퍼가 호출 시점의 `$PWD`를 실어 보내는 헤더(session/agy_hook.rs
/// 참고). 쿼리 문자열이 아니라 헤더인 이유: POSIX sh에는 표준
/// percent-encoding 도구가 없어 쿼리에 실으면 공백·비ASCII 경로가 깨진다.
const AGY_CWD_HEADER: &str = "x-agent-office-cwd";

async fn handle_hook(
    State(runtime): State<Arc<ObserverRuntime>>,
    Query(query): Query<HookQuery>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if query.agent.as_deref() == Some("agy") {
        if let Some(source) = query.source.as_deref() {
            // `HeaderValue::to_str()`은 ASCII 시각 문자만 허용해 비ASCII(한글 등)
            // 경로가 통째로 None이 된다. 헤더 값은 우리가 만든 클라이언트(hook.sh)가
            // curl -H로 그대로 실은 raw UTF-8 바이트이므로 from_utf8로 직접 읽는다.
            let cwd_override = headers
                .get(AGY_CWD_HEADER)
                .and_then(|v| std::str::from_utf8(v.as_bytes()).ok())
                .filter(|v| !v.trim().is_empty());
            runtime.ingest_agy_source(&query.session, source, &body, cwd_override);
        }
        return ok_response();
    }
    if query.agent.as_deref() == Some("pi") {
        if let Some(source) = query.source.as_deref() {
            runtime.ingest_pi_source(&query.session, source, &body);
        }
        return ok_response();
    }
    if query.agent.as_deref() == Some("kilo") {
        if let Some(source) = query.source.as_deref() {
            runtime.ingest_kilo_source(&query.session, source, &body);
        }
        return ok_response();
    }
    let Some(provider) = query.provider.as_deref().and_then(ObserverProvider::parse) else {
        return ok_response();
    };
    let body_event = || {
        serde_json::from_slice::<serde_json::Value>(&body)
            .ok()?
            .get("hook_event_name")?
            .as_str()
            .map(str::to_owned)
    };
    let Some(event_name) = query.event.or_else(body_event) else {
        return ok_response();
    };
    runtime.ingest(
        provider,
        &query.session,
        RawObserverHook {
            event_name: &event_name,
            body: &body,
        },
    );
    ok_response()
}

pub async fn serve(
    runtime: Arc<ObserverRuntime>,
    shutdown_rx: oneshot::Receiver<()>,
) -> std::io::Result<(u16, JoinHandle<()>)> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let app = Router::new()
        .route("/hook", post(handle_hook))
        .with_state(runtime);
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    Ok((port, handle))
}

type StartedServer = (u16, oneshot::Sender<()>, JoinHandle<()>);

struct InstalledServer {
    port: u16,
    shutdown: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

impl InstalledServer {
    fn from_started((port, shutdown, handle): StartedServer) -> Self {
        Self {
            port,
            shutdown,
            handle,
        }
    }

    fn shutdown(self) {
        let _ = self.shutdown.send(());
        let _detached = self.handle;
    }
}

#[derive(Default)]
struct ServerLifecycle {
    installed: Option<InstalledServer>,
    shutdown_requested: bool,
    generation: u64,
}

pub async fn serve_with_retry<F, Fut>(mut attempt: F) -> std::io::Result<StartedServer>
where
    F: FnMut(oneshot::Receiver<()>) -> Fut,
    Fut: Future<Output = std::io::Result<(u16, JoinHandle<()>)>>,
{
    let (shutdown, shutdown_rx) = oneshot::channel();
    match attempt(shutdown_rx).await {
        Ok((port, handle)) => Ok((port, shutdown, handle)),
        Err(_) => {
            let (retry_shutdown, retry_rx) = oneshot::channel();
            let (port, handle) = attempt(retry_rx).await?;
            Ok((port, retry_shutdown, handle))
        }
    }
}

#[derive(Default)]
pub struct ObserverServerState {
    start_gate: tokio::sync::Mutex<()>,
    lifecycle: Mutex<ServerLifecycle>,
    /// §핵심 5(docs/session-handoff-design.md): 세션 env의 AGENT_OFFICE_HOOK_URL은
    /// 스폰 시점 포트를 담는데, 재시작 후 입양된 세션은 옛 포트를 가리킨다.
    /// `<app_data_dir>/observer-port`에 현재 포트를 기록해 forwarder가 1회
    /// 재시도할 근거를 남긴다. `Default`(테스트 다수가 이 경로로 만든다)로는
    /// None -- 포트 파일을 쓰지 않는 무해한 동작.
    app_data_dir: Mutex<Option<std::path::PathBuf>>,
}

impl ObserverServerState {
    /// 앱 데이터 디렉터리를 지정한다. `lib.rs`의 프로덕션 부트스트랩만
    /// 호출 -- 테스트는 기본 None으로 포트 파일 기록을 건드리지 않는다.
    pub fn set_app_data_dir(&self, dir: std::path::PathBuf) {
        *self.app_data_dir.lock().unwrap() = Some(dir);
    }

    fn write_port_file(&self, port: u16) {
        let Some(dir) = self.app_data_dir.lock().unwrap().clone() else {
            return;
        };
        if let Err(error) = std::fs::create_dir_all(&dir) {
            eprintln!("observer-port: failed to create {}: {error}", dir.display());
            return;
        }
        if let Err(error) = std::fs::write(dir.join("observer-port"), port.to_string()) {
            eprintln!("observer-port: failed to write in {}: {error}", dir.display());
        }
    }

    async fn ensure_with<F, Fut>(&self, start: F) -> Option<u16>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = std::io::Result<StartedServer>>,
    {
        let _start = self.start_gate.lock().await;
        let generation = {
            let lifecycle = self.lifecycle.lock().unwrap();
            if lifecycle.shutdown_requested {
                return None;
            }
            if let Some(installed) = &lifecycle.installed {
                return Some(installed.port);
            }
            lifecycle.generation
        };

        let start_result = start().await;
        let mut late_server = None;
        let mut start_error = None;
        let port = {
            let mut lifecycle = self.lifecycle.lock().unwrap();
            if lifecycle.shutdown_requested || lifecycle.generation != generation {
                if let Ok(started) = start_result {
                    late_server = Some(InstalledServer::from_started(started));
                }
                None
            } else {
                match start_result {
                    Ok(started) => {
                        let installed = InstalledServer::from_started(started);
                        let port = installed.port;
                        lifecycle.installed = Some(installed);
                        Some(port)
                    }
                    Err(error) => {
                        start_error = Some(error);
                        None
                    }
                }
            }
        };
        if let Some(server) = late_server {
            server.shutdown();
        }
        if let Some(error) = start_error {
            eprintln!("observer server unavailable: {error}");
        }
        if let Some(port) = port {
            self.write_port_file(port);
        }
        port
    }

    pub async fn ensure(&self, runtime: Arc<ObserverRuntime>) -> Option<u16> {
        self.ensure_with(|| serve_with_retry(|rx| serve(runtime.clone(), rx)))
            .await
    }

    pub fn current_url(&self) -> Option<String> {
        self.lifecycle
            .lock()
            .unwrap()
            .installed
            .as_ref()
            .map(|server| format!("http://127.0.0.1:{}/hook", server.port))
    }

    pub fn shutdown(&self) {
        let installed = {
            let mut lifecycle = self.lifecycle.lock().unwrap();
            lifecycle.shutdown_requested = true;
            lifecycle.generation = lifecycle.generation.wrapping_add(1);
            lifecycle.installed.take()
        };
        if let Some(installed) = installed {
            installed.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{serve, serve_with_retry, ObserverServerState, StartedServer};
    use crate::notification::hub::{NotificationHub, SystemClock};
    use crate::observer::ObserverRuntime;
    use crate::state::fake::RecordingEvents;
    use crate::state::{AppEvents, SessionRegistry};
    use crate::types::{ActivityKind, NotificationSource, SessionState};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    fn fixture() -> (Arc<ObserverRuntime>, Arc<RecordingEvents>) {
        let registry = Arc::new(SessionRegistry::new());
        registry.insert("s1", "a1", SessionState::Running);
        let events = Arc::new(RecordingEvents::default());
        let app_events: Arc<dyn AppEvents> = events.clone();
        let hub = Arc::new(NotificationHub::new(
            registry,
            app_events,
            Arc::new(SystemClock),
            Duration::from_millis(3_000),
        ));
        let settings_dir = std::env::temp_dir().join(format!(
            "agent-office-observer-server-test-{}",
            uuid::Uuid::new_v4(),
        ));
        let runtime = Arc::new(ObserverRuntime::production(
            hub,
            settings_dir,
            std::env::current_exe().unwrap(),
        ));
        (runtime, events)
    }

    async fn started_server(port: u16) -> std::io::Result<StartedServer> {
        let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _ = shutdown_rx.await;
        });
        Ok((port, shutdown, handle))
    }

    #[tokio::test]
    async fn routes_claude_query_event_and_codex_body_event() {
        let (runtime, events) = fixture();
        let state = ObserverServerState::default();
        let port = state.ensure(runtime).await.unwrap();
        let client = reqwest::Client::new();

        client
            .post(format!(
                "http://127.0.0.1:{port}/hook?session=s1&provider=claude&event=UserPromptSubmit"
            ))
            .body(r#"{"prompt":"marker"}"#)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        client
            .post(format!(
                "http://127.0.0.1:{port}/hook?session=s1&provider=codex"
            ))
            .body(r#"{"hook_event_name":"Stop"}"#)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        let activities = events.activities();
        assert_eq!(activities.len(), 2);
        assert_eq!(activities[0].session_id, "s1");
        assert_eq!(activities[0].agent_id, "a1");
        assert_eq!(activities[0].kind, ActivityKind::Prompt);
        assert_eq!(activities[0].text.as_deref(), Some("marker"));
        assert_eq!(activities[1].kind, ActivityKind::SubCount);
        assert_eq!(activities[1].count, Some(0));

        let notifications = events.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].session_id, "s1");
        assert_eq!(notifications[0].agent_id, "a1");
        assert_eq!(notifications[0].source, NotificationSource::Stop);
        assert_eq!(notifications[0].message, "작업이 완료되었습니다.");

        assert_eq!(
            state.current_url().as_deref(),
            Some(format!("http://127.0.0.1:{port}/hook").as_str()),
        );
        state.shutdown();
    }

    /// pi v0.84.2 확장이 실제로 보내는 한 턴짜리 시퀀스를 그대로 재생한다.
    /// 페이로드는 spy 확장으로 덤프한 실측 이벤트에서 옮겨 온 것:
    ///   before_agent_start(prompt, cwd)
    ///   tool_execution_start(read {path}) / tool_execution_start(bash {command})
    ///   message_end(assistant, text 블록)
    ///   agent_settled
    /// 기대: 프롬프트 원문·도구 요약·내레이션이 라벨 파이프라인에 실리고,
    /// 완료 알림은 정확히 1건.
    #[tokio::test]
    async fn routes_pi_v084_turn_sequence_into_labelled_activities_and_one_stop() {
        let (runtime, events) = fixture();
        let state = ObserverServerState::default();
        let port = state.ensure(runtime).await.unwrap();
        let client = reqwest::Client::new();

        for (source, body) in [
            (
                "prompt",
                r#"{"prompt":"버그 고쳐줘","cwd":"/Users/me/dev/agent-office"}"#,
            ),
            (
                "tool",
                r#"{"tool_name":"read","tool_input":{"path":"/Users/me/dev/agent-office/src/main.rs"}}"#,
            ),
            (
                "tool",
                r#"{"tool_name":"bash","tool_input":{"command":"echo done"}}"#,
            ),
            ("tool", r#"{"assistant":"원인을 좁히는 중"}"#),
            ("stop", r#"{"message":"Pi finished a task"}"#),
        ] {
            client
                .post(format!(
                    "http://127.0.0.1:{port}/hook?session=s1&source={source}&agent=pi"
                ))
                .body(body)
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
        }

        let activities = events.activities();
        // prompt + tool×3 + Stop이 부수적으로 내는 SubCount 1건.
        assert_eq!(activities.len(), 5);
        assert_eq!(activities[0].kind, ActivityKind::Prompt);
        assert_eq!(activities[0].text.as_deref(), Some("버그 고쳐줘"));
        assert_eq!(activities[0].cwd.as_deref(), Some("/Users/me/dev/agent-office"));
        assert_eq!(activities[1].kind, ActivityKind::Tool);
        assert_eq!(activities[1].text.as_deref(), Some("read: main.rs"));
        assert_eq!(activities[2].text.as_deref(), Some("bash: echo done"));
        assert_eq!(activities[3].assistant_text.as_deref(), Some("원인을 좁히는 중"));
        assert_eq!(activities[3].text, None);
        assert_eq!(activities[4].kind, ActivityKind::SubCount);

        let notifications = events.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].source, NotificationSource::Stop);
        assert_eq!(notifications[0].message, "Pi finished a task");
        state.shutdown();
    }

    #[tokio::test]
    async fn routes_existing_pi_source_query_contract() {
        let (runtime, events) = fixture();
        let state = ObserverServerState::default();
        let port = state.ensure(runtime).await.unwrap();
        let client = reqwest::Client::new();

        for (source, body) in [
            ("prompt", r#"{"prompt":"pi task"}"#),
            ("tool", "{}"),
            ("stop", r#"{"message":"Pi finished a task"}"#),
        ] {
            client
                .post(format!(
                    "http://127.0.0.1:{port}/hook?session=s1&source={source}&agent=pi"
                ))
                .body(body)
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
        }

        let activities = events.activities();
        assert_eq!(activities.len(), 3);
        assert_eq!(activities[0].kind, ActivityKind::Prompt);
        assert_eq!(activities[0].text.as_deref(), Some("pi task"));
        assert_eq!(activities[1].kind, ActivityKind::Tool);
        assert_eq!(activities[2].kind, ActivityKind::SubCount);
        assert_eq!(activities[2].count, Some(0));

        let notifications = events.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].source, NotificationSource::Stop);
        assert_eq!(notifications[0].message, "Pi finished a task");
        state.shutdown();
    }

    /// Kilo 플러그인 훅 라우팅: `agent=kilo`는 pi와 같이 `provider=`/`event=`
    /// 계약을 타지 않는 갈래를 쓰지만, 소스 종류가 더 많다(sub-start/sub-stop/
    /// hook까지). 스파이크 실측(2026-09-10) 페이로드로 한 턴을 재생해 각 source가
    /// 기대한 ActivityKind/알림으로 매핑되는지 확인한다.
    #[tokio::test]
    async fn routes_all_six_kilo_sources_into_labelled_activities_and_notifications() {
        let (runtime, events) = fixture();
        let state = ObserverServerState::default();
        let port = state.ensure(runtime).await.unwrap();
        let client = reqwest::Client::new();

        for (source, body) in [
            (
                "prompt",
                r#"{"prompt":"버그 고쳐줘","cwd":"/Users/me/dev/agent-office"}"#,
            ),
            (
                "tool",
                r#"{"tool_name":"bash","tool_input":{"command":"echo done"}}"#,
            ),
            ("hook", r#"{"message":"Kilo needs permission: bash echo x"}"#),
            ("sub-start", "{}"),
            ("sub-stop", "{}"),
            ("stop", r#"{"message":"Kilo finished a task","running":0}"#),
        ] {
            client
                .post(format!(
                    "http://127.0.0.1:{port}/hook?session=s1&source={source}&agent=kilo"
                ))
                .body(body)
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
        }

        let activities = events.activities();
        // prompt + tool + sub-start + sub-stop + Stop이 부수적으로 내는 SubCount.
        assert_eq!(activities.len(), 5);
        assert_eq!(activities[0].kind, ActivityKind::Prompt);
        assert_eq!(activities[0].text.as_deref(), Some("버그 고쳐줘"));
        assert_eq!(activities[0].cwd.as_deref(), Some("/Users/me/dev/agent-office"));
        assert_eq!(activities[1].kind, ActivityKind::Tool);
        assert_eq!(activities[1].text.as_deref(), Some("bash: echo done"));
        assert_eq!(activities[2].kind, ActivityKind::SubStart);
        assert_eq!(activities[3].kind, ActivityKind::SubStop);
        assert_eq!(activities[4].kind, ActivityKind::SubCount);
        assert_eq!(activities[4].count, Some(0));

        let notifications = events.notifications();
        // hook(권한 알림) 1건 + stop(완료) 1건.
        assert_eq!(notifications.len(), 2);
        assert_eq!(notifications[0].source, NotificationSource::Hook);
        assert_eq!(notifications[0].message, "Kilo needs permission: bash echo x");
        assert_eq!(notifications[1].source, NotificationSource::Stop);
        assert_eq!(notifications[1].message, "Kilo finished a task");
        state.shutdown();
    }

    /// 백그라운드 서브에이전트가 아직 도는 중(`running>0`)의 Stop은 턴 경계일 뿐
    /// 완료가 아니다 — pi/agy와 같은 원칙(이슈 #27)을 kilo도 지켜야 한다.
    #[tokio::test]
    async fn routes_kilo_stop_with_running_children_suppresses_the_completion_notification() {
        let (runtime, events) = fixture();
        let state = ObserverServerState::default();
        let port = state.ensure(runtime).await.unwrap();
        let client = reqwest::Client::new();

        client
            .post(format!(
                "http://127.0.0.1:{port}/hook?session=s1&source=stop&agent=kilo"
            ))
            .body(r#"{"message":"Kilo finished a task","running":1}"#)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        let activities = events.activities();
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].kind, ActivityKind::SubCount);
        assert_eq!(activities[0].count, Some(1));
        assert!(events.notifications().is_empty(), "running>0 must not notify");
        state.shutdown();
    }

    /// agy(Antigravity CLI) 훅 라우팅: `agent=agy`는 `provider=`/`event=`
    /// 계약을 타지 않는 pi와 같은 갈래를 쓴다(§2/§3.4). `workspacePaths`가
    /// 빈 배열인 프롬프트가 `X-Agent-Office-Cwd` 헤더로 강등되는지, 반복
    /// invocationNum(1)이 버려지는지도 함께 확인한다(§4/스파이크 실측 1).
    #[tokio::test]
    async fn routes_agy_turn_sequence_via_agent_query_and_cwd_header() {
        let (runtime, events) = fixture();
        let state = ObserverServerState::default();
        let port = state.ensure(runtime).await.unwrap();
        let client = reqwest::Client::new();

        // 같은 턴 안의 반복 PreInvocation(invocationNum=1)은 버려진다.
        client
            .post(format!("http://127.0.0.1:{port}/hook?session=s1&agent=agy&source=prompt"))
            .body(r#"{"invocationNum":1,"workspacePaths":["/w"]}"#)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        // 턴 시작(invocationNum=0), workspacePaths가 비어 있어 헤더의 cwd로 강등.
        client
            .post(format!("http://127.0.0.1:{port}/hook?session=s1&agent=agy&source=prompt"))
            .header("X-Agent-Office-Cwd", "/Users/me/dev/agent-office")
            .body(r#"{"invocationNum":0,"workspacePaths":[]}"#)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        client
            .post(format!("http://127.0.0.1:{port}/hook?session=s1&agent=agy&source=tool"))
            .body(r#"{"name":"run_terminal_cmd","command":"echo done"}"#)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        client
            .post(format!("http://127.0.0.1:{port}/hook?session=s1&agent=agy&source=stop"))
            .body(r#"{"terminationReason":"model_stop"}"#)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        let activities = events.activities();
        // 반복 PreInvocation은 버려졌으므로 prompt(1) + tool(1) + Stop의
        // SubCount(1)만 남는다.
        assert_eq!(activities.len(), 3);
        assert_eq!(activities[0].kind, ActivityKind::Prompt);
        assert_eq!(activities[0].cwd.as_deref(), Some("/Users/me/dev/agent-office"));
        assert_eq!(activities[1].kind, ActivityKind::Tool);
        assert_eq!(activities[1].text.as_deref(), Some("run_terminal_cmd: echo done"));
        assert_eq!(activities[2].kind, ActivityKind::SubCount);

        let notifications = events.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].source, NotificationSource::Stop);
        assert_eq!(notifications[0].message, "Antigravity finished a task");
        state.shutdown();
    }

    /// 리뷰 지적: `HeaderValue::to_str()`은 ASCII 시각 문자만 허용해 한글 등
    /// 비ASCII 경로가 통째로 None이 됐다(`headers::HeaderValue::to_str` 계약).
    /// 헤더 값을 raw UTF-8 바이트로 만들어(`from_bytes`, `to_str`가 거부하는
    /// 값) 서버가 여전히 cwd로 읽는지 확인한다.
    #[tokio::test]
    async fn routes_agy_prompt_with_a_non_ascii_cwd_header() {
        let (runtime, events) = fixture();
        let state = ObserverServerState::default();
        let port = state.ensure(runtime).await.unwrap();
        let client = reqwest::Client::new();

        let cwd = "/Users/개발자/dev/agent-office";
        let header = axum::http::HeaderValue::from_bytes(cwd.as_bytes())
            .expect("raw UTF-8 header bytes must be constructible");
        // to_str()이라면 여기서 Err가 나야 회귀를 재현한 것이다.
        assert!(header.to_str().is_err(), "non-ASCII header must fail to_str()");

        client
            .post(format!("http://127.0.0.1:{port}/hook?session=s1&agent=agy&source=prompt"))
            .header("X-Agent-Office-Cwd", header)
            .body(r#"{"invocationNum":0,"workspacePaths":[]}"#)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        let activities = events.activities();
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].kind, ActivityKind::Prompt);
        assert_eq!(activities[0].cwd.as_deref(), Some(cwd));
        state.shutdown();
    }

    #[tokio::test]
    async fn routes_subagent_lifecycle_for_both_providers() {
        let (runtime, events) = fixture();
        let state = ObserverServerState::default();
        let port = state.ensure(runtime).await.unwrap();
        let client = reqwest::Client::new();

        for event in ["SubagentStart", "SubagentStop"] {
            client
                .post(format!(
                    "http://127.0.0.1:{port}/hook?session=s1&provider=claude&event={event}"
                ))
                .body("{}")
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
        }

        for event in ["SubagentStart", "SubagentStop"] {
            client
                .post(format!(
                    "http://127.0.0.1:{port}/hook?session=s1&provider=codex"
                ))
                .body(format!(r#"{{"hook_event_name":"{event}"}}"#))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
        }

        let activities = events.activities();
        assert_eq!(
            activities
                .iter()
                .map(|activity| activity.kind)
                .collect::<Vec<_>>(),
            vec![
                ActivityKind::SubStart,
                ActivityKind::SubStop,
                ActivityKind::SubStart,
                ActivityKind::SubStop,
            ],
        );
        assert!(events.notifications().is_empty());
        state.shutdown();
    }

    #[tokio::test]
    async fn unknown_and_malformed_hooks_are_200_noops() {
        let (runtime, events) = fixture();
        let state = ObserverServerState::default();
        let port = state.ensure(runtime).await.unwrap();
        let client = reqwest::Client::new();

        for (url, body) in [
            (
                format!("http://127.0.0.1:{port}/hook?session=s1&provider=unknown&event=Stop"),
                r#"{"message":"ignored"}"#,
            ),
            (
                format!("http://127.0.0.1:{port}/hook?session=s1&provider=claude&event=Unknown"),
                r#"{"message":"ignored"}"#,
            ),
            (
                format!("http://127.0.0.1:{port}/hook?session=unknown-session&provider=codex"),
                r#"{"hook_event_name":"Stop"}"#,
            ),
        ] {
            let response = client.post(url).body(body).send().await.unwrap();
            assert!(response.status().is_success());
        }

        let before = (events.activities().len(), events.notifications().len());
        let response = client
            .post(format!(
                "http://127.0.0.1:{port}/hook?session=s1&provider=codex"
            ))
            .body("not-json")
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        assert_eq!(
            (events.activities().len(), events.notifications().len()),
            before,
        );
        assert_eq!(before, (0, 0));

        state.shutdown();
    }

    #[tokio::test]
    async fn bind_failure_is_retried_once_then_propagated() {
        let attempts = AtomicUsize::new(0);
        let result = serve_with_retry(|shutdown_rx| {
            attempts.fetch_add(1, Ordering::SeqCst);
            drop(shutdown_rx);
            async {
                Err::<(u16, tokio::task::JoinHandle<()>), _>(std::io::Error::other(
                    "injected bind failure",
                ))
            }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn concurrent_ensure_calls_install_only_one_server() {
        let state = ObserverServerState::default();
        let attempts = AtomicUsize::new(0);

        let first = state.ensure_with(|| async {
            attempts.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(25)).await;
            started_server(41001).await
        });
        let second = state.ensure_with(|| async {
            attempts.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(25)).await;
            started_server(41002).await
        });
        let (first_port, second_port) = tokio::join!(first, second);

        assert_eq!(first_port, second_port);
        assert!(matches!(first_port, Some(41001 | 41002)));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        state.shutdown();
    }

    #[tokio::test]
    async fn failed_ensure_is_fail_open_and_can_be_retried() {
        let state = ObserverServerState::default();

        let failed = state
            .ensure_with(|| async { Err(std::io::Error::other("injected start failure")) })
            .await;
        assert_eq!(failed, None);
        assert_eq!(state.current_url(), None);

        let recovered = state
            .ensure_with(|| async { started_server(42001).await })
            .await;
        assert_eq!(recovered, Some(42001));
        state.shutdown();
    }

    #[tokio::test]
    async fn failed_server_start_returns_none_and_can_be_retried() {
        let state = ObserverServerState::default();
        let first = state
            .ensure_with(|| async { Err(std::io::Error::other("injected bind failure")) })
            .await;
        assert_eq!(first, None);
        assert_eq!(state.current_url(), None);

        let registry = Arc::new(SessionRegistry::new());
        let events: Arc<dyn AppEvents> = Arc::new(RecordingEvents::default());
        let hub = Arc::new(NotificationHub::new(
            registry,
            events,
            Arc::new(SystemClock),
            Duration::from_millis(3_000),
        ));
        let runtime = Arc::new(ObserverRuntime::new(
            hub,
            Vec::<Arc<dyn crate::observer::ObserverAdapter>>::new(),
        ));
        let second = state
            .ensure_with(|| serve_with_retry(|rx| serve(runtime.clone(), rx)))
            .await;
        assert!(second.is_some());
        state.shutdown();
    }

    #[tokio::test]
    async fn shutdown_clears_url_and_signals_the_installed_server() {
        let state = ObserverServerState::default();
        let signalled = Arc::new(AtomicBool::new(false));
        let task_flag = signalled.clone();

        let port = state
            .ensure_with(|| async move {
                let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
                let handle = tokio::spawn(async move {
                    if shutdown_rx.await.is_ok() {
                        task_flag.store(true, Ordering::SeqCst);
                    }
                });
                Ok((43001, shutdown, handle))
            })
            .await;
        assert_eq!(port, Some(43001));

        state.shutdown();
        assert_eq!(state.current_url(), None);
        tokio::time::timeout(Duration::from_secs(1), async {
            while !signalled.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("shutdown sender did not signal the installed server");
    }

    #[tokio::test]
    async fn shutdown_during_start_rejects_and_stops_the_late_server() {
        let state = Arc::new(ObserverServerState::default());
        let ensure_state = state.clone();
        let (start_entered, start_entered_rx) = tokio::sync::oneshot::channel();
        let (release_start, release_start_rx) = tokio::sync::oneshot::channel();
        let (shutdown_observed, shutdown_observed_rx) = tokio::sync::oneshot::channel();

        let ensure = tokio::spawn(async move {
            ensure_state
                .ensure_with(|| async move {
                    let _ = start_entered.send(());
                    let _ = release_start_rx.await;
                    let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
                    let handle = tokio::spawn(async move {
                        let _ = shutdown_observed.send(shutdown_rx.await.is_ok());
                    });
                    Ok((44001, shutdown, handle))
                })
                .await
        });

        start_entered_rx.await.unwrap();
        state.shutdown();
        release_start.send(()).unwrap();

        assert_eq!(ensure.await.unwrap(), None);
        assert_eq!(state.current_url(), None);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), shutdown_observed_rx)
                .await
                .expect("late server did not receive shutdown within timeout")
                .unwrap(),
            "late server sender was dropped instead of explicitly signalled"
        );
    }

    #[tokio::test]
    async fn ensure_after_shutdown_is_terminal_and_does_not_start() {
        let state = ObserverServerState::default();
        let attempts = AtomicUsize::new(0);
        state.shutdown();

        let result = state
            .ensure_with(|| {
                attempts.fetch_add(1, Ordering::SeqCst);
                async { started_server(45001).await }
            })
            .await;
        let current_url = state.current_url();
        state.shutdown();

        assert_eq!(result, None);
        assert_eq!(attempts.load(Ordering::SeqCst), 0);
        assert_eq!(current_url, None);
    }

    // ---- §핵심 5: observer-port 파일(docs/session-handoff-design.md) ----

    #[tokio::test]
    async fn ensure_writes_the_observer_port_file_when_app_data_dir_is_set() {
        let dir = std::env::temp_dir().join(format!(
            "agent-office-observer-port-test-{}",
            uuid::Uuid::new_v4(),
        ));
        let state = ObserverServerState::default();
        state.set_app_data_dir(dir.clone());

        let port = state
            .ensure_with(|| async { started_server(51001).await })
            .await
            .unwrap();
        assert_eq!(port, 51001);

        let written = std::fs::read_to_string(dir.join("observer-port")).unwrap();
        assert_eq!(written.trim(), "51001");

        state.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ensure_without_app_data_dir_does_not_write_a_port_file_anywhere_reachable() {
        // 기본값(Default) -- 대부분의 다른 테스트가 쓰는 경로. app_data_dir을
        // 세팅하지 않으면 포트 파일을 쓰려는 시도 자체가 없어야 한다(쓸 곳이
        // 없으므로 write_port_file은 조용히 no-op).
        let state = ObserverServerState::default();
        let port = state
            .ensure_with(|| async { started_server(51002).await })
            .await
            .unwrap();
        assert_eq!(port, 51002);
        state.shutdown();
        // 관찰 가능한 유일한 계약: app_data_dir이 None이면 어떤 파일도 만들지
        // 않는다는 것뿐이라(경로 자체가 없음) 패닉 없이 끝나면 충분하다.
    }
}
