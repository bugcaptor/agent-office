// src-tauri/src/ipc/commands/settings.rs
//
// App settings read/write plus CLI control (#55) 2-step approval commands.
// `apply_settings_effects` is shared with the CLI control handler
// (`control::settings_set`) so it stays `pub(crate)` and is called at its
// full crate path from `control/mod.rs` -- keep it re-exported at
// `crate::ipc::commands::apply_settings_effects` via the parent module's
// `pub use settings::*;`.

use tauri::State;

use crate::persistence::settings_store::AppSettings;
use crate::state::AppState;

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

pub(crate) async fn set_app_settings_inner(
    app_state: &AppState,
    settings: AppSettings,
) -> Result<(), String> {
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
