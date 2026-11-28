// src-tauri/src/files/receive.rs
//
// 받는 쪽: 받은 파일 폴더 안 `.incoming/<전송 id>/`에 파일마다 `<번호>.part`로 쓰다가, 다 받으면 SHA-256을
// 맞춰 보고 `<받은 파일 폴더>/<보낸 머신 또는 캐릭터 이름>/`로 옮긴다. 같은 이름이 있으면 덮어쓰지 않고
// `이름 (2).ext`로 둔다. 끊겨도 `.part`는 남아 이어 받는다.
use std::collections::HashMap;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use super::manifest::{hash_file, hex};
use super::model::{safe_name, split_relative, Transfer, MAX_FILES};

const INCOMING: &str = ".incoming";
const META: &str = "transfer.json";
/// 여유 공간 검사에 더 남겨 둘 몫.
const SPARE: u64 = 64 * 1024 * 1024;

#[derive(Default)]
pub struct Receiver {
    /// 받는 중인 전송(오퍼 때 기억, 앱을 다시 켜면 `transfer.json`에서 읽는다).
    open: Mutex<HashMap<String, Transfer>>,
    /// (전송, 파일) → (처음부터 이어서 해시한 바이트, 해시 상태).
    hashers: Mutex<HashMap<(String, usize), (u64, Sha256)>>,
}

fn staging(root: &Path, id: &str) -> PathBuf {
    root.join(INCOMING).join(safe_name(id))
}

fn part(root: &Path, id: &str, index: usize) -> PathBuf {
    staging(root, id).join(format!("{index}.part"))
}

/// 매니페스트 검사: 경로가 항목 안에 있고 빠져나가지 않는다.
pub fn validate(t: &Transfer) -> Result<(), String> {
    if t.files.len() > MAX_FILES || t.items.is_empty() || t.items.len() > 1000 {
        return Err("file-bad-manifest".into());
    }
    let items: Vec<String> = t.items.iter().map(|i| safe_name(i)).collect();
    for p in t.files.iter().map(|f| f.path.as_str()).chain(t.dirs.iter().map(String::as_str)) {
        let parts = split_relative(p)?;
        if !items.contains(&parts[0]) {
            return Err("file-bad-manifest".into());
        }
    }
    if t.files.iter().any(|f| f.sha256.len() != 64) {
        return Err("file-bad-manifest".into());
    }
    Ok(())
}

pub fn free_space(path: &Path) -> Option<u64> {
    let mut p = path.to_path_buf();
    while !p.exists() {
        p = p.parent()?.to_path_buf();
    }
    free_space_at(&p)
}

#[cfg(unix)]
fn free_space_at(p: &Path) -> Option<u64> {
    let s = nix::sys::statvfs::statvfs(p).ok()?;
    Some(s.blocks_available() as u64 * s.fragment_size() as u64)
}

#[cfg(windows)]
fn free_space_at(p: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut free = 0u64;
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut())
    };
    (ok != 0).then_some(free)
}

/// `name`이 `dir`에 없고 `taken`에도 없으면 그대로, 아니면 `이름 (2).ext`처럼.
fn unique(dir: &Path, name: &str, is_dir: bool, taken: &[String]) -> String {
    let free = |n: &str| !dir.join(n).exists() && !taken.iter().any(|t| t.eq_ignore_ascii_case(n));
    if free(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if !is_dir && i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..10_000).map(|n| format!("{stem} ({n}){ext}")).find(|n| free(n)).unwrap_or_else(|| format!("{stem} ({}){ext}", uuid::Uuid::new_v4().simple()))
}

impl Receiver {
    fn meta(&self, root: &Path, id: &str) -> Result<Transfer, String> {
        if let Some(t) = self.open.lock().unwrap().get(id) {
            return Ok(t.clone());
        }
        let bytes = std::fs::read(staging(root, id).join(META)).map_err(|_| "file-transfer-unknown".to_string())?;
        let t: Transfer = serde_json::from_slice(&bytes).map_err(|_| "file-transfer-unknown".to_string())?;
        self.open.lock().unwrap().insert(id.to_string(), t.clone());
        Ok(t)
    }

    /// 전송을 받기로 하고 파일마다 이어 받을 자리를 돌려준다. `resume`이 비면 새로 받는다.
    pub fn offer(&self, root: &Path, t: &Transfer, resume: &[u64]) -> Result<Vec<u64>, String> {
        validate(t)?;
        let dir = staging(root, &t.id);
        if resume.is_empty() && dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("file-write-failed: {e}"))?;
        let mut offsets = Vec::with_capacity(t.files.len());
        for (i, f) in t.files.iter().enumerate() {
            let p = part(root, &t.id, i);
            let have = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            let at = resume.get(i).copied().unwrap_or(0).min(have).min(f.size);
            if have != at {
                if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&p) {
                    let _ = file.set_len(at);
                }
            }
            offsets.push(at);
        }
        let need = t.total_bytes.saturating_sub(offsets.iter().sum());
        if let Some(free) = free_space(&dir) {
            if free < need + SPARE {
                return Err(format!("file-no-space: need {need}, free {free}"));
            }
        }
        let bytes = serde_json::to_vec(&t.public()).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(META), bytes).map_err(|e| format!("file-write-failed: {e}"))?;
        self.open.lock().unwrap().insert(t.id.clone(), t.public());
        self.hashers.lock().unwrap().retain(|(id, _), _| id != &t.id);
        Ok(offsets)
    }

    /// 청크 하나를 쓴다(블로킹).
    pub fn write(&self, root: &Path, id: &str, index: usize, offset: u64, data: &[u8]) -> Result<(), String> {
        let t = self.meta(root, id)?;
        let entry = t.files.get(index).ok_or("bad-request")?;
        if offset + data.len() as u64 > entry.size {
            return Err("bad-request".into());
        }
        let p = part(root, id, index);
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&p)
            .map_err(|e| format!("file-write-failed: {e}"))?;
        f.seek(SeekFrom::Start(offset)).map_err(|e| format!("file-write-failed: {e}"))?;
        f.write_all(data).map_err(|e| format!("file-write-failed: {e}"))?;
        let key = (id.to_string(), index);
        let mut hashers = self.hashers.lock().unwrap();
        match hashers.get_mut(&key) {
            Some((pos, h)) if *pos == offset => {
                h.update(data);
                *pos += data.len() as u64;
            }
            _ if offset == 0 => {
                let mut h = Sha256::new();
                h.update(data);
                hashers.insert(key, (data.len() as u64, h));
            }
            // 이어 받거나 순서가 어긋났다 — 끝낼 때 파일 전체를 다시 해시한다.
            _ => {
                hashers.remove(&key);
            }
        }
        Ok(())
    }

    /// 다 받은 것을 확인하고 제자리로 옮긴다(블로킹). 저장한 맨 위 항목 절대 경로를 돌려준다.
    /// `sub`는 받은 파일 폴더 아래 폴더 이름(보낸 머신이나 받는 캐릭터).
    pub fn finish(&self, root: &Path, id: &str, sub: &str) -> Result<Vec<String>, String> {
        let t = self.meta(root, id)?;
        for (i, f) in t.files.iter().enumerate() {
            let p = part(root, id, i);
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            if f.size == 0 && size == 0 && !p.exists() {
                std::fs::write(&p, b"").map_err(|e| format!("file-write-failed: {e}"))?;
            } else if size != f.size {
                return Err(format!("file-incomplete: {}", f.path));
            }
            let streamed = {
                let mut hashers = self.hashers.lock().unwrap();
                match hashers.remove(&(id.to_string(), i)) {
                    Some((pos, h)) if pos == f.size => Some(hex(&h.finalize())),
                    _ => None,
                }
            };
            let digest = match streamed {
                Some(d) => d,
                None => hash_file(&p, &|| false).map_err(|e| format!("file-read-failed: {e}"))?,
            };
            if digest != f.sha256 {
                // 이 파일은 처음부터 다시 받는다.
                if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&p) {
                    let _ = file.set_len(0);
                }
                return Err(format!("file-hash-mismatch: {}", f.path));
            }
        }
        let base = root.join(safe_name(sub));
        std::fs::create_dir_all(&base).map_err(|e| format!("file-write-failed: {e}"))?;
        let mut names: Vec<String> = Vec::new();
        for item in &t.items {
            let item = safe_name(item);
            let is_dir = !t.files.iter().any(|f| f.path == item);
            names.push(unique(&base, &item, is_dir, &names));
        }
        let mapped = |rel: &str| -> Result<PathBuf, String> {
            let parts = split_relative(rel)?;
            let k = t.items.iter().position(|i| safe_name(i) == parts[0]).ok_or("file-bad-manifest")?;
            let mut p = base.join(&names[k]);
            for seg in &parts[1..] {
                p.push(seg);
            }
            Ok(p)
        };
        for (i, f) in t.files.iter().enumerate() {
            let dest = mapped(&f.path)?;
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("file-write-failed: {e}"))?;
            }
            std::fs::rename(part(root, id, i), &dest).map_err(|e| format!("file-write-failed: {e}"))?;
        }
        for d in &t.dirs {
            let _ = std::fs::create_dir_all(mapped(d)?);
        }
        self.abort(root, id);
        Ok(names.iter().map(|n| base.join(n).to_string_lossy().into_owned()).collect())
    }

    /// 받던 것을 버린다.
    pub fn abort(&self, root: &Path, id: &str) {
        let _ = std::fs::remove_dir_all(staging(root, id));
        self.open.lock().unwrap().remove(id);
        self.hashers.lock().unwrap().retain(|(t, _), _| t != id);
    }
}

/// 받은 전송에서 파일 하나의 저장 위치(`saved`와 매니페스트로 계산).
pub fn saved_path(t: &Transfer, index: usize) -> Option<PathBuf> {
    let f = t.files.get(index)?;
    let parts = split_relative(&f.path).ok()?;
    let k = t.items.iter().position(|i| safe_name(i) == parts[0])?;
    let mut p = PathBuf::from(t.saved.get(k)?);
    for seg in &parts[1..] {
        p.push(seg);
    }
    Some(p)
}

#[cfg(test)]
pub mod tests_support {
    use super::*;

    pub fn transfer() -> Transfer {
        super::tests::transfer("t", &[("a.txt", b"hello")], &["a.txt"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::model::{FileEntry, Party, TransferState};

    pub fn transfer(id: &str, files: &[(&str, &[u8])], items: &[&str]) -> Transfer {
        let files: Vec<FileEntry> = files
            .iter()
            .map(|(p, b)| FileEntry { path: p.to_string(), size: b.len() as u64, sha256: hex(&Sha256::digest(b)), mtime: 0 })
            .collect();
        Transfer {
            id: id.into(),
            created_at: 0,
            updated_at: 0,
            from: Party { machine_id: "m1".into(), machine_name: "mac".into(), ..Default::default() },
            to: Party::default(),
            items: items.iter().map(|s| s.to_string()).collect(),
            total_bytes: files.iter().map(|f| f.size).sum(),
            files,
            dirs: vec![],
            done_bytes: 0,
            progress: vec![],
            state: TransferState::Sending,
            error: None,
            note: None,
            notify: false,
            saved: vec![],
            via: "app".into(),
            sources: vec![],
            cleanup: None,
        }
    }

    #[test]
    fn receives_in_chunks_resumes_and_never_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let r = Receiver::default();
        let t = transfer("t1", &[("docs/a.txt", b"hello world"), ("b.txt", b"")], &["docs", "b.txt"]);
        assert_eq!(r.offer(root, &t, &[]).unwrap(), vec![0, 0]);
        r.write(root, "t1", 0, 0, b"hello").unwrap();
        // 끊겼다가 다시 오퍼: 받은 데까지 이어 받는다.
        let r = Receiver::default();
        assert_eq!(r.offer(root, &t, &[5, 0]).unwrap(), vec![5, 0]);
        r.write(root, "t1", 0, 5, b" world").unwrap();
        assert!(r.write(root, "t1", 0, 10, b"xx").is_err(), "크기를 넘는 쓰기는 거절");
        std::fs::create_dir_all(root.join("mac/docs")).unwrap();
        let saved = r.finish(root, "t1", "mac").unwrap();
        assert_eq!(saved, vec![root.join("mac/docs (2)").to_string_lossy(), root.join("mac/b.txt").to_string_lossy()]);
        assert_eq!(std::fs::read(root.join("mac/docs (2)/a.txt")).unwrap(), b"hello world");
        assert!(!root.join(".incoming/t1").exists());
        let mut done = t.clone();
        done.saved = saved;
        assert_eq!(saved_path(&done, 0).unwrap(), root.join("mac/docs (2)/a.txt"));
    }

    #[test]
    fn hash_mismatch_resets_that_file() {
        let tmp = tempfile::tempdir().unwrap();
        let r = Receiver::default();
        let t = transfer("t2", &[("a.txt", b"abc")], &["a.txt"]);
        r.offer(tmp.path(), &t, &[]).unwrap();
        r.write(tmp.path(), "t2", 0, 0, b"abd").unwrap();
        assert!(r.finish(tmp.path(), "t2", "mac").unwrap_err().starts_with("file-hash-mismatch"));
        assert_eq!(r.offer(tmp.path(), &t, &[3]).unwrap(), vec![0]);
    }

    #[test]
    fn rejects_manifests_that_escape() {
        let mut t = transfer("t3", &[("a.txt", b"x")], &["a.txt"]);
        t.files[0].path = "../a.txt".into();
        assert!(validate(&t).is_err());
        t.files[0].path = "other/a.txt".into();
        assert!(validate(&t).is_err(), "항목 밖 경로");
    }
}
