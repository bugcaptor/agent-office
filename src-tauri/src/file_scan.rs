// src-tauri/src/file_scan.rs
//
// markdown.rs::list_markdown_files와 workdir/listing.rs::list_workdir_files가
// 공유하는 병렬 워커. 두 함수는 확장자 필터 유무만 다르고 나머지(WalkBuilder
// 설정, canonicalize·경로 정규화·MAX_LIST 상한·정렬)가 완전히 동일했던 복제를
// 없앤다.
//
// `ignore::WalkBuilder::build()`(단일스레드) 대신 `build_parallel()`을 써서
// 여러 스레드가 동시에 디렉터리를 순회한다 -- 대형 저장소에서 스캔 시간을
// 코어 수만큼 줄인다. 병렬 워커는 방문 순서가 비결정적이므로, 수집 후
// relPath 오름차순 정렬은 선택이 아니라 필수(기존 단일스레드 버전도 정렬은
// 했지만, 그때는 사실상 이미 어느 정도 안정적인 순서 위에 얹힌 것이었다).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use ignore::{WalkBuilder, WalkState};

/// 목록 결과 상한 -- 이 수에 도달하면 스캔을 멈추고 `truncated=true`.
pub const MAX_LIST: usize = 5000;

/// 스캔 항목 하나(호출자가 각자의 `*Entry` 타입으로 매핑).
pub struct ScannedFile {
    pub rel_path: String,
    pub name: String,
}

/// root 아래 파일을 병렬로 스캔한다. `ext_filter`가 `Some`이면 그 확장자
/// (소문자, 점 없이) 목록에 속하는 파일만, `None`이면 전체 파일을 담는다.
///
/// 반환은 (relPath 오름차순 정렬된 결과, truncated). `canonicalize`·`is_dir`
/// 체크·경로 정규화·MAX_LIST 조기 종료 로직을 모두 여기서 처리한다.
pub fn walk_files(
    root: &str,
    ext_filter: Option<&[&str]>,
) -> Result<(Vec<ScannedFile>, bool), String> {
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

    let files: Mutex<Vec<ScannedFile>> = Mutex::new(Vec::new());
    let truncated = AtomicBool::new(false);

    builder.build_parallel().run(|| {
        let canon_root = &canon_root;
        let files = &files;
        let truncated = &truncated;
        Box::new(move |entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => return WalkState::Continue, // 개별 항목 접근 오류는 조용히 건너뛴다.
            };
            // 파일만(디렉터리·심링크 등 제외). file_type은 root 자체엔 없을 수 있다.
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                return WalkState::Continue;
            }
            let path = entry.path();
            if let Some(exts) = ext_filter {
                if !has_extension(path, exts) {
                    return WalkState::Continue;
                }
            }
            let Ok(rel) = path.strip_prefix(canon_root) else {
                return WalkState::Continue; // root 하위가 아니면(있을 수 없지만) 스킵.
            };
            let rel_path = normalize_separators(rel);
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();

            let mut guard = files.lock().unwrap();
            if guard.len() >= MAX_LIST {
                truncated.store(true, Ordering::Relaxed);
                return WalkState::Quit; // 이미 상한 도달 -- 이 스레드는 조기 종료.
            }
            guard.push(ScannedFile { rel_path, name });
            if guard.len() >= MAX_LIST {
                truncated.store(true, Ordering::Relaxed);
                return WalkState::Quit;
            }
            WalkState::Continue
        })
    });

    let mut files = files.into_inner().unwrap();
    // relPath 오름차순 정렬(병렬 스캔이라 방문 순서가 비결정적이므로 안정적
    // 출력을 위해 필수).
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok((files, truncated.load(Ordering::Relaxed)))
}

/// path의 확장자가 `exts`(소문자, 점 없이) 중 하나인지(대소문자 무시).
fn has_extension(path: &Path, exts: &[&str]) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => {
            let lower = ext.to_ascii_lowercase();
            exts.contains(&lower.as_str())
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
