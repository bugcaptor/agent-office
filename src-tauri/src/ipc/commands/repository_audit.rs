// 작업 기록에서 발견한 저장소를 한 번에 점검한다. 이 경로는 의도적으로 fetch를
// 하지 않는다. 화면은 로컬 remote-tracking ref 기준의 "지금 조치할 일"만 보여준다.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tauri::State;

use crate::proc_runner::{self, ProcOutcome, ProcSpec};
use crate::session_events::types::SessionEventRecord;
use crate::state::AppState;

const GIT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryAuditEntry {
    pub path: String,
    pub name: String,
    pub last_worked_at: u64,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub has_remote: bool,
    pub ahead: i64,
    pub behind: i64,
    pub changed_count: usize,
    pub conflict_count: usize,
    pub timed_out: bool,
    pub unavailable: bool,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn repository_audit_list(
    app_state: State<'_, AppState>,
) -> Result<Vec<RepositoryAuditEntry>, String> {
    let event_root = app_state.session_event_root.clone();
    let profile_cwds = app_state
        .store
        .load()
        .agents
        .into_iter()
        .filter_map(|agent| agent.cwd.map(|cwd| (cwd, 0)))
        .collect();
    tauri::async_runtime::spawn_blocking(move || Ok(audit_repositories(&event_root, profile_cwds)))
        .await
        .map_err(|e| format!("저장소 점검을 실행하지 못했습니다: {e}"))?
}

fn audit_repositories(
    event_root: &Path,
    profile_cwds: Vec<(String, u64)>,
) -> Vec<RepositoryAuditEntry> {
    let mut candidates = event_cwds(event_root);
    for (cwd, at) in profile_cwds {
        candidates
            .entry(cwd)
            .and_modify(|known| *known = (*known).max(at))
            .or_insert(at);
    }

    // 후보를 순차 처리한다. 저장소 수가 많은 기록에서도 git 자식이 한꺼번에
    // 폭증하지 않도록 의도적으로 병렬화하지 않는다.
    // 서로 다른 작업 하위 폴더에서 같은 top-level 저장소가 발견될 수 있다.
    // inspect_candidate가 rev-parse로 돌려준 canonical root(path)를 키로 다시
    // 합쳐, 심볼릭 링크/하위 디렉터리 기록도 한 행으로 만든다.
    let mut deduped = BTreeMap::new();
    for (path, last_worked_at) in candidates {
        let Some(entry) = inspect_candidate(&path, last_worked_at) else {
            continue;
        };
        deduped
            .entry(entry.path.clone())
            .and_modify(|known: &mut RepositoryAuditEntry| {
                known.last_worked_at = known.last_worked_at.max(entry.last_worked_at);
            })
            .or_insert(entry);
    }
    let mut entries: Vec<_> = deduped.into_values().collect();
    entries.sort_by(|a, b| {
        audit_priority(a)
            .cmp(&audit_priority(b))
            .then_with(|| b.last_worked_at.cmp(&a.last_worked_at))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.path.cmp(&b.path))
    });
    entries
}

fn event_cwds(root: &Path) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    let Ok(days) = fs::read_dir(root) else {
        return out;
    };
    for day in days.flatten() {
        let path = day.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(file) = fs::File::open(path) else {
            continue;
        };
        // 작업 기록은 날짜별로 계속 커질 수 있으므로 전체 파일을 메모리에 올리지
        // 않고 한 줄씩 읽는다. 손상된 줄 하나는 다른 후보 수집을 막지 않는다.
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(record) = serde_json::from_str::<SessionEventRecord>(line) else {
                continue;
            };
            let Some(cwd) = record.cwd.filter(|cwd| !cwd.trim().is_empty()) else {
                continue;
            };
            out.entry(cwd)
                .and_modify(|known| *known = (*known).max(record.at))
                .or_insert(record.at);
        }
    }
    out
}

fn inspect_candidate(raw_path: &str, last_worked_at: u64) -> Option<RepositoryAuditEntry> {
    let fallback = PathBuf::from(raw_path);
    let dir = match fs::canonicalize(&fallback) {
        Ok(dir) => dir,
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            return None;
        }
        Err(_) => return Some(unavailable_entry(raw_path, last_worked_at)),
    };
    if !dir.is_dir() {
        return None;
    }

    let root_run = git(&dir, &["rev-parse", "--show-toplevel"]);
    if matches!(
        root_run.outcome,
        ProcOutcome::TimedOut | ProcOutcome::Overflowed
    ) {
        return Some(timed_out_entry(&dir, last_worked_at));
    }
    if !matches!(root_run.outcome, ProcOutcome::Exited { success: true }) {
        return Some(unavailable_entry(&dir.to_string_lossy(), last_worked_at));
    }
    let root = PathBuf::from(String::from_utf8_lossy(&root_run.stdout).trim());
    let root = match fs::canonicalize(root) {
        Ok(root) => root,
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            return None;
        }
        Err(_) => return Some(unavailable_entry(&dir.to_string_lossy(), last_worked_at)),
    };
    Some(inspect_repo_root(&root, last_worked_at))
}

fn inspect_repo_root(root: &Path, last_worked_at: u64) -> RepositoryAuditEntry {
    let status = git(root, &["status", "--porcelain=v2", "--branch", "-z"]);
    if matches!(
        status.outcome,
        ProcOutcome::TimedOut | ProcOutcome::Overflowed
    ) {
        return timed_out_entry(root, last_worked_at);
    }
    if !matches!(status.outcome, ProcOutcome::Exited { success: true }) {
        return unavailable_entry(&root.to_string_lossy(), last_worked_at);
    }
    let (branch, upstream, ahead, behind, changed_count, conflict_count) =
        parse_status(&status.stdout);
    let remote = git(root, &["remote"]);
    let has_remote = matches!(remote.outcome, ProcOutcome::Exited { success: true })
        && !remote.stdout.iter().all(u8::is_ascii_whitespace);
    let mut entry = base_entry(root, last_worked_at);
    entry.branch = branch;
    entry.upstream = upstream;
    entry.has_remote = has_remote;
    entry.ahead = ahead;
    entry.behind = behind;
    entry.changed_count = changed_count;
    entry.conflict_count = conflict_count;
    entry
}

fn git<'a>(cwd: &'a Path, args: &'a [&'a str]) -> proc_runner::ProcRun {
    proc_runner::run(ProcSpec {
        program: "git",
        args,
        cwd: Some(cwd),
        envs: &[],
        timeout: GIT_TIMEOUT,
        max_stdout_bytes: Some(1024 * 1024),
        cancel: None,
    })
}

fn parse_status(bytes: &[u8]) -> (Option<String>, Option<String>, i64, i64, usize, usize) {
    let mut branch = None;
    let mut upstream = None;
    let mut ahead = 0;
    let mut behind = 0;
    let mut changed = 0;
    let mut conflicts = 0;
    for token in bytes.split(|b| *b == 0).filter(|token| !token.is_empty()) {
        let line = String::from_utf8_lossy(token);
        if let Some(value) = line.strip_prefix("# branch.head ") {
            if value != "(detached)" && !value.is_empty() {
                branch = Some(value.to_string());
            }
        } else if let Some(value) = line.strip_prefix("# branch.upstream ") {
            upstream = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("# branch.ab ") {
            let mut values = value.split_whitespace();
            ahead = values
                .next()
                .and_then(|v| v.strip_prefix('+'))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            behind = values
                .next()
                .and_then(|v| v.strip_prefix('-'))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        } else if matches!(
            token.first(),
            Some(b'1') | Some(b'2') | Some(b'u') | Some(b'?')
        ) {
            changed += 1;
            if token.first() == Some(&b'u') {
                conflicts += 1;
            }
        }
    }
    (branch, upstream, ahead, behind, changed, conflicts)
}

fn base_entry(path: &Path, last_worked_at: u64) -> RepositoryAuditEntry {
    let path = path.to_string_lossy().into_owned();
    let name = Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&path)
        .to_string();
    RepositoryAuditEntry {
        path,
        name,
        last_worked_at,
        branch: None,
        upstream: None,
        has_remote: false,
        ahead: 0,
        behind: 0,
        changed_count: 0,
        conflict_count: 0,
        timed_out: false,
        unavailable: false,
    }
}

fn unavailable_entry(path: &str, last_worked_at: u64) -> RepositoryAuditEntry {
    let mut entry = base_entry(Path::new(path), last_worked_at);
    entry.unavailable = true;
    entry
}

fn timed_out_entry(path: &Path, last_worked_at: u64) -> RepositoryAuditEntry {
    let mut entry = base_entry(path, last_worked_at);
    entry.timed_out = true;
    entry
}

// 낮은 값이 화면의 위쪽이다.
fn audit_priority(entry: &RepositoryAuditEntry) -> u8 {
    if entry.conflict_count > 0 || entry.changed_count > 0 || entry.ahead > 0 || entry.behind > 0 {
        0
    } else if entry.timed_out || entry.unavailable || entry.upstream.is_none() || !entry.has_remote
    {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> RepositoryAuditEntry {
        base_entry(Path::new("/tmp/demo"), 1)
    }

    #[test]
    fn status_parser_counts_changes_conflicts_and_tracking() {
        let bytes = b"# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -3\01 .M N... 1 1 1 a b a.rs\0u UU N... 1 1 1 1 a b c conflict.rs\0? new.txt\0";
        let (branch, upstream, ahead, behind, changed, conflicts) = parse_status(bytes);
        assert_eq!(branch.as_deref(), Some("main"));
        assert_eq!(upstream.as_deref(), Some("origin/main"));
        assert_eq!((ahead, behind, changed, conflicts), (2, 3, 3, 1));
    }

    #[test]
    fn priority_and_recency_sort_attention_before_unknown_before_clean() {
        let mut clean = entry();
        clean.has_remote = true;
        clean.upstream = Some("origin/main".into());
        clean.last_worked_at = 99;
        let mut unknown = entry();
        unknown.unavailable = true;
        unknown.last_worked_at = 100;
        let mut attention = entry();
        attention.changed_count = 1;
        attention.last_worked_at = 1;
        let mut values = vec![clean, attention, unknown];
        values.sort_by(|a, b| {
            audit_priority(a)
                .cmp(&audit_priority(b))
                .then_with(|| b.last_worked_at.cmp(&a.last_worked_at))
                .then_with(|| a.name.cmp(&b.name))
        });
        assert_eq!(
            values.iter().map(audit_priority).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn missing_candidate_is_not_listed() {
        let entries = audit_repositories(
            Path::new("/definitely/missing/agent-office-events"),
            vec![("/definitely/missing/agent-office-repository".into(), 42)],
        );
        assert!(entries.is_empty());
    }

    #[test]
    fn event_scan_reads_every_jsonl_partition_and_keeps_latest_cwd_time() {
        let root =
            std::env::temp_dir().join(format!("agent-office-audit-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let first = r#"{"schemaVersion":1,"runId":"r","seq":1,"at":10,"agentId":"a","sessionId":"s","kind":"tool","cwd":"/work/a"}"#;
        let second = r#"{"schemaVersion":1,"runId":"r","seq":2,"at":20,"agentId":"a","sessionId":"s","kind":"tool","cwd":"/work/a"}"#;
        let other = r#"{"schemaVersion":1,"runId":"r","seq":3,"at":15,"agentId":"a","sessionId":"s","kind":"tool","cwd":"/work/b"}"#;
        fs::write(
            root.join("2026-01-01.jsonl"),
            format!("{first}\nnot-json\n"),
        )
        .unwrap();
        fs::write(
            root.join("2026-01-02.jsonl"),
            format!("{second}\n{other}\n"),
        )
        .unwrap();
        let found = event_cwds(&root);
        assert_eq!(found.get("/work/a"), Some(&20));
        assert_eq!(found.get("/work/b"), Some(&15));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn canonical_repo_root_deduplicates_recorded_subdirectories() {
        let root =
            std::env::temp_dir().join(format!("agent-office-audit-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("nested")).unwrap();
        let status = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&root)
            .status()
            .unwrap();
        assert!(status.success());

        let entries = audit_repositories(
            &root.join("no-events"),
            vec![
                (root.to_string_lossy().into_owned(), 10),
                (root.join("nested").to_string_lossy().into_owned(), 20),
                (root.join("deleted").to_string_lossy().into_owned(), 30),
            ],
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].path,
            fs::canonicalize(&root).unwrap().to_string_lossy()
        );
        assert_eq!(entries[0].last_worked_at, 20);
        assert!(!entries[0].unavailable);
        let _ = fs::remove_dir_all(root);
    }
}
