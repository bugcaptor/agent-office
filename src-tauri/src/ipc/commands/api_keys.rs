// src-tauri/src/ipc/commands/api_keys.rs
//
// 요약기용 외부 API 키(Anthropic·OpenRouter) 저장소 커맨드. 키 값은 어떤
// 커맨드도 렌더러로 돌려주지 않는다 — 존재 여부 bool만 내려준다.

use tauri::State;

use crate::key_store::ApiKeyStatus;
use crate::state::AppState;

/// 설정 UI가 표시하는 상태 — 키 값이 아니라 존재 여부 bool + env 유래 여부.
#[tauri::command(rename_all = "camelCase")]
pub async fn api_key_status(app_state: State<'_, AppState>) -> Result<ApiKeyStatus, String> {
    Ok(app_state.key_store.status())
}

/// 키 저장. `null`인 필드는 기존 값을 유지하고, 빈 문자열은 삭제다.
/// 반환값은 갱신된 마스킹 상태(렌더러가 즉시 UI를 맞출 수 있게).
#[tauri::command(rename_all = "camelCase")]
pub async fn set_api_keys(
    app_state: State<'_, AppState>,
    anthropic: Option<String>,
    openrouter: Option<String>,
) -> Result<ApiKeyStatus, String> {
    app_state
        .key_store
        .set(anthropic, openrouter)
        .map_err(|e| format!("key-save-failed: {e}"))
}
