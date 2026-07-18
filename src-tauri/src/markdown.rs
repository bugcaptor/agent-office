// src-tauri/src/markdown.rs
//
// 에이전트 작업 폴더 안의 마크다운 파일을 프런트가 목록·읽기·쓰기 할 수 있게
// 하는 IPC 커맨드 3종(`markdown_list_files`/`markdown_read_file`/
// `markdown_write_file`)의 구현부. vscode.rs/shell_export.rs와 같은 골격 --
// `#[tauri::command]` 얇은 래퍼가 테스트 가능한 순수 함수에 위임하고, 에러는
// 사용자에게 그대로 보여줄 수 있는 한국어 문자열이다.
//
// 핵심 안전장치는 "경로 봉쇄(path containment)"다. root와 root.join(rel_path)를
// 각각 canonicalize한 뒤, 결과가 canonical root 하위인지 확인한다. 이렇게 하면
// 절대경로·`..` 탈출·심볼릭 링크로 root 밖을 가리키는 rel_path가 모두
// canonicalize 후의 starts_with 검사에서 걸러진다(심링크는 canonicalize가
// 실제 대상 경로로 풀어 주므로).
//
// 쓰기는 shell_export.rs/settings_store.rs와 동일한 temp+rename 원자 쓰기라
// 프런트/에디터가 반쯤 쓰인 파일을 읽을 일이 없다. 낙관적 동시성 제어를 위해
// "{mtime_ms}:{size}" 형태의 version 문자열을 쓰며, 저장 직전 현재 version이
// 프런트가 마지막으로 읽은 expected_version과 다르면(다른 곳에서 수정됨)
// `Err("CONFLICT: ...")`로 접두사가 "CONFLICT"인 에러를 돌려준다(프런트가
// 접두사로 충돌을 판별).

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use ignore::WalkBuilder;

/// 목록 결과 상한 -- 이 수에 도달하면 스캔을 멈추고 `truncated=true`.
const MAX_LIST: usize = 5000;
/// 읽기 허용 최대 크기(2 MiB). 초과하면 에러(대용량 파일이 UI를 멈추지 않게).
const MAX_READ_BYTES: u64 = 2 * 1024 * 1024;

/// 마크다운으로 취급할 확장자(대소문자 무시).
const MARKDOWN_EXTENSIONS: [&str; 3] = ["md", "mdx", "markdown"];

/// 목록 결과. `truncated`는 상한(MAX_LIST)에 걸려 일부만 담겼음을 뜻한다.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownListResult {
    pub files: Vec<MarkdownFileEntry>,
    pub truncated: bool,
}

/// 목록 항목 하나. `rel_path`는 root 기준 상대경로(구분자 '/'로 정규화),
/// `name`은 파일명(마지막 경로 요소).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownFileEntry {
    pub rel_path: String,
    pub name: String,
}

/// 읽기 결과. `version`은 낙관적 동시성 제어용 "{mtime_ms}:{size}" 토큰.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownReadResult {
    pub content: String,
    pub version: String,
}

/// 쓰기 결과. 저장 후 새 `version`을 담아 프런트가 다음 저장에 쓰게 한다.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownWriteResult {
    pub version: String,
}

/// 파일 메타데이터로 "{mtime_ms}:{size}" version 토큰을 만든다. mtime을 못
/// 읽으면(플랫폼 미지원 등) 0ms로 폴백한다.
fn version_token(meta: &std::fs::Metadata) -> String {
    let size = meta.len();
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{mtime_ms}:{size}")
}

/// 경로 봉쇄: root와 root.join(rel_path)를 canonicalize해 (canonical root,
/// canonical target)을 돌려준다. target이 root 하위가 아니면(절대경로·`..`·
/// 심링크 탈출) 에러. canonicalize는 대상이 실존해야 하므로, 없는 파일은
/// 여기서 "찾을 수 없음" 취지의 에러가 된다.
fn resolve_within_root(root: &str, rel_path: &str) -> Result<(PathBuf, PathBuf), String> {
    let canon_root = std::fs::canonicalize(root)
        .map_err(|e| format!("작업 폴더를 찾을 수 없습니다: {root} ({e})"))?;
    let joined = canon_root.join(rel_path);
    let canon_target = std::fs::canonicalize(&joined)
        .map_err(|e| format!("파일을 찾을 수 없습니다: {rel_path} ({e})"))?;
    if !canon_target.starts_with(&canon_root) {
        return Err(format!("작업 폴더 밖의 경로는 접근할 수 없습니다: {rel_path}"));
    }
    Ok((canon_root, canon_target))
}

/// root 아래 마크다운 파일을 스캔한다. `ignore` 크레이트(WalkBuilder)로
/// .gitignore를 존중하고 hidden을 스킵하며 심링크는 따라가지 않는다.
/// require_git(false)로 `.git`이 없는 폴더에서도 .gitignore를 적용한다.
pub fn list_markdown_files(root: &str) -> Result<MarkdownListResult, String> {
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
        if !has_markdown_extension(path) {
            continue;
        }
        let Ok(rel) = path.strip_prefix(&canon_root) else {
            continue; // root 하위가 아니면(있을 수 없지만) 스킵.
        };
        let rel_path = normalize_separators(rel);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        files.push(MarkdownFileEntry { rel_path, name });

        if files.len() >= MAX_LIST {
            truncated = true;
            break;
        }
    }

    // relPath 오름차순 정렬(스캔 순서는 비결정적이므로 안정적 출력을 위해).
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(MarkdownListResult { files, truncated })
}

/// 확장자가 md/mdx/markdown 중 하나인지(대소문자 무시).
fn has_markdown_extension(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => {
            let lower = ext.to_ascii_lowercase();
            MARKDOWN_EXTENSIONS.contains(&lower.as_str())
        }
        None => false,
    }
}

/// 경로 구분자를 '/'로 정규화한다(Windows의 '\\'도 '/'로). 프런트는 항상
/// '/' 구분자 상대경로를 기대한다.
fn normalize_separators(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// root 기준 rel_path 파일을 읽는다. 경로 봉쇄를 적용하고, 2 MiB 초과·
/// 비UTF-8은 에러다.
pub fn read_markdown_file(root: &str, rel_path: &str) -> Result<MarkdownReadResult, String> {
    let (_canon_root, target) = resolve_within_root(root, rel_path)?;

    let meta = std::fs::metadata(&target)
        .map_err(|e| format!("파일 정보를 읽지 못했습니다: {rel_path} ({e})"))?;
    if !meta.is_file() {
        return Err(format!("파일이 아닙니다: {rel_path}"));
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(format!(
            "파일이 너무 큼: {rel_path} ({} bytes, 최대 {MAX_READ_BYTES} bytes)",
            meta.len()
        ));
    }

    let bytes = std::fs::read(&target)
        .map_err(|e| format!("파일을 읽지 못했습니다: {rel_path} ({e})"))?;
    let content = String::from_utf8(bytes)
        .map_err(|_| format!("UTF-8 텍스트가 아닌 파일은 열 수 없습니다: {rel_path}"))?;
    let version = version_token(&meta);
    Ok(MarkdownReadResult { content, version })
}

/// root 기준 rel_path 파일을 덮어쓴다(기존 파일 편집 전용 -- 없으면 에러).
/// 경로 봉쇄를 적용하고, 현재 version이 expected_version과 다르면
/// `Err("CONFLICT: ...")`(접두사 "CONFLICT")를 돌려준다. 저장은 같은
/// 디렉터리의 임시 파일에 쓴 뒤 rename하는 원자 쓰기다.
pub fn write_markdown_file(
    root: &str,
    rel_path: &str,
    content: &str,
    expected_version: &str,
) -> Result<MarkdownWriteResult, String> {
    // canonicalize가 실존을 요구하므로, 없는 파일은 여기서 에러가 난다
    // (기존 파일 편집 전용 계약을 자연히 만족).
    let (_canon_root, target) = resolve_within_root(root, rel_path)?;

    let meta = std::fs::metadata(&target)
        .map_err(|e| format!("파일 정보를 읽지 못했습니다: {rel_path} ({e})"))?;
    if !meta.is_file() {
        return Err(format!("파일이 아닙니다: {rel_path}"));
    }
    let current_version = version_token(&meta);
    if current_version != expected_version {
        // 접두사 "CONFLICT"로 프런트가 충돌을 판별한다(뒤 설명은 참고용).
        return Err(format!(
            "CONFLICT: 파일이 다른 곳에서 수정되었습니다: {rel_path}"
        ));
    }

    // 같은 디렉터리에 임시 파일 작성 후 rename(원자 저장). 다른 디렉터리로
    // rename하면 cross-device로 실패할 수 있으므로 부모 디렉터리를 쓴다.
    let parent = target
        .parent()
        .ok_or_else(|| format!("파일의 상위 폴더를 알 수 없습니다: {rel_path}"))?;
    let tmp = parent.join(format!(".md-tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, content.as_bytes())
        .map_err(|e| format!("파일을 저장하지 못했습니다: {rel_path} ({e})"))?;
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("파일 저장에 실패했습니다: {rel_path} ({e})"));
    }

    // 저장 후 새 version을 다시 계산해 돌려준다.
    let new_meta = std::fs::metadata(&target)
        .map_err(|e| format!("저장 후 파일 정보를 읽지 못했습니다: {rel_path} ({e})"))?;
    Ok(MarkdownWriteResult {
        version: version_token(&new_meta),
    })
}

/// 프런트가 부르는 커맨드: root 아래 마크다운 파일 목록. 구현은
/// `list_markdown_files`.
#[tauri::command(rename_all = "camelCase")]
pub async fn markdown_list_files(root: String) -> Result<MarkdownListResult, String> {
    list_markdown_files(&root)
}

/// 프런트가 부르는 커맨드: root 기준 rel_path 파일 읽기. 구현은
/// `read_markdown_file`.
#[tauri::command(rename_all = "camelCase")]
pub async fn markdown_read_file(
    root: String,
    rel_path: String,
) -> Result<MarkdownReadResult, String> {
    read_markdown_file(&root, &rel_path)
}

/// 프런트가 부르는 커맨드: root 기준 rel_path 파일 쓰기(낙관적 버전 검사).
/// 구현은 `write_markdown_file`.
#[tauri::command(rename_all = "camelCase")]
pub async fn markdown_write_file(
    root: String,
    rel_path: String,
    content: String,
    expected_version: String,
) -> Result<MarkdownWriteResult, String> {
    write_markdown_file(&root, &rel_path, &content, &expected_version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 마크다운 확장자 필터: md/mdx/markdown(대소문자 무시)만 통과.
    #[test]
    fn lists_only_markdown_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for name in ["a.md", "b.mdx", "c.markdown", "d.MD", "e.txt", "f.mdown"] {
            fs::write(root.join(name), "x").unwrap();
        }

        let result = list_markdown_files(root.to_str().unwrap()).unwrap();
        let names: Vec<_> = result.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["a.md", "b.mdx", "c.markdown", "d.MD"]);
        assert!(!result.truncated);
        // relPath는 root 기준 상대경로여야 한다.
        assert_eq!(result.files[0].rel_path, "a.md");
    }

    /// .gitignore와 숨김 파일은 제외된다(require_git(false)라 .git 없이도 적용).
    #[test]
    fn excludes_gitignored_and_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "ignored.md\nignored-dir/\n").unwrap();
        fs::write(root.join("visible.md"), "x").unwrap();
        fs::write(root.join("ignored.md"), "x").unwrap();
        fs::write(root.join(".hidden.md"), "x").unwrap();
        fs::create_dir(root.join("ignored-dir")).unwrap();
        fs::write(root.join("ignored-dir").join("inside.md"), "x").unwrap();
        // 하위 폴더의 정상 파일은 상대경로로 잡혀야 한다.
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub").join("nested.md"), "x").unwrap();

        let result = list_markdown_files(root.to_str().unwrap()).unwrap();
        let rels: Vec<_> = result.files.iter().map(|f| f.rel_path.as_str()).collect();
        assert_eq!(rels, vec!["sub/nested.md", "visible.md"]);
    }

    /// 정상 저장 후 재읽기가 내용·version 일치를 보장한다.
    #[test]
    fn write_then_read_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let root_str = root.to_str().unwrap();
        fs::write(root.join("note.md"), "old\n").unwrap();

        let initial = read_markdown_file(root_str, "note.md").unwrap();
        let new_content = "새 내용\n둘째 줄\n";
        let written =
            write_markdown_file(root_str, "note.md", new_content, &initial.version).unwrap();

        let reread = read_markdown_file(root_str, "note.md").unwrap();
        assert_eq!(reread.content, new_content);
        assert_eq!(reread.version, written.version);
        // 원자 쓰기의 임시 파일 잔여물이 없어야 한다.
        let has_tmp = fs::read_dir(root)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with(".md-tmp-"));
        assert!(!has_tmp, "임시 파일이 남으면 안 된다");
    }

    /// version이 다르면 CONFLICT 접두사 에러.
    #[test]
    fn write_with_stale_version_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let root_str = root.to_str().unwrap();
        fs::write(root.join("note.md"), "old\n").unwrap();

        let err = write_markdown_file(root_str, "note.md", "new\n", "0:0").unwrap_err();
        assert!(err.starts_with("CONFLICT"), "err={err}");
        // 파일은 그대로여야 한다(충돌 시 저장 안 함).
        assert_eq!(fs::read_to_string(root.join("note.md")).unwrap(), "old\n");
    }

    /// 없는 파일에 대한 쓰기는 에러(기존 파일 편집 전용).
    #[test]
    fn write_to_missing_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let root_str = dir.path().to_str().unwrap();
        let err = write_markdown_file(root_str, "nope.md", "x", "0:0").unwrap_err();
        assert!(!err.starts_with("CONFLICT"), "충돌이 아니라 부재 에러여야 한다: {err}");
    }

    /// `..`로 root 밖을 가리키는 rel_path는 거부.
    #[test]
    fn read_rejects_dotdot_escape() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir(&root).unwrap();
        // root 형제 위치에 비밀 파일.
        fs::write(dir.path().join("secret.md"), "secret").unwrap();
        fs::write(root.join("ok.md"), "ok").unwrap();

        let err = read_markdown_file(root.to_str().unwrap(), "../secret.md").unwrap_err();
        assert!(err.contains("작업 폴더"), "err={err}");
    }

    /// 절대경로 rel_path도 거부.
    #[test]
    fn read_rejects_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("ok.md"), "ok").unwrap();
        // /etc/hosts 등 실존 절대경로를 rel_path로 줘도 root 밖이라 거부.
        let err = read_markdown_file(root.to_str().unwrap(), "/etc/hosts").unwrap_err();
        assert!(
            err.contains("작업 폴더") || err.contains("찾을 수 없"),
            "err={err}"
        );
    }

    /// 심볼릭 링크로 root 밖을 가리키면 거부(canonicalize가 실제 대상으로 풀어냄).
    #[cfg(unix)]
    #[test]
    fn read_rejects_symlink_escape() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir(&root).unwrap();
        let outside = dir.path().join("outside.md");
        fs::write(&outside, "secret").unwrap();
        // root 안에 바깥 파일을 가리키는 심링크.
        std::os::unix::fs::symlink(&outside, root.join("link.md")).unwrap();

        let err = read_markdown_file(root.to_str().unwrap(), "link.md").unwrap_err();
        assert!(err.contains("작업 폴더"), "err={err}");
    }

    /// 비UTF-8 파일은 읽기 거부.
    #[test]
    fn read_rejects_non_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // 유효하지 않은 UTF-8 바이트열.
        fs::write(root.join("bin.md"), [0xff, 0xfe, 0x00, 0x80]).unwrap();
        let err = read_markdown_file(root.to_str().unwrap(), "bin.md").unwrap_err();
        assert!(err.contains("UTF-8"), "err={err}");
    }

    /// 2 MiB 초과 파일은 읽기 거부.
    #[test]
    fn read_rejects_oversize() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let big = vec![b'a'; (MAX_READ_BYTES as usize) + 1];
        fs::write(root.join("big.md"), big).unwrap();
        let err = read_markdown_file(root.to_str().unwrap(), "big.md").unwrap_err();
        assert!(err.contains("너무 큼"), "err={err}");
    }

    /// 확장자 판별 헬퍼 단위 검증.
    #[test]
    fn extension_matcher_is_case_insensitive() {
        assert!(has_markdown_extension(Path::new("a.md")));
        assert!(has_markdown_extension(Path::new("a.MdX")));
        assert!(has_markdown_extension(Path::new("a.MARKDOWN")));
        assert!(!has_markdown_extension(Path::new("a.txt")));
        assert!(!has_markdown_extension(Path::new("noext")));
    }
}
