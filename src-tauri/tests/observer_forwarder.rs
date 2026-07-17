use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    body::Bytes,
    extract::{OriginalUri, Query, State},
    http::{header::LOCATION, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use serde::Deserialize;
use tokio::sync::oneshot;

#[derive(Debug)]
struct CapturedRequest {
    session: String,
    provider: String,
    query: Vec<u8>,
    body: Vec<u8>,
}

#[derive(Deserialize)]
struct CaptureQuery {
    session: String,
    provider: String,
}

type CaptureTx = Arc<Mutex<Option<oneshot::Sender<CapturedRequest>>>>;

#[derive(Clone)]
struct CaptureState {
    tx: CaptureTx,
    redirect_to: Option<String>,
}

async fn capture_handler(
    State(state): State<CaptureState>,
    OriginalUri(uri): OriginalUri,
    Query(query): Query<CaptureQuery>,
    body: Bytes,
) -> Response {
    if let Some(tx) = state.tx.lock().unwrap().take() {
        let _ = tx.send(CapturedRequest {
            session: query.session,
            provider: query.provider,
            query: uri.query().unwrap_or_default().as_bytes().to_vec(),
            body: body.to_vec(),
        });
    }
    match state.redirect_to {
        Some(location) => (StatusCode::TEMPORARY_REDIRECT, [(LOCATION, location)]).into_response(),
        None => StatusCode::OK.into_response(),
    }
}

struct CaptureServer {
    url: String,
    captured: oneshot::Receiver<CapturedRequest>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl CaptureServer {
    async fn start() -> Self {
        Self::start_with_redirect(None).await
    }

    async fn start_redirecting_to(location: String) -> Self {
        Self::start_with_redirect(Some(location)).await
    }

    async fn start_with_redirect(redirect_to: Option<String>) -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let (captured_tx, captured) = oneshot::channel();
        let state = CaptureState {
            tx: Arc::new(Mutex::new(Some(captured_tx))),
            redirect_to,
        };
        let (shutdown, shutdown_rx) = oneshot::channel();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/hook", post(capture_handler))
                    .with_state(state),
            )
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
        });
        Self {
            url: format!("http://127.0.0.1:{port}/hook"),
            captured,
            shutdown: Some(shutdown),
        }
    }

    fn url(&self) -> &str {
        &self.url
    }

    fn origin(&self) -> &str {
        self.url.strip_suffix("/hook").unwrap()
    }

    async fn one_request(mut self) -> CapturedRequest {
        let request = tokio::time::timeout(Duration::from_secs(3), &mut self.captured)
            .await
            .expect("forwarder did not reach the capture server")
            .unwrap();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        request
    }

    async fn assert_no_request(mut self) {
        let result = tokio::time::timeout(Duration::from_millis(250), &mut self.captured).await;
        let unexpected = match result {
            Ok(Ok(request)) => Some(request),
            Ok(Err(error)) => panic!("capture channel closed unexpectedly: {error}"),
            Err(_) => None,
        };
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        assert!(
            unexpected.is_none(),
            "capture server received an unexpected request: {unexpected:?}"
        );
    }
}

const PROXY_ENV: &[&str] = &[
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
];
const NO_PROXY_ENV: &[&str] = &["NO_PROXY", "no_proxy"];

fn run_forwarder_configured(
    session: Option<&str>,
    url: &str,
    body: &[u8],
    configure: impl FnOnce(&mut std::process::Command),
) -> (u32, std::process::Output) {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_agent-office"));
    command
        .args(["--observer-forward", "codex"])
        .env("AGENT_OFFICE_HOOK_URL", url)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for name in PROXY_ENV.iter().chain(NO_PROXY_ENV) {
        command.env_remove(name);
    }
    match session {
        Some(session) => {
            command.env("AGENT_OFFICE_SESSION", session);
        }
        None => {
            command.env_remove("AGENT_OFFICE_SESSION");
        }
    }
    configure(&mut command);
    let mut child = command.spawn().unwrap();
    let pid = child.id();
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), body).unwrap();
    drop(child.stdin.take());
    (pid, child.wait_with_output().unwrap())
}

fn run_forwarder(session: Option<&str>, url: &str, body: &[u8]) -> (u32, std::process::Output) {
    run_forwarder_configured(session, url, body, |_| {})
}

#[test]
fn forwarder_mode_requires_the_exact_complete_argument_vector() {
    use agent_office_lib::maybe_run_observer_forwarder;

    assert_eq!(maybe_run_observer_forwarder(["agent-office"]), None);
    assert_eq!(
        maybe_run_observer_forwarder(["agent-office", "--observer-forward"]),
        None,
    );
    assert_eq!(
        maybe_run_observer_forwarder(["agent-office", "--observer-forward", "claude"]),
        None,
    );
    assert_eq!(
        maybe_run_observer_forwarder(["agent-office", "--observer-forward", "codex", "extra",]),
        None,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn built_binary_forwarder_relays_raw_stdin_and_exits_silently() {
    let capture = CaptureServer::start().await;
    let body = b" {\"hook_event_name\":\"UserPromptSubmit\",\"prompt\":\"raw marker\"}\r\n";
    let (pid, output) = run_forwarder(Some("ao-marker-session"), capture.url(), body);

    assert!(
        output.status.success(),
        "pid={pid}, stderr={:?}",
        output.stderr
    );
    assert!(
        output.stdout.is_empty(),
        "pid={pid}, stdout={:?}",
        output.stdout
    );
    assert!(
        output.stderr.is_empty(),
        "pid={pid}, stderr={:?}",
        output.stderr
    );
    let request = capture.one_request().await;
    println!(
        "forwarder_pid={pid} raw_query={:?} raw_body={:?}",
        request.query, request.body
    );
    assert_eq!(request.session, "ao-marker-session");
    assert_eq!(request.provider, "codex");
    assert_eq!(request.query, b"session=ao-marker-session&provider=codex");
    assert_eq!(request.body, body);
}

#[test]
fn built_binary_forwarder_fails_open_when_loopback_is_unreachable() {
    let (pid, output) = run_forwarder(
        Some("ao-unreachable-session"),
        "http://127.0.0.1:0/hook",
        br#"{"hook_event_name":"Stop"}"#,
    );

    assert!(
        output.status.success(),
        "pid={pid}, stderr={:?}",
        output.stderr
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn built_binary_forwarder_requires_a_nonempty_inherited_session() {
    let capture = CaptureServer::start().await;
    let (pid, output) = run_forwarder(Some(""), capture.url(), br#"{"hook_event_name":"Stop"}"#);

    assert!(
        output.status.success(),
        "pid={pid}, stderr={:?}",
        output.stderr
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    capture.assert_no_request().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn built_binary_forwarder_rejects_a_non_numeric_loopback_host() {
    let capture = CaptureServer::start().await;
    let non_numeric_url = capture.url().replace("127.0.0.1", "localhost");
    let (pid, output) = run_forwarder(
        Some("ao-loopback-identity"),
        &non_numeric_url,
        br#"{"hook_event_name":"Stop"}"#,
    );

    assert!(
        output.status.success(),
        "pid={pid}, stderr={:?}",
        output.stderr
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    capture.assert_no_request().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn built_binary_forwarder_never_sends_loopback_payload_through_environment_proxy() {
    let proxy = CaptureServer::start().await;
    let proxy_url = proxy.origin().to_owned();
    let body = br#"{"hook_event_name":"Stop","marker":"must-not-reach-proxy"}"#;
    let (pid, output) = run_forwarder_configured(
        Some("ao-proxy-isolation"),
        "http://127.0.0.1:0/hook",
        body,
        |command| {
            for name in PROXY_ENV {
                command.env(name, &proxy_url);
            }
            for name in NO_PROXY_ENV {
                command.env_remove(name);
            }
        },
    );

    assert!(
        output.status.success(),
        "pid={pid}, stderr={:?}",
        output.stderr
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    proxy.assert_no_request().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn built_binary_forwarder_does_not_follow_loopback_redirect() {
    let redirected = CaptureServer::start().await;
    let redirect_location = format!(
        "{}?session=redirect-target&provider=codex",
        redirected.url()
    );
    let initial = CaptureServer::start_redirecting_to(redirect_location).await;
    let body = br#"{"hook_event_name":"Stop","marker":"must-not-follow"}"#;
    let (pid, output) = run_forwarder(Some("ao-redirect-source"), initial.url(), body);

    assert!(
        output.status.success(),
        "pid={pid}, stderr={:?}",
        output.stderr
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    let initial_request = initial.one_request().await;
    assert_eq!(initial_request.session, "ao-redirect-source");
    assert_eq!(initial_request.provider, "codex");
    assert_eq!(initial_request.body, body);
    redirected.assert_no_request().await;
}

/// docs/session-handoff-design.md §핵심 5: 재시작 후 입양된 세션은
/// AGENT_OFFICE_HOOK_URL이 스폰 시점의(죽은) 포트를 가리킨다. 실 빌드 바이너리를
/// 죽은 포트로 1차 시도시키고, `AGENT_OFFICE_APP_DATA/observer-port`에 진짜
/// observer 서버의 현재 포트를 심어 두면 forwarder가 1회 재시도해 결국 캡처
/// 서버가 요청을 받아야 한다.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn built_binary_forwarder_retries_via_observer_port_file_when_primary_is_unreachable() {
    let capture = CaptureServer::start().await;
    let port = capture
        .origin()
        .rsplit(':')
        .next()
        .expect("capture origin must contain a port");

    let app_data_dir = std::env::temp_dir().join(format!(
        "agent-office-forwarder-port-file-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&app_data_dir).unwrap();
    std::fs::write(app_data_dir.join("observer-port"), port).unwrap();

    let body = br#"{"hook_event_name":"Stop","marker":"via-port-file-retry"}"#;
    let (pid, output) = run_forwarder_configured(
        Some("ao-port-file-retry"),
        "http://127.0.0.1:0/hook", // 1차 시도는 반드시 실패하는 죽은 포트
        body,
        |command| {
            command.env("AGENT_OFFICE_APP_DATA", &app_data_dir);
        },
    );

    assert!(
        output.status.success(),
        "pid={pid}, stderr={:?}",
        output.stderr
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());

    let request = capture.one_request().await;
    assert_eq!(request.session, "ao-port-file-retry");
    assert_eq!(request.provider, "codex");
    assert_eq!(request.body, body);

    let _ = std::fs::remove_dir_all(&app_data_dir);
}

/// 포트 파일이 없거나(AGENT_OFFICE_APP_DATA 미설정) 재시도 자체가 불가능해도
/// 여전히 조용히 종료해야 한다(베스트에포트, 기존 fail-open 계약 유지).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn built_binary_forwarder_fails_open_when_retry_port_file_is_absent() {
    let (pid, output) = run_forwarder_configured(
        Some("ao-no-port-file"),
        "http://127.0.0.1:0/hook",
        br#"{"hook_event_name":"Stop"}"#,
        |command| {
            command.env_remove("AGENT_OFFICE_APP_DATA");
        },
    );

    assert!(
        output.status.success(),
        "pid={pid}, stderr={:?}",
        output.stderr
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}
