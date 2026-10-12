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
    if profile.tmux_host == Some(true)
        || profile
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
