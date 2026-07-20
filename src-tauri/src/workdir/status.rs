// src-tauri/src/workdir/status.rs
//
// `git status --porcelain=v2 --branch -z` 조회와 그 출력을 파싱하는 파서
// 계열. git 바이너리 부재·비(非) git 저장소·타임아웃은 모두 에러가 아니라
// 정상 응답의 필드(is_repo=false / timed_out=true)로 표현한다.

use std::path::Path;
use std::time::Duration;

use super::git_runner::run_git;
use super::model::{GitFileStatus, GitStatusResult};

/// git status subprocess 타임아웃. 거대 저장소에서 UI가 멈추지 않도록 이 시간을
/// 넘기면 자식을 죽이고 `timed_out`을 세운다.
const GIT_STATUS_TIMEOUT: Duration = Duration::from_secs(3);

/// root의 git 상태를 조회한다. 저장소가 아니거나 git이 없으면 is_repo=false,
/// 타임아웃이면 timed_out=true인 정상 응답을 돌려준다(에러 문자열은 root가 아예
/// 없는 등 조회 이전 단계 실패에서만 반환).
pub fn collect_git_status(root: &str) -> Result<GitStatusResult, String> {
    let canon_root = std::fs::canonicalize(root)
        .map_err(|e| format!("작업 폴더를 찾을 수 없습니다: {root} ({e})"))?;
    if !canon_root.is_dir() {
        return Err(format!("작업 폴더가 디렉터리가 아닙니다: {root}"));
    }
    Ok(run_git_status(&canon_root, GIT_STATUS_TIMEOUT))
}

/// `git status --porcelain=v2 --branch -z`를 root에서 실행하고 결과를 파싱한다.
/// 타임아웃 초과 시 자식을 죽이고 timed_out 응답을 돌려준다. 실행/파이프 처리는
/// 공용 `run_git`에 위임한다.
fn run_git_status(root: &Path, timeout: Duration) -> GitStatusResult {
    let run = run_git(
        root,
        &["status", "--porcelain=v2", "--branch", "-z"],
        timeout,
    );
    // git 바이너리 부재 등 -- 저장소 아님으로 취급(뱃지 조용히 생략).
    if run.spawn_failed {
        return GitStatusResult::not_repo();
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
pub fn parse_porcelain_v2(bytes: &[u8]) -> GitStatusResult {
    let mut result = GitStatusResult {
        is_repo: true,
        branch: None,
        ahead: 0,
        behind: 0,
        entries: Vec::new(),
        timed_out: false,
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
        let r = collect_git_status(root).unwrap();
        assert!(r.is_repo, "이 크레이트는 git 저장소여야 함");
        assert!(!r.timed_out);
    }
}
