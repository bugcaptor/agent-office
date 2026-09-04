use serde::Deserialize;

use crate::types::{RunRecipe, RunRecipeSource, RunRecipesAgentState};

fn valid_relative_cwd(cwd: &str) -> bool {
    use std::path::Component;

    !cwd.trim().is_empty()
        && std::path::Path::new(cwd).is_relative()
        && !std::path::Path::new(cwd).components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentFile {
    version: u32,
    root: String,
    recipes: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentRecipe {
    id: Option<String>,
    label: Option<String>,
    command: Option<String>,
    cwd: Option<String>,
    long_running: Option<bool>,
    note: Option<String>,
}

pub fn read(
    path: &std::path::Path,
    expected_root: &str,
) -> (RunRecipesAgentState, Option<String>, Vec<RunRecipe>) {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (RunRecipesAgentState::Missing, None, vec![])
        }
        Err(e) => return (RunRecipesAgentState::Invalid, Some(e.to_string()), vec![]),
    };
    let file: AgentFile = match serde_json::from_str(&raw) {
        Ok(file) => file,
        Err(e) => return (RunRecipesAgentState::Invalid, Some(e.to_string()), vec![]),
    };
    if file.version != 1 || file.root != expected_root {
        return (
            RunRecipesAgentState::Invalid,
            Some("root-or-version-mismatch".into()),
            vec![],
        );
    }
    // agent 파일은 외부 작성물: 한 항목의 불량이 전체 조사 결과를 숨기지 않는다.
    let recipes = file
        .recipes
        .into_iter()
        .filter_map(|value| {
            let r: AgentRecipe = serde_json::from_value(value).ok()?;
            let (id, label, command) = (r.id?, r.label?, r.command?);
            if id.trim().is_empty() || label.trim().is_empty() || command.trim().is_empty() {
                return None;
            }
            if r.cwd.as_deref().is_some_and(|cwd| !valid_relative_cwd(cwd)) {
                return None;
            }
            Some(RunRecipe {
                id,
                label,
                command,
                cwd: r.cwd,
                long_running: r.long_running,
                note: r.note,
                created_at: None,
                source: RunRecipeSource::Agent,
            })
        })
        .collect();
    (RunRecipesAgentState::Ready, None, recipes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn accepts_good_items_and_skips_bad_ones() {
        let path =
            std::env::temp_dir().join(format!("run-recipe-agent-{}.json", std::process::id()));
        fs::write(&path, r#"{"version":1,"root":"/r","recipes":[{"id":"a","label":"A","command":"make a"},{"id":"bad","label":"B"},{"id":"wrong-type","label":"Wrong","command":42},false]}"#).unwrap();
        let (_, _, recipes) = read(&path, "/r");
        assert_eq!(recipes.len(), 1);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn broken_json_is_an_invalid_state_not_a_command_error() {
        let path =
            std::env::temp_dir().join(format!("run-recipe-broken-{}.json", std::process::id()));
        fs::write(&path, "{").unwrap();
        let (state, error, recipes) = read(&path, "/r");
        assert_eq!(state, RunRecipesAgentState::Invalid);
        assert!(error.is_some());
        assert!(recipes.is_empty());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn wrong_root_is_invalid_even_when_the_filename_matches() {
        let path =
            std::env::temp_dir().join(format!("run-recipe-root-{}.json", std::process::id()));
        fs::write(&path, r#"{"version":1,"root":"/other","recipes":[]}"#).unwrap();
        assert_eq!(read(&path, "/r").0, RunRecipesAgentState::Invalid);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn skips_recipes_that_escape_the_project_root() {
        let path = std::env::temp_dir().join(format!("run-recipe-cwd-{}.json", std::process::id()));
        fs::write(&path, r#"{"version":1,"root":"/r","recipes":[{"id":"escape","label":"Escape","command":"test","cwd":"../outside"},{"id":"inside","label":"Inside","command":"test","cwd":"web"}]}"#).unwrap();

        let (_, _, recipes) = read(&path, "/r");
        assert_eq!(recipes.len(), 1);
        assert_eq!(recipes[0].id, "inside");
        let _ = fs::remove_file(path);
    }
}
