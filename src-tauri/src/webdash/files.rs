//! 대화 로그에서 어시스턴트가 링크한 작업 폴더의 파일을 내려받는다.
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;
use pulldown_cmark::{Event, Parser, Tag};
use serde::Serialize;

use crate::session::manager::{expand_tilde, home_dir};

const MAX_FILE_BYTES: u64 = 24 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub(super) struct FileLink {
    name: String,
    #[serde(skip)]
    path: PathBuf,
}

pub(super) fn working_root(cwd: &str) -> Option<PathBuf> {
    let cwd = expand_tilde(cwd.to_string());
    if !Path::new(&cwd).is_absolute() {
        return None;
    }
    let root = fs::canonicalize(cwd).ok()?;
    let home = fs::canonicalize(home_dir()).ok();
    // 홈으로 시작한 세션에는 파일을 공유할 별도 작업 폴더가 없다.
    (root.is_dir() && root.parent().is_some() && !home.is_some_and(|home| home.starts_with(&root)))
        .then_some(root)
}

fn regular_file(root: &Path, path: &Path) -> Option<PathBuf> {
    let path = fs::canonicalize(root.join(path)).ok()?;
    if !path.starts_with(root) {
        return None;
    }
    let info = fs::metadata(&path).ok()?;
    (info.is_file() && info.len() <= MAX_FILE_BYTES).then_some(path)
}

pub(super) fn links(root: &Path, text: &str) -> Vec<FileLink> {
    let mut files: Vec<FileLink> = Vec::new();
    for event in Parser::new(text) {
        let Event::Start(Tag::Link { dest_url, .. }) = event else {
            continue;
        };
        let link = dest_url.split(['#', '?']).next().unwrap_or_default();
        let link = percent_decode_str(link.strip_prefix("sandbox:").unwrap_or(link)).decode_utf8();
        let Ok(link) = link else { continue };
        let path = Path::new(link.as_ref());
        if link.starts_with("//") || (!path.is_absolute() && link.contains(':')) {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(resolved) = regular_file(root, path) else {
            continue;
        };
        if files.iter().any(|file| file.path == resolved) {
            continue;
        }
        files.push(FileLink {
            name: name.to_string(),
            path: resolved,
        });
    }
    files
}

pub(super) fn read(root: &Path, link: &FileLink) -> Option<Vec<u8>> {
    let path = regular_file(root, &link.path)?;
    let mut open = OpenOptions::new();
    open.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = open.open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= MAX_FILE_BYTES).then_some(bytes)
}
