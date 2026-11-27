// src-tauri/src/link/client.rs
//
// 클라이언트 쪽: 호스트 하나와의 페어링(`link-client.json`, 0600 — 기기 토큰과 고정한 CA), 그리고 호스트
// 웹 상태판에 WebSocket 하나를 유지하는 연결 루프. 끊기면 백오프로 다시 붙고, 끊긴 동안 보낼 것은
// outbox에 쌓는다. 토큰이 거절되면(해제됨) 다시 붙지 않고, 프로토콜이 다르면 길게 쉬며 가끔만 다시 본다
// (호스트가 업데이트되면 저절로 붙게).
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::Connector;

use super::address::{HostAddress, TrustMode};
use super::outbox::Outbox;
use super::protocol::{self, Down, PairRequest, PairResponse, Refusal, Up, PROTOCOL};
use super::{client_tls, now_ms, Changed};

pub const PAIRING_FILE: &str = "link-client.json";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// 호스트가 이만큼 아무 프레임도 보내지 않으면(호스트 ping 20초) 끊긴 것으로 본다.
const SILENCE_LIMIT: Duration = Duration::from_secs(60);
const BACKOFF_BASE: Duration = Duration::from_secs(1);
pub const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// 프로토콜이 달라 거절됐을 때 다시 볼 간격.
const MISMATCH_RETRY: Duration = Duration::from_secs(10 * 60);

/// 호스트가 보낸 일을 받는 쪽(다음 단계들이 건다). 인자: 종류, 데이터.
pub type CommandHandler = Arc<dyn Fn(&str, &serde_json::Value) + Send + Sync>;
/// 호스트의 물음(`Down::Request`)에 답하는 쪽. 인자: 종류, 데이터.
pub type RequestHandler =
    Arc<dyn Fn(String, serde_json::Value) -> futures_util::future::BoxFuture<'static, Result<serde_json::Value, String>> + Send + Sync>;
/// 붙어 있는 동안만 보내는 것(상태 스냅숏·답)의 상한. 넘치면 오래된 것부터 버린다.
const LIVE_MAX: usize = 256;

/// 쌓지 않고 지금 연결로만 보내는 것. 연결이 바뀌면 비운다.
#[derive(Default)]
struct LiveQueue {
    items: Mutex<std::collections::VecDeque<Up>>,
    ready: Notify,
}

impl LiveQueue {
    fn push(&self, up: Up) {
        {
            let mut items = self.items.lock().unwrap();
            if items.len() >= LIVE_MAX {
                items.pop_front();
            }
            items.push_back(up);
        }
        self.ready.notify_one();
    }

    fn take(&self) -> Vec<Up> {
        self.items.lock().unwrap().drain(..).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredPairing {
    pub host_machine_id: String,
    pub host_name: String,
    pub device_id: String,
    pub token: String,
    /// LAN 주소로 페어링했다면 고정한 호스트 CA(PEM).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_pem: Option<String>,
    pub paired_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkClientState {
    /// 페어링이 없거나 주소가 비었다.
    Idle,
    Connecting,
    Connected,
    /// 끊겨서 다시 붙기를 기다린다(`retryAt`).
    Waiting,
    /// 거절됐다(해제·프로토콜 차이·다른 호스트). `error`에 이유.
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkClientStatus {
    pub state: LinkClientState,
    pub paired: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// 지금 쓰는 호스트 주소(정리한 모양).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// LAN 주소로 페어링했다면 고정한 호스트 CA 지문.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_protocol: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_app_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<u64>,
    /// 아직 호스트가 받지 않은 이벤트·보고 수.
    pub pending: usize,
}

#[derive(Default)]
struct Inner {
    enabled: bool,
    address: Option<HostAddress>,
    pairing: Option<StoredPairing>,
    state: Option<LinkClientState>,
    error: Option<String>,
    host_protocol: Option<u32>,
    host_app_version: Option<String>,
    retry_at: Option<u64>,
    connected_at: Option<u64>,
}

/// 한 번의 연결이 끝난 이유.
#[derive(Debug, PartialEq)]
enum End {
    /// 붙었다가 끊겼다.
    Dropped,
    Failed(String),
    Unauthorized,
    Mismatch { host_protocol: u32, host_app_version: String },
    HostChanged,
}

pub struct LinkClient {
    dir: Option<PathBuf>,
    machine_id: String,
    name: String,
    inner: Mutex<Inner>,
    outbox: Mutex<Outbox>,
    /// 설정·페어링이 바뀌었다(지금 연결을 버리고 다시 본다).
    kick: Notify,
    /// outbox에 새 것이 들어왔다.
    queued: Notify,
    on_command: RwLock<Option<CommandHandler>>,
    on_request: RwLock<Option<RequestHandler>>,
    live: Arc<LiveQueue>,
    /// 호스트와 붙은 횟수(welcome마다 1씩). 다시 붙었는지 가르는 데 쓴다.
    connections: std::sync::atomic::AtomicU64,
    changed: Changed,
}

impl LinkClient {
    /// `dir`가 없으면 파일에 쓰지 않는다(시험).
    pub fn new(dir: Option<PathBuf>, machine_id: String, name: String, changed: Changed) -> Self {
        let pairing = dir
            .as_ref()
            .and_then(|d| std::fs::read(d.join(PAIRING_FILE)).ok())
            .and_then(|b| serde_json::from_slice::<StoredPairing>(&b).ok());
        let outbox = Outbox::load(dir.as_ref().map(|d| d.join(super::outbox::FILE)));
        Self {
            dir,
            machine_id,
            name,
            inner: Mutex::new(Inner { pairing, ..Default::default() }),
            outbox: Mutex::new(outbox),
            kick: Notify::new(),
            queued: Notify::new(),
            on_command: RwLock::new(None),
            on_request: RwLock::new(None),
            live: Arc::new(LiveQueue::default()),
            connections: 0.into(),
            changed,
        }
    }

    pub fn set_request_handler(&self, handler: RequestHandler) {
        *self.on_request.write().unwrap() = Some(handler);
    }

    /// 지금 붙어 있으면 이번 연결의 번호(붙을 때마다 바뀐다), 아니면 None.
    pub fn connection(&self) -> Option<u64> {
        let connected = self.inner.lock().unwrap().state == Some(LinkClientState::Connected);
        connected.then(|| self.connections.load(std::sync::atomic::Ordering::SeqCst))
    }

    /// 지금 연결로만 보낸다(쌓지 않음). 붙어 있지 않으면 버리고 false.
    pub fn send_live(&self, kind: &str, data: serde_json::Value) -> bool {
        if self.connection().is_none() {
            return false;
        }
        self.live.push(Up::Live { kind: kind.into(), data });
        true
    }

    pub fn set_command_handler(&self, handler: CommandHandler) {
        *self.on_command.write().unwrap() = Some(handler);
    }

    /// 클라이언트 모드 여부와 호스트 주소. 바뀌면 지금 연결을 버리고 다시 본다.
    pub fn set_target(&self, enabled: bool, address: Option<HostAddress>) {
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.enabled == enabled && inner.address == address {
                return;
            }
            inner.enabled = enabled;
            inner.address = address;
        }
        self.kick.notify_one();
        (self.changed)();
    }

    pub fn status(&self) -> LinkClientStatus {
        let pending = self.outbox.lock().unwrap().len();
        let inner = self.inner.lock().unwrap();
        let state = inner.state.unwrap_or(LinkClientState::Idle);
        LinkClientStatus {
            state,
            paired: inner.pairing.is_some(),
            host_name: inner.pairing.as_ref().map(|p| p.host_name.clone()),
            device_id: inner.pairing.as_ref().map(|p| p.device_id.clone()),
            address: inner.address.as_ref().map(|a| a.display()),
            pinned_fingerprint: inner
                .pairing
                .as_ref()
                .and_then(|p| p.ca_pem.as_deref())
                .and_then(|pem| client_tls::der_from_pem(pem).ok())
                .map(|der| client_tls::fingerprint_der(&der)),
            error: inner.error.clone(),
            host_protocol: inner.host_protocol,
            host_app_version: inner.host_app_version.clone(),
            retry_at: inner.retry_at.filter(|_| state == LinkClientState::Waiting),
            connected_at: inner.connected_at.filter(|_| state == LinkClientState::Connected),
            pending,
        }
    }

    fn set_state(&self, f: impl FnOnce(&mut Inner)) {
        {
            let mut inner = self.inner.lock().unwrap();
            f(&mut inner);
        }
        (self.changed)();
    }

    /// 보낼 것을 쌓는다. 붙어 있으면 바로 나간다. `key`가 같은 안 보낸 것은 새 값으로 바뀐다.
    pub fn enqueue(&self, kind: &str, data: serde_json::Value, key: Option<&str>) {
        self.outbox.lock().unwrap().push(kind, data, key);
        self.queued.notify_one();
        (self.changed)();
    }

    // ── 페어링 ───────────────────────────────────────────────────────

    /// 호스트 주소와 그 호스트가 띄운 코드로 페어링한다. 성공하면 토큰(과 LAN이면 CA)을 저장하고 붙는다.
    pub async fn pair(&self, address: &HostAddress, code: &str) -> Result<StoredPairing, String> {
        let code = protocol::normalize_code(code);
        if code.len() != protocol::CODE_LEN {
            return Err("link-pair-code-invalid".into());
        }
        let trust = address.trust();
        let builder = reqwest::Client::builder().timeout(CONNECT_TIMEOUT).redirect(reqwest::redirect::Policy::none());
        let (client, capture) = match trust {
            TrustMode::PinnedCa => {
                let (config, capture) = client_tls::capture_config();
                (builder.use_preconfigured_tls(config), Some(capture))
            }
            TrustMode::Public => (builder.use_preconfigured_tls(client_tls::public_config()), None),
            TrustMode::Plain => (builder, None),
        };
        let client = client.build().map_err(|e| format!("link-unreachable: {e}"))?;
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        // 증명에 묶을 CA 지문은 TLS 핸드셰이크 뒤에야 안다. 같은 클라이언트로 먼저 한 번 열어 본다.
        let pin = match &capture {
            Some(capture) => {
                client.get(address.http_url("/ca.crt")).send().await.map_err(|e| unreachable(&e))?;
                let ca = capture.seen().ok_or("link-tls-failed")?;
                Some((client_tls::fingerprint_der(&ca), client_tls::pem_from_der(&ca)))
            }
            None => None,
        };
        let pin_fp = pin.as_ref().map(|(fp, _)| fp.clone());
        let req = PairRequest {
            protocol: PROTOCOL,
            app_version: protocol::app_version().into(),
            machine_id: self.machine_id.clone(),
            name: self.name.clone(),
            proof: protocol::client_proof(&code, &nonce, pin_fp.as_deref(), &self.machine_id),
            nonce: nonce.clone(),
            pin: pin_fp.clone(),
        };
        let res = client.post(address.http_url("/link/pair")).json(&req).send().await.map_err(|e| unreachable(&e))?;
        // 다시 열린 연결이 다른 인증서를 냈다면 거기서 멈춘다.
        if let (Some(capture), Some(fp)) = (&capture, &pin_fp) {
            if capture.seen().map(|c| client_tls::fingerprint_der(&c)).as_ref() != Some(fp) {
                return Err("link-tls-pin-mismatch".into());
            }
        }
        let status = res.status();
        let body: serde_json::Value = res.json().await.map_err(|_| format!("link-http-{}", status.as_u16()))?;
        if !status.is_success() {
            let refusal: Refusal = serde_json::from_value(body).unwrap_or_default();
            if refusal.error == "link-protocol-mismatch" {
                self.set_state(|i| {
                    i.host_protocol = refusal.host_protocol;
                    i.host_app_version = refusal.host_app_version.clone();
                });
            }
            return Err(if refusal.error.is_empty() { format!("link-http-{}", status.as_u16()) } else { refusal.error });
        }
        let res: PairResponse = serde_json::from_value(body["data"].clone()).map_err(|_| "link-pair-bad-response".to_string())?;
        let expected = protocol::host_proof(&code, &nonce, pin_fp.as_deref(), &res.device_id);
        if !crate::httpapi::ct_eq(expected.as_bytes(), res.proof.as_bytes()) {
            return Err("link-pair-host-unverified".into());
        }
        if res.host_machine_id == self.machine_id {
            return Err("link-self".into());
        }
        let stored = StoredPairing {
            host_machine_id: res.host_machine_id,
            host_name: res.host_name,
            device_id: res.device_id,
            token: res.token,
            ca_pem: pin.map(|(_, pem)| pem),
            paired_at: now_ms(),
        };
        self.save_pairing(Some(&stored))?;
        let host_changed = {
            let mut inner = self.inner.lock().unwrap();
            let changed = inner.pairing.as_ref().map(|p| &p.host_machine_id) != Some(&stored.host_machine_id);
            inner.pairing = Some(stored.clone());
            inner.address = Some(address.clone());
            (inner.error, inner.host_protocol, inner.host_app_version) = (None, None, None);
            changed
        };
        if host_changed {
            // 다른 호스트에 옛 보고를 보내지 않는다.
            self.outbox.lock().unwrap().reset();
        }
        self.kick.notify_one();
        (self.changed)();
        Ok(stored)
    }

    /// 페어링을 지운다(이 기기에서만 — 호스트 목록에는 남으니 호스트에서도 해제한다).
    pub fn forget(&self) -> Result<(), String> {
        self.save_pairing(None)?;
        self.outbox.lock().unwrap().reset();
        self.set_state(|i| {
            i.pairing = None;
            i.error = None;
            (i.host_protocol, i.host_app_version) = (None, None);
        });
        self.kick.notify_one();
        Ok(())
    }

    fn save_pairing(&self, pairing: Option<&StoredPairing>) -> Result<(), String> {
        let Some(dir) = &self.dir else { return Ok(()) };
        let path = dir.join(PAIRING_FILE);
        match pairing {
            None => match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            },
            Some(p) => {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                let bytes = serde_json::to_vec_pretty(p).map_err(|e| e.to_string())?;
                std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
                crate::httpapi::set_owner_only(&path);
                Ok(())
            }
        }
    }

    // ── 연결 루프 ────────────────────────────────────────────────────

    /// 앱 수명 동안 한 번 띄운다.
    pub async fn run(self: Arc<Self>) {
        let mut attempt: u32 = 0;
        loop {
            let target = {
                let inner = self.inner.lock().unwrap();
                match (inner.enabled, inner.address.clone(), inner.pairing.clone()) {
                    (true, Some(a), Some(p)) => Some((a, p)),
                    _ => None,
                }
            };
            let Some((address, pairing)) = target else {
                self.set_state(|i| i.state = Some(LinkClientState::Idle));
                self.kick.notified().await;
                attempt = 0;
                continue;
            };
            self.set_state(|i| i.state = Some(LinkClientState::Connecting));
            let end = tokio::select! {
                end = self.session(&address, &pairing) => end,
                _ = self.kick.notified() => {
                    attempt = 0;
                    continue;
                }
            };
            let wait = match end {
                End::Unauthorized => {
                    self.set_state(|i| {
                        i.state = Some(LinkClientState::Rejected);
                        i.error = Some("link-unauthorized".into());
                    });
                    None
                }
                End::HostChanged => {
                    self.set_state(|i| {
                        i.state = Some(LinkClientState::Rejected);
                        i.error = Some("link-host-changed".into());
                    });
                    None
                }
                End::Mismatch { host_protocol, host_app_version } => {
                    self.set_state(|i| {
                        i.state = Some(LinkClientState::Rejected);
                        i.error = Some("link-protocol-mismatch".into());
                        i.host_protocol = Some(host_protocol);
                        i.host_app_version = Some(host_app_version);
                    });
                    Some(MISMATCH_RETRY)
                }
                End::Dropped => {
                    attempt = 0;
                    Some(self.wait_retry(attempt, "link-disconnected".into()))
                }
                End::Failed(code) => {
                    let delay = self.wait_retry(attempt, code);
                    attempt = attempt.saturating_add(1);
                    Some(delay)
                }
            };
            match wait {
                Some(delay) => {
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = self.kick.notified() => attempt = 0,
                    }
                }
                None => {
                    self.kick.notified().await;
                    attempt = 0;
                }
            }
        }
    }

    fn wait_retry(&self, attempt: u32, error: String) -> Duration {
        let jitter = (uuid::Uuid::new_v4().as_bytes()[0] as f64) / 256.0;
        let delay = backoff(attempt, jitter);
        self.set_state(|i| {
            i.state = Some(LinkClientState::Waiting);
            i.error = Some(error);
            i.retry_at = Some(now_ms() + delay.as_millis() as u64);
        });
        delay
    }

    async fn session(&self, address: &HostAddress, pairing: &StoredPairing) -> End {
        let connector = match address.trust() {
            TrustMode::Plain => Connector::Plain,
            TrustMode::Public => Connector::Rustls(Arc::new(client_tls::public_config())),
            TrustMode::PinnedCa => match pairing.ca_pem.as_deref().map(client_tls::pinned_config) {
                Some(Ok(config)) => Connector::Rustls(Arc::new(config)),
                // 이름 주소로 페어링해 고정한 CA가 없는데 지금 IP로 붙으려 한다: 다시 페어링해야 한다.
                _ => return End::Failed("link-tls-not-pinned".into()),
            },
        };
        let mut request = match address.ws_url("/link/ws").into_client_request() {
            Ok(r) => r,
            Err(_) => return End::Failed("link-address-invalid".into()),
        };
        if let Ok(v) = HeaderValue::from_str(&format!("Bearer {}", pairing.token)) {
            request.headers_mut().insert("authorization", v);
        }
        let connected = tokio::time::timeout(
            CONNECT_TIMEOUT,
            tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector)),
        )
        .await;
        let mut ws = match connected {
            Err(_) => return End::Failed("link-unreachable: timeout".into()),
            Ok(Err(tokio_tungstenite::tungstenite::Error::Http(res))) => {
                return match res.status().as_u16() {
                    401 => End::Unauthorized,
                    404 => End::Failed("link-host-off".into()),
                    code => End::Failed(format!("link-http-{code}")),
                }
            }
            Ok(Err(tokio_tungstenite::tungstenite::Error::Tls(e))) => return End::Failed(format!("link-tls-failed: {e}")),
            Ok(Err(e)) => return End::Failed(format!("link-unreachable: {e}")),
            Ok(Ok((ws, _))) => ws,
        };
        let epoch = self.outbox.lock().unwrap().epoch().to_string();
        let hello = Up::Hello {
            protocol: PROTOCOL,
            app_version: protocol::app_version().into(),
            machine_id: self.machine_id.clone(),
            name: self.name.clone(),
            outbox_epoch: epoch,
        };
        if ws.send(Message::Text(serde_json::to_string(&hello).unwrap_or_default())).await.is_err() {
            return End::Failed("link-unreachable: closed".into());
        }
        // 첫 답: welcome이나 reject.
        let first = tokio::time::timeout(CONNECT_TIMEOUT, async {
            while let Some(Ok(msg)) = ws.next().await {
                if let Message::Text(text) = msg {
                    return serde_json::from_str::<Down>(&text).ok();
                }
            }
            None
        })
        .await;
        match first {
            Ok(Some(Down::Welcome { host_machine_id, acked, host_app_version, host_name, .. })) => {
                if host_machine_id != pairing.host_machine_id {
                    let _ = ws.close(None).await;
                    return End::HostChanged;
                }
                self.outbox.lock().unwrap().reconnected(acked);
                // 지난 연결에 보내려던 스냅숏·답은 이제 의미가 없다.
                self.live.take();
                self.connections.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                self.set_state(|i| {
                    i.state = Some(LinkClientState::Connected);
                    i.error = None;
                    i.connected_at = Some(now_ms());
                    i.host_protocol = Some(PROTOCOL);
                    i.host_app_version = Some(host_app_version);
                    if let Some(p) = i.pairing.as_mut() {
                        p.host_name = host_name;
                    }
                });
            }
            Ok(Some(Down::Reject { host_protocol, host_app_version, .. })) => {
                return End::Mismatch { host_protocol, host_app_version };
            }
            Ok(Some(Down::Revoked)) => return End::Unauthorized,
            _ => return End::Failed("link-handshake-failed".into()),
        }
        let mut heard = Instant::now();
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            // 쌓인 것을 먼저 비운다.
            let unsent = self.outbox.lock().unwrap().take_unsent();
            for item in unsent {
                let up = Up::Event { seq: item.seq, kind: item.kind, data: item.data };
                if ws.send(Message::Text(serde_json::to_string(&up).unwrap_or_default())).await.is_err() {
                    return End::Dropped;
                }
            }
            for up in self.live.take() {
                if ws.send(Message::Text(serde_json::to_string(&up).unwrap_or_default())).await.is_err() {
                    return End::Dropped;
                }
            }
            tokio::select! {
                msg = ws.next() => {
                    let Some(Ok(msg)) = msg else { return End::Dropped };
                    heard = Instant::now();
                    if let Message::Text(text) = msg {
                        match serde_json::from_str::<Down>(&text) {
                            Ok(Down::Ack { seq }) => {
                                self.outbox.lock().unwrap().ack(seq);
                                (self.changed)();
                            }
                            Ok(Down::Command { kind, data }) => {
                                let handler = self.on_command.read().unwrap().clone();
                                if let Some(h) = handler {
                                    h(&kind, &data);
                                }
                            }
                            Ok(Down::Request { id, kind, data }) => self.answer(id, kind, data),
                            Ok(Down::Revoked) => return End::Unauthorized,
                            _ => {}
                        }
                    }
                }
                _ = self.queued.notified() => {}
                _ = self.live.ready.notified() => {}
                _ = tick.tick() => {
                    if heard.elapsed() > SILENCE_LIMIT {
                        return End::Dropped;
                    }
                }
            }
        }
    }
}

impl LinkClient {
    /// 호스트의 물음에 답한다. 처리기는 따로 돌리고 답은 지금 연결로 보낸다(그 사이 끊기면 버린다).
    fn answer(&self, id: u64, kind: String, data: serde_json::Value) {
        let handler = self.on_request.read().unwrap().clone();
        let live = self.live.clone();
        let Some(handler) = handler else {
            live.push(Up::Reply { id, data: serde_json::Value::Null, error: Some("link-unsupported".into()) });
            return;
        };
        tokio::spawn(async move {
            let up = match handler(kind, data).await {
                Ok(data) => Up::Reply { id, data, error: None },
                Err(e) => Up::Reply { id, data: serde_json::Value::Null, error: Some(e) },
            };
            live.push(up);
        });
    }
}

fn unreachable(e: &reqwest::Error) -> String {
    format!("link-unreachable: {e}")
}

/// 다시 붙기 전 기다릴 시간: 1·2·4…초, 60초 상한, 최대 25% 흔들기(`jitter` 0..1).
pub fn backoff(attempt: u32, jitter: f64) -> Duration {
    let base = BACKOFF_BASE.saturating_mul(1u32 << attempt.min(16)).min(BACKOFF_MAX);
    base + base.mul_f64(0.25 * jitter.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_to_a_cap_with_bounded_jitter() {
        assert_eq!(backoff(0, 0.0), Duration::from_secs(1));
        assert_eq!(backoff(3, 0.0), Duration::from_secs(8));
        assert_eq!(backoff(10, 0.0), BACKOFF_MAX);
        assert_eq!(backoff(u32::MAX, 0.0), BACKOFF_MAX);
        assert_eq!(backoff(2, 1.0), Duration::from_secs(5));
    }
}
