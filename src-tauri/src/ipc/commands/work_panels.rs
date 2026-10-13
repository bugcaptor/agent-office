use crate::{
    state::AppState,
    types::{WorkSendRequest, WorkSendResult, WorkSendTarget},
};
use tauri::State;

fn validate_target(app_state: &AppState, agent_id: &str) -> Result<(), String> {
    let profiles = app_state.store.load();
    let profile = profiles
        .agents
        .iter()
        .find(|p| p.id == agent_id)
        .ok_or("work-send-agent-missing")?;
    if profile.clocked_out == Some(true) {
        return Err("work-send-clocked-out".into());
    }
    if profile
        .startup_command
        .as_ref()
        .is_some_and(|c| !c.trim().is_empty())
    {
        return Err("work-send-profile-unsupported".into());
    }
    if app_state.bot_runtime.is_running(agent_id)
        || app_state
            .automation_runtime
            .status()
            .agents
            .get(agent_id)
            .is_some_and(|a| a.running)
    {
        return Err("work-send-producer-running".into());
    }
    Ok(())
}
#[tauri::command(rename_all = "camelCase")]
pub async fn inspect_work_send_target(
    agent_id: String,
    app_state: State<'_, AppState>,
) -> Result<WorkSendTarget, String> {
    validate_target(&app_state, &agent_id)?;
    app_state
        .work_send_runtime
        .inspect(&app_state.gate, &agent_id)
}
#[tauri::command(rename_all = "camelCase")]
pub async fn send_work_item(
    request: WorkSendRequest,
    app_state: State<'_, AppState>,
) -> Result<WorkSendResult, String> {
    validate_target(&app_state, &request.agent_id)?;
    app_state
        .work_send_runtime
        .send(
            &app_state.gate,
            app_state.automation_store.app_data_dir().to_path_buf(),
            request,
        )
        .await
}

#[tauri::command]
pub fn scan_work_inbox(app_state: State<'_, AppState>) -> Result<Vec<crate::persistence::work_inbox::InboxEntry>, String> {
    crate::persistence::work_inbox::scan(&app_state.automation_store.app_data_dir().join("work-inbox"), std::time::SystemTime::now())
}
#[tauri::command(rename_all = "camelCase")]
pub fn ack_work_inbox(file_names: Vec<String>, app_state: State<'_, AppState>) -> Result<(), String> {
    crate::persistence::work_inbox::ack(&app_state.automation_store.app_data_dir().join("work-inbox"), &file_names)
}

/// 업무 목록 내보내기. 직렬화는 렌더러가 하고, 여기서는 고른 위치에 쓰기만 한다.
/// `filter_name`은 다이얼로그 형식 이름(현재 언어). 저장한 경로, 취소 시 None.
#[tauri::command(rename_all = "camelCase")]
pub async fn export_work_items_file(
    app: tauri::AppHandle,
    default_name: String,
    filter_name: String,
    content: String,
) -> Result<Option<String>, String> {
    super::save_text_with_dialog(&app, default_name, &filter_name, content).await
}
/// 업무 목록 가져오기. 고른 파일의 텍스트만 돌려주고 파싱·검증은 렌더러가 한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn import_work_items_file(
    app: tauri::AppHandle,
    filter_name: String,
) -> Result<Option<String>, String> {
    super::open_text_with_dialog(&app, &filter_name).await
}
