// src-tauri/src/ipc/commands/bot.rs
//
// Bot mode (#57) start/stop/status commands -- thin delegations into
// `AppState::bot_runtime`.

use tauri::State;

use crate::state::AppState;
use crate::types::*;

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
