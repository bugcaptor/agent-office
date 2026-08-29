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

/// 채팅 뷰(M2)의 전사 tail·키 매핑.
pub mod chat;
pub mod host;
pub mod pairing;
pub mod protocol;
/// 브라우저 클라이언트용 정적 자산 + allowlist RPC 디스패처.
pub mod rpc;
/// 바인드 주소 정책과 로컬 주소 탐지.
pub mod net;
/// 토큰 제시 경로(헤더·쿠키·서브프로토콜)와 오리진·원격 주소 정책.
mod auth;
/// 페어링 HTTP 핸들러.
mod pair;
/// WS 업그레이드와 한 뷰어 연결의 수명.
mod ws;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use axum::routing::{get, post};
use axum::Router;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::httpapi::{ct_eq, session_state_str, set_owner_only};
use crate::notification::hub::NotificationHub;
use crate::observer::server::ObserverServerState;
use crate::observer::ObserverRuntime;
use crate::persistence::profile_store::ProfileStore;
use crate::persistence::settings_store::{AppSettings, WebRemoteBind};
use crate::session::manager::SessionManager;
use crate::state::SessionRegistry;
use crate::types::SessionState;

use auth::remote_policy;
// 라우터가 이름으로 엮는 핸들러들 — 정의는 도메인별 파일에 있다.
pub use net::{choose_bind_ip, local_addr_hint, local_host_name, local_ip_addrs};
use pair::{pair_complete, pair_start};
use ws::ws_route;

use chat::ChatRegistry;
use host::WebRemoteHub;
use pairing::{ClientRecord, ClientTokenStore, PairingState};
use protocol::*;

/// WS 연결마다 붙는 일련번호. 채팅 구독 수명이 **연결**에 매여 있어서
/// (브라우저를 닫으면 tail이 멈춘다) 클라이언트 토큰이 아니라 연결이 단위다 —
/// 같은 토큰으로 탭을 두 개 열면 각각이 따로 구독한다.
static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);

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

// ── 서버 컨텍스트 ────────────────────────────────────────────────────

/// 페어링 요청이 오면 호스트 렌더러에 승인 다이얼로그를 띄우기 위한 알림.
pub type PairNotifyFn = Arc<dyn Fn(&pairing::PendingPairing) + Send + Sync>;

pub struct WebRemoteContext {
    pub manager: Arc<SessionManager>,
    pub registry: Arc<SessionRegistry>,
    pub store: ProfileStore,
    pub settings: Arc<RwLock<AppSettings>>,
    pub hub: Arc<WebRemoteHub>,
    /// 채팅 뷰의 전사 tail 소유자(M2).
    pub chat: Arc<ChatRegistry>,
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
    /// 커스텀 초상 PNG 저장소(`media.portrait` RPC). 네이티브와 **같은
    /// 인스턴스**를 공유한다 — 디렉터리 규약이 두 곳에 복제되지 않는다.
    pub portraits: Arc<crate::persistence::png_store::PngStore>,
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
    pub portraits: Arc<crate::persistence::png_store::PngStore>,
}

impl WebRemoteContext {
    pub fn new(deps: WebRemoteContextDeps) -> Self {
        let chat = ChatRegistry::new(deps.hub.clone());
        Self {
            manager: deps.manager,
            registry: deps.registry,
            store: deps.store,
            settings: deps.settings,
            hub: deps.hub,
            chat,
            tokens: ClientTokenStore::new(pairing::token_path(&deps.app_data_dir)),
            pairing: Arc::new(PairingState::default()),
            host_name: deps.host_name,
            app_data_dir: deps.app_data_dir,
            hub_notify: deps.hub_notify,
            observer: deps.observer,
            observer_server: deps.observer_server,
            live_usage: deps.live_usage,
            portraits: deps.portraits,
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
                    archetype: p.archetype,
                    portrait_updated_at: p.portrait_updated_at,
                    colors: p.colors,
                }
            })
            .collect()
    }
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

    /// 허용 네트워크·포트 변경 반영용 재기동. `ensure`는 멱등이라 이미 떠 있는
    /// 리스너의 바인드 주소를 바꾸지 못한다 — 먼저 내리고 새 정책으로 다시 연다.
    /// graceful shutdown은 신호 즉시 accept를 멈추고 리스너 소켓을 놓지만, 남은
    /// 연결(웹소켓 tail 등)이 다 빠질 때까지 태스크는 살아 있을 수 있어 짧은
    /// 시한부로만 기다린다(시한을 넘기면 배수는 백그라운드에 맡긴다).
    pub async fn rebind(&self, ctx: Arc<WebRemoteContext>, port: u16) -> Option<u16> {
        let installed = self.installed.lock().unwrap().take();
        if let Some(server) = installed {
            let _ = server.shutdown.send(());
            let _ = tokio::time::timeout(std::time::Duration::from_millis(500), server.handle)
                .await;
        }
        self.ensure(ctx, port).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    // ── 페어링 → WS → 복원/입력까지 실제 소켓으로 태우는 통합 테스트 ──────
    //
    // 와이어 계약(라우트·헤더·메시지 tag/필드 케이스)과 권한 게이트는 문서가
    // 아니라 여기서 지킨다.

    use futures_util::{SinkExt, StreamExt};

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
            legacy_note: None,
            seed: "seed".into(),
            created_at: 1,
            desk_index: 0,
            assigned_desk_index: None,
            cwd: None,
            legacy_appearance: None,
            portrait_request: None,
            portrait_updated_at: None,
            sprite_request: None,
            minimi_request: None,
            sprite_updated_at: None,
            minimi_updated_at: None,
            archetype: None,
            colors: None,
            shell: None,
            startup_command: None,
            personality_prompt: None,
            clocked_out: None,
            keyboard_sound: None,
            voice_id: None,
            bot: None,
            talk_receive: None,
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
            talk_enabled: false,
            talk_max_turns: crate::talk::DEFAULT_MAX_TURNS,
            talk_idle_quiet_ms: crate::talk::DEFAULT_IDLE_QUIET_MS,
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
            portraits: Arc::new(crate::persistence::png_store::PngStore::new(
                dir.join("portraits"),
                crate::persistence::png_store::MAX_PORTRAIT_BYTES,
            )),
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

    /// 허용 네트워크 변경은 rebind로만 반영된다 — ensure는 멱등이라 기존
    /// 리스너를 그대로 두고, rebind는 내렸다가 새 정책의 주소로 다시 연다.
    #[tokio::test]
    async fn rebind_reopens_listener_with_new_bind_policy() {
        let (ctx, dir) = build_ctx("rebind");
        let server = WebRemoteServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");
        let loopback = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
        assert_eq!(server.current_bound().unwrap().ip, loopback);

        ctx.settings.write().unwrap().web_remote_bind = WebRemoteBind::All;
        // ensure만으로는 그대로다(멱등) — 이게 원래 버그의 재현이다.
        assert_eq!(server.ensure(ctx.clone(), 0).await, Some(port));
        assert_eq!(server.current_bound().unwrap().ip, loopback);

        let new_port = server.rebind(ctx.clone(), 0).await.expect("재기동");
        let bound = server.current_bound().unwrap();
        assert_eq!(bound.ip, IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
        assert_eq!(bound.port, new_port);

        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
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
        // 직결(평문)에는 Secure가 없어야 한다 — 붙으면 브라우저가 쿠키를 버린다.
        assert!(!cookie.contains("Secure"), "직결 쿠키에 Secure: {cookie}");

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

    /// tailscale serve 경유(=`X-Forwarded-Proto: https`)로 페어링하면 쿠키에
    /// `Secure`가 붙는다. 헤더 판정이 실제 핸들러까지 배선돼 있는지가 요점이다.
    #[tokio::test]
    async fn pairing_behind_serve_issues_a_secure_cookie() {
        let (ctx, dir) = build_ctx("cookie-secure");
        let server = WebRemoteServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");

        let client = reqwest::Client::new();
        let started: serde_json::Value = client
            .post(format!("http://127.0.0.1:{port}/webremote/v1/pair/start"))
            .json(&serde_json::json!({ "clientName": "폰" }))
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
            .header("X-Forwarded-Proto", "https")
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
        assert!(cookie.contains("; Secure"), "serve 경유 쿠키: {cookie}");

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

    /// 알림은 터미널에 attach 하지 않아도 브라우저에 도착해야 한다(채팅 카드).
    #[tokio::test]
    async fn notifications_reach_a_client_that_never_attached() {
        let (ctx, dir) = build_ctx("web-notify-unattached");
        let server = WebRemoteServerState::default();
        let port = server.ensure(ctx.clone(), 0).await.expect("서버 기동");
        let token = pair(port, &ctx).await;
        let mut socket = open_ws(port, &token).await;
        let _hello = next_msg(&mut socket).await;
        let _agents = next_msg(&mut socket).await;

        // attach 없이 알림을 발행한다.
        let events = crate::webremote::host::WebRemoteEvents::new(ctx.hub.clone());
        // 구독자가 생길 때까지 잠깐 기다린다(WS 태스크가 subscribe 한다).
        for _ in 0..50 {
            if ctx.hub.has_clients() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        crate::state::AppEvents::notification_new(
            &events,
            &crate::types::NotificationEvent {
                id: "n1".into(),
                agent_id: "a1".into(),
                session_id: "s1".into(),
                message: "확인이 필요합니다".into(),
                dedup_key: "k".into(),
                at: 1,
                source: crate::types::NotificationSource::Hook,
            },
        );
        match next_msg(&mut socket).await {
            HostMsg::Notification { agent_id, payload } => {
                assert_eq!(agent_id, "a1");
                assert_eq!(payload["message"], "확인이 필요합니다");
            }
            other => panic!("알림이 와야 한다: {other:?}"),
        }
        server.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
