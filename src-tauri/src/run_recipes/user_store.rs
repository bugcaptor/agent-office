use crate::types::{RunRecipe, RunRecipeSource, RunRecipeUserInput};
use std::path::Path;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UserFile {
    version: u32,
    root: String,
    recipes: Vec<RunRecipeUserInput>,
}

fn validate(recipes: &[RunRecipeUserInput]) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    for r in recipes {
        if r.id.trim().is_empty()
            || r.label.trim().is_empty()
            || r.command.trim().is_empty()
            || r.created_at.trim().is_empty()
            || !ids.insert(&r.id)
        {
            return Err("invalid-user-recipe".into());
        }
    }
    Ok(())
}

pub fn load(path: &Path, root: &str) -> Result<Vec<RunRecipe>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.to_string()),
    };
    let file: UserFile = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    if file.version != 1 || file.root != root {
        return Err("invalid-user-recipe-file".into());
    }
    validate(&file.recipes)?;
    Ok(file
        .recipes
        .into_iter()
        .map(|r| RunRecipe {
            id: r.id,
            label: r.label,
            command: r.command,
            cwd: None,
            long_running: None,
            note: None,
            created_at: Some(r.created_at),
            source: RunRecipeSource::User,
        })
        .collect())
}

pub fn save(path: &Path, root: &str, recipes: Vec<RunRecipeUserInput>) -> Result<(), String> {
    validate(&recipes)?;
    let parent = path.parent().ok_or("invalid-user-recipe-path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(&UserFile {
        version: 1,
        root: root.to_string(),
        recipes,
    })
    .map_err(|e| e.to_string())?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_input_and_writes_atomically() {
        let dir = std::env::temp_dir().join(format!("run-recipe-user-{}", std::process::id()));
        let file = dir.join("x.user.json");
        assert!(save(
            &file,
            "/r",
            vec![RunRecipeUserInput {
                id: "".into(),
                label: "x".into(),
                command: "x".into(),
                created_at: "now".into()
            }]
        )
        .is_err());
        save(
            &file,
            "/r",
            vec![RunRecipeUserInput {
                id: "x".into(),
                label: "X".into(),
                command: "make x".into(),
                created_at: "now".into(),
            }],
        )
        .unwrap();
        assert_eq!(load(&file, "/r").unwrap().len(), 1);
        assert!(!std::fs::read_dir(&dir).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_unknown_fields_in_app_owned_files() {
        let dir =
            std::env::temp_dir().join(format!("run-recipe-user-strict-{}", std::process::id()));
        let file = dir.join("x.user.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &file,
            r#"{"version":1,"root":"/r","recipes":[],"unexpected":true}"#,
        )
        .unwrap();

        assert!(load(&file, "/r").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
