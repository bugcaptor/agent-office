// src-tauri/src/link/host.rs
//
// 호스트 쪽: 페어링 코드, 페어링된 기기 목록(`link-devices.json`, 0600 — 토큰은 SHA-256만 둔다),
// 붙어 있는 기기의 WebSocket. 리스너는 따로 열지 않고 웹 상태판 서버(webdash/)의 `/link/pair`·
// `/link/ws`로 받는다. 호스트는 클라이언트로 연결을 걸지 않는다.
//
// 같은 기계(machineId)가 다시 페어링하면 기기 id는 그대로 두고 토큰만 바꾼다 — 끊었다 붙거나
// 다시 페어링해도 같은 기기로 본다.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::protocol::{self, Down, PairRequest, PairResponse, Refusal, Up, PROTOCOL};
use super::{now_ms, Changed};

pub const DEVICES_FILE: &str = "link-devices.json";
/// 페어링 코드가 유효한 시간.
pub const CODE_TTL: Duration = Duration::from_secs(5 * 60);
/// 코드 하나에 허용하는 틀린 시도. 넘으면 코드를 버린다.
const MAX_FAILURES: u32 = 5;
/// 연결 뒤 hello를 기다리는 시간.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// 호스트가 ping을 보내는 간격. 클라이언트가 이만큼의 몇 배 동안 말이 없으면 끊는다.
pub const PING_INTERVAL: Duration = Duration::from_secs(20);
pub const DEAD_AFTER: Duration = Duration::from_secs(60);
/// 클라이언트에 물은 것(`request`)의 기본 기다림.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// 받을 메시지 하나의 상한(대화 로그 백필·이미지 한 장이 한 프레임에 온다).
const MAX_MESSAGE: usize = 64 * 1024 * 1024;

type Pending = tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>;

/// 붙은 기기가 보낸 이벤트를 받는 쪽(다음 단계들이 건다). 인자: 기기 id, 종류, 데이터.
pub type EventHandler = Arc<dyn Fn(&str, &str, &serde_json::Value) + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceRecord {
    id: String,
    machine_id: String,
    name: String,
    token_sha256: String,
    paired_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_seen_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    app_version: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DevicesFile {
    #[serde(default)]
    devices: Vec<DeviceRecord>,
}

/// 설정 화면이 보는 기기 하나.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkDevice {
    pub id: String,
    pub name: String,
    pub machine_id: String,
    pub paired_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<u64>,
    /// 붙어 있는 연결의 상대 주소.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkPairing {
    pub code: String,
    /// 만료(epoch ms).
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkHostStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pairing: Option<LinkPairing>,
    pub devices: Vec<LinkDevice>,
}

struct Pairing {
    code: String,
    created: Instant,
    expires_at: u64,
    failures: u32,
}

struct Conn {
    conn_id: u64,
    since: u64,
    address: Option<String>,
    tx: mpsc::UnboundedSender<Down>,
}

pub struct LinkHost {
    path: Option<PathBuf>,
    machine_id: String,
    host_name: Arc<dyn Fn() -> String + Send + Sync>,
    enabled: std::sync::atomic::AtomicBool,
    devices: Mutex<Vec<DeviceRecord>>,
    pairing: Mutex<Option<Pairing>>,
    conns: Mutex<HashMap<String, Conn>>,
    /// 기기별 (outbox epoch, 받은 마지막 seq). 메모리에만 둔다.
    seqs: Mutex<HashMap<String, (String, u64)>>,
    next_conn: std::sync::atomic::AtomicU64,
    /// 답을 기다리는 물음(id → (기기, 받을 곳)).
    pending: Mutex<HashMap<u64, (String, Pending)>>,
    next_request: std::sync::atomic::AtomicU64,
    on_event: RwLock<Option<EventHandler>>,
    changed: Changed,
}

type Refused = (StatusCode, Refusal);

fn refuse(status: StatusCode, error: &str) -> Refused {
    (status, Refusal { error: error.into(), ..Default::default() })
}

impl LinkHost {
    pub fn new(
        path: Option<PathBuf>,
        machine_id: String,
        host_name: Arc<dyn Fn() -> String + Send + Sync>,
        changed: Changed,
    ) -> Self {
        let devices = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<DevicesFile>(&b).ok())
            .unwrap_or_default()
            .devices;
        Self {
            path,
            machine_id,
            host_name,
            enabled: false.into(),
            devices: Mutex::new(devices),
            pairing: Mutex::new(None),
            conns: Mutex::new(HashMap::new()),
            seqs: Mutex::new(HashMap::new()),
            next_conn: 1.into(),
            pending: Mutex::new(HashMap::new()),
            next_request: 1.into(),
            on_event: RwLock::new(None),
            changed,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// 호스트 모드를 켜거나 끈다. 끄면 코드를 버리고 붙은 기기를 모두 끊는다.
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, std::sync::atomic::Ordering::SeqCst);
        if !on {
            *self.pairing.lock().unwrap() = None;
            // 송신기를 버리면 연결 태스크가 닫고 나간다.
            self.conns.lock().unwrap().clear();
        }
        (self.changed)();
    }

    /// 붙은 기기를 모두 끊는다(웹 상태판 서버가 내려갈 때). WebSocket은 업그레이드 뒤 리스너와 따로 돌아
    /// 리스너를 닫아도 남기 때문이다. 기기는 다시 붙는다.
    pub fn disconnect_all(&self) {
        let had = {
            let mut conns = self.conns.lock().unwrap();
            let had = !conns.is_empty();
            conns.clear();
            had
        };
        if had {
            (self.changed)();
        }
    }

    pub fn set_event_handler(&self, handler: EventHandler) {
        *self.on_event.write().unwrap() = Some(handler);
    }

    fn save(&self, devices: &[DeviceRecord]) {
        let Some(path) = &self.path else { return };
        let file = DevicesFile { devices: devices.to_vec() };
        if let Ok(bytes) = serde_json::to_vec_pretty(&file) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                crate::httpapi::set_owner_only(&tmp);
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    // ── 페어링 ───────────────────────────────────────────────────────

    /// 새 코드를 띄운다(있던 코드는 버린다).
    pub fn start_pairing(&self) -> Result<LinkPairing, String> {
        if !self.enabled() {
            return Err("link-not-host".into());
        }
        let code = protocol::new_code();
        let expires_at = now_ms() + CODE_TTL.as_millis() as u64;
        *self.pairing.lock().unwrap() = Some(Pairing { code: code.clone(), created: Instant::now(), expires_at, failures: 0 });
        (self.changed)();
        Ok(LinkPairing { code, expires_at })
    }

    pub fn cancel_pairing(&self) {
        *self.pairing.lock().unwrap() = None;
        (self.changed)();
    }

    /// 페어링 요청을 검사하고 토큰을 발급한다. `own_pin`은 이 호스트의 LAN CA 지문.
    pub async fn pair(&self, req: PairRequest, own_pin: Option<String>) -> Result<PairResponse, Refused> {
        if !self.enabled() {
            return Err(refuse(StatusCode::NOT_FOUND, "link-not-host"));
        }
        if req.protocol != PROTOCOL {
            return Err((
                StatusCode::CONFLICT,
                Refusal {
                    error: "link-protocol-mismatch".into(),
                    host_protocol: Some(PROTOCOL),
                    host_app_version: Some(protocol::app_version().into()),
                },
            ));
        }
        let machine_id = req.machine_id.trim().to_string();
        let name = req.name.trim().chars().take(64).collect::<String>();
        if machine_id.is_empty() || machine_id.len() > 128 || req.nonce.len() > 128 {
            return Err(refuse(StatusCode::BAD_REQUEST, "link-pair-bad-request"));
        }
        if machine_id == self.machine_id {
            return Err(refuse(StatusCode::BAD_REQUEST, "link-self"));
        }
        let code = {
            let mut pairing = self.pairing.lock().unwrap();
            let Some(p) = pairing.as_mut().filter(|p| p.created.elapsed() < CODE_TTL) else {
                *pairing = None;
                return Err(refuse(StatusCode::FORBIDDEN, "link-pair-no-code"));
            };
            let expected = protocol::client_proof(&p.code, &req.nonce, req.pin.as_deref(), &machine_id);
            if !crate::httpapi::ct_eq(expected.as_bytes(), req.proof.as_bytes()) {
                p.failures += 1;
                if p.failures >= MAX_FAILURES {
                    *pairing = None;
                    drop(pairing);
                    (self.changed)();
                }
                return Err(refuse(StatusCode::FORBIDDEN, "link-pair-code-invalid"));
            }
            // 코드는 맞는데 본 인증서가 이 호스트 것이 아니다 — 가운데서 누가 끼어들었다. 코드를 버린다.
            if req.pin.is_some() && req.pin != own_pin {
                *pairing = None;
                drop(pairing);
                (self.changed)();
                return Err(refuse(StatusCode::FORBIDDEN, "link-tls-pin-mismatch"));
            }
            let code = p.code.clone();
            *pairing = None;
            code
        };
        let token = protocol::new_token();
        let device_id = {
            let mut devices = self.devices.lock().unwrap();
            let id = match devices.iter_mut().find(|d| d.machine_id == machine_id) {
                Some(d) => {
                    d.token_sha256 = protocol::sha256_hex(&token);
                    d.name = name.clone();
                    d.app_version = Some(req.app_version.clone());
                    d.id.clone()
                }
                None => {
                    let id = format!("d-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
                    devices.push(DeviceRecord {
                        id: id.clone(),
                        machine_id: machine_id.clone(),
                        name: name.clone(),
                        token_sha256: protocol::sha256_hex(&token),
                        paired_at: now_ms(),
                        last_seen_at: None,
                        app_version: Some(req.app_version.clone()),
                    });
                    id
                }
            };
            self.save(&devices);
            id
        };
        // 옛 토큰으로 붙어 있던 연결은 끊는다.
        self.conns.lock().unwrap().remove(&device_id);
        (self.changed)();
        let host_name = {
            let f = self.host_name.clone();
            tokio::task::spawn_blocking(move || f()).await.unwrap_or_default()
        };
        Ok(PairResponse {
            proof: protocol::host_proof(&code, &req.nonce, req.pin.as_deref(), &device_id),
            device_id,
            token,
            host_name,
            host_machine_id: self.machine_id.clone(),
            host_app_version: protocol::app_version().into(),
            protocol: PROTOCOL,
        })
    }

    /// HTTP 응답으로(라우트가 부른다).
    pub async fn pair_response(&self, req: PairRequest, own_pin: Option<String>) -> Response {
        match self.pair(req, own_pin).await {
            Ok(res) => crate::httpapi::ok(res).into_response(),
            Err((status, refusal)) => (status, refusal_body(&refusal)).into_response(),
        }
    }

    /// 기기를 해제한다. 붙어 있으면 알리고 끊는다.
    pub fn revoke(&self, device_id: &str) -> bool {
        let removed = {
            let mut devices = self.devices.lock().unwrap();
            let before = devices.len();
            devices.retain(|d| d.id != device_id);
            let removed = devices.len() != before;
            if removed {
                self.save(&devices);
            }
            removed
        };
        if let Some(conn) = self.conns.lock().unwrap().remove(device_id) {
            let _ = conn.tx.send(Down::Revoked);
        }
        self.seqs.lock().unwrap().remove(device_id);
        (self.changed)();
        removed
    }

    fn device_by_token(&self, token: &str) -> Option<String> {
        let hash = protocol::sha256_hex(token);
        self.devices
            .lock()
            .unwrap()
            .iter()
            .find(|d| crate::httpapi::ct_eq(d.token_sha256.as_bytes(), hash.as_bytes()))
            .map(|d| d.id.clone())
    }

    pub fn status(&self) -> LinkHostStatus {
        let pairing = {
            let mut p = self.pairing.lock().unwrap();
            if p.as_ref().is_some_and(|p| p.created.elapsed() >= CODE_TTL) {
                *p = None;
            }
            p.as_ref().map(|p| LinkPairing { code: p.code.clone(), expires_at: p.expires_at })
        };
        let conns = self.conns.lock().unwrap();
        let devices = self
            .devices
            .lock()
            .unwrap()
            .iter()
            .map(|d| {
                let conn = conns.get(&d.id);
                LinkDevice {
                    id: d.id.clone(),
                    name: d.name.clone(),
                    machine_id: d.machine_id.clone(),
                    paired_at: d.paired_at,
                    last_seen_at: d.last_seen_at,
                    app_version: d.app_version.clone(),
                    connected: conn.is_some(),
                    connected_at: conn.map(|c| c.since),
                    address: conn.and_then(|c| c.address.clone()),
                }
            })
            .collect();
        LinkHostStatus { pairing, devices }
    }

    pub fn is_connected(&self, device_id: &str) -> bool {
        self.conns.lock().unwrap().contains_key(device_id)
    }

    /// 붙어 있는 기기에 묻고 답을 기다린다. 끊겨 있거나 `timeout` 안에 답이 없으면 실패한다.
    pub async fn request(
        &self,
        device_id: &str,
        kind: &str,
        data: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, String> {
        let id = self.next_request.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.lock().unwrap().insert(id, (device_id.to_string(), tx));
        let sent = {
            let conns = self.conns.lock().unwrap();
            conns.get(device_id).is_some_and(|c| c.tx.send(Down::Request { id, kind: kind.into(), data }).is_ok())
        };
        if !sent {
            self.pending.lock().unwrap().remove(&id);
            return Err("link-device-offline".into());
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(answer)) => answer,
            // 연결이 끊겨 받을 곳을 버렸다.
            Ok(Err(_)) => Err("link-device-offline".into()),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err("link-request-timeout".into())
            }
        }
    }

    /// 끊긴 기기에 걸린 물음을 모두 실패로 끝낸다.
    fn fail_pending(&self, device_id: &str) {
        let dropped: Vec<Pending> = {
            let mut pending = self.pending.lock().unwrap();
            let ids: Vec<u64> = pending.iter().filter(|(_, (d, _))| d == device_id).map(|(id, _)| *id).collect();
            ids.into_iter().filter_map(|id| pending.remove(&id)).map(|(_, tx)| tx).collect()
        };
        for tx in dropped {
            let _ = tx.send(Err("link-device-offline".into()));
        }
    }

    /// 붙어 있는 기기에 일을 보낸다. 끊겨 있으면 실패한다(호스트 쪽은 쌓아 두지 않는다).
    pub fn send(&self, device_id: &str, kind: &str, data: serde_json::Value) -> Result<(), String> {
        let conns = self.conns.lock().unwrap();
        let conn = conns.get(device_id).ok_or("link-device-offline")?;
        conn.tx.send(Down::Command { kind: kind.into(), data }).map_err(|_| "link-device-offline".to_string())
    }

    // ── WebSocket ────────────────────────────────────────────────────

    /// `/link/ws`. 기기 토큰이 맞을 때만 올린다.
    pub fn upgrade(self: Arc<Self>, ws: WebSocketUpgrade, token: Option<String>, peer: Option<SocketAddr>) -> Response {
        if !self.enabled() {
            return (StatusCode::NOT_FOUND, refusal_body(&Refusal { error: "link-not-host".into(), ..Default::default() })).into_response();
        }
        let Some(device_id) = token.as_deref().and_then(|t| self.device_by_token(t)) else {
            return (StatusCode::UNAUTHORIZED, refusal_body(&Refusal { error: "link-unauthorized".into(), ..Default::default() }))
                .into_response();
        };
        ws.max_message_size(MAX_MESSAGE)
            .max_frame_size(MAX_MESSAGE)
            .on_upgrade(move |socket| async move { self.run_socket(socket, device_id, peer).await })
    }

    async fn run_socket(self: Arc<Self>, socket: WebSocket, device_id: String, peer: Option<SocketAddr>) {
        let (mut sink, mut stream) = socket.split();
        let hello = tokio::time::timeout(HELLO_TIMEOUT, async {
            while let Some(Ok(msg)) = stream.next().await {
                if let Message::Text(text) = msg {
                    return serde_json::from_str::<Up>(&text).ok();
                }
            }
            None
        })
        .await;
        let Ok(Some(Up::Hello { protocol: their_protocol, app_version, machine_id: _, name, outbox_epoch })) = hello else {
            let _ = sink.close().await;
            return;
        };
        if their_protocol != PROTOCOL {
            let reject = Down::Reject {
                error: "link-protocol-mismatch".into(),
                host_protocol: PROTOCOL,
                host_app_version: protocol::app_version().into(),
            };
            let _ = send(&mut sink, &reject).await;
            let _ = sink.close().await;
            return;
        }
        // 해제됐는데 늦게 올라온 연결.
        let still_paired = {
            let mut devices = self.devices.lock().unwrap();
            let found = devices.iter_mut().find(|d| d.id == device_id);
            let ok = found.is_some();
            if let Some(d) = found {
                let name = name.trim().chars().take(64).collect::<String>();
                if !name.is_empty() {
                    d.name = name;
                }
                d.app_version = Some(app_version);
                d.last_seen_at = Some(now_ms());
            }
            if ok {
                self.save(&devices);
            }
            ok
        };
        if !still_paired {
            let _ = send(&mut sink, &Down::Revoked).await;
            let _ = sink.close().await;
            return;
        }
        let (tx, mut rx) = mpsc::unbounded_channel::<Down>();
        let conn_id = self.next_conn.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // 같은 기기의 옛 연결은 송신기를 버려 닫는다.
        self.conns.lock().unwrap().insert(
            device_id.clone(),
            Conn { conn_id, since: now_ms(), address: peer.map(|p| p.ip().to_string()), tx },
        );
        let acked = {
            let mut seqs = self.seqs.lock().unwrap();
            match seqs.get(&device_id) {
                Some((epoch, last)) if *epoch == outbox_epoch => *last,
                _ => {
                    seqs.insert(device_id.clone(), (outbox_epoch.clone(), 0));
                    0
                }
            }
        };
        (self.changed)();
        let host_name = {
            let f = self.host_name.clone();
            tokio::task::spawn_blocking(move || f()).await.unwrap_or_default()
        };
        let welcome = Down::Welcome {
            protocol: PROTOCOL,
            host_app_version: protocol::app_version().into(),
            host_name,
            host_machine_id: self.machine_id.clone(),
            device_id: device_id.clone(),
            acked,
        };
        if send(&mut sink, &welcome).await.is_ok() {
            self.pump(&mut sink, &mut stream, &mut rx, &device_id, &outbox_epoch).await;
        }
        let _ = sink.close().await;
        let mine = {
            let mut conns = self.conns.lock().unwrap();
            let mine = conns.get(&device_id).is_some_and(|c| c.conn_id == conn_id);
            if mine {
                conns.remove(&device_id);
            }
            mine
        };
        if mine {
            let mut devices = self.devices.lock().unwrap();
            if let Some(d) = devices.iter_mut().find(|d| d.id == device_id) {
                d.last_seen_at = Some(now_ms());
                self.save(&devices);
            }
        }
        if mine || !self.is_connected(&device_id) {
            self.fail_pending(&device_id);
        }
        (self.changed)();
    }

    async fn pump(
        &self,
        sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
        stream: &mut futures_util::stream::SplitStream<WebSocket>,
        rx: &mut mpsc::UnboundedReceiver<Down>,
        device_id: &str,
        epoch: &str,
    ) {
        let mut ping = tokio::time::interval(PING_INTERVAL);
        ping.tick().await;
        let mut heard = Instant::now();
        loop {
            tokio::select! {
                msg = stream.next() => {
                    let Some(Ok(msg)) = msg else { return };
                    heard = Instant::now();
                    match msg {
                        Message::Text(text) => match serde_json::from_str::<Up>(&text) {
                            Ok(Up::Live { kind, data }) => {
                                let handler = self.on_event.read().unwrap().clone();
                                if let Some(h) = handler {
                                    h(device_id, &kind, &data);
                                }
                            }
                            Ok(Up::Reply { id, data, error }) => {
                                let waiter = {
                                    let mut pending = self.pending.lock().unwrap();
                                    // 다른 기기가 남의 물음에 답하지 못하게 기기까지 맞춘다.
                                    match pending.get(&id) {
                                        Some((d, _)) if d == device_id => pending.remove(&id).map(|(_, tx)| tx),
                                        _ => None,
                                    }
                                };
                                if let Some(tx) = waiter {
                                    let _ = tx.send(match error {
                                        Some(e) => Err(e),
                                        None => Ok(data),
                                    });
                                }
                            }
                            Ok(Up::Event { seq, kind, data }) => {
                                let fresh = {
                                    let mut seqs = self.seqs.lock().unwrap();
                                    let entry = seqs.entry(device_id.to_string()).or_insert_with(|| (epoch.to_string(), 0));
                                    let fresh = seq > entry.1;
                                    if fresh {
                                        entry.1 = seq;
                                    }
                                    fresh
                                };
                                if fresh {
                                    let handler = self.on_event.read().unwrap().clone();
                                    if let Some(h) = handler {
                                        h(device_id, &kind, &data);
                                    }
                                }
                                if send(sink, &Down::Ack { seq }).await.is_err() {
                                    return;
                                }
                            }
                            _ => {}
                        },
                        Message::Close(_) => return,
                        _ => {}
                    }
                }
                out = rx.recv() => {
                    let Some(down) = out else { return };
                    let revoked = matches!(down, Down::Revoked);
                    if send(sink, &down).await.is_err() || revoked {
                        return;
                    }
                }
                _ = ping.tick() => {
                    if heard.elapsed() > DEAD_AFTER {
                        return;
                    }
                    if sink.send(Message::Ping(Vec::new())).await.is_err() {
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
impl LinkHost {
    /// 시험: 페어링을 거치지 않고 기기 하나를 넣는다. 기기 id를 돌려준다.
    pub(crate) fn add_test_device(&self, machine_id: &str, name: &str) -> String {
        let id = format!("d-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        self.devices.lock().unwrap().push(DeviceRecord {
            id: id.clone(),
            machine_id: machine_id.into(),
            name: name.into(),
            token_sha256: String::new(),
            paired_at: now_ms(),
            last_seen_at: None,
            app_version: None,
        });
        id
    }

    /// 시험: 붙어 있는 것처럼 둔다(보낸 것은 받는 쪽 `rx`로 나온다).
    pub(crate) fn set_test_connected(&self, device_id: &str, on: bool) -> Option<mpsc::UnboundedReceiver<Down>> {
        let mut conns = self.conns.lock().unwrap();
        if !on {
            conns.remove(device_id);
            return None;
        }
        let (tx, rx) = mpsc::unbounded_channel();
        conns.insert(device_id.into(), Conn { conn_id: 0, since: now_ms(), address: None, tx });
        Some(rx)
    }

    /// 시험: 클라이언트가 `Reply`를 보낸 것처럼 받는다.
    pub(crate) fn test_reply(&self, device_id: &str, id: u64, answer: Result<serde_json::Value, String>) {
        let waiter = {
            let mut pending = self.pending.lock().unwrap();
            match pending.get(&id) {
                Some((d, _)) if d == device_id => pending.remove(&id).map(|(_, tx)| tx),
                _ => None,
            }
        };
        if let Some(tx) = waiter {
            let _ = tx.send(answer);
        }
    }
}

async fn send(sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, down: &Down) -> Result<(), axum::Error> {
    let text = serde_json::to_string(down).unwrap_or_default();
    sink.send(Message::Text(text)).await
}

fn refusal_body(refusal: &Refusal) -> Json<serde_json::Value> {
    let mut v = serde_json::to_value(refusal).unwrap_or_default();
    if let Some(obj) = v.as_object_mut() {
        obj.insert("ok".into(), false.into());
    }
    Json(v)
}

/// 이 호스트의 LAN CA 지문을 돌려주는 함수(웹 상태판이 LAN HTTPS를 열지 않았으면 None).
pub type PinSource = Arc<dyn Fn() -> Option<String> + Send + Sync>;

/// 웹 상태판 서버에 붙일 연동 라우트. 웹 상태판 토큰이 아니라 페어링 코드·기기 토큰으로 지킨다.
pub fn router(host: Arc<LinkHost>, pin: PinSource) -> axum::Router {
    use axum::extract::{ConnectInfo, State};
    use axum::http::{header, HeaderMap};
    use axum::routing::{get, post};

    type St = State<(Arc<LinkHost>, PinSource)>;
    async fn pair(State((host, pin)): St, Json(req): Json<PairRequest>) -> Response {
        host.pair_response(req, pin()).await
    }
    async fn ws(
        State((host, _)): St,
        peer: Option<ConnectInfo<SocketAddr>>,
        headers: HeaderMap,
        upgrade: WebSocketUpgrade,
    ) -> Response {
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|t| t.trim().to_string());
        host.upgrade(upgrade, token, peer.map(|ConnectInfo(p)| p))
    }
    axum::Router::new().route("/link/pair", post(pair)).route("/link/ws", get(ws)).with_state((host, pin))
}
