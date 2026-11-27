// src-tauri/src/link/discuss.rs
//
// 머신 연동 팀 토의: 진행(누가 언제 말하나)과 결정 메모는 호스트 렌더러 실행기(`discussionRun.ts`)가 쥔다.
// 이 머신 캐릭터가 참가자면 호스트가 그 참가자 세션만 여기서 열게 한다 — 이 머신 폴더를 읽고 발언을 만든다.
//
// 호스트가 묻는 것(`Request`)
// - `discuss-open`: 호스트 토의(참가자 id는 호스트가 부르는 id)·팀원 이름·역할 카드·이번 차례를 받아 이 머신
//   토의 세션(`discuss-<토의 id>.<이 머신 캐릭터>`)을 연다. 작업 폴더는 호스트 토의 폴더가 든 저장소의 이 머신
//   클론(하위 경로까지, 읽기만 — 브랜치·워크트리는 건드리지 않는다), 클론이 없으면 캐릭터 시작 폴더다.
// - `discuss-turn`: 같은 세션에 차례 지시 하나.
// - `discuss-collect`: 그 차례의 발언(`office-discuss say` 제출 → 마지막 응답, 이 머신 토의와 같다).
// - `discuss-state`: 세션이 살아 있는지·턴 단계(`TurnTracker`)·턴 토큰(호스트가 비용 상한에 넣는다).
// - `discuss-close`: 세션을 끝낸다.
//
// 이 머신 렌더러에는 `link-discussion-session` 이벤트로 알려 토의 탭으로 보이게 한다(진행은 하지 않는다).
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tauri::{Emitter, Manager};

use crate::control::ControlContext;
use crate::discussion_session::{self as ds, Member, OpenInput};
use crate::types::{Discussion, DiscussionRole, WorkCli};

pub const KIND_OPEN: &str = "discuss-open";
pub const KIND_TURN: &str = "discuss-turn";
pub const KIND_COLLECT: &str = "discuss-collect";
pub const KIND_STATE: &str = "discuss-state";
pub const KIND_CLOSE: &str = "discuss-close";
/// 호스트가 넘길 수 있는 토의 물음.
pub const KINDS: [&str; 5] = [KIND_OPEN, KIND_TURN, KIND_COLLECT, KIND_STATE, KIND_CLOSE];
/// 이 머신 렌더러에 알리는 이벤트(열림·닫힘).
pub const EVENT: &str = "link-discussion-session";
/// 들고 있는 연동 토의 세션 수(동시 토의 1개 × 이 머신 캐릭터 몇 명이면 충분하다).
const MAX_LINKED: usize = 16;

#[derive(Debug, Clone)]
struct LinkedSession {
    session_id: String,
    cwd: String,
    cli: WorkCli,
}

/// 호스트가 연 이 머신 토의 세션들(세션 agentId → 수집에 쓸 것).
#[derive(Default)]
pub struct Linked {
    sessions: Mutex<HashMap<String, LinkedSession>>,
}

impl Linked {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    fn remember(&self, agent_id: &str, session: LinkedSession) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.insert(agent_id.to_string(), session);
        if sessions.len() > MAX_LINKED {
            if let Some(k) = sessions.keys().find(|k| k.as_str() != agent_id).cloned() {
                sessions.remove(&k);
            }
        }
    }
    fn get(&self, agent_id: &str) -> Option<LinkedSession> {
        self.sessions.lock().unwrap().get(agent_id).cloned()
    }
    fn forget(&self, agent_id: &str) {
        self.sessions.lock().unwrap().remove(agent_id);
    }
    /// 다른 토의의 세션과 `agent_id` 세션을 목록에서 빼서 돌려준다(이 토의의 다른 참가자는 둔다).
    fn drain_except(&self, discussion_id: &str, agent_id: &str) -> Vec<(String, LinkedSession)> {
        let mut sessions = self.sessions.lock().unwrap();
        let drop: Vec<String> = sessions
            .keys()
            .filter(|k| k.as_str() == agent_id || ds::parse_session_agent_id(k).is_none_or(|(d, _)| d != discussion_id))
            .cloned()
            .collect();
        drop.into_iter().filter_map(|k| sessions.remove(&k).map(|v| (k, v))).collect()
    }
}

/// 세션을 끝내고 이 머신 렌더러에 알린다.
fn dispose(handle: &tauri::AppHandle, app: &crate::state::AppState, key: &str, expected: Option<&str>) {
    if app.manager.session_id_for(key).is_some() {
        app.work_send_runtime.stop(key);
        let _ = app.manager.dispose_matching(key, expected);
        app.gate.remove_agent(key);
    }
    let _ = handle.emit_to("main", EVENT, json!({ "opened": false, "agentId": key }));
}

fn text<'a>(data: &'a Value, key: &str) -> Result<&'a str, String> {
    data.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| "bad-request".into())
}

/// 이 머신 캐릭터(`agentId`)와 토의 id로 세션 agentId.
fn session_key(data: &Value) -> Result<String, String> {
    ds::session_agent_id(text(data, "discussionId")?, text(data, "agentId")?)
}

/// 호스트 토의를 이 머신에서 열 모양으로 바꾼다: 작업 폴더를 이 머신 폴더로. 팀원 이름은 호스트가 준 것,
/// 이 참가자의 말투는 이 머신 프로필의 것을 쓴다.
pub fn prepare_open(
    data: &Value,
    profile: &crate::types::AgentProfile,
    cwd: Option<String>,
) -> Result<OpenInput, String> {
    let mut discussion: Discussion = serde_json::from_value(data.get("discussion").cloned().unwrap_or(Value::Null)).map_err(|_| "bad-request".to_string())?;
    let participant = text(data, "participant")?.to_string();
    let roles: Vec<DiscussionRole> = data.get("roles").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let mut members: HashMap<String, Member> = data
        .get("members")
        .and_then(Value::as_object)
        .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), Member { name: v.as_str()?.to_string(), personality: None }))).collect())
        .unwrap_or_default();
    members
        .entry(participant.clone())
        .or_insert_with(|| Member { name: profile.name.clone(), personality: None })
        .personality = profile.personality_prompt.clone();
    discussion.cwd = cwd;
    ds::validate_discussion(&discussion, &participant)?;
    Ok(OpenInput {
        agent_id: ds::session_agent_id(&discussion.id, &profile.id)?,
        discussion,
        participant,
        profile_id: profile.id.clone(),
        members,
        roles,
        lang: data.get("lang").and_then(Value::as_str).map(str::to_string),
        turn: data.get("turn").and_then(Value::as_str).unwrap_or("").to_string(),
        cols: None,
        rows: None,
    })
}

/// 호스트의 토의 물음에 답한다. 이 모듈이 맡지 않는 종류면 None.
pub async fn answer(
    handle: &tauri::AppHandle,
    linked: &Arc<Linked>,
    work: &Arc<super::work::ClientWork>,
    ctl: &Arc<ControlContext>,
    kind: &str,
    data: &Value,
) -> Option<Result<Value, String>> {
    if !KINDS.contains(&kind) {
        return None;
    }
    let Some(app) = handle.try_state::<crate::state::AppState>() else { return Some(Err("link-not-ready".into())) };
    Some(match kind {
        KIND_OPEN => open(handle, &app, linked, work, ctl, data).await,
        KIND_TURN => turn(&app, linked, data).await,
        KIND_COLLECT => collect(&app, linked, data).await,
        KIND_STATE => state(ctl, data),
        _ => close(handle, &app, linked, data),
    })
}

async fn open(
    handle: &tauri::AppHandle,
    app: &crate::state::AppState,
    linked: &Arc<Linked>,
    work: &Arc<super::work::ClientWork>,
    ctl: &Arc<ControlContext>,
    data: &Value,
) -> Result<Value, String> {
    let agent = text(data, "agentId")?.to_string();
    let profile = ctl.store.load().agents.into_iter().find(|p| p.id == agent).ok_or("work-send-agent-missing")?;
    // 턴이 끝난 것을 알 수 없으면 호스트 실행기가 멈춘다.
    if !ctl.settings.read().unwrap().observer_enabled {
        return Err("discussion-observer-off".into());
    }
    let repo = data.get("repo").cloned();
    let cwd = {
        let (work, ctl, start) = (work.clone(), ctl.clone(), profile.cwd.clone());
        tokio::task::spawn_blocking(move || -> Option<PathBuf> {
            let repo = repo?;
            let url = repo.get("url").and_then(Value::as_str)?;
            work.clone_dir(&ctl, url, repo.get("subpath").and_then(Value::as_str).unwrap_or(""), start.as_deref())
        })
        .await
        .map_err(|e| e.to_string())?
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| profile.cwd.clone().filter(|c| !c.is_empty()))
    };
    let input = prepare_open(data, &profile, cwd.clone())?;
    let (topic, discussion_id) = (input.discussion.topic.clone(), input.discussion.id.clone());
    // 호스트가 정본이다. 호스트가 다시 열면(호스트 재시작 뒤 계속 등) 남은 세션을 버리고, 호스트가 닫지 못한 지난 토의
    // 세션도 버린다(동시 토의 1개 제한에 걸리지 않게).
    for (key, old) in linked.drain_except(&discussion_id, &input.agent_id) {
        dispose(handle, app, &key, Some(&old.session_id));
    }
    if app.manager.is_running(&input.agent_id) {
        dispose(handle, app, &input.agent_id, None);
    }
    let cli = input.discussion.participants.iter().find(|p| p.agent_id == input.participant).map(|p| p.agent.cli.clone()).unwrap_or(WorkCli::Claude);
    let result = ds::open_session(app, input).await?;
    linked.remember(&result.agent_id, LinkedSession { session_id: result.session_id.clone(), cwd: cwd.clone().unwrap_or_default(), cli: cli.clone() });
    let _ = handle.emit_to(
        "main",
        EVENT,
        json!({
            "opened": true,
            "agentId": result.agent_id,
            "sessionId": result.session_id,
            "discussionId": discussion_id,
            "participantAgentId": agent,
            "cli": cli,
            "collection": result.collection,
            "observed": result.observed,
            "submittedAt": result.submitted_at,
            "hostName": data.get("hostName").cloned().unwrap_or(json!("")),
            "topic": topic,
        }),
    );
    let mut out = serde_json::to_value(&result).map_err(|e| e.to_string())?;
    out["cwd"] = json!(cwd);
    Ok(out)
}

/// 호스트가 연 세션이고 그 세션이 지금 세션인가.
fn linked_session(linked: &Linked, data: &Value) -> Result<(String, LinkedSession), String> {
    let key = session_key(data)?;
    let session = linked.get(&key).ok_or("discussion-session-missing")?;
    if text(data, "sessionId")? != session.session_id {
        return Err("work-send-session-changed".into());
    }
    Ok((key, session))
}

async fn turn(app: &crate::state::AppState, linked: &Linked, data: &Value) -> Result<Value, String> {
    let (key, session) = linked_session(linked, data)?;
    let text = data.get("text").and_then(Value::as_str).unwrap_or("").to_string();
    let sent = ds::send_turn(app, crate::types::WorkFollowUpRequest { agent_id: key, session_id: session.session_id, text }).await?;
    Ok(json!({ "submittedAt": sent.submitted_at }))
}

async fn collect(app: &crate::state::AppState, linked: &Linked, data: &Value) -> Result<Value, String> {
    let (key, session) = linked_session(linked, data)?;
    let since = data.get("since").and_then(Value::as_u64).ok_or("bad-request")?;
    let request = crate::types::DiscussionCollectRequest { agent_id: key, session_id: session.session_id, cli: session.cli, since };
    let got = ds::collect_utterance_in(app, request, session.cwd).await?;
    serde_json::to_value(got).map_err(|e| e.to_string())
}

/// 세션이 살아 있는지, 턴 단계, `after`보다 뒤의 턴 토큰. 시각은 모두 이 머신 시계다(차례 보낸 시각도).
fn state(ctl: &ControlContext, data: &Value) -> Result<Value, String> {
    let key = session_key(data)?;
    let session = text(data, "sessionId")?;
    let alive = ctl.manager.session_id_for(&key).as_deref() == Some(session) && ctl.manager.is_running(&key);
    let turn = ctl.turns.get(session).map(|o| json!({ "phase": o.phase, "at": o.at }));
    let after = data.get("after").and_then(Value::as_u64).unwrap_or(0);
    Ok(json!({ "alive": alive, "turn": turn, "usage": ctl.turns.discussion_usage_after(session, after) }))
}

fn close(handle: &tauri::AppHandle, app: &crate::state::AppState, linked: &Linked, data: &Value) -> Result<Value, String> {
    let key = session_key(data)?;
    // 다른 세션(이 머신 사람이 다시 연 것)은 닫지 않는다.
    let expected = data.get("sessionId").and_then(Value::as_str).filter(|s| !s.is_empty());
    dispose(handle, app, &key, expected);
    linked.forget(&key);
    Ok(json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> crate::types::AgentProfile {
        serde_json::from_value(json!({
            "id": "ada", "name": "Ada", "role": "dev", "seed": "s", "personalityPrompt": "dry humor", "createdAt": 1, "deskIndex": 0,
        }))
        .unwrap()
    }

    fn open_data() -> Value {
        json!({
            "agentId": "ada",
            "participant": "dev1:ada",
            "members": { "host-mod": "Mo", "dev1:ada": "Ada" },
            "roles": [{ "id": "critic", "name": "Critic", "stance": "critical" }],
            "turn": "Say hi",
            "lang": "en",
            "discussion": {
                "id": "disc-1", "topic": "Pick", "cwd": "/host/only/path",
                "participants": [
                    { "agentId": "host-mod", "agent": { "cli": "claude" } },
                    { "agentId": "dev1:ada", "roleId": "critic", "agent": { "cli": "codex" } },
                ],
                "moderatorAgentId": "host-mod", "state": "running", "createdAt": 1, "updatedAt": 1,
            },
        })
    }

    #[test]
    fn host_discussion_opens_here_with_this_machine_folder_and_persona() {
        let input = prepare_open(&open_data(), &profile(), Some("/clone/app".into())).unwrap();
        assert_eq!(input.agent_id, "discuss-disc-1.ada", "세션은 이 머신 캐릭터 id로 연다");
        assert_eq!(input.participant, "dev1:ada", "첫 프롬프트는 호스트가 부르는 참가자 id로 쓴다");
        assert_eq!(input.discussion.cwd.as_deref(), Some("/clone/app"), "호스트 폴더 대신 이 머신 클론");
        assert_eq!(input.members["dev1:ada"].personality.as_deref(), Some("dry humor"));
        assert_eq!(input.members["host-mod"].name, "Mo");
        assert_eq!(input.roles.len(), 1);
        let prompt = ds::build_briefing(
            &ds::Briefing {
                discussion: &input.discussion,
                participant: &input.participant,
                members: &input.members,
                roles: &input.roles,
                shim: std::path::Path::new("/bin/office-discuss"),
                turn: &input.turn,
            },
            crate::i18n::Lang::En,
        );
        assert!(prompt.contains("/clone/app") && !prompt.contains("/host/only/path"), "{prompt}");
        assert!(prompt.contains("Critic") && prompt.contains("Mo"));

        // 클론도 시작 폴더도 없으면 홈에서 연다.
        assert_eq!(prepare_open(&open_data(), &profile(), None).unwrap().discussion.cwd, None);
        // 참가자가 아니거나 끝난 토의는 열지 않는다.
        let mut data = open_data();
        data["participant"] = json!("dev1:nobody");
        assert_eq!(prepare_open(&data, &profile(), None).err().as_deref(), Some("discussion-session-not-participant"));
        let mut data = open_data();
        data["discussion"]["state"] = json!("finished");
        assert_eq!(prepare_open(&data, &profile(), None).err().as_deref(), Some("discussion-session-finished"));
        let mut data = open_data();
        data["discussion"] = json!("nope");
        assert_eq!(prepare_open(&data, &profile(), None).err().as_deref(), Some("bad-request"));
    }

    #[test]
    fn linked_sessions_answer_only_for_the_session_the_host_opened() {
        let linked = Linked::default();
        linked.remember("discuss-disc-1.ada", LinkedSession { session_id: "s-1".into(), cwd: "/c".into(), cli: WorkCli::Codex });
        let ask = |session: &str| json!({ "discussionId": "disc-1", "agentId": "ada", "sessionId": session });
        assert_eq!(linked_session(&linked, &ask("s-1")).unwrap().1.cwd, "/c");
        assert_eq!(linked_session(&linked, &ask("s-2")).unwrap_err(), "work-send-session-changed");
        assert_eq!(linked_session(&linked, &json!({ "discussionId": "disc-1", "agentId": "bob", "sessionId": "s-1" })).unwrap_err(), "discussion-session-missing");
        assert_eq!(linked_session(&linked, &json!({ "discussionId": "../x", "agentId": "ada", "sessionId": "s-1" })).unwrap_err(), "discussion-session-invalid-id");
        linked.remember("discuss-disc-1.bob", LinkedSession { session_id: "s-b".into(), cwd: String::new(), cli: WorkCli::Claude });
        linked.remember("discuss-old.ada", LinkedSession { session_id: "s-o".into(), cwd: String::new(), cli: WorkCli::Claude });
        let mut dropped: Vec<String> = linked.drain_except("disc-1", "discuss-disc-1.ada").into_iter().map(|(k, _)| k).collect();
        dropped.sort();
        assert_eq!(dropped, vec!["discuss-disc-1.ada", "discuss-old.ada"], "다시 여는 세션과 지난 토의 세션만 버린다");
        assert!(linked.get("discuss-disc-1.bob").is_some());
        for n in 0..MAX_LINKED + 3 {
            linked.remember(&format!("discuss-d.a{n}"), LinkedSession { session_id: "x".into(), cwd: String::new(), cli: WorkCli::Claude });
        }
        assert!(linked.sessions.lock().unwrap().len() <= MAX_LINKED);
    }
}
