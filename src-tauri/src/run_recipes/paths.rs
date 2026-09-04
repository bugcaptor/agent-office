use std::path::{Path, PathBuf};

/// 프로필 cwd의 레시피 키. canonicalize하지 않아 사용자가 적은 경로 문자열의
/// 정체성을 보존한다.
pub fn normalize_root(root: String) -> Result<String, String> {
    let mut root = crate::session::manager::expand_tilde(root);
    if root.trim().is_empty() {
        return Err("invalid-root".into());
    }
    if cfg!(windows) {
        root = root.replace('\\', "/").to_lowercase();
    }
    while root.len() > 1
        && root.ends_with('/')
        && !(cfg!(windows) && root.len() == 3 && root.as_bytes()[1] == b':')
    {
        root.pop();
    }
    if !Path::new(&root).is_absolute() {
        return Err("invalid-root".into());
    }
    Ok(root)
}

pub fn recipe_paths(app_data: &Path, root: &str) -> (PathBuf, PathBuf) {
    let mut hash = sha1_smol::Sha1::new();
    hash.update(root.as_bytes());
    let digest = hash.digest().to_string();
    let slug = crate::session::tmux_host::dir_slug(root);
    let base = format!("{slug}-{}", &digest[..12]);
    let dir = app_data.join("run-recipes");
    (
        dir.join(format!("{base}.agent.json")),
        dir.join(format!("{base}.user.json")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_tilde_and_trailing_separator_without_resolving_symlinks() {
        let root = normalize_root("/tmp/a/".into()).unwrap();
        assert_eq!(root, "/tmp/a");
        assert_eq!(
            normalize_root("relative".into()),
            Err("invalid-root".into())
        );
    }

    #[test]
    fn names_are_readable_and_stable() {
        let (agent, user) = recipe_paths(Path::new("/data"), "/work/Agent Office");
        assert!(agent.to_string_lossy().contains("agent-office-"));
        assert!(agent.to_string_lossy().ends_with(".agent.json"));
        assert!(user.to_string_lossy().ends_with(".user.json"));
    }
}
