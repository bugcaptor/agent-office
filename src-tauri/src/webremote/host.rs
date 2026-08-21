// src-tauri/src/webremote/host.rs
//
// 호스트 쪽 중계 허브(#7k §결정 2·4). 공유가 켜진 캐릭터마다
//
//   · `OutputSink`에 tap 하나를 달아 출력을 받아 적고(링버퍼 + 절대 오프셋),
//   · 라이브 청크와 앱 이벤트를 broadcast 채널로 모든 뷰어 연결에 팬아웃한다.
//
// 뷰어가 붙을 때의 첫 화면은 "화면 스냅샷 + 그 이후 델타"로 만든다. 스냅샷은
// 호스트 렌더러가 xterm을 직렬화해 올려 주고(SnapshotBridge), 그 기준 오프셋은
// **요청을 보낸 시점**의 오프셋으로 잡는다 — 직렬화~수신 사이에 흘러온 바이트가
// 스냅샷에도 들어가고 델타로도 한 번 더 오는 중복은 감수하되(수 ms, TUI는
// 재도색으로 흡수) 영구 유실 창은 만들지 않는다. 스냅샷을 못 받으면 링버퍼
// 전체를 리플레이하는 폴백으로 내려간다.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::{broadcast, oneshot};

use crate::session::manager::SessionManager;
use crate::session::output::OutputTap;
use crate::state::AppEvents;
use crate::types::OutputChunk;

use super::protocol::{HostMsg, RemoteOutput};

/// 공유 세션당 보관하는 raw tail 링버퍼 상한. 스냅샷이 주 복원 경로이고
/// 링은 "스냅샷 이후 델타 + 재접속 델타"를 담는 창이다.
pub const RING_CAP_BYTES: usize = 1024 * 1024;

/// 뷰어 연결에 팬아웃하는 broadcast 큐 깊이. 넘치면(느린 뷰어) 그 연결만
/// Lagged를 받고 스스로 재-attach 한다.
const BROADCAST_CAP: usize = 4096;

/// 렌더러 스냅샷 응답을 기다리는 시간. 넘기면 링버퍼 폴백.
pub const SNAPSHOT_TIMEOUT: Duration = Duration::from_millis(2000);

#[derive(Debug, Clone)]
pub struct RingChunk {
    /// 이 청크가 시작하는 절대 스트림 오프셋.
    pub offset: u64,
    pub data: String,
    pub bytes: u64,
    pub seq: u64,
    pub session_id: String,
}

#[derive(Debug, Clone)]
struct BaseSnapshot {
    data: String,
    offset: u64,
}

/// attach 한 번에 뷰어로 보낼 복원 계획.
#[derive(Debug, Clone)]
pub struct Replay {
    /// Some이면 화면 이미지를 먼저 복원한다.
    pub snapshot: Option<String>,
    /// 복원 직후의 절대 오프셋(= 이어지는 첫 청크의 offset).
    pub base_offset: u64,
    pub chunks: Vec<RingChunk>,
    /// 스냅샷 없이 링만으로 복원했는가 — 호출자가 스냅샷을 새로 받아올지
    /// 판단하는 신호.
    pub needs_snapshot: bool,
}

#[derive(Default)]
pub struct SharedStream {
    ring: VecDeque<RingChunk>,
    ring_bytes: usize,
    /// 다음 청크가 시작할 절대 오프셋(= 지금까지 흘린 raw 바이트 총합).
    total: u64,
    base: Option<BaseSnapshot>,
    session_id: Option<String>,
}

impl SharedStream {
    fn push(&mut self, chunk: &OutputChunk) {
        let entry = RingChunk {
            offset: self.total,
            data: chunk.data.clone(),
            bytes: chunk.bytes,
            seq: chunk.seq,
            session_id: chunk.session_id.clone(),
        };
        self.total = self.total.saturating_add(chunk.bytes);
        self.session_id = Some(chunk.session_id.clone());
        self.ring_bytes += entry.data.len();
        self.ring.push_back(entry);
        while self.ring_bytes > RING_CAP_BYTES {
            match self.ring.pop_front() {
                Some(dropped) => self.ring_bytes -= dropped.data.len(),
                None => break,
            }
        }
    }

    /// 링에 남아 있는 가장 오래된 오프셋(비어 있으면 현재 total).
    fn ring_start(&self) -> u64 {
        self.ring.front().map(|c| c.offset).unwrap_or(self.total)
    }

    fn chunks_from(&self, offset: u64) -> Vec<RingChunk> {
        self.ring
            .iter()
            .filter(|c| c.offset >= offset)
            .cloned()
            .collect()
    }

    /// 재접속(`last_offset`)이면 델타만, 첫 접속이면 스냅샷+이후를 계획한다.
    pub fn replay(&self, last_offset: Option<u64>) -> Replay {
        let ring_start = self.ring_start();
        // 델타 경로 — 뷰어가 아는 지점이 아직 링 안에 있으면 그것만 보낸다.
        if let Some(last) = last_offset {
            if last >= ring_start && last <= self.total {
                return Replay {
                    snapshot: None,
                    base_offset: last,
                    chunks: self.chunks_from(last),
                    needs_snapshot: false,
                };
            }
        }
        // 전체 복원 경로 — 스냅샷 기준점이 링 안에 있어야 "스냅샷 + 이후"가
        // 빈틈 없이 이어진다. 아니면 스냅샷을 버리고 링만 흘린다(그 경우
        // 호출자가 새 스냅샷을 받아 다시 계획한다).
        match &self.base {
            Some(base) if base.offset >= ring_start && base.offset <= self.total => Replay {
                snapshot: Some(base.data.clone()),
                base_offset: base.offset,
                chunks: self.chunks_from(base.offset),
                needs_snapshot: false,
            },
            _ => Replay {
                snapshot: None,
                base_offset: ring_start,
                chunks: self.chunks_from(ring_start),
                needs_snapshot: true,
            },
        }
    }

    fn set_base(&mut self, data: String, offset: u64) {
        self.base = Some(BaseSnapshot { data, offset });
    }

    pub fn total(&self) -> u64 {
        self.total
    }
}

// ── 렌더러 스냅샷 왕복 ────────────────────────────────────────────────

type SnapshotRequestFn = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// 호스트 렌더러에 "이 캐릭터의 xterm을 지금 직렬화해 달라"고 요청하고 응답을
/// 기다리는 다리. 요청 방출은 lib.rs가 AppHandle로 주입한다(이 모듈은 Tauri를
/// 모른다 — 테스트에서 페이크 주입이 가능하다).
#[derive(Default)]
pub struct SnapshotBridge {
    emit: Mutex<Option<SnapshotRequestFn>>,
    pending: Mutex<HashMap<String, oneshot::Sender<String>>>,
}

impl SnapshotBridge {
    pub fn set_emitter(&self, emit: SnapshotRequestFn) {
        *self.emit.lock() = Some(emit);
    }

    /// 렌더러가 `web_remote_submit_snapshot` 커맨드로 되돌려준다.
    pub fn submit(&self, request_id: &str, snapshot: String) {
        let tx = self.pending.lock().remove(request_id);
        if let Some(tx) = tx {
            let _ = tx.send(snapshot);
        }
    }

    /// 요청 후 타임아웃까지 대기. 방출기가 없거나(부트 이전) 응답이 없으면 None.
    pub async fn request(&self, agent_id: &str, timeout: Duration) -> Option<String> {
        let emit = self.emit.lock().clone()?;
        let request_id = uuid::Uuid::new_v4().simple().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(request_id.clone(), tx);
        emit(agent_id, &request_id);
        let got = tokio::time::timeout(timeout, rx).await;
        self.pending.lock().remove(&request_id);
        match got {
            Ok(Ok(snapshot)) => Some(snapshot),
            _ => None,
        }
    }
}

// ── 허브 ──────────────────────────────────────────────────────────────

pub struct WebRemoteHub {
    streams: Mutex<HashMap<String, Arc<Mutex<SharedStream>>>>,
    /// agentId → 설치된 tap id. 이 맵의 키가 곧 "공유 중인 캐릭터" 집합이다.
    taps: Mutex<HashMap<String, u64>>,
    tx: broadcast::Sender<Arc<HostMsg>>,
    pub snapshots: SnapshotBridge,
}

impl WebRemoteHub {
    pub fn new() -> Arc<Self> {
        let (tx, _rx) = broadcast::channel(BROADCAST_CAP);
        Arc::new(Self {
            streams: Mutex::new(HashMap::new()),
            taps: Mutex::new(HashMap::new()),
            tx,
            snapshots: SnapshotBridge::default(),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<HostMsg>> {
        self.tx.subscribe()
    }

    pub fn is_shared(&self, agent_id: &str) -> bool {
        self.taps.lock().contains_key(agent_id)
    }

    pub fn shared_agents(&self) -> Vec<String> {
        let mut v: Vec<String> = self.taps.lock().keys().cloned().collect();
        v.sort();
        v
    }

    pub fn broadcast(&self, msg: HostMsg) {
        // 수신자가 없으면 Err — 뷰어가 아무도 안 붙은 정상 상태다.
        let _ = self.tx.send(Arc::new(msg));
    }

    /// 캐릭터 공유를 켠다(멱등). sink는 agentId 수명이라 세션이 아직 없어도
    /// 미리 달아 둘 수 있다 — 이후 세션이 뜨면 첫 바이트부터 링에 담긴다.
    pub fn share(self: &Arc<Self>, manager: &SessionManager, agent_id: &str) {
        if self.taps.lock().contains_key(agent_id) {
            return;
        }
        self.streams
            .lock()
            .entry(agent_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(SharedStream::default())));
        let tap: Arc<dyn OutputTap> = Arc::new(HubTap {
            hub: Arc::downgrade(self),
            agent_id: agent_id.to_string(),
        });
        let id = manager.add_output_tap(agent_id, tap);
        self.taps.lock().insert(agent_id.to_string(), id);
    }

    /// 공유를 끈다. 링버퍼도 함께 버린다(다음에 켜면 새 스트림).
    pub fn unshare(&self, manager: &SessionManager, agent_id: &str) {
        let removed = self.taps.lock().remove(agent_id);
        if let Some(id) = removed {
            manager.remove_output_tap(agent_id, id);
        }
        self.streams.lock().remove(agent_id);
    }

    fn stream(&self, agent_id: &str) -> Option<Arc<Mutex<SharedStream>>> {
        self.streams.lock().get(agent_id).cloned()
    }

    fn on_chunk(&self, agent_id: &str, chunk: &OutputChunk) {
        let Some(stream) = self.stream(agent_id) else {
            return;
        };
        let offset = {
            let mut s = stream.lock();
            let offset = s.total();
            s.push(chunk);
            offset
        };
        self.broadcast(HostMsg::Output(RemoteOutput {
            agent_id: agent_id.to_string(),
            session_id: chunk.session_id.clone(),
            seq: chunk.seq,
            offset,
            data: chunk.data.clone(),
            bytes: chunk.bytes,
        }));
    }

    /// attach 처리: 델타면 즉시, 첫 접속이면 스냅샷을 한 번 받아 계획을 다시
    /// 세운다. 스냅샷 실패는 링버퍼 폴백이다(연결을 막지 않는다).
    pub async fn replay_for(&self, agent_id: &str, last_offset: Option<u64>) -> Option<Replay> {
        let stream = self.stream(agent_id)?;
        let plan = stream.lock().replay(last_offset);
        if !plan.needs_snapshot {
            return Some(plan);
        }
        // 기준 오프셋은 **요청 직전** 값으로 잡는다(유실 창 0, 중복 최소).
        let requested_at = stream.lock().total();
        match self.snapshots.request(agent_id, SNAPSHOT_TIMEOUT).await {
            Some(snapshot) => {
                let mut s = stream.lock();
                s.set_base(snapshot, requested_at);
                Some(s.replay(None))
            }
            None => Some(plan),
        }
    }

    /// 뷰어가 알아야 할 현재 터미널 크기.
    pub fn session_id_of(&self, agent_id: &str) -> Option<String> {
        self.stream(agent_id)?.lock().session_id.clone()
    }

    #[cfg(test)]
    pub fn push_for_test(&self, agent_id: &str, chunk: &OutputChunk) {
        self.on_chunk(agent_id, chunk);
    }

    #[cfg(test)]
    pub fn share_for_test(self: &Arc<Self>, agent_id: &str) {
        self.streams
            .lock()
            .entry(agent_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(SharedStream::default())));
        self.taps.lock().insert(agent_id.to_string(), 0);
    }
}

/// `OutputSink`에 꽂히는 tap. 허브를 **Weak**로 쥐어, 매니저의 sink가 tap을
/// 오래 들고 있어도 허브 수명이 그것에 매이지 않게 한다.
struct HubTap {
    hub: std::sync::Weak<WebRemoteHub>,
    agent_id: String,
}

impl OutputTap for HubTap {
    fn on_chunk(&self, chunk: &OutputChunk) {
        if let Some(hub) = self.hub.upgrade() {
            hub.on_chunk(&self.agent_id, chunk);
        }
    }
}

// ── 앱 이벤트 미러 ────────────────────────────────────────────────────

/// 공유 중인 캐릭터의 앱 이벤트만 뷰어로 흘리는 `AppEvents` 구현.
/// `CompositeEvents`로 `TauriEvents`와 나란히 세워 쓴다(§결정 4).
pub struct WebRemoteEvents {
    hub: Arc<WebRemoteHub>,
}

impl WebRemoteEvents {
    pub fn new(hub: Arc<WebRemoteHub>) -> Self {
        Self { hub }
    }

    fn mirror<T: serde::Serialize>(&self, agent_id: &str, make: impl FnOnce(serde_json::Value) -> HostMsg, ev: &T) {
        if !self.hub.is_shared(agent_id) {
            return;
        }
        if let Ok(payload) = serde_json::to_value(ev) {
            self.hub.broadcast(make(payload));
        }
    }
}

impl AppEvents for WebRemoteEvents {
    fn session_state(&self, ev: &crate::types::SessionStateEvent) {
        let agent_id = ev.agent_id.clone();
        self.mirror(
            &ev.agent_id,
            move |payload| HostMsg::SessionState { agent_id, payload },
            ev,
        );
    }

    fn notification_new(&self, ev: &crate::types::NotificationEvent) {
        let agent_id = ev.agent_id.clone();
        self.mirror(
            &ev.agent_id,
            move |payload| HostMsg::Notification { agent_id, payload },
            ev,
        );
    }

    fn notification_cleared(&self, agent_id: &str, ids: &[String]) {
        if !self.hub.is_shared(agent_id) {
            return;
        }
        self.hub.broadcast(HostMsg::NotificationCleared {
            agent_id: agent_id.to_string(),
            ids: ids.to_vec(),
        });
    }

    fn activity_event(&self, ev: &crate::types::ActivityEvent) {
        let agent_id = ev.agent_id.clone();
        self.mirror(
            &ev.agent_id,
            move |payload| HostMsg::Activity { agent_id, payload },
            ev,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(session: &str, agent: &str, data: &str, seq: u64) -> OutputChunk {
        OutputChunk {
            session_id: session.into(),
            agent_id: agent.into(),
            data: data.into(),
            frames: 1,
            seq,
            bytes: data.len() as u64,
        }
    }

    #[test]
    fn ring_tracks_absolute_offsets() {
        let mut s = SharedStream::default();
        s.push(&chunk("s1", "a", "hello", 1));
        s.push(&chunk("s1", "a", " world", 2));
        assert_eq!(s.total(), 11);
        let plan = s.replay(None);
        // 스냅샷이 없으니 링 전체 + "스냅샷 필요" 신호.
        assert!(plan.needs_snapshot);
        assert_eq!(plan.base_offset, 0);
        assert_eq!(plan.chunks.len(), 2);
        assert_eq!(plan.chunks[0].offset, 0);
        assert_eq!(plan.chunks[1].offset, 5);
    }

    #[test]
    fn reconnect_with_known_offset_sends_only_the_delta() {
        let mut s = SharedStream::default();
        s.push(&chunk("s1", "a", "aaaaa", 1));
        s.push(&chunk("s1", "a", "bbbbb", 2));
        let plan = s.replay(Some(5));
        assert!(!plan.needs_snapshot);
        assert!(plan.snapshot.is_none());
        assert_eq!(plan.base_offset, 5);
        assert_eq!(plan.chunks.len(), 1);
        assert_eq!(plan.chunks[0].data, "bbbbb");
    }

    #[test]
    fn caught_up_viewer_gets_nothing() {
        let mut s = SharedStream::default();
        s.push(&chunk("s1", "a", "aaaaa", 1));
        let plan = s.replay(Some(5));
        assert!(plan.chunks.is_empty());
        assert_eq!(plan.base_offset, 5);
    }

    #[test]
    fn snapshot_base_inside_ring_is_used_as_the_restore_point() {
        let mut s = SharedStream::default();
        s.push(&chunk("s1", "a", "old", 1));
        s.set_base("SCREEN".into(), 3);
        s.push(&chunk("s1", "a", "new", 2));
        let plan = s.replay(None);
        assert!(!plan.needs_snapshot);
        assert_eq!(plan.snapshot.as_deref(), Some("SCREEN"));
        assert_eq!(plan.base_offset, 3);
        assert_eq!(plan.chunks.len(), 1);
        assert_eq!(plan.chunks[0].data, "new");
    }

    #[test]
    fn stale_offset_outside_the_ring_falls_back_to_full_restore() {
        let mut s = SharedStream::default();
        // 링을 넘치게 밀어 앞부분을 밀어낸다.
        let big = "x".repeat(64 * 1024);
        for i in 0..20 {
            s.push(&chunk("s1", "a", &big, i));
        }
        assert!(s.ring_start() > 0, "링이 실제로 잘려야 하는 시나리오");
        let plan = s.replay(Some(0)); // 너무 오래된 지점
        assert!(plan.needs_snapshot);
        assert_eq!(plan.base_offset, s.ring_start());
    }

    #[test]
    fn ring_eviction_keeps_the_cap() {
        let mut s = SharedStream::default();
        let big = "y".repeat(100 * 1024);
        for i in 0..30 {
            s.push(&chunk("s1", "a", &big, i));
        }
        assert!(s.ring_bytes <= RING_CAP_BYTES);
        assert_eq!(s.total(), 30 * 100 * 1024);
    }

    #[test]
    fn base_older_than_ring_is_dropped_to_avoid_a_gap() {
        let mut s = SharedStream::default();
        s.set_base("OLD".into(), 0);
        let big = "z".repeat(64 * 1024);
        for i in 0..20 {
            s.push(&chunk("s1", "a", &big, i));
        }
        let plan = s.replay(None);
        // 스냅샷 기준점이 링 밖이면 이어붙일 수 없으므로 버린다.
        assert!(plan.snapshot.is_none());
        assert!(plan.needs_snapshot);
    }

    #[tokio::test]
    async fn hub_fans_out_output_to_subscribers() {
        let hub = WebRemoteHub::new();
        hub.share_for_test("a");
        let mut rx = hub.subscribe();
        hub.push_for_test("a", &chunk("s1", "a", "hi", 1));
        let msg = rx.try_recv().expect("broadcast");
        match &*msg {
            HostMsg::Output(out) => {
                assert_eq!(out.agent_id, "a");
                assert_eq!(out.data, "hi");
                assert_eq!(out.offset, 0);
                assert_eq!(out.bytes, 2);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn unshared_agent_output_is_ignored() {
        let hub = WebRemoteHub::new();
        let mut rx = hub.subscribe();
        hub.push_for_test("ghost", &chunk("s1", "ghost", "hi", 1));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn snapshot_bridge_roundtrip_and_timeout() {
        let bridge = Arc::new(SnapshotBridge::default());
        let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        {
            let bridge2 = bridge.clone();
            let seen2 = seen.clone();
            bridge.set_emitter(Arc::new(move |agent: &str, req: &str| {
                seen2.lock().push((agent.to_string(), req.to_string()));
                // 렌더러가 즉시 응답한 상황을 흉내낸다.
                bridge2.submit(req, format!("SNAP:{agent}"));
            }));
        }
        let got = bridge.request("ada", Duration::from_millis(500)).await;
        assert_eq!(got.as_deref(), Some("SNAP:ada"));
        assert_eq!(seen.lock().len(), 1);

        // 응답하지 않는 방출기 → 타임아웃 None.
        let quiet = SnapshotBridge::default();
        quiet.set_emitter(Arc::new(|_, _| {}));
        let none = quiet.request("ada", Duration::from_millis(50)).await;
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn replay_for_requests_a_snapshot_on_first_attach() {
        let hub = WebRemoteHub::new();
        hub.share_for_test("ada");
        hub.push_for_test("ada", &chunk("s1", "ada", "before", 1));
        let hub2 = hub.clone();
        hub.snapshots.set_emitter(Arc::new(move |_agent, req| {
            hub2.snapshots.submit(req, "SCREEN".into());
        }));
        let plan = hub.replay_for("ada", None).await.expect("plan");
        assert_eq!(plan.snapshot.as_deref(), Some("SCREEN"));
        // 스냅샷 기준점이 요청 시점(=6)이라 그 이후 델타만 붙는다.
        assert_eq!(plan.base_offset, 6);
        assert!(plan.chunks.is_empty());
    }

    #[tokio::test]
    async fn replay_for_falls_back_to_ring_when_snapshot_times_out() {
        let hub = WebRemoteHub::new();
        hub.share_for_test("ada");
        hub.push_for_test("ada", &chunk("s1", "ada", "hello", 1));
        hub.snapshots.set_emitter(Arc::new(|_, _| {})); // 응답 없음
        let plan = tokio::time::timeout(
            Duration::from_secs(5),
            hub.replay_for("ada", Some(999)), // 범위 밖 → 전체 복원 경로
        )
        .await
        .expect("타임아웃 안에 반환")
        .expect("plan");
        assert!(plan.snapshot.is_none());
        assert_eq!(plan.chunks.len(), 1);
    }
}
