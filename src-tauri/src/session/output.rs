// src-tauri/src/session/output.rs
//
// 출력 파이프라인: agentId당 Channel+백로그를 보관하는 OutputSink와, PTY
// reader가 만든 원시 바이트를 배칭해 그 Channel로 방출하는 output pump.
// 둘 다 세션 수명과는 독립적인 자원/태스크 -- session/manager.rs의
// SessionManager::sinks, install_session()이 이 모듈을 소비한다.

use std::sync::Arc;

use parking_lot::Mutex;

use tauri::ipc::Channel;

use crate::notification::hub::NotificationHub;
use crate::session::output_batcher::{FlushSink, OutputBatcher, MAX_BYTES, WINDOW_MS};
use crate::types::*;

const BACKLOG_CAP: usize = 256;

/// BEL은 단독 제어 문자일 때만 알림이다. OSC(제목·진행 표시·링크 등)의
/// 종결자도 같은 바이트를 쓰므로 PTY 청크 경계를 넘어 문자열 상태를 보존한다.
/// UTF-8 출력의 연속 바이트를 C1 제어 문자로 오인하지 않도록 ESC 형식만 읽는다.
#[derive(Default)]
struct BellDetector {
    state: BellState,
}

#[derive(Clone, Copy, Default)]
enum BellState {
    #[default]
    Ground,
    Escape,
    String {
        osc: bool,
    },
}

impl BellDetector {
    fn feed(&mut self, bytes: &[u8]) -> bool {
        let mut bell = false;
        for &byte in bytes {
            self.state = match self.state {
                BellState::String { osc } => match byte {
                    0x18 | 0x1a => BellState::Ground, // CAN/SUB: 시퀀스 취소
                    // ESC는 현재 문자열을 끝내고 새 시퀀스를 시작한다.
                    // 다음 바이트가 '\\'이면 ST, '['이면 CSI 등으로 이어진다.
                    0x1b => BellState::Escape,
                    0x07 if osc => BellState::Ground,
                    _ => BellState::String { osc },
                },
                state => match byte {
                    0x07 => {
                        bell = true;
                        state // BEL은 ESC 시퀀스도 취소하지 않는다.
                    }
                    0x1b => BellState::Escape,
                    b']' if matches!(state, BellState::Escape) => BellState::String { osc: true },
                    b'P' | b'X' | b'^' | b'_' if matches!(state, BellState::Escape) => {
                        // DCS/SOS/PM/APC 본문도 알림 신호로 해석하지 않는다.
                        BellState::String { osc: false }
                    }
                    _ => BellState::Ground,
                },
            };
        }
        bell
    }
}

/// 출력 tap — 렌더러 채널과 **별개로** 같은 청크를 흘려받는 구독자
/// (피어 세션 공유 #7k, docs/peer-session-share-design.md §결정 2).
///
/// `emit`이 이미 유일한 방출 지점이라 여기 한 겹만 얹으면 팬아웃이 끝난다.
/// `Vec`으로 보관하므로 뷰어가 여럿 붙어도 그대로 성립한다. 구현체는
/// **블로킹하면 안 된다** — emit은 출력 펌프 태스크에서 호출된다(피어 tap은
/// broadcast 채널에 던지고 즉시 반환한다).
pub trait OutputTap: Send + Sync {
    fn on_chunk(&self, chunk: &OutputChunk);
}

struct TapEntry {
    id: u64,
    tap: Arc<dyn OutputTap>,
}

pub(super) enum ReaderMsg {
    Data(Vec<u8>),
    /// adopt 복원 스냅샷(화면 이미지). 스트림 바이트로 계수하지 않는다(§#49 함정 2):
    /// base가 이미 이 지점을 가리키므로 offset에 잡히면 그만큼 데이터가 유실된다.
    /// 렌더러 누적 회계에 안 잡히도록 bytes=0 청크로 방출된다.
    Restore(Vec<u8>),
    Eof,
}

/// agentId당 출력 Channel + 등록 이전 백로그. FlushSink 구현체.
pub struct OutputSink {
    channel: Mutex<Option<Channel<OutputChunk>>>,
    backlog: Mutex<std::collections::VecDeque<OutputChunk>>,
    /// 렌더러 채널과 별개의 부가 구독자들(피어 세션 공유). backlog 의미론은
    /// **primary 전용**이다 — tap은 뷰어가 붙기 전 출력을 여기서 받지 않고,
    /// peer 쪽 링버퍼가 그 역할을 한다(목적이 다르다).
    taps: Mutex<Vec<TapEntry>>,
    next_tap_id: std::sync::atomic::AtomicU64,
}
impl OutputSink {
    /// pub: 웹 원격 tap이
    /// sink를 직접 만들어 쓴다 — 렌더러 파이프라인을 그대로 재사용하는 핵심.
    pub fn new() -> Self {
        Self {
            channel: Mutex::new(None),
            backlog: Mutex::new(Default::default()),
            taps: Mutex::new(Vec::new()),
            next_tap_id: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// tap을 등록하고 제거용 id를 반환한다.
    pub fn add_tap(&self, tap: Arc<dyn OutputTap>) -> u64 {
        let id = self
            .next_tap_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.taps.lock().push(TapEntry { id, tap });
        id
    }

    /// 등록된 tap을 제거한다. 없는 id는 무해한 no-op.
    pub fn remove_tap(&self, id: u64) {
        self.taps.lock().retain(|e| e.id != id);
    }

    pub fn tap_count(&self) -> usize {
        self.taps.lock().len()
    }

    /// 외부(피어 뷰어)에서 만든 청크를 이 sink로 흘린다 — 원격 세션의 출력을
    /// 렌더러 채널/백로그에 그대로 태우는 진입점.
    pub fn push_chunk(&self, chunk: OutputChunk) {
        self.emit(chunk);
    }

    pub fn attach(&self, ch: Channel<OutputChunk>) {
        // 락 순서 항상 channel → backlog (데드락 방지, emit과 동일 순서).
        let mut c = self.channel.lock();
        let mut b = self.backlog.lock();
        for chunk in b.drain(..) {
            let _ = ch.send(chunk);
        }
        *c = Some(ch);
    }
    pub fn detach(&self) {
        *self.channel.lock() = None;
    }
    /// 핸드오프 스냅샷 폴백(실증에서 발견된 빈틈): 프론트가 이 터미널을
    /// 한 번도 구독하지 않은 채 종료하면 xterm 쪽 직렬화 스냅샷이 없다 --
    /// 그 세션의 종료 전 출력은 여기 backlog에만 남아 있으므로, 원시
    /// 바이트를 이어붙여 스냅샷 대용으로 쓴다. **드레인하지 않고 복사만
    /// 한다** -- 핸드오프가 실패해도(데몬 연결 불가 등) 이 세션은 맵에
    /// 그대로 남아 출력이 이어져야 하므로 backlog를 비우면 안 된다.
    pub(super) fn backlog_snapshot(&self) -> Vec<u8> {
        self.backlog
            .lock()
            .iter()
            .flat_map(|chunk| chunk.data.as_bytes())
            .copied()
            .collect()
    }
}
impl FlushSink for OutputSink {
    fn emit(&self, chunk: OutputChunk) {
        // tap 팬아웃은 primary와 독립이다 — 렌더러가 붙었든(채널) 안 붙었든
        // (백로그) 공유 중인 세션의 출력은 언제나 tap으로 흐른다. 락은 겹치지
        // 않게 잡는다(tap 콜백을 primary 락 아래에서 부르지 않는다).
        {
            let taps = self.taps.lock();
            if !taps.is_empty() {
                let subscribers: Vec<Arc<dyn OutputTap>> =
                    taps.iter().map(|e| e.tap.clone()).collect();
                drop(taps);
                for tap in subscribers {
                    tap.on_chunk(&chunk);
                }
            }
        }
        let c = self.channel.lock();
        if let Some(ch) = c.as_ref() {
            let _ = ch.send(chunk); // Channel 전송 실패(웹뷰 소멸)는 무시
        } else {
            let mut b = self.backlog.lock();
            if b.len() >= BACKLOG_CAP {
                b.pop_front();
            }
            b.push_back(chunk);
        }
    }
}

impl Default for OutputSink {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn spawn_output_pump(
    session_id: String,
    agent_id: String,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<ReaderMsg>,
    sink: Arc<OutputSink>,
    hub: Arc<NotificationHub>,
    log: Option<Arc<crate::session_log::SessionLogHandle>>,
) {
    tokio::spawn(async move {
        let mut batcher = OutputBatcher::new(session_id.clone(), agent_id);
        let mut bell_detector = BellDetector::default();
        let mut deadline: Option<tokio::time::Instant> = None;
        loop {
            let timer = async {
                match deadline {
                    Some(d) => tokio::time::sleep_until(d).await,
                    None => std::future::pending::<()>().await, // 데드라인 없으면 영원히 대기
                }
            };
            tokio::select! {
                _ = timer => {
                    batcher.flush(&*sink);
                    deadline = None;
                }
                msg = rx.recv() => match msg {
                    Some(ReaderMsg::Data(bytes)) => {
                        if bell_detector.feed(&bytes) {
                            hub.on_bell(&session_id); // BEL 폴백(dedup이 연속 억제)
                        }
                        // 이슈 #39: Stop 이후 출력이 계속되면 "아직 작업중"으로 복귀시키는
                        // 휴리스틱에 바이트 수를 흘려 보낸다(Stop 감시 중이 아니면 즉시 반환).
                        hub.on_output(&session_id, bytes.len());
                        // 세션 로그 tee(docs/session-log-design.md §3.1). 채널로
                        // 던지고 잊는다 -- 파일 쓰기는 전용 스레드에서 한다.
                        if let Some(log) = log.as_ref() {
                            log.data(&bytes);
                        }
                        batcher.push(&bytes);
                        if batcher.pending_bytes() >= MAX_BYTES {
                            batcher.flush(&*sink);
                            deadline = None;
                        } else if deadline.is_none() {
                            deadline = Some(tokio::time::Instant::now()
                                + std::time::Duration::from_millis(WINDOW_MS));
                        }
                    }
                    Some(ReaderMsg::Restore(bytes)) => {
                        // §#49 함정 2: adopt 복원 스냅샷(화면 이미지)은 실시간
                        // 스트림 출력이 아니라 화면 복원이다. batcher를 거치면
                        // consumed>0으로 계수돼 offset이 부풀므로, bytes=0인 청크로
                        // 직접 방출한다. 순서 보존을 위해 혹시 남아 있을 pending을
                        // 먼저 flush한다(Restore는 항상 첫 메시지라 실제로는 없음).
                        // BEL/on_output 휴리스틱도 적용하지 않는다(실시간 출력 아님).
                        batcher.flush(&*sink);
                        deadline = None;
                        batcher.emit_uncounted(String::from_utf8_lossy(&bytes).into_owned(), &*sink);
                    }
                    Some(ReaderMsg::Eof) | None => {
                        batcher.flush_final(&*sink); // 잔여 강제 방출
                        // 로그도 잔여를 확정하고 마무리한다. Restore는 기록하지
                        // 않았으므로(화면 이미지) 여기서 새는 것은 없다.
                        if let Some(log) = log.as_ref() {
                            log.finish();
                        }
                        break;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification::hub::fake::FakeClock;
    use crate::state::{fake::RecordingEvents, SessionRegistry};
    use std::time::Duration;

    struct TestTap(tokio::sync::mpsc::UnboundedSender<OutputChunk>);

    impl OutputTap for TestTap {
        fn on_chunk(&self, chunk: &OutputChunk) {
            let _ = self.0.send(chunk.clone());
        }
    }

    #[tokio::test]
    async fn output_pump_only_notifies_for_live_standalone_bells() {
        for (messages, expected_bells) in [
            (
                vec![
                    ReaderMsg::Data(b"\x1b".to_vec()),
                    ReaderMsg::Data(b"]0;Claude: Bash\x07".to_vec()),
                    ReaderMsg::Data(b"\x1b]9;4;1;50".to_vec()),
                    ReaderMsg::Data(b"\x07tool output".to_vec()),
                ],
                0,
            ),
            (vec![ReaderMsg::Data(b"\x1b]0;Claude\x07\x07".to_vec())], 1),
            (
                vec![
                    ReaderMsg::Restore(b"restored\x07\x1b]0;unfinished".to_vec()),
                    ReaderMsg::Data(b"live output".to_vec()),
                ],
                0,
            ),
        ] {
            let registry = Arc::new(SessionRegistry::new());
            registry.insert("s1", "a1", SessionState::Running);
            let events = Arc::new(RecordingEvents::default());
            let hub = Arc::new(NotificationHub::new(
                registry,
                events.clone(),
                Arc::new(FakeClock::new()),
                Duration::from_secs(3),
            ));
            let sink = Arc::new(OutputSink::new());
            let (output_tx, mut output_rx) = tokio::sync::mpsc::unbounded_channel();
            sink.add_tap(Arc::new(TestTap(output_tx)));
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let mut expected_output = Vec::new();
            for message in messages {
                if let ReaderMsg::Data(bytes) | ReaderMsg::Restore(bytes) = &message {
                    expected_output.extend_from_slice(bytes);
                }
                tx.send(message).unwrap();
            }
            tx.send(ReaderMsg::Eof).unwrap();
            drop(tx);
            spawn_output_pump("s1".into(), "a1".into(), rx, sink, hub, None);
            let output = tokio::time::timeout(Duration::from_secs(2), async {
                let mut output = Vec::new();
                while let Some(chunk) = output_rx.recv().await {
                    output.extend_from_slice(chunk.data.as_bytes());
                }
                output
            })
            .await
            .expect("output pump must finish at EOF");
            assert_eq!(output, expected_output, "terminal bytes remain unchanged");
            let notifications = events.notifications();
            assert_eq!(notifications.len(), expected_bells);
            if expected_bells != 0 {
                assert_eq!(notifications[0].source, NotificationSource::Bell);
            }
        }
    }

    #[test]
    fn osc_updates_are_not_bells_at_any_chunk_boundary() {
        for sequence in [
            &b"\x1b]0;Claude: running tool\x07"[..],
            &b"\x1b]9;4;1;50\x07"[..],
            &b"\x1b]8;;https://example.com\x07link\x1b]8;;\x07"[..],
            &b"\x1b]2;title\x1b\\"[..],
        ] {
            for split in 0..=sequence.len() {
                let mut detector = BellDetector::default();
                assert!(!detector.feed(&sequence[..split]), "split {split}");
                assert!(!detector.feed(&sequence[split..]), "split {split}");
                assert!(detector.feed(b"\x07"), "real bell after split {split}");
            }
            let mut detector = BellDetector::default();
            for byte in sequence {
                assert!(!detector.feed(&[*byte]));
            }
        }
    }

    #[test]
    fn real_bells_preserve_state_for_the_rest_of_the_chunk() {
        let mut detector = BellDetector::default();
        assert!(detector.feed(b"\x07\x1b]0;partial"));
        assert!(!detector.feed(b" title\x07"));
        assert!(detector.feed(b"\x1b]0;title\x07\x07"));
        assert!(detector.feed(b"\x1b[31mwarning\x07\x1b[0m"));
    }

    #[test]
    fn other_control_strings_and_utf8_do_not_confuse_bell_detection() {
        for prefix in [b'P', b'X', b'^', b'_'] {
            let mut detector = BellDetector::default();
            assert!(!detector.feed(&[0x1b, prefix]));
            assert!(!detector.feed(b"payload\x07\x1b]nested\x07\x1b"));
            assert!(!detector.feed(b"\\"));
            assert!(detector.feed(b"\x07"));
        }
        for cancel in [0x18, 0x1a] {
            let mut detector = BellDetector::default();
            assert!(!detector.feed(b"\x1b]0;cancel"));
            assert!(!detector.feed(&[cancel]));
            assert!(detector.feed(b"\x07"));
        }
        let mut detector = BellDetector::default();
        // 일반 문자 ŝ의 UTF-8 연속 바이트 0x9d는 OSC 시작이 아니다.
        assert!(!detector.feed("한글ŝ".as_bytes()));
        assert!(detector.feed(b"\x07"));
    }

    #[test]
    fn escape_aborts_a_control_string_before_a_real_bell() {
        for prefix in [b']', b'P', b'X', b'^', b'_'] {
            let mut detector = BellDetector::default();
            assert!(!detector.feed(&[0x1b, prefix]));
            assert!(!detector.feed(b"partial\x1b"));
            assert!(detector.feed(b"[0m\x07"));
        }
    }
}
