// src-tauri/src/persistence/diary_store.rs
//
// 캐릭터 일기(#56) per-agent 영속화. `<dir>/<agentId>.jsonl` 하나당 한 캐릭터.
// 일기 한 편을 JSON 한 줄로 append한다. append-only라 O_APPEND 단건 write를
// 쓴다(작은 줄은 원자적). load는 줄 단위 파싱하되 손상된/빈 줄은 건너뛴다
// (부분 write 내성). session_time_store.rs의 append/load 패턴 + png_store.rs의
// agentId 경로 안전성 검증을 합친 형태다.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use crate::types::DiaryEntry;

#[derive(Debug)]
pub enum DiaryStoreError {
    /// agentId가 경로 요소로 안전하지 않음(구분자/`..`/빈 문자열).
    InvalidId,
    /// 파일 시스템 오류.
    Io(String),
}

impl std::fmt::Display for DiaryStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiaryStoreError::InvalidId => write!(f, "invalid agentId (unsafe path element)"),
            DiaryStoreError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for DiaryStoreError {}

/// `<dir>/<agentId>.jsonl` 파일들을 관리한다. `dir`은 주입(테스트는 tempdir).
pub struct DiaryStore {
    dir: PathBuf,
}

impl DiaryStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// `agent_id`를 경로 요소로 쓰기 전 안전성 검증(경로 조작 방지). 구분자/`..`/
    /// 빈 문자열을 거부한다(png_store.rs와 동일 규칙).
    fn validate_id(agent_id: &str) -> Result<(), DiaryStoreError> {
        if agent_id.is_empty()
            || agent_id.contains('/')
            || agent_id.contains('\\')
            || agent_id.contains("..")
        {
            return Err(DiaryStoreError::InvalidId);
        }
        Ok(())
    }

    fn path_for(&self, agent_id: &str) -> PathBuf {
        self.dir.join(format!("{agent_id}.jsonl"))
    }

    /// 일기 한 편을 JSON 한 줄로 append. 부모 디렉터리가 없으면 만든다.
    pub fn append(&self, agent_id: &str, entry: &DiaryEntry) -> Result<(), DiaryStoreError> {
        Self::validate_id(agent_id)?;
        let file = self.path_for(agent_id);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).map_err(|e| DiaryStoreError::Io(e.to_string()))?;
        }
        let mut line = serde_json::to_vec(entry).map_err(|e| DiaryStoreError::Io(e.to_string()))?;
        line.push(b'\n');
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)
            .map_err(|e| DiaryStoreError::Io(e.to_string()))?;
        f.write_all(&line)
            .map_err(|e| DiaryStoreError::Io(e.to_string()))?;
        Ok(())
    }

    /// 한 캐릭터의 일기 전체(작성순). 파일 부재 = 빈 Vec. 파싱 실패한 줄은 건너뛴다.
    pub fn load(&self, agent_id: &str) -> Result<Vec<DiaryEntry>, DiaryStoreError> {
        Self::validate_id(agent_id)?;
        let bytes = match fs::read(self.path_for(agent_id)) {
            Ok(b) => b,
            Err(_) => return Ok(Vec::new()),
        };
        let text = String::from_utf8_lossy(&bytes);
        Ok(text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<DiaryEntry>(l).ok())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> PathBuf {
        std::env::temp_dir().join(format!("agent-office-diary-store-test-{}", uuid::Uuid::new_v4()))
    }

    fn sample(at: u64, session: &str, body: &str) -> DiaryEntry {
        DiaryEntry {
            at,
            session_id: session.into(),
            body: body.into(),
        }
    }

    #[test]
    fn append_then_load_roundtrips_an_entry() {
        let dir = scratch_dir();
        let store = DiaryStore::new(dir.clone());
        let entry = sample(1_000, "s1", "오늘은 버그를 잡았다.");

        store.append("a1", &entry).expect("append succeeds");
        let loaded = store.load("a1").unwrap();

        assert_eq!(loaded, vec![entry]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_twice_then_load_returns_two_in_order() {
        let dir = scratch_dir();
        let store = DiaryStore::new(dir.clone());
        let first = sample(1_000, "s1", "첫째 날.");
        let second = sample(2_000, "s2", "둘째 날.");

        store.append("a1", &first).unwrap();
        store.append("a1", &second).unwrap();
        let loaded = store.load("a1").unwrap();

        assert_eq!(loaded, vec![first, second]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn entries_are_isolated_per_agent() {
        let dir = scratch_dir();
        let store = DiaryStore::new(dir.clone());
        store.append("a1", &sample(1, "s1", "A의 일기")).unwrap();
        store.append("a2", &sample(2, "s2", "B의 일기")).unwrap();

        assert_eq!(store.load("a1").unwrap(), vec![sample(1, "s1", "A의 일기")]);
        assert_eq!(store.load("a2").unwrap(), vec![sample(2, "s2", "B의 일기")]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_on_missing_file_returns_empty() {
        let store = DiaryStore::new(scratch_dir());
        assert!(store.load("nobody").unwrap().is_empty());
    }

    #[test]
    fn load_skips_a_corrupt_line_but_keeps_valid_ones() {
        let dir = scratch_dir();
        fs::create_dir_all(&dir).unwrap();
        let store = DiaryStore::new(dir.clone());
        let good = sample(1_000, "s1", "정상 줄");
        let good2 = sample(2_000, "s2", "정상 줄2");
        let mut content = serde_json::to_string(&good).unwrap();
        content.push('\n');
        content.push_str("not json\n");
        content.push_str(&serde_json::to_string(&good2).unwrap());
        content.push('\n');
        fs::write(dir.join("a1.jsonl"), content).unwrap();

        let loaded = store.load("a1").unwrap();

        assert_eq!(loaded, vec![good, good2]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_creates_the_missing_parent_dir() {
        let dir = scratch_dir();
        assert!(!dir.exists());
        let store = DiaryStore::new(dir.clone());

        store.append("a1", &sample(1, "s1", "x")).expect("append succeeds");

        assert!(dir.exists());
        assert!(dir.join("a1.jsonl").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_unsafe_agent_id() {
        let store = DiaryStore::new(scratch_dir());
        assert!(matches!(
            store.append("../evil", &sample(1, "s1", "x")),
            Err(DiaryStoreError::InvalidId)
        ));
        assert!(matches!(store.load(""), Err(DiaryStoreError::InvalidId)));
        assert!(matches!(store.load("a/b"), Err(DiaryStoreError::InvalidId)));
    }
}
