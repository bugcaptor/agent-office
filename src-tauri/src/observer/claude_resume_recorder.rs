// src-tauri/src/observer/claude_resume_recorder.rs
//
// 프로덕션 `ClaudeSessionSink` 구현. observer ingest가 Claude 훅 body에서
// 뽑아 넘긴 native 세션 ID를, SessionRegistry로 ao_session_id → agent_id
// 해석해 ClaudeResumeStore에 기록한다(docs/claude-session-resume-design.md §2).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::observer::ClaudeSessionSink;
use crate::persistence::claude_resume_store::ClaudeResumeStore;
use crate::state::SessionRegistry;

/// epoch ms를 돌려주는 시계. 프로덕션은 `now_ms`, 테스트는 고정/증가 클럭 주입.
type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

pub struct ClaudeResumeRecorder {
    registry: Arc<SessionRegistry>,
    store: Arc<ClaudeResumeStore>,
    clock: Clock,
    // ao_session_id → 마지막으로 기록한 native ID. Claude는 모든 훅마다
    // session_id를 실어 보내므로, 값이 바뀔 때만 store.record를 호출해
    // 훅마다 디스크를 다시 쓰는 것을 막는다.
    last_seen: Mutex<HashMap<String, String>>,
}

impl ClaudeResumeRecorder {
    pub fn new(registry: Arc<SessionRegistry>, store: Arc<ClaudeResumeStore>) -> Self {
        Self::with_clock(registry, store, Arc::new(crate::types::now_ms))
    }

    pub fn with_clock(
        registry: Arc<SessionRegistry>,
        store: Arc<ClaudeResumeStore>,
        clock: Clock,
    ) -> Self {
        Self {
            registry,
            store,
            clock,
            last_seen: Mutex::new(HashMap::new()),
        }
    }
}

impl ClaudeSessionSink for ClaudeResumeRecorder {
    fn record(&self, ao_session_id: &str, native_session_id: &str, cwd: Option<&str>) {
        // 미등록 세션(레지스트리에 없음)은 어느 에이전트 것인지 알 수 없어 버린다.
        let Some(agent_id) = self.registry.resolve_agent(ao_session_id) else {
            return;
        };
        {
            let seen = self.last_seen.lock().unwrap();
            if seen.get(ao_session_id).map(String::as_str) == Some(native_session_id) {
                return; // 값 불변 — 디스크 쓰기 생략
            }
        }
        // 디스크 반영에 성공했을 때만 "기록됨"으로 표시한다 — 실패를 표시해
        // 버리면 같은 ID를 실어 오는 후속 훅이 전부 위의 dedup에 걸려 영영
        // 재시도하지 않는다(리뷰 지적).
        if self
            .store
            .record(&agent_id, native_session_id, cwd, (self.clock)())
        {
            self.last_seen
                .lock()
                .unwrap()
                .insert(ao_session_id.to_string(), native_session_id.to_string());
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
            .join(format!(
                "agent-office-claude-resume-recorder-test-{}",
                uuid::Uuid::new_v4()
            ))
            .join("claude-resume.json")
    }

    /// 호출마다 1씩 증가하는 값을 돌려주는 클럭 — 기록이 실제로 일어났는지
    /// updatedAt 변화로 관측하기 위함.
    fn ticking_clock() -> Clock {
        let tick = Arc::new(AtomicU64::new(1));
        Arc::new(move || tick.fetch_add(1, Ordering::SeqCst))
    }

    #[test]
    fn repeated_same_native_id_records_only_once() {
        let file = scratch_file();
        let registry = Arc::new(SessionRegistry::new());
        registry.insert("s1", "a1", SessionState::Running);
        let store = Arc::new(ClaudeResumeStore::new(file.clone()));
        let recorder =
            ClaudeResumeRecorder::with_clock(registry, store.clone(), ticking_clock());

        recorder.record("s1", "native-1", Some("/w"));
        recorder.record("s1", "native-1", Some("/w"));

        let all = store.load_all();
        assert_eq!(all.len(), 1);
        // 두 번째 호출이 무시됐다면 updatedAt은 첫 기록값(1)에 머문다.
        assert_eq!(all["a1"].updated_at, 1);
        assert_eq!(all["a1"].session_id, "native-1");

        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn changed_native_id_is_recorded_again() {
        let file = scratch_file();
        let registry = Arc::new(SessionRegistry::new());
        registry.insert("s1", "a1", SessionState::Running);
        let store = Arc::new(ClaudeResumeStore::new(file.clone()));
        let recorder =
            ClaudeResumeRecorder::with_clock(registry, store.clone(), ticking_clock());

        recorder.record("s1", "native-1", None); // updatedAt = 1
        recorder.record("s1", "native-2", None); // 값 변경 → updatedAt = 2

        let all = store.load_all();
        assert_eq!(all["a1"].session_id, "native-2");
        assert_eq!(all["a1"].updated_at, 2);

        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn failed_save_is_retried_on_the_next_hook_with_the_same_id() {
        let file = scratch_file();
        let registry = Arc::new(SessionRegistry::new());
        registry.insert("s1", "a1", SessionState::Running);
        let store = Arc::new(ClaudeResumeStore::new(file.clone()));
        let recorder =
            ClaudeResumeRecorder::with_clock(registry, store.clone(), ticking_clock());

        // 대상 경로를 디렉터리로 막아 첫 저장을 실패시킨다.
        std::fs::create_dir_all(&file).unwrap();
        recorder.record("s1", "native-1", None);

        // 장애 해소 후 같은 ID의 후속 훅 — dedup에 걸리지 않고 재시도해야 한다.
        std::fs::remove_dir_all(&file).unwrap();
        recorder.record("s1", "native-1", None);

        let reloaded = ClaudeResumeStore::new(file.clone());
        assert_eq!(reloaded.load_all()["a1"].session_id, "native-1");

        // 성공 뒤에는 dedup이 다시 동작한다(updatedAt이 성공 시점 값에 머묾).
        let persisted_at = store.load_all()["a1"].updated_at;
        recorder.record("s1", "native-1", None);
        assert_eq!(store.load_all()["a1"].updated_at, persisted_at);

        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn unregistered_session_is_ignored() {
        let file = scratch_file();
        let registry = Arc::new(SessionRegistry::new()); // s-unknown 미등록
        let store = Arc::new(ClaudeResumeStore::new(file.clone()));
        let recorder =
            ClaudeResumeRecorder::with_clock(registry, store.clone(), ticking_clock());

        recorder.record("s-unknown", "native-x", None);

        assert!(store.load_all().is_empty());

        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }
}
