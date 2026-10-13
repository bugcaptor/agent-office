// src-tauri/src/ipc/commands/settings.rs
//
// App settings read/write plus CLI control (#55) 2-step approval commands.
// `apply_settings_patch` is shared with the CLI control handler
// (`control::settings_set`) so it stays `pub(crate)` and is called at its
// full crate path from `control/mod.rs` -- keep it re-exported at
// `crate::ipc::commands::apply_settings_patch` via the parent module's
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
        settings: app_state.settings.read().unwrap().clone(),
        first_run: app_state
            .settings_first_run
            .load(std::sync::atomic::Ordering::SeqCst),
    })
}

/// 렌더러가 바꾼 키만 담은 패치를 받아 백엔드 현재 값에 병합·저장한다(수정
/// F6) — 렌더러 사본 전체를 저장하면 CLI(`ctl settings set`)가 바꾼 값을
/// 되돌릴 수 있다. Observer OFF→ON이면 서버를 지연 기동한다(이미 떠 있으면
/// 재사용). ON→OFF는 서버 프로세스 자체를 내리지 않는다 — 이미 떠 있는
/// 세션들의 훅 POST는 계속 수신된다. 다만 캐시가 OFF로 갱신된 뒤로는
/// lib.rs의 observer URL getter가 (서버가 살아있어도) None을 돌려주므로, 이
/// 시점 이후 새로 만드는 세션에는 훅 배선(--settings·env·ZDOTDIR)이 전혀
/// 주입되지 않는다 -- "변경은 새 세션부터 적용" 정책의 실제 동작.
#[tauri::command(rename_all = "camelCase")]
pub async fn set_app_settings(
    app_state: State<'_, AppState>,
    patch: serde_json::Value,
) -> Result<(), String> {
    set_app_settings_inner(&app_state, patch).await
}

pub(crate) async fn set_app_settings_inner(
    app_state: &AppState,
    patch: serde_json::Value,
) -> Result<(), String> {
    let Some(patch) = patch.as_object() else {
        return Err("설정 패치는 JSON 객체여야 합니다".into());
    };
    let ctx = &app_state.control_ctx;
    let settings = apply_settings_patch(
        &app_state.settings_store,
        &app_state.settings,
        &app_state.hub,
        &app_state.observer_server,
        &app_state.observer,
        ctx.settings_changed.as_ref(),
        patch,
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

    // 잠자기 방지(#68) OFF 전환은 즉시 반영 — 렌더러 구독도 곧 set_keep_awake(false)를
    // 보내지만, 백엔드가 권위이자 즉시 해제하는 게이트다. (CLI control 경로는 이 함수를
    // 안 거치지만, 거기서 keep_awake를 끄더라도 lease TTL이 최대 3분 내 backstop 해제한다.)
    if !settings.keep_awake_enabled {
        app_state.wake_lock.deactivate();
    }

    // 세션 로그 토글은 기록 중인 세션에도 즉시 반영된다 — SessionManager와
    // 같은 Arc<AtomicBool>이라 다음 출력 청크부터 반영/중단된다.
    app_state
        .session_log_enabled
        .store(settings.session_log_enabled, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// 렌더러가 "지금 일하는 캐릭터가 하나 이상"이라고 통지한다(이슈 #68). `active`
/// 통지는 lease(180초 TTL)를 갱신하며 rising-edge에 즉시, 이후 주기적으로 온다.
/// 설정 `keep_awake_enabled`가 꺼져 있으면 조용히 무시한다(백엔드가 최종 게이트).
/// `active=false`는 즉시 해제한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn set_keep_awake(
    app_state: State<'_, AppState>,
    active: bool,
) -> Result<(), String> {
    let enabled = app_state.settings.read().unwrap().keep_awake_enabled;
    if active {
        if enabled {
            app_state
                .wake_lock
                .renew(std::time::Duration::from_secs(180));
        }
    } else {
        app_state.wake_lock.deactivate();
    }
    Ok(())
}

/// 현재 설정에 camelCase 키 패치를 덮어 새 설정을 만든다. 모르는 키는
/// 역직렬화에서 무시되고, 타입이 틀린 값은 오류다.
pub(crate) fn merge_settings_patch(
    current: &AppSettings,
    patch: &serde_json::Map<String, serde_json::Value>,
) -> Result<AppSettings, String> {
    let mut merged = serde_json::to_value(current).map_err(|e| e.to_string())?;
    if let Some(map) = merged.as_object_mut() {
        for (k, v) in patch {
            map.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(merged).map_err(|e| format!("설정 파싱 실패: {e}"))
}

/// 설정 저장의 공통 경로 — 패치 병합 + 디스크 저장 + 캐시 갱신 + 변경 통지 +
/// 홀드 시간 반영 + observer 서버 지연 기동. Tauri 커맨드
/// (`set_app_settings_inner`)와 CLI 제어 핸들러(`control::settings_set`)가
/// 공유한다. first_run 플래그와 control 서버 lifecycle은 호출자별로 달라
/// 여기서 다루지 않는다. 저장된 새 설정을 돌려준다.
pub(crate) async fn apply_settings_patch(
    settings_store: &crate::persistence::settings_store::SettingsStore,
    settings_cache: &std::sync::RwLock<AppSettings>,
    hub: &crate::notification::hub::NotificationHub,
    observer_server: &crate::observer::server::ObserverServerState,
    observer: &std::sync::Arc<crate::observer::ObserverRuntime>,
    notify: &(dyn Fn(&AppSettings) + Send + Sync),
    patch: &serde_json::Map<String, serde_json::Value>,
) -> Result<AppSettings, String> {
    // write 가드 하나 안에서 읽기·병합·저장·캐시 갱신·통지를 끝낸다(수정 F6) --
    // GUI 저장과 CLI 저장이 겹쳐도 한쪽 병합이 다른 쪽 결과를 못 보고 덮어쓰는
    // 일이 없고, 통지도 저장 순서대로 나가 렌더러가 마지막 값에 수렴한다.
    // 가드는 .await 지점 전에 스코프를 벗어난다(no-lock-across-await 계약 유지).
    let settings = {
        let mut guard = settings_cache.write().unwrap();
        let settings = merge_settings_patch(&guard, patch)?;
        settings_store.save(&settings).map_err(|e| e.to_string())?;
        // AppSettings는 Copy가 아니다 — 아래에서 계속 읽으므로 복제해 넣는다.
        *guard = settings.clone();
        notify(&settings);
        settings
    };
    // 이슈 #41: 질문 알림 홀드 시간 변경을 즉시 hub 에 반영한다.
    hub.set_hold_duration(std::time::Duration::from_millis(settings.attention_hold_ms));
    // 기본 알림 문구(문구 없는 훅의 폴백)도 UI 언어를 따라간다.
    hub.set_lang(crate::i18n::ui_lang(&settings));
    if settings.observer_enabled {
        let _ = observer_server.ensure(observer.clone()).await;
    }
    Ok(settings)
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
