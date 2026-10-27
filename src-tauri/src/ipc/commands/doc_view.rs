// src-tauri/src/ipc/commands/doc_view.rs
//
// Markdown 미리보기의 "브라우저로 열기" 버튼.
// 디스크의 파일을 `agent-office md`와 같은 변환기로 캐시에 쓰고 바로 opener로
// 연다 — 앱 안이므로 요청 폴더(`doc-view/open/`)를 거치지 않는다. 편집 중 버퍼는
// 받지 않는다(결과가 파일과 어긋나지 않게).

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::doc_view;

/// `root` 아래 `rel_path`(작업 폴더 경계 검증)를 변환해 캐시 HTML 경로를 돌려준다.
fn render_for_preview(data_dir: &Path, root: &str, rel_path: &str) -> Result<PathBuf, String> {
    let (_canon_root, source) = crate::markdown::resolve_within_root(root, rel_path)?;
    if !source.is_file() || !doc_view::has_markdown_extension(&source) {
        return Err(format!("not-a-markdown-file: {rel_path}"));
    }
    doc_view::render_to_cache(data_dir, &source)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn doc_view_open(app: AppHandle, root: String, rel_path: String) -> Result<(), String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let html = render_for_preview(&data_dir, &root, &rel_path)?;
    app.opener()
        .open_path(html.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|e| format!("open-failed: {rel_path} ({e})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn renders_the_saved_file_into_the_cache() {
        let work = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        fs::create_dir(work.path().join("docs")).unwrap();
        fs::write(work.path().join("docs/plan.md"), "# Plan\n\nbody\n").unwrap();
        let root = work.path().to_string_lossy().into_owned();

        let html = render_for_preview(data.path(), &root, "docs/plan.md").unwrap();

        let source = fs::canonicalize(work.path().join("docs/plan.md")).unwrap();
        assert_eq!(html, doc_view::cache_path(data.path(), &source));
        let body = fs::read_to_string(&html).unwrap();
        assert!(body.contains("<title>Plan</title>"), "{body}");
    }

    #[test]
    fn rejects_paths_outside_root_and_non_markdown_files() {
        let outer = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let work = outer.path().join("work");
        fs::create_dir(&work).unwrap();
        fs::write(outer.path().join("secret.md"), "# secret").unwrap();
        fs::write(work.join("notes.txt"), "plain").unwrap();
        let root = work.to_string_lossy().into_owned();

        let outside = render_for_preview(data.path(), &root, "../secret.md").unwrap_err();
        assert!(outside.starts_with("path-outside-root"), "{outside}");
        let plain = render_for_preview(data.path(), &root, "notes.txt").unwrap_err();
        assert!(plain.starts_with("not-a-markdown-file"), "{plain}");
        let missing = render_for_preview(data.path(), &root, "gone.md").unwrap_err();
        assert!(missing.starts_with("file-not-found"), "{missing}");
        assert!(!doc_view::cache_dir(data.path()).exists());
    }
}
