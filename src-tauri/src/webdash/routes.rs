// src-tauri/src/webdash/routes.rs
//
// 웹 상태판의 HTTP 라우트. 읽기는 백엔드가 가진 것(프로필·세션·턴 상태·알림·
// 전사·사용량·업무·토의 저장 파일)에서 바로 답하고, 렌더러가 주인인 쓰기는
// `bridge`로 넘긴다. 응답 봉투는 `ctl`과 같은 `{ok,data}`/`{ok:false,error}`다.
use std::collections::HashMap;
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
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::chat_queue::check_limits;
use crate::control::ControlContext;
use crate::httpapi::{fail, ok};
use crate::types::{is_office_agent_id, AgentProfile, WorkAgent, WorkCli, WorkStatus, OFFICE_AGENT_ID};

use super::WebDashState;

const PAGE: &str = include_str!("../../web/dash.html");
const STYLE: &str = include_str!("../../web/dash.css");
const APP_JS: &str = include_str!("../../web/dash.js");
const TASKS_JS: &str = include_str!("../../web/tasks.js");
const SHELL_JS: &str = include_str!("../../web/shell.js");
const FILES_JS: &str = include_str!("../../web/files.js");
/// 토의 발언을 웹으로 보낼 때의 글자 상한.
const BODY_CHARS: usize = 4000;
/// 업무 본문 바이트 상한(수신함 `.md` 상한 64 KiB와 맞춤).
const TASK_BODY_MAX_BYTES: usize = 64 * 1024;
/// 끝난 업무는 기본으로 최근 것만 보인다(`?allDone=1`이면 전체).
const DONE_TASKS: usize = 20;
/// 토의 속기록은 최근 발언만 보인다.
const TRANSCRIPT_ITEMS: usize = 200;

pub(super) struct Dash {
    pub ctl: Arc<ControlContext>,
    pub dash: Arc<WebDashState>,
}

type Ctx = State<Arc<Dash>>;

pub(super) async fn serve(ip: Ipv4Addr, port: u16, app: Router) -> std::io::Result<oneshot::Sender<()>> {
    let listener = tokio::net::TcpListener::bind((ip, port)).await?;
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

/// 상태판 앱(라우터). 평문 리스너(`serve`)와 TLS 리스너(`tls::serve`)가 함께 쓴다.
pub(super) fn app(ctl: Arc<ControlContext>, dash: Arc<WebDashState>) -> Router {
    // 컴퓨터 이름은 처음 한 번 프로세스를 띄워 읽는다. 첫 요청이 기다리지 않게 미리 읽어 둔다.
    tokio::task::spawn_blocking(super::computer_name);
    router(Arc::new(Dash { ctl, dash }))
}

fn router(ctx: Arc<Dash>) -> Router {
    let api = Router::new()
        .route("/state", get(state))
        .route("/log/:agent", get(log))
        .route("/portrait/:agent", get(portrait))
        .route("/image/:agent/:id", get(image))
        .route("/file/:agent/:epoch/:seq/:index", get(download_file))
        .route("/asks/:id", post(answer_ask))
        .route("/usage", get(usage))
        .route("/models/:cli", get(models))
        .route("/tasks", get(tasks).post(create_task))
        .route("/tasks/:id/edit", post(edit_task))
        .route("/tasks/:id/delete", post(delete_task))
        .route("/tasks/:id/reorder", post(reorder_task))
        .route("/tasks/:id/send", post(send_task))
        .route("/tasks/:id/autorun", post(autorun_task))
        .route("/tasks/:id/done", post(done_task))
        .route("/tasks/:id/schedule", post(schedule_task))
        .route("/discussions", get(discussions))
        .route("/discussions/:id", get(discussion))
        .route("/discussions/:id/start", post(start_discussion))
        .route("/discussions/:id/answer", post(answer_discussion))
        .route("/agents/:agent/:op", post(agent_command))
        .route("/office-agent", post(office_agent_send))
        .route("/shell/:agent", get(shell_state))
        .route("/shell/:agent/suggest", post(shell_suggest))
        .route("/shell/:agent/run", post(shell_run))
        .route("/office-agent/:op", post(office_agent_command))
        .merge(super::files_routes::routes())
        .layer(axum::middleware::from_fn_with_state(ctx.clone(), auth));
    let link = ctx.dash.link_host().map(|host| {
        let dash = Arc::downgrade(&ctx.dash);
        crate::link::host::router(host, Arc::new(move || dash.upgrade().and_then(|d| d.lan_fingerprint())))
    });
    let router = Router::new()
        .route("/", get(page))
        .route("/dash.css", get(style_asset))
        .route("/dash.js", get(app_js_asset))
        .route("/tasks.js", get(tasks_js_asset))
        .route("/shell.js", get(shell_js_asset))
        .route("/files.js", get(files_js_asset))
        .route("/ca.crt", get(lan_ca))
        .nest("/api", api)
        .with_state(ctx);
    match link {
        Some(link) => router.merge(link),
        None => router,
    }
}

// ── 인증 ─────────────────────────────────────────────────────────────

fn presented_token(req: &Request) -> Option<String> {
    if let Some(v) = req.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(t) = v.strip_prefix("Bearer ") {
            return Some(t.trim().to_string());
        }
    }
    // <img>는 헤더를 못 단다 — 초상과 대화 이미지는 쿼리 토큰을 쓴다.
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

/// LAN 자체 서명 HTTPS의 CA 인증서. 기기에 설치하면 경고 없이 열린다(공개 정보라 토큰을 묻지 않는다).
async fn lan_ca(State(ctx): Ctx) -> Response {
    let Some(pem) = ctx.dash.lan_ca_pem() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut res = Response::new(Body::from(pem));
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/x-x509-ca-cert"));
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_static("attachment; filename=\"agent-office-lan-ca.crt\""));
    res
}

/// 상태판 HTML. 탭 제목과 머리에 처음부터 컴퓨터 이름을 넣어, 상태를 받기 전에도 어느 기기인지 보이게 한다.
async fn page() -> Response {
    let host = tokio::task::spawn_blocking(super::computer_name).await.unwrap_or_default();
    let mut res = Response::new(Body::from(render_page(&host)));
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'"),
    );
    res
}

fn render_page(host: &str) -> String {
    let host = host.trim();
    if host.is_empty() {
        return PAGE.to_string();
    }
    let host = crate::doc_view::escape_html(host);
    PAGE.replacen("<title>Agent Office</title>", &format!("<title>{host} · Agent Office</title>"), 1)
        .replacen(r#"<h1 id="title">Agent Office</h1>"#, &format!(r#"<h1 id="title">{host}</h1>"#), 1)
}

fn static_asset(body: &'static str, content_type: &'static str) -> Response {
    let mut res = Response::new(Body::from(body));
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    res
}

async fn style_asset() -> Response {
    static_asset(STYLE, "text/css; charset=utf-8")
}

async fn app_js_asset() -> Response {
    static_asset(APP_JS, "text/javascript; charset=utf-8")
}

async fn tasks_js_asset() -> Response {
    static_asset(TASKS_JS, "text/javascript; charset=utf-8")
}

async fn shell_js_asset() -> Response {
    static_asset(SHELL_JS, "text/javascript; charset=utf-8")
}

async fn files_js_asset() -> Response {
    static_asset(FILES_JS, "text/javascript; charset=utf-8")
}

// ── 읽기 ─────────────────────────────────────────────────────────────

fn profile(ctx: &Dash, agent_id: &str) -> Option<AgentProfile> {
    ctx.ctl.store.load().agents.into_iter().find(|a| a.id == agent_id)
}

/// 일반 에이전트가 고를 수 있는 작업 폴더: 홈, 그다음 캐릭터들의 작업 폴더(겹치면 한 번).
fn folders(ctx: &Dash) -> Vec<String> {
    let mut out = vec![crate::session::manager::home_dir()];
    for agent in ctx.ctl.store.load().agents {
        let Some(cwd) = agent.cwd.filter(|c| !c.trim().is_empty()) else { continue };
        let cwd = crate::session::manager::expand_tilde(cwd);
        if !out.contains(&cwd) {
            out.push(cwd);
        }
    }
    out
}

/// 캐릭터 id → 이름.
fn agent_names(ctx: &Dash) -> HashMap<String, String> {
    let mut names: HashMap<String, String> = ctx.ctl.store.load().agents.into_iter().map(|a| (a.id, a.name)).collect();
    // 머신 연동 호스트: 다른 머신 캐릭터(토의 참가자·업무 담당)는 `이름 · 머신`.
    if let Some(remote) = ctx.dash.link_remote() {
        for m in remote.office_view().machines {
            for c in m.characters {
                names.entry(c.id.clone()).or_insert_with(|| format!("{} · {}", c.name, m.name));
            }
        }
    }
    names
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max).collect();
    s.push('…');
    s
}

/// 명령줄 → 에이전트 CLI 이름. `claude …`, `node /…/bin/codex …`,
/// `codex-aarch64-apple-darwin`, `/…/.local/bin/agy …`처럼 앞 두 토큰의 실행 파일 이름으로 본다.
pub(crate) fn cli_of_command(command: &str) -> Option<&'static str> {
    for token in command.split_whitespace().take(2) {
        let base = token.rsplit(['/', '\\']).next().unwrap_or(token);
        let base = base.strip_suffix(".exe").unwrap_or(base).to_ascii_lowercase();
        for cli in ["claude", "codex", "agy"] {
            if base == cli || base.starts_with(&format!("{cli}-")) {
                return Some(cli);
            }
        }
    }
    None
}

/// 터미널 포그라운드의 에이전트 CLI(블로킹). 그룹 리더가 CLI가 아니면 리더의 직계 자식을 본다 —
/// 업무 보내기는 CLI를 서브셸 `( … )`에서 띄워 리더가 셸 사본(`/bin/zsh -l -i`)이다.
pub(crate) fn foreground_cli(
    manager: &crate::session::manager::SessionManager,
    agent_id: &str,
    leader: Option<&str>,
) -> Option<&'static str> {
    cli_of_command(leader?)
        .or_else(|| manager.foreground_child_commands(agent_id).iter().find_map(|c| cli_of_command(c)))
}

async fn state(State(ctx): Ctx) -> Json<Value> {
    let view = super::snapshot::local_view(&ctx.ctl, &ctx.dash).await;
    let mut characters = view.characters;
    let mut output = view.output_at;
    let mut machines = Vec::new();
    if let Some(remote) = ctx.dash.link_remote() {
        let merged = remote.web_view();
        characters.extend(merged.characters);
        output.extend(merged.output_at);
        machines = merged.machines;
    }
    let body = json!({
        "app": env!("CARGO_PKG_VERSION"),
        "host": super::computer_name(),
        "characters": characters,
        "officeAgent": view.office_agent,
        "machines": machines,
        "folders": folders(&ctx),
        "home": crate::session::manager::home_dir(),
    });
    // 마지막 출력 시각·서버 시각은 몇 초마다 바뀐다. 지문(`rev`)에 넣지 않아 머리 카드를 괜히 다시 그리지 않는다.
    let mut body = with_rev(body);
    body["outputAt"] = Value::Object(output);
    body["now"] = json!(crate::types::now_ms());
    ok(body)
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
    /// 마지막 이만큼만 돌려준다(목록 미리보기). 없으면 전부.
    tail: Option<usize>,
}

async fn log(State(ctx): Ctx, Path(agent): Path<String>, Query(q): Query<LogQuery>) -> Json<Value> {
    if let Some((remote, device, id)) = remote_of(&ctx, &agent) {
        let ask = json!({ "agentId": id, "after": q.after, "epoch": q.epoch, "tail": q.tail });
        return match remote.request(&device, "log", ask).await {
            Ok(page) => ok(page),
            Err(e) => fail(e),
        };
    }
    let Some(cwd) = super::snapshot::agent_cwd(&ctx.ctl, &agent) else {
        return fail("unknown-agent");
    };
    let dash = ctx.dash.clone();
    match tokio::task::spawn_blocking(move || dash.logs.read(&agent, &cwd, q.after, q.epoch)).await {
        Ok(page) => ok(match q.tail {
            Some(n) => page.keep_last(n),
            None => page,
        }),
        Err(e) => fail(e.to_string()),
    }
}

/// 다른 머신 캐릭터 id(`기기id:캐릭터id`)면 (원격 보기, 기기, 그 머신의 캐릭터 id).
fn remote_of(ctx: &Dash, agent: &str) -> Option<(Arc<crate::link::RemoteView>, String, String)> {
    let remote = ctx.dash.link_remote()?;
    let (device, id) = remote.resolve(agent)?;
    Some((remote, device, id))
}

fn png_response(bytes: Vec<u8>, cacheable: bool) -> Response {
    let mut res = Response::new(Body::from(bytes));
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    if cacheable {
        // 주소에 초상 판(`v=`)이 실려 판이 바뀌면 주소도 바뀐다. 머신을 건너 다시 받지 않게 한다.
        res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=3600"));
    }
    res
}

async fn portrait(State(ctx): Ctx, Path(agent): Path<String>) -> Response {
    if let Some((remote, device, id)) = remote_of(&ctx, &agent) {
        return match remote.portrait(&device, &id).await {
            Some(png) => png_response(png.as_ref().clone(), true),
            None => StatusCode::NOT_FOUND.into_response(),
        };
    }
    let Ok(Some(encoded)) = ctx.ctl.portrait_store.load(&agent) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    png_response(bytes, false)
}

/// 대화 로그에 실린 이미지. `<img>`라 토큰은 쿼리(`t=`)로 온다.
async fn image(State(ctx): Ctx, Path((agent, id)): Path<(String, String)>) -> Response {
    let found = if let Some((remote, device, agent)) = remote_of(&ctx, &agent) {
        remote_image(&remote, &device, &agent, &id, "web").await.map(|(t, b)| (t, Arc::new(b)))
    } else {
        // 로그 수집기 잠금은 파일을 읽는 동안 잡혀 있을 수 있다 — 블로킹 스레드에서 기다린다.
        let dash = ctx.dash.clone();
        tokio::task::spawn_blocking(move || dash.logs.image(&agent, &id)).await.ok().flatten()
    };
    let Some((media_type, bytes)) = found else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(content_type) = HeaderValue::from_str(&media_type) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut res = Response::new(Body::from(bytes.as_ref().clone()));
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, content_type);
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; sandbox"));
    res
}

async fn download_file(
    State(ctx): Ctx,
    Path((agent, epoch, seq, index)): Path<(String, u64, u64, usize)>,
) -> Response {
    let found = if let Some((remote, device, agent)) = remote_of(&ctx, &agent) {
        let ask = json!({ "agentId": agent, "epoch": epoch, "seq": seq, "index": index });
        remote.request(&device, "file", ask).await.ok().and_then(|a| {
            let name = a.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            crate::link::remote::decode_blob(&a).map(|b| (name, b))
        })
    } else {
        let dash = ctx.dash.clone();
        tokio::task::spawn_blocking(move || dash.logs.file(&agent, epoch, seq, index)).await.ok().flatten()
    };
    let Some((_, bytes)) = found else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut res = Response::new(Body::from(bytes));
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_static("attachment"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    res
}

/// 다른 머신 대화 로그의 이미지(미디어 종류, 바이트).
pub(crate) async fn remote_image(
    remote: &crate::link::RemoteView,
    device: &str,
    agent: &str,
    id: &str,
    view: &str,
) -> Option<(String, Vec<u8>)> {
    let answer = remote.request(device, "image", json!({ "agentId": agent, "id": id, "view": view })).await.ok()?;
    let media_type = answer.get("mediaType").and_then(Value::as_str)?.to_string();
    let bytes = crate::link::remote::decode_blob(&answer)?;
    Some((media_type, bytes))
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

async fn models(Path(cli): Path<String>) -> Json<Value> {
    if parse_work_cli(&cli).is_none() {
        return fail("unknown-cli");
    }
    ok(crate::summarizer::list_cli_models(&cli).await)
}

#[derive(Deserialize, Default)]
struct TasksQuery {
    #[serde(rename = "allDone", default)]
    all_done: Option<bool>,
}

async fn tasks(State(ctx): Ctx, Query(q): Query<TasksQuery>) -> Json<Value> {
    let items = match ctx.ctl.work_panels.load_work_items() {
        Ok(items) => items,
        Err(e) => return fail(e),
    };
    let (observer_enabled, session_cost_enabled) = {
        let s = ctx.ctl.settings.read().unwrap();
        (s.observer_enabled, s.session_cost_enabled)
    };
    let names = agent_names(&ctx);
    let mut open: Vec<_> = items.iter().filter(|i| !is_done(i)).collect();
    open.sort_by(|a, b| a.order.partial_cmp(&b.order).unwrap_or(std::cmp::Ordering::Equal));
    let mut done: Vec<_> = items.iter().filter(|i| is_done(i)).collect();
    done.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    let done_total = done.len();
    let all_done = q.all_done.unwrap_or(false);
    if !all_done {
        done.truncate(DONE_TASKS);
    }
    let row = |i: &crate::types::WorkItem| {
        json!({
            "id": i.id,
            "title": i.title,
            "body": i.body,
            "status": i.status,
            "order": i.order,
            "assigneeAgentId": i.assignee_agent_id,
            "assigneeName": i.assignee_agent_id.as_ref().and_then(|id| names.get(id)),
            "agent": i.agent,
            "cli": i.agent.as_ref().map(|a| a.cli.clone()),
            "plan": i.plan,
            "dependsOn": i.depends_on,
            "maxFollowUps": i.max_follow_ups,
            "permissionWaitMinutes": i.permission_wait_minutes,
            "autoRun": i.auto_run,
            "schedule": i.schedule,
            "scheduled": i.schedule.is_some(),
            "scheduledFrom": i.scheduled_from,
            "sessionIds": i.session_ids,
            "lastError": i.last_error,
            "createdAt": i.created_at,
            "updatedAt": i.updated_at,
        })
    };
    ok(with_rev(json!({
        "open": open.into_iter().map(row).collect::<Vec<_>>(),
        "done": done.into_iter().map(row).collect::<Vec<_>>(),
        "doneTotal": done_total,
        "allDone": all_done,
        "observerEnabled": observer_enabled,
        "sessionCostEnabled": session_cost_enabled,
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
    let names = agent_names(&ctx);
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

pub(super) fn activity(ctx: &Dash, op: &str, agent: Option<&str>, detail: Option<&str>) {
    // 명령이 터미널 상태(CLI 실행·종료)를 바꿨을 수 있다 — 다음 `/state`가 다시 보게 한다.
    ctx.dash.probes.clear();
    let name = agent.and_then(|id| profile(ctx, id)).map(|p| p.name);
    ctx.dash.activity(json!({ "op": op, "agentId": agent, "name": name, "detail": detail }));
}

/// 렌더러에 넘기고 답을 기다린다.
async fn bridged(ctx: &Dash, payload: Value) -> Result<Value, String> {
    ctx.dash.bridge.send(payload).await
}

#[derive(Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentCommandBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli: Option<String>,
    /// `message`: 보낼 글.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// `queue-cancel`: 뺄 대기열 메시지.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// 앱 화면에 웹 조작 알림을 띄우지 않는다(에이전트끼리 대화 `crate::peer` — 세션 안에만 남긴다).
    #[serde(skip)]
    pub quiet: bool,
}

/// 원격 조작이 오래 걸릴 수 있는 것(CLI를 띄워 첫 프롬프트를 넣는 데 수십 초가 걸린다)까지 기다릴 시간.
pub(crate) const REMOTE_OP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

async fn agent_command(
    State(ctx): Ctx,
    Path((agent, op)): Path<(String, String)>,
    body: Option<Json<AgentCommandBody>>,
) -> Json<Value> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    // 다른 머신 캐릭터는 그 머신에 넘긴다(그 머신의 웹 상태판 조작과 같은 일을 한다).
    if let Some((remote, device, id)) = remote_of(&ctx, &agent) {
        let mut ask = serde_json::to_value(&body).unwrap_or_else(|_| json!({}));
        ask["agentId"] = json!(id);
        ask["op"] = json!(op);
        return match remote.request_with(&device, "op", ask, REMOTE_OP_TIMEOUT).await {
            Ok(v) => ok(v),
            Err(e) => fail(e),
        };
    }
    match run_agent_op(&ctx, &agent, &op, body).await {
        Ok(v) => ok(v),
        Err(e) => fail(e),
    }
}

/// 이 머신 캐릭터 하나에 웹 상태판 조작을 한다(`/api/agents/:agent/:op`, 호스트가 넘긴 원격 조작도 여기로 온다).
pub(crate) async fn agent_op(
    ctl: Arc<ControlContext>,
    dash: Arc<WebDashState>,
    agent: &str,
    op: &str,
    body: AgentCommandBody,
) -> Result<Value, String> {
    run_agent_op(&Dash { ctl, dash }, agent, op, body).await
}

async fn run_agent_op(ctx: &Dash, agent: &str, op: &str, body: AgentCommandBody) -> Result<Value, String> {
    let profile = profile(ctx, agent).ok_or("unknown-agent")?;
    let quiet = body.quiet;
    let activity = |ctx: &Dash, op: &str, agent: Option<&str>, detail: Option<&str>| {
        if !quiet {
            activity(ctx, op, agent, detail);
        }
    };
    match op {
        "message" => {
            let text = body.text.as_deref().unwrap_or("");
            // 셸 프롬프트만 있는 터미널(새 세션 등)이면 업무 보내기처럼 CLI를 띄워 첫 프롬프트로 넣는다.
            if let Some(result) = launch_with_prompt(ctx, &profile, body.cli.as_deref(), text).await {
                result?;
                activity(ctx, "message", Some(agent), Some(&first_line(text)));
                return Ok(json!({ "outcome": "launched" }));
            }
            return message(ctx, agent, text, quiet).await;
        }
        "queue-cancel" => return queue_cancel(ctx, agent, body.id.as_deref().unwrap_or("")).map(|()| Value::Null),
        _ => {}
    }
    match op {
        "interrupt" | "ctrl-c" => interrupt(ctx, agent, op)?,
        "launch" => launch(ctx, &profile, body.cli.as_deref()).await?,
        "restart" | "terminate" | "resume" => {
            bridged(ctx, json!({ "op": op, "agentId": agent })).await?;
        }
        _ => return Err("unknown-op".into()),
    }
    activity(ctx, op, Some(agent), body.cli.as_deref());
    Ok(Value::Null)
}

/// 터미널의 CLI에 직접 쓴 글을 넣는다(업무 보내기와 달리 업무 보드에 남지 않는다). 입력 대기면 바로,
/// 작업 중이면 대기열에 쌓아 턴이 끝날 때 넣는다 — 앱의 대화 보기와 같은 대기열이다.
async fn message(ctx: &Dash, agent: &str, text: &str, quiet: bool) -> Result<Value, String> {
    let office = is_office_agent_id(agent);
    let outcome = ctx.ctl.chat_queue.send(&ctx.ctl, agent, text).await?;
    let queued = outcome == crate::chat_queue::SendOutcome::Queued;
    let op = match (office, queued) {
        (true, true) => "office-agent-queued",
        (true, false) => "office-agent-follow-up",
        (false, true) => "message-queued",
        (false, false) => "message",
    };
    if !quiet {
        activity(ctx, op, (!office).then_some(agent), Some(&first_line(text)));
    }
    Ok(json!({ "outcome": outcome }))
}

fn queue_cancel(ctx: &Dash, agent: &str, id: &str) -> Result<(), String> {
    if ctx.ctl.chat_queue.cancel(agent, id) {
        Ok(())
    } else {
        Err("queue-gone".into())
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

/// 웹에서 띄울 CLI. 업무 보내기가 아는 CLI(`WorkCli`)는 그 실행 명령을 그대로 쓰고,
/// OpenCode는 앱 훅이 없어 맨 명령만 친다(작업 상태는 관찰하지 않는다).
enum LaunchCli {
    Work(WorkCli),
    Opencode,
}

fn launch_cli(name: &str) -> Option<LaunchCli> {
    Some(match name {
        "claude" => LaunchCli::Work(WorkCli::Claude),
        "codex" => LaunchCli::Work(WorkCli::Codex),
        "pi" => LaunchCli::Work(WorkCli::Pi),
        "kilo" => LaunchCli::Work(WorkCli::Kilo),
        "agy" => LaunchCli::Work(WorkCli::Agy),
        "opencode" => LaunchCli::Opencode,
        _ => return None,
    })
}

/// 셸 프롬프트에 있는 터미널에서 CLI를 띄운다. 프로필 기본 CLI가 같은
/// 종류면 그 모델·노력 설정을 따른다. CLI가 이미 돌고 있거나 쓰다 만 입력이 있으면 거절한다.
async fn launch(ctx: &Dash, profile: &AgentProfile, cli: Option<&str>) -> Result<(), String> {
    let cli = launch_cli(cli.unwrap_or("claude")).ok_or("unknown-cli")?;
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
    let command = match cli {
        LaunchCli::Work(cli) => {
            let agent = profile
                .default_agent
                .clone()
                .filter(|a| a.cli == cli)
                .unwrap_or(WorkAgent { cli, model: None, effort: None });
            crate::work_send::validated_command(&agent, syntax)?
        }
        LaunchCli::Opencode => "opencode".to_string(),
    };
    use crate::session::inject::{InjectSource, SubmitOutcome};
    match ctx.ctl.gate.submit(&profile.id, &session_id, &command, InjectSource::WorkSend).await {
        SubmitOutcome::Submitted => Ok(()),
        SubmitOutcome::Deferred(_) => Err("busy".into()),
        SubmitOutcome::SessionChanged => Err("session-changed".into()),
        SubmitOutcome::NotRunning => Err("not-running".into()),
    }
}

/// 터미널이 셸 프롬프트에 있으면(CLI가 없음) 업무 보내기와 같은 길로 CLI를 띄우고 글을 첫 프롬프트로 넣는다.
/// 업무 보드에는 남기지 않는다. 셸 프롬프트가 아니면 None — 호출자가 돌고 있는 CLI에 넣는다.
async fn launch_with_prompt(ctx: &Dash, profile: &AgentProfile, cli: Option<&str>, text: &str) -> Option<Result<(), String>> {
    let target = ctx.ctl.work_send.inspect(&ctx.ctl.gate, &profile.id).ok()?;
    let session_id = target.session_id.filter(|_| !target.restart_required)?;
    if let Err(e) = crate::chat_queue::check_text(text) {
        return Some(Err(e));
    }
    let picked = cli.and_then(launch_cli).and_then(|c| match c {
        LaunchCli::Work(c) => Some(c),
        LaunchCli::Opencode => None,
    });
    let default = profile.default_agent.clone();
    let agent = match picked {
        Some(cli) => default.filter(|a| a.cli == cli).unwrap_or(WorkAgent { cli, model: None, effort: None }),
        None => default.unwrap_or(WorkAgent { cli: WorkCli::Claude, model: None, effort: None }),
    };
    let request = crate::types::WorkSendRequest {
        agent_id: profile.id.clone(),
        session_id,
        human_input_revision: target.human_input_revision,
        fresh_session: false,
        agent,
        title: String::new(),
        body: text.to_string(),
    };
    Some(ctx.ctl.work_send.send(&ctx.ctl.gate, ctx.ctl.app_data_dir.clone(), request).await.map(|_| ()))
}

// ── 일반 에이전트 ─────────────────────────────────────────────────────
// 캐릭터 없는 사무실 세션 한 칸. 세션이 없으면(또는 CLI가 끝나 셸만 남았으면) 렌더러에 넘겨 곁 탭으로
// 새로 열고, CLI가 턴을 마치고 입력을 기다리면 같은 세션에 지시를 잇는다. 작업 중·사람 대기면 거절한다.
// `restart`는 웹에서 "지금 세션이 끊긴다" 경고를 확인한 새 세션 요청이다.

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OfficeAgentSend {
    text: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    cli: Option<String>,
    #[serde(default)]
    restart: bool,
}

async fn office_agent_send(State(ctx): Ctx, Json(b): Json<OfficeAgentSend>) -> Json<Value> {
    if let Err(e) = crate::office_agent::check_prompt(&b.text) {
        return fail(e);
    }
    let cli = b.cli.as_deref().unwrap_or("claude");
    if cli != "claude" && cli != "codex" {
        return fail("unknown-cli");
    }
    if crate::office_agent::is_opening() {
        return fail("office-agent-busy");
    }
    let running = ctx.ctl.manager.is_running(OFFICE_AGENT_ID);
    if running && !b.restart {
        let probe = {
            let ctx = ctx.clone();
            tokio::task::spawn_blocking(move || super::snapshot::probe_shells(&ctx.ctl, &ctx.dash.probes, &[OFFICE_AGENT_ID.to_string()]))
                .await
                .unwrap_or_default()
                .remove(OFFICE_AGENT_ID)
                .unwrap_or_default()
        };
        if probe.cli.is_some() {
            // 입력 대기면 이어 넣고, 작업 중이면 대기열에 쌓는다.
            return match message(&ctx, OFFICE_AGENT_ID, &b.text, false).await {
                Ok(v) => ok(v),
                Err(e) => fail(e),
            };
        }
        if probe.at_prompt != Some(true) {
            return fail("office-agent-shell-busy");
        }
        // CLI가 끝나 셸만 남았다. 끊길 대화가 없으니 새 세션으로 연다.
    }
    let cwd = match b.cwd.as_deref().filter(|c| !c.is_empty()) {
        Some(c) if folders(&ctx).iter().any(|f| f == c) => Some(c.to_string()),
        Some(_) => return fail("office-agent-invalid-cwd"),
        None => None,
    };
    crate::office_agent::clear_last_error();
    let payload = json!({
        "op": "office-agent-open",
        "prompt": b.text,
        "cwd": cwd,
        "cli": cli,
        "restartConfirmed": running,
    });
    match bridged(&ctx, payload).await {
        Ok(v) => {
            activity(&ctx, "office-agent-send", None, Some(&first_line(&b.text)));
            ok(v)
        }
        Err(e) => fail(e),
    }
}

async fn office_agent_command(
    State(ctx): Ctx,
    Path(op): Path<String>,
    body: Option<Json<AgentCommandBody>>,
) -> Json<Value> {
    if op == "queue-cancel" {
        let id = body.and_then(|Json(b)| b.id).unwrap_or_default();
        return match queue_cancel(&ctx, OFFICE_AGENT_ID, &id) {
            Ok(()) => ok(Value::Null),
            Err(e) => fail(e),
        };
    }
    let result = match op.as_str() {
        "interrupt" | "ctrl-c" => interrupt(&ctx, OFFICE_AGENT_ID, &op),
        "terminate" if !ctx.ctl.manager.is_running(OFFICE_AGENT_ID) => Err("not-running".into()),
        "terminate" => bridged(&ctx, json!({ "op": "office-agent-close" })).await.map(|_| ()),
        _ => Err("unknown-op".into()),
    };
    match result {
        Ok(()) => {
            activity(&ctx, &format!("office-agent-{op}"), None, None);
            ok(Value::Null)
        }
        Err(e) => fail(e),
    }
}

fn first_line(text: &str) -> String {
    clip(text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or(""), 80)
}

fn parse_work_cli(name: &str) -> Option<WorkCli> {
    Some(match name {
        "claude" => WorkCli::Claude,
        "codex" => WorkCli::Codex,
        "pi" => WorkCli::Pi,
        "kilo" => WorkCli::Kilo,
        "agy" => WorkCli::Agy,
        _ => return None,
    })
}

fn check_task_text(text: &str, max_bytes: usize) -> Result<(), &'static str> {
    if text.len() > max_bytes {
        return Err("too-large");
    }
    if text.bytes().any(|b| b < 0x20 && !matches!(b, b'\n' | b'\r' | b'\t')) {
        return Err("control-char");
    }
    Ok(())
}

fn validate_work_agent(agent: &WorkAgent) -> Result<(), &'static str> {
    if let Some(m) = agent.model.as_deref() {
        check_task_text(m, 256)?;
    }
    if let Some(e) = agent.effort.as_deref() {
        check_task_text(e, 64)?;
    }
    Ok(())
}

fn validate_auto_run_limits(max_follow_ups: Option<u32>, permission_wait_minutes: Option<u32>) -> Result<(), &'static str> {
    if let Some(n) = max_follow_ups {
        if n > 99 {
            return Err("bad-request");
        }
    }
    if let Some(m) = permission_wait_minutes {
        if !(1..=1440).contains(&m) {
            return Err("bad-request");
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewTask {
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    assignee_agent_id: Option<String>,
    #[serde(default)]
    cli: Option<String>,
    #[serde(default)]
    agent: Option<WorkAgent>,
    #[serde(default)]
    status: Option<WorkStatus>,
    #[serde(default)]
    depends_on: Option<Vec<String>>,
    #[serde(default)]
    plan: Option<bool>,
    #[serde(default)]
    max_follow_ups: Option<u32>,
    #[serde(default)]
    permission_wait_minutes: Option<u32>,
    #[serde(default)]
    send: bool,
    /// 터미널에 CLI가 돌고 있어 재시작해야 보낼 수 있을 때, 웹에서 "이전 세션이
    /// 끊긴다"는 경고를 보고 확인했다.
    #[serde(default)]
    restart: bool,
}

async fn create_task(State(ctx): Ctx, Json(t): Json<NewTask>) -> Json<Value> {
    let title = t.title.trim();
    if title.is_empty() && t.body.trim().is_empty() {
        return fail("empty");
    }
    if let Err(e) = check_limits(title) {
        return fail(e);
    }
    if let Err(e) = check_task_text(&t.body, TASK_BODY_MAX_BYTES) {
        return fail(e);
    }
    if let Err(e) = validate_auto_run_limits(t.max_follow_ups, t.permission_wait_minutes) {
        return fail(e);
    }
    let assignee = t.assignee_agent_id.filter(|s| !s.trim().is_empty());
    if let Some(id) = &assignee {
        if profile(&ctx, id).is_none() {
            return fail("unknown-agent");
        }
    }
    let agent = match (t.agent, t.cli.as_deref().filter(|c| !c.is_empty())) {
        (Some(a), _) => {
            if let Err(e) = validate_work_agent(&a) {
                return fail(e);
            }
            Some(a)
        }
        (None, Some(c)) => match parse_work_cli(c) {
            Some(cli) => Some(WorkAgent { cli, model: None, effort: None }),
            None => return fail("unknown-cli"),
        },
        (None, None) => None,
    };
    // 보내기가 터미널 재시작을 부르면(CLI가 돌고 있음) 웹에서 확인을 받은 뒤에만 한다.
    // 앱 확인창은 띄우지 않는다 — 폰에서 보낸 일이 책상 앞 확인을 기다리며 멈추지 않게.
    let send = t.send && assignee.is_some();
    if let (true, Some(id)) = (send, assignee.as_deref()) {
        let target = match ctx.ctl.work_send.inspect(&ctx.ctl.gate, id) {
            Ok(target) => target,
            Err(e) => return fail(e),
        };
        if target.has_uncommitted {
            return fail("human-input-pending");
        }
        if target.restart_required && !t.restart {
            return fail("restart-required");
        }
    }
    let payload = json!({
        "op": "work-create",
        "title": title,
        "body": t.body,
        "assigneeAgentId": assignee,
        "agent": agent,
        "status": t.status,
        "dependsOn": t.depends_on,
        "plan": t.plan,
        "maxFollowUps": t.max_follow_ups,
        "permissionWaitMinutes": t.permission_wait_minutes,
        "send": send,
        "restartConfirmed": send && t.restart,
    });
    let label = if title.is_empty() { first_line(&t.body) } else { title.to_string() };
    match bridged(&ctx, payload).await {
        Ok(v) => {
            activity(&ctx, if send { "work-create-send" } else { "work-create" }, assignee.as_deref(), Some(&label));
            ok(v)
        }
        Err(e) => fail(e),
    }
}

async fn edit_task(State(ctx): Ctx, Path(id): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    let Some(obj) = body.as_object() else {
        return fail("bad-request");
    };
    let item = ctx.ctl.work_panels.load_work_items().ok().and_then(|items| items.into_iter().find(|i| i.id == id));
    let Some(item) = item else {
        return fail("task-not-found");
    };
    let mut patch = serde_json::Map::new();
    for (k, v) in obj {
        match k.as_str() {
            "title" => {
                let Some(s) = v.as_str() else { return fail("bad-request"); };
                if let Err(e) = check_limits(s.trim()) { return fail(e); }
                patch.insert(k.clone(), json!(s.trim()));
            }
            "body" => {
                let Some(s) = v.as_str() else { return fail("bad-request"); };
                if let Err(e) = check_task_text(s, TASK_BODY_MAX_BYTES) { return fail(e); }
                patch.insert(k.clone(), json!(s));
            }
            "status" => {
                let Ok(st) = serde_json::from_value::<WorkStatus>(v.clone()) else { return fail("bad-request"); };
                patch.insert(k.clone(), json!(st));
            }
            "assigneeAgentId" => {
                if v.is_null() || v.as_str().is_some_and(|s| s.trim().is_empty()) {
                    patch.insert(k.clone(), Value::Null);
                } else if let Some(s) = v.as_str() {
                    if profile(&ctx, s).is_none() { return fail("unknown-agent"); }
                    patch.insert(k.clone(), json!(s));
                } else {
                    return fail("bad-request");
                }
            }
            "agent" => {
                if v.is_null() {
                    patch.insert(k.clone(), Value::Null);
                } else {
                    let Ok(a) = serde_json::from_value::<WorkAgent>(v.clone()) else { return fail("unknown-cli"); };
                    if let Err(e) = validate_work_agent(&a) { return fail(e); }
                    patch.insert(k.clone(), json!(a));
                }
            }
            "dependsOn" => {
                if v.is_null() {
                    patch.insert(k.clone(), Value::Null);
                } else {
                    let Ok(deps) = serde_json::from_value::<Vec<String>>(v.clone()) else { return fail("bad-request"); };
                    patch.insert(k.clone(), json!(deps));
                }
            }
            "plan" => {
                let Some(b) = v.as_bool() else { return fail("bad-request"); };
                patch.insert(k.clone(), json!(b));
            }
            "maxFollowUps" => {
                if v.is_null() {
                    patch.insert(k.clone(), Value::Null);
                } else if let Some(n) = v.as_u64().and_then(|n| u32::try_from(n).ok()) {
                    if let Err(e) = validate_auto_run_limits(Some(n), None) { return fail(e); }
                    patch.insert(k.clone(), json!(n));
                } else {
                    return fail("bad-request");
                }
            }
            "permissionWaitMinutes" => {
                if v.is_null() {
                    patch.insert(k.clone(), Value::Null);
                } else if let Some(m) = v.as_u64().and_then(|m| u32::try_from(m).ok()) {
                    if let Err(e) = validate_auto_run_limits(None, Some(m)) { return fail(e); }
                    patch.insert(k.clone(), json!(m));
                } else {
                    return fail("bad-request");
                }
            }
            _ => return fail("bad-request"),
        }
    }
    let next_title = patch.get("title").and_then(|v| v.as_str()).unwrap_or(&item.title);
    let next_body = patch.get("body").and_then(|v| v.as_str()).unwrap_or(&item.body);
    if next_title.trim().is_empty() && next_body.trim().is_empty() {
        return fail("empty");
    }
    let next_assignee = match patch.get("assigneeAgentId") {
        Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.as_str()),
        _ => item.assignee_agent_id.as_deref(),
    };
    let label = if next_title.trim().is_empty() { first_line(next_body) } else { next_title.to_string() };
    match bridged(&ctx, json!({ "op": "work-edit", "itemId": id, "patch": patch })).await {
        Ok(v) => {
            activity(&ctx, "work-edit", next_assignee, Some(&label));
            ok(v)
        }
        Err(e) => fail(e),
    }
}

async fn delete_task(State(ctx): Ctx, Path(id): Path<String>) -> Json<Value> {
    task_op(&ctx, "work-delete", &id).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReorderBody {
    target_id: String,
    #[serde(default)]
    after: bool,
}

async fn reorder_task(State(ctx): Ctx, Path(id): Path<String>, Json(b): Json<ReorderBody>) -> Json<Value> {
    match bridged(&ctx, json!({ "op": "work-reorder", "itemId": id, "targetId": b.target_id, "after": b.after })).await {
        Ok(v) => ok(v),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SendTaskBody {
    #[serde(default)]
    restart: bool,
}

async fn send_task(State(ctx): Ctx, Path(id): Path<String>, body: Option<Json<SendTaskBody>>) -> Json<Value> {
    let restart = body.map(|Json(b)| b.restart).unwrap_or(false);
    let item = ctx.ctl.work_panels.load_work_items().ok().and_then(|items| items.into_iter().find(|i| i.id == id));
    let Some(item) = item else {
        return fail("task-not-found");
    };
    let Some(assignee) = item.assignee_agent_id.as_deref() else {
        return fail("work-send-agent-missing");
    };
    let label = if item.title.trim().is_empty() { first_line(&item.body) } else { item.title.clone() };
    // 다른 머신 캐릭터에게 맡긴 업무는 앱이 그 머신에 넘긴다(그 머신 세션은 그 머신이 연다).
    if remote_of(&ctx, assignee).is_some() {
        return match bridged(&ctx, json!({ "op": "work-send", "itemId": id, "restartConfirmed": true })).await {
            Ok(v) => {
                activity(&ctx, "work-send", None, Some(&label));
                ok(v)
            }
            Err(e) => fail(e),
        };
    }
    if profile(&ctx, assignee).is_none() {
        return fail("work-send-agent-missing");
    }
    let target = match ctx.ctl.work_send.inspect(&ctx.ctl.gate, assignee) {
        Ok(target) => target,
        Err(e) => return fail(e),
    };
    if target.has_uncommitted {
        return fail("human-input-pending");
    }
    if target.restart_required && !restart {
        return fail("restart-required");
    }
    match bridged(&ctx, json!({ "op": "work-send", "itemId": id, "restartConfirmed": restart })).await {
        Ok(v) => {
            activity(&ctx, "work-send", Some(assignee), Some(&label));
            ok(v)
        }
        Err(e) => fail(e),
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AutoRunBody {
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    usage_guard: Option<bool>,
}

async fn autorun_task(State(ctx): Ctx, Path(id): Path<String>, body: Option<Json<AutoRunBody>>) -> Json<Value> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let op = match body.action.as_deref().unwrap_or("start") {
        "start" => "autorun-start",
        "pause" => "autorun-pause",
        "resume" => "autorun-resume",
        "end" => "autorun-end",
        "usage-guard" => "autorun-usage-guard",
        _ => return fail("bad-request"),
    };
    let item = ctx.ctl.work_panels.load_work_items().ok().and_then(|items| items.into_iter().find(|i| i.id == id));
    let Some(item) = item else {
        return fail("task-not-found");
    };
    let label = if item.title.trim().is_empty() { first_line(&item.body) } else { item.title.clone() };
    match bridged(&ctx, json!({ "op": op, "itemId": id, "usageGuard": body.usage_guard })).await {
        Ok(v) => {
            if op != "autorun-usage-guard" {
                activity(&ctx, op, item.assignee_agent_id.as_deref(), Some(&label));
            }
            ok(v)
        }
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScheduleBody {
    action: String,
    #[serde(default)]
    rule: Option<Value>,
}

async fn schedule_task(State(ctx): Ctx, Path(id): Path<String>, Json(b): Json<ScheduleBody>) -> Json<Value> {
    if !matches!(b.action.as_str(), "set" | "enable" | "disable" | "remove") {
        return fail("bad-request");
    }
    if b.action == "set" && b.rule.is_none() {
        return fail("bad-request");
    }
    let item = ctx.ctl.work_panels.load_work_items().ok().and_then(|items| items.into_iter().find(|i| i.id == id));
    let Some(item) = item else {
        return fail("task-not-found");
    };
    let label = if item.title.trim().is_empty() { first_line(&item.body) } else { item.title.clone() };
    match bridged(&ctx, json!({ "op": "work-schedule", "itemId": id, "action": b.action, "rule": b.rule })).await {
        Ok(v) => {
            activity(&ctx, "work-schedule", item.assignee_agent_id.as_deref(), Some(&label));
            ok(v)
        }
        Err(e) => fail(e),
    }
}

/// 끝난 업무를 완료로 옮기고, 담당 캐릭터에 떠 있는 알림(완료 카드·배지)도 내린다.
async fn done_task(State(ctx): Ctx, Path(id): Path<String>) -> Json<Value> {
    task_op(&ctx, "work-done", &id).await
}

async fn task_op(ctx: &Dash, op: &str, id: &str) -> Json<Value> {
    let item = ctx.ctl.work_panels.load_work_items().ok().and_then(|items| items.into_iter().find(|i| i.id == id));
    let Some(item) = item else {
        return fail("task-not-found");
    };
    let label = if item.title.trim().is_empty() { first_line(&item.body) } else { item.title.clone() };
    match bridged(ctx, json!({ "op": op, "itemId": id })).await {
        Ok(v) => {
            activity(ctx, op, item.assignee_agent_id.as_deref(), Some(&label));
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
    if let Err(e) = check_limits(&a.text) {
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

// ── 권한 요청·선택지 ─────────────────────────────────────────────────
// Claude `PermissionRequest` 훅이 붙들고 있는 요청에 답한다(`observer/permission.rs`).
// 터미널 대화상자도 함께 떠 있어 먼저 답한 쪽이 이긴다 — 이미 답한 요청이면 `ask-gone`.

#[derive(Deserialize, Serialize)]
pub(crate) struct AskAnswer {
    /// allow | always | deny | dismiss
    pub action: String,
    /// AskUserQuestion: 질문 → 답.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answers: Option<HashMap<String, String>>,
    /// 거부 사유(에이전트에게 전한다).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// 누구의 요청인가(다른 머신 캐릭터면 그 머신에 넘긴다). 없으면 요청 id로 찾는다.
    #[serde(rename = "agentId", default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

async fn answer_ask(State(ctx): Ctx, Path(id): Path<String>, Json(a): Json<AskAnswer>) -> Json<Value> {
    // 다른 머신의 요청이면 그 머신 훅 중계로 답한다. 캐릭터 id가 없으면 받아 둔 모습에서 요청 id로 찾는다.
    let remote = match a.agent_id.as_deref().and_then(|agent| remote_of(&ctx, agent)) {
        Some((remote, device, _)) => Some((remote, device)),
        None => ctx.dash.link_remote().and_then(|r| r.device_of_ask(&id).map(|d| (r, d))),
    };
    if let Some((remote, device)) = remote {
        let mut ask = serde_json::to_value(&a).unwrap_or_else(|_| json!({}));
        ask["askId"] = json!(id);
        if let Some(obj) = ask.as_object_mut() {
            obj.remove("agentId");
        }
        return match remote.request(&device, "ask", ask).await {
            Ok(_) => ok(json!({})),
            Err(e) => fail(e),
        };
    }
    match answer_ask_here(&ctx, &id, a) {
        Ok(()) => ok(json!({})),
        Err(e) => fail(e),
    }
}

/// 이 머신 훅 중계에 걸린 요청에 답한다(호스트가 넘긴 원격 답도 여기로 온다).
pub(crate) fn answer_ask_local(ctl: Arc<ControlContext>, dash: Arc<WebDashState>, id: &str, a: AskAnswer) -> Result<(), String> {
    answer_ask_here(&Dash { ctl, dash }, id, a)
}

fn answer_ask_here(ctx: &Dash, id: &str, a: AskAnswer) -> Result<(), String> {
    use crate::observer::permission::AskDecision;
    for text in a.answers.iter().flat_map(|m| m.values()).chain(a.message.iter()) {
        check_limits(text)?;
    }
    let relay = ctx.ctl.observer.permissions().clone();
    // 누구의 요청인가(앱 알림에 이름을 싣는다).
    let mut owner = None;
    for (sid, agent, _) in ctx.ctl.registry.snapshot() {
        if let Some(ask) = relay.list(&sid).into_iter().find(|x| x.id == id) {
            owner = Some((agent, ask.tool_name));
            break;
        }
    }
    let question = owner.as_ref().is_some_and(|(_, tool)| tool == "AskUserQuestion");
    let op = match a.action.as_str() {
        "allow" if question => "ask-answer",
        "allow" | "always" => "ask-allow",
        "deny" => "ask-deny",
        _ => "",
    };
    let decision = AskDecision::from_action(&a.action, a.answers, a.message)?;
    relay.answer(id, decision)?;
    if !op.is_empty() {
        let (agent, tool) = owner.map(|(a, t)| (Some(a), Some(t))).unwrap_or((None, None));
        let agent = agent.filter(|a| !is_office_agent_id(a));
        activity(ctx, op, agent.as_deref(), tool.as_deref());
    }
    Ok(())
}

// ── 셸 캐릭터 ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ShellQuery {
    rev: Option<u64>,
}

fn shell_profile(ctx: &Dash, agent: &str) -> Result<AgentProfile, String> {
    let p = profile(ctx, agent).ok_or("unknown-agent")?;
    if p.shell_assist != Some(true) {
        return Err("shell-not-enabled".into());
    }
    Ok(p)
}

/// 명령 블록 목록·지금 단계. 내용이 그대로면 `{same:true}`.
async fn shell_state(State(ctx): Ctx, Path(agent): Path<String>, Query(q): Query<ShellQuery>) -> Json<Value> {
    if let Err(e) = shell_profile(&ctx, &agent) {
        return fail(e);
    }
    ok(crate::shell_assist::snapshot(&agent, q.rev))
}

#[derive(Deserialize)]
struct ShellSuggestBody {
    text: String,
}

/// 말 → 명령 제안(실행하지 않는다). 로컬 모델이 느릴 수 있어 끝날 때까지 기다린다.
async fn shell_suggest(State(ctx): Ctx, Path(agent): Path<String>, Json(b): Json<ShellSuggestBody>) -> Json<Value> {
    if let Err(e) = shell_profile(&ctx, &agent) {
        return fail(e);
    }
    match crate::shell_assist::suggest(&agent, &b.text).await {
        Ok(s) => ok(json!(s)),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
struct ShellRunBody {
    command: String,
}

/// 사람이 고른(또는 직접 친) 명령을 셸에 넣고 Enter.
async fn shell_run(State(ctx): Ctx, Path(agent): Path<String>, Json(b): Json<ShellRunBody>) -> Json<Value> {
    if let Err(e) = shell_profile(&ctx, &agent) {
        return fail(e);
    }
    match crate::shell_assist::run(&agent, &b.command) {
        Ok(()) => {
            activity(&ctx, "shell-run", Some(&agent), Some(&first_line(&b.command)));
            ok(Value::Null)
        }
        Err(e) => fail(e),
    }
}

#[cfg(test)]
mod tests {
    use super::cli_of_command;

    #[test]
    fn launch_cli_accepts_every_launchable_cli() {
        use super::{launch_cli, LaunchCli};
        use crate::types::WorkCli;
        for (name, cli) in [("claude", WorkCli::Claude), ("codex", WorkCli::Codex), ("pi", WorkCli::Pi), ("kilo", WorkCli::Kilo), ("agy", WorkCli::Agy)] {
            assert!(matches!(launch_cli(name), Some(LaunchCli::Work(c)) if c == cli), "{name}");
        }
        assert!(matches!(launch_cli("opencode"), Some(LaunchCli::Opencode)));
        assert!(launch_cli("bash").is_none());
        assert!(launch_cli("Claude").is_none());
    }

    #[test]
    fn task_validation_guards_cli_text_and_auto_run_limits() {
        use super::{check_task_text, parse_work_cli, validate_auto_run_limits, validate_work_agent, TASK_BODY_MAX_BYTES};
        use crate::types::{WorkAgent, WorkCli};
        for cli in ["claude", "codex", "agy", "kilo", "pi"] {
            assert!(parse_work_cli(cli).is_some(), "{cli}");
        }
        assert!(parse_work_cli("opencode").is_none());
        assert!(check_task_text("정상 본문\n\t줄바꿈", TASK_BODY_MAX_BYTES).is_ok());
        assert_eq!(check_task_text("bad\x07bell", TASK_BODY_MAX_BYTES), Err("control-char"));
        assert_eq!(check_task_text(&"a".repeat(TASK_BODY_MAX_BYTES + 1), TASK_BODY_MAX_BYTES), Err("too-large"));
        assert!(validate_auto_run_limits(Some(0), Some(1)).is_ok());
        assert!(validate_auto_run_limits(Some(99), Some(1440)).is_ok());
        assert_eq!(validate_auto_run_limits(Some(100), None), Err("bad-request"));
        assert_eq!(validate_auto_run_limits(None, Some(0)), Err("bad-request"));
        assert_eq!(validate_auto_run_limits(None, Some(1441)), Err("bad-request"));
        assert!(validate_work_agent(&WorkAgent { cli: WorkCli::Claude, model: Some("opus".into()), effort: Some("high".into()) }).is_ok());
        assert_eq!(validate_work_agent(&WorkAgent { cli: WorkCli::Claude, model: Some("\x00".into()), effort: None }), Err("control-char"));
    }

    #[test]
    fn page_carries_the_computer_name_from_the_start() {
        let html = super::render_page("Mac <&> Studio");
        assert!(html.contains("<title>Mac &lt;&amp;&gt; Studio · Agent Office</title>"), "{html}");
        assert!(html.contains(r#"<h1 id="title">Mac &lt;&amp;&gt; Studio</h1>"#), "{html}");
        assert_eq!(super::render_page("  "), super::PAGE);
    }

    #[test]
    fn agent_cli_is_recognized_from_the_foreground_command_line() {
        assert_eq!(cli_of_command("claude --model opus"), Some("claude"));
        assert_eq!(cli_of_command("/Users/me/.local/bin/claude"), Some("claude"));
        assert_eq!(cli_of_command("node /opt/homebrew/bin/codex -m gpt-5"), Some("codex"));
        assert_eq!(cli_of_command("/Users/me/.local/bin/agy --dangerously-skip-permissions"), Some("agy"));
        assert_eq!(cli_of_command("agyx"), None);
        assert_eq!(cli_of_command("/opt/codex/codex-aarch64-apple-darwin"), Some("codex"));
        assert_eq!(cli_of_command("C:\\tools\\Codex.exe"), Some("codex"));
        assert_eq!(cli_of_command("npm test"), None);
        assert_eq!(cli_of_command("vim claude.md"), None);
        assert_eq!(cli_of_command("node build.js claude"), None);
    }
}
