// src-tauri/src/ipc/commands/session.rs
//
// Session lifecycle commands: create/dispose/handoff/adopt, PTY IO
// (write/resize/subscribe), and the notification queue tied to a live
// session. See `super`(`ipc::commands`) module doc for the shared
// no-lock-across-await contract.

use tauri::{ipc::Channel, State};

use crate::session_events::types::AgentEventProfile;
use crate::state::AppState;
use crate::types::*;

#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOpts {
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub cwd: Option<String>,
    pub shell: Option<String>,
    pub startup_command: Option<String>,
    pub personality_prompt: Option<String>,
    pub agent_name: Option<String>,
    pub agent_role: Option<String>,
}

pub(crate) fn event_profile(agent_id: &str, opts: &SessionOpts) -> AgentEventProfile {
    AgentEventProfile {
        name: opts
            .agent_name
            .clone()
            .unwrap_or_else(|| agent_id.to_string()),
        role: opts.agent_role.clone(),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn create_session(
    app_state: State<'_, AppState>,
    agent_id: String,
    opts: Option<SessionOpts>,
) -> Result<CreateSessionResult, String> {
    create_session_inner(&app_state, agent_id, opts).await
}

pub(crate) async fn create_session_inner(
    app_state: &AppState,
    agent_id: String,
    opts: Option<SessionOpts>,
) -> Result<CreateSessionResult, String> {
    let observer_enabled = app_state.settings.read().unwrap().observer_enabled;
    if observer_enabled {
        let _ = app_state
            .observer_server
            .ensure(app_state.observer.clone())
            .await;
    }
    let o = opts.unwrap_or_default();
    let profile = event_profile(&agent_id, &o);
    // catch_unwind: Tauri에서 커맨드가 패닉하면 invoke 프라미스가 영원히
    // settle되지 않는다 — 프론트는 "starting"에 고착되고 사용자는 앱 재시작
    // 전까지 그 에이전트의 터미널을 못 띄운다(2026-07-11 실사고). create()
    // 내부(스폰/하위 crate)의 잔여 패닉을 Err로 바꿔 프라미스를 반드시
    // settle시키고, 프론트가 exited로 전환해 재시도할 수 있게 한다.
    let manager = app_state.manager.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        manager.create_with_profile(
            CreateSessionRequest {
                agent_id,
                cols: o.cols,
                rows: o.rows,
                cwd: o.cwd,
                shell: o.shell,
                startup_command: o.startup_command,
                personality_prompt: o.personality_prompt,
                autostart_claude: None, // 항상 기본 false (SessionManager::create의 unwrap_or(false))
            },
            profile,
        )
    }));
    result.map_err(|panic| {
        let msg = panic_message(&panic);
        eprintln!("agent-office: create_session panicked: {msg}");
        format!("세션 생성 중 내부 오류(panic): {msg}")
    })?
}

/// `catch_unwind`가 잡은 패닉 페이로드에서 사람이 읽을 메시지를 뽑는다.
/// `create_session`/`handoff_sessions`/`adopt_detached_sessions`가 공유 —
/// Tauri 커맨드가 패닉하면 invoke 프라미스가 영원히 settle되지 않으므로
/// (2026-07-11 실사고), 어떤 커맨드든 내부 패닉을 반드시 Err로 바꿔
/// 돌려줘야 한다.
fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

/// 렌더러 셸 선택 드롭다운용: 호스트에 설치된 Windows 셸 목록(다른
/// 플랫폼은 빈 배열).
#[tauri::command(rename_all = "camelCase")]
pub async fn list_available_shells() -> Result<Vec<crate::session::shells::AvailableShell>, String>
{
    Ok(crate::session::shells::detect_shells())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn dispose_session(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<(), String> {
    app_state.manager.dispose(&agent_id);
    Ok(())
}

/// 세션 핸드오프 기능 지원 여부(docs/session-handoff-design.md). unix
/// 전용 -- Windows는 항상 false(모달은 기존 2버튼 유지).
#[tauri::command(rename_all = "camelCase")]
pub async fn handoff_supported() -> Result<bool, String> {
    Ok(cfg!(unix))
}

/// v2 상시 브로커 모드가 켜져 있는지(docs/session-broker-v2-design.md).
/// 렌더러는 이 값이 true일 때만 주기 스냅샷 업로드를 활성화한다. `handoff_supported`
/// 옆에 additive로 두어 기존 계약을 건드리지 않는다.
#[tauri::command(rename_all = "camelCase")]
pub async fn session_broker_mode(app_state: State<'_, AppState>) -> Result<bool, String> {
    Ok(app_state.manager.broker_mode())
}

/// 브로커 모드 주기 스냅샷 업로드: 렌더러가 30초마다 직렬화한 xterm 화면을
/// agentId 키로 올려 앱 크래시 후 화면 복원에 대비한다. 브로커 모드가 아니거나
/// 데몬에 못 닿으면 no-op이다.
#[tauri::command(rename_all = "camelCase")]
pub async fn upload_session_snapshots(
    app_state: State<'_, AppState>,
    snapshots: std::collections::HashMap<String, String>,
    // §#49: agentId -> 렌더러가 실제 렌더한 raw 스트림 바이트 누적치. 데몬이
    // 스냅샷 offset(=base+이 값) 이후만 리플레이해 유실 창을 없앤다.
    rendered_bytes: std::collections::HashMap<String, u64>,
) -> Result<(), String> {
    app_state.manager.upload_snapshots(&snapshots, &rendered_bytes);
    Ok(())
}

/// 앱 종료 확인 모달에서 "터미널 유지하고 종료" 선택 시 호출. Running
/// 세션들을 sessiond로 넘기고 넘긴 개수를 반환한다 -- 프론트는 이 수와
/// 무관하게 창을 닫고 종료를 진행한다(§핵심 3). 비unix에서는
/// `SessionManager::handoff_all`이 항상 0을 반환하는 no-op이다.
///
/// `snapshots`(agentId -> 직렬화된 xterm 화면)는 실증에서 발견된 빈틈 수정:
/// 데몬은 핸드오프 *이후* 출력만 링버퍼에 담으므로, 종료 직전 화면(예: ls
/// 결과)은 프론트가 xterm SerializeAddon으로 직렬화해 실어 보내지 않으면
/// 재입양 후 사라진다.
#[tauri::command(rename_all = "camelCase")]
pub async fn handoff_sessions(
    app_state: State<'_, AppState>,
    snapshots: std::collections::HashMap<String, String>,
    // §#49: agentId -> 렌더러가 실제 렌더한 raw 스트림 바이트 누적치(스냅샷 offset 계산용).
    rendered_bytes: std::collections::HashMap<String, u64>,
) -> Result<usize, String> {
    let manager = app_state.manager.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        manager.handoff_all(&snapshots, &rendered_bytes)
    }));
    result.map_err(|panic| {
        let msg = panic_message(&panic);
        eprintln!("agent-office: handoff_sessions panicked: {msg}");
        format!("세션 핸드오프 중 내부 오류(panic): {msg}")
    })
}

/// 부트스트랩 시 1회 호출: sessiond에 남아 있는 세션들을 되찾는다(§핵심 4).
/// 영속 프로필에 없는 agentId는 데몬에 Kill 지시되고 반환되지 않는다.
/// 비unix에서는 `SessionManager::adopt_detached`가 항상 빈 벡터를 반환한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn adopt_detached_sessions(
    app_state: State<'_, AppState>,
) -> Result<Vec<AdoptedSessionInfo>, String> {
    let manager = app_state.manager.clone();
    let known_agent_ids: std::collections::HashSet<String> = app_state
        .store
        .load()
        .agents
        .into_iter()
        .map(|a| a.id)
        .collect();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        manager.adopt_detached(&known_agent_ids)
    }));
    result.map_err(|panic| {
        let msg = panic_message(&panic);
        eprintln!("agent-office: adopt_detached_sessions panicked: {msg}");
        format!("세션 입양 중 내부 오류(panic): {msg}")
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn write_input(
    app_state: State<'_, AppState>,
    agent_id: String,
    data: String,
) -> Result<(), String> {
    app_state.manager.write_input(&agent_id, &data);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn resize_session(
    app_state: State<'_, AppState>,
    agent_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    app_state.manager.resize(&agent_id, cols, rows);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn subscribe_output(
    app_state: State<'_, AppState>,
    agent_id: String,
    channel: Channel<OutputChunk>,
) -> Result<(), String> {
    app_state.manager.attach_output(&agent_id, channel);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn unsubscribe_output(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<(), String> {
    app_state.manager.detach_output(&agent_id);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn list_notifications(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<Vec<NotificationEvent>, String> {
    Ok(app_state.manager.pending_notifications(&agent_id))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn clear_notifications(
    app_state: State<'_, AppState>,
    agent_id: String,
    ids: Option<Vec<String>>,
) -> Result<(), String> {
    if let Some(sid) = app_state.manager.session_id_for(&agent_id) {
        app_state.hub.clear(&sid, ids);
    }
    Ok(())
}
