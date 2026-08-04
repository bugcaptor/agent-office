// src-tauri/src/peer/mod.rs
//
// 피어 세션 공유(kbm #7k, docs/peer-session-share-design.md).
//
// 호스트(A)가 소유·실행 중인 에이전트 세션을 같은 네트워크의 뷰어(B)에서 보고
// 입력한다. 프로세스·PTY·observer 훅은 전부 호스트에 남고, 건너가는 것은
// ①출력 바이트 ②화면 스냅샷 ③앱 이벤트 ④입력 넷뿐이다.
//
// 왜 sessiond가 아니라 앱↔앱인가(§결정 1): 뷰어가 필요한 것은 PTY 바이트만이
// 아니라 observer가 파생시킨 캐릭터 상태·알림·프로필인데 그건 앱에만 있다.
// 게다가 sessiond는 unix 전용·기본 off·세션당 data conn 1개라 팬아웃 자체가
// 불가능하다. 팬아웃을 앱(OutputSink tap)에서 하면 그 제약을 건드리지 않는다.
//
// control 서버와의 관계: 2단계 옵트인·토큰 파일·상수시간 비교 **패턴만**
// 재사용하고 리스너와 Router는 분리한다. control은 127.0.0.1 전용이며
// `settings/set`·`create`처럼 네트워크에 내놓으면 안 되는 라우트를 가진다 —
// peer Router에는 그런 라우트가 아예 존재하지 않는다(권한 축소를 구조로 보장).

pub mod host;
pub mod pairing;
pub mod protocol;
pub mod viewer;
/// 웹 호스팅(#7m) — 브라우저 클라이언트용 정적 자산 + allowlist RPC 디스패처.
pub mod web;

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
use crate::persistence::settings_store::{AppSettings, PeerBind};
use crate::session::manager::SessionManager;
use crate::state::SessionRegistry;
use crate::types::SessionState;

use host::PeerHub;
use pairing::{PairingOutcome, PairingState, PeerRecord, PeerTokenStore};
use protocol::*;

/// 공유 대상 캐릭터 목록(호스트). 프로필과 분리해 두어 peer 기능이 캐릭터
/// 스키마를 건드리지 않게 한다.
pub const PEER_SHARED_FILE: &str = "peer-shared.json";

/// 브라우저 클라이언트(웹 호스팅 #7m)가 WS 업그레이드에 실어 보낼 인증 쿠키
/// 이름. 브라우저의 WebSocket API는 커스텀 헤더를 붙일 수 없으므로 헤더 인증만
/// 두면 브라우저는 아예 붙지 못한다.
pub const PEER_COOKIE_NAME: &str = "ao_peer_token";

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
pub fn bind_policy_allows(bind: PeerBind, ip: IpAddr) -> bool {
    match bind {
        PeerBind::Tailnet => is_tailnet_addr(ip) || is_loopback(ip),
        PeerBind::All => true,
        PeerBind::Loopback => is_loopback(ip),
    }
}

// ── 공유 대상 저장소 ──────────────────────────────────────────────────

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct SharedFile {
    #[serde(default)]
    agents: Vec<String>,
}

pub struct PeerShareStore {
    path: PathBuf,
}

impl PeerShareStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn load(&self) -> Vec<String> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        serde_json::from_str::<SharedFile>(&text)
            .map(|f| f.agents)
            .unwrap_or_default()
    }
    pub fn save(&self, agents: &[String]) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&SharedFile {
            agents: agents.to_vec(),
        })
        .unwrap_or_else(|_| "{}".into());
        std::fs::write(&self.path, text)
    }
}

// ── 서버 컨텍스트 ────────────────────────────────────────────────────

/// 페어링 요청이 오면 호스트 렌더러에 승인 다이얼로그를 띄우기 위한 알림.
pub type PairNotifyFn = Arc<dyn Fn(&pairing::PendingPairing) + Send + Sync>;

pub struct PeerContext {
    pub manager: Arc<SessionManager>,
    pub registry: Arc<SessionRegistry>,
    pub store: ProfileStore,
    pub settings: Arc<RwLock<AppSettings>>,
    pub hub: Arc<PeerHub>,
    pub tokens: PeerTokenStore,
    pub shares: PeerShareStore,
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

/// `PeerContext::new`의 인자 묶음 — 필드가 늘어도 호출부가 위치 인자 나열로
/// 무너지지 않게 한다.
pub struct PeerContextDeps {
    pub manager: Arc<SessionManager>,
    pub registry: Arc<SessionRegistry>,
    pub store: ProfileStore,
    pub settings: Arc<RwLock<AppSettings>>,
    pub hub: Arc<PeerHub>,
    pub app_data_dir: PathBuf,
    pub host_name: String,
    pub hub_notify: Arc<NotificationHub>,
    pub observer: Arc<ObserverRuntime>,
    pub observer_server: Arc<ObserverServerState>,
    pub live_usage: Arc<crate::usage::LiveUsageState>,
}

impl PeerContext {
    pub fn new(deps: PeerContextDeps) -> Self {
        Self {
            manager: deps.manager,
            registry: deps.registry,
            store: deps.store,
            settings: deps.settings,
            hub: deps.hub,
            tokens: PeerTokenStore::new(pairing::token_path(&deps.app_data_dir)),
            shares: PeerShareStore::new(deps.app_data_dir.join(PEER_SHARED_FILE)),
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
    pub fn web_hosting_enabled(&self) -> bool {
        self.settings
            .read()
            .map(|s| s.web_hosting_enabled)
            .unwrap_or(false)
    }

    /// 이 클라이언트가 그 캐릭터를 볼 수 있는가.
    ///
    /// - **브라우저**(`Web`): 내 기계를 내가 조종하는 것이므로 **내 캐릭터 전부**.
    ///   단 웹 호스팅 토글이 꺼져 있으면 아무것도 못 본다.
    /// - **앱↔앱 뷰어**(`Peer`): 손님이므로 **캐릭터별 공유 토글을 켠 것만**
    ///   (#7k의 기존 의미론 — 회귀 금지).
    ///
    /// **주의**: 앱↔앱 판정은 `hub.is_shared`(=tap 설치 여부)가 아니라 영속
    /// 공유 목록을 본다. 웹 클라이언트가 attach 하면서 tap을 깔면 그 캐릭터가
    /// 앱↔앱 뷰어에게도 보이게 되는 누출이 생기기 때문이다 — tap은 "출력을
    /// 받아 적는 중"일 뿐 "공유하기로 했다"가 아니다.
    pub fn agent_allowed(&self, record: &PeerRecord, agent_id: &str) -> bool {
        if record.kind.is_web() {
            return self.web_hosting_enabled()
                && self.store.load().agents.iter().any(|a| a.id == agent_id);
        }
        self.shares.load().iter().any(|a| a == agent_id)
    }

    /// 그 클라이언트에게 보여줄 캐릭터 목록(가시성 규칙은 `agent_allowed`와 동일).
    pub fn build_agents_for(&self, record: &PeerRecord) -> Vec<PeerAgent> {
        if record.kind.is_web() {
            if !self.web_hosting_enabled() {
                return Vec::new();
            }
            return self.agents_from(|_| true);
        }
        let shared: std::collections::HashSet<String> = self.shares.load().into_iter().collect();
        self.agents_from(move |id| shared.contains(id))
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

    fn bind_policy(&self) -> PeerBind {
        self.settings
            .read()
            .map(|s| s.peer_bind)
            .unwrap_or_default()
    }

    /// 저장된 공유 목록대로 tap을 설치한다(부팅 시 1회, 토글 시 재적용).
    pub fn apply_shares(self: &Arc<Self>) {
        let wanted: std::collections::HashSet<String> =
            self.shares.load().into_iter().collect();
        for agent in self.hub.shared_agents() {
            if !wanted.contains(&agent) {
                self.hub.unshare(&self.manager, &agent);
            }
        }
        for agent in wanted {
            self.hub.share(&self.manager, &agent);
        }
    }

    /// 캐릭터 공유 토글. 저장 + tap 설치/해제 + 뷰어에 목록 재송.
    pub fn set_shared(self: &Arc<Self>, agent_id: &str, shared: bool) -> Result<(), String> {
        if protocol::is_remote_agent(agent_id) {
            // 받은 세션을 다시 남에게 넘기는 체이닝은 v1 비목표다.
            return Err("원격 캐릭터는 다시 공유할 수 없습니다".into());
        }
        let mut agents = self.shares.load();
        agents.retain(|a| a != agent_id);
        if shared {
            agents.push(agent_id.to_string());
        }
        agents.sort();
        self.shares.save(&agents).map_err(|e| e.to_string())?;
        if shared {
            self.hub.share(&self.manager, agent_id);
        } else if !self.web_hosting_enabled() {
            // 웹 호스팅이 켜져 있으면 tap을 남겨 둔다 — 브라우저는 공유 토글과
            // 무관하게 어느 캐릭터에나 붙을 수 있으므로, 여기서 tap을 떼면
            // 붙어 있던 브라우저의 출력이 조용히 멎는다. 가시성은 이미 영속
            // 공유 목록으로 판정하므로(agent_allowed) 남은 tap이 앱↔앱 뷰어에게
            // 이 캐릭터를 노출시키지는 않는다.
            self.hub.unshare(&self.manager, agent_id);
        }
        self.hub.broadcast(HostMsg::Agents {
            agents: self.build_agents(),
        });
        Ok(())
    }

    /// 공유 중인 캐릭터의 뷰어용 메타(앱↔앱 뷰어 기준). 프로필(호스트 소유) +
    /// 레지스트리 상태 병합. 브로드캐스트용이라 가장 좁은 가시성을 쓴다.
    pub fn build_agents(&self) -> Vec<PeerAgent> {
        let shared: std::collections::HashSet<String> = self.shares.load().into_iter().collect();
        self.agents_from(move |id| shared.contains(id))
    }

    /// 프로필 + 실행 상태를 병합해 `PeerAgent`를 만든다. 어떤 캐릭터를 담을지는
    /// 호출자가 준 술어가 정한다(가시성 규칙의 단일 구현 지점).
    fn agents_from(&self, keep: impl Fn(&str) -> bool) -> Vec<PeerAgent> {
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
                PeerAgent {
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
    State(ctx): State<Arc<PeerContext>>,
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
    State(ctx): State<Arc<PeerContext>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<PairStartRequest>,
) -> Response {
    // 브라우저 클라이언트가 생기면서 페어링 표면이 커졌다 — 시작 자체를
    // IP별로 제한하지 않으면 "새 페어링을 계속 열어 코드를 무한 시도"가 된다.
    if !ctx.rate.allow_start(addr.ip()) {
        eprintln!("peer: pair/start 레이트리밋 초과 from {}", addr.ip());
        return (
            StatusCode::TOO_MANY_REQUESTS,
            fail("요청이 너무 잦습니다. 잠시 후 다시 시도하세요"),
        )
            .into_response();
    }
    if req.client_kind.is_web() && !ctx.web_hosting_enabled() {
        return (
            StatusCode::FORBIDDEN,
            fail("웹 호스팅이 꺼져 있습니다"),
        )
            .into_response();
    }
    let name = if req.viewer_name.trim().is_empty() {
        "이름 없는 손님".to_string()
    } else {
        req.viewer_name.trim().chars().take(60).collect()
    };
    let Some(pending) = ctx.pairing.start(&name, req.client_kind) else {
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
        proto_version: PEER_PROTO_VERSION,
    })
    .into_response()
}

async fn pair_complete(
    State(ctx): State<Arc<PeerContext>>,
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
    let client_kind = ctx
        .pairing
        .list()
        .into_iter()
        .find(|p| p.pairing_id == req.pairing_id)
        .map(|p| p.client_kind)
        .unwrap_or_default();
    match ctx.pairing.complete(&req.pairing_id, &req.code) {
        PairingOutcome::Approved(permission) => {
            let record = PeerRecord {
                peer_id: pairing::new_peer_id(),
                name: ctx
                    .pairing
                    .list()
                    .into_iter()
                    .find(|p| p.pairing_id == req.pairing_id)
                    .map(|p| p.viewer_name)
                    .unwrap_or_else(|| "뷰어".into()),
                token: pairing::new_token(),
                permission,
                created_at: pairing::now_ms(),
                kind: client_kind,
            };
            if let Err(e) = ctx.tokens.insert(record.clone()) {
                return fail(format!("토큰 저장 실패: {e}")).into_response();
            }
            let token = record.token.clone();
            let mut resp = ok(PairCompleteResponse {
                peer_token: record.token,
                peer_id: record.peer_id,
                host_name: ctx.host_name.clone(),
                permission,
                proto_version: PEER_PROTO_VERSION,
            })
            .into_response();
            // 브라우저 클라이언트(#7m)용 인증 쿠키. 앱↔앱 클라이언트는 본문의
            // 토큰만 쓰고 이 헤더를 무시하므로 양쪽에 무해하다. `Secure`는
            // https일 때만 의미가 있어 붙이지 않는다(v1은 tailnet 평문 전제 —
            // tailscale serve로 https를 씌우는 경로는 #7m §E 참고).
            if let Ok(value) = format!(
                "{PEER_COOKIE_NAME}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
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
            eprintln!("peer: 페어링 코드 불일치 from {}", addr.ip());
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
/// 1. `X-Agent-Office-Peer-Token` 헤더 — 앱↔앱(뷰어) 경로.
/// 2. `Cookie: ao_peer_token=…` — **브라우저 경로**. 브라우저의 WebSocket API는
///    커스텀 헤더를 붙일 수 없어서(웹 호스팅 #7m §D) 헤더만 보면 브라우저는
///    아예 붙지 못한다. 페어링 완료 시 HttpOnly 쿠키를 발급해 두면 업그레이드에
///    자동으로 동반된다.
/// 3. `Sec-WebSocket-Protocol: agent-office.token.<token>` — 쿠키를 못 쓰는
///    상황(교차 오리진 등)의 표준 관용 우회. 서버는 고른 서브프로토콜을 응답에
///    그대로 echo 해야 하므로 `ws.protocols(...)`로 되돌려준다.
fn presented_token(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get(PEER_TOKEN_HEADER).and_then(|v| v.to_str().ok()) {
        return Some(v.to_string());
    }
    if let Some(cookie) = headers.get(axum::http::header::COOKIE).and_then(|v| v.to_str().ok()) {
        for part in cookie.split(';') {
            if let Some(value) = part.trim().strip_prefix(&format!("{PEER_COOKIE_NAME}=")) {
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

fn authenticate(ctx: &PeerContext, headers: &HeaderMap) -> Option<PeerRecord> {
    ctx.tokens.authenticate(&presented_token(headers)?)
}

/// 브라우저 클라이언트가 생기는 순간 필요한 방어(#7m §D). `Origin`이 없으면
/// 브라우저가 아니다(앱↔앱) → 통과. 있으면 우리가 서빙하는 것과 같은 오리진만
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
    State(ctx): State<Arc<PeerContext>>,
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
        eprintln!("peer: WS 인증 실패 from {}", addr.ip());
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
async fn serve_ws(socket: WebSocket, ctx: Arc<PeerContext>, peer: PeerRecord) {
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
            proto_version: PEER_PROTO_VERSION,
            permission: peer.permission,
            peer_id: peer.peer_id.clone(),
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
            agents: ctx.build_agents_for(&peer),
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
                        let Ok(msg) = serde_json::from_str::<ViewerMsg>(&text) else {
                            let _ = send_msg(&mut sink, &HostMsg::Error {
                                message: "알 수 없는 메시지".into(),
                            }).await;
                            continue;
                        };
                        if handle_viewer_msg(&mut sink, &ctx, &peer, &mut attached, msg).await.is_err() {
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
                        let _ = send_msg(&mut sink, &HostMsg::Agents { agents: ctx.build_agents_for(&peer) }).await;
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

async fn handle_viewer_msg(
    sink: &mut WsSink,
    ctx: &Arc<PeerContext>,
    peer: &PeerRecord,
    attached: &mut HashMap<String, u64>,
    msg: ViewerMsg,
) -> Result<(), ()> {
    match msg {
        ViewerMsg::Ping => send_msg(sink, &HostMsg::Pong).await,
        ViewerMsg::Detach { agent_id } => {
            attached.remove(&agent_id);
            Ok(())
        }
        ViewerMsg::Attach {
            agent_id,
            last_offset,
        } => {
            if !ctx.agent_allowed(peer, &agent_id) {
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
        ViewerMsg::Input { agent_id, data } => {
            if !peer.permission.allows_input() {
                return send_msg(
                    sink,
                    &HostMsg::Error {
                        message: "읽기 전용으로 연결되어 입력할 수 없습니다".into(),
                    },
                )
                .await;
            }
            if !ctx.agent_allowed(peer, &agent_id) {
                return Ok(());
            }
            ctx.manager.write_input(&agent_id, &data);
            Ok(())
        }
        ViewerMsg::Rpc { id, cmd, args } => {
            let result = web::dispatch(ctx, peer, &cmd, args).await;
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
    ctx: &Arc<PeerContext>,
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
            &HostMsg::Output(PeerOutput {
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

fn router(ctx: Arc<PeerContext>) -> Router {
    Router::new()
        .route("/peer/v1/pair/start", post(pair_start))
        .route("/peer/v1/pair/complete", post(pair_complete))
        .route("/peer/v1/ws", get(ws_route))
        // 웹 호스팅(#7m): 같은 리스너에 라우트를 얹는다 — 별도 포트·프로세스 없음.
        .merge(web::routes())
        .layer(axum::middleware::from_fn_with_state(
            ctx.clone(),
            remote_policy,
        ))
        .with_state(ctx)
}

async fn serve(
    ctx: Arc<PeerContext>,
    port: u16,
    shutdown_rx: oneshot::Receiver<()>,
) -> std::io::Result<(u16, JoinHandle<()>)> {
    let bind_ip = match ctx.bind_policy() {
        PeerBind::Loopback => "127.0.0.1",
        _ => "0.0.0.0",
    };
    // 고정 포트가 점유돼 있으면 몇 칸 스캔한다(실제 포트는 설정 UI에 표시).
    let mut listener = None;
    for candidate in port..port.saturating_add(8) {
        match tokio::net::TcpListener::bind((bind_ip, candidate)).await {
            Ok(l) => {
                listener = Some(l);
                break;
            }
            Err(_) => continue,
        }
    }
    let listener = match listener {
        Some(l) => l,
        None => tokio::net::TcpListener::bind((bind_ip, 0)).await?,
    };
    let bound = listener.local_addr()?.port();
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

struct InstalledServer {
    port: u16,
    shutdown: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

#[derive(Default)]
pub struct PeerServerState {
    start_gate: tokio::sync::Mutex<()>,
    installed: Mutex<Option<InstalledServer>>,
}

impl PeerServerState {
    pub fn current_port(&self) -> Option<u16> {
        self.installed.lock().unwrap().as_ref().map(|s| s.port)
    }

    pub fn is_running(&self) -> bool {
        self.installed.lock().unwrap().is_some()
    }

    /// opt-in 기동(멱등). 실패해도 GUI 기능에는 영향이 없다(fail-open).
    pub async fn ensure(&self, ctx: Arc<PeerContext>, port: u16) -> Option<u16> {
        let _gate = self.start_gate.lock().await;
        if let Some(p) = self.current_port() {
            return Some(p);
        }
        let (shutdown, rx) = oneshot::channel();
        match serve(ctx, port, rx).await {
            Ok((bound, handle)) => {
                *self.installed.lock().unwrap() = Some(InstalledServer {
                    port: bound,
                    shutdown,
                    handle,
                });
                Some(bound)
            }
            Err(e) => {
                eprintln!("peer server unavailable: {e}");
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

/// 이 머신의 tailnet 주소(뷰어에게 불러 줄 주소를 설정 UI에 보여주기 위함).
/// 인터페이스 열거 없이 UDP 소켓의 로컬 주소를 캐내는 고전적 방법이다 —
/// 실제 패킷은 나가지 않는다.
pub fn local_addr_hint() -> Option<String> {
    let sock = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    // tailnet 게이트웨이 대역으로 "연결"해 보면 tailscale 인터페이스의 주소가 잡힌다.
    sock.connect(("100.100.100.100", 80))
        .or_else(|_| sock.connect(("8.8.8.8", 80)))
        .ok()?;
    sock.local_addr().ok().map(|a| a.ip().to_string())
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
    fn tailnet_policy_rejects_plain_lan_but_allows_loopback() {
        let lan = IpAddr::V4(Ipv4Addr::new(192, 168, 0, 5));
        let tail = IpAddr::V4(Ipv4Addr::new(100, 64, 1, 2));
        let local = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert!(!bind_policy_allows(PeerBind::Tailnet, lan));
        assert!(bind_policy_allows(PeerBind::Tailnet, tail));
        assert!(bind_policy_allows(PeerBind::Tailnet, local));

        assert!(bind_policy_allows(PeerBind::All, lan));
        assert!(!bind_policy_allows(PeerBind::Loopback, lan));
        assert!(!bind_policy_allows(PeerBind::Loopback, tail));
        assert!(bind_policy_allows(PeerBind::Loopback, local));
    }

    #[test]
    fn share_store_roundtrip() {
        let dir = std::env::temp_dir().join(format!("peer-share-{}", uuid::Uuid::new_v4()));
        let store = PeerShareStore::new(dir.join(PEER_SHARED_FILE));
        assert!(store.load().is_empty());
        store.save(&["a".into(), "b".into()]).unwrap();
        assert_eq!(store.load(), vec!["a".to_string(), "b".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 페어링 → WS → 복원/입력까지 실제 소켓으로 태우는 통합 테스트 ──────
    //
    // 와이어 계약(라우트·헤더·메시지 tag/필드 케이스)과 권한 게이트는 문서가
    // 아니라 여기서 지킨다.

    use crate::notification::hub::{NotificationHub, SystemClock};
    use crate::session::pty_factory::fake::FakePtyFactory;
    use crate::state::fake::RecordingEvents;
    use crate::state::AppEvents;
    use futures_util::{SinkExt as _, StreamExt as _};
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

    /// pub(crate): 웹 RPC 테스트(`peer::web::tests`)가 같은 픽스처를 쓴다.
    pub(crate) fn build_ctx(tag: &str) -> (Arc<PeerContext>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("peer-it-{tag}-{}", uuid::Uuid::new_v4()));
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
            peer_bind: PeerBind::Loopback,
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
        let ctx = Arc::new(PeerContext::new(PeerContextDeps {
            manager,
            registry,
            store,
            settings,
            hub: PeerHub::new(),
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
    async fn pair(port: u16, ctx: &Arc<PeerContext>) -> String {
        let client = reqwest::Client::new();
        let started: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/peer/v1/pair/start"))
            .json(&serde_json::json!({ "viewerName": "테스트뷰어" }))
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
            .post(format!("http://127.0.0.1:{port}/peer/v1/pair/complete"))
            .json(&serde_json::json!({ "pairingId": pairing_id, "code": code }))
            .send()
            .await
            .unwrap();
        assert_eq!(waiting.status(), reqwest::StatusCode::ACCEPTED);

        assert!(ctx.pairing.approve(&pairing_id, PeerPermission::Input));
        let done: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/peer/v1/pair/complete"))
            .json(&serde_json::json!({ "pairingId": pairing_id, "code": code }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(done["ok"], true);
        done["data"]["peerToken"].as_str().unwrap().to_string()
    }

    async fn open_ws(
        port: u16,
        token: &str,
    ) -> tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    > {
        let mut request = format!("ws://127.0.0.1:{port}/peer/v1/ws")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert(PEER_TOKEN_HEADER, token.parse().unwrap());
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
        let server = PeerServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");
        let token = pair(port, &ctx).await;

        ctx.set_shared("a1", true).unwrap();
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
                assert_eq!(permission, PeerPermission::Input);
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
                serde_json::to_string(&ViewerMsg::Attach {
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

    #[tokio::test]
    async fn unshared_agent_cannot_be_attached() {
        let (ctx, dir) = build_ctx("unshared");
        let server = PeerServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");
        let token = pair(port, &ctx).await;
        let mut socket = open_ws(port, &token).await;
        let _hello = next_msg(&mut socket).await;
        let _agents = next_msg(&mut socket).await;

        socket
            .send(TMessage::Text(
                serde_json::to_string(&ViewerMsg::Attach {
                    agent_id: "a1".into(), // 공유 토글을 켜지 않았다
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
    async fn read_only_peer_is_refused_input() {
        let (ctx, dir) = build_ctx("readonly");
        let server = PeerServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");
        let token = pair(port, &ctx).await;
        // 발급 후 권한을 읽기 전용으로 낮춘다(설정 UI의 권한 변경과 같은 경로).
        ctx.tokens
            .set_permission(
                &ctx.tokens.load()[0].peer_id.clone(),
                PeerPermission::ReadOnly,
            )
            .unwrap();
        ctx.set_shared("a1", true).unwrap();

        let mut socket = open_ws(port, &token).await;
        match next_msg(&mut socket).await {
            HostMsg::Hello { permission, .. } => {
                assert_eq!(permission, PeerPermission::ReadOnly)
            }
            other => panic!("hello가 먼저여야 한다: {other:?}"),
        }
        let _agents = next_msg(&mut socket).await;
        socket
            .send(TMessage::Text(
                serde_json::to_string(&ViewerMsg::Input {
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
        let server = PeerServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");
        let request = format!("ws://127.0.0.1:{port}/peer/v1/ws")
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
        let server = PeerServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");
        let token = pair(port, &ctx).await;
        // 한 번은 붙는다.
        let mut socket = open_ws(port, &token).await;
        let _ = next_msg(&mut socket).await;
        drop(socket);
        // 승인 취소 후에는 같은 토큰이 막힌다(매 연결 파일 대조).
        let peer_id = ctx.tokens.load()[0].peer_id.clone();
        ctx.tokens.remove(&peer_id).unwrap();
        let mut request = format!("ws://127.0.0.1:{port}/peer/v1/ws")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert(PEER_TOKEN_HEADER, token.parse().unwrap());
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

        headers.insert(PEER_TOKEN_HEADER, "from-header".parse().unwrap());
        assert_eq!(presented_token(&headers).as_deref(), Some("from-header"));

        let mut cookies = HeaderMap::new();
        cookies.insert(
            axum::http::header::COOKIE,
            format!("other=1; {PEER_COOKIE_NAME}=from-cookie; x=2")
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
            format!("{PEER_COOKIE_NAME}=").parse().unwrap(),
        );
        assert_eq!(presented_token(&empty), None);
    }

    #[test]
    fn origin_is_only_checked_for_browsers() {
        // Origin 없음 = 앱↔앱 → 통과.
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
        let server = PeerServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");

        let client = reqwest::Client::new();
        let started: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/peer/v1/pair/start"))
            .json(&serde_json::json!({ "viewerName": "브라우저" }))
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
        ctx.pairing.approve(&pairing_id, PeerPermission::Input);

        let resp = client
            .post(format!("http://127.0.0.1:{port}/peer/v1/pair/complete"))
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
        assert!(cookie.starts_with(PEER_COOKIE_NAME), "쿠키 발급: {cookie}");
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
        let mut request = format!("ws://127.0.0.1:{port}/peer/v1/ws")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            axum::http::header::COOKIE,
            format!("{PEER_COOKIE_NAME}={token}").parse().unwrap(),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        assert!(matches!(next_msg(&mut socket).await, HostMsg::Hello { .. }));

        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 웹 호스팅(#7m) 실경로 ─────────────────────────────────────────

    /// 브라우저 페어링 → 쿠키 WS → RPC까지. `clientKind: "web"`이면 공유
    /// 토글과 무관하게 내 캐릭터 전부가 보인다(주인 의미론).
    #[tokio::test]
    async fn web_client_pairs_and_drives_rpc_over_the_same_socket() {
        let (ctx, dir) = build_ctx("web-e2e");
        ctx.settings.write().unwrap().web_hosting_enabled = true;
        let server = PeerServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");

        let client = reqwest::Client::new();
        let started: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/peer/v1/pair/start"))
            .json(&serde_json::json!({ "viewerName": "휴대폰", "clientKind": "web" }))
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
        assert_eq!(
            pending.client_kind,
            PeerClientKind::Web,
            "승인 다이얼로그가 브라우저임을 알아야 한다"
        );
        ctx.pairing.approve(&pairing_id, PeerPermission::Input);
        let done = client
            .post(format!("http://127.0.0.1:{port}/peer/v1/pair/complete"))
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
        let mut request = format!("ws://127.0.0.1:{port}/peer/v1/ws")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            axum::http::header::COOKIE,
            format!("{PEER_COOKIE_NAME}={token}").parse().unwrap(),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        assert!(matches!(next_msg(&mut socket).await, HostMsg::Hello { .. }));
        // 공유 토글을 켠 적이 없는데도 캐릭터가 보인다(주인 의미론).
        match next_msg(&mut socket).await {
            HostMsg::Agents { agents } => assert_eq!(agents.len(), 1),
            other => panic!("agents가 와야 한다: {other:?}"),
        }

        // 같은 소켓에 RPC를 얹는다 — 새 라우트·새 소켓 없음.
        socket
            .send(TMessage::Text(
                serde_json::to_string(&ViewerMsg::Rpc {
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
                serde_json::to_string(&ViewerMsg::Rpc {
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
        let server = PeerServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");
        let client = reqwest::Client::new();

        let off = client
            .get(format!("http://127.0.0.1:{port}/web/"))
            .send()
            .await
            .unwrap();
        assert_eq!(off.status(), reqwest::StatusCode::NOT_FOUND);

        ctx.settings.write().unwrap().web_hosting_enabled = true;
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

    /// 브라우저 클라이언트가 생겨도 앱↔앱 뷰어의 가시성은 그대로여야 한다.
    #[tokio::test]
    async fn peer_client_visibility_is_unchanged_by_web_hosting() {
        let (ctx, dir) = build_ctx("web-regression");
        ctx.settings.write().unwrap().web_hosting_enabled = true;
        let server = PeerServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");
        let token = pair(port, &ctx).await; // clientKind 미지정 = peer

        let mut socket = open_ws(port, &token).await;
        let _hello = next_msg(&mut socket).await;
        match next_msg(&mut socket).await {
            // 공유 토글을 켠 적이 없으므로 손님에게는 아무것도 보이면 안 된다.
            HostMsg::Agents { agents } => assert!(agents.is_empty(), "{agents:?}"),
            other => panic!("agents가 와야 한다: {other:?}"),
        }
        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn forwarded_agent_extracts_target() {
        let out = HostMsg::Output(PeerOutput {
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
