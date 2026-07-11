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

async fn handle_hook(
    State(runtime): State<Arc<ObserverRuntime>>,
    Query(query): Query<HookQuery>,
    body: Bytes,
) -> impl IntoResponse {
    if query.agent.as_deref() == Some("pi") {
        if let Some(source) = query.source.as_deref() {
            runtime.ingest_pi_source(&query.session, source, &body);
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
}

impl ObserverServerState {
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
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].session_id, "s1");
        assert_eq!(activities[0].agent_id, "a1");
        assert_eq!(activities[0].kind, ActivityKind::Prompt);
        assert_eq!(activities[0].text.as_deref(), Some("marker"));

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
        assert_eq!(activities.len(), 2);
        assert_eq!(activities[0].kind, ActivityKind::Prompt);
        assert_eq!(activities[0].text.as_deref(), Some("pi task"));
        assert_eq!(activities[1].kind, ActivityKind::Tool);

        let notifications = events.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].source, NotificationSource::Stop);
        assert_eq!(notifications[0].message, "Pi finished a task");
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
}
