// src-tauri/src/persistence/discussion_inbox.rs
//
// 에이전트·CLI가 만든 토의의 수신함. `office-task --discuss`가
// `<work-inbox>/discussions/<시각>-<난수>.json`에 요청 하나를 원자적으로 쓰고, 렌더러의 수신함 주기가
// `scan_discussion_inbox`로 가져간다. 업무 수신함 폴더 아래라 Codex 샌드박스 쓰기 허용을 그대로 쓴다.
//
// - 파일은 누구나 쓸 수 있으므로 요청을 믿지 않는다. 주제·맥락·참가자·상한만 읽고 나머지(상태·라운드·
//   기록·비용·시각)는 여기서 새로 만든다. 그래서 수신함으로는 언제나 준비(ready) 토의만 생긴다.
// - 이미 있는 id는 덮지 않는다. 앞서 가져갔는데 파일 삭제만 실패한 것으로 보고 파일만 지운다.
// - 읽을 수 없거나 규칙에 어긋나는 파일은 `rejected/`로 옮기고 이유를 `.error`에 남긴다.

use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::discussion_store::{discussion_problem, DiscussionStore};
use super::profile_store::ProfileStore;
use super::work_inbox::{forbidden_control, reject, valid_task_id, MAX_BYTES};
use crate::types::{Discussion, DiscussionLimits, DiscussionParticipant, DiscussionState};

/// 요청 파일 형식 버전.
pub const REQUEST_VERSION: u32 = 1;
/// 한 번에 가져가는 파일 수.
const MAX_BATCH: usize = 16;

/// 수신함 요청 하나. Rust 안에서만(CLI가 쓰고 앱이 읽는다) 쓰는 형식이라 TS 미러가 없다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscussionRequest {
    pub version: u32,
    pub id: String,
    pub topic: String,
    #[serde(default)]
    pub context: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub moderator_agent_id: String,
    pub participants: Vec<DiscussionParticipant>,
    #[serde(default)]
    pub limits: DiscussionLimits,
    /// 만든 에이전트(세션 env). 앱은 저장하지 않는다 — 수신함·`rejected/` 파일을 보는 사람을 위한 기록.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_by: Option<String>,
}

/// 수신함 폴더(`<work-inbox>/discussions`).
pub fn inbox_dir(work_inbox: &Path) -> PathBuf {
    work_inbox.join("discussions")
}

fn accepted_name(name: &str) -> bool {
    name.ends_with(".json")
        && !name.starts_with('.')
        && name.len() <= 128
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// 요청을 준비 상태 토의로. 규칙에 어긋나면 이유 코드.
pub fn discussion_from_request(
    request: DiscussionRequest,
    known_agent: &dyn Fn(&str) -> bool,
    now: i64,
) -> Result<Discussion, &'static str> {
    if request.version > REQUEST_VERSION {
        return Err("unsupported-version");
    }
    if !valid_task_id(&request.id) {
        return Err("invalid-id");
    }
    if request.participants.iter().any(|p| !valid_task_id(&p.agent_id)) || !valid_task_id(&request.moderator_agent_id) {
        return Err("invalid-member");
    }
    if request.participants.iter().any(|p| !known_agent(&p.agent_id)) {
        return Err("unknown-member");
    }
    // CLI 실행 인자로 쓰일 칸(모델·추론 강도)과 역할 id는 한 줄짜리 짧은 값만 받는다.
    let short = |v: &Option<String>| v.as_deref().is_none_or(|t| !t.is_empty() && t.len() <= 128 && !t.chars().any(char::is_control));
    if request.participants.iter().any(|p| !short(&p.agent.model) || !short(&p.agent.effort) || !short(&p.role_id)) {
        return Err("invalid-agent");
    }
    let texts = [Some(request.topic.as_str()), Some(request.context.as_str()), request.cwd.as_deref()];
    if texts.into_iter().flatten().any(|t| forbidden_control(t).is_some()) {
        return Err("control-char");
    }
    // 사회자를 맨 앞에 둔다(저장 형식 규칙). 나머지는 받은 순서 그대로.
    let mut participants = request.participants;
    if let Some(at) = participants.iter().position(|p| p.agent_id == request.moderator_agent_id) {
        let moderator = participants.remove(at);
        participants.insert(0, moderator);
    }
    let cwd = request.cwd.map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
    if cwd.as_deref().is_some_and(|c| !Path::new(c).is_absolute()) {
        return Err("invalid-cwd");
    }
    let discussion = Discussion {
        id: request.id,
        topic: request.topic.trim().to_string(),
        context: request.context.trim().to_string(),
        cwd,
        participants,
        moderator_agent_id: request.moderator_agent_id,
        limits: request.limits,
        state: DiscussionState::Ready,
        round: 0,
        transcript: vec![],
        decision_memo_path: None,
        cost_usd: 0.0,
        created_at: now,
        updated_at: now,
        started_at: None,
        ended_at: None,
        active_ms: None,
        active_since: None,
        run: None,
    };
    if let Some(problem) = discussion_problem(&discussion) {
        // 저장소 검사 코드(`id`·`topic`·`moderator`…)를 수신함 이유로 그대로 쓴다.
        return Err(problem);
    }
    Ok(discussion)
}

/// 수신함의 요청 파일 이름. 파일 이름(시각) 순. 폴더가 없으면 빈 목록. 역할 수신함도 같은 규칙이다.
pub(super) fn request_names(dir: &Path) -> std::io::Result<Vec<String>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e),
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| accepted_name(n))
        .collect();
    names.sort();
    Ok(names)
}

/// 아직 가져가지 않은 요청들의 토의 id(읽을 수 있는 것만).
pub fn pending_ids(work_inbox: &Path) -> Vec<String> {
    let dir = inbox_dir(work_inbox);
    request_names(&dir)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|name| read_request(&dir.join(name)).ok())
        .map(|r| r.id)
        .collect()
}

fn read_request(path: &Path) -> Result<DiscussionRequest, &'static str> {
    let file = fs::File::open(path).map_err(|_| "unreadable")?;
    let mut bytes = vec![];
    file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes).map_err(|_| "unreadable")?;
    if bytes.len() > MAX_BYTES {
        return Err("too-large");
    }
    serde_json::from_slice(&bytes).map_err(|_| "invalid-json")
}

fn remove(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// 수신함을 읽어 새 토의를 저장하고, 저장한 것만 돌려준다. 파일 이름(시각) 순.
/// 렌더러는 돌려받은 것만 붙이므로, 파일 하나의 실패(거부·삭제)는 기록만 하고 나머지를 계속한다.
/// 저장소를 쓰지 못하면 거기서 멈추고 파일은 그대로 둔다(요청 탓이 아니다). 아무것도 못 가져왔을 때만 Err.
pub fn import(
    work_inbox: &Path,
    store: &DiscussionStore,
    known_agent: &dyn Fn(&str) -> bool,
    now: i64,
) -> Result<Vec<Discussion>, String> {
    let dir = inbox_dir(work_inbox);
    let names = request_names(&dir).map_err(|e| e.to_string())?;
    let mut imported = vec![];
    for name in names.into_iter().take(MAX_BATCH) {
        let path = dir.join(&name);
        let discussion = match read_request(&path).and_then(|r| discussion_from_request(r, known_agent, now)) {
            Ok(d) => d,
            Err("unreadable") => continue,
            Err(code) => {
                if let Err(e) = reject(&dir, &name, code, None) {
                    eprintln!("agent-office: 토의 수신함 {name} 거부 실패({code}): {e}");
                }
                continue;
            }
        };
        // 이미 있는 id면 앞서 가져갔는데 파일 삭제만 실패한 것이다. 파일만 지운다.
        match store.insert_if_absent(&discussion) {
            Ok(true) => imported.push(discussion),
            Ok(false) => {}
            Err(e) if imported.is_empty() => return Err(e),
            Err(e) => {
                eprintln!("agent-office: 토의 수신함 {name} 저장 실패: {e}");
                break;
            }
        }
        if let Err(e) = remove(&path) {
            eprintln!("agent-office: 토의 수신함 {name} 삭제 실패: {e}");
        }
    }
    Ok(imported)
}

/// 팀원 목록(`profiles.json`)으로 `import`한다. 목록을 읽지 못하면 가져오지 않는다 — 빈 목록으로 보면
/// 모든 요청이 `unknown-member`로 거부된다.
pub fn import_for_profiles(
    work_inbox: &Path,
    store: &DiscussionStore,
    profiles: &ProfileStore,
    now: i64,
) -> Result<Vec<Discussion>, String> {
    let agents: HashSet<String> = profiles.try_load().map_err(|e| e.to_string())?.agents.into_iter().map(|a| a.id).collect();
    import(work_inbox, store, &|id| agents.contains(id), now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{WorkAgent, WorkCli};

    fn participant(id: &str) -> DiscussionParticipant {
        DiscussionParticipant { agent_id: id.into(), role_id: None, agent: WorkAgent { cli: WorkCli::Claude, model: None, effort: None } }
    }
    fn request(id: &str) -> DiscussionRequest {
        DiscussionRequest {
            version: REQUEST_VERSION,
            id: id.into(),
            topic: " 저장 형식 ".into(),
            context: "맥락\n".into(),
            cwd: None,
            moderator_agent_id: "mod".into(),
            participants: vec![participant("a1"), participant("mod")],
            limits: DiscussionLimits::default(),
            requested_by: Some("a1".into()),
        }
    }
    fn write(dir: &Path, name: &str, body: &str) {
        fs::create_dir_all(inbox_dir(dir)).unwrap();
        fs::write(inbox_dir(dir).join(name), body).unwrap();
    }
    fn known(id: &str) -> bool { id != "ghost" }

    #[test]
    fn imports_a_ready_discussion_with_moderator_first_and_removes_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = DiscussionStore::new(tmp.path().to_path_buf());
        write(tmp.path(), "1-a.json", &serde_json::to_string(&request("disc-1")).unwrap());
        assert_eq!(pending_ids(tmp.path()), vec!["disc-1"]);
        let got = import(tmp.path(), &store, &known, 42).unwrap();
        assert_eq!(got.len(), 1);
        let d = &got[0];
        assert_eq!((d.state, d.round, d.created_at, d.updated_at), (DiscussionState::Ready, 0, 42, 42));
        assert_eq!(d.topic, "저장 형식");
        assert_eq!(d.participants.iter().map(|p| p.agent_id.as_str()).collect::<Vec<_>>(), vec!["mod", "a1"]);
        assert_eq!(store.load().unwrap(), got);
        assert!(pending_ids(tmp.path()).is_empty());
        assert!(import(tmp.path(), &store, &known, 43).unwrap().is_empty());
    }

    #[test]
    fn a_hand_written_running_discussion_still_arrives_ready() {
        let tmp = tempfile::tempdir().unwrap();
        let store = DiscussionStore::new(tmp.path().to_path_buf());
        let mut value = serde_json::to_value(request("disc-2")).unwrap();
        value["state"] = "running".into();
        value["round"] = 3.into();
        value["costUsd"] = 9.0.into();
        value["transcript"] = serde_json::json!([{ "at": 1, "round": 1, "kind": "statement", "text": "x" }]);
        write(tmp.path(), "1-a.json", &value.to_string());
        let d = &import(tmp.path(), &store, &known, 7).unwrap()[0];
        assert_eq!((d.state, d.round, d.cost_usd, d.transcript.len()), (DiscussionState::Ready, 0, 0.0, 0));
        assert!(d.run.is_none() && d.started_at.is_none());
    }

    #[test]
    fn an_existing_id_is_never_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let store = DiscussionStore::new(tmp.path().to_path_buf());
        let first = import_one(tmp.path(), &store, request("disc-3"));
        let mut again = request("disc-3");
        again.topic = "다른 주제".into();
        write(tmp.path(), "2-b.json", &serde_json::to_string(&again).unwrap());
        assert!(import(tmp.path(), &store, &known, 99).unwrap().is_empty());
        assert_eq!(store.load().unwrap(), vec![first]);
        assert!(pending_ids(tmp.path()).is_empty());
    }

    #[test]
    fn a_failed_reject_does_not_drop_the_rest_of_the_batch() {
        let tmp = tempfile::tempdir().unwrap();
        let store = DiscussionStore::new(tmp.path().to_path_buf());
        write(tmp.path(), "1-bad.json", "{not json");
        write(tmp.path(), "2-ok.json", &serde_json::to_string(&request("disc-8")).unwrap());
        // `rejected/`를 만들 수 없게 한다.
        fs::write(inbox_dir(tmp.path()).join("rejected"), "").unwrap();
        let got = import(tmp.path(), &store, &known, 1).unwrap();
        assert_eq!(got.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), vec!["disc-8"]);
        assert!(inbox_dir(tmp.path()).join("1-bad.json").exists());
        assert!(!inbox_dir(tmp.path()).join("2-ok.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn saved_discussions_are_returned_even_when_their_files_cannot_be_removed() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let store = DiscussionStore::new(tmp.path().to_path_buf());
        write(tmp.path(), "1-a.json", &serde_json::to_string(&request("disc-9")).unwrap());
        write(tmp.path(), "2-b.json", &serde_json::to_string(&request("disc-10")).unwrap());
        let dir = inbox_dir(tmp.path());
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        let got = import(tmp.path(), &store, &known, 1);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(got.unwrap().len(), 2);
        assert_eq!(store.load().unwrap().len(), 2);
        // 다음 주기는 이미 있는 id로 보고 파일만 지운다.
        assert!(import(tmp.path(), &store, &known, 2).unwrap().is_empty());
        assert!(pending_ids(tmp.path()).is_empty());
    }

    #[test]
    fn an_unreadable_store_keeps_the_request_in_the_inbox() {
        let tmp = tempfile::tempdir().unwrap();
        let store = DiscussionStore::new(tmp.path().to_path_buf());
        fs::write(tmp.path().join("discussions.json"), "{broken").unwrap();
        write(tmp.path(), "1-a.json", &serde_json::to_string(&request("disc-11")).unwrap());
        assert!(import(tmp.path(), &store, &known, 1).is_err());
        assert_eq!(pending_ids(tmp.path()), vec!["disc-11"]);
        assert!(!inbox_dir(tmp.path()).join("rejected").exists());
    }

    #[test]
    fn unreadable_profiles_skip_the_scan_instead_of_rejecting_everyone() {
        let tmp = tempfile::tempdir().unwrap();
        let store = DiscussionStore::new(tmp.path().to_path_buf());
        let profiles = ProfileStore::new(tmp.path().join("profiles.json"));
        fs::write(tmp.path().join("profiles.json"), "{broken").unwrap();
        write(tmp.path(), "1-a.json", &serde_json::to_string(&request("disc-12")).unwrap());
        assert!(import_for_profiles(tmp.path(), &store, &profiles, 1).is_err());
        assert_eq!(pending_ids(tmp.path()), vec!["disc-12"]);
        assert!(!inbox_dir(tmp.path()).join("rejected").exists());

        let agents: Vec<_> = ["mod", "a1"].iter().map(|id| serde_json::json!({ "id": id, "name": id, "role": "dev", "seed": "s", "createdAt": 1, "deskIndex": 0 })).collect();
        fs::write(tmp.path().join("profiles.json"), serde_json::json!({ "version": 1, "agents": agents }).to_string()).unwrap();
        assert_eq!(import_for_profiles(tmp.path(), &store, &profiles, 1).unwrap().len(), 1);
    }

    fn import_one(dir: &Path, store: &DiscussionStore, r: DiscussionRequest) -> Discussion {
        write(dir, "1-a.json", &serde_json::to_string(&r).unwrap());
        import(dir, store, &known, 1).unwrap().remove(0)
    }

    #[test]
    fn bad_requests_move_to_rejected_with_a_reason() {
        let tmp = tempfile::tempdir().unwrap();
        let store = DiscussionStore::new(tmp.path().to_path_buf());
        let mut ghost = request("disc-4");
        ghost.participants.push(participant("ghost"));
        let mut bad_id = request("../x");
        bad_id.id = "../x".into();
        let mut no_debaters = request("disc-5");
        no_debaters.participants.retain(|p| p.agent_id == "mod");
        let mut control = request("disc-6");
        control.context = "a\x1b[2Jb".into();
        let mut model = request("disc-7");
        model.participants[0].agent.model = Some("x\ny".into());
        let cases = [
            ("1.json", "{not json".to_string(), "invalid-json"),
            ("2.json", serde_json::to_string(&ghost).unwrap(), "unknown-member"),
            ("3.json", serde_json::to_string(&bad_id).unwrap(), "invalid-id"),
            ("4.json", serde_json::to_string(&no_debaters).unwrap(), "debaters"),
            ("5.json", serde_json::to_string(&control).unwrap(), "control-char"),
            ("6.json", "x".repeat(MAX_BYTES + 1), "too-large"),
            ("7.json", serde_json::to_string(&model).unwrap(), "invalid-agent"),
        ];
        for (name, body, _) in &cases {
            write(tmp.path(), name, body);
        }
        // 규칙 밖 이름은 건드리지 않는다.
        write(tmp.path(), "notes.txt", "x");
        assert!(import(tmp.path(), &store, &known, 1).unwrap().is_empty());
        assert!(store.load().unwrap().is_empty());
        for (name, _, code) in cases {
            let error = fs::read_to_string(inbox_dir(tmp.path()).join("rejected").join(format!("{name}.error"))).unwrap();
            assert_eq!(error.trim(), code, "{name}");
        }
        assert!(inbox_dir(tmp.path()).join("notes.txt").exists());
    }
}
