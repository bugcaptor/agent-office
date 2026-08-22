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

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use axum::extract::{Json, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use serde::Serialize;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::notification::hub::NotificationHub;
use crate::observer::server::ObserverServerState;
use crate::observer::ObserverRuntime;
use crate::persistence::profile_store::ProfileStore;
use crate::persistence::settings_store::{AppSettings, SettingsStore};
use crate::session::external::ExternalDetachReason;
use crate::session::manager::SessionManager;
use crate::session_events::types::AgentEventProfile;
use crate::state::SessionRegistry;
use crate::types::{CreateSessionRequest, SessionState};

use protocol::*;

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
    /// `attach --tmux`가 대상 세션 존재를 확인할 때 쓰는 확인기.
    /// 프로덕션은 `tmux::system_probe()`, 테스트는 가짜를 주입한다.
    pub tmux_probe: tmux::TmuxProbe,
}

impl ControlContext {
    fn read_token(&self) -> Option<String> {
        read_token_at(&self.app_data_dir)
    }
}

// ── 토큰/포트 파일 헬퍼(승인 커맨드와 서버가 공유) ─────────────────────

pub fn read_token_at(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(TOKEN_FILE)).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 새 토큰을 발급하고 `control-token`(0600)에 기록한다. 승인 시 호출.
pub fn issue_token_at(dir: &Path) -> std::io::Result<String> {
    std::fs::create_dir_all(dir)?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let path = dir.join(TOKEN_FILE);
    std::fs::write(&path, &token)?;
    set_owner_only(&path);
    Ok(token)
}

/// 승인 취소 — 토큰 파일을 지운다. 없으면 무해한 no-op. 이후 모든 요청 401.
pub fn revoke_token_at(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(dir.join(TOKEN_FILE)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(unix)]
fn set_owner_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn set_owner_only(_path: &Path) {}

/// 타이밍 부채널을 줄이는 상수시간 비교(길이는 고정이라 누설 무해).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn session_state_str(state: SessionState) -> &'static str {
    match state {
        SessionState::Starting => "starting",
        SessionState::Running => "running",
        SessionState::Exited => "exited",
        SessionState::Disposed => "disposed",
    }
}

/// catch_unwind 페이로드에서 사람이 읽을 메시지를 뽑는다(commands.rs와 동일).
fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

// ── 응답 봉투 ────────────────────────────────────────────────────────

fn ok<T: Serialize>(data: T) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "data": data }))
}
fn fail(msg: impl Into<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": false, "error": msg.into() }))
}

// ── 인증 미들웨어 ────────────────────────────────────────────────────

async fn auth(State(ctx): State<Arc<ControlContext>>, req: Request, next: Next) -> Response {
    let presented = req
        .headers()
        .get(TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let authorized = match (presented, ctx.read_token()) {
        (Some(p), Some(expected)) => ct_eq(p.as_bytes(), expected.as_bytes()),
        _ => false,
    };
    if authorized {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            fail("unauthorized: 앱 설정에서 CLI 제어를 승인했는지 확인하세요"),
        )
            .into_response()
    }
}

// ── 핸들러(기존 command 본문 재사용) ─────────────────────────────────

async fn ping(State(ctx): State<Arc<ControlContext>>) -> Json<serde_json::Value> {
    let profiles = ctx.store.load();
    let running = ctx
        .registry
        .snapshot()
        .into_iter()
        .filter(|(_, _, s)| matches!(s, SessionState::Running | SessionState::Starting))
        .count();
    ok(PingResult {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        agent_count: profiles.agents.len(),
        running_count: running,
    })
}

async fn list(State(ctx): State<Arc<ControlContext>>) -> Json<serde_json::Value> {
    let profiles = ctx.store.load();
    let mut by_agent: std::collections::HashMap<String, (String, SessionState)> =
        std::collections::HashMap::new();
    for (sid, agent, state) in ctx.registry.snapshot() {
        by_agent.insert(agent, (sid, state));
    }
    let entries: Vec<ListEntry> = profiles
        .agents
        .iter()
        .map(|a| {
            let live = by_agent.get(&a.id);
            ListEntry {
                agent_id: a.id.clone(),
                name: a.name.clone(),
                role: a.role.clone(),
                cwd: a.cwd.clone(),
                state: live.map(|(_, s)| session_state_str(*s).to_string()),
                session_id: live.map(|(sid, _)| sid.clone()),
            }
        })
        .collect();
    ok(entries)
}

async fn create(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<CreateParams>,
) -> Json<serde_json::Value> {
    // 스폰 본문은 `ipc::commands::spawn_session` 하나뿐이다 — 여기서 복제하던
    // observer ensure + catch_unwind + create_with_profile를 그쪽으로 합쳤다.
    let profile = AgentEventProfile {
        name: p.name.clone().unwrap_or_else(|| p.agent_id.clone()),
        role: p.role.clone(),
    };
    // 성격 프롬프트는 디스크 프로필에서 읽는다 — 렌더러 create_session이 늘
    // 함께 보내는 값이라, CLI로 띄운 세션만 성격 없이 뜨면 안 된다(M3에서
    // 고친 결함: 예전엔 무조건 None이었다). 프로필이 없으면 기존대로 None.
    let personality_prompt = profile_personality(&ctx, &p.agent_id);
    match crate::ipc::commands::spawn_session(
        &ctx.manager,
        &ctx.observer,
        &ctx.observer_server,
        &ctx.settings,
        CreateSessionRequest {
            agent_id: p.agent_id,
            cols: p.cols,
            rows: p.rows,
            cwd: p.cwd,
            shell: p.shell,
            startup_command: p.startup_command,
            personality_prompt,
            autostart_claude: None,
        },
        profile,
    )
    .await
    {
        Ok(created) => ok(created),
        Err(e) => fail(e),
    }
}

/// 디스크 프로필의 성격 프롬프트(없으면 None). create/attach가 공유한다.
fn profile_personality(ctx: &ControlContext, agent_id: &str) -> Option<String> {
    ctx.store
        .load()
        .agents
        .into_iter()
        .find(|a| a.id == agent_id)
        .and_then(|a| a.personality_prompt)
}

/// 앱 밖 터미널을 캐릭터에 붙인다 — 응답 `script`를 그 터미널에서 eval하면
/// 그 셸에서 뜬 claude의 훅이 이 캐릭터의 알림으로 흐른다.
///
/// create와 같은 골격이다(observer 선기동 + catch_unwind). persona와 표시용
/// 이름/역할은 디스크 프로필에서 읽는다 — 프로필이 없으면 붙일 캐릭터가 없다.
///
/// `tmux`가 오면 전혀 다른 경로다: 논리 세션 대신 **앱이 자기 PTY로 tmux
/// 클라이언트를 여는 일반 세션**을 만든다(control/tmux.rs 헤더 참조).
async fn attach(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<AttachParams>,
) -> Json<serde_json::Value> {
    if ctx.settings.read().unwrap().observer_enabled {
        let _ = ctx.observer_server.ensure(ctx.observer.clone()).await;
    }
    let Some(agent) = ctx
        .store
        .load()
        .agents
        .into_iter()
        .find(|a| a.id == p.agent_id)
    else {
        return fail(format!(
            "알 수 없는 캐릭터입니다: {} — 먼저 앱에서 캐릭터를 만들거나 `agent-office ctl create {}`로 세션을 시작하세요",
            p.agent_id, p.agent_id,
        ));
    };
    let profile = AgentEventProfile {
        name: agent.name.clone(),
        role: Some(agent.role.clone()).filter(|role| !role.is_empty()),
    };

    // tmux 모드: 논리 세션을 만들지 않고 `exec tmux attach-session`으로 도는
    // 일반 PTY 세션을 띄운다. 요청한 셸에는 심을 게 없으므로 `pid`(끊김 감지)도
    // 쓰지 않는다 — 세션 수명은 tmux 클라이언트의 종료(on_exit)가 결정한다.
    if let Some(raw_target) = p.tmux.as_deref() {
        let target = match tmux::validate_target(raw_target) {
            Ok(target) => target,
            Err(e) => return fail(e),
        };
        match (ctx.tmux_probe)(&target) {
            tmux::TmuxStatus::Present => {}
            tmux::TmuxStatus::Missing => {
                return fail(format!(
                    "tmux 세션 '{target}'을 찾을 수 없습니다 — `tmux ls`로 이름을 확인하세요"
                ))
            }
            tmux::TmuxStatus::Unavailable(why) => {
                return fail(format!(
                    "tmux를 실행할 수 없습니다(설치되지 않았거나 앱의 PATH에 없습니다): {why}"
                ))
            }
        }
        let manager = ctx.manager.clone();
        let startup_command = tmux::attach_command(&target);
        let personality_prompt = agent.personality_prompt.clone();
        let agent_id = p.agent_id.clone();
        let cwd = p.cwd.clone();
        // create는 살아 있는 세션이 있으면 새로 띄우지 않고 **재사용**한다 —
        // 그러면 tmux 클라이언트는 뜨지 않으므로 성공으로 위장하면 안 된다.
        // 돌아온 sid가 원래 있던 sid와 같은지로 정확히 가려낸다.
        let existing = ctx.manager.session_id_for(&p.agent_id);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            manager.create_with_profile(
                CreateSessionRequest {
                    agent_id,
                    // 크기는 렌더러가 터미널을 붙일 때 resize로 맞춘다(기본 80x24).
                    cols: None,
                    rows: None,
                    cwd,
                    shell: None,
                    startup_command: Some(startup_command),
                    personality_prompt,
                    autostart_claude: None,
                },
                profile,
            )
        }));
        return match result {
            Ok(Ok(created)) if existing.as_deref() == Some(created.session_id.as_str()) => {
                fail(format!(
                    "'{}'에는 이미 세션이 떠 있어 tmux에 붙지 못했습니다 — 탭을 닫거나 \
                     `agent-office ctl dispose {}` 후 다시 시도하세요",
                    p.agent_id, p.agent_id,
                ))
            }
            Ok(Ok(created)) => ok(AttachResult {
                session_id: created.session_id,
                mode: "tmux".to_string(),
                script: crate::session::attach_script::render_tmux_notice(&p.agent_id, &target),
            }),
            Ok(Err(e)) => fail(e),
            Err(panic) => fail(format!(
                "tmux attach 중 내부 오류(panic): {}",
                panic_message(&panic)
            )),
        };
    }

    let manager = ctx.manager.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        manager
            .attach_external_with_profile(
                &p.agent_id,
                p.pid,
                p.cwd.as_deref(),
                agent.personality_prompt.as_deref(),
                profile,
            )
            .map(|outcome| AttachResult {
                session_id: outcome.session_id().to_string(),
                mode: if outcome.is_new() { "new" } else { "bind" }.to_string(),
                script: crate::session::attach_script::render_attach_script(
                    &agent.name,
                    outcome.session_id(),
                    outcome.plan(),
                ),
            })
    }));
    match result {
        Ok(Ok(attached)) => ok(attached),
        Ok(Err(e)) => fail(e),
        Err(panic) => fail(format!(
            "attach 중 내부 오류(panic): {}",
            panic_message(&panic)
        )),
    }
}

/// 외부 세션을 끊는다. 붙어 있지 않았으면 `detached: false`(무해한 no-op).
async fn detach(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<DetachParams>,
) -> Json<serde_json::Value> {
    let detached = ctx
        .manager
        .detach_external(&p.agent_id, ExternalDetachReason::Detach);
    ok(DetachResult { detached })
}

async fn send(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<SendParams>,
) -> Json<serde_json::Value> {
    // write_input과 동일 — 존재하지 않는 agentId는 무해한 no-op.
    ctx.manager.write_input(&p.agent_id, &p.data);
    ok(serde_json::Value::Null)
}

async fn dispose(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<AgentParams>,
) -> Json<serde_json::Value> {
    ctx.manager.dispose(&p.agent_id);
    ok(serde_json::Value::Null)
}

async fn notifications(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<AgentParams>,
) -> Json<serde_json::Value> {
    ok(ctx.manager.pending_notifications(&p.agent_id))
}

async fn clear(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<ClearParams>,
) -> Json<serde_json::Value> {
    // clear_notifications과 동일: agentId→sessionId 해석 후 hub.clear.
    if let Some(sid) = ctx.manager.session_id_for(&p.agent_id) {
        ctx.hub.clear(&sid, p.ids);
    }
    ok(serde_json::Value::Null)
}

async fn settings_get(State(ctx): State<Arc<ControlContext>>) -> Json<serde_json::Value> {
    ok(ctx.settings.read().unwrap().clone())
}

async fn settings_set(
    State(ctx): State<Arc<ControlContext>>,
    Json(patch): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let Some(obj) = patch.as_object() else {
        return fail("본문은 JSON 객체여야 합니다");
    };
    // cli_enabled는 CLI로 바꿀 수 없다 — 자기 자신을 켜고/끄는 권한 상승을
    // 막는 보안 결정. GUI에서만 토글한다.
    if obj.contains_key("cliEnabled") || obj.contains_key("cli_enabled") {
        return fail("cliEnabled는 앱 설정에서만 변경할 수 있습니다");
    }
    let current = ctx.settings.read().unwrap().clone();
    let mut merged = match serde_json::to_value(current) {
        Ok(v) => v,
        Err(e) => return fail(e.to_string()),
    };
    if let Some(map) = merged.as_object_mut() {
        for (k, v) in obj {
            map.insert(k.clone(), v.clone());
        }
    }
    let new: AppSettings = match serde_json::from_value(merged) {
        Ok(s) => s,
        Err(e) => return fail(format!("설정 파싱 실패: {e}")),
    };
    match crate::ipc::commands::apply_settings_effects(
        &ctx.settings_store,
        &ctx.settings,
        &ctx.hub,
        &ctx.observer_server,
        &ctx.observer,
        new.clone(),
    )
    .await
    {
        Ok(()) => ok(new),
        Err(e) => fail(e),
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

    fn build_with_tmux(tag: &str, tmux_probe: tmux::TmuxProbe) -> Fixture {
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

    fn profile(id: &str, name: &str) -> crate::types::AgentProfile {
        crate::types::AgentProfile {
            id: id.into(),
            name: name.into(),
            role: "backend".into(),
            note: "".into(),
            seed: "seed".into(),
            created_at: 1,
            desk_index: 0,
            assigned_desk_index: None,
            cwd: None,
            appearance: None,
            portrait_updated_at: None,
            sprite_request: None,
            minimi_request: None,
            sprite_updated_at: None,
            minimi_updated_at: None,
            archetype: None,
            shell: None,
            startup_command: None,
            personality_prompt: None,
            clocked_out: None,
            keyboard_sound: None,
            voice_id: None,
            bot: None,
        }
    }
}
