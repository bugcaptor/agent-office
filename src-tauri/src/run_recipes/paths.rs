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
    let slug = dir_slug(root);
    let base = format!("{slug}-{}", &digest[..12]);
    let dir = app_data.join("run-recipes");
    (
        dir.join(format!("{base}.agent.json")),
        dir.join(format!("{base}.user.json")),
    )
}

/// 레시피 파일 이름의 폴더 조각. 소문자화 → `[a-z0-9]` 외 전부 `-` → 연속 `-`
/// 압축 → 앞뒤 `-` 제거 → 최대 20자. ASCII가 하나도 안 남으면 `"work"`.
/// 이미 저장된 레시피 파일 이름이 이 규칙에 묶여 있으므로 바꾸면 안 된다
/// (예전 tmux 자동 호스팅의 세션 이름 규칙에서 옮겨 왔다).
fn dir_slug(cwd: &str) -> String {
    const MAX_LEN: usize = 20;
    let trimmed = cwd.trim_end_matches('/');
    let base = trimmed.rsplit('/').next().unwrap_or(trimmed);
    let lower = base.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut last_was_dash = false;
    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_was_dash = false;
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    let truncated: String = trimmed.chars().take(MAX_LEN).collect();
    let truncated = truncated.trim_end_matches('-');
    if truncated.is_empty() {
        "work".to_string()
    } else {
        truncated.to_string()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn dir_slug_takes_last_path_segment() {
        assert_eq!(dir_slug("/Users/me/dev/agent-office"), "agent-office");
        assert_eq!(dir_slug("/Users/me/dev/agent-office/"), "agent-office");
        assert_eq!(dir_slug("/Users/me/dev/ex-07-raylib_space"), "ex-07-raylib-space");
        assert_eq!(dir_slug("/Users/me/dev/작업"), "work");
        assert_eq!(dir_slug("/"), "work");
        // 20자 절단.
        assert_eq!(dir_slug("/x/abcdefghijklmnopqrstuvwxyz"), "abcdefghijklmnopqrst");
    }

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
