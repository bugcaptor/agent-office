// src-tauri/src/control/autorun.rs
//
// 외부 에이전트가 플랜을 맡기고 결과를 받는 라우트. 읽기(`status`·`state`)는
// 저장된 업무(`work-items.json`)와 턴 추적기에서 바로 답하고, 쓰기(시작·재개·
// 멈춤·끝내기·넘기기)는 자동 진행 루프를 쥔 렌더러에 요청·응답으로 넘긴다 —
// GUI 버튼과 같은 함수(`autoRun.ts`)를 타야 두 경로가 갈라지지 않는다.
// `ask`는 플랜 없이 한 번 묻고 그 턴의 마지막 응답을 돌려준다.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Json, State};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::httpapi::{fail, ok};
use crate::types::{WorkFollowUpRequest, WorkItem, WorkStatus};

use super::turns::{TurnObservation, TurnPhase};
use super::ControlContext;

/// 같은 단계에서 CLI가 재개할 수 있는 횟수. 넘으면 사람에게 넘겨야 한다.
pub const MAX_CTL_RESUMES: usize = 2;
/// 렌더러가 쓰기 요청에 답할 시간.
const BRIDGE_TIMEOUT: Duration = Duration::from_secs(5);
/// `status --wait` 요청 하나가 붙잡는 최대 시간. 더 길게는 클라이언트가 다시 묻는다.
pub const MAX_WAIT_SECS: u64 = 50;
/// `ask`가 기다리는 최대 시간.
pub const MAX_ASK_SECS: u64 = 3600;
const LOG_TAIL: usize = 20;

// ── 상태 조립(순수) ───────────────────────────────────────────────────

fn title(item: &WorkItem) -> String {
    let t = item.title.trim();
    if !t.is_empty() {
        return t.to_string();
    }
    let body: String = item.body.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = body.chars().collect();
    if chars.len() > 40 {
        format!("{}…", chars[..40].iter().collect::<String>())
    } else {
        body
    }
}

fn status_str(status: &WorkStatus) -> &'static str {
    match status {
        WorkStatus::Todo => "todo",
        WorkStatus::InProgress => "in_progress",
        WorkStatus::Review => "review",
        WorkStatus::Done => "done",
    }
}

/// 루트가 직접·간접으로 기다리는 업무를 진행 순서대로. `autoRun.ts`의 `planItems`와 같은 순서다.
fn plan_items<'a>(root: &WorkItem, items: &'a [WorkItem]) -> Vec<&'a WorkItem> {
    let by_id: HashMap<&str, &WorkItem> = items.iter().map(|i| (i.id.as_str(), i)).collect();
    let mut members: HashMap<&str, &WorkItem> = HashMap::new();
    let mut stack: Vec<&str> = root.depends_on.iter().map(String::as_str).collect();
    while let Some(id) = stack.pop() {
        let Some(item) = by_id.get(id) else { continue };
        if id == root.id || members.contains_key(id) {
            continue;
        }
        members.insert(id, item);
        stack.extend(item.depends_on.iter().map(String::as_str));
    }
    let rank = |i: &WorkItem| (if i.status == WorkStatus::Done { 0.0 } else { 1e9 }) + i.order;
    let mut placed: HashSet<&str> = HashSet::new();
    let mut ordered = Vec::new();
    let mut rest: Vec<&WorkItem> = members.values().copied().collect();
    while !rest.is_empty() {
        let ready: Vec<&WorkItem> = rest
            .iter()
            .copied()
            .filter(|i| i.depends_on.iter().all(|d| !members.contains_key(d.as_str()) || placed.contains(d.as_str())))
            .collect();
        let pool = if ready.is_empty() { rest.clone() } else { ready };
        let next = pool
            .into_iter()
            .min_by(|a, b| rank(a).partial_cmp(&rank(b)).unwrap_or(std::cmp::Ordering::Equal))
            .expect("non-empty");
        placed.insert(next.id.as_str());
        ordered.push(next);
        rest.retain(|i| !placed.contains(i.id.as_str()));
    }
    ordered
}

/// 플랜 루트(메모 전용)는 선행 작업만, 아니면 루트까지 단계다. `runSteps`와 같다.
fn run_steps<'a>(root: &'a WorkItem, items: &'a [WorkItem]) -> Vec<&'a WorkItem> {
    let mut plan = plan_items(root, items);
    if !root.plan && !plan.is_empty() {
        plan.push(root);
    }
    plan
}

/// 현재 단계를 보낸 뒤 CLI가 재개한 횟수. 진행 기록에서 센다(렌더러도 같은 방식으로 막는다).
pub fn ctl_resumes_since_send(run: &Value) -> usize {
    let Some(log) = run.get("log").and_then(Value::as_array) else { return 0 };
    log.iter()
        .rev()
        .take_while(|e| !matches!(e.get("kind").and_then(Value::as_str), Some("send" | "next")))
        .filter(|e| e.get("kind").and_then(Value::as_str) == Some("resume") && e.get("text").and_then(Value::as_str) == Some("ctl"))
        .count()
}

/// 멈춘 이유에 맞는 다음 동작. 외부 에이전트는 이 목록에서 고른다.
fn actions_for(reason: &str, resumes: usize) -> Vec<&'static str> {
    let mut actions: Vec<&'static str> = match reason {
        "judge-stop" => vec!["answer", "resume", "escalate", "end"],
        "followup-limit" => vec!["split", "send", "resume", "escalate", "end"],
        "judge-failed" | "send-failed" => vec!["resume", "escalate", "end"],
        "session-ended" => vec!["create", "resume", "escalate", "end"],
        "app-restarted" | "ctl-pause" => vec!["resume", "end"],
        "task-changed" | "task-busy" => vec!["edit", "resume", "escalate", "end"],
        "permission-timeout" => vec!["focus", "resume", "escalate", "end"],
        "usage-wait" => vec!["wait", "resume"],
        // 사람의 입력·결정이다. 외부 에이전트는 기다리기만 한다.
        "human-input" => vec!["escalate", "wait"],
        "user" | "escalated" => vec!["wait"],
        _ => vec!["escalate", "end"],
    };
    if resumes >= MAX_CTL_RESUMES && reason != "usage-wait" && actions.contains(&"resume") {
        actions.retain(|a| *a != "resume");
        if !actions.contains(&"escalate") {
            actions.insert(0, "escalate");
        }
    }
    actions
}

/// 루트 하나의 자동 진행 상태. `turn`은 세션 id로 턴 관측을, `report`는 태스크 id로 작업자 보고를 준다.
pub fn build_status(
    items: &[WorkItem],
    root_id: &str,
    turn: &dyn Fn(&str) -> Option<TurnObservation>,
    report: &dyn Fn(&str) -> Option<String>,
) -> Result<Value, String> {
    let root = items.iter().find(|i| i.id == root_id).ok_or("auto-run-no-plan")?;
    let steps: Vec<Value> = run_steps(root, items)
        .into_iter()
        .map(|s| {
            let mut v = json!({ "id": s.id, "title": title(s), "status": status_str(&s.status) });
            if let Some(r) = report(&s.id) {
                v["report"] = Value::String(r);
            }
            v
        })
        .collect();
    let mut out = json!({
        "rootId": root.id,
        "title": title(root),
        "plan": root.plan,
        "rootStatus": status_str(&root.status),
        "steps": steps,
    });
    let Some(run) = root.auto_run.as_ref().filter(|r| r.is_object()) else {
        out["state"] = json!("none");
        return Ok(with_rev(out));
    };
    let run_state = run.get("state").and_then(Value::as_str).unwrap_or("paused");
    let reason = run.get("pauseReason").and_then(Value::as_str);
    let current_id = run.get("currentId").and_then(Value::as_str);
    let current = current_id.and_then(|id| items.iter().find(|i| i.id == id));
    let observation = current
        .and_then(|c| c.session_ids.last())
        .and_then(|sid| turn(sid));
    let resumes = ctl_resumes_since_send(run);

    let (state, attention) = match (run_state, reason) {
        ("finished", _) => ("finished", None),
        ("paused", Some("usage-wait")) => ("waiting-usage", Some(("usage-wait", None))),
        ("paused", r) => ("paused", Some((r.unwrap_or("user"), run.get("pauseDetail").and_then(Value::as_str)))),
        _ => match observation.as_ref() {
            Some(o) if o.phase == TurnPhase::NeedsHuman => ("needs-human", Some(("needs-human", o.message.as_deref()))),
            _ => ("running", None),
        },
    };
    out["state"] = json!(state);
    if let Some((reason, detail)) = attention {
        let actions = if reason == "needs-human" { vec!["focus", "escalate", "wait"] } else { actions_for(reason, resumes) };
        let mut a = json!({ "reason": reason, "actions": actions, "ctlResumes": resumes });
        if let Some(d) = detail.filter(|d| !d.trim().is_empty()) {
            a["detail"] = json!(d);
        }
        out["attention"] = a;
    }
    if let Some(c) = current {
        let mut v = json!({
            "taskId": c.id,
            "title": title(c),
            "status": status_str(&c.status),
            "followUps": run.get("followUps").cloned().unwrap_or(json!(0)),
        });
        if let Some(sent) = run.get("sentAt") {
            v["sentAt"] = sent.clone();
        }
        if let Some(sid) = c.session_ids.last() {
            v["sessionId"] = json!(sid);
        }
        if let Some(o) = observation {
            v["turn"] = serde_json::to_value(o).unwrap_or(Value::Null);
        }
        out["current"] = v;
    }
    for (from, to) in [("costUsd", "judgeCostUsd"), ("startedAt", "startedAt"), ("usageWait", "usageWait")] {
        if let Some(v) = run.get(from) {
            out[to] = v.clone();
        }
    }
    if let Some(log) = run.get("log").and_then(Value::as_array) {
        out["viaCtl"] = json!(log.iter().any(|e| e.get("kind").and_then(Value::as_str) == Some("start") && e.get("text").and_then(Value::as_str) == Some("ctl")));
        out["log"] = json!(log[log.len().saturating_sub(LOG_TAIL)..]);
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

/// 기다림을 끝낼 상태: 끝났거나 누군가 손을 써야 한다.
pub fn is_settled(status: &Value) -> bool {
    matches!(status.get("state").and_then(Value::as_str), Some("finished" | "paused" | "needs-human" | "none"))
}

// ── 렌더러 다리 ───────────────────────────────────────────────────────

type Pending = oneshot::Sender<Result<Value, String>>;

/// 쓰기 요청을 렌더러로 보내고 답을 기다린다. 렌더러는 `autorun_control_reply`로 답한다.
pub struct AutoRunBridge {
    pending: Mutex<HashMap<String, Pending>>,
    emit: Box<dyn Fn(&Value) -> Result<(), String> + Send + Sync>,
}

impl AutoRunBridge {
    pub fn new(emit: impl Fn(&Value) -> Result<(), String> + Send + Sync + 'static) -> Self {
        Self { pending: Mutex::new(HashMap::new()), emit: Box::new(emit) }
    }

    pub async fn request(&self, op: &str, root_id: &str, options: Value) -> Result<Value, String> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(request_id.clone(), tx);
        let event = json!({ "requestId": request_id, "op": op, "rootId": root_id, "options": options });
        if let Err(e) = (self.emit)(&event) {
            self.pending.lock().unwrap().remove(&request_id);
            return Err(format!("renderer-unavailable: {e}"));
        }
        match tokio::time::timeout(BRIDGE_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            _ => {
                self.pending.lock().unwrap().remove(&request_id);
                Err("renderer-unavailable".into())
            }
        }
    }

    /// 렌더러의 답. 모르는(시간이 지난) 요청이면 false.
    pub fn reply(&self, request_id: &str, result: Result<Value, String>) -> bool {
        match self.pending.lock().unwrap().remove(request_id) {
            Some(tx) => tx.send(result).is_ok(),
            None => false,
        }
    }
}

// ── 라우트 ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StatusParams {
    root_id: String,
    #[serde(default)]
    wait_secs: Option<u64>,
    #[serde(default)]
    since: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ControlParams {
    root_id: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    usage_guard: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StateParams {
    agent_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AskParams {
    agent_id: String,
    prompt: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    cli: Option<String>,
}

fn auto_run_allowed(ctx: &ControlContext) -> bool {
    ctx.settings.read().unwrap().cli_auto_run_enabled
}

fn current_status(ctx: &ControlContext, root_id: &str) -> Result<Value, String> {
    let items = ctx.work_panels.load_work_items()?;
    let inbox = ctx.app_data_dir.join("work-inbox");
    let now = std::time::SystemTime::now();
    build_status(
        &items,
        root_id,
        &|sid| ctx.turns.get(sid),
        &|task_id| crate::auto_run::read_report(&inbox, task_id, 0, now),
    )
}

pub(super) async fn status(State(ctx): State<Arc<ControlContext>>, Json(p): Json<StatusParams>) -> Json<Value> {
    let wait = Duration::from_secs(p.wait_secs.unwrap_or(0).min(MAX_WAIT_SECS));
    let deadline = Instant::now() + wait;
    let mut turns = ctx.turns.subscribe();
    loop {
        let status = match current_status(&ctx, &p.root_id) {
            Ok(s) => s,
            Err(e) => return fail(e),
        };
        let changed = p.since.as_deref().is_some_and(|since| status["rev"].as_str() != Some(since));
        let done = wait.is_zero() || changed || (p.since.is_none() && is_settled(&status));
        let now = Instant::now();
        if done || now >= deadline {
            let mut status = status;
            if !wait.is_zero() {
                status["timedOut"] = json!(!(changed || (p.since.is_none() && is_settled(&status))));
            }
            return ok(status);
        }
        // 업무 저장은 파일로만 보이므로 짧게 다시 읽는다. 턴 변화는 바로 깨운다.
        let nap = (deadline - now).min(Duration::from_millis(500));
        let _ = tokio::time::timeout(nap, turns.changed()).await;
    }
}

async fn forward(ctx: &ControlContext, op: &str, p: ControlParams) -> Json<Value> {
    let options = json!({ "message": p.message, "usageGuard": p.usage_guard });
    match ctx.autorun.request(op, &p.root_id, options).await {
        Ok(_) => match current_status(ctx, &p.root_id) {
            Ok(s) => ok(s),
            Err(_) => ok(Value::Null),
        },
        Err(e) => fail(e),
    }
}

macro_rules! gated_op {
    ($name:ident, $op:literal) => {
        pub(super) async fn $name(State(ctx): State<Arc<ControlContext>>, Json(p): Json<ControlParams>) -> Json<Value> {
            if !auto_run_allowed(&ctx) {
                return fail("cli-auto-run-disabled");
            }
            forward(&ctx, $op, p).await
        }
    };
}
gated_op!(start, "start");
gated_op!(resume, "resume");
gated_op!(pause, "pause");
gated_op!(end, "end");

/// 사람에게 넘긴다. 위험을 줄이는 쪽이라 자동 진행 허용 없이도 연다.
pub(super) async fn escalate(State(ctx): State<Arc<ControlContext>>, Json(p): Json<ControlParams>) -> Json<Value> {
    if p.message.as_deref().map(str::trim).unwrap_or("").is_empty() {
        return fail("escalate: 사람에게 전할 메시지가 필요합니다");
    }
    forward(&ctx, "escalate", p).await
}

fn turn_state(ctx: &ControlContext, agent_id: &str) -> Value {
    let running = ctx.manager.is_running(agent_id);
    let session_id = ctx.manager.session_id_for(agent_id);
    let observation = session_id.as_deref().and_then(|sid| ctx.turns.get(sid));
    let state = match (&observation, running) {
        (_, false) if session_id.is_none() => "none",
        (_, false) => "ended",
        (Some(o), true) => match o.phase {
            TurnPhase::Working => "working",
            TurnPhase::NeedsHuman => "needs-human",
            TurnPhase::Idle => "idle",
            TurnPhase::Ended => "ended",
        },
        (None, true) => "idle",
    };
    let mut v = json!({ "agentId": agent_id, "state": state, "running": running });
    if let Some(sid) = session_id {
        v["sessionId"] = json!(sid);
    }
    if let Some(o) = observation {
        v["at"] = json!(o.at);
        if let Some(m) = o.message.filter(|_| o.phase == TurnPhase::NeedsHuman) {
            v["message"] = json!(m);
        }
    }
    v
}

pub(super) async fn state(State(ctx): State<Arc<ControlContext>>, Json(p): Json<StateParams>) -> Json<Value> {
    ok(turn_state(&ctx, &p.agent_id))
}

/// 한 번 묻고 턴이 끝나면 마지막 응답을 돌려준다. 사람이 답해야 하면 그 상태로 바로 돌려준다.
pub(super) async fn ask(State(ctx): State<Arc<ControlContext>>, Json(p): Json<AskParams>) -> Json<Value> {
    if !auto_run_allowed(&ctx) {
        return fail("cli-auto-run-disabled");
    }
    if p.prompt.trim().is_empty() {
        return fail("ask: 프롬프트가 필요합니다");
    }
    let Some(session_id) = ctx.manager.session_id_for(&p.agent_id).filter(|_| ctx.manager.is_running(&p.agent_id)) else {
        return fail("not-running");
    };
    let mut turns = ctx.turns.subscribe();
    let submitted_at = match ctx
        .work_send
        .follow_up(&ctx.gate, WorkFollowUpRequest { agent_id: p.agent_id.clone(), session_id: session_id.clone(), text: p.prompt.clone() })
        .await
    {
        Ok(r) => r.submitted_at,
        Err(e) => return fail(e),
    };
    let deadline = Instant::now() + Duration::from_secs(p.timeout_secs.unwrap_or(600).clamp(1, MAX_ASK_SECS));
    loop {
        if let Some(o) = ctx.turns.get(&session_id).filter(|o| o.at >= submitted_at) {
            match o.phase {
                TurnPhase::Idle => break,
                TurnPhase::Ended => return fail("session-ended"),
                TurnPhase::NeedsHuman => {
                    return ok(json!({ "agentId": p.agent_id, "sessionId": session_id, "state": "needs-human", "message": o.message, "submittedAt": submitted_at }))
                }
                TurnPhase::Working => {}
            }
        }
        let now = Instant::now();
        if now >= deadline {
            return ok(json!({ "agentId": p.agent_id, "sessionId": session_id, "state": "working", "timedOut": true, "submittedAt": submitted_at }));
        }
        let _ = tokio::time::timeout((deadline - now).min(Duration::from_secs(1)), turns.changed()).await;
    }
    let cli = p.cli.clone().unwrap_or_else(|| default_cli(&ctx, &p.agent_id));
    let cwd = ctx.manager.cwd_of(&p.agent_id).unwrap_or_default();
    let lookup = ctx.agent_session_lookup.clone();
    let agent_id = p.agent_id.clone();
    let reply = tauri::async_runtime::spawn_blocking(move || {
        use crate::session_log::agent_transcript::{claude, codex};
        let mut source = match cli.as_str() {
            "claude" => claude::source(lookup),
            "codex" => codex::source(),
            _ => None,
        }?;
        crate::auto_run::last_reply(source.as_mut(), &agent_id, &cwd)
    })
    .await
    .ok()
    .flatten();
    ok(json!({ "agentId": p.agent_id, "sessionId": session_id, "state": "idle", "reply": reply, "submittedAt": submitted_at }))
}

/// 프로필의 기본 CLI. 없으면 claude.
fn default_cli(ctx: &ControlContext, agent_id: &str) -> String {
    ctx.store
        .load()
        .agents
        .into_iter()
        .find(|a| a.id == agent_id)
        .and_then(|a| a.default_agent)
        .map(|w| serde_json::to_value(w.cli).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "claude".into()))
        .unwrap_or_else(|| "claude".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, status: WorkStatus, order: f64, after: &[&str]) -> WorkItem {
        WorkItem {
            id: id.into(),
            title: format!("{id} title"),
            body: String::new(),
            assignee_agent_id: Some("a1".into()),
            agent: None,
            status,
            order,
            session_ids: vec![format!("s-{id}")],
            last_error: None,
            depends_on: after.iter().map(|s| s.to_string()).collect(),
            plan: false,
            auto_run: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn plan(run: Option<Value>) -> Vec<WorkItem> {
        let mut root = item("root", WorkStatus::Todo, 9.0, &["b"]);
        root.plan = true;
        root.auto_run = run;
        vec![item("a", WorkStatus::Done, 1.0, &[]), item("b", WorkStatus::InProgress, 0.5, &["a"]), root]
    }

    fn none_turn(_: &str) -> Option<TurnObservation> {
        None
    }
    fn no_report(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn status_without_run_lists_steps_in_dependency_order_with_reports() {
        let items = plan(None);
        let status = build_status(&items, "root", &none_turn, &|id| (id == "a").then(|| "status: done".to_string())).unwrap();
        assert_eq!(status["state"], "none");
        let steps = status["steps"].as_array().unwrap();
        assert_eq!(steps.iter().map(|s| s["id"].as_str().unwrap()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(steps[0]["report"], "status: done");
        assert!(steps[1].get("report").is_none());
        assert!(is_settled(&status));
        assert_eq!(build_status(&items, "nope", &none_turn, &no_report).unwrap_err(), "auto-run-no-plan");
    }

    #[test]
    fn non_plan_root_runs_last() {
        let mut items = plan(None);
        items[2].plan = false;
        let status = build_status(&items, "root", &none_turn, &no_report).unwrap();
        let ids: Vec<_> = status["steps"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect();
        assert_eq!(ids, ["a", "b", "root"]);
    }

    #[test]
    fn running_step_waiting_for_permission_reads_as_needs_human() {
        let items = plan(Some(json!({ "state": "running", "currentId": "b", "followUps": 0, "costUsd": 0.1, "log": [{ "at": 1, "kind": "start", "text": "ctl" }] })));
        let running = build_status(&items, "root", &none_turn, &no_report).unwrap();
        assert_eq!(running["state"], "running");
        assert!(!is_settled(&running));
        assert_eq!(running["viaCtl"], true);
        let waiting = build_status(
            &items,
            "root",
            &|sid| (sid == "s-b").then(|| TurnObservation { phase: TurnPhase::NeedsHuman, at: 5, agent_id: "a1".into(), message: Some("Bash: rm -rf build".into()) }),
            &no_report,
        )
        .unwrap();
        assert_eq!(waiting["state"], "needs-human");
        assert_eq!(waiting["attention"]["detail"], "Bash: rm -rf build");
        assert_eq!(waiting["current"]["turn"]["phase"], "needs-human");
        assert!(is_settled(&waiting));
        assert_ne!(running["rev"], waiting["rev"]);
    }

    #[test]
    fn paused_reason_maps_to_actions_and_ctl_resume_limit_forces_escalation() {
        let log = |resumes: usize| {
            let mut log = vec![json!({ "at": 1, "kind": "send", "taskId": "b" })];
            for _ in 0..resumes {
                log.push(json!({ "at": 2, "kind": "pause", "text": "judge-failed" }));
                log.push(json!({ "at": 3, "kind": "resume", "text": "ctl" }));
            }
            log.push(json!({ "at": 4, "kind": "pause", "text": "judge-failed" }));
            json!({ "state": "paused", "pauseReason": "judge-failed", "pauseDetail": "timeout", "currentId": "b", "followUps": 0, "costUsd": 0, "log": log })
        };
        let fresh = build_status(&plan(Some(log(0))), "root", &none_turn, &no_report).unwrap();
        assert_eq!(fresh["state"], "paused");
        assert_eq!(fresh["attention"]["reason"], "judge-failed");
        assert_eq!(fresh["attention"]["detail"], "timeout");
        assert_eq!(fresh["attention"]["actions"], json!(["resume", "escalate", "end"]));
        let worn = build_status(&plan(Some(log(MAX_CTL_RESUMES))), "root", &none_turn, &no_report).unwrap();
        assert_eq!(worn["attention"]["actions"], json!(["escalate", "end"]));
        assert_eq!(worn["attention"]["ctlResumes"], MAX_CTL_RESUMES);
    }

    #[test]
    fn human_pauses_offer_no_self_recovery_and_usage_wait_keeps_running() {
        let by_ctl = build_status(&plan(Some(json!({ "state": "paused", "pauseReason": "ctl-pause", "followUps": 0, "costUsd": 0, "log": [] }))), "root", &none_turn, &no_report).unwrap();
        assert_eq!(by_ctl["attention"]["actions"], json!(["resume", "end"]));
        for reason in ["user", "escalated"] {
            let s = build_status(&plan(Some(json!({ "state": "paused", "pauseReason": reason, "followUps": 0, "costUsd": 0, "log": [] }))), "root", &none_turn, &no_report).unwrap();
            assert_eq!(s["attention"]["actions"], json!(["wait"]));
        }
        let usage = build_status(&plan(Some(json!({ "state": "paused", "pauseReason": "usage-wait", "usageWait": { "until": 99 }, "followUps": 0, "costUsd": 0, "log": [] }))), "root", &none_turn, &no_report).unwrap();
        assert_eq!(usage["state"], "waiting-usage");
        assert_eq!(usage["usageWait"]["until"], 99);
        assert!(!is_settled(&usage));
    }

    #[test]
    fn ctl_resume_count_resets_on_next_step() {
        let run = json!({ "log": [
            { "kind": "resume", "text": "ctl" },
            { "kind": "next", "taskId": "a" },
            { "kind": "resume", "text": "ctl" },
            { "kind": "resume" },
        ] });
        assert_eq!(ctl_resumes_since_send(&run), 1);
    }

    #[tokio::test]
    async fn bridge_relays_reply_and_times_out_without_one() {
        let sent: Arc<Mutex<Vec<Value>>> = Arc::default();
        let bridge = Arc::new(AutoRunBridge::new({
            let sent = sent.clone();
            move |v: &Value| {
                sent.lock().unwrap().push(v.clone());
                Ok(())
            }
        }));
        let replier = {
            let bridge = bridge.clone();
            let sent = sent.clone();
            tokio::spawn(async move {
                loop {
                    let id = sent.lock().unwrap().first().map(|v| v["requestId"].as_str().unwrap().to_string());
                    if let Some(id) = id {
                        assert!(bridge.reply(&id, Err("auto-run-paused-by-human".into())));
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
        };
        let result = bridge.request("resume", "root", json!({})).await;
        replier.await.unwrap();
        assert_eq!(result.unwrap_err(), "auto-run-paused-by-human");
        assert_eq!(sent.lock().unwrap()[0]["op"], "resume");
        assert!(!bridge.reply("unknown", Ok(Value::Null)));

        let failing = AutoRunBridge::new(|_: &Value| Err("no window".into()));
        assert!(failing.request("start", "root", json!({})).await.unwrap_err().starts_with("renderer-unavailable"));
    }
}
