use crate::{
    state::AppState,
    types::{RunRecipeProbeTarget, RunRecipeUserInput, RunRecipesReadResult},
};
use tauri::State;

fn targets(
    app_state: &AppState,
    root: String,
) -> Result<(String, std::path::PathBuf, std::path::PathBuf), String> {
    let root = super::paths::normalize_root(root)?;
    let (agent, user) = super::paths::recipe_paths(&app_state.control_ctx.app_data_dir, &root);
    Ok((root, agent, user))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn run_recipes_read(
    app_state: State<'_, AppState>,
    root: String,
) -> Result<RunRecipesReadResult, String> {
    let (root, agent, user) = targets(&app_state, root)?;
    let (agent_state, agent_error, agent_recipes) = super::agent_file::read(&agent, &root);
    let user_recipes = super::user_store::load(&user, &root)?;
    Ok(RunRecipesReadResult {
        root,
        agent_file_path: agent.to_string_lossy().into_owned(),
        agent_state,
        agent_error,
        agent_recipes,
        user_recipes,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn run_recipes_user_save(
    app_state: State<'_, AppState>,
    root: String,
    recipes: Vec<RunRecipeUserInput>,
) -> Result<(), String> {
    let (root, _agent, user) = targets(&app_state, root)?;
    super::user_store::save(&user, &root, recipes)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn run_recipes_agent_clear(
    app_state: State<'_, AppState>,
    root: String,
) -> Result<(), String> {
    let (_root, agent, _user) = targets(&app_state, root)?;
    match std::fs::remove_file(agent) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn run_recipes_probe_target(
    app_state: State<'_, AppState>,
    root: String,
) -> Result<RunRecipeProbeTarget, String> {
    let (root, agent, _user) = targets(&app_state, root)?;
    std::fs::create_dir_all(agent.parent().ok_or("invalid-agent-recipe-path")?)
        .map_err(|e| e.to_string())?;
    Ok(RunRecipeProbeTarget {
        root,
        agent_file_path: agent.to_string_lossy().into_owned(),
    })
}
