// src-tauri/src/session_log/mod.rs
//
// 세션 로그(상시 터미널 전사) 서브시스템. docs/session-log-design.md 가 정본.
//
//   PTY reader → ReaderMsg::Data → output pump ─┬→ 배칭 → Channel → xterm
//                                               └→ SessionLogHandle (여기)
//                                                    → 전용 스레드
//                                                    → TranscriptFilter → SessionLogWriter
//                                                                            ↑
//   에이전트 JSONL 전사 ────────── 수집 스레드 ── TranscriptTailer ──────────┘
//   (훅이 알려 준 transcript_path, 없으면 <CLAUDE_CONFIG_DIR|~/.claude>/projects)
//
// 두 번째 경로가 필요한 이유: Claude Code·Codex는 대체 화면에서 돌아 PTY
// 전사에 대화가 남지 않는다. 대신 두 CLI가 스스로 남기는 JSONL 전사를 tail 해
// 같은 파일에 끼워 넣는다(agent_transcript/, docs/session-log-design.md §3.4).
//
// 왜 전용 스레드인가: 파일 쓰기는 블로킹이다. output pump는 모든 세션 출력이
// 지나는 async 태스크라 여기서 디스크를 기다리면 터미널 전체가 끊긴다
// (브로커 v2에서 io락 블로킹으로 이미 한 번 데인 자리다). 채널로 던지고 잊는다.
//
// 어떤 실패도 세션을 막지 않는다 -- 로그는 부가 기능이다. open 실패는 None,
// 쓰기 실패는 무시.

pub mod agent_transcript;
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
/// 에이전트 JSONL 전사를 다시 들여다보는 주기. 대화 단위 기록이라 초 단위
/// 지연은 문제가 안 되고, 폴링 비용(파일 크기 확인 몇 번)은 이 정도가 적당하다.
const INGEST_TICK: Duration = Duration::from_secs(2);

enum Msg {
    Data(Vec<u8>),
    /// 이미 사람이 읽을 수 있는 줄들(에이전트 JSONL 전사에서 온 것).
    /// 전사 필터를 거치지 않고 바로 파일로 간다.
    Lines(Vec<String>),
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
    /// 수집 스레드 종료 신호. 핸들이 사라지면 다음 틱에 스스로 끝난다.
    ingest_stop: Arc<AtomicBool>,
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
        lookup: Option<Arc<dyn agent_transcript::AgentSessionLookup>>,
    ) -> Option<Self> {
        let mut sources: Vec<Box<dyn agent_transcript::TranscriptSource>> = Vec::new();
        if let Some(lookup) = lookup {
            sources.extend(agent_transcript::claude::source(lookup));
        }
        sources.extend(agent_transcript::codex::source());
        Self::spawn_with_sources(root, agent_id, session_id, cwd, rows, enabled, sources)
    }

    /// 전사 소스를 직접 주입하는 형태. 테스트가 가짜 소스로 수집 경로 전체를
    /// 돌리는 데 쓰고, `spawn`은 홈에서 실제 소스를 찾아 이걸 부른다.
    pub fn spawn_with_sources(
        root: &Path,
        agent_id: &str,
        session_id: &str,
        cwd: &str,
        rows: u16,
        enabled: Arc<AtomicBool>,
        sources: Vec<Box<dyn agent_transcript::TranscriptSource>>,
    ) -> Option<Self> {
        let mut writer =
            SessionLogWriter::open(root, agent_id, session_id, cwd, SystemTime::now())?;
        let (tx, rx) = mpsc::sync_channel::<Msg>(QUEUE_CAP);
        let dropped = Arc::new(AtomicUsize::new(0));
        let thread_dropped = dropped.clone();
        let ingest_stop = Arc::new(AtomicBool::new(false));

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
                        // JSONL 전사에서 온 줄. PTY 쪽 라이브 영역과 섞이지
                        // 않게 먼저 그쪽을 확정하고 뒤에 붙인다 -- 그러지 않으면
                        // 셸 출력 중간에 대화가 끼어들어 순서가 뒤집혀 보인다.
                        Ok(Msg::Lines(lines)) => {
                            let mut pending = filter.flush_idle();
                            pending.extend(lines);
                            writer.write_lines(&pending, SystemTime::now());
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

        spawn_ingest(
            agent_id,
            cwd,
            sources,
            tx.clone(),
            enabled.clone(),
            ingest_stop.clone(),
        );

        Some(Self {
            tx,
            enabled,
            dropped,
            ingest_stop,
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
        self.ingest_stop.store(true, Ordering::Relaxed);
        let _ = self.tx.try_send(Msg::Eof);
    }
}

impl Drop for SessionLogHandle {
    fn drop(&mut self) {
        // 수집 스레드는 tx 사본을 들고 있다 -- 세우지 않으면 채널이 닫히지 않아
        // 기록 스레드가 Disconnected로 마무리하는 경로가 막힌다.
        self.ingest_stop.store(true, Ordering::Relaxed);
    }
}

/// 에이전트 JSONL 전사 수집 스레드. 소스가 하나도 없으면 아예 만들지 않는다.
fn spawn_ingest(
    agent_id: &str,
    cwd: &str,
    sources: Vec<Box<dyn agent_transcript::TranscriptSource>>,
    tx: SyncSender<Msg>,
    enabled: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) {
    let mut tailer = agent_transcript::TranscriptTailer::new(agent_id, cwd, sources);
    if tailer.is_empty() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name(format!("session-log-ingest-{agent_id}"))
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(INGEST_TICK);
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                // 설정이 꺼져 있으면 읽기만 하고 버린다 -- 다시 켤 때 그 사이
                // 대화가 쏟아지지 않게 오프셋은 계속 전진시킨다.
                let lines = tailer.tick();
                if lines.is_empty() || !enabled.load(Ordering::Relaxed) {
                    continue;
                }
                // 기록 스레드가 사라졌으면(세션 종료) 여기서 끝낸다.
                if let Err(mpsc::TrySendError::Disconnected(_)) = tx.try_send(Msg::Lines(lines)) {
                    break;
                }
            }
        });
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

    /// 수집 스레드는 2초 틱이라 더 오래 기다린다.
    fn wait_long(path: &Path, needle: &str) -> String {
        for _ in 0..150 {
            if let Ok(body) = std::fs::read_to_string(path) {
                if body.contains(needle) {
                    return body;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
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
            SessionLogHandle::spawn(&root, "term-1", "sess1234", "/tmp", 24, gate, None).unwrap();
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
            SessionLogHandle::spawn(&root, "term-1", "sess1234", "/tmp", 24, gate.clone(), None)
                .unwrap();
        handle.data(b"secret\r\n");
        handle.finish();
        let path = only_log(&root);
        let body = wait_for(&path, "세션 종료");
        assert!(!body.contains("secret"), "{body}");
        std::fs::remove_dir_all(&root).ok();
    }

    /// 대체 화면 안에서 도는 에이전트를 흉내낸다: PTY로는 마커만 오고 대화는
    /// JSONL 전사에서 온다. 두 경로가 한 파일에 함께 남아야 한다.
    struct FakeSource {
        path: std::path::PathBuf,
    }

    impl agent_transcript::TranscriptSource for FakeSource {
        fn label(&self) -> &'static str {
            "fake"
        }
        fn locate(&mut self, _agent_id: &str, _cwd: &str) -> Option<std::path::PathBuf> {
            self.path.exists().then(|| self.path.clone())
        }
        fn parse(&self, raw: &str) -> Vec<agent_transcript::TranscriptItem> {
            vec![agent_transcript::TranscriptItem::speech(
                agent_transcript::ItemRole::User,
                raw,
            )]
        }
        /// 이 테스트는 기록 배관만 본다 — 글리프는 파서 테스트가 지킨다.
        fn render(&self, raw: &str) -> Vec<String> {
            vec![format!("▶ {raw}")]
        }
    }

    #[test]
    fn agent_transcript_lines_land_in_the_same_log() {
        let root = scratch();
        let jsonl = root.join("fake.jsonl");
        std::fs::write(&jsonl, b"").unwrap();
        let gate = Arc::new(AtomicBool::new(true));
        let handle = SessionLogHandle::spawn_with_sources(
            &root,
            "term-1",
            "sess1234",
            "/tmp",
            24,
            gate,
            vec![Box::new(FakeSource {
                path: jsonl.clone(),
            })],
        )
        .unwrap();

        // 대체 화면 진입(=claude 시작)만 PTY로 보인다.
        handle.data(b"\x1b[?1049h");

        // 로그 파일은 agentId 하위에 있고, jsonl은 root 직하라 섞이지 않는다.
        let path = std::fs::read_dir(root.join("term-1"))
            .unwrap()
            .flatten()
            .next()
            .unwrap()
            .path();
        // 수집 스레드가 파일에 붙을 때까지(첫 틱) 기다린다.
        let body = wait_long(&path, "[fake 전사 연결:");
        assert!(body.contains("[fake 전사 연결:"), "{body}");
        assert!(body.contains(transcript::ALT_ENTER_MARKER), "{body}");

        // 붙은 뒤에 들어온 대화가 같은 파일에 이어져야 한다.
        std::fs::write(&jsonl, b"hello-from-jsonl\n").unwrap();
        let body = wait_long(&path, "hello-from-jsonl");
        assert!(body.contains("▶ hello-from-jsonl"), "{body}");
        handle.finish();
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn idle_flush_lands_lines_without_eof() {
        let root = scratch();
        let gate = Arc::new(AtomicBool::new(true));
        let handle =
            SessionLogHandle::spawn(&root, "term-1", "sess1234", "/tmp", 2, gate, None).unwrap();
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
