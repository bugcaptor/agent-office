// src-tauri/src/peer/viewer.rs
//
// 뷰어 쪽(#7k §결정 3). 원격 세션은 "스폰"이 아니라 "이미 있는 세션에 붙기"라
// `PtyFactory` 심을 재사용하지 않는다 — SessionManager의 호스트 전용 부작용
// (셸 해석·래퍼 스크립트 생성·env 주입·세션 로그·cleanup·waiter)을 전부 플래그로
// 우회해야 하기 때문이다. 대신 여기 얇은 레지스트리를 두고 **`OutputSink`는
// 그대로 재사용**한다: `subscribe_output`/`write_input` 커맨드가 `peer:` 접두사를
// 보고 이쪽으로 라우팅하면 렌더러·터미널·알림 파이프라인은 무수정으로 돈다.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tokio::sync::mpsc;

use crate::session::output::OutputSink;
use crate::types::OutputChunk;

use super::pairing::{PeerHostRecord, PeerHostStore};
use super::protocol::*;

const RECONNECT_MIN: Duration = Duration::from_secs(1);
const RECONNECT_MAX: Duration = Duration::from_secs(30);
const VIEWER_PING_EVERY: Duration = Duration::from_secs(20);

/// 뷰어가 렌더러로 이벤트를 내보내는 경계(Tauri 비의존 — 테스트 주입점).
pub trait ViewerEvents: Send + Sync {
    fn emit(&self, event: &str, payload: serde_json::Value);
}

/// 아무 데도 내보내지 않는 기본 구현(테스트/부트 이전).
pub struct NullViewerEvents;
impl ViewerEvents for NullViewerEvents {
    fn emit(&self, _event: &str, _payload: serde_json::Value) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PeerConnState {
    Connecting,
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerStatus {
    pub peer_id: String,
    pub label: String,
    pub address: String,
    pub state: PeerConnState,
    pub permission: PeerPermission,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 이 피어가 공유 중인 캐릭터(키는 `peer:<peerId>:<agentId>`).
    pub agents: Vec<serde_json::Value>,
}

struct PeerConn {
    record: PeerHostRecord,
    state: PeerConnState,
    error: Option<String>,
    /// WS 쓰기 태스크로 보내는 큐. 연결이 끊긴 동안은 None.
    tx: Option<mpsc::UnboundedSender<ViewerMsg>>,
    /// 이 피어가 알려온 캐릭터 목록(원본 agentId 기준).
    agents: Vec<PeerAgent>,
    permission: PeerPermission,
    stop: Arc<AtomicBool>,
}

/// 원격 세션 레지스트리. sink 맵 + 피어 연결 맵이 전부다(상태 기계 없음).
pub struct ViewerRegistry {
    sinks: Mutex<HashMap<String, Arc<OutputSink>>>,
    /// 렌더러가 구독 중이라 재연결 시 다시 attach 해야 하는 키.
    wanted: Mutex<HashSet<String>>,
    /// 키별 마지막으로 적용한 절대 오프셋(재접속 델타 기준점).
    offsets: Mutex<HashMap<String, u64>>,
    conns: Mutex<HashMap<String, PeerConn>>,
    events: Mutex<Arc<dyn ViewerEvents>>,
    hosts: PeerHostStore,
    viewer_name: String,
}

impl ViewerRegistry {
    pub fn new(hosts: PeerHostStore, viewer_name: String) -> Arc<Self> {
        Arc::new(Self {
            sinks: Mutex::new(HashMap::new()),
            wanted: Mutex::new(HashSet::new()),
            offsets: Mutex::new(HashMap::new()),
            conns: Mutex::new(HashMap::new()),
            events: Mutex::new(Arc::new(NullViewerEvents)),
            hosts,
            viewer_name,
        })
    }

    pub fn set_events(&self, events: Arc<dyn ViewerEvents>) {
        *self.events.lock() = events;
    }

    fn emit(&self, event: &str, payload: serde_json::Value) {
        let events = self.events.lock().clone();
        events.emit(event, payload);
    }

    fn sink_for(&self, key: &str) -> Arc<OutputSink> {
        self.sinks
            .lock()
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(OutputSink::new()))
            .clone()
    }

    // ── 렌더러 커맨드가 라우팅해 오는 표면 ──────────────────────────

    /// `subscribe_output`의 원격 갈래. 채널을 붙이고 호스트에 attach를 건다.
    pub fn attach_output(&self, key: &str, channel: Channel<OutputChunk>) {
        self.sink_for(key).attach(channel);
        self.wanted.lock().insert(key.to_string());
        self.send_attach(key);
    }

    pub fn detach_output(&self, key: &str) {
        if let Some(sink) = self.sinks.lock().get(key) {
            sink.detach();
        }
        self.wanted.lock().remove(key);
        if let Some((peer_id, agent_id)) = split_namespaced(key) {
            self.send(peer_id, ViewerMsg::Detach {
                agent_id: agent_id.to_string(),
            });
        }
    }

    /// `write_input`의 원격 갈래. 권한이 없으면 호스트가 거부한다(서버가 최종 판단).
    pub fn write_input(&self, key: &str, data: &str) {
        if let Some((peer_id, agent_id)) = split_namespaced(key) {
            self.send(peer_id, ViewerMsg::Input {
                agent_id: agent_id.to_string(),
                data: data.to_string(),
            });
        }
    }

    fn send_attach(&self, key: &str) {
        if let Some((peer_id, agent_id)) = split_namespaced(key) {
            let last = self.offsets.lock().get(key).copied();
            self.send(peer_id, ViewerMsg::Attach {
                agent_id: agent_id.to_string(),
                last_offset: last,
            });
        }
    }

    fn send(&self, peer_id: &str, msg: ViewerMsg) {
        let conns = self.conns.lock();
        if let Some(conn) = conns.get(peer_id) {
            if let Some(tx) = conn.tx.as_ref() {
                let _ = tx.send(msg);
            }
        }
    }

    // ── 연결 관리 ────────────────────────────────────────────────────

    pub fn status(&self) -> Vec<PeerStatus> {
        let conns = self.conns.lock();
        let mut out: Vec<PeerStatus> = conns
            .values()
            .map(|c| PeerStatus {
                peer_id: c.record.peer_id.clone(),
                label: c.record.label.clone(),
                address: c.record.address.clone(),
                state: c.state,
                permission: c.permission,
                error: c.error.clone(),
                agents: c
                    .agents
                    .iter()
                    .map(|a| namespaced_agent_json(&c.record.peer_id, a))
                    .collect(),
            })
            .collect();
        out.sort_by(|a, b| a.label.cmp(&b.label));
        out
    }

    /// 저장된 피어 중 자동 연결 대상을 전부 띄운다(부팅 시 1회).
    pub fn start_all(self: &Arc<Self>) {
        for host in self.hosts.load() {
            if host.auto_connect {
                self.connect(host);
            }
        }
    }

    /// 한 피어에 연결(멱등 — 이미 있으면 무시).
    pub fn connect(self: &Arc<Self>, record: PeerHostRecord) {
        let peer_id = record.peer_id.clone();
        {
            let mut conns = self.conns.lock();
            if conns.contains_key(&peer_id) {
                return;
            }
            conns.insert(
                peer_id.clone(),
                PeerConn {
                    permission: record.permission,
                    record: record.clone(),
                    state: PeerConnState::Connecting,
                    error: None,
                    tx: None,
                    agents: Vec::new(),
                    stop: Arc::new(AtomicBool::new(false)),
                },
            );
        }
        let stop = self
            .conns
            .lock()
            .get(&peer_id)
            .map(|c| c.stop.clone())
            .unwrap_or_default();
        let registry = self.clone();
        tokio::spawn(async move {
            registry.run_peer(record, stop).await;
        });
        self.publish_status();
    }

    /// 연결을 끊는다(저장된 피어 자체는 남는다).
    pub fn disconnect(&self, peer_id: &str) {
        let removed = self.conns.lock().remove(peer_id);
        if let Some(conn) = removed {
            conn.stop.store(true, Ordering::SeqCst);
        }
        // 이 피어의 원격 캐릭터 잔여 상태 정리.
        let prefix = format!("{PEER_AGENT_PREFIX}{peer_id}:");
        self.sinks.lock().retain(|k, _| !k.starts_with(&prefix));
        self.offsets.lock().retain(|k, _| !k.starts_with(&prefix));
        self.wanted.lock().retain(|k| !k.starts_with(&prefix));
        self.publish_status();
    }

    /// 피어를 목록에서도 제거.
    pub fn forget(&self, peer_id: &str) -> Result<(), String> {
        self.disconnect(peer_id);
        self.hosts.remove(peer_id).map_err(|e| e.to_string())
    }

    pub fn hosts(&self) -> Vec<PeerHostRecord> {
        self.hosts.load()
    }

    pub fn remember(&self, record: PeerHostRecord) -> Result<(), String> {
        self.hosts.insert(record).map_err(|e| e.to_string())
    }

    pub fn viewer_name(&self) -> &str {
        &self.viewer_name
    }

    fn publish_status(&self) {
        let payload = serde_json::to_value(self.status()).unwrap_or(serde_json::Value::Null);
        self.emit("peer-status", payload);
    }

    fn set_state(&self, peer_id: &str, state: PeerConnState, error: Option<String>) {
        {
            let mut conns = self.conns.lock();
            if let Some(conn) = conns.get_mut(peer_id) {
                conn.state = state;
                conn.error = error;
                if state != PeerConnState::Connected {
                    conn.tx = None;
                }
            }
        }
        self.publish_status();
    }

    /// 재접속 루프. 뷰어 끊김·호스트 앱 종료 모두 여기서 흡수한다.
    async fn run_peer(self: Arc<Self>, record: PeerHostRecord, stop: Arc<AtomicBool>) {
        let mut backoff = RECONNECT_MIN;
        while !stop.load(Ordering::SeqCst) {
            self.set_state(&record.peer_id, PeerConnState::Connecting, None);
            match self.clone().run_session(&record, stop.clone()).await {
                Ok(()) => {
                    backoff = RECONNECT_MIN;
                }
                Err(e) => {
                    self.set_state(&record.peer_id, PeerConnState::Disconnected, Some(e));
                }
            }
            if stop.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(RECONNECT_MAX);
        }
        self.set_state(&record.peer_id, PeerConnState::Disconnected, None);
    }

    /// 연결 1회분. 정상 종료(호스트가 닫음)면 Ok, 실패면 Err(사유).
    async fn run_session(
        self: Arc<Self>,
        record: &PeerHostRecord,
        stop: Arc<AtomicBool>,
    ) -> Result<(), String> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        use tokio_tungstenite::tungstenite::Message as TMessage;

        let url = format!("ws://{}/peer/v1/ws", record.address);
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| format!("주소가 올바르지 않습니다: {e}"))?;
        request.headers_mut().insert(
            PEER_TOKEN_HEADER,
            record
                .token
                .parse()
                .map_err(|_| "토큰 형식 오류".to_string())?,
        );
        let (socket, _resp) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| format!("연결 실패: {e}"))?;
        let (mut sink, mut stream) = socket.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<ViewerMsg>();

        {
            let mut conns = self.conns.lock();
            let Some(conn) = conns.get_mut(&record.peer_id) else {
                return Ok(()); // 그 사이 disconnect
            };
            conn.tx = Some(tx);
            conn.state = PeerConnState::Connected;
            conn.error = None;
        }
        self.publish_status();

        // 렌더러가 이미 보고 있던 캐릭터는 즉시 다시 붙는다.
        for key in self.wanted.lock().iter().cloned().collect::<Vec<_>>() {
            if key.starts_with(&format!("{PEER_AGENT_PREFIX}{}:", record.peer_id)) {
                self.send_attach(&key);
            }
        }

        let mut ping = tokio::time::interval(VIEWER_PING_EVERY);
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let result = loop {
            if stop.load(Ordering::SeqCst) {
                break Ok(());
            }
            tokio::select! {
                incoming = stream.next() => match incoming {
                    Some(Ok(TMessage::Text(text))) => {
                        match serde_json::from_str::<HostMsg>(&text) {
                            Ok(msg) => self.on_host_msg(&record.peer_id, msg),
                            Err(e) => eprintln!("peer: 알 수 없는 메시지 무시: {e}"),
                        }
                    }
                    Some(Ok(TMessage::Close(_))) | None => break Ok(()),
                    Some(Ok(_)) => {}
                    Some(Err(e)) => break Err(format!("연결이 끊겼습니다: {e}")),
                },
                outgoing = rx.recv() => match outgoing {
                    Some(msg) => {
                        let text = serde_json::to_string(&msg).unwrap_or_default();
                        if sink.send(TMessage::Text(text)).await.is_err() {
                            break Err("전송 실패".into());
                        }
                    }
                    None => break Ok(()),
                },
                _ = ping.tick() => {
                    if sink.send(TMessage::Ping(Vec::new())).await.is_err() {
                        break Err("연결이 끊겼습니다".into());
                    }
                }
            }
        };
        let _ = sink.close().await;
        {
            let mut conns = self.conns.lock();
            if let Some(conn) = conns.get_mut(&record.peer_id) {
                conn.tx = None;
                conn.state = PeerConnState::Disconnected;
            }
        }
        self.publish_status();
        result
    }

    fn on_host_msg(&self, peer_id: &str, msg: HostMsg) {
        match msg {
            HostMsg::Hello { permission, .. } => {
                let mut conns = self.conns.lock();
                if let Some(conn) = conns.get_mut(peer_id) {
                    conn.permission = permission;
                }
                drop(conns);
                self.publish_status();
            }
            HostMsg::Agents { agents } => {
                {
                    let mut conns = self.conns.lock();
                    if let Some(conn) = conns.get_mut(peer_id) {
                        conn.agents = agents;
                    }
                }
                self.publish_status();
            }
            HostMsg::Restore {
                agent_id,
                snapshot,
                base_offset,
                cols,
                rows,
                session_id,
            } => {
                let key = namespaced_agent_id(peer_id, &agent_id);
                self.offsets.lock().insert(key.clone(), base_offset);
                if let Some(snapshot) = snapshot {
                    // 화면 이미지 — 스트림 바이트로 계수하지 않는다(§#49와 동일 규칙).
                    self.push_chunk(&key, session_id.clone().unwrap_or_default(), 0, snapshot, 0);
                }
                self.emit(
                    "peer-resized",
                    serde_json::json!({ "agentId": key, "cols": cols, "rows": rows }),
                );
            }
            HostMsg::Output(out) => {
                let key = namespaced_agent_id(peer_id, &out.agent_id);
                self.offsets
                    .lock()
                    .insert(key.clone(), out.offset + out.bytes);
                self.push_chunk(&key, out.session_id, out.seq, out.data, out.bytes);
            }
            HostMsg::Activity { agent_id, payload } => {
                self.emit_rewritten("activity-event", peer_id, &agent_id, payload);
            }
            HostMsg::SessionState { agent_id, payload } => {
                self.emit_rewritten("session-state", peer_id, &agent_id, payload);
            }
            HostMsg::Notification { agent_id, payload } => {
                self.emit_rewritten("notification-new", peer_id, &agent_id, payload);
            }
            HostMsg::NotificationCleared { agent_id, ids } => {
                let key = namespaced_agent_id(peer_id, &agent_id);
                self.emit(
                    "notification-cleared",
                    serde_json::json!({ "agentId": key, "ids": ids }),
                );
            }
            HostMsg::Resized {
                agent_id,
                cols,
                rows,
            } => {
                let key = namespaced_agent_id(peer_id, &agent_id);
                self.emit(
                    "peer-resized",
                    serde_json::json!({ "agentId": key, "cols": cols, "rows": rows }),
                );
            }
            HostMsg::Pong => {}
            // 웹 클라이언트 전용 프레임 — 앱↔앱 뷰어는 RPC를 쓰지 않는다.
            HostMsg::RpcResult { .. } => {}
            HostMsg::Error { message } => {
                eprintln!("peer host error: {message}");
                self.emit(
                    "peer-error",
                    serde_json::json!({ "peerId": peer_id, "message": message }),
                );
            }
        }
    }

    /// 호스트 이벤트의 agentId를 네임스페이스 키로 바꿔 그대로 재방출한다 —
    /// 렌더러는 원격/로컬을 같은 파이프라인으로 소비한다.
    fn emit_rewritten(
        &self,
        event: &str,
        peer_id: &str,
        agent_id: &str,
        mut payload: serde_json::Value,
    ) {
        let key = namespaced_agent_id(peer_id, agent_id);
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("agentId".into(), serde_json::Value::String(key.clone()));
            // sessionId도 피어 네임스페이스로 감싼다 — 로컬 세션 id와 충돌하지
            // 않게(렌더러는 이 값을 키로만 쓴다).
            if let Some(sid) = obj.get("sessionId").and_then(|v| v.as_str()) {
                let scoped = format!("{PEER_AGENT_PREFIX}{peer_id}:{sid}");
                obj.insert("sessionId".into(), serde_json::Value::String(scoped));
            }
            obj.insert("remote".into(), serde_json::Value::Bool(true));
        }
        self.emit(event, payload);
    }

    fn push_chunk(&self, key: &str, session_id: String, seq: u64, data: String, bytes: u64) {
        let sink = self.sink_for(key);
        sink.push_chunk(OutputChunk {
            session_id: format!("{PEER_AGENT_PREFIX}{session_id}"),
            agent_id: key.to_string(),
            data,
            frames: 1,
            seq,
            bytes,
        });
    }
}

fn namespaced_agent_json(peer_id: &str, agent: &PeerAgent) -> serde_json::Value {
    let mut value = serde_json::to_value(agent).unwrap_or(serde_json::Value::Null);
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "agentId".into(),
            serde_json::Value::String(namespaced_agent_id(peer_id, &agent.agent_id)),
        );
        obj.insert(
            "localAgentId".into(),
            serde_json::Value::String(agent.agent_id.clone()),
        );
        obj.insert(
            "peerId".into(),
            serde_json::Value::String(peer_id.to_string()),
        );
    }
    value
}

// ── 페어링 클라이언트 ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairStartOutcome {
    pub pairing_id: String,
    pub host_name: String,
    pub expires_in: u64,
}

/// `pair/start` — 호스트 화면에 승인 다이얼로그와 코드를 띄운다.
pub async fn pair_start(address: &str, viewer_name: &str) -> Result<PairStartOutcome, String> {
    let url = format!("http://{address}/peer/v1/pair/start");
    let body = PairStartRequest {
        viewer_name: viewer_name.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        proto_version: PEER_PROTO_VERSION,
        // 앱↔앱 뷰어다(브라우저가 아니다) — 가시성은 공유 토글을 따른다.
        client_kind: PeerClientKind::Peer,
    };
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("호스트에 닿지 못했습니다: {e}"))?;
    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("응답을 읽지 못했습니다: {e}"))?;
    if value["ok"].as_bool() != Some(true) {
        return Err(value["error"]
            .as_str()
            .unwrap_or("페어링을 시작하지 못했습니다")
            .to_string());
    }
    let parsed: PairStartResponse = serde_json::from_value(value["data"].clone())
        .map_err(|e| format!("응답 형식 오류: {e}"))?;
    Ok(PairStartOutcome {
        pairing_id: parsed.pairing_id,
        host_name: parsed.host_name,
        expires_in: parsed.expires_in,
    })
}

/// `pair/complete` — 코드를 제시해 토큰을 받는다. 호스트가 아직 승인 버튼을
/// 누르지 않았으면 `Ok(None)`이라 뷰어는 잠시 후 다시 부르면 된다.
pub async fn pair_complete(
    address: &str,
    pairing_id: &str,
    code: &str,
) -> Result<Option<PeerHostRecord>, String> {
    let url = format!("http://{address}/peer/v1/pair/complete");
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&PairCompleteRequest {
            pairing_id: pairing_id.to_string(),
            code: code.to_string(),
        })
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("호스트에 닿지 못했습니다: {e}"))?;
    let status = resp.status();
    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("응답을 읽지 못했습니다: {e}"))?;
    if status == reqwest::StatusCode::ACCEPTED {
        return Ok(None); // 승인 대기 — 재시도하면 된다
    }
    if value["ok"].as_bool() != Some(true) {
        return Err(value["error"]
            .as_str()
            .unwrap_or("페어링에 실패했습니다")
            .to_string());
    }
    let parsed: PairCompleteResponse = serde_json::from_value(value["data"].clone())
        .map_err(|e| format!("응답 형식 오류: {e}"))?;
    Ok(Some(PeerHostRecord {
        peer_id: parsed.peer_id,
        label: parsed.host_name,
        address: address.to_string(),
        token: parsed.peer_token,
        permission: parsed.permission,
        auto_connect: true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Arc<ViewerRegistry> {
        let dir = std::env::temp_dir().join(format!("peer-viewer-{}", uuid::Uuid::new_v4()));
        ViewerRegistry::new(PeerHostStore::new(dir.join("hosts.json")), "테스트".into())
    }

    #[derive(Default)]
    struct Recorder {
        events: Mutex<Vec<(String, serde_json::Value)>>,
    }
    impl ViewerEvents for Recorder {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            self.events.lock().push((event.to_string(), payload));
        }
    }

    #[test]
    fn output_lands_in_the_sink_and_tracks_offsets() {
        let reg = registry();
        let key = namespaced_agent_id("p1", "ada");
        reg.on_host_msg(
            "p1",
            HostMsg::Output(PeerOutput {
                agent_id: "ada".into(),
                session_id: "s1".into(),
                seq: 1,
                offset: 0,
                data: "hello".into(),
                bytes: 5,
            }),
        );
        assert_eq!(reg.offsets.lock().get(&key).copied(), Some(5));
        // sink가 만들어져 백로그에 쌓여 있어야 한다(렌더러 미구독 상태).
        assert!(reg.sinks.lock().contains_key(&key));
    }

    #[test]
    fn restore_sets_the_base_offset_without_counting_the_snapshot() {
        let reg = registry();
        let key = namespaced_agent_id("p1", "ada");
        reg.on_host_msg(
            "p1",
            HostMsg::Restore {
                agent_id: "ada".into(),
                snapshot: Some("SCREEN".into()),
                base_offset: 120,
                cols: 80,
                rows: 24,
                session_id: Some("s1".into()),
            },
        );
        assert_eq!(reg.offsets.lock().get(&key).copied(), Some(120));
    }

    #[test]
    fn events_are_rewritten_to_the_namespaced_agent_id() {
        let reg = registry();
        let rec = Arc::new(Recorder::default());
        reg.set_events(rec.clone());
        reg.on_host_msg(
            "p1",
            HostMsg::Activity {
                agent_id: "ada".into(),
                payload: serde_json::json!({
                    "agentId": "ada",
                    "sessionId": "s1",
                    "kind": "prompt",
                    "at": 1
                }),
            },
        );
        let events = rec.events.lock();
        let (name, payload) = events.last().expect("이벤트");
        assert_eq!(name, "activity-event");
        assert_eq!(payload["agentId"], "peer:p1:ada");
        assert_eq!(payload["sessionId"], "peer:p1:s1");
        assert_eq!(payload["remote"], true);
        assert_eq!(payload["kind"], "prompt");
    }

    #[test]
    fn disconnect_clears_that_peers_remote_state_only() {
        let reg = registry();
        let mine = namespaced_agent_id("p1", "ada");
        let other = namespaced_agent_id("p2", "bob");
        reg.sink_for(&mine);
        reg.sink_for(&other);
        reg.wanted.lock().insert(mine.clone());
        reg.wanted.lock().insert(other.clone());
        reg.offsets.lock().insert(mine.clone(), 10);
        reg.offsets.lock().insert(other.clone(), 20);

        reg.disconnect("p1");

        assert!(!reg.sinks.lock().contains_key(&mine));
        assert!(reg.sinks.lock().contains_key(&other));
        assert!(!reg.wanted.lock().contains(&mine));
        assert!(reg.wanted.lock().contains(&other));
        assert_eq!(reg.offsets.lock().get(&other).copied(), Some(20));
    }

    #[test]
    fn write_input_on_a_local_key_is_a_noop() {
        let reg = registry();
        // 접두사가 없으면 원격이 아니므로 아무 피어에도 보내지 않는다.
        reg.write_input("ada", "hi");
        assert!(reg.conns.lock().is_empty());
    }

    #[test]
    fn namespaced_agent_json_carries_both_ids() {
        let json = namespaced_agent_json(
            "p1",
            &PeerAgent {
                agent_id: "ada".into(),
                name: "아다".into(),
                ..Default::default()
            },
        );
        assert_eq!(json["agentId"], "peer:p1:ada");
        assert_eq!(json["localAgentId"], "ada");
        assert_eq!(json["peerId"], "p1");
        assert_eq!(json["name"], "아다");
    }
}
