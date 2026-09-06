// src-tauri/src/persistence/agy_resume_store.rs
//
// Antigravity CLI(agy) native `conversationId` 영속화(`agy-resume.json`,
// Tauri app data dir). docs/antigravity-support-design.md §3.5: "모든 이벤트에
// conversationId가 실려 온다. ClaudeSessionSink::record와 같은 sink에 provider를
// 넣어 기록한다. 스토어 파일은 provider별로 나눈다." -- claude_resume_store.rs와
// 같은 구조(에이전트당 최신 1건, tmp→rename 원자 쓰기, 로드 파손은 빈 상태로
// fail-open)를 그대로 따르되 provider별 파일을 분리한다. v1은 기록까지만 -- 재개
// 명령(`agy --conversation <id>`) UI는 범위 밖(§3.5).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// 에이전트 1명의 최신 agy 대화 스냅샷. serde는 렌더러 IPC 계약(camelCase).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgyResumeEntry {
    pub conversation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// 훅 body의 `transcriptPath`(실측: `transcript_full.jsonl`을 가리킨다).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AgyResumeFile {
    #[serde(default)]
    agents: HashMap<String, AgyResumeEntry>,
}

/// `agy-resume.json`을 읽고 쓰는 스토어. `ClaudeResumeStore`와 동일한 구조 --
/// 인메모리 미러 + 임계구역 내 병합·저장, 경로는 주입(테스트가 tempdir을 가리키게).
pub struct AgyResumeStore {
    file: PathBuf,
    state: Mutex<AgyResumeFile>,
}

impl AgyResumeStore {
    pub fn new(file: PathBuf) -> Self {
        let state = Self::load_file(&file);
        Self {
            file,
            state: Mutex::new(state),
        }
    }

    fn load_file(file: &PathBuf) -> AgyResumeFile {
        match std::fs::read(file) {
            Ok(bytes) => serde_json::from_slice::<AgyResumeFile>(&bytes).unwrap_or_default(),
            Err(_) => AgyResumeFile::default(),
        }
    }

    /// 에이전트의 최신 대화를 기록하고 디스크에 반영한다. 반환값은 디스크
    /// 반영 성공 여부(claude_resume_store.rs와 같은 재시도 계약).
    pub fn record(
        &self,
        agent_id: &str,
        conversation_id: &str,
        cwd: Option<&str>,
        transcript_path: Option<&str>,
        at_ms: u64,
    ) -> bool {
        let mut guard = self.state.lock().unwrap();
        let kept = guard
            .agents
            .get(agent_id)
            .filter(|prev| prev.conversation_id == conversation_id)
            .and_then(|prev| prev.transcript_path.clone());
        guard.agents.insert(
            agent_id.to_string(),
            AgyResumeEntry {
                conversation_id: conversation_id.to_string(),
                cwd: cwd.map(str::to_string),
                transcript_path: transcript_path.map(str::to_string).or(kept),
                updated_at: at_ms,
            },
        );
        match Self::save_file(&self.file, &guard) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("agy-resume.json 저장 실패: {e}");
                false
            }
        }
    }

    pub fn load_all(&self) -> HashMap<String, AgyResumeEntry> {
        self.state.lock().unwrap().agents.clone()
    }

    fn save_file(file: &PathBuf, state: &AgyResumeFile) -> std::io::Result<()> {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(state)?;
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("agy-resume.json");
        let tmp = file.with_file_name(format!("{name}.tmp-{}", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, &bytes)?;
        if let Err(e) = std::fs::rename(&tmp, file) {
            let _ = std::fs::remove_file(&tmp);
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
                "agent-office-agy-resume-store-test-{}",
                uuid::Uuid::new_v4()
            ))
            .join("agy-resume.json")
    }

    #[test]
    fn record_then_load_all_roundtrips_across_instances() {
        let file = scratch_file();
        {
            let store = AgyResumeStore::new(file.clone());
            store.record("a1", "conv-1", Some("/w/project"), Some("/t/transcript_full.jsonl"), 1_000);
        }
        let reloaded = AgyResumeStore::new(file.clone());
        let all = reloaded.load_all();
        assert_eq!(all.len(), 1);
        let entry = &all["a1"];
        assert_eq!(entry.conversation_id, "conv-1");
        assert_eq!(entry.cwd.as_deref(), Some("/w/project"));
        assert_eq!(entry.transcript_path.as_deref(), Some("/t/transcript_full.jsonl"));
        assert_eq!(entry.updated_at, 1_000);

        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn transcript_path_survives_a_hook_without_it_but_not_a_new_conversation() {
        let file = scratch_file();
        let store = AgyResumeStore::new(file.clone());
        store.record("a1", "conv-1", None, Some("/t/transcript_full.jsonl"), 1_000);
        store.record("a1", "conv-1", Some("/w"), None, 2_000);
        assert_eq!(
            store.load_all()["a1"].transcript_path.as_deref(),
            Some("/t/transcript_full.jsonl"),
        );

        store.record("a1", "conv-2", Some("/w"), None, 3_000);
        assert_eq!(store.load_all()["a1"].transcript_path, None);

        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn record_overwrites_previous_entry_for_same_agent() {
        let file = scratch_file();
        let store = AgyResumeStore::new(file.clone());
        store.record("a1", "conv-old", None, None, 1_000);
        store.record("a1", "conv-new", Some("/w"), None, 2_000);

        let all = store.load_all();
        assert_eq!(all.len(), 1);
        assert_eq!(all["a1"].conversation_id, "conv-new");

        let _ = fs::remove_dir_all(file.parent().unwrap());
    }
}
