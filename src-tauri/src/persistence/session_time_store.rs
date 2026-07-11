// src-tauri/src/persistence/session_time_store.rs
//
// 세션 턴 시계열 로그(`session-times.jsonl`, Tauri app data dir) 영속화.
// 턴이 종료될 때마다 한 줄(JSON)씩 append한다. append-only라 원자적
// temp+rename이 아니라 O_APPEND 단건 write를 쓴다(작은 줄은 원자적).
// load는 줄 단위 파싱하되 손상된/빈 줄은 건너뛴다(부분 write 내성).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use crate::types::SessionTurnRecord;

pub struct SessionTimeStore {
    file: PathBuf,
}

impl SessionTimeStore {
    pub fn new(file: PathBuf) -> Self {
        Self { file }
    }

    /// 한 건을 JSON 한 줄로 append. 부모 디렉터리가 없으면 만든다.
    pub fn append(&self, record: &SessionTurnRecord) -> std::io::Result<()> {
        if let Some(parent) = self.file.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut line = serde_json::to_vec(record)?;
        line.push(b'\n');
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)?;
        f.write_all(&line)?;
        Ok(())
    }

    /// 누적 기록 전체. 파일 부재 = 빈 Vec. 파싱 실패한 줄은 건너뛴다.
    pub fn load(&self) -> Vec<SessionTurnRecord> {
        let bytes = match fs::read(&self.file) {
            Ok(b) => b,
            Err(_) => return Vec::new(),
        };
        let text = String::from_utf8_lossy(&bytes);
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<SessionTurnRecord>(l).ok())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn scratch_file() -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "agent-office-session-time-store-test-{}",
                uuid::Uuid::new_v4()
            ))
            .join("session-times.jsonl")
    }

    fn sample(agent_id: &str, started_at: u64, ended_at: u64) -> SessionTurnRecord {
        SessionTurnRecord {
            agent_id: agent_id.into(),
            started_at,
            ended_at,
            total_ms: ended_at - started_at,
            worked_ms: ended_at - started_at,
            waited_ms: 0,
        }
    }

    #[test]
    fn append_then_load_roundtrips_a_record() {
        let file = scratch_file();
        let store = SessionTimeStore::new(file.clone());
        let rec = sample("a1", 1_000, 4_000);

        store.append(&rec).expect("append succeeds");
        let loaded = store.load();

        assert_eq!(loaded, vec![rec]);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn append_twice_then_load_returns_two_in_order() {
        let file = scratch_file();
        let store = SessionTimeStore::new(file.clone());
        let first = sample("a1", 1_000, 2_000);
        let second = sample("a1", 3_000, 5_000);

        store.append(&first).unwrap();
        store.append(&second).unwrap();
        let loaded = store.load();

        assert_eq!(loaded, vec![first, second]);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn load_on_missing_file_returns_empty() {
        let store = SessionTimeStore::new(scratch_file());
        assert!(store.load().is_empty());
    }

    #[test]
    fn load_skips_a_corrupt_line_but_keeps_valid_ones() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let good = sample("a1", 1_000, 2_000);
        let mut content = serde_json::to_string(&good).unwrap();
        content.push('\n');
        content.push_str("not json\n");
        let good2 = sample("a1", 3_000, 4_000);
        content.push_str(&serde_json::to_string(&good2).unwrap());
        content.push('\n');
        fs::write(&file, content).unwrap();

        let store = SessionTimeStore::new(file.clone());
        let loaded = store.load();

        assert_eq!(loaded, vec![good, good2]);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn append_creates_the_missing_parent_dir() {
        let file = scratch_file();
        let parent = file.parent().unwrap().to_path_buf();
        assert!(!parent.exists());

        let store = SessionTimeStore::new(file.clone());
        store
            .append(&sample("a1", 1_000, 2_000))
            .expect("append succeeds");

        assert!(parent.exists());
        assert!(file.exists());
        let _ = fs::remove_dir_all(&parent);
    }
}
