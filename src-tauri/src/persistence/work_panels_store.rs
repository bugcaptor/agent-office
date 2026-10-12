//! 업무 스냅샷과 월별 세션 요약. 데이터는 설정 토글과 무관하게 보존한다.
use crate::types::{SessionSummary, WorkItem};
use chrono::{DateTime, Datelike, Utc};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    sync::Mutex,
};

pub const RETENTION_MS: u64 = 300 * 86_400_000;
pub struct WorkPanelsStore {
    root: PathBuf,
    lock: Mutex<()>,
}
impl WorkPanelsStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            lock: Mutex::new(()),
        }
    }
    pub fn load_work_items(&self) -> Result<Vec<WorkItem>, String> {
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        match fs::read(self.root.join("work-items.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e.to_string()),
        }
    }
    pub fn save_work_items(&self, items: &[WorkItem]) -> Result<(), String> {
        let mut ids = HashSet::new();
        if items.iter().any(|i| {
            i.id.is_empty()
                || (i.title.trim().is_empty() && i.body.trim().is_empty())
                || !i.order.is_finite()
                || !ids.insert(&i.id)
        }) {
            return Err("invalid-work-items".into());
        }
        let bytes = serde_json::to_vec(items).map_err(|e| e.to_string())?;
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let tmp = self.root.join("work-items.json.tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        // Windows rename은 기존 목적지를 교체하지 못한다. 기존 스토어와 같은 규약.
        #[cfg(windows)]
        if self.root.join("work-items.json").exists() {
            fs::remove_file(self.root.join("work-items.json")).map_err(|e| e.to_string())?;
        }
        fs::rename(tmp, self.root.join("work-items.json")).map_err(|e| e.to_string())
    }
    fn summaries_dir(&self) -> PathBuf {
        self.root.join("session-summaries/v1")
    }
    pub fn append_summaries(&self, items: &[SessionSummary]) -> Result<(), String> {
        // 먼저 전량 검증해 잘못된 배치가 부분 기록되지 않게 한다.
        let mut batches: HashMap<String, Vec<u8>> = HashMap::new();
        for item in items {
            let at = i64::try_from(item.started_at).map_err(|_| "invalid-summary-date")?;
            let date = DateTime::<Utc>::from_timestamp_millis(at).ok_or("invalid-summary-date")?;
            if !(1970..=9999).contains(&date.year())
                || item.session_id.is_empty()
                || item.agent_id.is_empty()
                || item.prompts.len() > 50
                || item.prompts.iter().any(|p| p.text.chars().count() > 300)
            {
                return Err("invalid-session-summary".into());
            }
            let buf = batches.entry(date.format("%Y-%m").to_string()).or_default();
            serde_json::to_writer(&mut *buf, item).map_err(|e| e.to_string())?;
            buf.push(b'\n');
        }
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        if batches.is_empty() {
            return Ok(());
        }
        fs::create_dir_all(self.summaries_dir()).map_err(|e| e.to_string())?;
        for (month, bytes) in batches {
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.summaries_dir().join(format!("{month}.jsonl")))
                .map_err(|e| e.to_string())?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_data())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn load_summaries(&self, now: u64) -> Result<Vec<SessionSummary>, String> {
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        let cutoff = now.saturating_sub(RETENTION_MS);
        let cutoff_month = DateTime::<Utc>::from_timestamp_millis(cutoff as i64)
            .ok_or("invalid-summary-date")?
            .format("%Y-%m")
            .to_string();
        let entries = match fs::read_dir(self.summaries_dir()) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.to_string()),
        };
        let mut latest = HashMap::new();
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            let Some(name) = path.file_stem().and_then(|n| n.to_str()) else {
                continue;
            };
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl")
                || chrono::NaiveDate::parse_from_str(&format!("{name}-01"), "%Y-%m-%d").is_err()
            {
                continue;
            }
            if name < cutoff_month.as_str() {
                fs::remove_file(&path).map_err(|e| e.to_string())?;
                continue;
            }
            let file = fs::File::open(&path).map_err(|e| e.to_string())?;
            for line in BufReader::new(file).lines() {
                let line = line.map_err(|e| e.to_string())?;
                // 찢긴 마지막 줄/손상 줄은 건너뛴다. 같은 세션의 마지막 정상 줄이 이긴다.
                if let Ok(item) = serde_json::from_str::<SessionSummary>(&line) {
                    if item.started_at >= cutoff {
                        latest.insert(item.session_id.clone(), item);
                    }
                }
            }
        }
        let mut items: Vec<SessionSummary> = latest.into_values().collect();
        items.sort_by_key(|i| std::cmp::Reverse(i.started_at));
        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> WorkPanelsStore {
        WorkPanelsStore::new(
            std::env::temp_dir().join(format!("office-work-panels-{}", uuid::Uuid::new_v4())),
        )
    }
    fn summary(at: u64) -> SessionSummary {
        serde_json::from_value(serde_json::json!({"sessionId":"s1","agentId":"a1","agentName":"Alice","startedAt":at,"prompts":[]})).unwrap()
    }
    #[test]
    fn snapshots_roundtrip_and_invalid_writes_preserve_existing() {
        let store = store();
        assert!(store.load_work_items().unwrap().is_empty());
        let item: WorkItem = serde_json::from_value(serde_json::json!({"id":"w1","title":"Review","body":"Details","status":"in_progress","order":0,"sessionIds":["s1"],"createdAt":1,"updatedAt":2,"agent":{"cli":"pi","model":"m"}})).unwrap();
        store.save_work_items(&[item.clone()]).unwrap();
        let mut invalid = item.clone();
        invalid.id = "".into();
        assert!(store.save_work_items(&[invalid]).is_err());
        assert_eq!(store.load_work_items().unwrap(), vec![item]);
        store.save_work_items(&[]).unwrap();
        assert!(store.load_work_items().unwrap().is_empty());
        fs::remove_dir_all(store.root).unwrap();
    }
    #[test]
    fn untitled_work_roundtrips_but_empty_work_preserves_existing() {
        let store = store();
        let item: WorkItem = serde_json::from_value(serde_json::json!({"id":"w1","title":"","body":"Body only\nDetails","status":"todo","order":0,"sessionIds":[],"createdAt":1,"updatedAt":2})).unwrap();
        store.save_work_items(&[item.clone()]).unwrap();
        assert_eq!(store.load_work_items().unwrap(), vec![item.clone()]);
        let mut invalid = item.clone();
        invalid.body = " ".into();
        assert!(store.save_work_items(&[invalid]).is_err());
        assert_eq!(store.load_work_items().unwrap(), vec![item]);
        fs::remove_dir_all(store.root).unwrap();
    }
    #[test]
    fn summaries_last_line_wins_and_expired_months_are_pruned() {
        let store = store();
        let now = 1_800_000_000_000;
        let old = summary(now - RETENTION_MS - 40 * 86_400_000);
        store.append_summaries(&[old]).unwrap();
        let first = summary(now);
        let mut updated = first.clone();
        updated.goal = Some("Finished".into());
        store.append_summaries(&[first, updated.clone()]).unwrap();
        let month = DateTime::<Utc>::from_timestamp_millis(now as i64)
            .unwrap()
            .format("%Y-%m");
        writeln!(
            OpenOptions::new()
                .append(true)
                .open(store.summaries_dir().join(format!("{month}.jsonl")))
                .unwrap(),
            "broken"
        )
        .unwrap();
        assert_eq!(store.load_summaries(now).unwrap(), vec![updated]);
        assert_eq!(fs::read_dir(store.summaries_dir()).unwrap().count(), 1);
        fs::remove_dir_all(store.root).unwrap();
    }
    #[test]
    fn invalid_batch_writes_nothing() {
        let store = store();
        let valid = summary(1_800_000_000_000);
        let mut invalid = valid.clone();
        invalid.prompts = vec![crate::types::SessionSummaryPrompt {
            at: valid.started_at,
            text: "🙂".repeat(301),
            origin: None,
        }];
        assert!(store.append_summaries(&[valid, invalid]).is_err());
        assert!(!store.summaries_dir().exists());
    }
}
