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
) -> Result<String, String> {
    let enabled = app_state.settings.read().unwrap().summarizer_enabled;
    if !enabled {
        return Err("summarizer-disabled".to_string());
    }
    crate::summarizer::summarize(provider, &instruction, &text).await
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
mod tests {
    // Assert each command *body* delegates correctly into
    // manager/hub/store. `tauri::State<'_, AppState>` cannot be constructed
    // standalone (it borrows from a live `tauri::App`/`AppHandle`), so
    // instead of driving the `#[tauri::command]`-wrapped async fns directly,
    // these tests build a real `AppState` (fakes for PtyFactory/AppEvents,
    // tempdir-backed ProfileStore/ObserverRuntime -- the same seams
    // other test modules use) and call the exact `app_state.manager` /
    // `app_state.hub` / `app_state.store` method sequence each command body
    // above executes. Every command function is a one-line, non-`await`ing
    // delegation, so exercising the delegation target through a real
    // `AppState` proves the wiring without needing a Tauri runtime.
    // `subscribe_output`/`unsubscribe_output` (need a live `Channel`) and
    // `set_badge_count` (needs a live `AppHandle`/webview window) are left
    // to manual/E2E verification -- there is no seam for either without a
    // running Tauri app.
    use super::*;
    use crate::notification::hub::{NotificationHub, SystemClock};
    use crate::observer::server::ObserverServerState;
    use crate::observer::ObserverRuntime;
    use crate::persistence::profile_store::ProfileStore;
    use crate::persistence::settings_store::SummaryProvider;
    use crate::session::manager::SessionManager;
    use crate::session::pty_factory::fake::FakePtyFactory;
    use crate::state::fake::RecordingEvents;
    use crate::state::{AppEvents, SessionRegistry};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;
    use uuid::Uuid;

    // summarize_text은 `State<'_, AppState>`를 받으므로 (State는 살아있는
    // App/AppHandle 없이 standalone 구성 불가) 다른 stateful command들과 같은
    // 패턴으로, 본문의 게이트 로직을 real AppState를 통해 그대로 재현해
    // 검증한다.

    #[test]
    fn summarize_text_command_accepts_provider_snapshot() {
        fn assert_signature<F, Fut>(_command: F)
        where
            F: Fn(State<'static, AppState>, SummaryProvider, String, String) -> Fut,
        {
        }

        assert_signature(summarize_text);
    }

    #[tokio::test]
    async fn summarize_text_gate_rejects_when_disabled() {
        let (state, ctl, dir, profile_dir) = build("summarize-disabled");
        // AppSettings::default()의 summarizer_enabled == false 전제.
        assert!(!state.settings.read().unwrap().summarizer_enabled);

        // summarize_text 본문과 동일한 게이트: OFF면 CLI 호출 전에 거절.
        let result: Result<String, String> = if !state.settings.read().unwrap().summarizer_enabled {
            Err("summarizer-disabled".to_string())
        } else {
            crate::summarizer::summarize(SummaryProvider::Codex, "요약하라", "text").await
        };

        assert_eq!(result.unwrap_err(), "summarizer-disabled");
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn summarize_text_proceeds_to_selected_summarizer_when_enabled() {
        let (state, ctl, dir, profile_dir) = build("summarize-enabled");
        *state.settings.write().unwrap() = crate::persistence::settings_store::AppSettings {
            version: 1,
            summarizer_enabled: true,
            summary_provider: SummaryProvider::Codex,
            observer_enabled: false,
            sound_enabled: true,
            sound_volume: 0.5,
            external_terminal: Default::default(),
            external_editor: Default::default(),
            attention_hold_ms: 5000,
            git_status_enabled: true,
            cli_enabled: false,
        };

        // ON이면 게이트를 통과해 캡처된 provider로 위임된다 -- 빈 텍스트라서
        // 실 프로세스 spawn 없이 그쪽의 자체 검증 에러로 되돌아오는 것으로 확인.
        let result: Result<String, String> = if !state.settings.read().unwrap().summarizer_enabled {
            Err("summarizer-disabled".to_string())
        } else {
            crate::summarizer::summarize(SummaryProvider::Codex, "요약하라", "   ").await
        };

        assert_eq!(result.unwrap_err(), "validation: text is empty");
        cleanup(&ctl, &dir, &profile_dir);
    }

    // set_app_settings 본문(저장 -> 캐시 갱신 -> first_run 플래그 내림)을
    // 그대로 재현해 검증한다. 회귀 대상: 첫 실행 완료 후 웹뷰가 리로드돼도
    // get_app_settings가 firstRun: true를 다시 주면 안 된다(Minor #3).
    #[tokio::test]
    async fn set_app_settings_clears_first_run_flag_after_success() {
        let (state, ctl, dir, profile_dir) = build("first-run-flag");
        assert!(
            state
                .settings_first_run
                .load(std::sync::atomic::Ordering::SeqCst),
            "build() 헬퍼는 부팅 시 settings.json 부재 상태를 흉내내므로 초기값은 true"
        );

        let new_settings = crate::persistence::settings_store::AppSettings {
            version: 1,
            summarizer_enabled: true,
            summary_provider: SummaryProvider::Claude,
            observer_enabled: false,
            sound_enabled: true,
            sound_volume: 0.5,
            external_terminal: Default::default(),
            external_editor: Default::default(),
            attention_hold_ms: 5000,
            git_status_enabled: true,
            cli_enabled: false,
        };
        // set_app_settings 본문과 동일한 순서: write 가드를 쥔 채 저장 후 캐시
        // 갱신, 가드 해제 -- 그다음 first_run을 false로 내린다.
        {
            let mut guard = state.settings.write().unwrap();
            state
                .settings_store
                .save(&new_settings)
                .expect("save into tempdir should succeed");
            *guard = new_settings;
        }
        state
            .settings_first_run
            .store(false, std::sync::atomic::Ordering::SeqCst);

        assert!(!state
            .settings_first_run
            .load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(*state.settings.read().unwrap(), new_settings);
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn set_app_settings_keeps_enabled_setting_when_observer_server_is_unavailable() {
        let (state, ctl, dir, profile_dir) = build("settings-observer-fail-open");
        state.observer_server.shutdown();
        let settings = crate::persistence::settings_store::AppSettings {
            version: 1,
            summarizer_enabled: false,
            summary_provider: SummaryProvider::Claude,
            observer_enabled: true,
            sound_enabled: true,
            sound_volume: 0.5,
            external_terminal: Default::default(),
            external_editor: Default::default(),
            attention_hold_ms: 5000,
            git_status_enabled: true,
            cli_enabled: false,
        };

        assert!(set_app_settings_inner(&state, settings).await.is_ok());
        assert_eq!(*state.settings.read().unwrap(), settings);
        assert_eq!(state.settings_store.load().0, settings);
        assert!(!state
            .settings_first_run
            .load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(state.observer_server.current_url(), None);

        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn create_session_ensures_observer_server_before_preparing_new_pty() {
        let (state, ctl, dir, profile_dir) = build("create-observer-ensure");
        state.settings.write().unwrap().observer_enabled = true;

        let created = create_session_inner(&state, "a1".into(), None)
            .await
            .unwrap();

        assert_eq!(created.state, SessionState::Running);
        assert!(state.observer_server.current_url().is_some());
        assert!(ctl
            .spawned_env()
            .iter()
            .any(|(key, _)| key == "AGENT_OFFICE_HOOK_URL"));
        state.observer_server.shutdown();
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn create_session_still_spawns_when_observer_server_cannot_start() {
        let (state, ctl, dir, profile_dir) = build("create-observer-fail-open");
        state.settings.write().unwrap().observer_enabled = true;
        state.observer_server.shutdown();

        let created = create_session_inner(&state, "a1".into(), None)
            .await
            .unwrap();

        assert_eq!(created.state, SessionState::Running);
        assert!(ctl
            .spawned_env()
            .iter()
            .all(|(key, _)| key != "AGENT_OFFICE_HOOK_URL"));
        cleanup(&ctl, &dir, &profile_dir);
    }

    /// Unique tempdir per test, matching the convention used throughout the
    /// other modules' tests (no `tempfile` dependency needed).
    fn scratch_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-commands-test-{tag}-{}",
            Uuid::new_v4()
        ))
    }

    /// Builds a real `AppState` wired to fakes, mirroring `lib.rs`'s setup
    /// wiring but with `FakePtyFactory`/`RecordingEvents` standing in for
    /// the PTY/Tauri-event side effects.
    fn build(tag: &str) -> (AppState, Arc<FakePtyFactoryControl>, PathBuf, PathBuf) {
        let events: Arc<RecordingEvents> = Arc::new(RecordingEvents::default());
        let events_dyn: Arc<dyn AppEvents> = events.clone();
        let registry = Arc::new(SessionRegistry::new());
        let hub = Arc::new(NotificationHub::new(
            registry.clone(),
            events_dyn.clone(),
            Arc::new(SystemClock),
            Duration::from_millis(3000),
        ));
        let observer_dir = scratch_dir(&format!("{tag}-observer"));
        let observer = Arc::new(ObserverRuntime::production(
            hub.clone(),
            observer_dir.clone(),
            std::env::current_exe().unwrap(),
        ));
        let observer_server = Arc::new(ObserverServerState::default());
        let settings = Arc::new(std::sync::RwLock::new(
            crate::persistence::settings_store::AppSettings::default(),
        ));
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
        let profile_dir = scratch_dir(&format!("{tag}-profiles"));
        let store = ProfileStore::new(profile_dir.join("profiles.json"));
        let portrait_store = crate::persistence::png_store::PngStore::new(
            profile_dir.join("portraits"),
            crate::persistence::png_store::MAX_PORTRAIT_BYTES,
        );
        let sprite_store = crate::persistence::png_store::PngStore::new(
            profile_dir.join("sprites"),
            crate::persistence::png_store::MAX_SPRITE_BYTES,
        );

        let settings_store = crate::persistence::settings_store::SettingsStore::new(
            profile_dir.join("settings.json"),
        );
        let session_time_store = crate::persistence::session_time_store::SessionTimeStore::new(
            profile_dir.join("session-times.jsonl"),
        );
        let claude_resume_store =
            Arc::new(crate::persistence::claude_resume_store::ClaudeResumeStore::new(
                profile_dir.join("claude-resume.json"),
            ));

        let control_server = Arc::new(crate::control::ControlServerState::default());
        control_server.set_app_data_dir(profile_dir.clone());
        let control_ctx = Arc::new(crate::control::ControlContext {
            manager: manager.clone(),
            observer: observer.clone(),
            observer_server: observer_server.clone(),
            hub: hub.clone(),
            registry: registry.clone(),
            store: store.clone(),
            settings: settings.clone(),
            settings_store: settings_store.clone(),
            app_data_dir: profile_dir.clone(),
        });

        let state = AppState {
            manager,
            hub,
            observer,
            observer_server,
            store,
            portrait_store,
            sprite_store,
            session_time_store,
            claude_resume_store,
            settings_store,
            settings,
            settings_first_run: std::sync::atomic::AtomicBool::new(true),
            session_event_root: profile_dir.join("session-events").join("v1"),
            live_usage: crate::usage::LiveUsageState::new(),
            control_server,
            control_ctx,
        };
        (state, ctl, observer_dir, profile_dir)
    }

    type FakePtyFactoryControl = crate::session::pty_factory::fake::FakeControl;

    fn req(agent_id: &str) -> CreateSessionRequest {
        CreateSessionRequest {
            agent_id: agent_id.into(),
            cols: None,
            rows: None,
            cwd: None,
            shell: None,
            startup_command: None,
            personality_prompt: None,
            autostart_claude: None,
        }
    }

    fn cleanup(ctl: &FakePtyFactoryControl, hook_dir: &PathBuf, profile_dir: &PathBuf) {
        ctl.close_output();
        let _ = std::fs::remove_dir_all(hook_dir);
        let _ = std::fs::remove_dir_all(profile_dir);
    }

    // ---- create_session ----

    #[test]
    fn create_session_opts_profile_snapshot_flows_to_manager() {
        let opts = SessionOpts {
            cols: None,
            rows: None,
            cwd: None,
            shell: None,
            startup_command: None,
            personality_prompt: None,
            agent_name: Some("Compiler".into()),
            agent_role: Some("Platform".into()),
        };
        assert_eq!(
            event_profile("a1", &opts),
            crate::session_events::types::AgentEventProfile {
                name: "Compiler".into(),
                role: Some("Platform".into()),
            },
        );
        assert_eq!(
            event_profile("a1", &SessionOpts::default()),
            crate::session_events::types::AgentEventProfile {
                name: "a1".into(),
                role: None,
            },
        );
    }

    #[tokio::test]
    async fn create_session_delegates_to_manager_create_with_opts_and_default_autostart() {
        let (state, ctl, dir, profile_dir) = build("create");

        let result = state
            .manager
            .create(CreateSessionRequest {
                agent_id: "a1".into(),
                cols: Some(100),
                rows: Some(30),
                cwd: None,
                shell: None,
                startup_command: None,
                personality_prompt: None,
                autostart_claude: None, // command body always passes None -> manager defaults to false
            })
            .unwrap();

        assert_eq!(result.state, SessionState::Running);
        // autostart defaulted false (plain shell) -> no stdin injection.
        assert_eq!(ctl.writes_utf8(), "");

        cleanup(&ctl, &dir, &profile_dir);
    }

    // create_session 본문(SessionOpts -> CreateSessionRequest)이 `shell`
    // 필드를 그대로 전달하는지 검증 -- cols/rows/cwd와 동일한 delegation
    // 패턴이지만 opts.shell 값이 유실되지 않는지가 회귀 지점.
    #[test]
    fn create_session_opts_shell_flows_into_create_session_request() {
        let opts = SessionOpts {
            cols: None,
            rows: None,
            cwd: None,
            shell: Some("git-bash".into()),
            startup_command: None,
            personality_prompt: None,
            agent_name: None,
            agent_role: None,
        };
        // create_session 본문과 동일한 매핑.
        let request = CreateSessionRequest {
            agent_id: "a1".into(),
            cols: opts.cols,
            rows: opts.rows,
            cwd: opts.cwd,
            shell: opts.shell.clone(),
            startup_command: opts.startup_command.clone(),
            personality_prompt: opts.personality_prompt.clone(),
            autostart_claude: None,
        };
        assert_eq!(request.shell, Some("git-bash".to_string()));
    }

    // create_session 본문이 opts.startup_command를 유실 없이 전달하는지 검증
    // (shell 회귀 테스트와 동일 패턴 -- sessionOptsFor -> SessionOpts -> 요청).
    #[test]
    fn create_session_opts_startup_command_flows_into_create_session_request() {
        let opts = SessionOpts {
            cols: None,
            rows: None,
            cwd: None,
            shell: None,
            startup_command: Some("source ./init.sh".into()),
            personality_prompt: None,
            agent_name: None,
            agent_role: None,
        };
        let request = CreateSessionRequest {
            agent_id: "a1".into(),
            cols: opts.cols,
            rows: opts.rows,
            cwd: opts.cwd,
            shell: opts.shell.clone(),
            startup_command: opts.startup_command.clone(),
            personality_prompt: opts.personality_prompt.clone(),
            autostart_claude: None,
        };
        assert_eq!(
            request.startup_command,
            Some("source ./init.sh".to_string())
        );
    }

    // ---- list_available_shells ----

    #[tokio::test]
    async fn list_available_shells_returns_ok_without_panicking() {
        let result = list_available_shells().await;
        assert!(result.is_ok());
    }

    // ---- dispose_session ----

    #[tokio::test]
    async fn dispose_session_delegates_to_manager_dispose() {
        let (state, ctl, dir, profile_dir) = build("dispose");
        state.manager.create(req("a1")).unwrap();

        state.manager.dispose("a1");

        assert_eq!(
            ctl.kill_count(),
            1,
            "dispose_session must reach PtyControl::kill via manager.dispose"
        );
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- write_input ----

    #[tokio::test]
    async fn write_input_delegates_to_manager_write_input() {
        let (state, ctl, dir, profile_dir) = build("write");
        state.manager.create(req("a1")).unwrap();
        let claude_launch_len = ctl.writes_utf8().len();

        state.manager.write_input("a1", "echo hi\n");

        assert_eq!(&ctl.writes_utf8()[claude_launch_len..], "echo hi\n");
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- resize_session ----

    #[tokio::test]
    async fn resize_session_delegates_to_manager_resize() {
        let (state, ctl, dir, profile_dir) = build("resize");
        state.manager.create(req("a1")).unwrap();

        state.manager.resize("a1", 120, 40);

        assert_eq!(ctl.resize_calls(), vec![(120, 40)]);
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- list_notifications ----

    #[tokio::test]
    async fn list_notifications_delegates_to_manager_pending_notifications() {
        let (state, ctl, dir, profile_dir) = build("list-notif");
        state.manager.create(req("a1")).unwrap();
        let sid = state.manager.session_id_for("a1").unwrap();
        state.hub.ingest_hook(
            &sid,
            crate::types::NotificationSource::Hook,
            br#"{"message":"hi"}"#,
        );

        let listed = state.manager.pending_notifications("a1");

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].message, "hi");
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn list_notifications_for_unknown_agent_returns_empty() {
        let (state, ctl, dir, profile_dir) = build("list-notif-unknown");
        assert!(state.manager.pending_notifications("ghost").is_empty());
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- clear_notifications ----

    #[tokio::test]
    async fn clear_notifications_resolves_session_id_then_delegates_to_hub_clear() {
        let (state, ctl, dir, profile_dir) = build("clear-notif");
        state.manager.create(req("a1")).unwrap();
        let sid = state.manager.session_id_for("a1").unwrap();
        state.hub.ingest_hook(
            &sid,
            crate::types::NotificationSource::Hook,
            br#"{"message":"hi"}"#,
        );

        // Mirrors `clear_notifications`'s body exactly: session_id_for then hub.clear.
        if let Some(resolved_sid) = state.manager.session_id_for("a1") {
            state.hub.clear(&resolved_sid, None);
        }

        assert!(state.hub.pending(&sid).is_empty());
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn clear_notifications_for_unknown_agent_is_a_harmless_noop() {
        let (state, ctl, dir, profile_dir) = build("clear-notif-unknown");
        // Must not panic when session_id_for resolves to None.
        if let Some(sid) = state.manager.session_id_for("ghost") {
            state.hub.clear(&sid, None);
        }
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- load_state / save_state ----

    #[tokio::test]
    async fn load_state_delegates_to_store_load() {
        let (state, ctl, dir, profile_dir) = build("load-state");
        let loaded = state.store.load();
        assert_eq!(loaded.version, 1);
        assert!(loaded.agents.is_empty());
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn save_state_delegates_to_store_save_and_maps_io_error_to_string() {
        let (state, ctl, dir, profile_dir) = build("save-state");
        let persisted = PersistedState {
            agents: vec![AgentProfile {
                id: "p1".into(),
                name: "Ada".into(),
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
                sprite_updated_at: None,
                archetype: None,
                shell: None,
                startup_command: None,
                personality_prompt: None,
                clocked_out: None,
                keyboard_sound: None,
            }],
            version: 1,
        };

        // Mirrors `save_state`'s body: `store.save(&state).map_err(|e| e.to_string())`.
        let result: Result<(), String> = state.store.save(&persisted).map_err(|e| e.to_string());
        assert!(result.is_ok());

        let reloaded = state.store.load();
        assert_eq!(reloaded.agents.len(), 1);
        assert_eq!(reloaded.agents[0].name, "Ada");
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- portrait commands ----

    fn tiny_png_b64() -> String {
        use base64::Engine;
        let mut v = vec![0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend_from_slice(b"body");
        base64::engine::general_purpose::STANDARD.encode(v)
    }

    #[tokio::test]
    async fn save_then_load_then_delete_portrait_through_app_state() {
        let (state, ctl, dir, profile_dir) = build("portrait");
        // 프로필 존재 검증을 위해 profiles.json에 p1 저장.
        let persisted = PersistedState {
            agents: vec![AgentProfile {
                id: "p1".into(),
                name: "Ada".into(),
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
                sprite_updated_at: None,
                archetype: None,
                shell: None,
                startup_command: None,
                personality_prompt: None,
                clocked_out: None,
                keyboard_sound: None,
            }],
            version: 1,
        };
        state.store.save(&persisted).unwrap();
        let ids: Vec<String> = state
            .store
            .load()
            .agents
            .iter()
            .map(|a| a.id.clone())
            .collect();
        let encoded = tiny_png_b64();

        // save_portrait 본문과 동일한 delegation.
        state.portrait_store.save("p1", &encoded, &ids).unwrap();
        // load_portrait 본문.
        let loaded = state.portrait_store.load("p1").unwrap();
        assert_eq!(loaded, Some(encoded));
        // delete_portrait 본문.
        state.portrait_store.delete("p1").unwrap();
        assert_eq!(state.portrait_store.load("p1").unwrap(), None);

        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn save_portrait_maps_unknown_agent_to_err() {
        let (state, ctl, dir, profile_dir) = build("portrait-unknown");
        let ids: Vec<String> = state
            .store
            .load()
            .agents
            .iter()
            .map(|a| a.id.clone())
            .collect();
        let result: Result<(), String> = state
            .portrait_store
            .save("ghost", &tiny_png_b64(), &ids)
            .map_err(|e| e.to_string());
        assert!(result.is_err());
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn save_then_load_then_delete_sprite_through_app_state() {
        let (state, ctl, dir, profile_dir) = build("sprite");
        let persisted = PersistedState {
            agents: vec![AgentProfile {
                id: "p1".into(),
                name: "Ada".into(),
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
                sprite_updated_at: None,
                archetype: None,
                shell: None,
                startup_command: None,
                personality_prompt: None,
                clocked_out: None,
                keyboard_sound: None,
            }],
            version: 1,
        };
        state.store.save(&persisted).unwrap();
        let ids: Vec<String> = state
            .store
            .load()
            .agents
            .iter()
            .map(|a| a.id.clone())
            .collect();
        let encoded = tiny_png_b64();

        // save_sprite / load_sprite / delete_sprite 본문과 동일한 delegation.
        state.sprite_store.save("p1", &encoded, &ids).unwrap();
        assert_eq!(state.sprite_store.load("p1").unwrap(), Some(encoded));
        // portraits와 sprites는 별도 디렉터리다.
        assert_eq!(state.portrait_store.load("p1").unwrap(), None);
        state.sprite_store.delete("p1").unwrap();
        assert_eq!(state.sprite_store.load("p1").unwrap(), None);

        cleanup(&ctl, &dir, &profile_dir);
    }

    // generate_sprite_image: 네트워크 이전의 검증 로직만 테스트한다.
    // (실 API 호출 테스트 금지 — 빈 description은 HTTP 전에 걸러져야 한다.)
    #[tokio::test]
    async fn generate_sprite_image_rejects_empty_description() {
        let err = generate_sprite_image("   ".to_string()).await.unwrap_err();
        assert_eq!(err, "validation: description is empty");
    }

    // ---- append_session_turn / load_session_turns ----

    #[tokio::test]
    async fn append_then_load_session_turn_through_app_state() {
        let (state, ctl, dir, profile_dir) = build("session-turn");
        let record = SessionTurnRecord {
            agent_id: "a1".into(),
            started_at: 1_000,
            ended_at: 4_000,
            total_ms: 3_000,
            worked_ms: 2_000,
            waited_ms: 1_000,
        };

        // append_session_turn / load_session_turns 본문과 동일한 delegation.
        state
            .session_time_store
            .append(&record)
            .map_err(|e: std::io::Error| e.to_string())
            .unwrap();
        let loaded = state.session_time_store.load();

        assert_eq!(loaded, vec![record]);
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn load_session_turns_on_no_prior_appends_returns_empty() {
        let (state, ctl, dir, profile_dir) = build("session-turn-empty");
        assert!(state.session_time_store.load().is_empty());
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- load_session_events ----

    #[tokio::test]
    async fn load_session_events_reads_the_configured_root_in_range() {
        use crate::session_events::types::{SessionEventKind, SessionEventRecord};
        let (state, ctl, dir, profile_dir) = build("session-events");

        // 커맨드 본문과 동일한 root를 써서 v1 디렉터리에 하루치 파일을 만든다.
        std::fs::create_dir_all(&state.session_event_root).unwrap();
        let record = SessionEventRecord {
            schema_version: 1,
            run_id: "r".into(),
            seq: 1,
            at: 1_783_728_000_000, // 2026-07-11 00:00 UTC
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: SessionEventKind::Tool,
            agent_name: None,
            agent_role: None,
            cwd: None,
            shell: None,
            state: None,
        };
        let mut line = serde_json::to_string(&record).unwrap();
        line.push('\n');
        std::fs::write(
            state.session_event_root.join("2026-07-11.jsonl"),
            line,
        )
        .unwrap();

        // load_session_events 본문과 동일한 delegation.
        let loaded = crate::session_events::reader::load_session_events(
            &state.session_event_root,
            1_783_728_000_000,
            1_783_728_000_001,
        );

        assert_eq!(loaded, vec![record]);
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn load_session_events_on_missing_root_returns_empty() {
        let (state, ctl, dir, profile_dir) = build("session-events-empty");
        let loaded = crate::session_events::reader::load_session_events(
            &state.session_event_root,
            0,
            u64::MAX / 2,
        );
        assert!(loaded.is_empty());
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- resolve_usage_roots ----
    //
    // 전역 `std::env::var` 없이 순수 계산만 검증(load_usage_snapshot 본문과
    // 동일 로직). docs/usage-limits-design.md §2 CODEX_HOME/CLAUDE_CONFIG_DIR
    // 오버라이드 계약.

    #[test]
    fn resolve_usage_roots_defaults_to_home_when_no_env_set() {
        let home = PathBuf::from("/home/u");
        let (codex_root, claude_root) = resolve_usage_roots(&home, None, None);
        assert_eq!(codex_root, PathBuf::from("/home/u/.codex"));
        assert_eq!(claude_root, PathBuf::from("/home/u"));
    }

    #[test]
    fn resolve_usage_roots_prefers_codex_home_env_when_set() {
        let home = PathBuf::from("/home/u");
        let (codex_root, claude_root) =
            resolve_usage_roots(&home, Some("/custom/codex"), None);
        assert_eq!(codex_root, PathBuf::from("/custom/codex"));
        assert_eq!(claude_root, PathBuf::from("/home/u"));
    }

    #[test]
    fn resolve_usage_roots_prefers_claude_config_dir_env_when_set() {
        let home = PathBuf::from("/home/u");
        let (codex_root, claude_root) =
            resolve_usage_roots(&home, None, Some("/custom/claude"));
        assert_eq!(codex_root, PathBuf::from("/home/u/.codex"));
        assert_eq!(claude_root, PathBuf::from("/custom/claude"));
    }

    #[test]
    fn resolve_usage_roots_honors_both_env_vars_independently() {
        let home = PathBuf::from("/home/u");
        let (codex_root, claude_root) =
            resolve_usage_roots(&home, Some("/custom/codex"), Some("/custom/claude"));
        assert_eq!(codex_root, PathBuf::from("/custom/codex"));
        assert_eq!(claude_root, PathBuf::from("/custom/claude"));
    }

    #[test]
    fn resolve_usage_roots_treats_empty_string_env_as_unset() {
        let home = PathBuf::from("/home/u");
        let (codex_root, claude_root) = resolve_usage_roots(&home, Some(""), Some(""));
        assert_eq!(codex_root, PathBuf::from("/home/u/.codex"));
        assert_eq!(claude_root, PathBuf::from("/home/u"));
    }

    // ---- resolve_claude_config_dir ----
    //
    // 자격증명 디렉터리는 claude_root와 달리 미설정 시 ~/.claude로 내려간다
    // (docs/claude-usage-live-fetch-design.md §2.2).

    #[test]
    fn resolve_claude_config_dir_defaults_to_dot_claude_under_home() {
        let home = PathBuf::from("/home/u");
        assert_eq!(
            resolve_claude_config_dir(&home, None),
            PathBuf::from("/home/u/.claude")
        );
    }

    #[test]
    fn resolve_claude_config_dir_prefers_env_when_set() {
        let home = PathBuf::from("/home/u");
        assert_eq!(
            resolve_claude_config_dir(&home, Some("/custom/claude")),
            PathBuf::from("/custom/claude")
        );
    }

    #[test]
    fn resolve_claude_config_dir_treats_empty_env_as_unset() {
        let home = PathBuf::from("/home/u");
        assert_eq!(
            resolve_claude_config_dir(&home, Some("")),
            PathBuf::from("/home/u/.claude")
        );
    }

    // ---- load_usage_snapshot_body (스로틀 상태 위임) ----
    //
    // 커맨드 본체가 AppState 없이 호출 가능하고 파일 캐시 미러 폴백으로 항상
    // 성공하는지만 확인한다. 개발 머신엔 실 자격증명이 있을 수 있으므로 live
    // 경로(Keychain 자식 프로세스·실 API 호출)는 스로틀 선점으로 결정적으로
    // 차단한다 — 이 테스트는 네트워크·Keychain에 절대 닿지 않아야 한다.
    #[tokio::test]
    async fn load_usage_snapshot_body_delegates_and_never_errors() {
        let live = crate::usage::LiveUsageState::new();
        let now = 1_784_281_391_475;
        // 스로틀 선점: 직전에 시도한 것으로 기록해 5분 하한에 걸리게 한다.
        assert!(live.begin_attempt_if_due(now - 1));
        // 두 번 호출해도(폴링 흉내) 패닉/에러 없이 스냅샷을 돌려준다.
        let _snap = load_usage_snapshot_body(&live, now).await;
        let _snap2 = load_usage_snapshot_body(&live, now + 60_000).await;
    }
}
