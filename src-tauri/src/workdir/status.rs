// src-tauri/src/workdir/status.rs
//
// `git status --porcelain=v2 --branch -z` 조회와 그 출력을 파싱하는 파서
// 계열. git 바이너리 부재·비(非) git 저장소·타임아웃·사용자 취소는 모두 에러가
// 아니라 정상 응답의 필드(is_repo=false / timed_out=true / canceled=true)로
// 표현한다.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::git_runner::{register_cancel, run_git};
use super::model::{GitBranchResult, GitFileStatus, GitStatusResult};

/// git status subprocess 타임아웃. **UX 제한이 아니라 폭주 방지 백스톱이다** --
/// 거대 저장소에서 status가 수십 초 걸리는 건 정상이라 짧은 상한은 "무조건
/// 실패"만 낳는다. 1차 탈출구는 사용자 취소(`op_id` + `workdir_git_cancel`)이고,
/// 이 값은 취소도 하지 않은 채 영영 매달린 자식을 결국 정리하는 마지막 그물이다.
const GIT_STATUS_TIMEOUT: Duration = Duration::from_secs(120);

/// 파싱해서 담을 엔트리 상한(이슈 #70). `--untracked-files=all`로 미추적
/// 디렉터리를 파일 단위로 펼치면서, gitignore되지 않은 대량 산출물 폴더가
/// 있으면 수만 건이 나올 수 있다 -- IPC 직렬화·렌더가 무거워지지 않도록
/// listing.rs의 MAX_LIST와 같은 5000개에서 자르고 `truncated`를 세운다.
const MAX_STATUS_ENTRIES: usize = 5000;

/// 라벨용 브랜치 조회 타임아웃. `git rev-parse --abbrev-ref HEAD`는 인덱스나
/// 워킹트리를 훑지 않고 `.git/HEAD`만 읽어 거대 저장소에서도 즉답이라, status의
/// 분 단위 백스톱(취소 UI가 딸린)과 달리 짧게 잡는다 -- 이쪽은 사용자가 부른
/// 조회가 아니라 30초 주기 폴링이고, 표시 못 하면 브랜치를 생략할 뿐이다.
const GIT_BRANCH_TIMEOUT: Duration = Duration::from_secs(2);

/// root의 git 상태를 조회한다. 저장소가 아니거나 git이 없으면 is_repo=false,
/// 타임아웃이면 timed_out=true, 사용자가 취소했으면 canceled=true인 정상 응답을
/// 돌려준다(에러 문자열은 root가 아예 없는 등 조회 이전 단계 실패에서만 반환).
///
/// `op_id`를 주면 그 id로 취소 플래그를 등록해, 조회가 오래 걸릴 때 프런트가
/// `workdir_git_cancel`로 중단시킬 수 있다. 등록은 이 함수가 끝나면 자동 해제된다.
pub fn collect_git_status(root: &str, op_id: Option<&str>) -> Result<GitStatusResult, String> {
    let canon_root = std::fs::canonicalize(root)
        .map_err(|e| format!("작업 폴더를 찾을 수 없습니다: {root} ({e})"))?;
    if !canon_root.is_dir() {
        return Err(format!("작업 폴더가 디렉터리가 아닙니다: {root}"));
    }
    let cancel = register_cancel(op_id);
    Ok(run_git_status(
        &canon_root,
        GIT_STATUS_TIMEOUT,
        Some(cancel.flag()),
    ))
}

/// root의 현재 브랜치명만 가볍게 조회한다(라벨 표면의 "프로젝트 (브랜치)").
/// `collect_git_status`와 달리 취소(opId)를 받지 않는다 -- 30초 주기 폴링이라
/// 사용자가 끊을 UI 자체가 없고, 조회도 `.git/HEAD` 한 번 읽기로 끝난다.
///
/// 폴더가 없거나·git이 없거나·비저장소거나·타임아웃이면 전부 `is_repo=false`인
/// 정상 응답이다(에러 반환 없음) -- 폴링 경로라 실패를 에러로 올려봐야 콘솔만
/// 시끄럽고, 호출부가 할 일은 어느 경우든 "브랜치 생략"으로 같다.
pub fn collect_git_branch(root: &str) -> GitBranchResult {
    let Ok(canon_root) = std::fs::canonicalize(root) else {
        return GitBranchResult::not_repo();
    };
    if !canon_root.is_dir() {
        return GitBranchResult::not_repo();
    }
    let run = run_git(
        &canon_root,
        &["rev-parse", "--abbrev-ref", "HEAD"],
        GIT_BRANCH_TIMEOUT,
        None,
    );
    // spawn 실패(git 부재)·타임아웃·non-zero(비저장소, 커밋 없는 새 저장소) 모두
    // 브랜치를 표시하지 않는다. canceled는 취소 플래그를 안 넘겼으니 나올 수 없다.
    if run.spawn_failed || run.timed_out || run.canceled || !run.success {
        return GitBranchResult::not_repo();
    }
    parse_abbrev_ref(&run.stdout)
}

/// `git rev-parse --abbrev-ref HEAD` 출력을 브랜치명으로 해석한다. detached
/// HEAD면 git이 브랜치명 대신 문자열 `HEAD`를 그대로 뱉으므로 branch=None으로
/// 접는다(빈 출력도 같은 취급). 여기 오는 건 exit 0인 출력뿐이라 is_repo는 항상 true.
pub fn parse_abbrev_ref(stdout: &[u8]) -> GitBranchResult {
    let name = String::from_utf8_lossy(stdout).trim().to_string();
    if name.is_empty() || name == "HEAD" {
        return GitBranchResult {
            is_repo: true,
            branch: None,
        };
    }
    GitBranchResult {
        is_repo: true,
        branch: Some(name),
    }
}

/// `git status --porcelain=v2 --branch -z --untracked-files=all`을 root에서
/// 실행하고 결과를 파싱한다. 타임아웃 초과 시 자식을 죽이고 timed_out 응답을
/// 돌려준다. 실행/파이프 처리는 공용 `run_git`에 위임한다.
///
/// `--untracked-files=all`(이슈 #70): 기본값 `normal`은 미추적 디렉터리를 접어
/// `? newfolder/` 한 줄로만 보고하기 때문에, 새로 추가된 폴더 안의 개별 파일이
/// 목록에도 안 뜨고 diff(`--no-index`)도 디렉터리를 가리켜 비어 버린다. `all`로
/// 펼치면 파일 단위 `? newfolder/a.txt` 레코드가 나와 기존 흐름이 그대로 성립한다.
fn run_git_status(root: &Path, timeout: Duration, cancel: Option<&AtomicBool>) -> GitStatusResult {
    let run = run_git(
        root,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ],
        timeout,
        cancel,
    );
    // git 바이너리 부재 등 -- 저장소 아님으로 취급(뱃지 조용히 생략).
    if run.spawn_failed {
        return GitStatusResult::not_repo();
    }
    // 사용자 취소(타임아웃보다 먼저 -- 사유가 더 구체적이다).
    if run.canceled {
        return GitStatusResult::canceled();
    }
    // 타임아웃.
    if run.timed_out {
        return GitStatusResult::timed_out();
    }
    if run.success {
        // exit 0: 정상 파싱.
        parse_porcelain_v2(&run.stdout)
    } else {
        // non-zero: 비 git 저장소(혹은 기타 git 에러) -- 뱃지 생략.
        GitStatusResult::not_repo()
    }
}

/// `git status --porcelain=v2 --branch -z` 출력을 파싱한다. 레코드는 NUL로
/// 구분되며, rename(type 2) 레코드만 예외적으로 경로 뒤에 원본경로가 NUL로 한
/// 필드 더 붙는다 -- 그래서 토큰을 순회하며 type 2를 만나면 다음 토큰 하나를
/// 원본경로로 소비한다.
///
/// 참고 포맷:
/// - `# branch.head <name>` / `# branch.ab +N -M`
/// - `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`           (일반 변경)
/// - `2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <Xscore> <path>`  (rename/copy; +원본경로)
/// - `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>` (충돌)
/// - `? <path>`  (untracked)  /  `! <path>` (ignored; 스킵)
///
/// 엔트리가 `MAX_STATUS_ENTRIES`에 닿으면 거기서 멈추고 `truncated`를 세운다.
/// 브랜치 헤더(`# ...`)는 항상 엔트리보다 앞에 오므로 중단해도 손실이 없다.
pub fn parse_porcelain_v2(bytes: &[u8]) -> GitStatusResult {
    let mut result = GitStatusResult {
        is_repo: true,
        branch: None,
        ahead: 0,
        behind: 0,
        entries: Vec::new(),
        timed_out: false,
        canceled: false,
        truncated: false,
    };

    let tokens: Vec<&[u8]> = bytes
        .split(|&b| b == 0)
        .filter(|t| !t.is_empty())
        .collect();

    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        match tok.first() {
            Some(b'#') => {
                let line = String::from_utf8_lossy(tok);
                if let Some(rest) = line.strip_prefix("# branch.head ") {
                    let name = rest.trim();
                    // detached HEAD는 "(detached)" 라고 나온다 -- 브랜치 없음.
                    result.branch = if name == "(detached)" || name.is_empty() {
                        None
                    } else {
                        Some(name.to_string())
                    };
                } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
                    // "+N -M" 형태.
                    let mut parts = rest.split_whitespace();
                    if let Some(a) = parts.next() {
                        result.ahead = a.trim_start_matches('+').parse().unwrap_or(0);
                    }
                    if let Some(b) = parts.next() {
                        result.behind = b.trim_start_matches('-').parse().unwrap_or(0);
                    }
                }
            }
            Some(b'1') | Some(b'u') => {
                if let Some((xy, path)) = parse_changed_entry(tok) {
                    result.entries.push(make_status(xy, path));
                }
            }
            Some(b'2') => {
                if let Some((xy, path)) = parse_changed_entry(tok) {
                    result.entries.push(make_status(xy, path));
                }
                // rename/copy는 다음 토큰이 원본경로 -- 소비만 하고 버린다.
                i += 1;
            }
            // "? <path>": 앞 2바이트("? ") 제거. 경로가 없으면(있을 수 없지만) 스킵.
            Some(b'?') if tok.len() > 2 => {
                let path = String::from_utf8_lossy(&tok[2..]).into_owned();
                result.entries.push(GitFileStatus {
                    path,
                    status: "?".to_string(),
                    xy: "??".to_string(),
                });
            }
            // '!'(ignored) 및 알 수 없는 라인은 스킵.
            _ => {}
        }
        i += 1;
        // 상한 도달: 남은 레코드는 버리고 절단 표시(이슈 #70).
        if result.entries.len() >= MAX_STATUS_ENTRIES {
            result.truncated = i < tokens.len();
            break;
        }
    }

    result
}

/// type 1/2/u 레코드에서 (XY 2글자, 경로)를 뽑는다. 경로는 공백을 포함할 수
/// 있으므로 "마지막 필드"로 취급한다. type 2는 XY 뒤 필드 수가 하나 더(Xscore)
/// 많지만, "경로 = 마지막 공백 이후 전체"라 필드 개수와 무관하게 처리된다.
fn parse_changed_entry(tok: &[u8]) -> Option<(String, String)> {
    let s = String::from_utf8_lossy(tok);
    let mut parts = s.splitn(3, ' ');
    let _kind = parts.next()?; // '1' | '2' | 'u'
    let xy = parts.next()?; // "MD" 등 2글자
    let rest = parts.next()?; // "<sub> ... <path>"
    // 경로는 마지막 공백 이후 전체. rsplit 한 번으로 뒤 필드만 떼면 경로 중간의
    // 공백이 보존된다: rest = "N... <path>" 에서 rsplitn(?, ' ')는 부적절하므로,
    // 필드 개수만큼 앞에서 건너뛴다.
    let path = skip_fixed_fields(rest, xy.as_bytes(), tok.first())?;
    Some((xy.to_string(), path))
}

/// `rest`(= XY 다음부터)에서 고정 메타 필드를 건너뛰고 경로만 돌려준다.
/// 고정 필드 개수: type 1 → 6(sub,mH,mI,mW,hH,hI), type 2 → 7(+Xscore),
/// type u → 8(sub,m1,m2,m3,mW,h1,h2,h3). 경로는 그 뒤 전체(공백 포함).
fn skip_fixed_fields(rest: &str, _xy: &[u8], kind: Option<&u8>) -> Option<String> {
    let fixed = match kind {
        Some(b'1') => 6,
        Some(b'2') => 7,
        Some(b'u') => 8,
        _ => return None,
    };
    // fixed개 필드를 공백으로 건너뛰고 나머지 전부를 경로로.
    let mut it = rest.splitn(fixed + 1, ' ');
    for _ in 0..fixed {
        it.next()?;
    }
    let path = it.next()?;
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

/// XY(스테이지 X + 워킹트리 Y)에서 표시용 단일 뱃지 문자를 고른다: 워킹트리
/// 쪽(Y)이 변경돼 있으면 Y, 아니면 스테이지 쪽(X). 충돌(u 레코드)은 XY가 둘 다
/// 알파벳이라 그대로 첫 글자가 잡히지만, 표시는 'U'로 통일한다.
fn make_status(xy: String, path: String) -> GitFileStatus {
    let x = xy.chars().next().unwrap_or('.');
    let y = xy.chars().nth(1).unwrap_or('.');
    // 충돌 상태(양쪽 다 대문자이고 unmerged 조합)는 'U'로.
    let is_conflict = matches!(
        (x, y),
        ('D', 'D') | ('A', 'A') | ('U', _) | (_, 'U')
    );
    let status = if is_conflict {
        'U'
    } else if y != '.' {
        y
    } else {
        x
    };
    GitFileStatus {
        path,
        status: status.to_string(),
        xy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 토큰들을 NUL로 이어 porcelain -z 출력 바이트를 만든다(끝에도 NUL).
    fn nul_join(tokens: &[&str]) -> Vec<u8> {
        let mut v = Vec::new();
        for t in tokens {
            v.extend_from_slice(t.as_bytes());
            v.push(0);
        }
        v
    }

    /// 라벨용 브랜치 파서: 흔한 이름·슬래시 포함 이름 모두 그대로 통과하고,
    /// 트레일링 개행은 벗겨진다.
    #[test]
    fn abbrev_ref_parses_branch_name() {
        let r = parse_abbrev_ref(b"main\n");
        assert!(r.is_repo);
        assert_eq!(r.branch.as_deref(), Some("main"));
        assert_eq!(
            parse_abbrev_ref(b"feature/label-branch\n").branch.as_deref(),
            Some("feature/label-branch")
        );
    }

    /// detached HEAD면 git이 브랜치명 대신 "HEAD"를 뱉는다 -- 저장소이긴 하나
    /// 표시할 브랜치는 없다. 빈 출력도 같은 취급.
    #[test]
    fn abbrev_ref_detached_head_has_no_branch() {
        let r = parse_abbrev_ref(b"HEAD\n");
        assert!(r.is_repo);
        assert_eq!(r.branch, None);
        let empty = parse_abbrev_ref(b"  \n");
        assert!(empty.is_repo);
        assert_eq!(empty.branch, None);
    }

    /// 없는 폴더는 에러가 아니라 "브랜치 없음" 정상 응답이다(폴링 경로).
    #[test]
    fn collect_git_branch_missing_root_is_not_repo() {
        let r = collect_git_branch("/definitely/not/a/real/path/agent-office");
        assert!(!r.is_repo);
        assert_eq!(r.branch, None);
    }

    #[test]
    fn parses_branch_and_ab() {
        let bytes = nul_join(&[
            "# branch.oid abc123",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -3",
        ]);
        let r = parse_porcelain_v2(&bytes);
        assert!(r.is_repo);
        assert_eq!(r.branch.as_deref(), Some("main"));
        assert_eq!(r.ahead, 2);
        assert_eq!(r.behind, 3);
        assert!(r.entries.is_empty());
    }

    #[test]
    fn detached_head_has_no_branch() {
        let bytes = nul_join(&["# branch.head (detached)", "# branch.ab +0 -0"]);
        let r = parse_porcelain_v2(&bytes);
        assert_eq!(r.branch, None);
    }

    #[test]
    fn parses_ordinary_modified_entry() {
        // 워킹트리 수정(스테이지 안 됨): XY = ".M".
        let bytes = nul_join(&["1 .M N... 100644 100644 100644 aaa bbb src/lib.rs"]);
        let r = parse_porcelain_v2(&bytes);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].path, "src/lib.rs");
        assert_eq!(r.entries[0].status, "M");
        assert_eq!(r.entries[0].xy, ".M");
    }

    #[test]
    fn staged_added_uses_x_when_worktree_clean() {
        // 스테이지된 추가(워킹트리 클린): XY = "A.".
        let bytes = nul_join(&["1 A. N... 000000 100644 100644 000 bbb new.txt"]);
        let r = parse_porcelain_v2(&bytes);
        assert_eq!(r.entries[0].status, "A");
        assert_eq!(r.entries[0].xy, "A.");
    }

    #[test]
    fn path_with_spaces_is_preserved() {
        let bytes = nul_join(&["1 .M N... 100644 100644 100644 aaa bbb my dir/a b.txt"]);
        let r = parse_porcelain_v2(&bytes);
        assert_eq!(r.entries[0].path, "my dir/a b.txt");
        assert_eq!(r.entries[0].status, "M");
    }

    #[test]
    fn rename_entry_consumes_orig_path_token() {
        // type 2 뒤에는 원본경로 토큰이 하나 더 온다. 그 뒤 일반 엔트리가
        // 정상적으로 이어져야 파싱 오프셋이 맞는 것.
        let bytes = nul_join(&[
            "2 R. N... 100644 100644 100644 aaa bbb R100 new/name.rs",
            "old/name.rs",
            "1 .M N... 100644 100644 100644 ccc ddd other.rs",
        ]);
        let r = parse_porcelain_v2(&bytes);
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[0].path, "new/name.rs");
        assert_eq!(r.entries[0].status, "R");
        assert_eq!(r.entries[1].path, "other.rs");
        assert_eq!(r.entries[1].status, "M");
    }

    #[test]
    fn untracked_and_ignored() {
        let bytes = nul_join(&["? untracked.txt", "! ignored.txt"]);
        let r = parse_porcelain_v2(&bytes);
        // untracked만 잡히고 ignored는 스킵.
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].path, "untracked.txt");
        assert_eq!(r.entries[0].status, "?");
        assert_eq!(r.entries[0].xy, "??");
    }

    /// 이슈 #70: `--untracked-files=all`이면 새 폴더가 접히지 않고 내부 파일이
    /// 각각 `? <경로>` 레코드로 온다 -- 파서는 그대로 파일 단위 엔트리를 만든다.
    #[test]
    fn untracked_directory_expands_to_individual_files() {
        let bytes = nul_join(&[
            "? docs/new/a.md",
            "? docs/new/b.md",
            "? docs/new/deep/nested/c.md",
        ]);
        let r = parse_porcelain_v2(&bytes);
        assert_eq!(r.entries.len(), 3);
        let paths: Vec<&str> = r.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["docs/new/a.md", "docs/new/b.md", "docs/new/deep/nested/c.md"]);
        assert!(r.entries.iter().all(|e| e.status == "?"));
        assert!(!r.truncated);
    }

    /// 이슈 #70: 엔트리가 상한을 넘으면 상한까지만 담고 truncated를 세운다.
    #[test]
    fn entries_are_capped_and_flagged() {
        let mut tokens: Vec<String> = vec!["# branch.head main".to_string()];
        for i in 0..(MAX_STATUS_ENTRIES + 10) {
            tokens.push(format!("? junk/f{i}.txt"));
        }
        let refs: Vec<&str> = tokens.iter().map(|s| s.as_str()).collect();
        let r = parse_porcelain_v2(&nul_join(&refs));
        assert_eq!(r.entries.len(), MAX_STATUS_ENTRIES);
        assert!(r.truncated);
        // 브랜치 헤더는 엔트리보다 앞에 오므로 절단돼도 살아 있어야 한다.
        assert_eq!(r.branch.as_deref(), Some("main"));
    }

    /// 상한과 정확히 같은 개수면 절단이 아니다(경계).
    #[test]
    fn exactly_at_cap_is_not_truncated() {
        let tokens: Vec<String> = (0..MAX_STATUS_ENTRIES)
            .map(|i| format!("? junk/f{i}.txt"))
            .collect();
        let refs: Vec<&str> = tokens.iter().map(|s| s.as_str()).collect();
        let r = parse_porcelain_v2(&nul_join(&refs));
        assert_eq!(r.entries.len(), MAX_STATUS_ENTRIES);
        assert!(!r.truncated);
    }

    #[test]
    fn unmerged_entry_maps_to_u() {
        // 충돌: u 레코드, XY = "UU".
        let bytes =
            nul_join(&["u UU N... 100644 100644 100644 100644 aaa bbb ccc conflict.rs"]);
        let r = parse_porcelain_v2(&bytes);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].path, "conflict.rs");
        assert_eq!(r.entries[0].status, "U");
    }

    #[test]
    fn deleted_entry() {
        // 워킹트리 삭제: XY = " D" 아님 -- v2는 ".D".
        let bytes = nul_join(&["1 .D N... 100644 100644 000000 aaa bbb gone.rs"]);
        let r = parse_porcelain_v2(&bytes);
        assert_eq!(r.entries[0].status, "D");
    }

    #[test]
    fn empty_output_is_clean_repo() {
        let r = parse_porcelain_v2(&[]);
        assert!(r.is_repo);
        assert!(r.entries.is_empty());
        assert_eq!(r.branch, None);
    }

    /// 실제 이 저장소에서 git status를 호출하는 스모크(호스트 git 검증용).
    #[test]
    fn this_repo_is_detected_as_git() {
        let root = env!("CARGO_MANIFEST_DIR");
        let r = collect_git_status(root, None).unwrap();
        assert!(r.is_repo, "이 크레이트는 git 저장소여야 함");
        assert!(!r.timed_out);
        assert!(!r.canceled);
    }

    /// 이미 서 있는 취소 플래그로 들어오면 status는 에러가 아니라
    /// `canceled=true` 정상 응답이 된다(timed_out과 구분).
    #[test]
    fn canceled_run_reports_canceled_not_timed_out() {
        use std::sync::atomic::AtomicBool;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let cancel = AtomicBool::new(true);
        let r = run_git_status(root, GIT_STATUS_TIMEOUT, Some(&cancel));
        assert!(r.canceled);
        assert!(!r.timed_out);
        assert!(r.is_repo);
        assert!(r.entries.is_empty());
    }

    /// op_id 레지스트리를 통한 취소가 실제 조회 결과까지 닿는지: 등록 →
    /// `request_cancel` → 그 플래그로 조회 → canceled 응답.
    #[test]
    fn cancel_via_registry_reaches_the_query() {
        use super::super::git_runner::{register_cancel, request_cancel};
        let guard = register_cancel(Some("test-status-cancel-op"));
        request_cancel("test-status-cancel-op");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let r = run_git_status(root, GIT_STATUS_TIMEOUT, Some(guard.flag()));
        assert!(r.canceled);
    }
}
