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
use crate::persistence::work_inbox::{forbidden_control, MAX_BYTES};
use crate::types::{Discussion, DiscussionPauseReason, DiscussionState};

use super::autorun::MAX_WAIT_SECS;
use super::{enum_str, with_rev, ControlContext};

/// 방금 만든 토의를 앱이 수신함에서 가져갈 때까지 시작·답하기가 기다리는 시간.
const IMPORT_WAIT: Duration = Duration::from_secs(10);
const LAST_TEXT_CHARS: usize = 400;

// ── 상태 조립(순수) ───────────────────────────────────────────────────

fn usage_waiting(d: &Discussion) -> bool {
    d.state == DiscussionState::Paused
        && d.run.as_ref().is_some_and(|r| r.pause_reason == Some(DiscussionPauseReason::UsageWait))
}

/// 응답의 `state`. 저장 상태의 serde 이름이고, 사용량 대기로 멈춘 것만 `waiting-usage`다(스스로 이어 간다).
fn state_label(d: &Discussion) -> String {
    if usage_waiting(d) { "waiting-usage".into() } else { enum_str(&d.state) }
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
    let usage_wait = usage_waiting(d);
    let state = state_label(d);
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

/// 사람(CLI)이 사회자에게 주는 답. CRLF·CR은 LF로 바꾸고, 탭·줄바꿈 밖의 제어 문자나 수신함 상한을
/// 넘는 답은 거절한다 — 기록에 들어가면 이후 사회자 프롬프트가 모두 보내기 검사(`work_prompt`)에 막힌다.
pub fn normalize_answer(text: &str) -> Result<String, &'static str> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    if text.trim().is_empty() {
        return Err("discussion-answer-empty");
    }
    if text.len() > MAX_BYTES || forbidden_control(&text).is_some() {
        return Err("discussion-answer-invalid");
    }
    Ok(text)
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

/// 수신함을 먼저, 저장소를 나중에 읽는다. 가져오기는 저장한 뒤 파일을 지우므로 이 순서여야 그 사이에
/// 가져간 토의가 둘 중 어디에도 없는 순간(`discussion-not-found`)을 보지 않는다.
fn read_both(
    pending: impl FnOnce() -> Vec<String>,
    load: impl FnOnce() -> Result<Vec<Discussion>, String>,
) -> Result<(Vec<String>, Vec<Discussion>), String> {
    let pending = pending();
    Ok((pending, load()?))
}

fn snapshot(ctx: &ControlContext, store: &DiscussionStore) -> Result<(Vec<String>, Vec<Discussion>), String> {
    read_both(|| pending(ctx), || store.load())
}

fn current_status(ctx: &ControlContext, id: &str) -> Result<Value, String> {
    let store = store(ctx);
    let (pending, discussions) = snapshot(ctx, &store)?;
    let profiles = ctx.store.load();
    build_status(
        &discussions,
        &pending,
        id,
        &|agent_id| profiles.agents.iter().find(|a| a.id == agent_id).map(|a| a.name.clone()),
        &|id| store.existing_memo_path(id).ok().map(|p| p.to_string_lossy().into_owned()),
        writes_allowed(ctx),
    )
}

pub(super) async fn list(State(ctx): State<Arc<ControlContext>>) -> Json<Value> {
    let (pending, discussions) = match snapshot(&ctx, &store(&ctx)) {
        Ok(both) => both,
        Err(e) => return fail(e),
    };
    let mut rows: Vec<Value> = pending
        .into_iter()
        .filter(|id| !discussions.iter().any(|d| &d.id == id))
        .map(|id| json!({ "id": id, "state": "pending" }))
        .collect();
    rows.extend(discussions.iter().map(|d| json!({ "id": d.id, "topic": d.topic, "state": state_label(d), "updatedAt": d.updated_at })));
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
    let (pending, discussions) = match snapshot(&ctx, &store) {
        Ok(both) => both,
        Err(e) => return fail(e),
    };
    let Some(d) = discussions.iter().find(|d| d.id == p.id) else {
        return fail(if pending.contains(&p.id) { "discussion-pending" } else { "discussion-not-found" });
    };
    match store.read_memo(&d.id) {
        Ok(Some(markdown)) => {
            let path = store.existing_memo_path(&d.id).ok().map(|p| p.to_string_lossy().into_owned());
            ok(json!({ "id": d.id, "state": state_label(d), "path": path, "markdown": markdown }))
        }
        Ok(None) => fail("discussion-memo-not-found"),
        Err(e) => fail(e),
    }
}

/// 방금 만든 토의를 앱이 가져갈 때까지 기다린다. 없거나 끝내 가져가지 않으면 이유 코드.
async fn await_imported(ctx: &ControlContext, id: &str) -> Result<(), &'static str> {
    let deadline = Instant::now() + IMPORT_WAIT;
    let store = store(ctx);
    loop {
        let (pending, discussions) = snapshot(ctx, &store).map_err(|_| "discussion-store-unreadable")?;
        if discussions.iter().any(|d| d.id == id) {
            return Ok(());
        }
        if !pending.iter().any(|p| p == id) {
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
    if let Err(e) = ctx.discussion.send(payload).await {
        return fail(e);
    }
    match current_status(ctx, id) {
        Ok(status) => ok(status),
        Err(e) => fail(e),
    }
}

pub(super) async fn start(State(ctx): State<Arc<ControlContext>>, Json(p): Json<IdParams>) -> Json<Value> {
    forward(&ctx, "start", &p.id, json!({})).await
}

pub(super) async fn answer(State(ctx): State<Arc<ControlContext>>, Json(p): Json<AnswerParams>) -> Json<Value> {
    let text = match normalize_answer(&p.text) {
        Ok(text) => text,
        Err("discussion-answer-empty") => return fail("discuss answer: 답할 내용이 필요합니다"),
        Err(code) => return fail(code),
    };
    forward(&ctx, "answer", &p.id, json!({ "text": text })).await
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
        DiscussionRun { phase, turns: vec![], seen: Default::default(), question: None, pause_reason: None, pause_detail: None, forced: None, forced_at_ms: None, usage_wait: None, usage_override: None, pending_memo: None }
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

    #[test]
    fn a_discussion_imported_between_the_two_reads_is_still_found() {
        let tmp = tempfile::tempdir().unwrap();
        let inbox = tmp.path().join("work-inbox");
        let store = DiscussionStore::new(tmp.path().to_path_buf());
        let d = discussion(DiscussionState::Ready);
        let request = discussion_inbox::DiscussionRequest {
            version: discussion_inbox::REQUEST_VERSION, id: d.id.clone(), topic: d.topic.clone(), context: String::new(), cwd: None,
            moderator_agent_id: d.moderator_agent_id.clone(), participants: d.participants.clone(), limits: d.limits.clone(), requested_by: None,
        };
        let dir = discussion_inbox::inbox_dir(&inbox);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("1-a.json"), serde_json::to_vec(&request).unwrap()).unwrap();
        // 앱이 두 읽기 사이에 가져간다(저장 → 파일 삭제).
        let (pending, discussions) = read_both(
            || discussion_inbox::pending_ids(&inbox),
            || {
                assert_eq!(discussion_inbox::import(&inbox, &store, &|_| true, 5).unwrap().len(), 1);
                store.load()
            },
        )
        .unwrap();
        assert_eq!(status(&discussions, &pending, true).unwrap()["state"], "ready");
    }

    #[test]
    fn answers_are_normalized_and_control_characters_rejected() {
        assert_eq!(normalize_answer("a\r\nb\rc\td").unwrap(), "a\nb\nc\td");
        assert_eq!(normalize_answer(" \r\n ").unwrap_err(), "discussion-answer-empty");
        // `git log --color` 같은 출력이 기록에 들어가면 이후 사회자 프롬프트가 모두 막힌다.
        assert_eq!(normalize_answer("\x1b[33mabc\x1b[0m").unwrap_err(), "discussion-answer-invalid");
        assert_eq!(normalize_answer("a\0b").unwrap_err(), "discussion-answer-invalid");
        assert!(normalize_answer(&"x".repeat(MAX_BYTES)).is_ok());
        assert_eq!(normalize_answer(&"x".repeat(MAX_BYTES + 1)).unwrap_err(), "discussion-answer-invalid");
    }

    #[test]
    fn list_and_status_report_a_usage_wait_the_same_way() {
        let mut waiting = discussion(DiscussionState::Paused);
        waiting.run = Some(DiscussionRun { pause_reason: Some(DiscussionPauseReason::UsageWait), ..run(DiscussionPhase::Speakers) });
        assert_eq!(state_label(&waiting), "waiting-usage");
        assert_eq!(status(&[waiting], &[], true).unwrap()["state"], "waiting-usage");
        assert_eq!(state_label(&discussion(DiscussionState::WaitingUser)), "waiting_user");
        assert_eq!(state_label(&discussion(DiscussionState::Paused)), "paused");
    }
}
