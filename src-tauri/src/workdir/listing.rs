// src-tauri/src/workdir/listing.rs
//
// 작업 폴더 파일 목록 스캔. markdown.rs의 스캐너를 확장자 필터만 빼고 그대로
// 재현한다: `ignore` 크레이트(WalkBuilder)로 .gitignore를 존중하고 hidden을
// 스킵하며 심링크는 따라가지 않는다.

use std::path::Path;

use ignore::WalkBuilder;

use super::model::{WorkdirFileEntry, WorkdirListResult};

/// 목록 결과 상한 -- 이 수에 도달하면 스캔을 멈추고 `truncated=true`.
const MAX_LIST: usize = 5000;

/// root 아래 파일을 스캔한다. markdown.rs의 스캐너와 동일하되 확장자 필터가
/// 없다: `.gitignore`를 존중하고 hidden을 스킵하며 심링크는 따라가지 않는다.
pub fn list_workdir_files(root: &str) -> Result<WorkdirListResult, String> {
    let canon_root = std::fs::canonicalize(root)
        .map_err(|e| format!("작업 폴더를 찾을 수 없습니다: {root} ({e})"))?;
    if !canon_root.is_dir() {
        return Err(format!("작업 폴더가 디렉터리가 아닙니다: {root}"));
    }

    let mut builder = WalkBuilder::new(&canon_root);
    builder
        .follow_links(false) // 심링크는 따라가지 않는다(root 밖 유출 방지).
        .hidden(true) // 숨김 파일/폴더 스킵.
        .git_ignore(true) // .gitignore 존중.
        .require_git(false); // .git이 없어도 .gitignore를 적용.

    let mut files = Vec::new();
    let mut truncated = false;

    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue, // 개별 항목 접근 오류는 조용히 건너뛴다.
        };
        // 파일만(디렉터리·심링크 등 제외). file_type은 root 자체엔 없을 수 있다.
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(&canon_root) else {
            continue; // root 하위가 아니면(있을 수 없지만) 스킵.
        };
        let rel_path = normalize_separators(rel);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        files.push(WorkdirFileEntry { rel_path, name });

        if files.len() >= MAX_LIST {
            truncated = true;
            break;
        }
    }

    // relPath 오름차순 정렬(스캔 순서는 비결정적이므로 안정적 출력을 위해).
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(WorkdirListResult { files, truncated })
}

/// 경로 구분자를 '/'로 정규화한다(Windows의 '\\' → '/').
fn normalize_separators(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/")
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
