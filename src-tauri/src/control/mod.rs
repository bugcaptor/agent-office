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
/// VS Code 연결 확장이 캐릭터·외형·사용량을 읽는 제한된 표시 API.
mod display;
pub mod protocol;
/// 토큰 파일과 인증 미들웨어.
mod token;
/// 세션·알림·설정 핸들러.
mod routes;
/// 동료 대화 라우트.

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
use routes::{clear, create, dispose, list, notifications, ping, send, settings_get, settings_set};
use display::{appearance, capabilities, characters, focus, usage};
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
    /// 토큰 파일(`control-token`)을 대조할 위치. 서버의 app_data_dir과 동일.
    pub app_data_dir: PathBuf,
    /// 자동 입력 관문(kbm #2t9 Phase 2).
    pub gate: Arc<crate::session::inject::InjectGate>,
    /// 네이티브 커맨드와 같은 사용량 스로틀/캐시 상태.
    pub live_usage: Arc<crate::usage::LiveUsageState>,
    /// 커스텀 초상 저장소. 읽기 API도 앱과 같은 바이트 상한을 쓴다.
    pub portrait_store: Arc<crate::persistence::png_store::PngStore>,
    /// 커스텀 스프라이트 저장소. 읽기 API도 앱과 같은 바이트 상한을 쓴다.
    pub sprite_store: crate::persistence::png_store::PngStore,
    /// 창을 복원하고 렌더러에 표시 전용 선택을 알린다. 세션을 생성하지 않는다.
    pub focus_agent: Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>,
    /// 설정이 저장될 때마다(GUI·CLI 공통) 새 설정 전체를 렌더러에 알린다(수정 F6).
    /// 설정 write 가드 안에서 불리므로 블로킹·재진입하지 않아야 한다.
    pub settings_changed: Arc<dyn Fn(&AppSettings) + Send + Sync>,
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
        .route("/v1/send", post(send))
        .route("/v1/dispose", post(dispose))
        .route("/v1/notifications", post(notifications))
        .route("/v1/clear", post(clear))
        .route("/v1/settings/get", post(settings_get))
        .route("/v1/settings/set", post(settings_set))
        .route("/v1/display/capabilities", post(capabilities))
        .route("/v1/display/characters", post(characters))
        .route("/v1/display/appearance", post(appearance))
        .route("/v1/display/usage", post(usage))
        .route("/v1/display/focus", post(focus))
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
    use base64::Engine;
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
        /// `settings_changed` 통지로 받은 설정들(저장 순서).
        changed: Arc<std::sync::Mutex<Vec<AppSettings>>>,
    }

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-control-test-{tag}-{}",
            uuid::Uuid::new_v4()
        ))
    }

    fn build(tag: &str) -> Fixture {
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
        let manager = Arc::new(SessionManager::new(
            Arc::new(fac),
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
        let gate = Arc::new(crate::session::inject::InjectGate::new(
            Arc::new(crate::session::inject::ManagerSink::new(manager.clone())),
            Arc::new(crate::state::BotPromptArms::new()),
        ));
        let changed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let ctx = Arc::new(ControlContext {
            manager,
            observer,
            observer_server,
            hub,
            registry,
            store,
            settings,
            settings_store,
            app_data_dir: dir.clone(),
            gate,
            live_usage: Arc::new(crate::usage::LiveUsageState::new()),
            portrait_store: Arc::new(crate::persistence::png_store::PngStore::new(
                dir.join("portraits"),
                crate::persistence::png_store::MAX_PORTRAIT_BYTES,
            )),
            sprite_store: crate::persistence::png_store::PngStore::new(
                dir.join("sprites"),
                crate::persistence::png_store::MAX_SPRITE_BYTES,
            ),
            focus_agent: Arc::new(|_| Ok(())),
            settings_changed: {
                let changed = changed.clone();
                Arc::new(move |s: &AppSettings| changed.lock().unwrap().push(s.clone()))
            },
        });
        let state = ControlServerState::default();
        state.set_app_data_dir(dir.clone());
        Fixture {
            state,
            ctx,
            ctl,
            dir,
            _observer_dir: observer_dir,
            changed,
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

        // 실행 중 세션이라 send 성공 — 관문을 거쳐 PTY stdin에 실린다.
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

    async fn post(port: u16, token: &str, path: &str, body: serde_json::Value) -> serde_json::Value {
        reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/v1/{path}"))
            .header(TOKEN_HEADER, token)
            .json(&body)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    /// 수정 F7(A11·A12): create/send/dispose가 실패를 ok로 보고하지 않는다.
    #[tokio::test]
    async fn create_send_dispose_report_missing_targets_as_failures() {
        let f = build("f7");
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

        // 프로필 없는 id는 PTY를 띄우지 않고 거절한다(숨은 세션 방지).
        let hidden = post(port, &token, "create", serde_json::json!({ "agentId": "ghost" })).await;
        assert_eq!(hidden["ok"], false, "{hidden}");
        assert_eq!(hidden["error"], "unknown_agent");
        assert!(f.ctx.manager.session_id_for("ghost").is_none());

        // 세션이 없으면 send·dispose 모두 not-found.
        let sent = post(port, &token, "send", serde_json::json!({ "agentId": "a1", "data": "x" })).await;
        assert_eq!(sent["error"], "not-found", "{sent}");
        let disposed = post(port, &token, "dispose", serde_json::json!({ "agentId": "nobody" })).await;
        assert_eq!(disposed["error"], "not-found", "{disposed}");

        // 셸이 죽은(Exited) 세션에 send는 not-running이고 stdin에 아무것도 안 간다.
        let created = post(port, &token, "create", serde_json::json!({ "agentId": "a1" })).await;
        assert_eq!(created["ok"], true, "{created}");
        f.ctl.fire_exit_signal(9);
        for _ in 0..200 {
            if !f.ctx.manager.is_running("a1") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let before = f.ctl.writes_raw().len();
        let sent = post(port, &token, "send", serde_json::json!({ "agentId": "a1", "data": "x" })).await;
        assert_eq!(sent["error"], "not-running", "{sent}");
        assert_eq!(f.ctl.writes_raw().len(), before);

        // 끝난 세션도 치울 수는 있고, 같은 id를 두 번 dispose하면 두 번째는 not-found.
        let first = post(port, &token, "dispose", serde_json::json!({ "agentId": "a1" })).await;
        assert_eq!(first["ok"], true, "{first}");
        let second = post(port, &token, "dispose", serde_json::json!({ "agentId": "a1" })).await;
        assert_eq!(second["error"], "not-found", "{second}");
        cleanup(&f);
    }

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

    /// 수정 F6: CLI 변경이 렌더러에 통지되고, 뒤이은 GUI 패치 저장이 그 값을
    /// 되돌리지 않는다.
    #[tokio::test]
    async fn settings_set_notifies_renderer_and_survives_gui_patch() {
        let f = build("settings-sync");
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let token = f.state.issue_token().unwrap();
        assert!(f.ctx.settings.read().unwrap().git_status_enabled);

        let resp: serde_json::Value = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/v1/settings/set"))
            .header(TOKEN_HEADER, &token)
            .json(&serde_json::json!({ "gitStatusEnabled": false }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["ok"], true);
        {
            let changed = f.changed.lock().unwrap();
            assert_eq!(changed.len(), 1);
            assert!(!changed[0].git_status_enabled);
        }

        // GUI는 바꾼 키만 보낸다 — 백엔드가 현재 값에 병합한다.
        let gui_patch = serde_json::json!({ "typingSoundEnabled": false });
        let saved = crate::ipc::commands::apply_settings_patch(
            &f.ctx.settings_store,
            &f.ctx.settings,
            &f.ctx.hub,
            &f.ctx.observer_server,
            &f.ctx.observer,
            f.ctx.settings_changed.as_ref(),
            gui_patch.as_object().unwrap(),
        )
        .await
        .unwrap();
        assert!(!saved.git_status_enabled);
        assert!(!saved.typing_sound_enabled);
        assert_eq!(f.ctx.settings_store.load().0, saved);
        assert_eq!(f.changed.lock().unwrap().last(), Some(&saved));
        cleanup(&f);
    }

    #[tokio::test]
    async fn settings_set_rejects_wrong_types_without_saving_or_notifying() {
        let f = build("settings-bad-type");
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let token = f.state.issue_token().unwrap();
        let before = f.ctx.settings.read().unwrap().clone();

        let resp: serde_json::Value = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/v1/settings/set"))
            .header(TOKEN_HEADER, &token)
            .json(&serde_json::json!({ "gitStatusEnabled": "nope" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["ok"], false);
        assert_eq!(*f.ctx.settings.read().unwrap(), before);
        assert!(f.changed.lock().unwrap().is_empty());
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

    #[tokio::test]
    async fn display_routes_require_auth_validate_revisions_and_never_start_a_session() {
        let f = build("display-api");
        let mut profile = profile("a1", "Ada");
        profile.portrait_updated_at = Some(55);
        f.ctx.store.save(&crate::types::PersistedState {
            agents: vec![profile], version: 1, vacation_mode: None,
        }).unwrap();
        let port = f.state.ensure(f.ctx.clone()).await.unwrap();
        let client = reqwest::Client::new();

        let unauthenticated = client
            .post(format!("http://127.0.0.1:{port}/v1/display/characters"))
            .json(&serde_json::json!({}))
            .send().await.unwrap();
        assert_eq!(unauthenticated.status(), reqwest::StatusCode::UNAUTHORIZED);

        let token = f.state.issue_token().unwrap();
        let capabilities: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/display/capabilities"))
            .header(TOKEN_HEADER, &token).json(&serde_json::json!({}))
            .send().await.unwrap().json().await.unwrap();
        assert!(capabilities["data"]["features"].as_array().unwrap()
            .iter().any(|feature| feature == "portrait"));
        assert!(!capabilities["data"]["features"].as_array().unwrap()
            .iter().any(|feature| feature == "ide-session-focus"));
        let characters: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/display/characters"))
            .header(TOKEN_HEADER, &token).json(&serde_json::json!({}))
            .send().await.unwrap().json().await.unwrap();
        assert_eq!(characters["ok"], true);
        assert_eq!(characters["data"]["characters"][0]["agentId"], "a1");
        assert_eq!(characters["data"]["characters"][0]["portraitUpdatedAt"], 55);
        let revision = characters["data"]["characters"][0]["revision"].as_str().unwrap();

        let stale: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/display/appearance"))
            .header(TOKEN_HEADER, &token)
            .json(&serde_json::json!({ "agentId": "a1", "revision": "stale" }))
            .send().await.unwrap().json().await.unwrap();
        assert_eq!(stale["error"], "appearance_revision_mismatch");

        let unknown: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/display/focus"))
            .header(TOKEN_HEADER, &token).json(&serde_json::json!({ "agentId": "missing" }))
            .send().await.unwrap().json().await.unwrap();
        assert_eq!(unknown["error"], "unknown_agent");

        let focus: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/display/focus"))
            .header(TOKEN_HEADER, &token).json(&serde_json::json!({ "agentId": "a1" }))
            .send().await.unwrap().json().await.unwrap();
        assert_eq!(focus["data"]["focused"], true);
        assert!(f.ctx.manager.session_id_for("a1").is_none());

        // IDE 연결 대화상자는 제거됐다. 예전 확장이 보내는 intent는 무시하고
        // 프로필 선택으로만 처리한다(와이어 호환).
        let connect_codex: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/display/focus"))
            .header(TOKEN_HEADER, &token).json(&serde_json::json!({ "agentId": "a1", "intent": "connectCodex" }))
            .send().await.unwrap().json().await.unwrap();
        assert_eq!(connect_codex["data"]["focused"], true);

        let appearance: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/display/appearance"))
            .header(TOKEN_HEADER, &token).json(&serde_json::json!({ "agentId": "a1", "revision": revision }))
            .send().await.unwrap().json().await.unwrap();
        assert_eq!(appearance["ok"], true);
        assert!(appearance["data"]["pngBase64"].is_null());

        let mut portrait = vec![0; 33];
        portrait[..8].copy_from_slice(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
        portrait[8..12].copy_from_slice(&13u32.to_be_bytes());
        portrait[12..16].copy_from_slice(b"IHDR");
        portrait[16..20].copy_from_slice(&240u32.to_be_bytes());
        portrait[20..24].copy_from_slice(&320u32.to_be_bytes());
        let portrait = base64::engine::general_purpose::STANDARD.encode(portrait);
        f.ctx.portrait_store.save("a1", &portrait, &["a1".into()]).unwrap();
        let portrait_response: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/v1/display/appearance"))
            .header(TOKEN_HEADER, &token)
            .json(&serde_json::json!({ "agentId": "a1", "revision": revision, "kind": "portrait" }))
            .send().await.unwrap().json().await.unwrap();
        assert_eq!(portrait_response["data"]["pngBase64"], portrait);
        assert_eq!(portrait_response["data"]["frameCount"], 1);
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
            default_agent: None,
        }
    }
}
