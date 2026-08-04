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
    /// pub: 피어 세션 공유의 뷰어 레지스트리(`peer::viewer`)가 원격 에이전트용
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
                        if bytes.contains(&0x07) {
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
