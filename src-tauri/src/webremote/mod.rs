// src-tauri/src/webremote/mod.rs
//
// 웹 원격(docs/web-remote-design.md, 선행 docs/archive/web-hosting-design.md).
//
// 앱이 소유·실행 중인 에이전트 세션을 tailnet의 **브라우저**에서 보고 입력한다.
// 프로세스·PTY·observer 훅은 전부 이 앱에 남고, 건너가는 것은 ①출력 바이트
// ②화면 스냅샷 ③앱 이벤트 ④입력 넷뿐이다. 앱↔앱 접속은 범위 밖이다 —
// 클라이언트는 브라우저 하나뿐이고, 가시성은 "내 캐릭터 전부"(주인 의미론)다.
//
// control 서버와의 관계: 2단계 옵트인·토큰 파일·상수시간 비교 **패턴만**
// 재사용하고 리스너와 Router는 분리한다. control은 127.0.0.1 전용이며
// `settings/set`·`create`처럼 네트워크에 내놓으면 안 되는 라우트를 가진다 —
// 이쪽 Router에는 그런 라우트가 아예 존재하지 않는다(권한 축소를 구조로 보장).

pub mod host;
pub mod pairing;
pub mod protocol;
/// 브라우저 클라이언트용 정적 자산 + allowlist RPC 디스패처.
pub mod rpc;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Json, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::notification::hub::NotificationHub;
use crate::observer::server::ObserverServerState;
use crate::observer::ObserverRuntime;
use crate::persistence::profile_store::ProfileStore;
use crate::persistence::settings_store::{AppSettings, WebRemoteBind};
use crate::session::manager::SessionManager;
use crate::state::SessionRegistry;
use crate::types::SessionState;

use host::WebRemoteHub;
use pairing::{PairingOutcome, PairingState, ClientRecord, ClientTokenStore};
use protocol::*;

/// 브라우저 클라이언트가 WS 업그레이드에 실어 보낼 인증 쿠키
/// 이름. 브라우저의 WebSocket API는 커스텀 헤더를 붙일 수 없으므로 헤더 인증만
/// 두면 브라우저는 아예 붙지 못한다.
pub const WEB_REMOTE_COOKIE_NAME: &str = "ao_web_remote_token";

/// 쿠키를 못 쓰는 상황의 보조 경로 — `Sec-WebSocket-Protocol`에 토큰을 싣는
/// 표준 관용. 값 형식은 `agent-office.token.<token>`.
pub const WS_TOKEN_PROTOCOL_PREFIX: &str = "agent-office.token.";

/// WS keepalive: 이 주기로 ping을 보내고,
const WS_PING_EVERY: Duration = Duration::from_secs(20);
/// 이 시간 동안 아무것도 못 받으면 연결을 버린다(좀비 뷰어 정리).
const WS_IDLE_TIMEOUT: Duration = Duration::from_secs(45);

// ── 파일 권한 / 상수시간 비교(control과 동일 규칙) ────────────────────

#[cfg(unix)]
fn set_owner_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn set_owner_only(_path: &Path) {}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// tailnet(Tailscale CGNAT 대역 100.64.0.0/10) 주소인가.
pub fn is_tailnet_addr(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 100 && (64..128).contains(&o[1])
        }
        // Tailscale IPv6는 fd7a:115c:a1e0::/48.
        IpAddr::V6(v6) => {
            let s = v6.segments();
            s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0
        }
    }
}

fn is_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}

/// 이 원격 주소를 정책상 받아 줄 것인가.
///
/// 설계 문서의 "tailscale 인터페이스에만 바인드"는 인터페이스 열거 크레이트를
/// 새로 들이지 않고 **원격 주소 허용목록**으로 등가 구현한다 — 포트는 열리되
/// tailnet 밖 클라이언트는 페어링·WS 어느 것도 시작하지 못하므로, "기본
/// 구성에서 평문이 LAN에 흐르지 않는다"는 보안 성질은 그대로다.
pub fn bind_policy_allows(bind: WebRemoteBind, ip: IpAddr) -> bool {
    match bind {
        WebRemoteBind::Tailnet => is_tailnet_addr(ip) || is_loopback(ip),
        WebRemoteBind::All => true,
        WebRemoteBind::Loopback => is_loopback(ip),
    }
}

// ── 서버 컨텍스트 ────────────────────────────────────────────────────

/// 페어링 요청이 오면 호스트 렌더러에 승인 다이얼로그를 띄우기 위한 알림.
pub type PairNotifyFn = Arc<dyn Fn(&pairing::PendingPairing) + Send + Sync>;

pub struct WebRemoteContext {
    pub manager: Arc<SessionManager>,
    pub registry: Arc<SessionRegistry>,
    pub store: ProfileStore,
    pub settings: Arc<RwLock<AppSettings>>,
    pub hub: Arc<WebRemoteHub>,
    pub tokens: ClientTokenStore,
    pub pairing: Arc<PairingState>,
    pub host_name: String,
    pub app_data_dir: PathBuf,
    /// 웹 RPC가 쓰는 앱 상태 조각들(웹 호스팅 #7m). 커맨드 본문을 복제하지
    /// 않고 `ipc::commands::spawn_session` 등 공용 함수에 그대로 넘긴다.
    pub hub_notify: Arc<NotificationHub>,
    pub observer: Arc<ObserverRuntime>,
    pub observer_server: Arc<ObserverServerState>,
    pub live_usage: Arc<crate::usage::LiveUsageState>,
    pub rate: pairing::PairRateLimiter,
    pair_notify: Mutex<Option<PairNotifyFn>>,
}

/// `WebRemoteContext::new`의 인자 묶음 — 필드가 늘어도 호출부가 위치 인자 나열로
/// 무너지지 않게 한다.
pub struct WebRemoteContextDeps {
    pub manager: Arc<SessionManager>,
    pub registry: Arc<SessionRegistry>,
    pub store: ProfileStore,
    pub settings: Arc<RwLock<AppSettings>>,
    pub hub: Arc<WebRemoteHub>,
    pub app_data_dir: PathBuf,
    pub host_name: String,
    pub hub_notify: Arc<NotificationHub>,
    pub observer: Arc<ObserverRuntime>,
    pub observer_server: Arc<ObserverServerState>,
    pub live_usage: Arc<crate::usage::LiveUsageState>,
}

impl WebRemoteContext {
    pub fn new(deps: WebRemoteContextDeps) -> Self {
        Self {
            manager: deps.manager,
            registry: deps.registry,
            store: deps.store,
            settings: deps.settings,
            hub: deps.hub,
            tokens: ClientTokenStore::new(pairing::token_path(&deps.app_data_dir)),
            pairing: Arc::new(PairingState::default()),
            host_name: deps.host_name,
            app_data_dir: deps.app_data_dir,
            hub_notify: deps.hub_notify,
            observer: deps.observer,
            observer_server: deps.observer_server,
            live_usage: deps.live_usage,
            rate: pairing::PairRateLimiter::default(),
            pair_notify: Mutex::new(None),
        }
    }

    /// 웹 호스팅이 켜져 있는가(정적 자산·웹 RPC 게이트). 매 요청 확인하므로
    /// 토글이 서버 재시작 없이 즉시 반영된다(control 토큰 파일 대조와 같은 패턴).
    pub fn web_remote_enabled(&self) -> bool {
        self.settings
            .read()
            .map(|s| s.web_remote_enabled)
            .unwrap_or(false)
    }

    /// 이 클라이언트가 그 캐릭터를 볼 수 있는가.
    ///
    /// 클라이언트는 브라우저 하나뿐이고 **내 기계를 내가 조종하는 것**이므로
    /// 내 캐릭터 전부가 대상이다(주인 의미론). 단 웹 원격 토글이 꺼져 있으면
    /// 아무것도 못 본다 — 매 요청 확인이라 토글이 즉시 반영된다.
    pub fn agent_allowed(&self, _record: &ClientRecord, agent_id: &str) -> bool {
        self.web_remote_enabled() && self.store.load().agents.iter().any(|a| a.id == agent_id)
    }

    /// 그 클라이언트에게 보여줄 캐릭터 목록(가시성 규칙은 `agent_allowed`와 동일).
    pub fn build_agents_for(&self, _record: &ClientRecord) -> Vec<RemoteAgent> {
        if !self.web_remote_enabled() {
            return Vec::new();
        }
        self.agents_from(|_| true)
    }

    pub fn set_pair_notify(&self, f: PairNotifyFn) {
        *self.pair_notify.lock().unwrap() = Some(f);
    }

    fn notify_pairing(&self, pending: &pairing::PendingPairing) {
        let f = self.pair_notify.lock().unwrap().clone();
        if let Some(f) = f {
            f(pending);
        }
    }

    fn bind_policy(&self) -> WebRemoteBind {
        self.settings
            .read()
            .map(|s| s.web_remote_bind)
            .unwrap_or_default()
    }

    /// 프로필 + 실행 상태를 병합해 `RemoteAgent`를 만든다. 어떤 캐릭터를 담을지는
    /// 호출자가 준 술어가 정한다(가시성 규칙의 단일 구현 지점).
    fn agents_from(&self, keep: impl Fn(&str) -> bool) -> Vec<RemoteAgent> {
        let mut by_agent: HashMap<String, (String, SessionState)> = HashMap::new();
        for (sid, agent, state) in self.registry.snapshot() {
            by_agent.insert(agent, (sid, state));
        }
        self.store
            .load()
            .agents
            .into_iter()
            .filter(|p| keep(&p.id))
            .map(|p| {
                let live = by_agent.get(&p.id);
                let (cols, rows) = self.manager.size_of(&p.id).unwrap_or((0, 0));
                RemoteAgent {
                    agent_id: p.id.clone(),
                    name: p.name,
                    role: Some(p.role),
                    seed: p.seed,
                    cwd: p.cwd,
                    state: live.map(|(_, s)| session_state_str(*s).to_string()),
                    session_id: live.map(|(sid, _)| sid.clone()),
                    cols,
                    rows,
                }
            })
            .collect()
    }
}

fn session_state_str(state: SessionState) -> &'static str {
    match state {
        SessionState::Starting => "starting",
        SessionState::Running => "running",
        SessionState::Exited => "exited",
        SessionState::Disposed => "disposed",
    }
}

// ── 응답 봉투 ────────────────────────────────────────────────────────

fn ok<T: serde::Serialize>(data: T) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "data": data }))
}
fn fail(msg: impl Into<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": false, "error": msg.into() }))
}

// ── 원격 주소 정책 미들웨어 ───────────────────────────────────────────

async fn remote_policy(
    State(ctx): State<Arc<WebRemoteContext>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    if bind_policy_allows(ctx.bind_policy(), addr.ip()) {
        next.run(req).await
    } else {
        (
            StatusCode::FORBIDDEN,
            fail("이 네트워크에서는 접근이 허용되지 않습니다"),
        )
            .into_response()
    }
}

// ── 페어링 핸들러 ────────────────────────────────────────────────────

async fn pair_start(
    State(ctx): State<Arc<WebRemoteContext>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<PairStartRequest>,
) -> Response {
    // 브라우저 클라이언트가 생기면서 페어링 표면이 커졌다 — 시작 자체를
    // IP별로 제한하지 않으면 "새 페어링을 계속 열어 코드를 무한 시도"가 된다.
    if !ctx.rate.allow_start(addr.ip()) {
        eprintln!("webremote: pair/start 레이트리밋 초과 from {}", addr.ip());
        return (
            StatusCode::TOO_MANY_REQUESTS,
            fail("요청이 너무 잦습니다. 잠시 후 다시 시도하세요"),
        )
            .into_response();
    }
    if !ctx.web_remote_enabled() {
        return (
            StatusCode::FORBIDDEN,
            fail("웹 원격이 꺼져 있습니다"),
        )
            .into_response();
    }
    let name = if req.client_name.trim().is_empty() {
        "이름 없는 손님".to_string()
    } else {
        req.client_name.trim().chars().take(60).collect()
    };
    let Some(pending) = ctx.pairing.start(&name) else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            fail("대기 중인 연결 요청이 너무 많습니다"),
        )
            .into_response();
    };
    ctx.notify_pairing(&pending);
    ok(PairStartResponse {
        pairing_id: pending.pairing_id,
        expires_in: pairing::PAIRING_TTL.as_secs(),
        host_name: ctx.host_name.clone(),
        proto_version: WEB_REMOTE_PROTO_VERSION,
    })
    .into_response()
}

async fn pair_complete(
    State(ctx): State<Arc<WebRemoteContext>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<PairCompleteRequest>,
) -> Response {
    if !ctx.rate.auth_allowed(addr.ip()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            fail("인증 실패가 잦습니다. 잠시 후 다시 시도하세요"),
        )
            .into_response();
    }
    match ctx.pairing.complete(&req.pairing_id, &req.code) {
        PairingOutcome::Approved(permission) => {
            let record = ClientRecord {
                client_id: pairing::new_client_id(),
                name: ctx
                    .pairing
                    .list()
                    .into_iter()
                    .find(|p| p.pairing_id == req.pairing_id)
                    .map(|p| p.client_name)
                    .unwrap_or_else(|| "브라우저".into()),
                token: pairing::new_token(),
                permission,
                created_at: pairing::now_ms(),
            };
            if let Err(e) = ctx.tokens.insert(record.clone()) {
                return fail(format!("토큰 저장 실패: {e}")).into_response();
            }
            let token = record.token.clone();
            let mut resp = ok(PairCompleteResponse {
                client_token: record.token,
                client_id: record.client_id,
                host_name: ctx.host_name.clone(),
                permission,
                proto_version: WEB_REMOTE_PROTO_VERSION,
            })
            .into_response();
            // 브라우저 인증 쿠키. `Secure`는 https일 때만 의미가 있어 붙이지
            // 않는다(v1은 tailnet 평문 전제 — tailscale serve로 https를 씌우는
            // 경로는 M3).
            if let Ok(value) = format!(
                "{WEB_REMOTE_COOKIE_NAME}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
                pairing::TOKEN_MAX_AGE_SECS
            )
            .parse()
            {
                resp.headers_mut()
                    .insert(axum::http::header::SET_COOKIE, value);
            }
            resp
        }
        PairingOutcome::AwaitingApproval => (
            StatusCode::ACCEPTED,
            fail("호스트에서 승인을 기다리는 중입니다"),
        )
            .into_response(),
        PairingOutcome::Rejected => {
            (StatusCode::FORBIDDEN, fail("호스트가 연결을 거부했습니다")).into_response()
        }
        PairingOutcome::WrongCode { remaining } => {
            ctx.rate.note_auth_failure(addr.ip());
            eprintln!("webremote: 페어링 코드 불일치 from {}", addr.ip());
            (
                StatusCode::UNAUTHORIZED,
                fail(format!("코드가 맞지 않습니다(남은 시도 {remaining}회)")),
            )
                .into_response()
        }
        PairingOutcome::Expired => (
            StatusCode::GONE,
            fail("페어링이 만료됐습니다. 다시 시도하세요"),
        )
            .into_response(),
    }
}

// ── WS ────────────────────────────────────────────────────────────────

/// 제시된 토큰을 뽑는다. 세 경로를 모두 본다:
///
/// 1. `X-Agent-Office-Web-Remote-Token` 헤더 — 브라우저가 아닌 클라이언트(진단 도구
///    등)의 경로.
/// 2. `Cookie: ao_web_remote_token=…` — **브라우저 경로**. 브라우저의 WebSocket API는
///    커스텀 헤더를 붙일 수 없어서(웹 호스팅 #7m §D) 헤더만 보면 브라우저는
///    아예 붙지 못한다. 페어링 완료 시 HttpOnly 쿠키를 발급해 두면 업그레이드에
///    자동으로 동반된다.
/// 3. `Sec-WebSocket-Protocol: agent-office.token.<token>` — 쿠키를 못 쓰는
///    상황(교차 오리진 등)의 표준 관용 우회. 서버는 고른 서브프로토콜을 응답에
///    그대로 echo 해야 하므로 `ws.protocols(...)`로 되돌려준다.
fn presented_token(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get(WEB_REMOTE_TOKEN_HEADER).and_then(|v| v.to_str().ok()) {
        return Some(v.to_string());
    }
    if let Some(cookie) = headers.get(axum::http::header::COOKIE).and_then(|v| v.to_str().ok()) {
        for part in cookie.split(';') {
            if let Some(value) = part.trim().strip_prefix(&format!("{WEB_REMOTE_COOKIE_NAME}=")) {
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    subprotocol_token(headers)
}

/// `Sec-WebSocket-Protocol` 목록에서 `agent-office.token.<token>` 항목을 찾는다.
fn subprotocol_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get(axum::http::header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok())?;
    raw.split(',')
        .map(str::trim)
        .find_map(|p| p.strip_prefix(WS_TOKEN_PROTOCOL_PREFIX))
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

fn authenticate(ctx: &WebRemoteContext, headers: &HeaderMap) -> Option<ClientRecord> {
    ctx.tokens.authenticate(&presented_token(headers)?)
}

/// 브라우저 클라이언트에 필요한 방어(#7m §D). `Origin`이 없으면
/// 브라우저가 아니다 → 통과. 있으면 우리가 서빙하는 것과 같은 오리진만
/// 허용한다 — 남의 페이지가 사용자의 쿠키를 업고 WS를 여는 것을 막는다.
fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(axum::http::header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let Some(host) = headers.get(axum::http::header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let origin_host = origin
        .split("://")
        .nth(1)
        .unwrap_or(origin)
        .trim_end_matches('/');
    origin_host == host
}

async fn ws_route(
    State(ctx): State<Arc<WebRemoteContext>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !ctx.rate.auth_allowed(addr.ip()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            fail("인증 실패가 잦습니다. 잠시 후 다시 시도하세요"),
        )
            .into_response();
    }
    if !origin_allowed(&headers) {
        return (
            StatusCode::FORBIDDEN,
            fail("forbidden: 허용되지 않은 오리진입니다"),
        )
            .into_response();
    }
    let Some(record) = authenticate(&ctx, &headers) else {
        ctx.rate.note_auth_failure(addr.ip());
        eprintln!("webremote: WS 인증 실패 from {}", addr.ip());
        return (
            StatusCode::UNAUTHORIZED,
            fail("unauthorized: 페어링이 취소됐거나 토큰이 무효합니다"),
        )
            .into_response();
    };
    // 서브프로토콜로 인증했다면 그 값을 그대로 echo 해야 브라우저가 핸드셰이크를
    // 받아들인다.
    let ws = match subprotocol_token(&headers) {
        Some(token) => ws.protocols([format!("{WS_TOKEN_PROTOCOL_PREFIX}{token}")]),
        None => ws,
    };
    ws.on_upgrade(move |socket| serve_ws(socket, ctx, record))
}

/// 한 뷰어 연결의 수명. 읽기(뷰어 메시지)·쓰기(broadcast 팬아웃)·keepalive를
/// 한 루프에서 select 한다 — 쓰기 주체가 하나라 소켓 배타 잠금이 필요 없다.
async fn serve_ws(socket: WebSocket, ctx: Arc<WebRemoteContext>, client: ClientRecord) {
    let (mut sink, mut stream) = socket.split();
    let mut rx = ctx.hub.subscribe();
    // agentId → 다음에 기대하는 절대 오프셋(구멍 감지 + 재접속 기준점).
    let mut attached: HashMap<String, u64> = HashMap::new();
    let mut last_seen = Instant::now();
    let mut ping = tokio::time::interval(WS_PING_EVERY);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    if send_msg(
        &mut sink,
        &HostMsg::Hello {
            host_name: ctx.host_name.clone(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            proto_version: WEB_REMOTE_PROTO_VERSION,
            permission: client.permission,
            client_id: client.client_id.clone(),
        },
    )
    .await
    .is_err()
    {
        return;
    }
    let _ = send_msg(
        &mut sink,
        &HostMsg::Agents {
            agents: ctx.build_agents_for(&client),
        },
    )
    .await;

    loop {
        tokio::select! {
            incoming = stream.next() => {
                let Some(Ok(frame)) = incoming else { break };
                last_seen = Instant::now();
                match frame {
                    Message::Text(text) => {
                        let Ok(msg) = serde_json::from_str::<ClientMsg>(&text) else {
                            let _ = send_msg(&mut sink, &HostMsg::Error {
                                message: "알 수 없는 메시지".into(),
                            }).await;
                            continue;
                        };
                        if handle_client_msg(&mut sink, &ctx, &client, &mut attached, msg).await.is_err() {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            event = rx.recv() => match event {
                Ok(msg) => {
                    if let Some(agent) = forwarded_agent(&msg) {
                        if !attached.contains_key(agent) {
                            continue;
                        }
                    }
                    if let HostMsg::Output(out) = &*msg {
                        let expected = attached.get(&out.agent_id).copied().unwrap_or(out.offset);
                        if out.offset != expected {
                            // 구멍(느린 뷰어의 broadcast 유실 등) — 그 캐릭터만
                            // 기준점부터 다시 복원한다.
                            if restore_agent(&mut sink, &ctx, &out.agent_id, Some(expected), &mut attached).await.is_err() {
                                break;
                            }
                            continue;
                        }
                        attached.insert(out.agent_id.clone(), out.offset + out.bytes);
                    }
                    // 세션 상태가 바뀌면 목록 메타(state/크기)도 같이 갱신한다.
                    if matches!(&*msg, HostMsg::SessionState { .. }) {
                        let _ = send_msg(&mut sink, &HostMsg::Agents { agents: ctx.build_agents_for(&client) }).await;
                    }
                    if send_msg(&mut sink, &msg).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // 큐를 놓쳤다 — 붙어 있는 캐릭터 전부를 마지막 지점부터 복원.
                    let agents: Vec<(String, u64)> =
                        attached.iter().map(|(a, o)| (a.clone(), *o)).collect();
                    let mut failed = false;
                    for (agent, offset) in agents {
                        if restore_agent(&mut sink, &ctx, &agent, Some(offset), &mut attached).await.is_err() {
                            failed = true;
                            break;
                        }
                    }
                    if failed { break; }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            _ = ping.tick() => {
                if last_seen.elapsed() > WS_IDLE_TIMEOUT {
                    break;
                }
                if sink.send(Message::Ping(Vec::new())).await.is_err() {
                    break;
                }
            }
        }
    }
    let _ = sink.close().await;
}

/// 이 메시지가 특정 캐릭터에 매인 것이면 그 agentId(구독 필터용).
fn forwarded_agent(msg: &HostMsg) -> Option<&str> {
    match msg {
        HostMsg::Output(out) => Some(&out.agent_id),
        HostMsg::Activity { agent_id, .. }
        | HostMsg::SessionState { agent_id, .. }
        | HostMsg::Notification { agent_id, .. }
        | HostMsg::NotificationCleared { agent_id, .. }
        | HostMsg::Resized { agent_id, .. }
        | HostMsg::Restore { agent_id, .. } => Some(agent_id),
        _ => None,
    }
}

type WsSink = futures_util::stream::SplitSink<WebSocket, Message>;

async fn send_msg(sink: &mut WsSink, msg: &HostMsg) -> Result<(), ()> {
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    sink.send(Message::Text(text)).await.map_err(|_| ())
}

async fn handle_client_msg(
    sink: &mut WsSink,
    ctx: &Arc<WebRemoteContext>,
    client: &ClientRecord,
    attached: &mut HashMap<String, u64>,
    msg: ClientMsg,
) -> Result<(), ()> {
    match msg {
        ClientMsg::Ping => send_msg(sink, &HostMsg::Pong).await,
        ClientMsg::Detach { agent_id } => {
            attached.remove(&agent_id);
            Ok(())
        }
        ClientMsg::Attach {
            agent_id,
            last_offset,
        } => {
            if !ctx.agent_allowed(client, &agent_id) {
                return send_msg(
                    sink,
                    &HostMsg::Error {
                        message: format!("접근할 수 없는 캐릭터입니다: {agent_id}"),
                    },
                )
                .await;
            }
            // 웹 클라이언트는 공유 토글 없이 붙으므로 tap이 아직 없을 수 있다.
            // sink는 agentId 수명이라 세션 전에 달아도 안전하고, share()는 멱등이다.
            ctx.hub.share(&ctx.manager, &agent_id);
            restore_agent(sink, ctx, &agent_id, last_offset, attached).await
        }
        ClientMsg::Input { agent_id, data } => {
            if !client.permission.allows_input() {
                return send_msg(
                    sink,
                    &HostMsg::Error {
                        message: "읽기 전용으로 연결되어 입력할 수 없습니다".into(),
                    },
                )
                .await;
            }
            if !ctx.agent_allowed(client, &agent_id) {
                return Ok(());
            }
            ctx.manager.write_input(&agent_id, &data);
            Ok(())
        }
        ClientMsg::Rpc { id, cmd, args } => {
            let result = rpc::dispatch(ctx, client, &cmd, args).await;
            let msg = match result {
                Ok(data) => HostMsg::RpcResult {
                    id,
                    ok: true,
                    data: Some(data),
                    error: None,
                },
                Err(error) => HostMsg::RpcResult {
                    id,
                    ok: false,
                    data: None,
                    error: Some(error),
                },
            };
            send_msg(sink, &msg).await
        }
    }
}

/// 복원(스냅샷+델타 또는 델타만)을 보내고 `attached` 기준점을 갱신한다.
async fn restore_agent(
    sink: &mut WsSink,
    ctx: &Arc<WebRemoteContext>,
    agent_id: &str,
    last_offset: Option<u64>,
    attached: &mut HashMap<String, u64>,
) -> Result<(), ()> {
    let Some(plan) = ctx.hub.replay_for(agent_id, last_offset).await else {
        return Ok(());
    };
    let (cols, rows) = ctx.manager.size_of(agent_id).unwrap_or((0, 0));
    send_msg(
        sink,
        &HostMsg::Restore {
            agent_id: agent_id.to_string(),
            snapshot: plan.snapshot.clone(),
            base_offset: plan.base_offset,
            cols,
            rows,
            session_id: ctx.hub.session_id_of(agent_id),
        },
    )
    .await?;
    let mut next = plan.base_offset;
    for chunk in plan.chunks {
        next = chunk.offset + chunk.bytes;
        send_msg(
            sink,
            &HostMsg::Output(RemoteOutput {
                agent_id: agent_id.to_string(),
                session_id: chunk.session_id,
                seq: chunk.seq,
                offset: chunk.offset,
                data: chunk.data,
                bytes: chunk.bytes,
            }),
        )
        .await?;
    }
    attached.insert(agent_id.to_string(), next);
    Ok(())
}

// ── 라우터 / 서버 수명 ────────────────────────────────────────────────

fn router(ctx: Arc<WebRemoteContext>) -> Router {
    Router::new()
        .route("/webremote/v1/pair/start", post(pair_start))
        .route("/webremote/v1/pair/complete", post(pair_complete))
        .route("/webremote/v1/ws", get(ws_route))
        // 웹 호스팅(#7m): 같은 리스너에 라우트를 얹는다 — 별도 포트·프로세스 없음.
        .merge(rpc::routes())
        .layer(axum::middleware::from_fn_with_state(
            ctx.clone(),
            remote_policy,
        ))
        .with_state(ctx)
}

/// 리스너를 실제로 붙일 주소와 tailnet 탐지 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindChoice {
    pub ip: IpAddr,
    /// 로컬 인터페이스에서 tailnet 주소를 찾았는가. `Tailnet` 정책일 때
    /// false면 루프백 폴백이고, 설정 UI가 "tailscale 미탐지"를 띄운다.
    pub tailnet_found: bool,
}

/// 정책 + 이 머신의 로컬 주소 목록 → 바인드할 주소. **순수 함수**라 실제
/// 인터페이스 없이 단위 테스트한다(주소 목록이 곧 입력이다).
///
/// `Tailnet`(기본)은 tailscale 인터페이스 주소에만 리스너를 연다 — 전
/// 인터페이스에 열고 원격 주소로 거르던 예전 방식보다 노출 표면이 작다
/// (tailnet 밖에서는 포트 자체가 닫혀 보인다). IPv4를 우선하는 것은
/// 사용자에게 불러 줄 주소가 짧아야 하기 때문이다.
pub fn choose_bind_ip(bind: WebRemoteBind, addrs: &[IpAddr]) -> BindChoice {
    let tailnet = addrs
        .iter()
        .copied()
        .find(|ip| ip.is_ipv4() && is_tailnet_addr(*ip))
        .or_else(|| addrs.iter().copied().find(|ip| is_tailnet_addr(*ip)));
    let loopback = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
    match bind {
        WebRemoteBind::Loopback => BindChoice {
            ip: loopback,
            tailnet_found: tailnet.is_some(),
        },
        WebRemoteBind::All => BindChoice {
            ip: IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
            tailnet_found: tailnet.is_some(),
        },
        // 못 찾으면 서버는 띄우되 루프백에만 연다 — 조용히 전 인터페이스로
        // 넓히면 "tailnet만 허용"이라는 설정이 거짓말이 된다.
        WebRemoteBind::Tailnet => match tailnet {
            Some(ip) => BindChoice {
                ip,
                tailnet_found: true,
            },
            None => BindChoice {
                ip: loopback,
                tailnet_found: false,
            },
        },
    }
}

/// 이 머신에 붙어 있는 로컬 IP 주소들.
///
/// unix는 `getifaddrs(3)` 인터페이스 열거(nix — portable-pty가 이미 끌고 오는
/// 의존이라 트리가 늘지 않는다). 그 외 플랫폼은 열거 API가 없어 UDP 소켓의
/// 소스 주소를 캐내는 고전적 방법으로 대신한다(실제 패킷은 나가지 않는다) —
/// tailnet 대역으로 "연결"하면 라우팅 테이블이 tailscale 인터페이스 주소를
/// 골라 준다.
pub fn local_ip_addrs() -> Vec<IpAddr> {
    #[cfg(unix)]
    {
        match nix::ifaddrs::getifaddrs() {
            Ok(ifaces) => {
                let mut out = Vec::new();
                for iface in ifaces {
                    let Some(addr) = iface.address else { continue };
                    if let Some(v4) = addr.as_sockaddr_in() {
                        out.push(IpAddr::V4(std::net::Ipv4Addr::from(v4.ip())));
                    } else if let Some(v6) = addr.as_sockaddr_in6() {
                        out.push(IpAddr::V6(v6.ip()));
                    }
                }
                return out;
            }
            Err(e) => {
                eprintln!("webremote: getifaddrs 실패({e}) — 소켓 프로브로 대체");
            }
        }
    }
    probe_local_addrs()
}

/// 인터페이스 열거를 못 쓰는 경로의 대안. tailnet 대역과 공용 대역 각각에
/// "연결"해 보고 소스 주소를 모은다(UDP connect는 패킷을 보내지 않는다).
fn probe_local_addrs() -> Vec<IpAddr> {
    let mut out = Vec::new();
    for target in [("100.100.100.100", 80u16), ("8.8.8.8", 80)] {
        let Ok(sock) = std::net::UdpSocket::bind(("0.0.0.0", 0)) else {
            continue;
        };
        if sock.connect(target).is_ok() {
            if let Ok(local) = sock.local_addr() {
                if !out.contains(&local.ip()) {
                    out.push(local.ip());
                }
            }
        }
    }
    out
}

async fn serve(
    ctx: Arc<WebRemoteContext>,
    port: u16,
    shutdown_rx: oneshot::Receiver<()>,
) -> std::io::Result<(Bound, JoinHandle<()>)> {
    let choice = choose_bind_ip(ctx.bind_policy(), &local_ip_addrs());
    // 고정 포트가 점유돼 있으면 몇 칸 스캔한다(실제 포트는 설정 UI에 표시).
    let mut listener = None;
    for candidate in port..port.saturating_add(8) {
        match tokio::net::TcpListener::bind((choice.ip, candidate)).await {
            Ok(l) => {
                listener = Some(l);
                break;
            }
            Err(_) => continue,
        }
    }
    let listener = match listener {
        Some(l) => l,
        None => tokio::net::TcpListener::bind((choice.ip, 0)).await?,
    };
    let bound = Bound {
        port: listener.local_addr()?.port(),
        ip: choice.ip,
        tailnet_found: choice.tailnet_found,
    };
    let app = router(ctx).into_make_service_with_connect_info::<SocketAddr>();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    Ok((bound, handle))
}

/// 실제로 열린 리스너의 사실들(설정 UI가 그대로 보여준다).
#[derive(Debug, Clone, Copy)]
pub struct Bound {
    pub port: u16,
    pub ip: IpAddr,
    pub tailnet_found: bool,
}

struct InstalledServer {
    bound: Bound,
    shutdown: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

#[derive(Default)]
pub struct WebRemoteServerState {
    start_gate: tokio::sync::Mutex<()>,
    installed: Mutex<Option<InstalledServer>>,
}

impl WebRemoteServerState {
    pub fn current_port(&self) -> Option<u16> {
        self.installed.lock().unwrap().as_ref().map(|s| s.bound.port)
    }

    /// 실제로 열린 리스너의 주소·포트·tailnet 탐지 결과. 서버가 없으면 None.
    pub fn current_bound(&self) -> Option<Bound> {
        self.installed.lock().unwrap().as_ref().map(|s| s.bound)
    }

    pub fn is_running(&self) -> bool {
        self.installed.lock().unwrap().is_some()
    }

    /// opt-in 기동(멱등). 실패해도 GUI 기능에는 영향이 없다(fail-open).
    pub async fn ensure(&self, ctx: Arc<WebRemoteContext>, port: u16) -> Option<u16> {
        let _gate = self.start_gate.lock().await;
        if let Some(p) = self.current_port() {
            return Some(p);
        }
        let (shutdown, rx) = oneshot::channel();
        match serve(ctx, port, rx).await {
            Ok((bound, handle)) => {
                let port = bound.port;
                *self.installed.lock().unwrap() = Some(InstalledServer {
                    bound,
                    shutdown,
                    handle,
                });
                Some(port)
            }
            Err(e) => {
                eprintln!("web remote server unavailable: {e}");
                None
            }
        }
    }

    pub fn shutdown(&self) {
        let installed = self.installed.lock().unwrap().take();
        if let Some(server) = installed {
            let _ = server.shutdown.send(());
            let _detached = server.handle;
        }
    }
}

/// 이 머신을 사람이 알아볼 이름(호스트 승인/뷰어 목록에 표시).
pub fn local_host_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "agent-office".into())
}

/// 브라우저에 불러 줄 주소(설정 UI 표시용). 서버가 떠 있으면 **실제 바인드
/// 주소**가 정답이고, 아직 안 떴으면 tailnet 주소를 추정해 미리 보여준다.
pub fn local_addr_hint() -> Option<String> {
    let addrs = local_ip_addrs();
    addrs
        .iter()
        .copied()
        .find(|ip| ip.is_ipv4() && is_tailnet_addr(*ip))
        .or_else(|| addrs.iter().copied().find(|ip| is_tailnet_addr(*ip)))
        .or_else(|| probe_local_addrs().into_iter().next())
        .map(|ip| ip.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn tailnet_range_detection() {
        assert!(is_tailnet_addr(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))));
        assert!(is_tailnet_addr(IpAddr::V4(Ipv4Addr::new(100, 107, 46, 116))));
        assert!(is_tailnet_addr(IpAddr::V4(Ipv4Addr::new(100, 127, 255, 254))));
        // 경계 밖
        assert!(!is_tailnet_addr(IpAddr::V4(Ipv4Addr::new(100, 63, 0, 1))));
        assert!(!is_tailnet_addr(IpAddr::V4(Ipv4Addr::new(100, 128, 0, 1))));
        assert!(!is_tailnet_addr(IpAddr::V4(Ipv4Addr::new(192, 168, 0, 5))));
    }

    #[test]
    fn tailnet_policy_binds_the_tailscale_interface_address() {
        let addrs = [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::new(192, 168, 0, 5)),
            IpAddr::V4(Ipv4Addr::new(100, 107, 46, 116)),
        ];
        let choice = choose_bind_ip(WebRemoteBind::Tailnet, &addrs);
        assert_eq!(choice.ip, IpAddr::V4(Ipv4Addr::new(100, 107, 46, 116)));
        assert!(choice.tailnet_found);
    }

    #[test]
    fn tailnet_policy_falls_back_to_loopback_when_tailscale_is_absent() {
        // tailscale이 없는 기계 — 서버는 뜨되 LAN에는 절대 열리지 않는다.
        let addrs = [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::new(192, 168, 0, 5)),
        ];
        let choice = choose_bind_ip(WebRemoteBind::Tailnet, &addrs);
        assert_eq!(choice.ip, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert!(!choice.tailnet_found, "설정 UI가 미탐지를 알려야 한다");

        // 주소를 하나도 못 구한 경우도 같은 폴백.
        let empty = choose_bind_ip(WebRemoteBind::Tailnet, &[]);
        assert_eq!(empty.ip, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert!(!empty.tailnet_found);
    }

    #[test]
    fn tailnet_choice_prefers_ipv4_but_takes_ipv6_when_thats_all_there_is() {
        let v6: IpAddr = "fd7a:115c:a1e0::1".parse().unwrap();
        let both = [v6, IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))];
        assert_eq!(
            choose_bind_ip(WebRemoteBind::Tailnet, &both).ip,
            IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)),
            "불러 줄 주소가 짧아야 한다"
        );
        assert_eq!(choose_bind_ip(WebRemoteBind::Tailnet, &[v6]).ip, v6);
    }

    #[test]
    fn all_and_loopback_policies_ignore_the_tailnet_address() {
        let addrs = [IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))];
        let all = choose_bind_ip(WebRemoteBind::All, &addrs);
        assert_eq!(all.ip, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        assert!(all.tailnet_found, "탐지 결과 자체는 정책과 무관하게 보고한다");
        let lo = choose_bind_ip(WebRemoteBind::Loopback, &addrs);
        assert_eq!(lo.ip, IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn tailnet_policy_rejects_plain_lan_but_allows_loopback() {
        let lan = IpAddr::V4(Ipv4Addr::new(192, 168, 0, 5));
        let tail = IpAddr::V4(Ipv4Addr::new(100, 64, 1, 2));
        let local = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert!(!bind_policy_allows(WebRemoteBind::Tailnet, lan));
        assert!(bind_policy_allows(WebRemoteBind::Tailnet, tail));
        assert!(bind_policy_allows(WebRemoteBind::Tailnet, local));

        assert!(bind_policy_allows(WebRemoteBind::All, lan));
        assert!(!bind_policy_allows(WebRemoteBind::Loopback, lan));
        assert!(!bind_policy_allows(WebRemoteBind::Loopback, tail));
        assert!(bind_policy_allows(WebRemoteBind::Loopback, local));
    }

    // ── 페어링 → WS → 복원/입력까지 실제 소켓으로 태우는 통합 테스트 ──────
    //
    // 와이어 계약(라우트·헤더·메시지 tag/필드 케이스)과 권한 게이트는 문서가
    // 아니라 여기서 지킨다.

    use crate::notification::hub::{NotificationHub, SystemClock};
    use crate::session::pty_factory::fake::FakePtyFactory;
    use crate::state::fake::RecordingEvents;
    use crate::state::AppEvents;
    use std::time::Duration;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message as TMessage;

    fn test_profile(id: &str, name: &str) -> crate::types::AgentProfile {
        crate::types::AgentProfile {
            id: id.into(),
            name: name.into(),
            role: "backend".into(),
            note: String::new(),
            seed: "seed".into(),
            created_at: 1,
            desk_index: 0,
            assigned_desk_index: None,
            cwd: None,
            appearance: None,
            portrait_updated_at: None,
            sprite_request: None,
            sprite_updated_at: None,
            archetype: None,
            shell: None,
            startup_command: None,
            personality_prompt: None,
            clocked_out: None,
            keyboard_sound: None,
            voice_id: None,
            bot: None,
        }
    }

    /// pub(crate): 웹 RPC 테스트(`webremote::rpc::tests`)가 같은 픽스처를 쓴다.
    pub(crate) fn build_ctx(tag: &str) -> (Arc<WebRemoteContext>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("webremote-it-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let events: Arc<dyn AppEvents> = Arc::new(RecordingEvents::default());
        let registry = Arc::new(SessionRegistry::new());
        let hub_notify = Arc::new(NotificationHub::new(
            registry.clone(),
            events.clone(),
            Arc::new(SystemClock),
            Duration::from_millis(3000),
        ));
        let observer = Arc::new(crate::observer::ObserverRuntime::production(
            hub_notify.clone(),
            dir.join("observer"),
            std::env::current_exe().unwrap(),
        ));
        let settings = Arc::new(RwLock::new(AppSettings {
            web_remote_bind: WebRemoteBind::Loopback,
            // 페어링·정적자산·RPC가 전부 이 토글을 매 요청 확인한다 —
            // 켜 두지 않으면 픽스처가 페어링 단계에서 403이다.
            web_remote_enabled: true,
            ..AppSettings::default()
        }));
        let observer_server = Arc::new(crate::observer::server::ObserverServerState::default());
        let get_observer_url =
            crate::make_observer_url_getter(settings.clone(), observer_server.clone());
        let (fac, _ctl) = FakePtyFactory::new();
        let manager = Arc::new(SessionManager::new(
            Arc::new(fac),
            observer.clone(),
            registry.clone(),
            events,
            hub_notify.clone(),
            get_observer_url,
        ));
        let store = ProfileStore::new(dir.join("profiles.json"));
        store
            .save(&crate::types::PersistedState {
                agents: vec![test_profile("a1", "아다")],
                version: 1,
                vacation_mode: None,
            })
            .unwrap();
        let ctx = Arc::new(WebRemoteContext::new(WebRemoteContextDeps {
            manager,
            registry,
            store,
            settings,
            hub: WebRemoteHub::new(),
            app_data_dir: dir.clone(),
            host_name: "테스트호스트".into(),
            hub_notify,
            observer,
            observer_server,
            live_usage: Arc::new(crate::usage::LiveUsageState::new()),
        }));
        (ctx, dir)
    }

    /// 페어링 왕복 — 승인 전에는 202(대기), 승인 후에는 토큰 발급.
    async fn pair(port: u16, ctx: &Arc<WebRemoteContext>) -> String {
        let client = reqwest::Client::new();
        let started: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/webremote/v1/pair/start"))
            .json(&serde_json::json!({ "clientName": "테스트뷰어" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(started["ok"], true);
        let pairing_id = started["data"]["pairingId"].as_str().unwrap().to_string();
        let code = ctx
            .pairing
            .list()
            .into_iter()
            .find(|p| p.pairing_id == pairing_id)
            .unwrap()
            .code;

        // 승인 전: 코드가 맞아도 202.
        let waiting = client
            .post(format!("http://127.0.0.1:{port}/webremote/v1/pair/complete"))
            .json(&serde_json::json!({ "pairingId": pairing_id, "code": code }))
            .send()
            .await
            .unwrap();
        assert_eq!(waiting.status(), reqwest::StatusCode::ACCEPTED);

        assert!(ctx.pairing.approve(&pairing_id, ClientPermission::Input));
        let done: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/webremote/v1/pair/complete"))
            .json(&serde_json::json!({ "pairingId": pairing_id, "code": code }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(done["ok"], true);
        done["data"]["clientToken"].as_str().unwrap().to_string()
    }

    async fn open_ws(
        port: u16,
        token: &str,
    ) -> tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    > {
        let mut request = format!("ws://127.0.0.1:{port}/webremote/v1/ws")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert(WEB_REMOTE_TOKEN_HEADER, token.parse().unwrap());
        let (socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        socket
    }

    async fn next_msg(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> HostMsg {
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
                .await
                .expect("호스트 응답 대기 타임아웃")
                .expect("스트림 종료")
                .expect("프레임 오류");
            if let TMessage::Text(text) = frame {
                return serde_json::from_str(&text).expect("호스트 메시지 파싱");
            }
        }
    }

    #[tokio::test]
    async fn pair_then_attach_streams_backlog_and_live_output() {
        let (ctx, dir) = build_ctx("stream");
        let server = WebRemoteServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");
        let token = pair(port, &ctx).await;

        // tap은 브라우저가 attach할 때 깔리지만, 여기서는 "이미 한 번 붙었던"
        // 상태를 만들어 attach 이전 백로그가 링에 남는 경로를 태운다.
        ctx.hub.share(&ctx.manager, "a1");
        // 스냅샷 요청에는 아무도 답하지 않게 두고(링버퍼 폴백 경로) 백로그를 쌓는다.
        ctx.hub.snapshots.set_emitter(Arc::new(|_, _| {}));
        ctx.hub.push_for_test(
            "a1",
            &crate::types::OutputChunk {
                session_id: "s1".into(),
                agent_id: "a1".into(),
                data: "before-attach".into(),
                frames: 1,
                seq: 1,
                bytes: 13,
            },
        );

        let mut socket = open_ws(port, &token).await;
        // hello → agents 순서.
        match next_msg(&mut socket).await {
            HostMsg::Hello {
                permission,
                host_name,
                ..
            } => {
                assert_eq!(permission, ClientPermission::Input);
                assert_eq!(host_name, "테스트호스트");
            }
            other => panic!("hello가 먼저여야 한다: {other:?}"),
        }
        match next_msg(&mut socket).await {
            HostMsg::Agents { agents } => {
                assert_eq!(agents.len(), 1);
                assert_eq!(agents[0].agent_id, "a1");
                assert_eq!(agents[0].name, "아다");
            }
            other => panic!("agents가 와야 한다: {other:?}"),
        }

        socket
            .send(TMessage::Text(
                serde_json::to_string(&ClientMsg::Attach {
                    agent_id: "a1".into(),
                    last_offset: None,
                })
                .unwrap(),
            ))
            .await
            .unwrap();

        match next_msg(&mut socket).await {
            HostMsg::Restore {
                agent_id,
                base_offset,
                snapshot,
                ..
            } => {
                assert_eq!(agent_id, "a1");
                assert_eq!(base_offset, 0, "스냅샷이 없으면 링 시작부터");
                assert!(snapshot.is_none());
            }
            other => panic!("restore가 와야 한다: {other:?}"),
        }
        match next_msg(&mut socket).await {
            HostMsg::Output(out) => {
                assert_eq!(out.data, "before-attach");
                assert_eq!(out.offset, 0);
            }
            other => panic!("백로그 출력이 와야 한다: {other:?}"),
        }

        // attach 이후의 라이브 출력이 이어서 흐른다.
        ctx.hub.push_for_test(
            "a1",
            &crate::types::OutputChunk {
                session_id: "s1".into(),
                agent_id: "a1".into(),
                data: "live".into(),
                frames: 1,
                seq: 2,
                bytes: 4,
            },
        );
        match next_msg(&mut socket).await {
            HostMsg::Output(out) => {
                assert_eq!(out.data, "live");
                assert_eq!(out.offset, 13, "절대 오프셋이 이어져야 한다");
            }
            other => panic!("라이브 출력이 와야 한다: {other:?}"),
        }

        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 프로필에 없는 agentId는 붙을 수 없다 — 웹은 "내 캐릭터 전부"를 보지만
    /// 그 목록은 프로필 저장소가 정본이라 임의 문자열로는 tap을 깔 수 없다.
    #[tokio::test]
    async fn unknown_agent_cannot_be_attached() {
        let (ctx, dir) = build_ctx("unknown-agent");
        let server = WebRemoteServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");
        let token = pair(port, &ctx).await;
        let mut socket = open_ws(port, &token).await;
        let _hello = next_msg(&mut socket).await;
        let _agents = next_msg(&mut socket).await;

        socket
            .send(TMessage::Text(
                serde_json::to_string(&ClientMsg::Attach {
                    agent_id: "ghost".into(), // 프로필에 없는 캐릭터
                    last_offset: None,
                })
                .unwrap(),
            ))
            .await
            .unwrap();
        match next_msg(&mut socket).await {
            HostMsg::Error { message } => assert!(message.contains("접근할 수 없는")),
            other => panic!("에러가 와야 한다: {other:?}"),
        }
        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn read_only_client_is_refused_input() {
        let (ctx, dir) = build_ctx("readonly");
        let server = WebRemoteServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");
        let token = pair(port, &ctx).await;
        // 발급 후 권한을 읽기 전용으로 낮춘다(설정 UI의 권한 변경과 같은 경로).
        ctx.tokens
            .set_permission(
                &ctx.tokens.load()[0].client_id.clone(),
                ClientPermission::ReadOnly,
            )
            .unwrap();

        let mut socket = open_ws(port, &token).await;
        match next_msg(&mut socket).await {
            HostMsg::Hello { permission, .. } => {
                assert_eq!(permission, ClientPermission::ReadOnly)
            }
            other => panic!("hello가 먼저여야 한다: {other:?}"),
        }
        let _agents = next_msg(&mut socket).await;
        socket
            .send(TMessage::Text(
                serde_json::to_string(&ClientMsg::Input {
                    agent_id: "a1".into(),
                    data: "rm -rf /".into(),
                })
                .unwrap(),
            ))
            .await
            .unwrap();
        match next_msg(&mut socket).await {
            HostMsg::Error { message } => assert!(message.contains("읽기 전용")),
            other => panic!("입력 거부 에러가 와야 한다: {other:?}"),
        }
        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ws_without_token_is_rejected() {
        let (ctx, dir) = build_ctx("noauth");
        let server = WebRemoteServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");
        let request = format!("ws://127.0.0.1:{port}/webremote/v1/ws")
            .into_client_request()
            .unwrap();
        let err = tokio_tungstenite::connect_async(request).await;
        assert!(err.is_err(), "토큰 없이 업그레이드되면 안 된다");
        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn revoked_token_cannot_reconnect() {
        let (ctx, dir) = build_ctx("revoked");
        let server = WebRemoteServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");
        let token = pair(port, &ctx).await;
        // 한 번은 붙는다.
        let mut socket = open_ws(port, &token).await;
        let _ = next_msg(&mut socket).await;
        drop(socket);
        // 승인 취소 후에는 같은 토큰이 막힌다(매 연결 파일 대조).
        let client_id = ctx.tokens.load()[0].client_id.clone();
        ctx.tokens.remove(&client_id).unwrap();
        let mut request = format!("ws://127.0.0.1:{port}/webremote/v1/ws")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert(WEB_REMOTE_TOKEN_HEADER, token.parse().unwrap());
        assert!(tokio_tungstenite::connect_async(request).await.is_err());
        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 브라우저 경로(#7m §D): 커스텀 헤더를 못 붙이는 클라이언트가 쿠키/
    /// 서브프로토콜로도 인증돼야 한다. 헤더만 보던 시절엔 브라우저가 아예
    /// 붙지 못했다.
    #[test]
    fn token_is_accepted_from_header_cookie_or_subprotocol() {
        let mut headers = HeaderMap::new();
        assert_eq!(presented_token(&headers), None);

        headers.insert(WEB_REMOTE_TOKEN_HEADER, "from-header".parse().unwrap());
        assert_eq!(presented_token(&headers).as_deref(), Some("from-header"));

        let mut cookies = HeaderMap::new();
        cookies.insert(
            axum::http::header::COOKIE,
            format!("other=1; {WEB_REMOTE_COOKIE_NAME}=from-cookie; x=2")
                .parse()
                .unwrap(),
        );
        assert_eq!(presented_token(&cookies).as_deref(), Some("from-cookie"));

        let mut proto = HeaderMap::new();
        proto.insert(
            axum::http::header::SEC_WEBSOCKET_PROTOCOL,
            format!("chat, {WS_TOKEN_PROTOCOL_PREFIX}from-proto")
                .parse()
                .unwrap(),
        );
        assert_eq!(presented_token(&proto).as_deref(), Some("from-proto"));
        assert_eq!(subprotocol_token(&proto).as_deref(), Some("from-proto"));

        // 값이 빈 쿠키/서브프로토콜은 토큰으로 치지 않는다.
        let mut empty = HeaderMap::new();
        empty.insert(
            axum::http::header::COOKIE,
            format!("{WEB_REMOTE_COOKIE_NAME}=").parse().unwrap(),
        );
        assert_eq!(presented_token(&empty), None);
    }

    #[test]
    fn origin_is_only_checked_for_browsers() {
        // Origin 없음 = 브라우저가 아님 → 통과.
        assert!(origin_allowed(&HeaderMap::new()));

        let mut same = HeaderMap::new();
        same.insert(axum::http::header::ORIGIN, "http://100.64.1.2:47800".parse().unwrap());
        same.insert(axum::http::header::HOST, "100.64.1.2:47800".parse().unwrap());
        assert!(origin_allowed(&same));

        // 남의 페이지가 사용자의 쿠키를 업고 여는 WS는 막는다.
        let mut cross = HeaderMap::new();
        cross.insert(axum::http::header::ORIGIN, "http://evil.example".parse().unwrap());
        cross.insert(axum::http::header::HOST, "100.64.1.2:47800".parse().unwrap());
        assert!(!origin_allowed(&cross));

        // Host를 못 읽으면 판단 불가 → 거부(안전 쪽).
        let mut no_host = HeaderMap::new();
        no_host.insert(axum::http::header::ORIGIN, "http://x".parse().unwrap());
        assert!(!origin_allowed(&no_host));
    }

    #[tokio::test]
    async fn pairing_issues_a_browser_cookie() {
        let (ctx, dir) = build_ctx("cookie");
        let server = WebRemoteServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");

        let client = reqwest::Client::new();
        let started: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/webremote/v1/pair/start"))
            .json(&serde_json::json!({ "clientName": "브라우저" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let pairing_id = started["data"]["pairingId"].as_str().unwrap().to_string();
        let code = ctx
            .pairing
            .list()
            .into_iter()
            .find(|p| p.pairing_id == pairing_id)
            .unwrap()
            .code;
        ctx.pairing.approve(&pairing_id, ClientPermission::Input);

        let resp = client
            .post(format!("http://127.0.0.1:{port}/webremote/v1/pair/complete"))
            .json(&serde_json::json!({ "pairingId": pairing_id, "code": code }))
            .send()
            .await
            .unwrap();
        let cookie = resp
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(cookie.starts_with(WEB_REMOTE_COOKIE_NAME), "쿠키 발급: {cookie}");
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));

        // 그 쿠키만으로 WS가 열려야 한다(브라우저는 헤더를 못 붙인다).
        let token = cookie
            .split(';')
            .next()
            .unwrap()
            .split('=')
            .nth(1)
            .unwrap()
            .to_string();
        let mut request = format!("ws://127.0.0.1:{port}/webremote/v1/ws")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            axum::http::header::COOKIE,
            format!("{WEB_REMOTE_COOKIE_NAME}={token}").parse().unwrap(),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        assert!(matches!(next_msg(&mut socket).await, HostMsg::Hello { .. }));

        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 웹 호스팅(#7m) 실경로 ─────────────────────────────────────────

    /// 브라우저 페어링 → 쿠키 WS → RPC까지. 웹 원격 토글이 켜져 있으면
    /// 내 캐릭터 전부가 보인다(주인 의미론).
    #[tokio::test]
    async fn web_client_pairs_and_drives_rpc_over_the_same_socket() {
        let (ctx, dir) = build_ctx("web-e2e");
        ctx.settings.write().unwrap().web_remote_enabled = true;
        let server = WebRemoteServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");

        let client = reqwest::Client::new();
        let started: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/webremote/v1/pair/start"))
            .json(&serde_json::json!({ "clientName": "휴대폰" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let pairing_id = started["data"]["pairingId"].as_str().unwrap().to_string();
        let pending = ctx
            .pairing
            .list()
            .into_iter()
            .find(|p| p.pairing_id == pairing_id)
            .unwrap();
        ctx.pairing.approve(&pairing_id, ClientPermission::Input);
        let done = client
            .post(format!("http://127.0.0.1:{port}/webremote/v1/pair/complete"))
            .json(&serde_json::json!({ "pairingId": pairing_id, "code": pending.code }))
            .send()
            .await
            .unwrap();
        let cookie = done
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap()
            .to_string();
        let token = cookie.split(';').next().unwrap().split('=').nth(1).unwrap();

        // 쿠키만으로 붙는다(브라우저는 헤더를 못 붙인다).
        let mut request = format!("ws://127.0.0.1:{port}/webremote/v1/ws")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            axum::http::header::COOKIE,
            format!("{WEB_REMOTE_COOKIE_NAME}={token}").parse().unwrap(),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        assert!(matches!(next_msg(&mut socket).await, HostMsg::Hello { .. }));
        // 캐릭터별 공유 토글 없이 내 캐릭터 전부가 보인다(주인 의미론).
        match next_msg(&mut socket).await {
            HostMsg::Agents { agents } => assert_eq!(agents.len(), 1),
            other => panic!("agents가 와야 한다: {other:?}"),
        }

        // 같은 소켓에 RPC를 얹는다 — 새 라우트·새 소켓 없음.
        socket
            .send(TMessage::Text(
                serde_json::to_string(&ClientMsg::Rpc {
                    id: 1,
                    cmd: "agents.list".into(),
                    args: serde_json::json!({}),
                })
                .unwrap(),
            ))
            .await
            .unwrap();
        match next_msg(&mut socket).await {
            HostMsg::RpcResult { id, ok, data, .. } => {
                assert_eq!(id, 1);
                assert!(ok);
                assert_eq!(data.unwrap().as_array().unwrap().len(), 1);
            }
            other => panic!("rpcResult가 와야 한다: {other:?}"),
        }

        // allowlist 밖은 소켓 위에서도 거부된다.
        socket
            .send(TMessage::Text(
                serde_json::to_string(&ClientMsg::Rpc {
                    id: 2,
                    cmd: "set_app_settings".into(),
                    args: serde_json::json!({ "cliEnabled": true }),
                })
                .unwrap(),
            ))
            .await
            .unwrap();
        match next_msg(&mut socket).await {
            HostMsg::RpcResult { id, ok, error, .. } => {
                assert_eq!(id, 2);
                assert!(!ok);
                assert_eq!(error.unwrap().code, "unknownCmd");
            }
            other => panic!("거부가 와야 한다: {other:?}"),
        }

        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 정적 자산은 토글이 꺼져 있으면 404다(서버 재시작 없이 즉시).
    #[tokio::test]
    async fn web_assets_are_gated_by_the_toggle() {
        let (ctx, dir) = build_ctx("web-gate");
        ctx.settings.write().unwrap().web_remote_enabled = false;
        let server = WebRemoteServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");
        let client = reqwest::Client::new();

        let off = client
            .get(format!("http://127.0.0.1:{port}/web/"))
            .send()
            .await
            .unwrap();
        assert_eq!(off.status(), reqwest::StatusCode::NOT_FOUND);

        ctx.settings.write().unwrap().web_remote_enabled = true;
        let on = client
            .get(format!("http://127.0.0.1:{port}/web/"))
            .send()
            .await
            .unwrap();
        // 클라이언트가 빌드돼 있으면 200, 아니면 "빌드되지 않았습니다" 404 —
        // 어느 쪽이든 **토글이 꺼졌을 때와는 다른 응답**이어야 한다.
        assert!(
            on.status().is_success() || on.text().await.unwrap().contains("빌드되지"),
            "토글이 켜지면 게이트가 아니라 자산 유무가 응답을 결정한다"
        );
        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 토글을 끄면 이미 발급된 토큰으로 붙어도 아무 캐릭터도 보이지 않고
    /// attach도 막힌다(매 요청 확인 — 서버 재시작 없이 즉시 차단).
    #[tokio::test]
    async fn turning_the_toggle_off_hides_every_agent() {
        let (ctx, dir) = build_ctx("web-toggle-off");
        let server = WebRemoteServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");
        let token = pair(port, &ctx).await;

        ctx.settings.write().unwrap().web_remote_enabled = false;
        let mut socket = open_ws(port, &token).await;
        let _hello = next_msg(&mut socket).await;
        match next_msg(&mut socket).await {
            HostMsg::Agents { agents } => assert!(agents.is_empty(), "{agents:?}"),
            other => panic!("agents가 와야 한다: {other:?}"),
        }
        socket
            .send(TMessage::Text(
                serde_json::to_string(&ClientMsg::Attach {
                    agent_id: "a1".into(),
                    last_offset: None,
                })
                .unwrap(),
            ))
            .await
            .unwrap();
        match next_msg(&mut socket).await {
            HostMsg::Error { message } => assert!(message.contains("접근할 수 없는")),
            other => panic!("에러가 와야 한다: {other:?}"),
        }
        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn forwarded_agent_extracts_target() {
        let out = HostMsg::Output(RemoteOutput {
            agent_id: "ada".into(),
            session_id: "s".into(),
            seq: 1,
            offset: 0,
            data: "x".into(),
            bytes: 1,
        });
        assert_eq!(forwarded_agent(&out), Some("ada"));
        assert_eq!(forwarded_agent(&HostMsg::Pong), None);
        assert_eq!(
            forwarded_agent(&HostMsg::Agents { agents: vec![] }),
            None
        );
    }
}
