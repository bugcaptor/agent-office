// src-tauri/src/persistence/claude_resume_store.rs
//
// Claude native 세션 ID(리줌 ID) 영속화(`claude-resume.json`, Tauri app data dir).
// 에이전트당 최신 1건만 보관(docs/claude-session-resume-design.md §3).
// profile_store와 같은 tmp→rename 원자 쓰기 — 크래시 중 write가 끼어들어도
// 리더는 옛 파일 또는 완전히 써진 새 파일만 본다. 로드 파손은 빈 상태로
// fail-open(리줌 편의 기능이라 못 읽어도 앱은 정상 동작해야 한다).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// 에이전트 1명의 최신 Claude 세션 스냅샷. serde는 렌더러 IPC 계약(camelCase).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeResumeEntry {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ClaudeResumeFile {
    #[serde(default)]
    agents: HashMap<String, ClaudeResumeEntry>,
}

/// `claude-resume.json`을 읽고 쓰는 스토어. 디스크 원본의 인메모리 미러를
/// 들고 있어 `record`가 병합+저장을 한 임계구역에서 함께 한다(에이전트당
/// 최신 1건 유지). 경로는 주입(테스트가 tempdir을 가리키게).
pub struct ClaudeResumeStore {
    file: PathBuf,
    state: Mutex<ClaudeResumeFile>,
}

impl ClaudeResumeStore {
    /// 파일에서 즉시 로드해 인메모리 미러를 채운다(부재/파손은 빈 상태).
    pub fn new(file: PathBuf) -> Self {
        let state = Self::load_file(&file);
        Self {
            file,
            state: Mutex::new(state),
        }
    }

    fn load_file(file: &PathBuf) -> ClaudeResumeFile {
        match std::fs::read(file) {
            Ok(bytes) => serde_json::from_slice::<ClaudeResumeFile>(&bytes).unwrap_or_default(),
            Err(_) => ClaudeResumeFile::default(),
        }
    }

    /// 에이전트의 최신 세션을 기록하고 디스크에 반영한다. 반환값은 디스크
    /// 반영 성공 여부 — 실패해도 인메모리 상태는 유지되지만(load_all은 최신값),
    /// 호출자(recorder)가 false를 보고 "기록됨" 처리를 미뤄야 다음 훅이
    /// 같은 ID로 저장을 재시도한다(리뷰 지적: 일시적 IO 실패의 영구 유실 방지).
    pub fn record(&self, agent_id: &str, session_id: &str, cwd: Option<&str>, at_ms: u64) -> bool {
        let mut guard = self.state.lock().unwrap();
        guard.agents.insert(
            agent_id.to_string(),
            ClaudeResumeEntry {
                session_id: session_id.to_string(),
                cwd: cwd.map(str::to_string),
                updated_at: at_ms,
            },
        );
        match Self::save_file(&self.file, &guard) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("claude-resume.json 저장 실패: {e}");
                false
            }
        }
    }

    /// 현재 보관 중인 전체 스냅샷(agentId → entry).
    pub fn load_all(&self) -> HashMap<String, ClaudeResumeEntry> {
        self.state.lock().unwrap().agents.clone()
    }

    fn save_file(file: &PathBuf, state: &ClaudeResumeFile) -> std::io::Result<()> {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(state)?;
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("claude-resume.json");
        let tmp = file.with_file_name(format!("{name}.tmp-{}", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, &bytes)?;
        if let Err(e) = std::fs::rename(&tmp, file) {
            let _ = std::fs::remove_file(&tmp); // 실패 시 temp 누수 방지
            return Err(e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch_file() -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "agent-office-claude-resume-store-test-{}",
                uuid::Uuid::new_v4()
            ))
            .join("claude-resume.json")
    }

    #[test]
    fn record_then_load_all_roundtrips_across_instances() {
        let file = scratch_file();
        {
            let store = ClaudeResumeStore::new(file.clone());
            store.record("a1", "native-1", Some("/w/project"), 1_000);
        }
        // 새 인스턴스로 디스크에서 다시 읽어 실제 영속화됐는지 확인.
        let reloaded = ClaudeResumeStore::new(file.clone());
        let all = reloaded.load_all();
        assert_eq!(all.len(), 1);
        let entry = &all["a1"];
        assert_eq!(entry.session_id, "native-1");
        assert_eq!(entry.cwd.as_deref(), Some("/w/project"));
        assert_eq!(entry.updated_at, 1_000);

        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn record_overwrites_previous_entry_for_same_agent() {
        let file = scratch_file();
        let store = ClaudeResumeStore::new(file.clone());
        store.record("a1", "native-old", None, 1_000);
        store.record("a1", "native-new", Some("/w"), 2_000);

        let all = store.load_all();
        assert_eq!(all.len(), 1);
        assert_eq!(all["a1"].session_id, "native-new");
        assert_eq!(all["a1"].cwd.as_deref(), Some("/w"));
        assert_eq!(all["a1"].updated_at, 2_000);

        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn record_keeps_entries_for_distinct_agents() {
        let file = scratch_file();
        let store = ClaudeResumeStore::new(file.clone());
        store.record("a1", "native-1", None, 1_000);
        store.record("a2", "native-2", None, 1_500);

        let all = store.load_all();
        assert_eq!(all.len(), 2);
        assert_eq!(all["a1"].session_id, "native-1");
        assert_eq!(all["a2"].session_id, "native-2");

        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn corrupt_file_loads_as_empty_and_still_accepts_records() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"not json at all").unwrap();

        let store = ClaudeResumeStore::new(file.clone());
        assert!(store.load_all().is_empty());

        // fail-open 후에도 정상적으로 기록·저장돼야 한다.
        store.record("a1", "native-1", None, 3_000);
        assert_eq!(store.load_all()["a1"].session_id, "native-1");

        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn record_reports_save_failure_but_keeps_in_memory_state() {
        let file = scratch_file();
        // 대상 경로 자체를 디렉터리로 만들어 tmp→rename을 실패시킨다.
        fs::create_dir_all(&file).unwrap();

        let store = ClaudeResumeStore::new(file.clone());
        assert!(!store.record("a1", "native-1", None, 1_000));
        // 디스크는 실패했어도 인메모리 미러는 최신값(load_all은 앱 수명 내 UI용).
        assert_eq!(store.load_all()["a1"].session_id, "native-1");

        // 장애 해소 후 재시도는 성공해야 한다.
        fs::remove_dir_all(&file).unwrap();
        assert!(store.record("a1", "native-1", None, 2_000));
        let reloaded = ClaudeResumeStore::new(file.clone());
        assert_eq!(reloaded.load_all()["a1"].session_id, "native-1");

        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn load_all_on_missing_file_is_empty() {
        let store = ClaudeResumeStore::new(scratch_file());
        assert!(store.load_all().is_empty());
    }

    #[test]
    fn record_leaves_no_temp_file_behind() {
        let file = scratch_file();
        let store = ClaudeResumeStore::new(file.clone());
        store.record("a1", "native-1", None, 1_000);

        let dir = file.parent().unwrap();
        let names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(names.iter().any(|n| n == "claude-resume.json"));
        assert!(
            !names.iter().any(|n| n.contains(".tmp")),
            "no temp file should remain after record: {names:?}"
        );

        let _ = fs::remove_dir_all(dir);
    }
}
