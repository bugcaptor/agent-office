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

use super::model::{WorkdirFileEntry, WorkdirListResult};
use crate::file_scan::walk_files;

/// root 아래 파일을 스캔한다. markdown.rs와 동일한 워커를 확장자 필터 없이
/// 쓴다: `.gitignore`를 존중하고 hidden을 스킵하며 심링크는 따라가지 않는다.
pub fn list_workdir_files(root: &str) -> Result<WorkdirListResult, String> {
    let (scanned, truncated) = walk_files(root, None)?;
    let files = scanned
        .into_iter()
        .map(|f| WorkdirFileEntry {
            rel_path: f.rel_path,
            name: f.name,
        })
        .collect();
    Ok(WorkdirListResult { files, truncated })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonexistent_root_is_error() {
        // tests 모듈 기준 super::super = workdir(listing의 부모)이므로
        // super::super::status는 workdir::status를 가리킨다.
        assert!(super::super::status::collect_git_status("/definitely/not/a/dir/xyzzy").is_err());
        assert!(list_workdir_files("/definitely/not/a/dir/xyzzy").is_err());
    }
}
