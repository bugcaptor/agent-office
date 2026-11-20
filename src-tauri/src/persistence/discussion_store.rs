// src-tauri/src/persistence/discussion_store.rs
//
// 팀 토의 영속화. `<app_data>/discussions.json` 하나(`DiscussionsFile`)다.
//
// - 렌더러 실행기와 CLI(ctl)가 같은 앱 안에서 함께 쓰므로 목록 통째 교체가
//   아니라 토의 하나씩 upsert·삭제하고, 읽기-고치기-쓰기를 뮤텍스 안에서 한다.
// - 파일 부재는 빈 목록이다(시드할 것이 없다).
// - 손상·미래 버전은 Err이고 파일을 건드리지 않는다(role_store 정책).
// - 구조만 검사한다. 지워진 팀원·역할 id는 거부하지 않는다 — 읽는 쪽이 "없음"으로 본다.
// - 쓰기는 `durable::write_atomic`(fsync + rename).

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use super::durable::write_atomic;
use crate::types::{Discussion, DiscussionLimits, DiscussionsFile};

/// 이 빌드가 읽을 수 있는 최대 스키마 버전. TS `DISCUSSIONS_SCHEMA_VERSION` 미러.
pub const DISCUSSIONS_SCHEMA_VERSION: u32 = 1;
/// 사회자를 뺀 토론자 수 상한. 사회자 포함 최대 5명. TS 미러.
pub const DISCUSSION_MAX_DEBATERS: usize = 4;
/// 상한 범위(포함). TS `DISCUSSION_LIMIT_RANGES` 미러.
pub const MAX_ROUNDS_RANGE: (u32, u32) = (1, 12);
pub const MAX_MINUTES_RANGE: (u32, u32) = (5, 240);
pub const MAX_COST_USD_RANGE: (f64, f64) = (0.5, 50.0);

/// 저장할 수 있는 토의인지. 실패 이유 코드는 TS `discussionProblem`과 같다.
pub fn discussion_problem(d: &Discussion) -> Option<&'static str> {
    if d.id.trim().is_empty() {
        return Some("id");
    }
    if d.topic.trim().is_empty() {
        return Some("topic");
    }
    let mut ids = HashSet::new();
    if d.participants.iter().any(|p| p.agent_id.trim().is_empty() || !ids.insert(p.agent_id.as_str())) {
        return Some("duplicate");
    }
    if !ids.contains(d.moderator_agent_id.as_str()) {
        return Some("moderator");
    }
    let debaters = d.participants.len() - 1;
    if debaters == 0 {
        return Some("debaters");
    }
    if debaters > DISCUSSION_MAX_DEBATERS {
        return Some("too-many");
    }
    if !limits_ok(&d.limits) {
        return Some("limits");
    }
    None
}

fn limits_ok(l: &DiscussionLimits) -> bool {
    let within = |v: u32, (lo, hi): (u32, u32)| v >= lo && v <= hi;
    within(l.max_rounds, MAX_ROUNDS_RANGE)
        && within(l.max_minutes, MAX_MINUTES_RANGE)
        && l.max_cost_usd.is_finite()
        && l.max_cost_usd >= MAX_COST_USD_RANGE.0
        && l.max_cost_usd <= MAX_COST_USD_RANGE.1
}

pub struct DiscussionStore {
    file: PathBuf,
    lock: Mutex<()>,
}

impl DiscussionStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { file: dir.join("discussions.json"), lock: Mutex::new(()) }
    }

    pub fn load(&self) -> Result<Vec<Discussion>, String> {
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        Ok(self.read()?.discussions)
    }

    fn read(&self) -> Result<DiscussionsFile, String> {
        let bytes = match fs::read(&self.file) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DiscussionsFile { version: DISCUSSIONS_SCHEMA_VERSION, discussions: vec![] })
            }
            Err(e) => return Err(e.to_string()),
        };
        let parsed: DiscussionsFile =
            serde_json::from_slice(&bytes).map_err(|e| format!("discussions-corrupt: {e}"))?;
        if parsed.version > DISCUSSIONS_SCHEMA_VERSION {
            return Err(format!("discussions-unsupported-version: {}", parsed.version));
        }
        Ok(parsed)
    }

    fn write(&self, file: &DiscussionsFile) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(file).map_err(|e| e.to_string())?;
        write_atomic(&self.file, &bytes).map_err(|e| e.to_string())
    }

    /// 같은 id가 있으면 그 자리에서 바꾸고, 없으면 끝에 붙인다.
    pub fn save(&self, discussion: &Discussion) -> Result<(), String> {
        if discussion_problem(discussion).is_some() {
            return Err("invalid-discussion".into());
        }
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        let mut file = self.read()?;
        file.version = DISCUSSIONS_SCHEMA_VERSION;
        match file.discussions.iter_mut().find(|d| d.id == discussion.id) {
            Some(slot) => *slot = discussion.clone(),
            None => file.discussions.push(discussion.clone()),
        }
        self.write(&file)
    }

    /// 없는 id면 아무것도 하지 않는다.
    pub fn delete(&self, id: &str) -> Result<(), String> {
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        let mut file = self.read()?;
        let before = file.discussions.len();
        file.discussions.retain(|d| d.id != id);
        if file.discussions.len() == before {
            return Ok(());
        }
        self.write(&file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DiscussionParticipant, DiscussionState, WorkAgent, WorkCli};

    fn store() -> (DiscussionStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("office-discussions-{}", uuid::Uuid::new_v4()));
        (DiscussionStore::new(dir.clone()), dir)
    }
    fn participant(agent_id: &str) -> DiscussionParticipant {
        DiscussionParticipant {
            agent_id: agent_id.into(),
            role_id: Some("preset-critic".into()),
            agent: WorkAgent { cli: WorkCli::Claude, model: None, effort: None },
        }
    }
    fn discussion(id: &str, debaters: usize) -> Discussion {
        let mut participants = vec![participant("mod")];
        participants.extend((0..debaters).map(|i| participant(&format!("a{i}"))));
        Discussion {
            id: id.into(),
            topic: "저장 형식을 바꿀까".into(),
            context: String::new(),
            cwd: None,
            participants,
            moderator_agent_id: "mod".into(),
            limits: DiscussionLimits::default(),
            state: DiscussionState::Ready,
            round: 0,
            transcript: vec![],
            decision_memo_path: None,
            cost_usd: 0.0,
            created_at: 1,
            updated_at: 1,
            started_at: None,
            ended_at: None,
            run: None,
        }
    }

    #[test]
    fn missing_file_is_empty_and_upsert_delete_keep_order() {
        let (store, dir) = store();
        assert_eq!(store.load().unwrap(), vec![]);
        store.delete("nope").unwrap();
        assert!(!dir.join("discussions.json").exists());
        let (a, b) = (discussion("d1", 2), discussion("d2", 4));
        store.save(&a).unwrap();
        store.save(&b).unwrap();
        let mut a2 = a.clone();
        a2.topic = "바뀐 주제".into();
        store.save(&a2).unwrap();
        assert_eq!(store.load().unwrap(), vec![a2.clone(), b.clone()]);
        store.delete("d1").unwrap();
        assert_eq!(store.load().unwrap(), vec![b]);
        let names: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("discussions.json")]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_discussions_are_rejected_without_touching_the_file() {
        let (store, dir) = store();
        let good = discussion("d1", 1);
        store.save(&good).unwrap();
        let mut cases: Vec<(Discussion, &str)> = vec![];
        let mut d = discussion(" ", 1); d.topic = "t".into(); cases.push((d, "id"));
        let mut d = discussion("x", 1); d.topic = "  ".into(); cases.push((d, "topic"));
        let mut d = discussion("x", 2); d.participants[2].agent_id = "a0".into(); cases.push((d, "duplicate"));
        let mut d = discussion("x", 1); d.moderator_agent_id = "ghost".into(); cases.push((d, "moderator"));
        cases.push((discussion("x", 0), "debaters"));
        cases.push((discussion("x", 5), "too-many"));
        for limits in [
            DiscussionLimits { max_rounds: 0, ..Default::default() },
            DiscussionLimits { max_rounds: 13, ..Default::default() },
            DiscussionLimits { max_minutes: 4, ..Default::default() },
            DiscussionLimits { max_cost_usd: 0.1, ..Default::default() },
            DiscussionLimits { max_cost_usd: f64::NAN, ..Default::default() },
        ] {
            let mut d = discussion("x", 1); d.limits = limits; cases.push((d, "limits"));
        }
        for (bad, code) in cases {
            assert_eq!(discussion_problem(&bad), Some(code));
            assert_eq!(store.save(&bad), Err("invalid-discussion".into()));
        }
        assert_eq!(discussion_problem(&discussion("x", 4)), None);
        assert_eq!(store.load().unwrap(), vec![good]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_or_future_file_is_an_error_and_is_preserved() {
        let (store, dir) = store();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("discussions.json");
        for raw in [&b"{broken"[..], &br#"{"version":9,"discussions":[]}"#[..]] {
            fs::write(&path, raw).unwrap();
            assert!(store.load().is_err());
            assert!(store.save(&discussion("d1", 1)).is_err());
            assert!(store.delete("d1").is_err());
            assert_eq!(fs::read(&path).unwrap(), raw);
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn optional_fields_default_when_absent() {
        let parsed: Discussion = serde_json::from_str(
            r#"{"id":"d","topic":"t","participants":[{"agentId":"m","agent":{"cli":"codex"}},{"agentId":"a","agent":{"cli":"claude"}}],
                "moderatorAgentId":"m","state":"ready","createdAt":1,"updatedAt":2}"#,
        )
        .unwrap();
        assert_eq!(parsed.limits, DiscussionLimits::default());
        assert_eq!((parsed.context.as_str(), parsed.round, parsed.cost_usd), ("", 0, 0.0));
        assert!(parsed.transcript.is_empty() && parsed.cwd.is_none() && parsed.participants[0].role_id.is_none());
        assert_eq!(discussion_problem(&parsed), None);
    }
}
