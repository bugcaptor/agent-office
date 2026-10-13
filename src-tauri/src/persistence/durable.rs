// src-tauri/src/persistence/durable.rs
//
// settings.json·profiles.json이 공유하는 파일 안전장치(수정 F2·F5).
//
// - `write_atomic`: 같은 폴더의 고유 tmp에 쓰고 fsync한 뒤 rename한다. fsync가
//   없으면 rename은 끝났는데 데이터 블록은 아직 디스크에 없는 상태에서 전원이
//   나가 빈 파일이 남을 수 있다(work_inbox_cli.rs `write`와 같은 방식).
// - `CorruptBackup`: 있는데 읽을 수 없는 파일을 `<name>.corrupt-<시각>`으로
//   복사해 둔다. 원본은 그대로 두고, 프로세스당 한 번만 만든다.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// `bytes`를 `file`에 원자적으로 쓴다. 부모 폴더가 없으면 만든다.
pub fn write_atomic(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("data");
    // 같은 폴더여야 rename이 기기 간 복사가 아닌 원자 교체가 된다. uuid는
    // 동시 저장끼리 tmp를 덮어쓰지 않게 한다.
    let tmp = file.with_file_name(format!("{name}.tmp-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut f = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, file)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
        return result;
    }
    sync_parent_dir(file);
    Ok(())
}

/// rename(디렉터리 항목 변경)까지 디스크에 내린다. 최선 노력 — 실패해도
/// 파일 내용은 이미 fsync됐으므로 저장 자체는 성공으로 둔다.
fn sync_parent_dir(file: &Path) {
    #[cfg(unix)]
    if let Some(dir) = file.parent().and_then(|p| fs::File::open(p).ok()) {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = file;
}

/// 손상 원본 백업 슬롯. 클론(AppState·control 등)끼리 공유해 로드할 때마다
/// 백업이 쌓이지 않게 한다.
#[derive(Clone, Default)]
pub struct CorruptBackup(Arc<Mutex<Option<PathBuf>>>);

impl CorruptBackup {
    /// `file`을 `<name>.corrupt-<시각>`으로 복사한다(원본은 그대로 둔다).
    /// 이미 만든 백업이 남아 있으면 그 경로를 돌려준다. 복사 실패는 None.
    pub fn backup(&self, file: &Path) -> Option<PathBuf> {
        let mut made = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(path) = made.as_ref().filter(|p| p.exists()) {
            return Some(path.clone());
        }
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("data");
        let stamp = chrono::Local::now().format("%Y%m%dT%H%M%S");
        let path = file.with_file_name(format!("{name}.corrupt-{stamp}"));
        match fs::copy(file, &path) {
            Ok(_) => {
                *made = Some(path.clone());
                Some(path)
            }
            Err(e) => {
                eprintln!("agent-office: {name} 백업 실패: {e}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_file(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("agent-office-durable-test-{}", uuid::Uuid::new_v4()))
            .join(name)
    }

    #[test]
    fn write_atomic_creates_dir_replaces_file_and_leaves_no_temp() {
        let file = scratch_file("data.json");
        write_atomic(&file, b"one").unwrap();
        write_atomic(&file, b"two").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"two");
        let names: Vec<String> = fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["data.json".to_string()]);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn backup_is_made_once_and_shared_between_clones() {
        let file = scratch_file("data.json");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"{broken").unwrap();
        let slot = CorruptBackup::default();
        let first = slot.backup(&file).expect("backup made");
        assert_eq!(fs::read(&first).unwrap(), b"{broken");
        assert_eq!(slot.clone().backup(&file), Some(first));
        assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 2);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }
}
