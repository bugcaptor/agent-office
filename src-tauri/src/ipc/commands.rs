// src-tauri/src/ipc/commands.rs
//
// The renderer-facing Tauri commands, using the exact invoke names the
// frontend calls. Every command is a thin delegation into
// `SessionManager`/`NotificationHub`/`ProfileStore`/`SettingsStore` — no
// lock is held across an `.await` point. Most bodies have no `.await` at
// all (`async fn` is required by Tauri for commands that take `State`);
// the exceptions (`summarize_text`, `generate_sprite_image`,
// `set_app_settings`) hold no lock when they yield.
//
// The `State<'_, AppState>` parameter is named `app_state` everywhere
// (not `state`) so it never collides with the `state: PersistedState`
// payload parameter on `save_state` -- Tauri's IPC argument binding matches
// JS argument keys to Rust parameter names, so a name collision there would
// silently break `save_state`'s payload mapping.

use tauri::{ipc::Channel, AppHandle, Manager, State};

use crate::persistence::settings_store::AppSettings;
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

fn event_profile(agent_id: &str, opts: &SessionOpts) -> AgentEventProfile {
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

async fn create_session_inner(
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

#[tauri::command(rename_all = "camelCase")]
pub async fn load_state(app_state: State<'_, AppState>) -> Result<PersistedState, String> {
    Ok(app_state.store.load())
}

/// 주의: Tauri `State` 파라미터는 `app_state`, JS 페이로드 `{ state }`는
/// `state` 파라미터로 받는다 (이름 충돌 회피 -- JS 인자 키와 Rust 파라미터명이
/// 일치해야 매핑된다).
#[tauri::command(rename_all = "camelCase")]
pub async fn save_state(
    app_state: State<'_, AppState>,
    state: PersistedState,
) -> Result<(), String> {
    app_state.store.save(&state).map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn set_badge_count(app: AppHandle, count: i64) -> Result<(), String> {
    // Verified against the installed tauri = 2.11.5 source
    // (src/webview/webview_window.rs, src/window/mod.rs): the design's guess
    // matches exactly. `WebviewWindow::set_badge_count(&self, count:
    // Option<i64>) -> tauri::Result<()>` (it just delegates to
    // `Window::set_badge_count`) -- no `AppHandle`/`Window`-level badge
    // method exists, so we must fetch the window first. `None` (or `0`,
    // which we normalize to `None`) clears the badge. Cross-platform: a
    // no-op on Windows/Android at runtime (doc comment says "Unsupported"
    // there), so no `#[cfg(target_os = ...)]` gate is needed here.
    if let Some(win) = app.get_webview_window("main") {
        win.set_badge_count(if count > 0 { Some(count) } else { None })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn save_portrait(
    app_state: State<'_, AppState>,
    agent_id: String,
    png_base64: String,
) -> Result<(), String> {
    let ids: Vec<String> = app_state
        .store
        .load()
        .agents
        .iter()
        .map(|a| a.id.clone())
        .collect();
    app_state
        .portrait_store
        .save(&agent_id, &png_base64, &ids)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn load_portrait(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<Option<String>, String> {
    app_state
        .portrait_store
        .load(&agent_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn delete_portrait(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<(), String> {
    app_state
        .portrait_store
        .delete(&agent_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn save_sprite(
    app_state: State<'_, AppState>,
    agent_id: String,
    png_base64: String,
) -> Result<(), String> {
    let ids: Vec<String> = app_state
        .store
        .load()
        .agents
        .iter()
        .map(|a| a.id.clone())
        .collect();
    app_state
        .sprite_store
        .save(&agent_id, &png_base64, &ids)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn load_sprite(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<Option<String>, String> {
    app_state
        .sprite_store
        .load(&agent_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn delete_sprite(app_state: State<'_, AppState>, agent_id: String) -> Result<(), String> {
    app_state
        .sprite_store
        .delete(&agent_id)
        .map_err(|e| e.to_string())
}

/// 머리 위 라벨 요약: 요청 시작 시 렌더러가 캡처한 provider의 로컬 CLI를
/// 호출한다. 유저 크레딧을 소모하므로 opt-in — 설정 OFF면
/// "summarizer-disabled"로 거절하고 렌더러가 원문 폴백으로 처리한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn summarize_text(
    app_state: State<'_, AppState>,
    provider: crate::persistence::settings_store::SummaryProvider,
    instruction: String,
    text: String,
    // 목적별 타임아웃(#66). 미전달(구 렌더러)이면 라벨(20초)로 취급.
    purpose: Option<crate::summarizer::SummaryPurpose>,
) -> Result<String, String> {
    let enabled = app_state.settings.read().unwrap().summarizer_enabled;
    if !enabled {
        return Err("summarizer-disabled".to_string());
    }
    crate::summarizer::summarize(provider, purpose.unwrap_or_default(), &instruction, &text).await
}

/// PixelLab로 64×64 스프라이트 1장 생성. AppState 비의존
/// (stateless) — 이 command만은 본문에 .await가 있으나 락을 전혀 잡지
/// 않으므로 파일 머리말의 "no lock across await" 계약과 무관하다.
#[tauri::command(rename_all = "camelCase")]
pub async fn generate_sprite_image(
    description: String,
) -> Result<crate::pixellab::GeneratedImage, String> {
    let trimmed = description.trim();
    if trimmed.is_empty() {
        return Err("validation: description is empty".to_string());
    }
    // pixen maxLength 2000 — 초과분은 뒤를 자른다 (char 경계 안전).
    let capped: String = trimmed
        .chars()
        .take(crate::pixellab::DESCRIPTION_MAX_CHARS)
        .collect();
    crate::pixellab::generate_image(&capped)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAppSettingsResult {
    pub settings: AppSettings,
    pub first_run: bool,
}

/// CLI 제어(#55) 상태 — 설정 다이얼로그의 2단계 승인 UI가 표시한다.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlStatus {
    /// 설정 `cli_enabled`(서버 기동 대상 여부).
    pub enabled: bool,
    /// control 서버가 실제로 떠 있는지.
    pub running: bool,
    /// 승인됨(토큰 발급됨) 여부.
    pub approved: bool,
    /// 현재 바인딩된 포트(서버가 떠 있을 때만).
    pub port: Option<u16>,
    /// 연결 안내에 쓰는 app_data 경로.
    pub app_data_dir: String,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_app_settings(
    app_state: State<'_, AppState>,
) -> Result<GetAppSettingsResult, String> {
    Ok(GetAppSettingsResult {
        settings: *app_state.settings.read().unwrap(),
        first_run: app_state
            .settings_first_run
            .load(std::sync::atomic::Ordering::SeqCst),
    })
}

/// 저장 + 캐시 갱신. Observer OFF→ON이면 서버를 지연 기동한다(이미 떠 있으면
/// 재사용). ON→OFF는 서버 프로세스 자체를 내리지 않는다 — 이미 떠 있는
/// 세션들의 훅 POST는 계속 수신된다. 다만 캐시가 OFF로 갱신된 뒤로는
/// lib.rs의 observer URL getter가 (서버가 살아있어도) None을 돌려주므로, 이
/// 시점 이후 새로 만드는 세션에는 훅 배선(--settings·env·ZDOTDIR)이 전혀
/// 주입되지 않는다 -- "변경은 새 세션부터 적용" 정책의 실제 동작.
#[tauri::command(rename_all = "camelCase")]
pub async fn set_app_settings(
    app_state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<(), String> {
    set_app_settings_inner(&app_state, settings).await
}

async fn set_app_settings_inner(app_state: &AppState, settings: AppSettings) -> Result<(), String> {
    apply_settings_effects(
        &app_state.settings_store,
        &app_state.settings,
        &app_state.hub,
        &app_state.observer_server,
        &app_state.observer,
        settings,
    )
    .await?;
    app_state
        .settings_first_run
        .store(false, std::sync::atomic::Ordering::SeqCst);

    // CLI 제어 서버(#55) lifecycle: cli_enabled 토글에 따라 기동/정지. 켜면
    // control-port가 기록되고, 끄면 서버를 내리고 포트 파일을 지운다(토큰은
    // 유지 — 재활성화 시 재승인 불필요). GUI에서만 이 토글을 조작한다.
    if settings.cli_enabled {
        let _ = app_state
            .control_server
            .ensure(app_state.control_ctx.clone())
            .await;
    } else {
        app_state.control_server.shutdown();
    }
    Ok(())
}

/// 설정 저장의 공통 부수효과 — 디스크 저장 + 캐시 갱신 + 홀드 시간 반영 +
/// observer 서버 지연 기동. Tauri 커맨드(`set_app_settings_inner`)와 CLI 제어
/// 핸들러(`control::settings_set`)가 공유한다. first_run 플래그와 control 서버
/// lifecycle은 호출자별로 달라 여기서 다루지 않는다.
pub(crate) async fn apply_settings_effects(
    settings_store: &crate::persistence::settings_store::SettingsStore,
    settings_cache: &std::sync::RwLock<AppSettings>,
    hub: &crate::notification::hub::NotificationHub,
    observer_server: &crate::observer::server::ObserverServerState,
    observer: &std::sync::Arc<crate::observer::ObserverRuntime>,
    settings: AppSettings,
) -> Result<(), String> {
    // write 가드를 먼저 잡고 쥔 채 저장(동기, await 없음) 후 캐시를 갱신한다 --
    // 그래야 두 호출이 겹쳐도 "디스크에 쓴 값"과 "캐시에 남는 값"이 서로 다른
    // 호출 것이 되는 경합이 없다. 가드는 .await 지점 전에 스코프를 벗어난다
    // (no-lock-across-await 계약 유지).
    {
        let mut guard = settings_cache.write().unwrap();
        settings_store.save(&settings).map_err(|e| e.to_string())?;
        *guard = settings;
    }
    // 이슈 #41: 질문 알림 홀드 시간 변경을 즉시 hub 에 반영한다.
    hub.set_hold_duration(std::time::Duration::from_millis(settings.attention_hold_ms));
    if settings.observer_enabled {
        let _ = observer_server.ensure(observer.clone()).await;
    }
    Ok(())
}

/// CLI 제어(#55) 상태를 렌더러 설정 UI에 보고한다. 서버 기동 여부·승인 여부·
/// 포트·app_data 경로. 설정 다이얼로그의 2단계 승인 UI가 이걸 폴링/조회한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn control_status(app_state: State<'_, AppState>) -> Result<ControlStatus, String> {
    Ok(ControlStatus {
        enabled: app_state.settings.read().unwrap().cli_enabled,
        running: app_state.control_server.is_running(),
        approved: app_state.control_server.is_approved(),
        port: app_state.control_server.current_port(),
        app_data_dir: app_state.control_ctx.app_data_dir.to_string_lossy().into_owned(),
    })
}

/// CLI 제어를 **명시적 승인**한다(2단계 옵트인의 2단계) — 새 토큰을 발급하고
/// `control-token`(0600)에 기록한다. 이후 CLI 요청이 인증된다. cli_enabled가
/// 꺼져 있으면 서버가 없으므로 승인만으로는 동작하지 않는다(안내는 UI에서).
#[tauri::command(rename_all = "camelCase")]
pub async fn control_approve(app_state: State<'_, AppState>) -> Result<(), String> {
    app_state.control_server.issue_token().map(|_| ())
}

/// 승인 취소 — 토큰을 폐기한다. 이후 모든 CLI 요청이 401.
#[tauri::command(rename_all = "camelCase")]
pub async fn control_revoke(app_state: State<'_, AppState>) -> Result<(), String> {
    app_state.control_server.revoke_token()
}

/// 이 탭의 봇 모드를 시작한다(#57). 폴링 태스크를 띄우고 즉시 초기 상태를
/// 반환한다 — 저장소 slug 감지·tea 계정 확인은 태스크가 비동기로 하므로 실패는
/// 이후 `bot_status`의 `error`로 드러난다. 렌더러는 이와 별개로 로컬 키 입력을
/// 잠근다(봇과 사람이 같은 stdin을 두드리는 혼선 방지).
#[tauri::command(rename_all = "camelCase")]
pub async fn bot_start(
    agent_id: String,
    app_state: State<'_, AppState>,
) -> Result<BotAgentStatus, String> {
    Ok(app_state.bot_runtime.start(app_state.bot_ctx.clone(), agent_id))
}

/// 이 탭의 봇 모드를 중단한다 — 폴링 태스크를 내린다. 로컬 조작 복귀는 렌더러가
/// 입력 잠금을 풀어 처리한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn bot_stop(agent_id: String, app_state: State<'_, AppState>) -> Result<(), String> {
    app_state.bot_runtime.stop(&agent_id);
    Ok(())
}

/// 봇 모드가 켜진 탭들의 상태 스냅샷(#57). 렌더러가 폴링해 "봇 운전 중" 배지와
/// 현재 이슈·오류를 표시한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn bot_status(app_state: State<'_, AppState>) -> Result<BotStatus, String> {
    Ok(app_state.bot_runtime.status())
}

/// 에이전트 작업 폴더를 Visual Studio Code로 연다. `path`는 렌더러가
/// 프로필의 `cwd`를 그대로 전달한다(미설정 시 메뉴가 비활성화되므로 폴백
/// 없음). 시작 폴더 UI가 `~/dev/foo`류 입력을 허용하므로 세션 생성과
/// 동일한 틸드 확장을 거친다. 구현/OS별 실행 전략은 `crate::vscode` 참조.
#[tauri::command(rename_all = "camelCase")]
pub async fn open_in_vscode(path: String) -> Result<(), String> {
    crate::vscode::open_dir_in_vscode(&crate::session::manager::expand_tilde(path))
}

/// 에이전트 작업 폴더를 외부 터미널 앱으로 연다. 전달/확장 규칙은
/// `open_in_vscode`와 동일. 어떤 앱을 쓸지는 앱 설정 `externalTerminal`
/// (macOS 전용 — Terminal.app/iTerm)을 따른다. 구현/OS별 실행 전략은
/// `crate::terminal` 참조.
#[tauri::command(rename_all = "camelCase")]
pub async fn open_in_terminal(
    app_state: State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let prefer_iterm = matches!(
        app_state.settings.read().unwrap().external_terminal,
        crate::persistence::settings_store::ExternalTerminal::Iterm
    );
    crate::terminal::open_dir_in_terminal(
        &crate::session::manager::expand_tilde(path),
        prefer_iterm,
    )
}

/// 이슈 #42: 셸 출력(터미널 버퍼 plain text)을 임시 .txt 파일로 쓰고 사용자가
/// 설정한 외부 에디터로 연다. `content`는 렌더러(TerminalRegistry.getPlainText)가
/// 추출한 현재 화면(스크롤백 포함), `agent_name`은 파일명에 쓸 표시 이름이다.
/// 어떤 에디터를 쓸지는 앱 설정 `externalEditor`(system/vscode)를 따른다.
/// 성공 시 쓴 파일의 절대 경로 문자열을 돌려준다. 구현은 `crate::shell_export`.
#[tauri::command(rename_all = "camelCase")]
pub async fn export_terminal_output(
    app_state: State<'_, AppState>,
    agent_name: String,
    content: String,
) -> Result<String, String> {
    // 파일명 충돌 없이 매번 새 파일 -- 초 단위 timestamp를 파일명에 넣는다.
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let file = crate::shell_export::write_export_file(&agent_name, &content, &timestamp)?;

    // 설정 read 가드는 에디터 프로세스(블로킹 status 대기)를 실행하기 전에
    // 드롭한다 -- 실행이 길어져도 설정 락을 쥐고 있지 않도록.
    let use_vscode = {
        let guard = app_state.settings.read().unwrap();
        matches!(
            guard.external_editor,
            crate::persistence::settings_store::ExternalEditor::Vscode
        )
    };
    crate::shell_export::open_file_in_editor(&file, use_vscode)?;
    Ok(file.to_string_lossy().into_owned())
}

/// 네이티브 폴더 선택 다이얼로그를 띄운다. 사용자가 고른 절대 경로,
/// 취소 시 None. `initial_dir`이 (틸드 확장 후) 실존 디렉터리면 거기서
/// 시작한다 — 아니면 OS 기본 위치. 다이얼로그 표시의 메인 스레드 디스패치는
/// tauri-plugin-dialog가 처리하므로 async 커맨드 스레드에서 안전하다.
#[tauri::command(rename_all = "camelCase")]
pub async fn pick_directory(
    app: tauri::AppHandle,
    initial_dir: Option<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let mut builder = app.dialog().file();
    if let Some(dir) = initial_dir {
        let expanded = crate::session::manager::expand_tilde(dir);
        if std::path::Path::new(&expanded).is_dir() {
            builder = builder.set_directory(expanded);
        }
    }

    // 콜백 → oneshot 브리지: blocking_pick_folder는 async 런타임 스레드를
    // 다이얼로그가 닫힐 때까지 점유하므로 쓰지 않는다.
    let (tx, rx) = tokio::sync::oneshot::channel();
    builder.pick_folder(move |folder| {
        let _ = tx.send(folder);
    });
    let picked = rx
        .await
        .map_err(|_| "폴더 선택 다이얼로그가 응답 없이 종료되었습니다".to_string())?;
    match picked {
        None => Ok(None),
        Some(fp) => Ok(Some(
            fp.into_path()
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .into_owned(),
        )),
    }
}

/// 완료된 턴 1건을 로컬 시계열 로그(session-times.jsonl)에 append.
#[tauri::command(rename_all = "camelCase")]
pub async fn append_session_turn(
    app_state: State<'_, AppState>,
    record: crate::types::SessionTurnRecord,
) -> Result<(), String> {
    app_state
        .session_time_store
        .append(&record)
        .map_err(|e| e.to_string())
}

/// 누적된 세션 턴 기록 전체를 읽는다(통계용).
#[tauri::command(rename_all = "camelCase")]
pub async fn load_session_turns(
    app_state: State<'_, AppState>,
) -> Result<Vec<crate::types::SessionTurnRecord>, String> {
    Ok(app_state.session_time_store.load())
}

/// 캐릭터 일기(#56) 한 편을 per-agent 로그(`diaries/<agentId>.jsonl`)에 append.
/// 본문 생성은 렌더러가 `summarize_text`로 이미 마친 상태 — 여기선 저장만 한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn append_diary_entry(
    app_state: State<'_, AppState>,
    agent_id: String,
    entry: crate::types::DiaryEntry,
) -> Result<(), String> {
    app_state
        .diary_store
        .append(&agent_id, &entry)
        .map_err(|e| e.to_string())
}

/// 한 캐릭터의 일기 전체(작성순)를 읽는다(열람 오버레이용).
#[tauri::command(rename_all = "camelCase")]
pub async fn load_diary(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<Vec<crate::types::DiaryEntry>, String> {
    app_state
        .diary_store
        .load(&agent_id)
        .map_err(|e| e.to_string())
}

/// 한 캐릭터의 작업 로그 버퍼 전체를 스냅샷 저장한다(#60). 렌더러가 버퍼 변경 시
/// 디바운스로 호출한다. `items`가 비면 스냅샷 파일을 삭제한다(일기화로 소진된 캐릭터).
#[tauri::command(rename_all = "camelCase")]
pub async fn save_work_log(
    app_state: State<'_, AppState>,
    agent_id: String,
    items: Vec<crate::types::WorkLogItem>,
) -> Result<(), String> {
    app_state
        .work_log_store
        .save(&agent_id, &items)
        .map_err(|e| e.to_string())
}

/// 전 캐릭터의 작업 로그 스냅샷을 읽는다(부팅 복원용). `agentId -> items` 맵.
/// 손상/부재는 조용히 건너뛰므로 항상 성공한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn load_work_logs(
    app_state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, Vec<crate::types::WorkLogItem>>, String> {
    Ok(app_state.work_log_store.load_all())
}

/// 세션 이벤트 시계열에서 `from_at..=to_at`(epoch ms) 범위를 읽는다(분석 패널용).
/// 읽기 전용 — 수집 측 `SessionEventStore`는 건드리지 않는다
/// (docs/session-analytics-design.md §4.2). reader가 없는 파일·손상 줄을
/// 건너뛰므로 반환은 항상 성공한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn load_session_events(
    app_state: State<'_, AppState>,
    from_at: u64,
    to_at: u64,
) -> Result<Vec<crate::session_events::types::SessionEventRecord>, String> {
    Ok(crate::session_events::reader::load_session_events(
        &app_state.session_event_root,
        from_at,
        to_at,
    ))
}

/// 에이전트별 최신 Claude native 세션(리줌) 스냅샷 전체를 읽는다(이어하기
/// 메뉴용). 렌더러는 메뉴를 열 때만 조회하므로 이벤트 푸시가 필요 없다
/// (docs/claude-session-resume-design.md §5). 반환은 항상 성공.
#[tauri::command(rename_all = "camelCase")]
pub async fn list_claude_resume_sessions(
    app_state: State<'_, AppState>,
) -> Result<
    std::collections::HashMap<
        String,
        crate::persistence::claude_resume_store::ClaudeResumeEntry,
    >,
    String,
> {
    Ok(app_state.claude_resume_store.load_all())
}

/// 사용량 소스 루트 결정의 순수 계산부. 전역 `std::env::var` 접근과
/// 분리해 두어야 단위 테스트에서 프로세스 전역 env에 손대지 않고 조합을
/// 검증할 수 있다(docs/usage-limits-design.md §2). 실제 CLI가 존중하는
/// 표준 오버라이드를 그대로 따른다: Codex는 `CODEX_HOME`(설정되면
/// `<CODEX_HOME>/sessions`를 읽음), Claude는 `CLAUDE_CONFIG_DIR`(설정되면
/// `<CLAUDE_CONFIG_DIR>/.claude.json`을 읽음 -- claude.rs::load의 파일명
/// 결합 로직은 그대로 두고 루트만 바꾼다). 빈 문자열 env는 미설정으로
/// 취급(일부 셸/런처가 unset 대신 빈 문자열을 넘기는 경우 대비).
fn resolve_usage_roots(
    home: &std::path::Path,
    codex_home_env: Option<&str>,
    claude_config_env: Option<&str>,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let codex_root = codex_home_env
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let claude_root = claude_config_env
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.to_path_buf());
    (codex_root, claude_root)
}

/// Claude 자격증명(.credentials.json)·스코프 Keychain의 기준 디렉터리
/// (docs/claude-usage-live-fetch-design.md §2.2). `.claude.json`을 읽는
/// `claude_root`와 다르다: CLAUDE_CONFIG_DIR가 설정되면 그 경로, 미설정이면
/// `~/.claude`(claude_root=홈과 구분됨). 빈 문자열 env는 미설정 취급.
fn resolve_claude_config_dir(
    home: &std::path::Path,
    claude_config_env: Option<&str>,
) -> std::path::PathBuf {
    claude_config_env
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"))
}

/// 구독 사용량(rate limit) 스냅샷을 읽는다(인자 없음,
/// docs/usage-limits-design.md). Claude(`.claude.json`)와 Codex
/// (`sessions/`)를 각각 독립 파싱하므로 한쪽 소스가 실패해도 커맨드는
/// 성공하고 실패한 provider만 `null`이 된다. 상태를 건드리지 않는 순수 읽기라
/// AppState가 필요 없다 -- 루트 경로만 `resolve_usage_roots`로 유도해 usage
/// 모듈에 넘긴다. 기본은 홈 디렉터리 하위(`~/.codex`, `~/.claude.json`)이고,
/// `CODEX_HOME`/`CLAUDE_CONFIG_DIR` 환경변수가 설정돼 있으면 그쪽을 우선한다.
/// 이슈 #33: 실시간 조회를 얹었다. 스로틀 상태(`AppState::live_usage`)를
/// 넘겨 렌더러 60초 폴링에 얹혀 리셋 경계 후 실제 값을 빠르게 반영한다.
/// 실시간 경로가 실패하면 현행과 동일하게 파일 캐시 미러만 반환한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn load_usage_snapshot(
    app_state: State<'_, AppState>,
) -> Result<crate::usage::UsageSnapshot, String> {
    Ok(load_usage_snapshot_body(&app_state.live_usage, chrono::Utc::now().timestamp_millis()).await)
}

/// 커맨드 본체(AppState 없이 호출 가능 — 기존 커맨드 테스트 관례). 전역 env를
/// 여기서만 읽고 순수 리졸버로 경로를 유도한 뒤 usage 모듈에 위임한다.
async fn load_usage_snapshot_body(
    live: &crate::usage::LiveUsageState,
    now_ms: i64,
) -> crate::usage::UsageSnapshot {
    let home = std::path::PathBuf::from(crate::session::manager::home_dir());
    let codex_home_env = std::env::var("CODEX_HOME").ok();
    let claude_config_env = std::env::var("CLAUDE_CONFIG_DIR").ok();
    let (codex_root, claude_root) = resolve_usage_roots(
        &home,
        codex_home_env.as_deref(),
        claude_config_env.as_deref(),
    );
    let claude_config_dir = resolve_claude_config_dir(&home, claude_config_env.as_deref());
    crate::usage::load_usage_snapshot_with_live(
        live,
        &claude_root,
        &claude_config_dir,
        &codex_root,
        now_ms,
    )
    .await
}

#[cfg(test)]
mod tests;
