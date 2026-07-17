// src-tauri/src/sessiond/daemon.rs
//
// 세션 핸드오프 데몬 본체(unix 전용). `main.rs`가 `--sessiond <socket_path>`로
// 재실행할 때 `run_daemon()`이 이 파일의 진입점이다. 앱이 죽어도 넘겨받은
// PTY 마스터 fd를 쥐고 있는 한 세션(셸/claude)은 살아있다 -- 데몬의 유일한
// 책임은 그 fd들과 세션 메타데이터, 그리고 앱이 재시작해 돌아올 때까지의
// 출력 링버퍼를 보관하는 것.
//
// 테이블 로직(SessionEntry/handle_connection)은 실 `UnixListener`
// accept 루프와 분리돼 있다 -- 테스트는 `socketpair`로 서버 쪽 fd 하나를
// `handle_connection`에 직접 물려 구동한다(§테스트).

use std::collections::{HashMap, VecDeque};
use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;

use super::protocol::{self, Message, SessionInfo};

/// 데몬이 세션당 보관하는 미전달 출력 상한(§프로토콜). base64 팽창은 전송
/// 시점(AdoptOk)에만 계산 — 보관은 원본 바이트로.
const RING_CAPACITY: usize = 512 * 1024;

/// 기동 후 이 시간 안에 Handoff가 하나도 없으면 고아 데몬으로 보고 종료.
const FIRST_HANDOFF_TIMEOUT: Duration = Duration::from_secs(60);

struct RingBuffer {
    buf: VecDeque<u8>,
    cap: usize,
}

impl RingBuffer {
    fn new(cap: usize) -> Self {
        Self { buf: VecDeque::with_capacity(cap.min(64 * 1024)), cap }
    }

    fn push(&mut self, data: &[u8]) {
        if data.len() >= self.cap {
            self.buf.clear();
            self.buf.extend(&data[data.len() - self.cap..]);
            return;
        }
        let overflow = (self.buf.len() + data.len()).saturating_sub(self.cap);
        for _ in 0..overflow {
            self.buf.pop_front();
        }
        self.buf.extend(data.iter().copied());
    }

    fn snapshot(&self) -> Vec<u8> {
        self.buf.iter().copied().collect()
    }

    fn len(&self) -> usize {
        self.buf.len()
    }
}

struct SessionEntry {
    session_id: String,
    pid: Option<i32>,
    pgid: Option<i32>,
    rows: u16,
    cols: u16,
    cwd: String,
    cleanup_paths: Vec<String>,
    master_fd: RawFd,
    ring: Arc<Mutex<RingBuffer>>,
    exited: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    interrupt: crate::session::poll_reader::ReaderInterrupt,
    reader_join: Option<std::thread::JoinHandle<()>>,
    /// Adopt/Kill 경로가 정지+close를 이미 수동으로 마쳤으면 true -- Drop이
    /// 같은 작업을 중복 수행하지 않게 막는 가드.
    consumed: bool,
}

impl SessionEntry {
    /// 리더를 확정적으로 멈춘다(§핵심 1의 데몬 측 절반: "데몬이 reader
    /// 스레드를 정지시킨 뒤 fd를 반환한다"). `master_fd`는 그대로 열려 있다
    /// -- 닫기는 호출자 책임(Adopt는 전송 후, Kill은 즉시).
    fn stop_reader(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.interrupt.interrupt();
        if let Some(j) = self.reader_join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for SessionEntry {
    fn drop(&mut self) {
        if self.consumed {
            return;
        }
        // 안전망: 중복 Handoff로 덮어써진 옛 엔트리, 또는 테이블이 통째로
        // 버려지는 경우(있다면) -- 정지 후 fd를 닫아 누수를 막는다.
        self.stop_reader();
        let _ = nix::unistd::close(self.master_fd);
    }
}

fn spawn_reader(
    master_fd: RawFd,
    ring: Arc<Mutex<RingBuffer>>,
    exited: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
) -> io::Result<(crate::session::poll_reader::ReaderInterrupt, std::thread::JoinHandle<()>)> {
    let (mut reader, interrupt) = crate::session::poll_reader::spawn(master_fd)?;
    let handle = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => ring.lock().unwrap().push(&buf[..n]),
                Err(_) => break,
            }
        }
        // stopping이 true면 우리가 의도적으로(Adopt/Kill) 인터럽트한 것 --
        // 그 외에는 커널 fd 자체가 EOF/에러를 반환한 것이므로 실제 프로세스
        // 종료로 간주한다.
        if !stopping.load(Ordering::SeqCst) {
            exited.store(true, Ordering::SeqCst);
        }
    });
    Ok((interrupt, handle))
}

type Table = Mutex<HashMap<String, SessionEntry>>;

/// 연결 하나(=하나의 fd, Hello 이후 여러 요청을 순차로 받을 수 있다)를
/// 소진될 때까지(연결 종료/오류) 처리한다. 실 accept 루프와 테스트
/// (socketpair) 양쪽에서 동일하게 쓰는 핵심 로직.
fn handle_connection(fd: RawFd, table: &Table, ever_handoff: &AtomicBool) {
    loop {
        let (msg, recv_fd) = match protocol::read_frame(fd) {
            Ok(v) => v,
            Err(_) => return, // 연결 종료 또는 프로토콜 오류 -- 이 연결은 여기까지.
        };
        match msg {
            Message::Hello { proto } => {
                let reply = if proto == protocol::PROTO_VERSION {
                    Message::HelloOk { proto: protocol::PROTO_VERSION }
                } else {
                    Message::Error {
                        message: format!("unsupported protocol version {proto}"),
                    }
                };
                let _ = protocol::write_frame(fd, &reply, None);
            }
            Message::Handoff {
                agent_id,
                session_id,
                pid,
                pgid,
                rows,
                cols,
                cwd,
                cleanup_paths,
            } => {
                let Some(master_fd) = recv_fd else {
                    let _ = protocol::write_frame(
                        fd,
                        &Message::Error { message: "Handoff must carry a master fd".into() },
                        None,
                    );
                    continue;
                };
                let ring = Arc::new(Mutex::new(RingBuffer::new(RING_CAPACITY)));
                let exited = Arc::new(AtomicBool::new(false));
                let stopping = Arc::new(AtomicBool::new(false));
                match spawn_reader(master_fd, ring.clone(), exited.clone(), stopping.clone()) {
                    Ok((interrupt, reader_join)) => {
                        let entry = SessionEntry {
                            session_id,
                            pid,
                            pgid,
                            rows,
                            cols,
                            cwd,
                            cleanup_paths,
                            master_fd,
                            ring,
                            exited,
                            stopping,
                            interrupt,
                            reader_join: Some(reader_join),
                            consumed: false,
                        };
                        table.lock().unwrap().insert(agent_id, entry);
                        ever_handoff.store(true, Ordering::SeqCst);
                        let _ = protocol::write_frame(fd, &Message::HandoffOk, None);
                    }
                    Err(e) => {
                        let _ = nix::unistd::close(master_fd);
                        let _ = protocol::write_frame(
                            fd,
                            &Message::Error { message: format!("failed to start reader: {e}") },
                            None,
                        );
                    }
                }
            }
            Message::List => {
                let sessions: Vec<SessionInfo> = table
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|(agent_id, e)| SessionInfo {
                        agent_id: agent_id.clone(),
                        session_id: e.session_id.clone(),
                        pid: e.pid,
                        rows: e.rows,
                        cols: e.cols,
                        cwd: e.cwd.clone(),
                        exited: e.exited.load(Ordering::SeqCst),
                        buffered_bytes: e.ring.lock().unwrap().len(),
                    })
                    .collect();
                let _ = protocol::write_frame(fd, &Message::ListOk { sessions }, None);
            }
            Message::Adopt { agent_id } => {
                let entry = table.lock().unwrap().remove(&agent_id);
                match entry {
                    None => {
                        let _ = protocol::write_frame(
                            fd,
                            &Message::Error { message: format!("unknown agent_id: {agent_id}") },
                            None,
                        );
                    }
                    Some(mut entry) => {
                        // 리더를 먼저 확정적으로 멈춘 뒤(§핵심 1) fd를 보낸다.
                        entry.stop_reader();
                        let buffer_b64 = {
                            use base64::Engine;
                            base64::engine::general_purpose::STANDARD
                                .encode(entry.ring.lock().unwrap().snapshot())
                        };
                        let reply = Message::AdoptOk {
                            agent_id: agent_id.clone(),
                            session_id: entry.session_id.clone(),
                            pid: entry.pid,
                            pgid: entry.pgid,
                            rows: entry.rows,
                            cols: entry.cols,
                            cwd: entry.cwd.clone(),
                            cleanup_paths: entry.cleanup_paths.clone(),
                            buffer_b64,
                        };
                        let _ = protocol::write_frame(fd, &reply, Some(entry.master_fd));
                        let _ = nix::unistd::close(entry.master_fd);
                        entry.consumed = true;
                    }
                }
            }
            Message::Kill { agent_id } => {
                if let Some(mut entry) = table.lock().unwrap().remove(&agent_id) {
                    if let Some(pgid) = entry.pgid.or(entry.pid) {
                        let _ = killpg(Pid::from_raw(pgid), Signal::SIGKILL);
                    }
                    entry.stop_reader();
                    let _ = nix::unistd::close(entry.master_fd);
                    entry.consumed = true;
                }
                let _ = protocol::write_frame(fd, &Message::KillOk, None);
            }
            other => {
                let _ = protocol::write_frame(
                    fd,
                    &Message::Error { message: format!("unexpected message: {other:?}") },
                    None,
                );
            }
        }
    }
}

/// 데몬이 스스로 종료하기로 판단했을 때 실행할 동작. 실 프로세스에서는
/// 소켓 파일 삭제 + `process::exit(0)`이지만, 테스트는 이 훅을 채널
/// 신호로 바꿔치기해 "언제 종료 조건이 성립하는지"를 실제 프로세스를
/// 죽이지 않고 검증한다(같은 테스트 바이너리 안에서 `process::exit`를
/// 부르면 다른 모든 테스트까지 함께 죽는다).
pub(crate) type ShutdownHook = Arc<dyn Fn() + Send + Sync>;

fn default_shutdown_hook(socket_path: PathBuf) -> ShutdownHook {
    Arc::new(move || {
        let _ = std::fs::remove_file(&socket_path);
        std::process::exit(0);
    })
}

/// 프로세스 진입점. main.rs의 `--sessiond <socket_path>` 분기에서 호출.
/// stdio 리다이렉트는 스폰하는 앱 쪽 책임(client.rs) -- 여기서는 하지 않는다.
pub fn run_daemon(socket_path: PathBuf) -> i32 {
    let hook = default_shutdown_hook(socket_path.clone());
    match run_daemon_inner(socket_path, FIRST_HANDOFF_TIMEOUT, hook) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("agent-office sessiond: fatal: {e}");
            1
        }
    }
}

/// `pub(crate)`: client.rs의 테스트가 실 `UnixListener` 배선(accept 루프 +
/// 종료 훅)까지 통째로 검증할 때, 프로세스를 죽이지 않는 안전한 훅을 넣어
/// 재사용한다.
pub(crate) fn run_daemon_inner(
    socket_path: PathBuf,
    first_handoff_timeout: Duration,
    on_shutdown: ShutdownHook,
) -> io::Result<()> {
    let _ = std::fs::remove_file(&socket_path); // 이전에 죽은 데몬이 남긴 소켓 파일 정리
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(&socket_path)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
    }

    let table: Arc<Table> = Arc::new(Mutex::new(HashMap::new()));
    let ever_handoff = Arc::new(AtomicBool::new(false));

    {
        let ever = ever_handoff.clone();
        let hook = on_shutdown.clone();
        std::thread::spawn(move || {
            std::thread::sleep(first_handoff_timeout);
            if !ever.load(Ordering::SeqCst) {
                hook();
            }
        });
    }

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        let table = table.clone();
        let ever = ever_handoff.clone();
        let hook = on_shutdown.clone();
        std::thread::spawn(move || {
            handle_connection(stream.as_raw_fd(), &table, &ever);
            drop(stream);
            // 연결이 끊길 때마다 테이블이 비어 있으면 종료(§프로토콜 "데몬 수명").
            if table.lock().unwrap().is_empty() {
                hook();
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::sys::socket::{socketpair, AddressFamily, SockFlag, SockType};
    use nix::unistd::{close, pipe, read as nix_read, write as nix_write};
    use std::time::Duration;

    /// 하나의 socketpair 절반을 `handle_connection`에 물려 백그라운드
    /// 스레드로 돌리고, 다른 절반(`client_fd`)을 테스트가 직접
    /// read_frame/write_frame으로 구동한다. 테이블은 테스트가 직접 훑어
    /// 검증할 수 있게 통째로 반환.
    struct Harness {
        client_fd: RawFd,
        table: Arc<Table>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl Harness {
        fn new() -> Self {
            let (client_fd, server_fd) =
                socketpair(AddressFamily::Unix, SockType::Stream, None, SockFlag::empty()).unwrap();
            let table: Arc<Table> = Arc::new(Mutex::new(HashMap::new()));
            let ever = Arc::new(AtomicBool::new(false));
            let table_for_thread = table.clone();
            let handle = std::thread::spawn(move || {
                handle_connection(server_fd, &table_for_thread, &ever);
                let _ = close(server_fd);
            });
            Harness { client_fd, table, handle: Some(handle) }
        }

        fn send(&self, msg: &Message, fd: Option<RawFd>) {
            protocol::write_frame(self.client_fd, msg, fd).unwrap();
        }

        fn recv(&self) -> (Message, Option<RawFd>) {
            protocol::read_frame(self.client_fd).unwrap()
        }

        fn finish(mut self) {
            let _ = close(self.client_fd);
            if let Some(h) = self.handle.take() {
                h.join().unwrap();
            }
        }
    }

    /// 짧고 프로세스/스레드 조합으로 유일한 문자열(소켓 경로 길이 상한 대응).
    fn short_id() -> String {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("{:x}{:x}", std::process::id(), n)
    }

    fn wait_until<F: Fn() -> bool>(pred: F) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !pred() {
            assert!(std::time::Instant::now() < deadline, "condition not met within timeout");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn hello_ok_on_matching_protocol_version() {
        let h = Harness::new();
        h.send(&Message::Hello { proto: protocol::PROTO_VERSION }, None);
        let (reply, fd) = h.recv();
        assert!(fd.is_none());
        assert!(matches!(reply, Message::HelloOk { proto } if proto == protocol::PROTO_VERSION));
        h.finish();
    }

    #[test]
    fn hello_errors_on_protocol_mismatch() {
        let h = Harness::new();
        h.send(&Message::Hello { proto: 99 }, None);
        let (reply, _) = h.recv();
        assert!(matches!(reply, Message::Error { .. }));
        h.finish();
    }

    #[test]
    fn handoff_registers_session_and_list_reflects_it() {
        let h = Harness::new();
        let (master_read, master_write) = pipe().unwrap();

        h.send(
            &Message::Handoff {
                agent_id: "a1".into(),
                session_id: "s1".into(),
                pid: Some(111),
                pgid: Some(111),
                rows: 24,
                cols: 80,
                cwd: "/tmp/work".into(),
                cleanup_paths: vec!["/tmp/settings.json".into()],
            },
            Some(master_read),
        );
        let _ = close(master_read); // 전송측 사본은 곧바로 닫아도 무방(데몬이 독립 사본을 받음)
        let (reply, _) = h.recv();
        assert!(matches!(reply, Message::HandoffOk));
        assert_eq!(h.table.lock().unwrap().len(), 1);

        h.send(&Message::List, None);
        let (reply, _) = h.recv();
        match reply {
            Message::ListOk { sessions } => {
                assert_eq!(sessions.len(), 1);
                assert_eq!(sessions[0].agent_id, "a1");
                assert_eq!(sessions[0].session_id, "s1");
                assert_eq!(sessions[0].pid, Some(111));
                assert_eq!(sessions[0].cwd, "/tmp/work");
                assert!(!sessions[0].exited);
                assert_eq!(sessions[0].buffered_bytes, 0);
            }
            other => panic!("unexpected reply: {other:?}"),
        }

        nix_write(master_write, b"hello from pty").unwrap();
        wait_until(|| {
            h.send(&Message::List, None);
            let (reply, _) = h.recv();
            matches!(&reply, Message::ListOk { sessions } if sessions[0].buffered_bytes > 0)
        });

        let _ = close(master_write);
        h.finish();
    }

    #[test]
    fn adopt_removes_from_table_stops_reader_and_hands_back_buffered_output() {
        let h = Harness::new();
        let (master_read, master_write) = pipe().unwrap();

        h.send(
            &Message::Handoff {
                agent_id: "a1".into(),
                session_id: "s1".into(),
                pid: Some(222),
                pgid: Some(222),
                rows: 24,
                cols: 80,
                cwd: "/tmp".into(),
                cleanup_paths: vec![],
            },
            Some(master_read),
        );
        let _ = close(master_read);
        let (reply, _) = h.recv();
        assert!(matches!(reply, Message::HandoffOk));

        nix_write(master_write, b"buffered-before-adopt").unwrap();
        wait_until(|| h.table.lock().unwrap()["a1"].ring.lock().unwrap().len() > 0);

        h.send(&Message::Adopt { agent_id: "a1".into() }, None);
        let (reply, fd) = h.recv();
        let adopted_fd = fd.expect("AdoptOk must carry the master fd");
        match reply {
            Message::AdoptOk { agent_id, session_id, pid, buffer_b64, .. } => {
                assert_eq!(agent_id, "a1");
                assert_eq!(session_id, "s1");
                assert_eq!(pid, Some(222));
                use base64::Engine;
                let decoded = base64::engine::general_purpose::STANDARD.decode(buffer_b64).unwrap();
                assert_eq!(decoded, b"buffered-before-adopt");
            }
            other => panic!("unexpected reply: {other:?}"),
        }
        assert!(
            h.table.lock().unwrap().is_empty(),
            "Adopt must remove the entry from the table"
        );

        // 데몬 쪽 리더는 정지됐으니, 입양된 fd로 이어 쓴 바이트는 우리가
        // 받은 fd에서 직접 읽혀야 한다(이중 리더가 아니라는 증거).
        nix_write(master_write, b"after-adopt").unwrap();
        let mut buf = [0u8; 32];
        let n = nix_read(adopted_fd, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"after-adopt");

        let _ = close(adopted_fd);
        let _ = close(master_write);
        h.finish();
    }

    #[test]
    fn adopt_of_unknown_agent_returns_error() {
        let h = Harness::new();
        h.send(&Message::Adopt { agent_id: "ghost".into() }, None);
        let (reply, fd) = h.recv();
        assert!(fd.is_none());
        assert!(matches!(reply, Message::Error { .. }));
        h.finish();
    }

    #[test]
    fn kill_removes_from_table_and_closes_master_fd() {
        let h = Harness::new();
        let (master_read, master_write) = pipe().unwrap();
        h.send(
            &Message::Handoff {
                agent_id: "a1".into(),
                session_id: "s1".into(),
                pid: Some(333),
                pgid: Some(333),
                rows: 24,
                cols: 80,
                cwd: "/tmp".into(),
                cleanup_paths: vec![],
            },
            Some(master_read),
        );
        let _ = close(master_read);
        h.recv();

        h.send(&Message::Kill { agent_id: "a1".into() }, None);
        let (reply, _) = h.recv();
        assert!(matches!(reply, Message::KillOk));
        assert!(h.table.lock().unwrap().is_empty());

        let _ = close(master_write);
        h.finish();
    }

    #[test]
    fn reader_marks_session_exited_on_real_eof_not_on_adopt_interrupt() {
        let h = Harness::new();
        let (master_read, master_write) = pipe().unwrap();
        h.send(
            &Message::Handoff {
                agent_id: "a1".into(),
                session_id: "s1".into(),
                pid: Some(444),
                pgid: Some(444),
                rows: 24,
                cols: 80,
                cwd: "/tmp".into(),
                cleanup_paths: vec![],
            },
            Some(master_read),
        );
        let _ = close(master_read);
        h.recv();

        // 쓰기 끝을 닫아 "프로세스 종료"를 흉내낸다 -- 리더는 진짜 EOF를 본다.
        let _ = close(master_write);

        wait_until(|| {
            h.send(&Message::List, None);
            let (reply, _) = h.recv();
            matches!(&reply, Message::ListOk { sessions } if sessions[0].exited)
        });

        h.finish();
    }

    #[test]
    fn table_becomes_empty_after_adopting_the_only_session() {
        let h = Harness::new();
        let (master_read, master_write) = pipe().unwrap();
        h.send(
            &Message::Handoff {
                agent_id: "solo".into(),
                session_id: "s1".into(),
                pid: Some(555),
                pgid: Some(555),
                rows: 24,
                cols: 80,
                cwd: "/tmp".into(),
                cleanup_paths: vec![],
            },
            Some(master_read),
        );
        let _ = close(master_read);
        h.recv();
        assert_eq!(h.table.lock().unwrap().len(), 1);

        h.send(&Message::Adopt { agent_id: "solo".into() }, None);
        let (_, fd) = h.recv();
        assert!(h.table.lock().unwrap().is_empty());

        let _ = close(fd.unwrap());
        let _ = close(master_write);
        h.finish();
    }

    /// 실 `UnixListener` accept 루프(`run_daemon_inner`)까지 통째로 검증:
    /// 소켓에 실제로 connect해 Handoff/List/Adopt를 왕복하고, 세션이 하나
    /// 뿐이던 테이블이 Adopt로 비워진 뒤 연결을 끊으면 종료 훅이 정확히
    /// 한 번 불리는지 확인한다. `on_shutdown`을 채널로 바꿔치기해
    /// `process::exit`가 테스트 프로세스를 죽이지 않게 한다.
    #[test]
    fn run_daemon_inner_fires_shutdown_hook_once_table_empties_after_disconnect() {
        use std::os::unix::net::UnixStream;

        // macOS/BSD sockaddr_un.sun_path 상한(~104바이트) 안에 들어가도록
        // 짧은 경로를 쓴다 -- temp_dir() + 서술적인 이름은 쉽게 초과한다.
        let dir = std::env::temp_dir().join(format!("ao-sd-{}", short_id()));
        let _ = std::fs::create_dir_all(&dir);
        let socket_path = dir.join("s.sock");

        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel::<()>();
        let hook: ShutdownHook = Arc::new(move || {
            let _ = shutdown_tx.send(());
        });

        let socket_for_daemon = socket_path.clone();
        std::thread::spawn(move || {
            let _ = run_daemon_inner(socket_for_daemon, Duration::from_secs(60), hook);
        });

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !socket_path.exists() {
            assert!(std::time::Instant::now() < deadline, "daemon never bound the socket");
            std::thread::sleep(Duration::from_millis(10));
        }

        let stream = UnixStream::connect(&socket_path).unwrap();
        let fd = stream.as_raw_fd();
        protocol::write_frame(fd, &Message::Hello { proto: protocol::PROTO_VERSION }, None).unwrap();
        assert!(matches!(protocol::read_frame(fd).unwrap().0, Message::HelloOk { .. }));

        let (master_read, master_write) = pipe().unwrap();
        protocol::write_frame(
            fd,
            &Message::Handoff {
                agent_id: "only".into(),
                session_id: "s1".into(),
                pid: Some(1),
                pgid: Some(1),
                rows: 24,
                cols: 80,
                cwd: "/tmp".into(),
                cleanup_paths: vec![],
            },
            Some(master_read),
        )
        .unwrap();
        let _ = close(master_read);
        assert!(matches!(protocol::read_frame(fd).unwrap().0, Message::HandoffOk));

        protocol::write_frame(fd, &Message::Adopt { agent_id: "only".into() }, None).unwrap();
        let (reply, adopted_fd) = protocol::read_frame(fd).unwrap();
        assert!(matches!(reply, Message::AdoptOk { .. }));
        let _ = close(adopted_fd.unwrap());
        let _ = close(master_write);

        // 종료 훅은 "연결이 끊길 때" 평가되므로, 아직 연결을 안 끊은
        // 지금은 신호가 오면 안 된다.
        assert!(shutdown_rx.try_recv().is_err());

        drop(stream); // 연결 종료 -> 테이블이 비었으니 훅이 불려야 한다.
        shutdown_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("shutdown hook must fire once the table empties after disconnect");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
