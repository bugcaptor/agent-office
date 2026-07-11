use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use uuid::Uuid;

use super::types::{SessionEventDraft, SessionEventRecord};

pub struct SessionEventStore {
    root: PathBuf,
    run_id: String,
    next_seq: Mutex<u64>,
}

impl SessionEventStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            run_id: Uuid::new_v4().to_string(),
            next_seq: Mutex::new(1),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn append(&self, draft: SessionEventDraft) -> io::Result<SessionEventRecord> {
        let mut next_seq = self.next_seq.lock();
        let seq = *next_seq;
        *next_seq = (*next_seq).saturating_add(1);
        let record = SessionEventRecord {
            schema_version: 1,
            run_id: self.run_id.clone(),
            seq,
            at: draft.at,
            agent_id: draft.agent_id,
            session_id: draft.session_id,
            kind: draft.kind,
            agent_name: draft.agent_name,
            agent_role: draft.agent_role,
            cwd: draft.cwd,
            shell: draft.shell,
            state: draft.state,
        };
        let path = self.path_for(record.at)?;
        fs::create_dir_all(&self.root)?;
        let mut line = serde_json::to_vec(&record)?;
        line.push(b'\n');
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(&line)?;
        Ok(record)
    }

    fn path_for(&self, at: u64) -> io::Result<PathBuf> {
        let millis = i64::try_from(at).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "event timestamp exceeds i64")
        })?;
        let date = DateTime::<Utc>::from_timestamp_millis(millis)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid event timestamp"))?
            .format("%Y-%m-%d")
            .to_string();
        Ok(self.root.join(format!("{date}.jsonl")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_events::types::{SessionEventDraft, SessionEventKind};
    use std::collections::HashSet;
    use std::fs;
    use std::sync::Arc;

    const BEFORE_UTC_MIDNIGHT: u64 = 1_783_727_999_999;
    const AT_UTC_MIDNIGHT: u64 = 1_783_728_000_000;

    fn scratch_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-session-events-{}",
            uuid::Uuid::new_v4()
        ))
    }

    fn deterministic_store(root: PathBuf) -> SessionEventStore {
        SessionEventStore {
            root,
            run_id: "run-1".into(),
            next_seq: Mutex::new(1),
        }
    }

    fn draft(at: u64) -> SessionEventDraft {
        SessionEventDraft::simple("a1", "s1", SessionEventKind::Tool, at)
    }

    fn read_records(path: &std::path::Path) -> Vec<SessionEventRecord> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn append_creates_a_v1_record_with_run_id_and_sequence() {
        let root = scratch_root();
        let store = deterministic_store(root.clone());
        let record = store.append(draft(AT_UTC_MIDNIGHT)).unwrap();
        assert_eq!(record.schema_version, 1);
        assert_eq!(record.run_id, "run-1");
        assert_eq!(record.seq, 1);
        assert_eq!(read_records(&root.join("2026-07-11.jsonl")), vec![record]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn append_partitions_on_the_event_utc_date() {
        let root = scratch_root();
        let store = deterministic_store(root.clone());
        store.append(draft(BEFORE_UTC_MIDNIGHT)).unwrap();
        store.append(draft(AT_UTC_MIDNIGHT)).unwrap();
        assert_eq!(read_records(&root.join("2026-07-10.jsonl")).len(), 1);
        assert_eq!(read_records(&root.join("2026-07-11.jsonl")).len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_appends_produce_complete_unique_lines() {
        let root = scratch_root();
        let store = Arc::new(deterministic_store(root.clone()));
        let threads: Vec<_> = (0..32)
            .map(|_| {
                let store = store.clone();
                std::thread::spawn(move || store.append(draft(AT_UTC_MIDNIGHT)).unwrap())
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let records = read_records(&root.join("2026-07-11.jsonl"));
        let seqs: HashSet<_> = records.iter().map(|record| record.seq).collect();
        assert_eq!(records.len(), 32);
        assert_eq!(seqs.len(), 32);
        assert_eq!(seqs.iter().copied().min(), Some(1));
        assert_eq!(seqs.iter().copied().max(), Some(32));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_append_consumes_its_sequence_number() {
        let root = scratch_root();
        fs::write(&root, b"not a directory").unwrap();
        let store = deterministic_store(root.clone());
        assert!(store.append(draft(AT_UTC_MIDNIGHT)).is_err());
        fs::remove_file(&root).unwrap();
        let record = store.append(draft(AT_UTC_MIDNIGHT)).unwrap();
        assert_eq!(record.seq, 2);
        let _ = fs::remove_dir_all(root);
    }
}
