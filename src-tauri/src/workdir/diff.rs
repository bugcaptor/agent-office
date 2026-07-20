// src-tauri/src/workdir/diff.rs
//
// 파일 diff, 파일 히스토리, 커밋 변경파일, 저장소 로그, 외부 difftool 실행.
// 모두 `run_git`(git_runner) 위에 얹힌 조회 계열이며, 결과 파싱 함수(parse_*)도
// 여기 함께 둔다.

use std::process::{Command, Stdio};
use std::time::Duration;

use super::git_runner::{canon_dir, run_git, sanitize_rel_path, valid_commit};
use super::model::{
    GitCommitFileEntry, GitCommitFilesResult, GitDiffResult, GitFileHistoryResult,
};

/// diff/log/show subprocess 타임아웃. status보다 넉넉하되(대용량 diff·긴 로그)
/// UI가 무한정 멈추지 않도록 상한을 둔다.
const GIT_QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// diff 텍스트 상한(바이트). 이 크기를 넘으면 잘라내고 `truncated=true`.
const MAX_DIFF_BYTES: usize = 1024 * 1024;
/// diff 텍스트 상한(줄 수). 이 줄 수를 넘으면 잘라내고 `truncated=true`.
const MAX_DIFF_LINES: usize = 5000;
/// 파일 히스토리 조회 상한(요청 limit을 이 값으로 클램프).
const HISTORY_MAX_LIMIT: usize = 200;

/// 커밋 변경파일 목록 파싱 상한. 거대 커밋(수만 파일)에서 메모리를 보호하기 위해
/// 이 수까지만 파싱하고 이후는 `has_more`로 표현한다.
const COMMIT_FILES_PARSE_CAP: usize = 20_000;
/// 커밋 변경파일 페이지 크기 상한(요청 limit을 이 값으로 클램프).
const COMMIT_FILES_MAX_LIMIT: usize = 1000;

/// git diff 출력 바이트를 상한(바이트/줄)에 맞춰 잘라 GitDiffResult로 만든다.
/// 바이너리 판정은 git이 내는 "Binary files ..." / "GIT binary patch" 표식으로.
fn finalize_diff(bytes: &[u8]) -> GitDiffResult {
    let text = String::from_utf8_lossy(bytes);
    let binary = text.contains("Binary files ") || text.contains("GIT binary patch");
    let mut out = String::new();
    let mut truncated = false;
    for (idx, line) in text.lines().enumerate() {
        if idx >= MAX_DIFF_LINES || out.len() + line.len() + 1 > MAX_DIFF_BYTES {
            truncated = true;
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    GitDiffResult {
        diff: out,
        binary,
        truncated,
        timed_out: false,
    }
}

/// root 기준 `rel_path`의 diff를 `mode`에 맞춰 뽑는다.
/// - `worktreeVsIndex`: 워킹트리↔인덱스(미스테이지 변경) `git diff`
/// - `indexVsHead`: 인덱스↔HEAD(스테이지된 변경) `git diff --cached`
/// - `worktreeVsHead`: 워킹트리↔HEAD(전체 변경 합본) `git diff HEAD`
/// - `untracked`: 미추적 파일을 새 파일로 `git diff --no-index /dev/null <path>`
///
/// 미추적 파일은 일반 `git diff`가 아무것도 내지 않으므로 `untracked` 모드가
/// 필요하다(프런트가 뱃지 '?'를 보고 이 모드로 요청).
pub fn git_diff_file(root: &str, rel_path: &str, mode: &str) -> Result<GitDiffResult, String> {
    let canon_root = canon_dir(root)?;
    let rel = sanitize_rel_path(rel_path)?;
    let args: Vec<&str> = match mode {
        "worktreeVsIndex" => vec!["diff", "--", &rel],
        "indexVsHead" => vec!["diff", "--cached", "--", &rel],
        "worktreeVsHead" => vec!["diff", "HEAD", "--", &rel],
        // /dev/null은 git이 diff --no-index에서 크로스플랫폼으로 특별 처리한다.
        "untracked" => vec!["diff", "--no-index", "--", "/dev/null", &rel],
        other => return Err(format!("알 수 없는 diff 모드: {other}")),
    };
    let run = run_git(&canon_root, &args, GIT_QUERY_TIMEOUT);
    if run.spawn_failed {
        return Err("git 실행에 실패했습니다(설치 여부 확인)".to_string());
    }
    if run.timed_out {
        return Ok(GitDiffResult {
            diff: String::new(),
            binary: false,
            truncated: false,
            timed_out: true,
        });
    }
    // diff --no-index는 차이가 있으면 exit 1을 내지만 정상 출력이다. 그래서
    // success와 무관하게 stdout을 그대로 파싱한다(exit 2 등 에러도 빈 stdout이면
    // 빈 diff로 귀결되어 UI에 "변경 없음"으로 보인다).
    Ok(finalize_diff(&run.stdout))
}

/// root 기준 `rel_path`의 커밋 히스토리를 `git log --follow`로 가져온다(페이지네이션).
pub fn git_file_history(
    root: &str,
    rel_path: &str,
    limit: usize,
    skip: usize,
) -> Result<GitFileHistoryResult, String> {
    let canon_root = canon_dir(root)?;
    let rel = sanitize_rel_path(rel_path)?;
    let limit = limit.clamp(1, HISTORY_MAX_LIMIT);
    // 필드 구분 US(0x1f), 레코드 구분은 -z의 NUL. subject에 개행이 있어도 안전.
    let n_arg = format!("-n{limit}");
    let skip_arg = format!("--skip={skip}");
    let args = [
        "log",
        "--follow",
        "--date=format:%Y-%m-%d %H:%M",
        "--format=%H%x1f%h%x1f%an%x1f%ad%x1f%s",
        "-z",
        &n_arg,
        &skip_arg,
        "--",
        &rel,
    ];
    let run = run_git(&canon_root, &args, GIT_QUERY_TIMEOUT);
    if run.spawn_failed {
        return Err("git 실행에 실패했습니다(설치 여부 확인)".to_string());
    }
    if run.timed_out {
        return Ok(GitFileHistoryResult {
            commits: Vec::new(),
            has_more: false,
            timed_out: true,
        });
    }
    Ok(parse_history(&run.stdout, limit))
}

/// `git log ... -z --format=%H\x1f%h\x1f%an\x1f%ad\x1f%s` 출력을 파싱한다.
/// 레코드는 NUL로 구분되며 각 레코드 안은 US(0x1f)로 5개 필드가 나뉜다.
fn parse_history(bytes: &[u8], limit: usize) -> GitFileHistoryResult {
    use super::model::GitCommitEntry;
    let mut commits = Vec::new();
    for rec in bytes.split(|&b| b == 0) {
        // 레코드 사이 개행이 섞일 수 있어 앞뒤 공백/개행을 다듬는다.
        let s = String::from_utf8_lossy(rec);
        let s = s.trim_matches(['\n', '\r']);
        if s.is_empty() {
            continue;
        }
        let mut f = s.splitn(5, '\u{1f}');
        let hash = f.next().unwrap_or("").to_string();
        let short_hash = f.next().unwrap_or("").to_string();
        let author = f.next().unwrap_or("").to_string();
        let date = f.next().unwrap_or("").to_string();
        let subject = f.next().unwrap_or("").to_string();
        if hash.is_empty() {
            continue;
        }
        commits.push(GitCommitEntry {
            hash,
            short_hash,
            author,
            date,
            subject,
        });
    }
    let has_more = commits.len() >= limit;
    GitFileHistoryResult {
        commits,
        has_more,
        timed_out: false,
    }
}

/// 특정 커밋이 `rel_path`에 만든 변경(diff)을 `git show`로 가져온다. `--format=`로
/// 커밋 메시지 헤더를 지워 diff만 남긴다.
pub fn git_diff_commit(root: &str, commit: &str, rel_path: &str) -> Result<GitDiffResult, String> {
    let canon_root = canon_dir(root)?;
    if !valid_commit(commit) {
        return Err(format!("잘못된 커밋 해시입니다: {commit}"));
    }
    let rel = sanitize_rel_path(rel_path)?;
    let args = ["show", "--format=", commit, "--", &rel];
    let run = run_git(&canon_root, &args, GIT_QUERY_TIMEOUT);
    if run.spawn_failed {
        return Err("git 실행에 실패했습니다(설치 여부 확인)".to_string());
    }
    if run.timed_out {
        return Ok(GitDiffResult {
            diff: String::new(),
            binary: false,
            truncated: false,
            timed_out: true,
        });
    }
    Ok(finalize_diff(&run.stdout))
}

/// 한 커밋이 바꾼 파일 목록을 `git show --name-status`로 가져온다(페이지네이션).
/// pathspec 없이 커밋 하나만 받으므로 `valid_commit` 검증이 필수다(옵션·경로
/// 주입 차단). 병합 커밋은 git이 기본 combined diff를 쓰므로 변경파일이 비어
/// 있을 수 있다(정상 — 프런트가 "표시할 파일 변경 없음"으로 안내).
pub fn git_commit_files(
    root: &str,
    commit: &str,
    limit: usize,
    skip: usize,
) -> Result<GitCommitFilesResult, String> {
    let canon_root = canon_dir(root)?;
    if !valid_commit(commit) {
        return Err(format!("잘못된 커밋 해시입니다: {commit}"));
    }
    let limit = limit.clamp(1, COMMIT_FILES_MAX_LIMIT);
    // `--format=`로 커밋 헤더를 지우고 `--name-status -M -z`로 파일별 상태만 뽑는다.
    let args = ["show", "--format=", "--name-status", "-M", "-z", commit];
    let run = run_git(&canon_root, &args, GIT_QUERY_TIMEOUT);
    if run.spawn_failed {
        return Err("git 실행에 실패했습니다(설치 여부 확인)".to_string());
    }
    if run.timed_out {
        return Ok(GitCommitFilesResult {
            files: Vec::new(),
            has_more: false,
            timed_out: true,
        });
    }
    let all = parse_name_status(&run.stdout);
    let total = all.len();
    let files: Vec<GitCommitFileEntry> = all.into_iter().skip(skip).take(limit).collect();
    // 이 페이지 뒤로 남은 게 있거나, 파싱 상한에 걸려 잘렸으면 더 있음.
    let has_more = skip + files.len() < total || total >= COMMIT_FILES_PARSE_CAP;
    Ok(GitCommitFilesResult {
        files,
        has_more,
        timed_out: false,
    })
}

/// `git show/diff-tree --name-status -z` 출력을 파싱한다. `-z`에서 레코드는
/// `STATUS\0경로\0` 반복이며, rename/copy(R/C)만 `STATUS\0원본\0새경로\0`로 경로가
/// 둘 붙는다 — 이 경우 새 경로를 표시 경로로 쓴다. 병합 combined 상태("MM" 등)는
/// 첫 글자만 취한다. 파싱 상한(COMMIT_FILES_PARSE_CAP)에 걸리면 멈춘다.
fn parse_name_status(bytes: &[u8]) -> Vec<GitCommitFileEntry> {
    let toks: Vec<&[u8]> = bytes.split(|&b| b == 0).filter(|t| !t.is_empty()).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        if out.len() >= COMMIT_FILES_PARSE_CAP {
            break;
        }
        let status_raw = String::from_utf8_lossy(toks[i]);
        i += 1;
        // 상태 코드의 첫 글자(예: "R097" → 'R', "MM" → 'M').
        let code = status_raw.chars().next().unwrap_or('?');
        let path = if code == 'R' || code == 'C' {
            // rename/copy: 다음 두 토큰이 (원본, 새 경로). 새 경로를 표시.
            let _orig = toks.get(i);
            i += 1;
            match toks.get(i) {
                Some(p) => {
                    i += 1;
                    String::from_utf8_lossy(p).into_owned()
                }
                None => break, // 짝이 안 맞으면 중단(방어).
            }
        } else {
            match toks.get(i) {
                Some(p) => {
                    i += 1;
                    String::from_utf8_lossy(p).into_owned()
                }
                None => break,
            }
        };
        if path.is_empty() {
            continue;
        }
        out.push(GitCommitFileEntry {
            path,
            status: code.to_string(),
        });
    }
    out
}

/// 저장소 전체 커밋 로그를 `git log`로 가져온다(파일 pathspec/`--follow` 없음).
/// `all_branches`면 `--all`로 모든 참조를, `query`가 있으면 커밋 메시지를
/// 대소문자 무시·고정 문자열로 필터(`--grep`)한다. 결과 타입은 파일 히스토리와
/// 같은 `GitFileHistoryResult`를 재사용한다.
pub fn git_repo_log(
    root: &str,
    limit: usize,
    skip: usize,
    all_branches: bool,
    query: &str,
) -> Result<GitFileHistoryResult, String> {
    let canon_root = canon_dir(root)?;
    let limit = limit.clamp(1, HISTORY_MAX_LIMIT);
    let n_arg = format!("-n{limit}");
    let skip_arg = format!("--skip={skip}");
    // `--grep=<q>`는 값이 옵션에 묶여 주입이 불가하고, `-F`(고정 문자열)·`-i`
    // (대소문자 무시)로 예측 가능한 부분일치 검색을 한다.
    let grep_arg = format!("--grep={query}");
    let mut args: Vec<&str> = vec![
        "log",
        "--date=format:%Y-%m-%d %H:%M",
        "--format=%H%x1f%h%x1f%an%x1f%ad%x1f%s",
        "-z",
        &n_arg,
        &skip_arg,
    ];
    if all_branches {
        args.push("--all");
    }
    let has_query = !query.is_empty();
    if has_query {
        args.push(&grep_arg);
        args.push("-i");
        args.push("-F");
    }
    let run = run_git(&canon_root, &args, GIT_QUERY_TIMEOUT);
    if run.spawn_failed {
        return Err("git 실행에 실패했습니다(설치 여부 확인)".to_string());
    }
    if run.timed_out {
        return Ok(GitFileHistoryResult {
            commits: Vec::new(),
            has_more: false,
            timed_out: true,
        });
    }
    // 비 git 저장소/빈 저장소는 exit non-zero거나 빈 출력 → 빈 목록으로 귀결.
    Ok(parse_history(&run.stdout, limit))
}

/// 외부 비교 도구(`git difftool`)를 fire-and-forget으로 띄운다. GUI 도구가
/// 설정돼 있어야 의미가 있고, 미설정이면 백그라운드에서 조용히 실패한다(인앱
/// diff가 항상 폴백이므로 여기서는 spawn 성공만 확인). `commit`이 있으면 그
/// 커밋의 변경을, 없으면 `mode`에 따른 현재 변경을 연다.
pub fn launch_difftool(
    root: &str,
    rel_path: &str,
    mode: &str,
    commit: Option<&str>,
) -> Result<(), String> {
    let canon_root = canon_dir(root)?;
    let rel = sanitize_rel_path(rel_path)?;
    let mut args: Vec<String> = vec!["difftool".to_string(), "-y".to_string()];
    match commit {
        Some(c) => {
            if !valid_commit(c) {
                return Err(format!("잘못된 커밋 해시입니다: {c}"));
            }
            // "<hash>^!"는 그 커밋 하나의 변경(부모↔커밋)을 뜻한다.
            args.push(format!("{c}^!"));
        }
        None => match mode {
            "indexVsHead" => args.push("--cached".to_string()),
            "worktreeVsHead" => args.push("HEAD".to_string()),
            // worktreeVsIndex(기본)·untracked 등은 추가 rev 없이 워킹트리 비교.
            _ => {}
        },
    }
    args.push("--".to_string());
    args.push(rel);

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut cmd = Command::new("git");
    cmd.current_dir(&canon_root)
        .args(&arg_refs)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("외부 비교 도구 실행 실패: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_diff_flags_binary() {
        let d = finalize_diff(b"Binary files a/x.png and b/x.png differ\n");
        assert!(d.binary);
        assert!(!d.truncated);
    }

    #[test]
    fn finalize_diff_truncates_by_lines() {
        let mut big = String::new();
        for i in 0..(MAX_DIFF_LINES + 100) {
            big.push_str(&format!("+line {i}\n"));
        }
        let d = finalize_diff(big.as_bytes());
        assert!(d.truncated);
        assert_eq!(d.diff.lines().count(), MAX_DIFF_LINES);
    }

    #[test]
    fn parse_history_splits_records_and_fields() {
        // 필드 US(0x1f), 레코드 NUL. 2개 커밋.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(
            "abc1234def5678\u{1f}abc1234\u{1f}Alice\u{1f}2026-01-01 10:00\u{1f}first subject"
                .as_bytes(),
        );
        bytes.push(0);
        bytes.extend_from_slice(
            "9990000aaaa\u{1f}9990000\u{1f}Bob\u{1f}2026-02-02 11:11\u{1f}second".as_bytes(),
        );
        bytes.push(0);
        let r = parse_history(&bytes, 50);
        assert_eq!(r.commits.len(), 2);
        assert_eq!(r.commits[0].hash, "abc1234def5678");
        assert_eq!(r.commits[0].short_hash, "abc1234");
        assert_eq!(r.commits[0].author, "Alice");
        assert_eq!(r.commits[0].date, "2026-01-01 10:00");
        assert_eq!(r.commits[0].subject, "first subject");
        assert_eq!(r.commits[1].author, "Bob");
        assert!(!r.has_more); // 2 < 50
    }

    #[test]
    fn parse_history_has_more_when_full() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice("aaaaaaa\u{1f}aaaaaaa\u{1f}A\u{1f}d\u{1f}s".as_bytes());
        bytes.push(0);
        let r = parse_history(&bytes, 1);
        assert_eq!(r.commits.len(), 1);
        assert!(r.has_more); // 1 >= limit(1)
    }

    #[test]
    fn unknown_diff_mode_is_error() {
        let root = env!("CARGO_MANIFEST_DIR");
        assert!(git_diff_file(root, "Cargo.toml", "bogus").is_err());
    }

    #[test]
    fn diff_out_of_root_is_error() {
        let root = env!("CARGO_MANIFEST_DIR");
        assert!(git_diff_file(root, "../../../etc/passwd", "worktreeVsHead").is_err());
        assert!(git_file_history(root, "../escape", 10, 0).is_err());
        assert!(git_diff_commit(root, "0000000", "../escape").is_err());
    }

    #[test]
    fn diff_commit_rejects_bad_hash() {
        let root = env!("CARGO_MANIFEST_DIR");
        // 유효하지 않은 해시는 sanitize 이전에 거부.
        assert!(git_diff_commit(root, "not-a-hash", "Cargo.toml").is_err());
    }

    /// 실제 저장소에서 파일 히스토리를 조회하는 스모크. workdir.rs는 커밋
    /// 이력이 있으므로 최소 1건 이상 나와야 한다.
    #[test]
    fn this_repo_file_history_smoke() {
        let root = env!("CARGO_MANIFEST_DIR");
        let r = git_file_history(root, "src/workdir.rs", 10, 0).unwrap();
        assert!(!r.timed_out);
        assert!(
            !r.commits.is_empty(),
            "workdir.rs는 커밋 이력이 있어야 함"
        );
        // 해시는 hex 40자여야 한다.
        assert!(valid_commit(&r.commits[0].hash));
    }

    /// 특정 커밋(#11 도입 커밋)의 workdir.rs diff를 뽑는 스모크.
    #[test]
    fn this_repo_diff_commit_smoke() {
        let root = env!("CARGO_MANIFEST_DIR");
        let r =
            git_diff_commit(root, "dd7c2d861e6c0619e58bed7340efebe2ae7915db", "src/workdir.rs")
                .unwrap();
        assert!(!r.timed_out);
        // 그 커밋이 workdir.rs를 새로 추가했으므로 diff에 파일 헤더가 있어야 한다.
        assert!(r.diff.contains("diff --git") || r.diff.contains("new file"));
    }

    #[test]
    fn parse_name_status_basic_and_rename() {
        // "M\0a.rs\0A\0b.rs\0R097\0old.md\0new.md\0" — 마지막은 rename(원본+새경로).
        let mut bytes = Vec::new();
        for t in ["M", "a.rs", "A", "b.rs", "R097", "old.md", "new.md"] {
            bytes.extend_from_slice(t.as_bytes());
            bytes.push(0);
        }
        let files = parse_name_status(&bytes);
        assert_eq!(files.len(), 3);
        assert_eq!(files[0], GitCommitFileEntry { path: "a.rs".into(), status: "M".into() });
        assert_eq!(files[1], GitCommitFileEntry { path: "b.rs".into(), status: "A".into() });
        // rename은 새 경로를 표시하고 상태는 'R'.
        assert_eq!(files[2], GitCommitFileEntry { path: "new.md".into(), status: "R".into() });
    }

    #[test]
    fn parse_name_status_combined_takes_first_char() {
        // 병합 combined 상태 "MM"은 첫 글자 'M'로.
        let mut bytes = Vec::new();
        for t in ["MM", "merged.rs"] {
            bytes.extend_from_slice(t.as_bytes());
            bytes.push(0);
        }
        let files = parse_name_status(&bytes);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, "M");
        assert_eq!(files[0].path, "merged.rs");
    }

    #[test]
    fn parse_name_status_empty_is_empty() {
        assert!(parse_name_status(&[]).is_empty());
    }

    #[test]
    fn commit_files_rejects_bad_hash() {
        let root = env!("CARGO_MANIFEST_DIR");
        assert!(git_commit_files(root, "not-a-hash", 100, 0).is_err());
    }

    /// 실제 커밋(#11 도입, dd7c2d8)의 변경파일 목록 스모크. 그 커밋은 workdir.rs를
    /// 새로 추가했으므로 목록에 status 'A'로 나와야 한다.
    #[test]
    fn this_repo_commit_files_smoke() {
        let root = env!("CARGO_MANIFEST_DIR");
        let r = git_commit_files(root, "dd7c2d861e6c0619e58bed7340efebe2ae7915db", 500, 0).unwrap();
        assert!(!r.timed_out);
        assert!(
            r.files.iter().any(|f| f.path == "src-tauri/src/workdir.rs" && f.status == "A"),
            "workdir.rs가 status A로 있어야 함: {:?}",
            r.files
        );
    }

    /// 커밋 변경파일 페이지네이션: limit=1이면 첫 1건 + has_more.
    #[test]
    fn this_repo_commit_files_paginates() {
        let root = env!("CARGO_MANIFEST_DIR");
        let p0 = git_commit_files(root, "dd7c2d861e6c0619e58bed7340efebe2ae7915db", 1, 0).unwrap();
        assert_eq!(p0.files.len(), 1);
        assert!(p0.has_more, "이 커밋은 파일이 여러 개라 has_more여야 함");
        let p1 = git_commit_files(root, "dd7c2d861e6c0619e58bed7340efebe2ae7915db", 1, 1).unwrap();
        assert_eq!(p1.files.len(), 1);
        assert_ne!(p0.files[0].path, p1.files[0].path, "skip이 다음 파일을 줘야 함");
    }

    /// 리포 전체 로그 스모크: 최근 커밋이 최신순으로 나오고 has_more 페이징.
    #[test]
    fn this_repo_repo_log_smoke() {
        let root = env!("CARGO_MANIFEST_DIR");
        let r = git_repo_log(root, 5, 0, false, "").unwrap();
        assert!(!r.timed_out);
        assert_eq!(r.commits.len(), 5);
        assert!(r.has_more);
        assert!(valid_commit(&r.commits[0].hash));
    }

    /// 리포 로그 검색: 존재하는 부분 문자열로 grep하면 결과가 나오고, 없는
    /// 문자열이면 빈 목록.
    #[test]
    fn this_repo_repo_log_search() {
        let root = env!("CARGO_MANIFEST_DIR");
        // 커밋 메시지에 흔한 접두 "feat" 검색(대소문자 무시).
        let hit = git_repo_log(root, 50, 0, false, "FEAT").unwrap();
        assert!(!hit.commits.is_empty(), "feat 커밋이 있어야 함");
        assert!(hit.commits.iter().all(|c| c.subject.to_lowercase().contains("feat")));
        // 존재하지 않을 매우 특이한 문자열.
        let miss = git_repo_log(root, 50, 0, false, "zzz_no_such_commit_xyzzy_qqq").unwrap();
        assert!(miss.commits.is_empty());
    }
}
