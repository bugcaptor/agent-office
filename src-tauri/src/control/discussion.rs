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
/// 참가자별 최근 발언은 중계용 한 줄이라 더 짧게 자른다(전문은 결정 메모·토의 탭에서).
const PARTICIPANT_TEXT_CHARS: usize = 160;

// ── 상태 조립(순수) ───────────────────────────────────────────────────

fn usage_waiting(d: &Discussion) -> bool {
    d.state == DiscussionState::Paused
        && d.run.as_ref().is_some_and(|r| r.pause_reason == Some(DiscussionPauseReason::UsageWait))
}

/// 응답의 `state`. 저장 상태의 serde 이름이고, 사용량 대기로 멈춘 것만 `waiting-usage`다(스스로 이어 간다).
fn state_label(d: &Discussion) -> String {
    if usage_waiting(d) { "waiting-usage".into() } else { enum_str(&d.state) }
}

fn clip(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() > max {
        format!("{}…", chars[..max].iter().collect::<String>())
    } else {
        text.to_string()
    }
}

/// 지금 차례를 받아 말하는 중인 참가자와 차례를 받은 시각. 진행·정리 중에 차례를 보냈고 발언을 아직 받지
/// 않은 참가자다(렌더러 `officeStage.ts`의 `speakingSince`와 같은 규칙).
fn speaking_since(d: &Discussion, agent_id: &str) -> Option<i64> {
    if !matches!(d.state, DiscussionState::Running | DiscussionState::WrappingUp) {
        return None;
    }
    d.run.as_ref()?.turns.iter().find(|t| t.agent_id == agent_id && t.text.is_none()).and_then(|t| t.sent_at)
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
            // 중계용: 말하는 중인지와 최근 발언 한 줄. 시각은 절대값만 준다 — "몇 분 전"을 넣으면 내용이
            // 그대로여도 rev가 바뀌어 `--since` 대기가 쓸모없어진다.
            let since = speaking_since(d, &p.agent_id);
            v["speaking"] = json!(since.is_some());
            if let Some(at) = since {
                v["speakingSince"] = json!(at);
            }
            // 기록에는 공개된 발언만 있다(동시 첫 라운드는 전원이 낸 뒤 한꺼번에 들어온다).
            if let Some(u) = d.transcript.iter().rev().find(|u| u.agent_id.as_deref() == Some(p.agent_id.as_str())) {
                v["last"] = json!({ "at": u.at, "round": u.round, "kind": enum_str(&u.kind), "text": clip(&u.text, PARTICIPANT_TEXT_CHARS) });
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
        let mut v = json!({ "kind": enum_str(&last.kind), "round": last.round, "text": clip(&last.text, LAST_TEXT_CHARS) });
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

pub(crate) fn current_status(ctx: &ControlContext, id: &str) -> Result<Value, String> {
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
    match list_rows(&ctx) {
        Ok(rows) => ok(rows),
        Err(e) => fail(e),
    }
}

/// 목록 행(수신함에서 아직 안 가져간 것 먼저). `ctl discuss list`와 웹 상태판이 같이 쓴다.
pub(crate) fn list_rows(ctx: &ControlContext) -> Result<Vec<Value>, String> {
    let (pending, discussions) = snapshot(ctx, &store(ctx))?;
    let mut rows: Vec<Value> = pending
        .into_iter()
        .filter(|id| !discussions.iter().any(|d| &d.id == id))
        .map(|id| json!({ "id": id, "state": "pending" }))
        .collect();
    rows.extend(discussions.iter().map(|d| json!({ "id": d.id, "topic": d.topic, "state": state_label(d), "updatedAt": d.updated_at })));
    Ok(rows)
}

/// 공개된 발언(속기록). 없는 토의면 None.
pub(crate) fn transcript(ctx: &ControlContext, id: &str) -> Option<Vec<crate::types::DiscussionUtterance>> {
    store(ctx).load().ok()?.into_iter().find(|d| d.id == id).map(|d| d.transcript)
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
    use crate::types::{
        DiscussionLimits, DiscussionParticipant, DiscussionPhase, DiscussionRun, DiscussionTurn, DiscussionUtterance, DiscussionUtteranceKind,
        WorkAgent, WorkCli,
    };

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
        assert_eq!(s["participants"][0], json!({ "agentId": "mod", "moderator": true, "cli": "codex", "model": "m", "name": "사회자", "speaking": false }));
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

    #[test]
    fn participants_carry_speaking_and_their_latest_published_utterance() {
        let said = |at: i64, agent: Option<&str>, kind: DiscussionUtteranceKind, text: &str| DiscussionUtterance {
            at, round: 1, kind, agent_id: agent.map(String::from), text: text.into(),
        };
        let turn = |agent: &str, sent_at: Option<i64>, text: Option<&str>| DiscussionTurn {
            agent_id: agent.into(), question: None, session_id: None, sent_at, nudged: None, text: text.map(String::from),
        };
        let mut d = discussion(DiscussionState::Running);
        d.transcript = vec![
            said(10, Some("a1"), DiscussionUtteranceKind::Opening, "첫 의견"),
            said(20, Some("mod"), DiscussionUtteranceKind::Moderator, "- a1: 예산은?"),
            said(30, None, DiscussionUtteranceKind::Notice, "알림"),
            said(40, Some("a1"), DiscussionUtteranceKind::Statement, &"가".repeat(PARTICIPANT_TEXT_CHARS + 5)),
        ];
        // 사회자 차례: a1의 이전 차례는 이미 받았고(text), 사회자는 아직 말하는 중이다.
        d.run = Some(DiscussionRun { turns: vec![turn("mod", Some(50), None), turn("a1", Some(35), Some("받음"))], ..run(DiscussionPhase::Moderator) });
        let s = status(&[d.clone()], &[], true).unwrap();
        let (m, a) = (&s["participants"][0], &s["participants"][1]);
        assert_eq!((m["speaking"].clone(), m["speakingSince"].clone()), (json!(true), json!(50)));
        assert_eq!(m["last"], json!({ "at": 20, "round": 1, "kind": "moderator", "text": "- a1: 예산은?" }));
        assert_eq!((a["speaking"].clone(), a.get("speakingSince")), (json!(false), None));
        assert_eq!(a["last"]["at"], 40);
        assert_eq!(a["last"]["text"].as_str().unwrap().chars().count(), PARTICIPANT_TEXT_CHARS + 1); // + "…"

        // 멈춘 토의는 차례가 남아 있어도 말하는 중이 아니다.
        let mut paused = d.clone();
        paused.state = DiscussionState::Paused;
        assert_eq!(status(&[paused], &[], true).unwrap()["participants"][0]["speaking"], false);

        // 동시 첫 라운드에서 받았지만 공개 전인 발언(turn.text)은 내보내지 않는다.
        let mut opening = discussion(DiscussionState::Running);
        opening.run = Some(DiscussionRun { turns: vec![turn("a1", Some(5), Some("비공개"))], ..run(DiscussionPhase::Opening) });
        let s = status(&[opening], &[], true).unwrap();
        assert_eq!((s["participants"][1]["speaking"].clone(), s["participants"][1].get("last")), (json!(false), None));
    }
}
