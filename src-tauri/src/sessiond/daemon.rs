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
// `handle_connection`에 직접 물려 구동한다.

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

/// 데몬이 세션당 보관하는 미전달 출력 상한. base64 팽창은 전송
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
    /// 종료 직전 xterm 화면 스냅샷(원본 바이트, base64 디코딩 완료). Adopt
    /// 응답에 그대로 되돌려준다 — 데몬은 이 스냅샷을 해석/가공하지 않고
    /// 불투명한 바이트열로만 보관한다.
    snapshot: Vec<u8>,
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
    /// 리더를 확정적으로 멈춘다(핸드오프의 데몬 측 절반: reader
    /// 스레드를 정지시킨 뒤 fd를 반환한다). `master_fd`는 그대로 열려 있다
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

// ── 오피코드별 핸들러 ──────────────────────────────────────────────────
//
// 각 함수는 `handle_connection`의 원래 match arm 몸체를 그대로 옮긴 것이다
// (락 획득 지점·범위·순서, 응답 프레임, 에러 처리 전부 불변). 대부분은 응답을
// 직접 소켓에 쓰고 반환하며, `handle_connection`은 파싱한 필드를 그대로
// 넘기는 얇은 디스패처로 남는다.

/// `Hello`: 클라이언트가 요청한 proto를 데몬 상한으로 낮춰 수락한다
/// (HelloOk{min(proto, PROTO_VERSION)}). proto 0은 유효한 버전이 아니므로
/// 거부한다.
fn handle_hello(fd: RawFd, proto: u32) {
    let reply = if proto >= 1 {
        Message::HelloOk { proto: proto.min(protocol::PROTO_VERSION) }
    } else {
        Message::Error { message: format!("unsupported protocol version {proto}") }
    };
    let _ = protocol::write_frame(fd, &reply, None);
}

/// `Handoff`: 앱이 넘긴 세션을 v1 테이블에 등록한다. 반환값 `false`는 원래
/// `continue`(fd 없이 온 잘못된 요청)에 해당한다 -- 호출자가 루프를 이어간다.
#[allow(clippy::too_many_arguments)]
fn handle_handoff(
    fd: RawFd,
    table: &Table,
    ever_handoff: &AtomicBool,
    recv_fd: Option<RawFd>,
    agent_id: String,
    session_id: String,
    pid: Option<i32>,
    pgid: Option<i32>,
    rows: u16,
    cols: u16,
    cwd: String,
    cleanup_paths: Vec<String>,
    snapshot_b64: String,
) {
    let Some(master_fd) = recv_fd else {
        let _ = protocol::write_frame(
            fd,
            &Message::Error { message: "Handoff must carry a master fd".into() },
            None,
        );
        return;
    };
    // 디코딩 실패(손상된 base64 등)는 스냅샷 없음으로 취급한다 -- fd는 이미
    // 받았으므로 핸드오프 자체를 거부하지 않는다(핸드오프 실패
    // 시에도 세션 표시가 깨지면 안 된다는 원칙).
    let snapshot = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.decode(&snapshot_b64).unwrap_or_default()
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
                snapshot,
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

/// `List`: 테이블의 세션 목록을 돌려준다.
fn handle_list(fd: RawFd, table: &Table) {
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

/// `Adopt`(v1): 리더를 확정적으로 멈춘 뒤 fd를 응답에 실어 돌려준다.
fn handle_adopt(fd: RawFd, table: &Table, agent_id: String) {
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
            // 리더를 먼저 확정적으로 멈춘 뒤 fd를 보낸다.
            entry.stop_reader();
            let buffer_b64 = {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .encode(entry.ring.lock().unwrap().snapshot())
            };
            let snapshot_b64 = {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(&entry.snapshot)
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
                snapshot_b64,
            };
            let _ = protocol::write_frame(fd, &reply, Some(entry.master_fd));
            let _ = nix::unistd::close(entry.master_fd);
            entry.consumed = true;
        }
    }
}

/// `Kill`: 테이블 항목의 프로세스 그룹을 죽이고 fd를 닫는다.
fn handle_kill(fd: RawFd, table: &Table, agent_id: String) {
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

/// 알려지지 않은/예상 밖 메시지: 에러 프레임을 돌려준다.
fn handle_unexpected(fd: RawFd, other: Message) {
    let _ = protocol::write_frame(
        fd,
        &Message::Error { message: format!("unexpected message: {other:?}") },
        None,
    );
}

/// 연결 하나(=하나의 fd, Hello 이후 여러 요청을 순차로 받을 수 있다)를
/// 소진될 때까지(연결 종료/오류) 처리한다. 실 accept 루프와 테스트
/// (socketpair) 양쪽에서 동일하게 쓰는 핵심 로직. 요청을 파싱해 오피코드별
/// 핸들러로 디스패치하는 얇은 라우터 -- 프로토콜 바이트/순서/락 범위는
/// 각 `handle_*` 함수 안에 원래 그대로 보존돼 있다.
fn handle_connection(fd: RawFd, table: &Table, ever_handoff: &AtomicBool) {
    loop {
        let (msg, recv_fd) = match protocol::read_frame(fd) {
            Ok(v) => v,
            Err(_) => return, // 연결 종료 또는 프로토콜 오류 -- 이 연결은 여기까지.
        };
        match msg {
            Message::Hello { proto } => handle_hello(fd, proto),
            Message::Handoff {
                agent_id,
                session_id,
                pid,
                pgid,
                rows,
                cols,
                cwd,
                cleanup_paths,
                snapshot_b64,
            } => handle_handoff(
                fd,
                table,
                ever_handoff,
                recv_fd,
                agent_id,
                session_id,
                pid,
                pgid,
                rows,
                cols,
                cwd,
                cleanup_paths,
                snapshot_b64,
            ),
            Message::List => handle_list(fd, table),
            Message::Adopt { agent_id } => handle_adopt(fd, table, agent_id),
            Message::Kill { agent_id } => handle_kill(fd, table, agent_id),
            other => handle_unexpected(fd, other),
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
            // 연결이 끊길 때마다 테이블이 비어 있으면 종료.
            if table.lock().unwrap().is_empty() {
                hook();
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
