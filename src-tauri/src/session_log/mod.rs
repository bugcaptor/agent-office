// src-tauri/src/session_log/mod.rs
//
// 세션 로그(상시 터미널 전사) 서브시스템. docs/session-log-design.md 가 정본.
//
//   PTY reader → ReaderMsg::Data → output pump ─┬→ 배칭 → Channel → xterm
//                                               └→ SessionLogHandle (여기)
//                                                    → 전용 스레드
//                                                    → TranscriptFilter → SessionLogWriter
//
// 왜 전용 스레드인가: 파일 쓰기는 블로킹이다. output pump는 모든 세션 출력이
// 지나는 async 태스크라 여기서 디스크를 기다리면 터미널 전체가 끊긴다
// (브로커 v2에서 io락 블로킹으로 이미 한 번 데인 자리다). 채널로 던지고 잊는다.
//
// 어떤 실패도 세션을 막지 않는다 -- 로그는 부가 기능이다. open 실패는 None,
// 쓰기 실패는 무시.

pub mod gc;
pub mod store;
pub mod study;
pub mod transcript;

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use store::SessionLogWriter;
use transcript::TranscriptFilter;

/// 출력이 이만큼 멎으면 라이브 영역에서 화면 한 장만 남기고 확정한다.
/// (조용한 세션의 마지막 대화가 파일에 닿게 하는 경로 -- §3.6)
const IDLE_FLUSH: Duration = Duration::from_secs(3);
/// 스레드 루프의 대기 단위. 유휴 판정 해상도를 정한다.
const TICK: Duration = Duration::from_millis(500);
/// 기록 스레드로 보내는 큐의 상한(청크 수). 청크는 대개 ≤8KiB이므로 ~8MiB가
/// 하드 상한이다. 무제한 큐로 두면 `cat` 한 방에 수백 MB가 메모리에 쌓인다.
/// 넘치면 그 청크는 버리고(로그는 부가 기능) 유실 사실만 파일에 남긴다.
const QUEUE_CAP: usize = 1024;

enum Msg {
    Data(Vec<u8>),
    Rows(u16),
    Eof,
}

/// 세션 하나의 로그 기록 핸들. `Drop` 되면 스레드가 EOF로 마무리한다.
pub struct SessionLogHandle {
    tx: SyncSender<Msg>,
    /// 설정 토글(`sessionLogEnabled`). 꺼지면 즉시 흘려보낸다 -- 스레드는
    /// 살아 있으므로 다시 켜면 이어서 기록한다.
    enabled: Arc<AtomicBool>,
    /// 큐가 꽉 차 버린 청크 수. 기록 스레드가 다음 쓰기에서 털어 마커로 남긴다.
    dropped: Arc<AtomicUsize>,
}

impl SessionLogHandle {
    /// 세션 로그 기록을 시작한다. 루트가 없거나 agentId가 경로로 안전하지
    /// 않거나 파일을 못 열면 `None`(기록 없이 세션은 정상 동작).
    pub fn spawn(
        root: &Path,
        agent_id: &str,
        session_id: &str,
        cwd: &str,
        rows: u16,
        enabled: Arc<AtomicBool>,
    ) -> Option<Self> {
        let mut writer =
            SessionLogWriter::open(root, agent_id, session_id, cwd, SystemTime::now())?;
        let (tx, rx) = mpsc::sync_channel::<Msg>(QUEUE_CAP);
        let dropped = Arc::new(AtomicUsize::new(0));
        let thread_dropped = dropped.clone();

        std::thread::Builder::new()
            .name(format!("session-log-{agent_id}"))
            .spawn(move || {
                let mut filter = TranscriptFilter::new(rows);
                let mut last_data = SystemTime::now();
                let mut dirty = false;
                loop {
                    match rx.recv_timeout(TICK) {
                        Ok(Msg::Data(bytes)) => {
                            // 큐 넘침으로 버린 청크가 있으면 그 사실을 먼저 남긴다.
                            // 조용히 비는 로그보다 "여기서 빠졌다"가 낫다.
                            let missed = thread_dropped.swap(0, Ordering::Relaxed);
                            if missed > 0 {
                                writer.write_lines(
                                    &[format!("… (출력이 너무 빨라 {missed}개 조각을 건너뜀)")],
                                    SystemTime::now(),
                                );
                            }
                            let lines = filter.feed(&bytes);
                            if !lines.is_empty() {
                                writer.write_lines(&lines, SystemTime::now());
                            }
                            last_data = SystemTime::now();
                            dirty = true;
                        }
                        Ok(Msg::Rows(rows)) => filter.set_rows(rows),
                        Ok(Msg::Eof) => break,
                        Err(RecvTimeoutError::Timeout) => {
                            let idle = SystemTime::now()
                                .duration_since(last_data)
                                .unwrap_or_default();
                            if dirty && idle >= IDLE_FLUSH {
                                let lines = filter.flush_idle();
                                if !lines.is_empty() {
                                    writer.write_lines(&lines, SystemTime::now());
                                }
                                dirty = false;
                            }
                        }
                        // 핸들이 사라짐(세션 소멸) -- EOF와 같게 마무리한다.
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                let lines = filter.flush_all();
                let now = SystemTime::now();
                if !lines.is_empty() {
                    writer.write_lines(&lines, now);
                }
                writer.finish(now);
            })
            .ok()?;

        Some(Self {
            tx,
            enabled,
            dropped,
        })
    }

    /// PTY 원시 바이트. **절대 블로킹하지 않는다** -- 이 호출은 모든 세션
    /// 출력이 지나는 async 펌프 위에서 일어난다. 큐가 꽉 차면 그 청크는 버리고
    /// 카운터만 올린다(기록 스레드가 마커로 남긴다).
    pub fn data(&self, bytes: &[u8]) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        if self.tx.try_send(Msg::Data(bytes.to_vec())).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// 터미널 크기 변경 -- 라이브 영역 계산에 쓴다.
    pub fn resize(&self, rows: u16) {
        let _ = self.tx.try_send(Msg::Rows(rows));
    }

    /// 세션 종료. 잔여를 전부 확정하고 스레드를 끝낸다. 큐가 꽉 차 이 신호를
    /// 놓쳐도, 핸들이 모두 드롭되면 채널 끊김으로 같은 마무리를 탄다.
    pub fn finish(&self) {
        let _ = self.tx.try_send(Msg::Eof);
    }
}

/// 부팅 직후 1회 + 6시간마다 보존 정리를 도는 백그라운드 스레드.
/// `lib.rs`가 앱 데이터 디렉터리를 알게 된 시점에 한 번 호출한다.
pub fn spawn_gc(root: std::path::PathBuf) {
    std::thread::Builder::new()
        .name("session-log-gc".into())
        .spawn(move || loop {
            let report = gc::sweep(
                &root,
                gc::MAX_AGE,
                gc::MAX_TOTAL_BYTES,
                SystemTime::now(),
            );
            if report.removed_by_age + report.removed_by_size > 0 {
                eprintln!(
                    "session-log gc: 기간 {} 개 / 용량 {} 개 삭제, 남은 {} MB",
                    report.removed_by_age,
                    report.removed_by_size,
                    report.remaining_bytes / (1024 * 1024)
                );
            }
            std::thread::sleep(gc::SWEEP_INTERVAL);
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agent-office-session-log-handle-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 파일에 내용이 나타날 때까지 짧게 기다린다(전용 스레드라 비동기).
    fn wait_for(path: &Path, needle: &str) -> String {
        for _ in 0..100 {
            if let Ok(body) = std::fs::read_to_string(path) {
                if body.contains(needle) {
                    return body;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::read_to_string(path).unwrap_or_default()
    }

    fn only_log(root: &Path) -> std::path::PathBuf {
        let agent_dir = std::fs::read_dir(root).unwrap().flatten().next().unwrap().path();
        std::fs::read_dir(agent_dir)
            .unwrap()
            .flatten()
            .next()
            .unwrap()
            .path()
    }

    #[test]
    fn records_transcript_to_file() {
        let root = scratch();
        let gate = Arc::new(AtomicBool::new(true));
        let handle =
            SessionLogHandle::spawn(&root, "term-1", "sess1234", "/tmp", 24, gate).unwrap();
        // 라이브 영역(80줄)을 넘겨야 확정되므로 EOF로 마무리시킨다.
        handle.data(b"\x1b[32m$ echo hi\x1b[0m\r\nhi\r\n");
        handle.finish();

        let path = only_log(&root);
        let body = wait_for(&path, "echo hi");
        assert!(body.contains("$ echo hi"), "{body}");
        assert!(body.contains("\nhi\n"), "{body}");
        assert!(!body.contains("\x1b["), "ANSI가 남았다: {body:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn disabled_gate_drops_output() {
        let root = scratch();
        let gate = Arc::new(AtomicBool::new(false));
        let handle =
            SessionLogHandle::spawn(&root, "term-1", "sess1234", "/tmp", 24, gate.clone()).unwrap();
        handle.data(b"secret\r\n");
        handle.finish();
        let path = only_log(&root);
        let body = wait_for(&path, "세션 종료");
        assert!(!body.contains("secret"), "{body}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn idle_flush_lands_lines_without_eof() {
        let root = scratch();
        let gate = Arc::new(AtomicBool::new(true));
        let handle =
            SessionLogHandle::spawn(&root, "term-1", "sess1234", "/tmp", 2, gate).unwrap();
        handle.data(b"one\r\ntwo\r\nthree\r\nfour\r\n");
        // 유휴 확정(3초)까지 기다린다 -- EOF 없이도 파일에 닿아야 한다.
        let path = only_log(&root);
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        let mut body = String::new();
        while std::time::Instant::now() < deadline {
            body = std::fs::read_to_string(&path).unwrap_or_default();
            if body.contains("one") {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(body.contains("one"), "유휴 확정이 안 됐다: {body}");
        handle.finish();
        std::fs::remove_dir_all(&root).ok();
    }
}
