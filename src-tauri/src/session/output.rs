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
}
impl OutputSink {
    pub(super) fn new() -> Self {
        Self {
            channel: Mutex::new(None),
            backlog: Mutex::new(Default::default()),
        }
    }
    pub(super) fn attach(&self, ch: Channel<OutputChunk>) {
        // 락 순서 항상 channel → backlog (데드락 방지, emit과 동일 순서).
        let mut c = self.channel.lock();
        let mut b = self.backlog.lock();
        for chunk in b.drain(..) {
            let _ = ch.send(chunk);
        }
        *c = Some(ch);
    }
    pub(super) fn detach(&self) {
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

pub(super) fn spawn_output_pump(
    session_id: String,
    agent_id: String,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<ReaderMsg>,
    sink: Arc<OutputSink>,
    hub: Arc<NotificationHub>,
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
                        break;
                    }
                }
            }
        }
    });
}
