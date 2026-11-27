// src-tauri/src/link/repos.rs
//
// 머신 연동 업무 분배의 작업 폴더: 업무는 저장소(git 원격 주소)로 머신과 상관없이 같은 곳을 가리킨다.
// 클라이언트는 이 머신의 클론들을 찾아 정규화한 원격 주소 목록만 호스트에 알리고(로컬 경로는 보내지 않는다),
// 업무를 받으면 그 주소의 클론에서 하위 경로·브랜치·워크트리를 정해 작업 폴더를 만든다.
//
// 찾는 곳: 캐릭터 시작 폴더와 그 바로 아래 폴더, 작업 기록(`session-events`)의 폴더. 각 폴더의
// 저장소 최상위에서 `origin`(없으면 첫 원격) 주소를 읽는다.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::proc_runner::{self, ProcOutcome, ProcSpec};

const GIT_TIMEOUT: Duration = Duration::from_secs(10);
/// 한 번 찾을 때 보는 폴더 상한(시작 폴더 아래가 아주 넓을 때).
const MAX_CANDIDATES: usize = 400;

/// git 원격 주소를 머신과 상관없이 같은 모양으로: `호스트/경로`(소문자 호스트, 사용자·포트·`.git`·끝 `/` 없음).
/// `git@github.com:acme/app.git`, `https://user@github.com/acme/app`, `ssh://git@github.com:22/acme/app.git`
/// 모두 `github.com/acme/app`이다. 로컬 경로(`/srv/app.git`, `file://`)는 그대로(끝 `.git`만 뗀다) 둔다.
pub fn normalize_remote(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.chars().any(char::is_control) {
        return None;
    }
    let trim_path = |p: &str| {
        let p = p.trim_matches('/');
        let p = p.strip_suffix(".git").unwrap_or(p);
        p.trim_end_matches('/').to_string()
    };
    if let Some(rest) = s.strip_prefix("file://") {
        let p = trim_path(rest);
        return (!p.is_empty()).then(|| format!("/{p}"));
    }
    if s.starts_with('/') || s.starts_with('.') || s.starts_with('~') || (s.len() > 2 && s.as_bytes()[1] == b':' && s.as_bytes()[0].is_ascii_alphabetic()) {
        let p = trim_path(s);
        let lead = if s.starts_with('/') { "/" } else { "" };
        return (!p.is_empty()).then(|| format!("{lead}{p}"));
    }
    let (host, path) = if let Some((_, rest)) = s.split_once("://") {
        // scheme://[user@]host[:port]/path
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
        // `ssh://git@host:owner/repo`처럼 포트 자리에 경로를 쓴 것도 받는다.
        let (host, extra) = match host.split_once(':') {
            Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => (h, String::new()),
            Some((h, more)) => (h, format!("{more}/")),
            None => (host, String::new()),
        };
        (host.to_string(), format!("{extra}{path}"))
    } else if let Some((left, path)) = s.split_once(':') {
        // scp 모양: [user@]host:path
        let host = left.rsplit_once('@').map(|(_, h)| h).unwrap_or(left);
        (host.to_string(), path.to_string())
    } else if let Some((host, path)) = s.split_once('/').filter(|(h, _)| h.contains('.')) {
        // 이미 정규형(`github.com/acme/app`)인 것.
        (host.to_string(), path.to_string())
    } else {
        return None;
    };
    let host = host.trim().to_ascii_lowercase();
    let path = trim_path(&path);
    if host.is_empty() || path.is_empty() || host.contains('/') {
        return None;
    }
    Some(format!("{host}/{path}"))
}

fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let run = proc_runner::run(ProcSpec {
        program: "git",
        args,
        cwd: Some(cwd),
        envs: &[],
        timeout: GIT_TIMEOUT,
        max_stdout_bytes: Some(256 * 1024),
        cancel: None,
    });
    match run.outcome {
        ProcOutcome::Exited { success: true } => Some(String::from_utf8_lossy(&run.stdout).trim().to_string()),
        _ => None,
    }
}

/// 폴더가 든 저장소의 최상위.
fn toplevel(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    git(dir, &["rev-parse", "--show-toplevel"]).filter(|s| !s.is_empty()).map(PathBuf::from)
}

/// 저장소 최상위의 원격 주소(`origin`, 없으면 첫 원격) 정규형.
fn remote_of(root: &Path) -> Option<String> {
    if let Some(url) = git(root, &["remote", "get-url", "origin"]) {
        return normalize_remote(&url);
    }
    let first = git(root, &["remote"])?.lines().next()?.trim().to_string();
    normalize_remote(&git(root, &["remote", "get-url", &first])?)
}

/// 폴더가 든 저장소의 원격 주소 정규형과 저장소 최상위에서의 하위 경로(없으면 빈 문자열). 저장소가 아니거나
/// 원격이 없으면 None(블로킹 — git을 부른다). 토의 작업 폴더를 다른 머신의 클론으로 옮길 때 쓴다.
pub fn locate(dir: &Path) -> Option<(String, String)> {
    let root = toplevel(dir)?;
    let url = remote_of(&root)?;
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let sub = canon(dir)
        .strip_prefix(canon(&root))
        .ok()
        .map(|p| p.components().filter_map(|c| match c {
            std::path::Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        }).collect::<Vec<_>>().join("/"))
        .unwrap_or_default();
    Some((url, sub))
}

/// 찾아볼 폴더들: 시작 폴더와 그 바로 아래 폴더, 작업 기록의 폴더.
pub fn candidates(start_dirs: &[PathBuf], event_root: Option<&Path>) -> Vec<PathBuf> {
    let mut out = BTreeSet::new();
    for dir in start_dirs {
        out.insert(dir.clone());
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten().take(MAX_CANDIDATES) {
                let name = entry.file_name();
                if name.to_string_lossy().starts_with('.') {
                    continue;
                }
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    out.insert(entry.path());
                }
            }
        }
    }
    if let Some(root) = event_root {
        for cwd in crate::ipc::commands::repository_audit::event_cwds(root).into_keys() {
            out.insert(PathBuf::from(cwd));
        }
    }
    out.into_iter().take(MAX_CANDIDATES).collect()
}

/// 원격 주소 정규형 → 그 클론의 최상위 폴더들(같은 주소의 클론이 여럿일 수 있다).
pub fn scan(candidates: &[PathBuf]) -> BTreeMap<String, Vec<PathBuf>> {
    let mut roots = BTreeSet::new();
    for dir in candidates {
        // 저장소가 아닌 폴더에서 rev-parse를 덜 부르게 `.git`이 있거나 위로 저장소가 있을 법한 곳만 본다.
        if let Some(root) = toplevel(dir) {
            roots.insert(root);
        }
    }
    let mut out: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for root in roots {
        if let Some(url) = remote_of(&root) {
            out.entry(url).or_default().push(root);
        }
    }
    out
}

fn valid_branch(root: &Path, branch: &str) -> bool {
    !branch.is_empty() && !branch.starts_with('-') && git(root, &["check-ref-format", "--branch", branch]).is_some()
}

fn branch_exists(root: &Path, branch: &str) -> bool {
    git(root, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_some()
}

/// 업무 id로 만든 워크트리 이름에 쓸 수 있는 글자만 남긴다.
fn safe_component(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect()
}

/// 업무 하나의 작업 폴더를 정한다(필요하면 브랜치를 바꾸거나 워크트리를 만든다).
/// 오류는 이유 코드(`link-repo-dirty`·`link-branch-invalid`·`link-worktree-failed:…`·`link-subpath-missing`).
pub fn prepare(root: &Path, task_id: &str, subpath: Option<&str>, branch: Option<&str>, worktree: bool) -> Result<PathBuf, String> {
    let branch = branch.map(str::trim).filter(|b| !b.is_empty());
    if let Some(b) = branch {
        if !valid_branch(root, b) {
            return Err("link-branch-invalid".into());
        }
    }
    let base = if worktree {
        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "repo".into());
        let parent = root.parent().unwrap_or(root);
        let path = parent.join(format!("{name}.worktrees")).join(safe_component(task_id));
        if toplevel(&path).is_none() {
            std::fs::create_dir_all(path.parent().unwrap_or(parent)).map_err(|e| format!("link-worktree-failed: {e}"))?;
            let target = path.to_string_lossy().into_owned();
            let default_branch = format!("office/{}", safe_component(task_id));
            let b = branch.unwrap_or(&default_branch);
            let args: Vec<&str> = if branch_exists(root, b) {
                vec!["worktree", "add", &target, b]
            } else {
                vec!["worktree", "add", "-b", b, &target]
            };
            if git(root, &args).is_none() {
                return Err(format!("link-worktree-failed: {b}"));
            }
        }
        path
    } else {
        if let Some(b) = branch {
            let current = git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
            if current != b {
                // 사람이 하던 변경을 섞지 않는다. 깨끗할 때만 브랜치를 바꾼다.
                let dirty = git(root, &["status", "--porcelain"]).map(|s| !s.is_empty()).unwrap_or(true);
                if dirty {
                    return Err("link-repo-dirty".into());
                }
                let ok = if branch_exists(root, b) { git(root, &["checkout", b]) } else { git(root, &["checkout", "-b", b]) };
                if ok.is_none() {
                    return Err(format!("link-checkout-failed: {b}"));
                }
            }
        }
        root.to_path_buf()
    };
    let cwd = match subpath.map(|s| s.trim().trim_matches('/')).filter(|s| !s.is_empty()) {
        Some(sub) => {
            // 저장소 밖으로 나가는 하위 경로는 받지 않는다.
            if Path::new(sub).components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
                return Err("link-subpath-missing".into());
            }
            base.join(sub)
        }
        None => base,
    };
    if !cwd.is_dir() {
        return Err("link-subpath-missing".into());
    }
    Ok(cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_urls_normalize_to_host_and_path() {
        for raw in [
            "git@github.com:acme/app.git",
            "https://github.com/acme/app",
            "https://user:tok@GitHub.com/acme/app.git/",
            "ssh://git@github.com:22/acme/app.git",
            "ssh://git@github.com/acme/app",
            "git://github.com/acme/app.git",
            "github.com:acme/app",
            "github.com/acme/app",
        ] {
            assert_eq!(normalize_remote(raw).as_deref(), Some("github.com/acme/app"), "{raw}");
        }
        assert_eq!(normalize_remote("ssh://git@host.example:acme/app.git").as_deref(), Some("host.example/acme/app"));
        assert_eq!(normalize_remote("/srv/git/app.git").as_deref(), Some("/srv/git/app"));
        assert_eq!(normalize_remote("file:///srv/git/app.git").as_deref(), Some("/srv/git/app"));
        assert_eq!(normalize_remote(""), None);
        assert_eq!(normalize_remote("no-colon-or-scheme"), None);
        assert_eq!(normalize_remote("https://github.com/"), None);
        assert_ne!(normalize_remote("git@github.com:acme/App"), normalize_remote("git@github.com:acme/app"), "경로 대소문자는 다른 저장소");
    }

    fn git_ok(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git").args(args).current_dir(dir).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn repo(dir: &Path, remote: &str) {
        std::fs::create_dir_all(dir.join("web")).unwrap();
        git_ok(dir, &["init", "-q", "-b", "main"]);
        git_ok(dir, &["config", "user.email", "t@example.com"]);
        git_ok(dir, &["config", "user.name", "t"]);
        std::fs::write(dir.join("web/a.txt"), "a").unwrap();
        git_ok(dir, &["add", "."]);
        git_ok(dir, &["commit", "-q", "-m", "init"]);
        git_ok(dir, &["remote", "add", "origin", remote]);
    }

    #[test]
    fn scans_clones_and_prepares_branches_and_worktrees() {
        if std::process::Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let base = std::env::temp_dir().join(format!("link-repos-{}", uuid::Uuid::new_v4()));
        let app = base.join("dev").join("app");
        repo(&app, "git@github.com:acme/app.git");
        std::fs::create_dir_all(base.join("dev").join("notes")).unwrap();
        let found = scan(&candidates(&[base.join("dev")], None));
        let app = std::fs::canonicalize(&app).unwrap();
        let roots = found.get("github.com/acme/app").expect("시작 폴더 아래 클론을 찾는다");
        assert_eq!(roots.iter().map(|r| std::fs::canonicalize(r).unwrap()).collect::<Vec<_>>(), vec![app.clone()]);
        assert_eq!(found.len(), 1, "저장소가 아닌 폴더는 없다: {found:?}");

        assert_eq!(locate(&app.join("web")), Some(("github.com/acme/app".to_string(), "web".to_string())));
        assert_eq!(locate(&app), Some(("github.com/acme/app".to_string(), String::new())));
        assert_eq!(locate(&base.join("dev").join("notes")), None);
        // 하위 경로.
        assert_eq!(prepare(&app, "t-1", Some("web"), None, false).unwrap(), app.join("web"));
        assert_eq!(prepare(&app, "t-1", Some("nope"), None, false).unwrap_err(), "link-subpath-missing");
        assert_eq!(prepare(&app, "t-1", Some("../dev"), None, false).unwrap_err(), "link-subpath-missing");
        // 브랜치: 깨끗하면 만들어 바꾼다. 변경이 있으면 거절한다.
        assert_eq!(prepare(&app, "t-1", None, Some("feature/x"), false).unwrap(), app);
        assert_eq!(git(&app, &["rev-parse", "--abbrev-ref", "HEAD"]).as_deref(), Some("feature/x"));
        std::fs::write(app.join("web/a.txt"), "changed").unwrap();
        assert_eq!(prepare(&app, "t-1", None, Some("main"), false).unwrap_err(), "link-repo-dirty");
        assert_eq!(prepare(&app, "t-1", None, Some("bad..name"), false).unwrap_err(), "link-branch-invalid");
        // 워크트리: 업무마다 따로, 두 번째는 그대로 쓴다. 사람이 하던 변경과 섞이지 않는다.
        let wt = prepare(&app, "t-2", Some("web"), None, true).unwrap();
        assert!(wt.ends_with("app.worktrees/t-2/web"), "{wt:?}");
        assert_eq!(std::fs::read_to_string(wt.join("a.txt")).unwrap(), "a");
        assert_eq!(prepare(&app, "t-2", Some("web"), None, true).unwrap(), wt);
        assert_eq!(git(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]).as_deref(), Some("office/t-2"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
