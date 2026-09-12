use std::path::Path;
use tauri::State;

use crate::ide_sessions::persona::IdePersonaContext;
use crate::ide_sessions::{self, transcripts::Candidate, ConnectResult};
use crate::session_events::types::AgentEventProfile;
use crate::state::AppState;

#[tauri::command(rename_all = "camelCase")]
pub async fn list_ide_sessions(
    app_state: State<'_, AppState>,
    provider: Option<String>,
    cwd: Option<String>,
) -> Result<Vec<Candidate>, String> {
    require_ide(&app_state)?;
    tauri::async_runtime::spawn_blocking(move || {
        ide_sessions::discover(provider.as_deref(), cwd.as_deref())
    })
    .await
    .map_err(|_| "ide-list-failed".to_string())?
}

#[tauri::command(rename_all = "camelCase")]
pub async fn connect_ide_session(
    app_state: State<'_, AppState>,
    agent_id: String,
    file: String,
    provider: String,
    source_session_id: String,
) -> Result<ConnectResult, String> {
    require_ide(&app_state)?;
    let agent = app_state
        .store
        .load()
        .agents
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .ok_or("observed-agent-not-found")?;
    let cwd = agent
        .cwd
        .filter(|cwd| !cwd.is_empty())
        .ok_or("observed-profile-cwd-missing")?;
    let profile = AgentEventProfile {
        name: agent.name,
        role: Some(agent.role).filter(|role| !role.is_empty()),
    };
    let manager = app_state.manager.clone();
    let settings = app_state.settings.clone();
    tauri::async_runtime::spawn_blocking(move || {
        ide_sessions::connect(
            &manager,
            &settings,
            &agent_id,
            &cwd,
            profile,
            Path::new(&file),
            &provider,
            &source_session_id,
        )
    })
    .await
    .map_err(|_| "ide-connect-failed".to_string())?
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_ide_persona(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<Option<IdePersonaContext>, String> {
    require_ide(&app_state)?;
    let profile = app_state
        .store
        .load()
        .agents
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .ok_or("observed-agent-not-found")?;
    let manager = app_state.manager.clone();
    tauri::async_runtime::spawn_blocking(move || ide_sessions::persona::get(&manager, &profile))
        .await
        .map_err(|_| "ide-persona-get-failed".to_string())?
}

#[tauri::command(rename_all = "camelCase")]
pub async fn prepare_ide_persona(
    app_state: State<'_, AppState>,
    agent_id: String,
    session_id: String,
    personality_prompt: String,
) -> Result<IdePersonaContext, String> {
    require_ide(&app_state)?;
    let profile = app_state
        .store
        .load()
        .agents
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .ok_or("observed-agent-not-found")?;
    let manager = app_state.manager.clone();
    let settings = app_state.settings.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _configuration = manager.observed_configuration.lock();
        require_ide_settings(&settings.read().unwrap())?;
        ide_sessions::persona::prepare(&manager, &profile, &session_id, &personality_prompt)
    })
    .await
    .map_err(|_| "ide-persona-prepare-failed".to_string())?
}

fn require_ide(state: &AppState) -> Result<(), String> {
    require_ide_settings(&state.settings.read().unwrap())
}

fn require_ide_settings(settings: &crate::persistence::settings_store::AppSettings) -> Result<(), String> {
    if !settings.ide_connection_enabled {
        return Err("ide-connection-disabled".into());
    }
    if !settings.observer_enabled {
        return Err("observed-observer-disabled".into());
    }
    Ok(())
}
