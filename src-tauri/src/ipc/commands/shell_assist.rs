// src-tauri/src/ipc/commands/shell_assist.rs
//
// 셸 캐릭터(`crate::shell_assist`)의 LLM 설정과 앱 입력줄의 명령 제안.

use crate::shell_assist::config::{LocalEndpoint, ShellAssistConfig};
use crate::shell_assist::llm::Suggestion;

#[tauri::command(rename_all = "camelCase")]
pub fn shell_assist_get_config() -> ShellAssistConfig {
    crate::shell_assist::load_config()
}

#[tauri::command(rename_all = "camelCase")]
pub fn shell_assist_set_config(config: ShellAssistConfig) -> Result<ShellAssistConfig, String> {
    crate::shell_assist::save_config(config)
}

/// 설정 화면의 연결 확인: 그 엔드포인트의 모델 목록.
#[tauri::command(rename_all = "camelCase")]
pub async fn shell_assist_list_models(endpoint: LocalEndpoint) -> Result<Vec<String>, String> {
    crate::shell_assist::llm::list_models(&endpoint).await
}

/// 앱 터미널 입력줄: 말 → 명령. 렌더러가 받은 명령을 입력줄에 채운다(실행은 사람이 Enter).
#[tauri::command(rename_all = "camelCase")]
pub async fn shell_assist_suggest(agent_id: String, text: String) -> Result<Suggestion, String> {
    crate::shell_assist::suggest(&agent_id, &text).await
}
