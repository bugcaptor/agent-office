// src-tauri/src/bot/state_store.rs
//
// 봇 폴링의 지속 상태(`bot-state.json`, app_data): since 커서, 처리한 댓글 id
// 이력(멱등·무한루프 방지의 핵심), 진행 중 잡. 쓰기는 profile_store와 같은
// temp+rename 원자 쓰기라 크래시 중에도 파일이 truncate되지 않는다.
// 설계 정본은 docs/bot-mode-design.md.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

/// 처리 이력이 무한히 커지지 않게 하는 상한. 댓글 id는 단조 증가하므로 초과분은
/// 가장 작은(오래된) id부터 버린다.
const PROCESSED_CAP: usize = 4000;

/// 봇 잡의 진행 단계.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobPhase {
    /// 슬래시 명령을 인지해 이슈를 이 캐릭터에 바인딩함(아직 미주입).
    Bound,
    /// 프롬프트를 주입하고 작업 진행 중.
    Working,
    /// 커밋 푸시/PR 감지로 완료됨.
    Done,
}

/// 캐릭터(agentId) 하나가 맡은 이슈 잡.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub issue: u64,
    #[serde(default)]
    pub branch: Option<String>,
    pub phase: JobPhase,
    /// 마지막 진행 보고 시각(ISO8601). 5분 스로틀 판단용.
    #[serde(default)]
    pub last_report_at: Option<String>,
}

/// `bot-state.json` 전체.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotState {
    pub version: u32,
    /// 마지막으로 본 댓글/이슈 `updated_at`(ISO8601). 증분 폴링 커서.
    #[serde(default)]
    pub since_cursor: Option<String>,
    /// 처리 완료한 댓글 id(정렬 유지). 재시작·폴링 겹침에도 정확히 1회 처리.
    #[serde(default)]
    pub processed_comment_ids: Vec<u64>,
    /// 이슈 **본문**으로 이미 트리거한 이슈 번호(정렬 유지). 댓글은 id로 멱등을
    /// 보장하지만 이슈 본문은 id가 없어 번호로 "한 번만 트리거"를 보장한다.
    #[serde(default)]
    pub triggered_issues: Vec<u64>,
    /// agentId → 진행 잡.
    #[serde(default)]
    pub jobs: BTreeMap<String, Job>,
}

impl Default for BotState {
    fn default() -> Self {
        Self {
            version: 1,
            since_cursor: None,
            processed_comment_ids: Vec::new(),
            triggered_issues: Vec::new(),
            jobs: BTreeMap::new(),
        }
    }
}

impl BotState {
    /// 이미 처리한 댓글인지.
    pub fn is_processed(&self, comment_id: u64) -> bool {
        self.processed_comment_ids.binary_search(&comment_id).is_ok()
    }

    /// 댓글을 처리 완료로 표시한다. 정렬 유지 + 상한 초과 시 오래된 id부터 GC.
    pub fn mark_processed(&mut self, comment_id: u64) {
        if let Err(pos) = self.processed_comment_ids.binary_search(&comment_id) {
            self.processed_comment_ids.insert(pos, comment_id);
        }
        if self.processed_comment_ids.len() > PROCESSED_CAP {
            let overflow = self.processed_comment_ids.len() - PROCESSED_CAP;
            self.processed_comment_ids.drain(0..overflow);
        }
    }

    /// 이슈 본문으로 이미 트리거한 이슈인지.
    pub fn is_issue_triggered(&self, issue: u64) -> bool {
        self.triggered_issues.binary_search(&issue).is_ok()
    }

    /// 이슈 본문 트리거를 기록한다(정렬 유지).
    pub fn mark_issue_triggered(&mut self, issue: u64) {
        if let Err(pos) = self.triggered_issues.binary_search(&issue) {
            self.triggered_issues.insert(pos, issue);
        }
    }

    /// 커서를 더 최근(문자열 비교상 더 큰) 값으로만 전진시킨다. ISO8601은 사전식
    /// 비교가 시간순과 일치하므로 안전하다.
    pub fn advance_cursor(&mut self, updated_at: &str) {
        if updated_at.is_empty() {
            return;
        }
        let advance = match &self.since_cursor {
            Some(cur) => updated_at > cur.as_str(),
            None => true,
        };
        if advance {
            self.since_cursor = Some(updated_at.to_string());
        }
    }
}

/// `bot-state.json`을 읽고 쓰는 스토어. 파일 경로만 보유한다(profile_store와 동형).
#[derive(Clone)]
pub struct BotStateStore {
    file: PathBuf,
}

impl BotStateStore {
    pub fn new(file: PathBuf) -> Self {
        Self { file }
    }

    /// 파일이 없거나 파손/버전 불일치면 기본값으로 폴백한다.
    pub fn load(&self) -> BotState {
        match fs::read(&self.file) {
            Ok(bytes) => match serde_json::from_slice::<BotState>(&bytes) {
                Ok(s) if s.version == 1 => s,
                _ => BotState::default(),
            },
            Err(_) => BotState::default(),
        }
    }

    /// temp+rename 원자 쓰기. 부모 디렉터리가 없으면 만든다.
    pub fn save(&self, state: &BotState) -> std::io::Result<()> {
        if let Some(parent) = self.file.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(state)?;
        let tmp = self.tmp_path();
        fs::write(&tmp, &bytes)?;
        if let Err(e) = fs::rename(&tmp, &self.file) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        Ok(())
    }

    fn tmp_path(&self) -> PathBuf {
        let name = self
            .file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("bot-state.json");
        self.file
            .with_file_name(format!("{name}.tmp-{}", uuid::Uuid::new_v4()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_file() -> PathBuf {
        std::env::temp_dir()
            .join(format!("agent-office-bot-state-test-{}", uuid::Uuid::new_v4()))
            .join("bot-state.json")
    }

    #[test]
    fn load_returns_default_when_missing() {
        let store = BotStateStore::new(scratch_file());
        let s = store.load();
        assert_eq!(s.version, 1);
        assert!(s.processed_comment_ids.is_empty());
        assert!(s.jobs.is_empty());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let store = BotStateStore::new(scratch_file());
        let mut s = BotState::default();
        s.mark_processed(1101);
        s.advance_cursor("2026-07-19T09:00:00Z");
        s.jobs.insert(
            "agent-1".into(),
            Job {
                issue: 57,
                branch: Some("bot/issue-57".into()),
                phase: JobPhase::Working,
                last_report_at: None,
            },
        );
        store.save(&s).unwrap();
        let loaded = store.load();
        assert!(loaded.is_processed(1101));
        assert_eq!(loaded.since_cursor.as_deref(), Some("2026-07-19T09:00:00Z"));
        assert_eq!(loaded.jobs.get("agent-1").unwrap().issue, 57);
        assert_eq!(loaded.jobs.get("agent-1").unwrap().phase, JobPhase::Working);
    }

    #[test]
    fn mark_processed_dedups_and_sorts() {
        let mut s = BotState::default();
        s.mark_processed(5);
        s.mark_processed(3);
        s.mark_processed(5); // dup
        assert_eq!(s.processed_comment_ids, vec![3, 5]);
        assert!(s.is_processed(3));
        assert!(!s.is_processed(4));
    }

    #[test]
    fn processed_cap_drops_oldest() {
        let mut s = BotState::default();
        for id in 0..(PROCESSED_CAP as u64 + 10) {
            s.mark_processed(id);
        }
        assert_eq!(s.processed_comment_ids.len(), PROCESSED_CAP);
        // 오래된 0..9 는 버려지고 최신은 남는다.
        assert!(!s.is_processed(0));
        assert!(s.is_processed(PROCESSED_CAP as u64 + 9));
    }

    #[test]
    fn cursor_only_advances_forward() {
        let mut s = BotState::default();
        s.advance_cursor("2026-07-19T09:00:00Z");
        s.advance_cursor("2026-07-19T08:00:00Z"); // 과거 → 무시
        assert_eq!(s.since_cursor.as_deref(), Some("2026-07-19T09:00:00Z"));
        s.advance_cursor("2026-07-19T10:00:00Z"); // 미래 → 전진
        assert_eq!(s.since_cursor.as_deref(), Some("2026-07-19T10:00:00Z"));
    }
}
