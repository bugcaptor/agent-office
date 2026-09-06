// src-tauri/src/observer/agy_resume_recorder.rs
//
// 프로덕션 `AgySessionSink` 구현. observer ingest가 agy 훅 body에서 뽑아
// 넘긴 conversationId를 SessionRegistry로 ao_session_id → agent_id 해석해
// AgyResumeStore에 기록한다. claude_resume_recorder.rs와 같은 구조(같은 값
// dedup, 전사 경로 뒤늦게 도착 허용, 디스크 반영 성공 시에만 dedup 표시).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::observer::AgySessionSink;
use crate::persistence::agy_resume_store::AgyResumeStore;
use crate::state::SessionRegistry;

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

pub struct AgyResumeRecorder {
    registry: Arc<SessionRegistry>,
    store: Arc<AgyResumeStore>,
    clock: Clock,
    last_seen: Mutex<HashMap<String, (String, Option<String>)>>,
}

impl AgyResumeRecorder {
    pub fn new(registry: Arc<SessionRegistry>, store: Arc<AgyResumeStore>) -> Self {
        Self::with_clock(registry, store, Arc::new(crate::types::now_ms))
    }

    pub fn with_clock(registry: Arc<SessionRegistry>, store: Arc<AgyResumeStore>, clock: Clock) -> Self {
        Self {
            registry,
            store,
            clock,
            last_seen: Mutex::new(HashMap::new()),
        }
    }
}

impl AgySessionSink for AgyResumeRecorder {
    fn record(
        &self,
        ao_session_id: &str,
        conversation_id: &str,
        cwd: Option<&str>,
        transcript_path: Option<&str>,
    ) {
        let Some(agent_id) = self.registry.resolve_agent(ao_session_id) else {
            return;
        };
        let key = (conversation_id.to_string(), transcript_path.map(str::to_string));
        {
            let seen = self.last_seen.lock().unwrap();
            if seen.get(ao_session_id) == Some(&key) {
                return;
            }
        }
        if self.store.record(&agent_id, conversation_id, cwd, transcript_path, (self.clock)()) {
            self.last_seen
                .lock()
                .unwrap()
                .insert(ao_session_id.to_string(), key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SessionState;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch_file() -> PathBuf {
        std::env::temp_dir()
            .join(format!("agent-office-agy-resume-recorder-test-{}", uuid::Uuid::new_v4()))
            .join("agy-resume.json")
    }

    fn ticking_clock() -> Clock {
        let tick = Arc::new(AtomicU64::new(1));
        Arc::new(move || tick.fetch_add(1, Ordering::SeqCst))
    }

    #[test]
    fn repeated_same_conversation_id_records_only_once() {
        let file = scratch_file();
        let registry = Arc::new(SessionRegistry::new());
        registry.insert("s1", "a1", SessionState::Running);
        let store = Arc::new(AgyResumeStore::new(file.clone()));
        let recorder = AgyResumeRecorder::with_clock(registry, store.clone(), ticking_clock());

        recorder.record("s1", "conv-1", Some("/w"), Some("/t/transcript_full.jsonl"));
        recorder.record("s1", "conv-1", Some("/w"), Some("/t/transcript_full.jsonl"));

        let all = store.load_all();
        assert_eq!(all.len(), 1);
        assert_eq!(all["a1"].updated_at, 1);
        assert_eq!(all["a1"].conversation_id, "conv-1");

        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn unregistered_session_is_dropped() {
        let file = scratch_file();
        let registry = Arc::new(SessionRegistry::new());
        let store = Arc::new(AgyResumeStore::new(file.clone()));
        let recorder = AgyResumeRecorder::with_clock(registry, store.clone(), ticking_clock());

        recorder.record("unknown", "conv-1", None, None);

        assert!(store.load_all().is_empty());
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }
}
