// src-tauri/src/workdir/listing.rs
//
// 작업 폴더 파일 목록 스캔. markdown.rs와 공유하는 `file_scan::walk_files`(병렬
// WalkBuilder)에 위임한다 -- `.gitignore`를 존중하고 hidden을 스킵하며
// 심링크는 따라가지 않는다. markdown.rs와 다른 점은 확장자 필터가 없다는
// 것뿐이라 `ext_filter: None`으로 호출한다.
//
// 이 함수는 항상 워커(WalkBuilder) 고정이다 -- Everything(es.exe) 백엔드로
// 옮기지 않는다: 확장자 필터가 없어 es.exe 출력이 통째로(수십만 건) 쏟아질 수
// 있고, gitignore 필터를 걸기 전에는 MAX_LIST 절단도 할 수 없어(필터링 후에야
// 진짜 개수를 알 수 있으므로) es.exe 사용이 오히려 워커보다 느려지는
// 역효과가 난다. Everything 백엔드는 md 확장자로 좁혀지는 markdown.rs 전용.
//
// 다만 위 우려는 "확장자 필터 없이 root 전체를 그대로 훑는" 목록 스캔에만
// 해당한다(이슈 #67 후속). 팔레트에 검색어가 입력된 검색(`search_workdir_files`)
// 은 사정이 다르다 -- 검색어 자체가 es.exe `path:` 필터가 되어 후보를 먼저
// 좁혀 주므로, 확장자 필터 없이도 수십만 건이 쏟아질 위험이 실질적으로 없다
// (목록의 5000개 상한 안에 들어오지 못해 검색이 안 되던 문제를 풀기 위한
// 기능이라, 애초에 이 함수는 백엔드가 Everything일 때만 호출될 것을 전제한다).
// 그래서 검색 경로만 Everything을 허용한다. 이 함수 자체는 설정을 모른다 --
// 백엔드가 Walker냐 Everything이냐의 게이팅은 호출부(commands.rs)가 한다.

use super::model::{WorkdirFileEntry, WorkdirListResult, WorkdirSearchResult};
use crate::file_scan::walk_files;

/// root 아래 파일을 스캔한다. markdown.rs와 동일한 워커를 확장자 필터 없이
/// 쓴다: 심링크는 따라가지 않고, `include_ignored`가 false면 `.gitignore`를
/// 존중하며 hidden을 스킵한다. true면 무시 규칙과 hidden 스킵을 모두 끄고
/// (`.git/` 내부만 제외) 디스크에 있는 그대로를 보여준다 -- 팔레트의
/// "숨김·무시 포함" 토글(앱 설정 `workdirShowIgnored`)이 그리로 온다.
pub fn list_workdir_files(root: &str, include_ignored: bool) -> Result<WorkdirListResult, String> {
    let (scanned, truncated) = walk_files(root, None, include_ignored)?;
    let files = scanned
        .into_iter()
        .map(|f| WorkdirFileEntry {
            rel_path: f.rel_path,
            name: f.name,
        })
        .collect();
    Ok(WorkdirListResult { files, truncated })
}

/// root 아래에서 `query`와 일치하는 파일을 Everything(es.exe)으로 검색한다.
/// 백엔드 설정은 모른다 -- 항상 시도하고, es.exe가 없거나(비Windows·미설치)
/// 실패/타임아웃이면(`None`) 조용히 `used_index: false` + 빈 목록으로 답한다
/// (에러가 아니다 -- 호출부가 이 신호로 클라이언트 fuzzy 필터 폴백으로
/// 되돌아간다). `query`가 공백뿐이면 es.exe를 부르지도 않고 즉시
/// `used_index: false` + 빈 목록을 돌려준다. `include_ignored`는 목록 스캔과
/// 같은 의미다 -- true면 gitignore/숨김 필터를 적용하지 않는다(팔레트 토글과
/// 어긋나지 않게 하기 위함).
pub fn search_workdir_files(
    root: &str,
    query: &str,
    include_ignored: bool,
) -> Result<WorkdirSearchResult, String> {
    let canon_root = std::fs::canonicalize(root)
        .map_err(|e| format!("작업 폴더를 찾을 수 없습니다: {root} ({e})"))?;
    if query.trim().is_empty() {
        return Ok(WorkdirSearchResult {
            files: Vec::new(),
            truncated: false,
            used_index: false,
        });
    }
    match crate::file_index::search_files_via_everything(&canon_root, query, include_ignored) {
        Some((scanned, truncated)) => {
            let files = scanned
                .into_iter()
                .map(|f| WorkdirFileEntry {
                    rel_path: f.rel_path,
                    name: f.name,
                })
                .collect();
            Ok(WorkdirSearchResult {
                files,
                truncated,
                used_index: true,
            })
        }
        None => Ok(WorkdirSearchResult {
            files: Vec::new(),
            truncated: false,
            used_index: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonexistent_root_is_error() {
        // tests 모듈 기준 super::super = workdir(listing의 부모)이므로
        // super::super::status는 workdir::status를 가리킨다.
        assert!(super::super::status::collect_git_status("/definitely/not/a/dir/xyzzy", None).is_err());
        assert!(list_workdir_files("/definitely/not/a/dir/xyzzy", false).is_err());
    }

    /// 기본(무시 존중) 스캔은 `.gitignore` 대상과 dot 파일을 빼고, 토글을 켠
    /// 스캔은 둘 다 담는다. `.git/` 내부는 어느 쪽에서도 나오지 않는다.
    #[test]
    fn include_ignored_reveals_gitignored_and_hidden_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(root.join("visible.txt"), "v").unwrap();
        std::fs::write(root.join("ignored.txt"), "i").unwrap();
        std::fs::write(root.join(".hidden"), "h").unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".git").join("HEAD"), "ref: refs/heads/main").unwrap();
        let root = root.to_str().unwrap();

        let paths = |include_ignored: bool| -> Vec<String> {
            list_workdir_files(root, include_ignored)
                .unwrap()
                .files
                .into_iter()
                .map(|f| f.rel_path)
                .collect()
        };

        let default_scan = paths(false);
        assert!(default_scan.contains(&"visible.txt".to_string()), "{default_scan:?}");
        assert!(!default_scan.contains(&"ignored.txt".to_string()), "{default_scan:?}");
        assert!(!default_scan.contains(&".hidden".to_string()), "{default_scan:?}");

        let full_scan = paths(true);
        assert!(full_scan.contains(&"visible.txt".to_string()), "{full_scan:?}");
        assert!(full_scan.contains(&"ignored.txt".to_string()), "{full_scan:?}");
        assert!(full_scan.contains(&".hidden".to_string()), "{full_scan:?}");
        assert!(full_scan.contains(&".gitignore".to_string()), "{full_scan:?}");
        // `.git/` 내부는 무시 규칙을 꺼도 항상 제외.
        assert!(
            !full_scan.iter().any(|p| p.starts_with(".git/")),
            "{full_scan:?}"
        );
    }

    #[test]
    fn search_nonexistent_root_is_error() {
        assert!(search_workdir_files("/definitely/not/a/dir/xyzzy", "abc", false).is_err());
    }

    /// 빈 쿼리(공백뿐)는 es.exe를 부르지 않고 즉시 used_index:false + 빈 목록.
    #[test]
    fn search_blank_query_short_circuits_without_index() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let result = search_workdir_files(root, "   ", false).unwrap();
        assert!(!result.used_index);
        assert!(result.files.is_empty());
        assert!(!result.truncated);
    }

    /// es.exe가 없는(대부분의 CI/개발 머신) 환경에서도 에러 없이 조용히
    /// used_index:false로 폴백해야 한다(설치돼 있으면 결과 유무는 검증하지
    /// 않는다 -- 실측 등가성은 file_index::mod 쪽 테스트가 담당).
    #[test]
    fn search_never_errors_regardless_of_es_availability() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let result = search_workdir_files(root, "abc", false);
        assert!(result.is_ok(), "result={result:?}");
    }
}
