// src-tauri/src/control/mod.rs
//
// 로컬 CLI 제어 서버(이슈 #55, docs/cli-control-design.md). 실행 중인 GUI 앱
// 프로세스 안에서 `127.0.0.1`에 임의 포트로 axum HTTP 서버를 띄워, 같은
// 머신의 `agent-office ctl …` 클라이언트(control/client.rs)나 스크립트가
// 세션을 프로그래밍 방식으로 조종하게 한다. `observer/server.rs`의
// ObserverServerState 생명주기를 본떴다(임의 포트·포트 파일·graceful shutdown).
//
// 보안(2단계 옵트인):
//   1) 설정 `cli_enabled`가 켜져야 서버가 뜨고 `control-port`가 기록된다.
//   2) 앱에서 **명시적 승인**(control_approve 커맨드)으로 `control-token`이
//      발급돼야만 요청이 인증된다 — 토큰이 없으면 모든 요청이 401.
// 토큰 파일은 0600, 서버는 매 요청 시 `control-token` 파일 내용과 대조하므로
// 승인(파일 생성)/취소(파일 삭제)가 서버 재시작 없이 즉시 반영된다.

pub mod client;
pub mod protocol;
pub mod tmux;
/// 토큰 파일과 인증 미들웨어.
mod token;
/// 세션·알림·설정 핸들러.
mod routes;
/// 동료 대화 라우트.
mod talk;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use axum::routing::post;
use axum::Router;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::notification::hub::NotificationHub;
use crate::observer::server::ObserverServerState;
use crate::observer::ObserverRuntime;
use crate::persistence::profile_store::ProfileStore;
use crate::persistence::settings_store::{AppSettings, SettingsStore};
use crate::session::manager::SessionManager;
use crate::state::SessionRegistry;

use protocol::*;

// 라우터가 이름으로 엮는 핸들러들 — 정의는 도메인별 파일에 있다.
pub use token::{issue_token_at, read_token_at, revoke_token_at};
use routes::{
    attach, clear, create, detach, dispose, list, notifications, ping, send, settings_get,
    settings_set,
};
use talk::{talk_end, talk_inbox, talk_reply, talk_roster, talk_send};
use token::auth;

/// control 핸들러가 기존 command 본문과 동일한 동작을 내기 위해 쥐는 앱
/// 상태 클론들. `AppState`가 보유한 Arc/스토어를 setup에서 clone해 담는다
/// (`AppState` 자체는 Tauri가 소유해 Arc로 꺼낼 수 없으므로, 필요한 조각만
/// 복제한다 — ObserverServerState가 여러 Arc를 clone해 쓰는 것과 같은 관례).
pub struct ControlContext {
    pub manager: Arc<SessionManager>,
    pub observer: Arc<ObserverRuntime>,
    pub observer_server: Arc<ObserverServerState>,
    pub hub: Arc<NotificationHub>,
    pub registry: Arc<SessionRegistry>,
    pub store: ProfileStore,
    pub settings: Arc<RwLock<AppSettings>>,
    pub settings_store: SettingsStore,
    /// 동료 대화 허브(docs/agent-talk-design.md). 큐·대화 상태·킬 스위치를 쥔다.
    pub talk: Arc<crate::talk::TalkHub>,
    /// 토큰 파일(`control-token`)을 대조할 위치. 서버의 app_data_dir과 동일.
    pub app_data_dir: PathBuf,
    /// `attach --tmux`가 대상 세션 존재를 확인할 때 쓰는 확인기.
    /// 프로덕션은 `tmux::system_probe()`, 테스트는 가짜를 주입한다.
    pub tmux_probe: tmux::TmuxProbe,
}

impl ControlContext {
    fn read_token(&self) -> Option<String> {
        read_token_at(&self.app_data_dir)
    }
}

fn router(ctx: Arc<ControlContext>) -> Router {
    Router::new()
        .route("/v1/ping", post(ping))
        .route("/v1/list", post(list))
        .route("/v1/create", post(create))
        .route("/v1/attach", post(attach))
        .route("/v1/detach", post(detach))
        .route("/v1/send", post(send))
        .route("/v1/dispose", post(dispose))
        .route("/v1/notifications", post(notifications))
        .route("/v1/clear", post(clear))
        .route("/v1/talk/roster", post(talk_roster))
        .route("/v1/talk/send", post(talk_send))
        .route("/v1/talk/reply", post(talk_reply))
        .route("/v1/talk/inbox", post(talk_inbox))
        .route("/v1/talk/end", post(talk_end))
        .route("/v1/settings/get", post(settings_get))
        .route("/v1/settings/set", post(settings_set))
        .layer(axum::middleware::from_fn_with_state(ctx.clone(), auth))
        .with_state(ctx)
}

async fn serve(
    ctx: Arc<ControlContext>,
    shutdown_rx: oneshot::Receiver<()>,
) -> std::io::Result<(u16, JoinHandle<()>)> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let app = router(ctx);
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    Ok((port, handle))
}

// ── 서버 생명주기(ObserverServerState 미러의 경량판) ──────────────────

struct InstalledServer {
    port: u16,
    shutdown: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

#[derive(Default)]
pub struct ControlServerState {
    /// 동시 ensure 직렬화(부트 + set_app_settings가 겹치는 드문 경우 방어).
    start_gate: tokio::sync::Mutex<()>,
    installed: Mutex<Option<InstalledServer>>,
    app_data_dir: Mutex<Option<PathBuf>>,
}

impl ControlServerState {
    pub fn set_app_data_dir(&self, dir: PathBuf) {
        *self.app_data_dir.lock().unwrap() = Some(dir);
    }

    fn data_dir(&self) -> Option<PathBuf> {
        self.app_data_dir.lock().unwrap().clone()
    }

    pub fn current_port(&self) -> Option<u16> {
        self.installed.lock().unwrap().as_ref().map(|s| s.port)
    }

    pub fn is_running(&self) -> bool {
        self.installed.lock().unwrap().is_some()
    }

    /// 승인 여부 = 토큰 파일 존재.
    pub fn is_approved(&self) -> bool {
        self.data_dir().and_then(|d| read_token_at(&d)).is_some()
    }

    pub fn issue_token(&self) -> Result<String, String> {
        let dir = self.data_dir().ok_or("app data dir 미설정")?;
        issue_token_at(&dir).map_err(|e| e.to_string())
    }

    pub fn revoke_token(&self) -> Result<(), String> {
        let dir = self.data_dir().ok_or("app data dir 미설정")?;
        revoke_token_at(&dir).map_err(|e| e.to_string())
    }

    /// opt-in 기동(멱등) — 이미 떠 있으면 그 포트를 재사용한다. bind 실패는
    /// 1회 재시도 후 fail-open(None)으로, GUI 기능에 영향을 주지 않는다.
    pub async fn ensure(&self, ctx: Arc<ControlContext>) -> Option<u16> {
        let _gate = self.start_gate.lock().await;
        if let Some(port) = self.current_port() {
            return Some(port);
        }
        let (shutdown, rx) = oneshot::channel();
        let started = match serve(ctx.clone(), rx).await {
            Ok((port, handle)) => Some((port, shutdown, handle)),
            Err(_) => {
                let (shutdown2, rx2) = oneshot::channel();
                match serve(ctx.clone(), rx2).await {
                    Ok((port, handle)) => Some((port, shutdown2, handle)),
                    Err(e) => {
                        eprintln!("control server unavailable: {e}");
                        None
                    }
                }
            }
        };
        let (port, shutdown, handle) = started?;
        *self.installed.lock().unwrap() = Some(InstalledServer {
            port,
            shutdown,
            handle,
        });
        self.write_port_file(port);
        Some(port)
    }

    /// 서버를 내리고 포트 파일을 지운다(토큰은 유지 — "한 번 승인하면 지속",
    /// 재활성화 시 재승인 불필요. 명시적 취소는 revoke_token). 종료 훅과
    /// cli_enabled OFF 전환에서 호출.
    pub fn shutdown(&self) {
        let installed = self.installed.lock().unwrap().take();
        if let Some(server) = installed {
            let _ = server.shutdown.send(());
            let _detached = server.handle;
        }
        self.remove_port_file();
    }

    fn write_port_file(&self, port: u16) {
        let Some(dir) = self.data_dir() else {
            return;
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("control-port: create {} 실패: {e}", dir.display());
            return;
        }
        if let Err(e) = std::fs::write(dir.join(PORT_FILE), port.to_string()) {
            eprintln!("control-port: write {} 실패: {e}", dir.display());
        }
    }

    fn remove_port_file(&self) {
        if let Some(dir) = self.data_dir() {
            let _ = std::fs::remove_file(dir.join(PORT_FILE));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification::hub::{NotificationHub, SystemClock};
    use crate::session::pty_factory::fake::{FakeControl, FakePtyFactory};
    use crate::state::fake::RecordingEvents;
    use crate::state::AppEvents;
    use std::time::Duration;

    struct Fixture {
        state: ControlServerState,
        ctx: Arc<ControlContext>,
        /// 가짜 PTY 핸들 — 세션 stdin에 뭐가 실렸는지(=startup_command) 본다.
        ctl: Arc<FakeControl>,
        dir: PathBuf,
        _observer_dir: PathBuf,
    }

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-control-test-{tag}-{}",
            uuid::Uuid::new_v4()
        ))
    }

    /// tmux를 쓰지 않는 테스트용 기본 확인기 — 실수로 tmux 경로를 타면
    /// 조용히 통과하지 말고 눈에 띄게 실패하라고 Unavailable을 낸다.
    fn build(tag: &str) -> Fixture {
        build_with_tmux(
            tag,
            Arc::new(|_| tmux::TmuxStatus::Unavailable("테스트 확인기".into())),
        )
    }

    /// 세션 둘 이상을 띄우는 테스트(동료 대화)용 — 매 spawn마다 새 FakeControl을
    /// 주는 팩토리를 쓴다. 나머지 배선은 `build`와 같다.
    fn build_multi(tag: &str) -> Fixture {
        build_inner(
            tag,
            Arc::new(|_| tmux::TmuxStatus::Unavailable("테스트 확인기".into())),
            true,
        )
    }

    fn build_with_tmux(tag: &str, tmux_probe: tmux::TmuxProbe) -> Fixture {
        build_inner(tag, tmux_probe, false)
    }

    fn build_inner(tag: &str, tmux_probe: tmux::TmuxProbe, multi: bool) -> Fixture {
        let events: Arc<RecordingEvents> = Arc::new(RecordingEvents::default());
        let events_dyn: Arc<dyn AppEvents> = events.clone();
        let registry = Arc::new(SessionRegistry::new());
        let hub = Arc::new(NotificationHub::new(
            registry.clone(),
            events_dyn.clone(),
            Arc::new(SystemClock),
            Duration::from_millis(3000),
        ));
        let observer_dir = scratch(&format!("{tag}-observer"));
        let observer = Arc::new(ObserverRuntime::production(
            hub.clone(),
            observer_dir.clone(),
            std::env::current_exe().unwrap(),
        ));
        let observer_server = Arc::new(ObserverServerState::default());
        let settings = Arc::new(RwLock::new(AppSettings::default()));
        let get_observer_url =
            crate::make_observer_url_getter(settings.clone(), observer_server.clone());
        let (fac, ctl) = FakePtyFactory::new();
        let factory: Arc<dyn crate::session::pty_factory::PtyFactory> = if multi {
            Arc::new(crate::session::pty_factory::fake::MultiFakePtyFactory::new())
        } else {
            Arc::new(fac)
        };
        let manager = Arc::new(SessionManager::new(
            factory,
            observer.clone(),
            registry.clone(),
            events_dyn,
            hub.clone(),
            get_observer_url,
        ));
        let dir = scratch(&format!("{tag}-data"));
        std::fs::create_dir_all(&dir).unwrap();
        let store = ProfileStore::new(dir.join("profiles.json"));
        let settings_store = SettingsStore::new(dir.join("settings.json"));
        // 대화 테스트는 이 허브를 켜고 쓴다(기본은 꺼짐 = 라우트가 전부 거절).
        let talk_hub = Arc::new(crate::talk::TalkHub::default());
        let ctx = Arc::new(ControlContext {
            manager,
            observer,
            observer_server,
            hub,
            registry,
            store,
            settings,
            settings_store,
            talk: talk_hub,
            app_data_dir: dir.clone(),
            tmux_probe,
        });
        let state = ControlServerState::default();
        state.set_app_data_dir(dir.clone());
        Fixture {
            state,
            ctx,
            ctl,
            dir,
            _observer_dir: observer_dir,
        }
    }

    fn cleanup(f: &Fixture) {
        f.state.shutdown();
        let _ = std::fs::remove_dir_all(&f.dir);
        let _ = std::fs::remove_dir_all(&f._observer_dir);
    }

    #[test]
    fn token_issue_read_revoke_roundtrip() {
        let dir = scratch("token");
        assert!(read_token_at(&dir).is_none());
        let token = issue_token_at(&dir).unwrap();
        assert_eq!(read_token_at(&dir).as_deref(), Some(token.as_str()));
        revoke_token_at(&dir).unwrap();
        assert!(read_token_at(&dir).is_none());
        // revoke가 멱등(파일 없어도 Ok).
        revoke_token_at(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn issued_token_file_is_owner_only_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("token-perm");
        issue_token_at(&dir).unwrap();
        let mode = std::fs::metadata(dir.join(TOKEN_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn missing_token_rejects_with_401() {
        let f = build("no-token");
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        // 토큰 미발급(미승인) → 서버는 떠 있지만 모든 요청 401.
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://127.0.0.1:{port}/v1/ping"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
        cleanup(&f);
    }

    #[tokio::test]
    async fn wrong_token_rejects_and_correct_token_authorizes() {
        let f = build("token-check");
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let token = f.state.issue_token().unwrap();
        let client = reqwest::Client::new();

        let bad = client
            .post(format!("http://127.0.0.1:{port}/v1/ping"))
            .header(TOKEN_HEADER, "deadbeef")
            .send()
            .await
            .unwrap();
        assert_eq!(bad.status(), reqwest::StatusCode::UNAUTHORIZED);

        let good = client
            .post(format!("http://127.0.0.1:{port}/v1/ping"))
            .header(TOKEN_HEADER, &token)
            .send()
            .await
            .unwrap();
        assert!(good.status().is_success());
        let body: serde_json::Value = good.json().await.unwrap();
        assert_eq!(body["ok"], true);
        assert_eq!(body["data"]["agentCount"], 0);
        cleanup(&f);
    }

    #[tokio::test]
    async fn revoke_makes_previously_valid_token_401() {
        let f = build("revoke");
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let token = f.state.issue_token().unwrap();
        let client = reqwest::Client::new();
        let ping = |t: String| {
            let client = client.clone();
            async move {
                client
                    .post(format!("http://127.0.0.1:{port}/v1/ping"))
                    .header(TOKEN_HEADER, t)
                    .send()
                    .await
                    .unwrap()
                    .status()
            }
        };
        assert!(ping(token.clone()).await.is_success());
        f.state.revoke_token().unwrap();
        assert_eq!(
            ping(token.clone()).await,
            reqwest::StatusCode::UNAUTHORIZED
        );
        cleanup(&f);
    }

    #[tokio::test]
    async fn create_then_send_then_list_roundtrip() {
        let f = build("roundtrip");
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let token = f.state.issue_token().unwrap();
        // 프로필 하나 저장(list가 병합해 보여줄 대상).
        let profiles = crate::types::PersistedState {
            agents: vec![profile("a1", "Ada")],
            version: 1,
            vacation_mode: None,
        };
        f.ctx.store.save(&profiles).unwrap();
        let client = reqwest::Client::new();

        let created: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/create"))
            .header(TOKEN_HEADER, &token)
            .json(&serde_json::json!({ "agentId": "a1" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(created["ok"], true);
        assert_eq!(created["data"]["state"], "running");

        // send는 no-op 성공(FakePtyFactory라 실 stdin은 검증 안 함).
        let sent: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/send"))
            .header(TOKEN_HEADER, &token)
            .json(&serde_json::json!({ "agentId": "a1", "data": "echo hi\n" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(sent["ok"], true);

        let listed: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/list"))
            .header(TOKEN_HEADER, &token)
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(listed["ok"], true);
        let arr = listed["data"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["agentId"], "a1");
        assert_eq!(arr[0]["state"], "running");
        cleanup(&f);
    }

    #[tokio::test]
    async fn attach_returns_an_evaluable_script_and_detach_reissues_a_session() {
        let f = build("attach");
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let token = f.state.issue_token().unwrap();
        // 훅 ON(기본값은 OFF) — attach 핸들러가 observer 서버를 선기동하고,
        // 그래야 스크립트에 훅 env와 claude 래퍼가 실린다.
        f.ctx.settings.write().unwrap().observer_enabled = true;
        f.ctx
            .store
            .save(&crate::types::PersistedState {
                agents: vec![profile("a1", "Ada")],
                version: 1,
                vacation_mode: None,
            })
            .unwrap();
        let client = reqwest::Client::new();
        let attach = |body: serde_json::Value| {
            let client = client.clone();
            let token = token.clone();
            async move {
                client
                    .post(format!("http://127.0.0.1:{port}/v1/attach"))
                    .header(TOKEN_HEADER, &token)
                    .json(&body)
                    .send()
                    .await
                    .unwrap()
                    .json::<serde_json::Value>()
                    .await
                    .unwrap()
            }
        };

        let first = attach(serde_json::json!({ "agentId": "a1", "cwd": "/tmp/proj" })).await;
        assert_eq!(first["ok"], true);
        assert_eq!(first["data"]["mode"], "new");
        let sid = first["data"]["sessionId"].as_str().unwrap().to_string();
        let script = first["data"]["script"].as_str().unwrap();
        assert!(
            script.contains(&format!("export AGENT_OFFICE_SESSION='{sid}'")),
            "{script}",
        );
        assert!(script.contains("claude() {"), "{script}");

        // 붙어 있는 동안 다시 attach하면 새 sid를 재발급한다(외부 세션 교체).
        let again = attach(serde_json::json!({ "agentId": "a1" })).await;
        assert_eq!(again["data"]["mode"], "new");
        assert_ne!(again["data"]["sessionId"].as_str().unwrap(), sid);
        let second_sid = again["data"]["sessionId"].as_str().unwrap().to_string();

        // detach는 실제로 끊었는지 알려주고, 두 번째는 no-op이다.
        let detach = |body: serde_json::Value| {
            let client = client.clone();
            let token = token.clone();
            async move {
                client
                    .post(format!("http://127.0.0.1:{port}/v1/detach"))
                    .header(TOKEN_HEADER, &token)
                    .json(&body)
                    .send()
                    .await
                    .unwrap()
                    .json::<serde_json::Value>()
                    .await
                    .unwrap()
            }
        };
        let detached = detach(serde_json::json!({ "agentId": "a1" })).await;
        assert_eq!(detached["ok"], true);
        assert_eq!(detached["data"]["detached"], true);
        assert_eq!(
            detach(serde_json::json!({ "agentId": "a1" })).await["data"]["detached"],
            false
        );

        // detach 후 재attach는 또 다른 sid를 발급한다.
        let third = attach(serde_json::json!({ "agentId": "a1" })).await;
        assert_eq!(third["data"]["mode"], "new");
        assert_ne!(third["data"]["sessionId"].as_str().unwrap(), second_sid);

        // 앱 안 PTY 세션이 살아 있으면 그 sid에 합류한다(1캐릭터 1세션).
        let created: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/create"))
            .header(TOKEN_HEADER, &token)
            .json(&serde_json::json!({ "agentId": "a1" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let pty_sid = created["data"]["sessionId"].as_str().unwrap().to_string();
        let bound = attach(serde_json::json!({ "agentId": "a1" })).await;
        assert_eq!(bound["data"]["mode"], "bind");
        assert_eq!(bound["data"]["sessionId"].as_str().unwrap(), pty_sid);

        cleanup(&f);
    }

    #[tokio::test]
    async fn attach_rejects_an_unknown_agent_and_requires_the_token() {
        let f = build("attach-unknown");
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let client = reqwest::Client::new();

        // 미승인(토큰 없음) → 401.
        let unauthorized = client
            .post(format!("http://127.0.0.1:{port}/v1/attach"))
            .json(&serde_json::json!({ "agentId": "nope" }))
            .send()
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);

        let token = f.state.issue_token().unwrap();
        let body: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/attach"))
            .header(TOKEN_HEADER, &token)
            .json(&serde_json::json!({ "agentId": "nope" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(body["ok"], false);
        assert!(
            body["error"].as_str().unwrap().contains("ctl create"),
            "{body}"
        );
        cleanup(&f);
    }

    /// tmux 모드는 논리 세션이 아니라 **일반 PTY 세션**을 띄운다 — 그래서
    /// 검증할 것은 (1) 응답 mode/script, (2) 그 세션 stdin에 실린 시작 명령이다.
    #[tokio::test]
    async fn attach_with_tmux_spawns_a_client_session_and_returns_a_comment_only_script() {
        let asked: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let seen = asked.clone();
        let f = build_with_tmux(
            "tmux-ok",
            Arc::new(move |target: &str| {
                seen.lock().unwrap().push(target.to_string());
                tmux::TmuxStatus::Present
            }),
        );
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let token = f.state.issue_token().unwrap();
        let mut agent = profile("a1", "Ada");
        agent.personality_prompt = Some("짧게 답한다".into());
        f.ctx
            .store
            .save(&crate::types::PersistedState {
                agents: vec![agent],
                version: 1,
                vacation_mode: None,
            })
            .unwrap();

        let body: serde_json::Value = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/v1/attach"))
            .header(TOKEN_HEADER, &token)
            .json(&serde_json::json!({ "agentId": "a1", "tmux": "  work  ", "pid": 4242 }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(body["ok"], true, "{body}");
        assert_eq!(body["data"]["mode"], "tmux");
        // 확인기에는 다듬은 이름이 간다.
        assert_eq!(asked.lock().unwrap().as_slice(), ["work".to_string()]);

        // eval해도 무해해야 한다 — 전부 코멘트 줄이고, pane 안내가 들어 있다.
        let script = body["data"]["script"].as_str().unwrap();
        assert!(script.lines().all(|l| l.starts_with('#')), "{script}");
        assert!(script.contains("tmux 세션 'work'"), "{script}");
        assert!(script.contains("ctl attach a1"), "{script}");

        // 세션 stdin에 tmux 클라이언트 기동 명령이 실렸다(기존 PTY 파이프라인 재사용).
        let writes = f.ctl.writes_utf8();
        assert!(
            writes.contains("exec tmux attach-session -t 'work'"),
            "{writes}"
        );
        // 프로필 성격도 함께 실린다(create 경로 공유).
        assert!(
            f.ctl
                .spawned_env()
                .iter()
                .any(|(k, v)| k == "AGENT_OFFICE_PERSONA" && v == "짧게 답한다"),
            "{:?}",
            f.ctl.spawned_env()
        );

        // 이미 세션이 떠 있으면 create가 그것을 재사용해 tmux 클라이언트가 뜨지
        // 않는다 — 성공으로 위장하지 않고 거절한다.
        let again: serde_json::Value = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/v1/attach"))
            .header(TOKEN_HEADER, &token)
            .json(&serde_json::json!({ "agentId": "a1", "tmux": "work" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(again["ok"], false, "{again}");
        assert!(
            again["error"].as_str().unwrap().contains("ctl dispose"),
            "{again}"
        );
        cleanup(&f);
    }

    #[tokio::test]
    async fn attach_with_tmux_rejects_a_bad_target_or_a_missing_tmux_session() {
        let calls: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
        let counted = calls.clone();
        let f = build_with_tmux(
            "tmux-missing",
            Arc::new(move |_| {
                *counted.lock().unwrap() += 1;
                tmux::TmuxStatus::Missing
            }),
        );
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let token = f.state.issue_token().unwrap();
        f.ctx
            .store
            .save(&crate::types::PersistedState {
                agents: vec![profile("a1", "Ada")],
                version: 1,
                vacation_mode: None,
            })
            .unwrap();
        let client = reqwest::Client::new();
        let attach = |body: serde_json::Value| {
            let client = client.clone();
            let token = token.clone();
            async move {
                client
                    .post(format!("http://127.0.0.1:{port}/v1/attach"))
                    .header(TOKEN_HEADER, &token)
                    .json(&body)
                    .send()
                    .await
                    .unwrap()
                    .json::<serde_json::Value>()
                    .await
                    .unwrap()
            }
        };

        // 개행이 섞인 대상은 tmux를 돌려보기도 전에 거절한다.
        let bad = attach(serde_json::json!({ "agentId": "a1", "tmux": "work\nrm -rf /" })).await;
        assert_eq!(bad["ok"], false);
        assert!(bad["error"].as_str().unwrap().contains("제어문자"), "{bad}");
        let empty = attach(serde_json::json!({ "agentId": "a1", "tmux": "  " })).await;
        assert_eq!(empty["ok"], false);
        assert_eq!(*calls.lock().unwrap(), 0);

        // tmux는 돌았지만 그런 세션이 없다.
        let missing = attach(serde_json::json!({ "agentId": "a1", "tmux": "nope" })).await;
        assert_eq!(missing["ok"], false);
        assert!(
            missing["error"]
                .as_str()
                .unwrap()
                .contains("찾을 수 없습니다"),
            "{missing}"
        );
        assert_eq!(*calls.lock().unwrap(), 1);
        cleanup(&f);

        // tmux 자체가 없다.
        let f = build_with_tmux(
            "tmux-absent",
            Arc::new(|_| tmux::TmuxStatus::Unavailable("No such file or directory".into())),
        );
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let token = f.state.issue_token().unwrap();
        f.ctx
            .store
            .save(&crate::types::PersistedState {
                agents: vec![profile("a1", "Ada")],
                version: 1,
                vacation_mode: None,
            })
            .unwrap();
        let absent: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/attach"))
            .header(TOKEN_HEADER, &token)
            .json(&serde_json::json!({ "agentId": "a1", "tmux": "work" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(absent["ok"], false);
        assert!(
            absent["error"]
                .as_str()
                .unwrap()
                .contains("tmux를 실행할 수 없습니다"),
            "{absent}"
        );
        cleanup(&f);
    }

    /// CLI로 띄운 세션도 프로필 성격을 받아야 한다(예전엔 무조건 None이었다).
    #[tokio::test]
    async fn create_passes_the_profile_personality_prompt() {
        let f = build("create-persona");
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let token = f.state.issue_token().unwrap();
        let mut agent = profile("a1", "Ada");
        agent.personality_prompt = Some("항상 존댓말로 답한다".into());
        f.ctx
            .store
            .save(&crate::types::PersistedState {
                agents: vec![agent],
                version: 1,
                vacation_mode: None,
            })
            .unwrap();

        let created: serde_json::Value = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/v1/create"))
            .header(TOKEN_HEADER, &token)
            .json(&serde_json::json!({ "agentId": "a1" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(created["ok"], true, "{created}");
        assert!(
            f.ctl
                .spawned_env()
                .iter()
                .any(|(k, v)| k == "AGENT_OFFICE_PERSONA" && v == "항상 존댓말로 답한다"),
            "{:?}",
            f.ctl.spawned_env()
        );
        cleanup(&f);
    }

    #[tokio::test]
    async fn settings_set_rejects_cli_enabled_but_allows_others() {
        let f = build("settings");
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let token = f.state.issue_token().unwrap();
        let client = reqwest::Client::new();

        let rejected: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/settings/set"))
            .header(TOKEN_HEADER, &token)
            .json(&serde_json::json!({ "cliEnabled": false }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(rejected["ok"], false);

        let ok_resp: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/settings/set"))
            .header(TOKEN_HEADER, &token)
            .json(&serde_json::json!({ "typingSoundEnabled": false }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(ok_resp["ok"], true);
        assert_eq!(ok_resp["data"]["typingSoundEnabled"], false);
        assert!(!f.ctx.settings.read().unwrap().typing_sound_enabled);
        // 3분할 이후 `soundEnabled`는 와이어에 없다 — 병합 패치로 들어와도
        // 무시된다(로드 시점 마이그레이션만 옛 키를 인정한다).
        assert!(f.ctx.settings.read().unwrap().notify_sound_enabled);
        cleanup(&f);
    }

    #[tokio::test]
    async fn port_file_written_on_ensure_and_removed_on_shutdown() {
        let f = build("port-file");
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let written = std::fs::read_to_string(f.dir.join(PORT_FILE)).unwrap();
        assert_eq!(written.trim(), port.to_string());
        f.state.shutdown();
        assert!(!f.dir.join(PORT_FILE).exists());
        let _ = std::fs::remove_dir_all(&f.dir);
        let _ = std::fs::remove_dir_all(&f._observer_dir);
    }

    // ── 동료 대화(docs/agent-talk-design.md) ─────────────────────────

    /// 캐릭터 둘을 띄우고 각자의 세션 id(=발신자 신원 헤더 값)를 돌려준다.
    async fn two_characters(
        f: &Fixture,
        port: u16,
        token: &str,
    ) -> (reqwest::Client, String, String) {
        let profiles = crate::types::PersistedState {
            agents: vec![profile("a1", "Ada"), profile("a2", "Bob")],
            version: 1,
            vacation_mode: None,
        };
        f.ctx.store.save(&profiles).unwrap();
        let client = reqwest::Client::new();
        let mut sids = Vec::new();
        for agent in ["a1", "a2"] {
            let created: serde_json::Value = client
                .post(format!("http://127.0.0.1:{port}/v1/create"))
                .header(TOKEN_HEADER, token)
                .json(&serde_json::json!({ "agentId": agent }))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            sids.push(
                created["data"]["sessionId"]
                    .as_str()
                    .unwrap_or_else(|| panic!("create {agent} 실패: {created}"))
                    .to_string(),
            );
        }
        (client, sids[0].clone(), sids[1].clone())
    }

    async fn talk_post(
        client: &reqwest::Client,
        port: u16,
        token: &str,
        route: &str,
        session: Option<&str>,
        body: serde_json::Value,
    ) -> serde_json::Value {
        let mut req = client
            .post(format!("http://127.0.0.1:{port}/v1/talk/{route}"))
            .header(TOKEN_HEADER, token)
            .json(&body);
        if let Some(sid) = session {
            req = req.header(SESSION_HEADER, sid);
        }
        req.send().await.unwrap().json().await.unwrap()
    }

    #[tokio::test]
    async fn talk_routes_require_the_toggle_and_a_real_office_session() {
        let f = build_multi("talk-gate");
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let token = f.state.issue_token().unwrap();
        let (client, sid_a, _sid_b) = two_characters(&f, port, &token).await;

        // 토글이 꺼져 있으면 무엇도 되지 않는다(킬 스위치가 곧 게이트다).
        let off = talk_post(&client, port, &token, "roster", Some(&sid_a), serde_json::json!({})).await;
        assert_eq!(off["ok"], false);
        assert!(off["error"].as_str().unwrap().contains("꺼져"));

        f.ctx.talk.set_enabled(true);
        // 세션 헤더가 없으면 발신자를 특정할 수 없다 — 앱 밖 셸은 참여 불가.
        let anon = talk_post(&client, port, &token, "roster", None, serde_json::json!({})).await;
        assert_eq!(anon["ok"], false);
        assert!(anon["error"].as_str().unwrap().contains("오피스 세션"));

        // 남의 세션 id를 지어내도 캐릭터로 이어지지 않는다(사칭 차단).
        let fake = talk_post(&client, port, &token, "roster", Some("made-up"), serde_json::json!({})).await;
        assert_eq!(fake["ok"], false);

        let roster = talk_post(&client, port, &token, "roster", Some(&sid_a), serde_json::json!({})).await;
        assert_eq!(roster["ok"], true);
        let rows = roster["data"].as_array().unwrap();
        let me = rows.iter().find(|r| r["agentId"] == "a1").unwrap();
        let other = rows.iter().find(|r| r["agentId"] == "a2").unwrap();
        assert_eq!(me["isMe"], true);
        assert_eq!(me["reachable"], false);
        assert_eq!(other["reachable"], true);
        cleanup(&f);
    }

    #[tokio::test]
    async fn talk_send_reply_roundtrip_between_two_characters() {
        let f = build_multi("talk-roundtrip");
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let token = f.state.issue_token().unwrap();
        f.ctx.talk.set_enabled(true);
        let (client, sid_a, sid_b) = two_characters(&f, port, &token).await;

        // 이름으로도 상대를 지정할 수 있다.
        let sent = talk_post(
            &client,
            port,
            &token,
            "send",
            Some(&sid_a),
            serde_json::json!({ "to": "Bob", "text": "배포 스크립트 어디 있어?" }),
        )
        .await;
        assert_eq!(sent["ok"], true, "{sent}");
        let conv = sent["data"]["convId"].as_str().unwrap().to_string();

        // 받는 쪽이 대기 중이면 PTY 주입 대신 응답으로 건네준다.
        let inbox = talk_post(&client, port, &token, "inbox", Some(&sid_b), serde_json::json!({})).await;
        let msgs = inbox["data"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["convId"], conv.as_str());
        assert_eq!(msgs[0]["fromName"], "Ada");

        let replied = talk_post(
            &client,
            port,
            &token,
            "reply",
            Some(&sid_b),
            serde_json::json!({ "convId": conv, "text": "scripts/deploy.sh" }),
        )
        .await;
        assert_eq!(replied["ok"], true, "{replied}");

        let back = talk_post(&client, port, &token, "inbox", Some(&sid_a), serde_json::json!({})).await;
        let msgs = back["data"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["text"], "scripts/deploy.sh");

        // 참여자가 아닌 세션은 그 대화를 닫을 수 없다.
        let ended = talk_post(
            &client,
            port,
            &token,
            "end",
            Some(&sid_a),
            serde_json::json!({ "convId": conv }),
        )
        .await;
        assert_eq!(ended["ok"], true);
        cleanup(&f);
    }

    #[tokio::test]
    async fn talk_send_refuses_receivers_that_opted_out() {
        let f = build_multi("talk-optout");
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let token = f.state.issue_token().unwrap();
        f.ctx.talk.set_enabled(true);
        let (client, sid_a, _sid_b) = two_characters(&f, port, &token).await;

        let mut profiles = f.ctx.store.load();
        profiles
            .agents
            .iter_mut()
            .find(|a| a.id == "a2")
            .unwrap()
            .talk_receive = Some(false);
        f.ctx.store.save(&profiles).unwrap();

        let refused = talk_post(
            &client,
            port,
            &token,
            "send",
            Some(&sid_a),
            serde_json::json!({ "to": "a2", "text": "안녕" }),
        )
        .await;
        assert_eq!(refused["ok"], false);
        assert!(refused["error"].as_str().unwrap().contains("수신 꺼짐"), "{refused}");

        // 없는 상대도 조용히 큐에 쌓이지 않고 그 자리에서 거절된다.
        let missing = talk_post(
            &client,
            port,
            &token,
            "send",
            Some(&sid_a),
            serde_json::json!({ "to": "없는사람", "text": "안녕" }),
        )
        .await;
        assert_eq!(missing["ok"], false);
        cleanup(&f);
    }

    #[tokio::test]
    async fn cli_cannot_turn_the_talk_switch_on_by_itself() {
        let f = build("talk-escalation");
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let token = f.state.issue_token().unwrap();
        let client = reqwest::Client::new();
        let resp: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/settings/set"))
            .header(TOKEN_HEADER, &token)
            .json(&serde_json::json!({ "talkEnabled": true }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["ok"], false);
        assert!(!f.ctx.talk.is_enabled());
        cleanup(&f);
    }

    fn profile(id: &str, name: &str) -> crate::types::AgentProfile {
        crate::types::AgentProfile {
            id: id.into(),
            name: name.into(),
            role: "backend".into(),
            legacy_note: None,
            seed: "seed".into(),
            created_at: 1,
            desk_index: 0,
            assigned_desk_index: None,
            cwd: None,
            legacy_appearance: None,
            portrait_request: None,
            portrait_updated_at: None,
            sprite_request: None,
            minimi_request: None,
            sprite_updated_at: None,
            minimi_updated_at: None,
            archetype: None,
            colors: None,
            shell: None,
            startup_command: None,
            personality_prompt: None,
            clocked_out: None,
            keyboard_sound: None,
            voice_id: None,
            bot: None,
            talk_receive: None,
        }
    }
}
