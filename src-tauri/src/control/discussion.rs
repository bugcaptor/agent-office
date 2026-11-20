// src-tauri/src/control/discussion.rs
//
// 외부 에이전트·스크립트가 토의를 시작하고 결과를 받는 라우트(`ctl discuss …`). 읽기(목록·상태·
// 결정 메모)는 저장된 `discussions.json`과 메모 파일에서 바로 답하고, 쓰기(시작·사회자 질문에
// 답하기)는 토의 실행기를 쥔 렌더러에 넘긴다 — 카드 버튼과 같은 함수(`discussionRun.ts`)를 타므로
// 진행 시간·실행 기록(activeMs·run.*)을 ctl이 따로 쓰지 않는다. 쓰기는 'CLI 자동 진행 허용'이 필요하다.
// 결론 확인(review → finished)은 사람의 일이라 열지 않는다.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Json, State};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::httpapi::{fail, ok};
use crate::persistence::discussion_inbox;
use crate::persistence::discussion_store::DiscussionStore;
use crate::types::{Discussion, DiscussionPauseReason, DiscussionState};

use super::autorun::MAX_WAIT_SECS;
use super::ControlContext;

/// 방금 만든 토의를 앱이 수신함에서 가져갈 때까지 시작·답하기가 기다리는 시간.
const IMPORT_WAIT: Duration = Duration::from_secs(10);
const LAST_TEXT_CHARS: usize = 400;

// ── 상태 조립(순수) ───────────────────────────────────────────────────

fn state_str(state: DiscussionState) -> &'static str {
    match state {
        DiscussionState::Ready => "ready",
        DiscussionState::Running => "running",
        DiscussionState::WaitingUser => "waiting_user",
        DiscussionState::WrappingUp => "wrapping_up",
        DiscussionState::Review => "review",
        DiscussionState::Finished => "finished",
        DiscussionState::Paused => "paused",
    }
}

fn enum_str<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

fn clip(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() > LAST_TEXT_CHARS {
        format!("{}…", chars[..LAST_TEXT_CHARS].iter().collect::<String>())
    } else {
        text.to_string()
    }
}

/// 토의 하나의 상태. `name`은 팀원 이름을, `memo`는 결정 메모 경로(파일이 있을 때)를 준다.
/// `can_write`는 ctl 쓰기(시작·답하기)가 허용됐는지 — 고를 수 있는 다음 동작에 반영한다.
pub fn build_status(
    discussions: &[Discussion],
    pending: &[String],
    id: &str,
    name: &dyn Fn(&str) -> Option<String>,
    memo: &dyn Fn(&str) -> Option<String>,
    can_write: bool,
) -> Result<Value, String> {
    let Some(d) = discussions.iter().find(|d| d.id == id) else {
        if pending.iter().any(|p| p == id) {
            // 아직 앱이 수신함에서 가져가지 않았다. 몇 초 안에 준비 상태가 된다.
            return Ok(with_rev(json!({ "id": id, "state": "pending" })));
        }
        return Err("discussion-not-found".into());
    };
    let run = d.run.as_ref();
    let usage_wait = run.is_some_and(|r| r.pause_reason == Some(DiscussionPauseReason::UsageWait));
    let state = match d.state {
        DiscussionState::Paused if usage_wait => "waiting-usage",
        other => state_str(other),
    };
    let writes = |actions: &[&'static str]| -> Vec<&'static str> {
        actions.iter().copied().filter(|a| can_write || !matches!(*a, "start" | "answer")).collect()
    };
    let attention = match d.state {
        DiscussionState::Ready => Some(json!({ "reason": "not-started", "actions": writes(&["start", "wait"]) })),
        DiscussionState::WaitingUser => {
            let mut a = json!({ "reason": "question", "actions": writes(&["answer", "wait"]) });
            if let Some(q) = run.and_then(|r| r.question.as_deref()).filter(|q| !q.trim().is_empty()) {
                a["detail"] = json!(q);
            }
            Some(a)
        }
        DiscussionState::Paused if usage_wait => None,
        // 멈춘 토의는 사람이 카드에서 계속하거나 끝낸다.
        DiscussionState::Paused => {
            let reason = run.and_then(|r| r.pause_reason).map(|r| enum_str(&r)).unwrap_or_else(|| "user".into());
            let mut a = json!({ "reason": reason, "actions": ["wait"] });
            if let Some(detail) = run.and_then(|r| r.pause_detail.as_deref()).filter(|t| !t.trim().is_empty()) {
                a["detail"] = json!(detail);
            }
            Some(a)
        }
        _ => None,
    };
    let participants: Vec<Value> = d
        .participants
        .iter()
        .map(|p| {
            let mut v = json!({
                "agentId": p.agent_id,
                "moderator": p.agent_id == d.moderator_agent_id,
                "cli": enum_str(&p.agent.cli),
            });
            if let Some(n) = name(&p.agent_id) {
                v["name"] = json!(n);
            }
            if let Some(r) = &p.role_id {
                v["roleId"] = json!(r);
            }
            if let Some(m) = &p.agent.model {
                v["model"] = json!(m);
            }
            v
        })
        .collect();
    let mut out = json!({
        "id": d.id,
        "topic": d.topic,
        "state": state,
        "round": d.round,
        "limits": d.limits,
        "costUsd": d.cost_usd,
        "activeMs": d.active_ms.unwrap_or(0),
        "participants": participants,
        "utterances": d.transcript.len(),
        "createdAt": d.created_at,
        "updatedAt": d.updated_at,
    });
    let memo_path = memo(&d.id);
    out["memo"] = json!(memo_path.is_some());
    if let Some(path) = memo_path {
        out["memoPath"] = json!(path);
    }
    if let Some(a) = attention {
        out["attention"] = a;
    }
    for (key, value) in [("activeSince", d.active_since), ("startedAt", d.started_at), ("endedAt", d.ended_at)] {
        if let Some(v) = value {
            out[key] = json!(v);
        }
    }
    if let Some(r) = run {
        out["phase"] = json!(enum_str(&r.phase));
        if let Some(f) = r.forced {
            out["forced"] = json!(enum_str(&f));
        }
        if let Some(w) = &r.usage_wait {
            out["usageWait"] = serde_json::to_value(w).unwrap_or(Value::Null);
        }
    }
    if let Some(last) = d.transcript.last() {
        let mut v = json!({ "kind": enum_str(&last.kind), "round": last.round, "text": clip(&last.text) });
        if let Some(a) = &last.agent_id {
            v["agentId"] = json!(a);
        }
        out["last"] = v;
    }
    Ok(with_rev(out))
}

/// 내용이 바뀔 때만 달라지는 짧은 지문. `--wait --since`가 놓친 변화 없이 기다리게 한다.
fn with_rev(mut status: Value) -> Value {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    status.to_string().hash(&mut h);
    status["rev"] = json!(format!("{:016x}", h.finish()));
    status
}

/// 기다림을 끝낼 상태: 결론이 났거나 누군가 손을 써야 한다(시작·답·계속).
pub fn is_settled(status: &Value) -> bool {
    matches!(
        status.get("state").and_then(Value::as_str),
        Some("ready" | "waiting_user" | "paused" | "review" | "finished")
    )
}

// ── 라우트 ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StatusParams {
    id: String,
    #[serde(default)]
    wait_secs: Option<u64>,
    #[serde(default)]
    since: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IdParams {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AnswerParams {
    id: String,
    text: String,
}

fn store(ctx: &ControlContext) -> DiscussionStore {
    // 읽기만 한다. 앱 쓰기는 원자적 교체라 읽는 쪽이 반쯤 쓴 파일을 보지 않는다.
    DiscussionStore::new(ctx.app_data_dir.clone())
}

fn pending(ctx: &ControlContext) -> Vec<String> {
    discussion_inbox::pending_ids(&ctx.app_data_dir.join("work-inbox"))
}

fn writes_allowed(ctx: &ControlContext) -> bool {
    ctx.settings.read().unwrap().cli_auto_run_enabled
}

fn current_status(ctx: &ControlContext, id: &str) -> Result<Value, String> {
    let store = store(ctx);
    let discussions = store.load()?;
    let profiles = ctx.store.load();
    build_status(
        &discussions,
        &pending(ctx),
        id,
        &|agent_id| profiles.agents.iter().find(|a| a.id == agent_id).map(|a| a.name.clone()),
        &|id| store.existing_memo_path(id).ok().map(|p| p.to_string_lossy().into_owned()),
        writes_allowed(ctx),
    )
}

pub(super) async fn list(State(ctx): State<Arc<ControlContext>>) -> Json<Value> {
    let discussions = match store(&ctx).load() {
        Ok(d) => d,
        Err(e) => return fail(e),
    };
    let mut rows: Vec<Value> = pending(&ctx)
        .into_iter()
        .filter(|id| !discussions.iter().any(|d| &d.id == id))
        .map(|id| json!({ "id": id, "state": "pending" }))
        .collect();
    rows.extend(discussions.iter().map(|d| json!({ "id": d.id, "topic": d.topic, "state": state_str(d.state), "updatedAt": d.updated_at })));
    ok(rows)
}

pub(super) async fn status(State(ctx): State<Arc<ControlContext>>, Json(p): Json<StatusParams>) -> Json<Value> {
    let wait = Duration::from_secs(p.wait_secs.unwrap_or(0).min(MAX_WAIT_SECS));
    let deadline = Instant::now() + wait;
    loop {
        let status = match current_status(&ctx, &p.id) {
            Ok(s) => s,
            Err(e) => return fail(e),
        };
        let changed = p.since.as_deref().is_some_and(|since| status["rev"].as_str() != Some(since));
        let settled = changed || (p.since.is_none() && is_settled(&status));
        let now = Instant::now();
        if wait.is_zero() || settled || now >= deadline {
            let mut status = status;
            if !wait.is_zero() {
                status["timedOut"] = json!(!settled);
            }
            return ok(status);
        }
        // 토의는 렌더러가 파일로만 저장하므로 짧게 다시 읽는다.
        tokio::time::sleep((deadline - now).min(Duration::from_millis(500))).await;
    }
}

pub(super) async fn memo(State(ctx): State<Arc<ControlContext>>, Json(p): Json<IdParams>) -> Json<Value> {
    let store = store(&ctx);
    let discussions = match store.load() {
        Ok(d) => d,
        Err(e) => return fail(e),
    };
    let Some(d) = discussions.iter().find(|d| d.id == p.id) else {
        return fail(if pending(&ctx).contains(&p.id) { "discussion-pending" } else { "discussion-not-found" });
    };
    match store.read_memo(&d.id) {
        Ok(Some(markdown)) => {
            let path = store.existing_memo_path(&d.id).ok().map(|p| p.to_string_lossy().into_owned());
            ok(json!({ "id": d.id, "state": state_str(d.state), "path": path, "markdown": markdown }))
        }
        Ok(None) => fail("discussion-memo-not-found"),
        Err(e) => fail(e),
    }
}

/// 방금 만든 토의를 앱이 가져갈 때까지 기다린다. 없거나 끝내 가져가지 않으면 이유 코드.
async fn await_imported(ctx: &ControlContext, id: &str) -> Result<(), &'static str> {
    let deadline = Instant::now() + IMPORT_WAIT;
    loop {
        if store(ctx).load().map_err(|_| "discussion-store-unreadable")?.iter().any(|d| d.id == id) {
            return Ok(());
        }
        if !pending(ctx).iter().any(|p| p == id) {
            return Err("discussion-not-found");
        }
        if Instant::now() >= deadline {
            return Err("discussion-pending");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn forward(ctx: &ControlContext, op: &str, id: &str, options: Value) -> Json<Value> {
    if !writes_allowed(ctx) {
        return fail("cli-auto-run-disabled");
    }
    if let Err(code) = await_imported(ctx, id).await {
        return fail(code);
    }
    let payload = json!({ "op": op, "discussionId": id, "options": options });
    match ctx.discussion.send(payload).await {
        Ok(_) => match current_status(ctx, id) {
            Ok(s) => ok(s),
            Err(_) => ok(Value::Null),
        },
        Err(e) => fail(e),
    }
}

pub(super) async fn start(State(ctx): State<Arc<ControlContext>>, Json(p): Json<IdParams>) -> Json<Value> {
    forward(&ctx, "start", &p.id, json!({})).await
}

pub(super) async fn answer(State(ctx): State<Arc<ControlContext>>, Json(p): Json<AnswerParams>) -> Json<Value> {
    if p.text.trim().is_empty() {
        return fail("discuss answer: 답할 내용이 필요합니다");
    }
    forward(&ctx, "answer", &p.id, json!({ "text": p.text })).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DiscussionLimits, DiscussionParticipant, DiscussionPhase, DiscussionRun, WorkAgent, WorkCli};

    fn discussion(state: DiscussionState) -> Discussion {
        let p = |id: &str| DiscussionParticipant { agent_id: id.into(), role_id: None, agent: WorkAgent { cli: WorkCli::Codex, model: Some("m".into()), effort: None } };
        Discussion {
            id: "disc-1".into(), topic: "주제".into(), context: String::new(), cwd: None,
            participants: vec![p("mod"), p("a1")], moderator_agent_id: "mod".into(),
            limits: DiscussionLimits::default(), state, round: 0, transcript: vec![], decision_memo_path: None,
            cost_usd: 0.0, created_at: 1, updated_at: 1, started_at: None, ended_at: None,
            active_ms: None, active_since: None, run: None,
        }
    }
    fn run(phase: DiscussionPhase) -> DiscussionRun {
        DiscussionRun { phase, turns: vec![], seen: Default::default(), question: None, pause_reason: None, pause_detail: None, forced: None, forced_at_ms: None, usage_wait: None, usage_override: None }
    }
    fn status(list: &[Discussion], pending: &[String], can_write: bool) -> Result<Value, String> {
        build_status(list, pending, "disc-1", &|id| (id == "mod").then(|| "사회자".to_string()), &|_| None, can_write)
    }

    #[test]
    fn states_map_to_attention_and_settling() {
        let ready = status(&[discussion(DiscussionState::Ready)], &[], true).unwrap();
        assert_eq!(ready["state"], "ready");
        assert_eq!(ready["attention"], json!({ "reason": "not-started", "actions": ["start", "wait"] }));
        assert!(is_settled(&ready));
        // 쓰기가 막혀 있으면 시작·답하기를 고를 수 없다.
        let ready = status(&[discussion(DiscussionState::Ready)], &[], false).unwrap();
        assert_eq!(ready["attention"]["actions"], json!(["wait"]));

        let mut asking = discussion(DiscussionState::WaitingUser);
        asking.run = Some(DiscussionRun { question: Some("예산은?".into()), ..run(DiscussionPhase::User) });
        let s = status(&[asking], &[], true).unwrap();
        assert_eq!(s["attention"], json!({ "reason": "question", "detail": "예산은?", "actions": ["answer", "wait"] }));
        assert!(is_settled(&s));

        let mut waiting = discussion(DiscussionState::Paused);
        waiting.run = Some(DiscussionRun { pause_reason: Some(DiscussionPauseReason::UsageWait), ..run(DiscussionPhase::Speakers) });
        let s = status(&[waiting], &[], true).unwrap();
        assert_eq!(s["state"], "waiting-usage");
        assert!(s.get("attention").is_none() && !is_settled(&s));

        let mut stuck = discussion(DiscussionState::Paused);
        stuck.run = Some(DiscussionRun { pause_reason: Some(DiscussionPauseReason::NoUtterance), ..run(DiscussionPhase::Speakers) });
        let s = status(&[stuck], &[], true).unwrap();
        assert_eq!(s["attention"], json!({ "reason": "no-utterance", "actions": ["wait"] }));
        assert_eq!(s["phase"], "speakers");

        for (state, settled) in [(DiscussionState::Running, false), (DiscussionState::WrappingUp, false), (DiscussionState::Review, true), (DiscussionState::Finished, true)] {
            assert_eq!(is_settled(&status(&[discussion(state)], &[], true).unwrap()), settled, "{state:?}");
        }
    }

    #[test]
    fn pending_unknown_memo_and_participants() {
        let s = status(&[], &["disc-1".into()], true).unwrap();
        assert_eq!(s["state"], "pending");
        assert!(!is_settled(&s));
        assert_eq!(status(&[], &[], true).unwrap_err(), "discussion-not-found");

        let s = build_status(&[discussion(DiscussionState::Review)], &[], "disc-1", &|_| None, &|_| Some("/m/disc-1.md".into()), true).unwrap();
        assert_eq!((s["memo"].clone(), s["memoPath"].clone()), (json!(true), json!("/m/disc-1.md")));
        let s = status(&[discussion(DiscussionState::Review)], &[], true).unwrap();
        assert_eq!(s["memo"], false);
        assert_eq!(s["participants"][0], json!({ "agentId": "mod", "moderator": true, "cli": "codex", "model": "m", "name": "사회자" }));
        // 같은 내용이면 같은 rev, 바뀌면 다른 rev.
        let a = status(&[discussion(DiscussionState::Running)], &[], true).unwrap();
        let b = status(&[discussion(DiscussionState::Running)], &[], true).unwrap();
        let c = status(&[discussion(DiscussionState::Review)], &[], true).unwrap();
        assert_eq!(a["rev"], b["rev"]);
        assert_ne!(a["rev"], c["rev"]);
    }
}
