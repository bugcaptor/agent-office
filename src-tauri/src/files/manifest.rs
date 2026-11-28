// src-tauri/src/files/manifest.rs
//
// 보내는 쪽: 고른 파일·폴더를 훑어 매니페스트(상대 경로·크기·SHA-256·수정 시각)와 원본 경로를 만들고,
// 보내는 동안 청크를 읽는다. 심볼릭 링크는 따라가지 않고 건너뛴다(폴더 밖을 끌어오지 않게).
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::model::{safe_name, FileEntry, MAX_FILES};

#[derive(Debug)]
pub struct Manifest {
    pub items: Vec<String>,
    pub files: Vec<FileEntry>,
    pub dirs: Vec<String>,
    pub sources: Vec<PathBuf>,
}

impl Manifest {
    pub fn total(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

pub fn mtime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn hash_file(path: &Path, cancel: &dyn Fn() -> bool) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        if cancel() {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled"));
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 고른 경로들로 매니페스트를 만든다(블로킹 — 해시 때문에 큰 파일은 오래 걸린다).
pub fn build(paths: &[PathBuf], cancel: &dyn Fn() -> bool) -> Result<Manifest, String> {
    if paths.is_empty() {
        return Err("file-no-paths".into());
    }
    let mut m = Manifest { items: Vec::new(), files: Vec::new(), dirs: Vec::new(), sources: Vec::new() };
    for path in paths {
        let meta = std::fs::symlink_metadata(path).map_err(|_| format!("file-not-found: {}", path.display()))?;
        let name = path
            .file_name()
            .map(|n| safe_name(&n.to_string_lossy()))
            .ok_or_else(|| format!("file-bad-path: {}", path.display()))?;
        if m.items.iter().any(|i| i.eq_ignore_ascii_case(&name)) {
            return Err(format!("file-duplicate-name: {name}"));
        }
        // 고른 것 자체가 링크면 가리키는 곳을 보낸다(사람이 고른 것이니). 폴더 안의 링크만 건너뛴다.
        let meta = if meta.file_type().is_symlink() { std::fs::metadata(path).map_err(|e| e.to_string())? } else { meta };
        if meta.is_file() {
            m.items.push(name.clone());
            push_file(&mut m, path, &meta, name)?;
        } else if meta.is_dir() {
            m.items.push(name.clone());
            walk(&mut m, path, &name)?;
        } else {
            return Err(format!("file-not-regular: {}", path.display()));
        }
    }
    for (i, f) in m.files.iter_mut().enumerate() {
        f.sha256 = hash_file(&m.sources[i], cancel).map_err(|e| {
            if e.kind() == std::io::ErrorKind::Interrupted {
                "cancelled".to_string()
            } else {
                format!("file-read-failed: {}: {e}", m.sources[i].display())
            }
        })?;
    }
    Ok(m)
}

fn push_file(m: &mut Manifest, path: &Path, meta: &std::fs::Metadata, rel: String) -> Result<(), String> {
    if m.files.len() >= MAX_FILES {
        return Err("file-too-many".into());
    }
    m.files.push(FileEntry { path: rel, size: meta.len(), sha256: String::new(), mtime: mtime_ms(meta) });
    m.sources.push(path.to_path_buf());
    Ok(())
}

fn walk(m: &mut Manifest, dir: &Path, rel: &str) -> Result<(), String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("file-read-failed: {}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|e| e.file_name());
    let before = m.files.len() + m.dirs.len();
    for entry in entries {
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let child = format!("{rel}/{}", safe_name(&entry.file_name().to_string_lossy()));
        if ft.is_dir() {
            walk(m, &entry.path(), &child)?;
        } else if ft.is_file() {
            let meta = entry.metadata().map_err(|e| e.to_string())?;
            push_file(m, &entry.path(), &meta, child)?;
        }
    }
    if m.files.len() + m.dirs.len() == before {
        m.dirs.push(rel.to_string());
    }
    Ok(())
}

/// 보내는 동안 원본에서 한 청크를 읽는다. 크기·수정 시각이 매니페스트와 다르면 거절한다.
pub fn read_chunk(source: &Path, entry: &FileEntry, offset: u64, len: u64) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(source).map_err(|_| "file-source-missing".to_string())?;
    if meta.len() != entry.size || (entry.mtime != 0 && mtime_ms(&meta) != entry.mtime) {
        return Err("file-source-changed".into());
    }
    if offset > entry.size || len > super::model::CHUNK * 2 {
        return Err("bad-request".into());
    }
    let len = len.min(entry.size - offset);
    let mut f = std::fs::File::open(source).map_err(|e| format!("file-read-failed: {e}"))?;
    f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf).map_err(|e| format!("file-read-failed: {e}"))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_folders_keeps_empty_dirs_and_skips_links() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(root.join("src/empty")).unwrap();
        std::fs::write(root.join("src/a.txt"), b"hello").unwrap();
        std::fs::write(tmp.path().join("one.bin"), b"x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(tmp.path().join("one.bin"), root.join("link")).unwrap();
        let m = build(&[root.clone(), tmp.path().join("one.bin")], &|| false).unwrap();
        assert_eq!(m.items, vec!["proj", "one.bin"]);
        let paths: Vec<&str> = m.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["proj/src/a.txt", "one.bin"]);
        assert_eq!(m.dirs, vec!["proj/src/empty"]);
        assert_eq!(m.files[0].sha256, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
        assert_eq!(m.total(), 6);
        assert_eq!(read_chunk(&m.sources[0], &m.files[0], 1, 3).unwrap(), b"ell");
        std::fs::write(root.join("src/a.txt"), b"changed!").unwrap();
        assert_eq!(read_chunk(&m.sources[0], &m.files[0], 0, 3).unwrap_err(), "file-source-changed");
        assert!(build(&[tmp.path().join("one.bin"), tmp.path().join("one.bin")], &|| false).unwrap_err().starts_with("file-duplicate-name"));
    }
}
