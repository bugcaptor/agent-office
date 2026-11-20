// src-tauri/src/webdash/routes.rs
//
// 웹 상태판의 HTTP 라우트. 읽기는 백엔드가 가진 것(프로필·세션·턴 상태·알림·
// 전사·사용량·업무·토의 저장 파일)에서 바로 답하고, 렌더러가 주인인 쓰기는
// `bridge`로 넘긴다. 응답 봉투는 `ctl`과 같은 `{ok,data}`/`{ok:false,error}`다.
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{ConnectInfo, Json, Path, Query, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::control::ControlContext;
use crate::httpapi::{fail, ok, session_state_str};
use crate::types::{AgentProfile, WorkAgent, WorkCli};

use super::WebDashState;

const PAGE: &str = include_str!("../../web/dash.html");
/// 업무 본문·토의 발언을 웹으로 보낼 때의 글자 상한.
const BODY_CHARS: usize = 4000;
/// 끝난 업무는 최근 것만 보인다.
const DONE_TASKS: usize = 20;
/// 토의 속기록은 최근 발언만 보인다.
const TRANSCRIPT_ITEMS: usize = 200;
/// 웹에서 받는 글(업무 본문·토의 답)의 바이트 상한.
const MAX_INPUT_BYTES: usize = 32 * 1024;

pub(super) struct Dash {
    pub ctl: Arc<ControlContext>,
    pub dash: Arc<WebDashState>,
}

type Ctx = State<Arc<Dash>>;

pub(super) async fn serve(
    ip: Ipv4Addr,
    port: u16,
    ctl: Arc<ControlContext>,
    dash: Arc<WebDashState>,
) -> std::io::Result<oneshot::Sender<()>> {
    let listener = tokio::net::TcpListener::bind((ip, port)).await?;
    let app = router(Arc::new(Dash { ctl, dash }));
    let (tx, rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(async move {
                let _ = rx.await;
            })
            .await;
    });
    Ok(tx)
}

fn router(ctx: Arc<Dash>) -> Router {
    let api = Router::new()
        .route("/state", get(state))
        .route("/log/:agent", get(log))
        .route("/portrait/:agent", get(portrait))
        .route("/usage", get(usage))
        .route("/tasks", get(tasks).post(create_task))
        .route("/tasks/:id/send", post(send_task))
        .route("/tasks/:id/autorun", post(autorun_task))
        .route("/discussions", get(discussions))
        .route("/discussions/:id", get(discussion))
        .route("/discussions/:id/start", post(start_discussion))
        .route("/discussions/:id/answer", post(answer_discussion))
        .route("/agents/:agent/:op", post(agent_command))
        .layer(axum::middleware::from_fn_with_state(ctx.clone(), auth));
    Router::new()
        .route("/", get(page))
        .nest("/api", api)
        .with_state(ctx)
}

// ── 인증 ─────────────────────────────────────────────────────────────

fn presented_token(req: &Request) -> Option<String> {
    if let Some(v) = req.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(t) = v.strip_prefix("Bearer ") {
            return Some(t.trim().to_string());
        }
    }
    // <img>는 헤더를 못 단다 — 초상 요청만 쿼리 토큰을 쓴다.
    req.uri().query().and_then(|q| {
        q.split('&').find_map(|kv| kv.strip_prefix("t=").map(str::to_string))
    })
}

async fn auth(State(ctx): Ctx, req: Request, next: Next) -> Response {
    let token = presented_token(&req);
    if !ctx.dash.authorized(token.as_deref()) {
        return (StatusCode::UNAUTHORIZED, fail("unauthorized")).into_response();
    }
    if let Some(ConnectInfo(addr)) = req.extensions().get::<ConnectInfo<SocketAddr>>() {
        ctx.dash.note_client(addr.ip());
    }
    let mut res = next.run(req).await;
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

async fn page() -> Response {
    let mut res = Response::new(Body::from(PAGE));
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'self'; img-src 'self' data:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'"),
    );
    res
}

// ── 읽기 ─────────────────────────────────────────────────────────────

fn profile(ctx: &Dash, agent_id: &str) -> Option<AgentProfile> {
    ctx.ctl.store.load().agents.into_iter().find(|a| a.id == agent_id)
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max).collect();
    s.push('…');
    s
}

async fn state(State(ctx): Ctx) -> Json<Value> {
    let profiles = ctx.ctl.store.load();
    let mut live = std::collections::HashMap::new();
    for (sid, agent, st) in ctx.ctl.registry.snapshot() {
        live.insert(agent, (sid, st));
    }
    let characters: Vec<Value> = profiles
        .agents
        .iter()
        .map(|a| {
            let session = live.get(&a.id);
            let turn = session.and_then(|(sid, _)| ctx.ctl.turns.get(sid));
            let waiting: Vec<Value> = ctx
                .ctl
                .manager
                .pending_notifications(&a.id)
                .into_iter()
                .map(|n| json!({ "source": n.source, "message": n.message, "at": n.at }))
                .collect();
            json!({
                "agentId": a.id,
                "name": a.name,
                "role": a.role,
                "cwd": a.cwd,
                "clockedOut": a.clocked_out,
                "portrait": a.portrait_updated_at,
                "defaultCli": a.default_agent.as_ref().map(|w| w.cli.clone()),
                "session": session.map(|(sid, st)| json!({ "sessionId": sid, "state": session_state_str(*st) })),
                "turn": turn,
                "waiting": waiting,
                "label": ctx.dash.label(&a.id),
            })
        })
        .collect();
    let body = json!({ "app": env!("CARGO_PKG_VERSION"), "characters": characters });
    ok(with_rev(body))
}

/// 내용이 같으면 같은 지문. 브라우저가 바뀐 것만 다시 그린다.
fn with_rev(mut v: Value) -> Value {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.to_string().hash(&mut h);
    v["rev"] = json!(format!("{:016x}", h.finish()));
    v
}

#[derive(Deserialize)]
struct LogQuery {
    after: Option<u64>,
    epoch: Option<u64>,
}

async fn log(State(ctx): Ctx, Path(agent): Path<String>, Query(q): Query<LogQuery>) -> Json<Value> {
    let Some(p) = profile(&ctx, &agent) else {
        return fail("unknown-agent");
    };
    // Codex 전사는 작업 폴더로 찾는다. 세션이 cd 했으면 그 폴더가 맞다.
    let cwd = ctx
        .ctl
        .gate
        .sink()
        .cwd_of(&agent)
        .or(p.cwd)
        .or_else(|| dirs_home().map(|h| h.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let dash = ctx.dash.clone();
    match tokio::task::spawn_blocking(move || dash.logs.read(&agent, &cwd, q.after, q.epoch)).await {
        Ok(page) => ok(page),
        Err(e) => fail(e.to_string()),
    }
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(Into::into)
}

async fn portrait(State(ctx): Ctx, Path(agent): Path<String>) -> Response {
    let Ok(Some(encoded)) = ctx.ctl.portrait_store.load(&agent) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut res = Response::new(Body::from(bytes));
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    res
}

async fn usage(State(ctx): Ctx) -> Json<Value> {
    let openrouter = ctx.ctl.key_store.openrouter_key();
    let opencode = ctx.ctl.key_store.opencode_key();
    let keys = crate::usage::ApiKeyRequest {
        openrouter: openrouter.as_deref(),
        opencode: opencode.as_deref(),
        force: false,
    };
    let snapshot = crate::ipc::commands::load_usage_snapshot_body(
        &ctx.ctl.live_usage,
        keys,
        chrono::Utc::now().timestamp_millis(),
    )
    .await;
    ok(snapshot)
}

async fn tasks(State(ctx): Ctx) -> Json<Value> {
    let items = match ctx.ctl.work_panels.load_work_items() {
        Ok(items) => items,
        Err(e) => return fail(e),
    };
    let names: std::collections::HashMap<String, String> =
        ctx.ctl.store.load().agents.into_iter().map(|a| (a.id, a.name)).collect();
    let mut open: Vec<_> = items.iter().filter(|i| !is_done(i)).collect();
    open.sort_by(|a, b| a.order.partial_cmp(&b.order).unwrap_or(std::cmp::Ordering::Equal));
    let mut done: Vec<_> = items.iter().filter(|i| is_done(i)).collect();
    done.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    done.truncate(DONE_TASKS);
    let row = |i: &crate::types::WorkItem| {
        json!({
            "id": i.id,
            "title": i.title,
            "body": clip(&i.body, BODY_CHARS),
            "status": i.status,
            "assigneeAgentId": i.assignee_agent_id,
            "assigneeName": i.assignee_agent_id.as_ref().and_then(|id| names.get(id)),
            "cli": i.agent.as_ref().map(|a| a.cli.clone()),
            "plan": i.plan,
            "dependsOn": i.depends_on,
            "autoRun": i.auto_run.as_ref().and_then(|a| a.get("state")).cloned(),
            "scheduled": i.schedule.is_some(),
            "lastError": i.last_error,
            "updatedAt": i.updated_at,
        })
    };
    ok(with_rev(json!({
        "open": open.into_iter().map(row).collect::<Vec<_>>(),
        "done": done.into_iter().map(row).collect::<Vec<_>>(),
    })))
}

fn is_done(item: &crate::types::WorkItem) -> bool {
    serde_json::to_value(&item.status).ok().and_then(|v| v.as_str().map(|s| s == "done")).unwrap_or(false)
}

async fn discussions(State(ctx): Ctx) -> Json<Value> {
    match crate::control::discussion::list_rows(&ctx.ctl) {
        Ok(rows) => ok(rows),
        Err(e) => fail(e),
    }
}

async fn discussion(State(ctx): Ctx, Path(id): Path<String>) -> Json<Value> {
    let status = match crate::control::discussion::current_status(&ctx.ctl, &id) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let names: std::collections::HashMap<String, String> =
        ctx.ctl.store.load().agents.into_iter().map(|a| (a.id, a.name)).collect();
    let transcript = crate::control::discussion::transcript(&ctx.ctl, &id).unwrap_or_default();
    let skip = transcript.len().saturating_sub(TRANSCRIPT_ITEMS);
    let rows: Vec<Value> = transcript
        .into_iter()
        .skip(skip)
        .map(|u| {
            json!({
                "at": u.at,
                "round": u.round,
                "kind": u.kind,
                "agentId": u.agent_id,
                "name": u.agent_id.as_ref().and_then(|id| names.get(id)),
                "text": clip(&u.text, BODY_CHARS),
            })
        })
        .collect();
    ok(json!({ "status": status, "transcript": rows }))
}

// ── 쓰기(간접 명령) ───────────────────────────────────────────────────

fn activity(ctx: &Dash, op: &str, agent: Option<&str>, detail: Option<&str>) {
    let name = agent.and_then(|id| profile(ctx, id)).map(|p| p.name);
    ctx.dash.activity(json!({ "op": op, "agentId": agent, "name": name, "detail": detail }));
}

/// 렌더러에 넘기고 답을 기다린다.
async fn bridged(ctx: &Dash, payload: Value) -> Result<Value, String> {
    ctx.dash.bridge.send(payload).await
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AgentCommandBody {
    #[serde(default)]
    cli: Option<String>,
}

async fn agent_command(
    State(ctx): Ctx,
    Path((agent, op)): Path<(String, String)>,
    body: Option<Json<AgentCommandBody>>,
) -> Json<Value> {
    let Some(profile) = profile(&ctx, &agent) else {
        return fail("unknown-agent");
    };
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let result = match op.as_str() {
        "interrupt" | "ctrl-c" => interrupt(&ctx, &agent, &op),
        "launch" => launch(&ctx, &profile, body.cli.as_deref()).await,
        "restart" | "terminate" | "resume" => {
            bridged(&ctx, json!({ "op": op, "agentId": agent })).await.map(|_| ())
        }
        _ => Err("unknown-op".into()),
    };
    match result {
        Ok(()) => {
            activity(&ctx, &op, Some(&agent), body.cli.as_deref());
            ok(Value::Null)
        }
        Err(e) => fail(e),
    }
}

/// 진행 중인 작업 끊기. Esc는 Claude·Codex의 중단 키라 사람 입력 표시를 남기지
/// 않고 그대로 보낸다. Ctrl-C는 사람 입력 경로로 보내 미제출 입력 표시도 지운다.
fn interrupt(ctx: &Dash, agent: &str, op: &str) -> Result<(), String> {
    if !ctx.ctl.manager.is_running(agent) {
        return Err("not-running".into());
    }
    if op == "interrupt" {
        ctx.ctl.gate.sink().write(agent, "\x1b");
    } else {
        ctx.ctl.gate.note_human(agent, "\x03");
    }
    Ok(())
}

/// 셸 프롬프트에 있는 터미널에서 Claude·Codex를 띄운다. 프로필 기본 CLI가 같은
/// 종류면 그 모델·노력 설정을 따른다. CLI가 이미 돌고 있거나 쓰다 만 입력이 있으면 거절한다.
async fn launch(ctx: &Dash, profile: &AgentProfile, cli: Option<&str>) -> Result<(), String> {
    let cli = match cli.unwrap_or("claude") {
        "claude" => WorkCli::Claude,
        "codex" => WorkCli::Codex,
        _ => return Err("unknown-cli".into()),
    };
    let target = ctx.ctl.work_send.inspect(&ctx.ctl.gate, &profile.id)?;
    let Some(session_id) = target.session_id else {
        return Err("not-running".into());
    };
    if target.has_uncommitted {
        return Err("human-input-pending".into());
    }
    if target.restart_required {
        return Err("not-at-shell-prompt".into());
    }
    let shell = ctx.ctl.gate.sink().shell_path_for(&profile.id).unwrap_or_default();
    let syntax = crate::work_send::cli_transition::LaunchShell::for_shell_path(&shell)
        .ok_or("shell-unsupported")?
        .syntax();
    let agent = profile
        .default_agent
        .clone()
        .filter(|a| a.cli == cli)
        .unwrap_or(WorkAgent { cli, model: None, effort: None });
    let command = crate::work_send::validated_command(&agent, syntax)?;
    use crate::session::inject::{InjectSource, SubmitOutcome};
    match ctx.ctl.gate.submit(&profile.id, &session_id, &command, InjectSource::WorkSend).await {
        SubmitOutcome::Submitted => Ok(()),
        SubmitOutcome::Deferred(_) => Err("busy".into()),
        SubmitOutcome::SessionChanged => Err("session-changed".into()),
        SubmitOutcome::NotRunning => Err("not-running".into()),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewTask {
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    assignee_agent_id: Option<String>,
    #[serde(default)]
    cli: Option<String>,
    #[serde(default)]
    send: bool,
}

fn check_text(text: &str) -> Result<(), String> {
    if text.len() > MAX_INPUT_BYTES {
        return Err("too-large".into());
    }
    if text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err("control-char".into());
    }
    Ok(())
}

async fn create_task(State(ctx): Ctx, Json(t): Json<NewTask>) -> Json<Value> {
    let title = t.title.trim();
    if title.is_empty() && t.body.trim().is_empty() {
        return fail("empty");
    }
    if let Err(e) = check_text(title).and_then(|_| check_text(&t.body)) {
        return fail(e);
    }
    if let Some(id) = &t.assignee_agent_id {
        if profile(&ctx, id).is_none() {
            return fail("unknown-agent");
        }
    }
    if t.cli.as_deref().is_some_and(|c| c != "claude" && c != "codex") {
        return fail("unknown-cli");
    }
    let payload = json!({
        "op": "work-create",
        "title": title,
        "body": t.body,
        "assigneeAgentId": t.assignee_agent_id,
        "cli": t.cli,
        "send": t.send,
    });
    match bridged(&ctx, payload).await {
        Ok(v) => {
            activity(&ctx, if t.send { "work-create-send" } else { "work-create" }, t.assignee_agent_id.as_deref(), Some(title));
            ok(v)
        }
        Err(e) => fail(e),
    }
}

async fn send_task(State(ctx): Ctx, Path(id): Path<String>) -> Json<Value> {
    task_op(&ctx, "work-send", &id).await
}

async fn autorun_task(State(ctx): Ctx, Path(id): Path<String>) -> Json<Value> {
    task_op(&ctx, "autorun-start", &id).await
}

async fn task_op(ctx: &Dash, op: &str, id: &str) -> Json<Value> {
    let item = ctx.ctl.work_panels.load_work_items().ok().and_then(|items| items.into_iter().find(|i| i.id == id));
    let Some(item) = item else {
        return fail("task-not-found");
    };
    match bridged(ctx, json!({ "op": op, "itemId": id })).await {
        Ok(v) => {
            activity(ctx, op, item.assignee_agent_id.as_deref(), Some(&item.title));
            ok(v)
        }
        Err(e) => fail(e),
    }
}

async fn start_discussion(State(ctx): Ctx, Path(id): Path<String>) -> Json<Value> {
    match bridged(&ctx, json!({ "op": "discuss-start", "discussionId": id })).await {
        Ok(v) => {
            activity(&ctx, "discuss-start", None, None);
            ok(v)
        }
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
struct Answer {
    text: String,
}

async fn answer_discussion(State(ctx): Ctx, Path(id): Path<String>, Json(a): Json<Answer>) -> Json<Value> {
    if a.text.trim().is_empty() {
        return fail("empty");
    }
    if let Err(e) = check_text(&a.text) {
        return fail(e);
    }
    match bridged(&ctx, json!({ "op": "discuss-answer", "discussionId": id, "text": a.text })).await {
        Ok(v) => {
            activity(&ctx, "discuss-answer", None, None);
            ok(v)
        }
        Err(e) => fail(e),
    }
}
