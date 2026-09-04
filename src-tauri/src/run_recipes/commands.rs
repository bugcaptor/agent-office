use crate::{
    state::AppState,
    types::{RunRecipeProbeTarget, RunRecipeProcess, RunRecipeStartInput, RunRecipeUserInput, RunRecipesReadResult},
};
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub async fn run_recipe_start(
    app_state: State<'_, AppState>, input: RunRecipeStartInput,
) -> Result<RunRecipeProcess, String> {
    app_state.run_recipes.start(input)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn run_recipe_status(
    app_state: State<'_, AppState>, agent_id: String,
) -> Result<Option<RunRecipeProcess>, String> {
    app_state.run_recipes.status(&agent_id)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn run_recipe_stop(
    app_state: State<'_, AppState>, agent_id: String,
) -> Result<(), String> {
    app_state.run_recipes.stop(&agent_id)
}

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
