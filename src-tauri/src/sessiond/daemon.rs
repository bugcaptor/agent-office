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
use std::io::{self, Read, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use nix::sys::signal::{killpg, Signal};
use nix::sys::socket::{shutdown, Shutdown};
use nix::unistd::Pid;

use super::protocol::{self, write_all_raw, ExitStatusMsg, Message, SessionInfo};

/// 데몬이 세션당 보관하는 미전달 출력 상한(§프로토콜). base64 팽창은 전송
/// 시점(AdoptOk)에만 계산 — 보관은 원본 바이트로.
const RING_CAPACITY: usize = 512 * 1024;

/// 기동 후 이 시간 안에 Handoff가 하나도 없으면 고아 데몬으로 보고 종료.
const FIRST_HANDOFF_TIMEOUT: Duration = Duration::from_secs(60);

struct RingBuffer {
    buf: VecDeque<u8>,
    cap: usize,
    /// 이 링에 지금까지 push된 누적 총 바이트 수(롤오버로 버려진 것 포함).
    /// `snapshot_offset` 기반 부분 리플레이(§P2-b)의 좌표계 -- 링 안 첫 바이트의
    /// 누적 인덱스는 `total - buf.len()`이다.
    total: u64,
}

impl RingBuffer {
    fn new(cap: usize) -> Self {
        Self { buf: VecDeque::with_capacity(cap.min(64 * 1024)), cap, total: 0 }
    }

    fn push(&mut self, data: &[u8]) {
        self.total = self.total.saturating_add(data.len() as u64);
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

    /// 누적 인덱스 `offset` 이후의 바이트만 돌려준다(링에 남은 범위로 클램프).
    /// 스냅샷 이후 출력만 리플레이하는 데 쓴다: `offset`이 링 시작보다 앞이면
    /// (오래돼 이미 롤오버) 링 전체를, 링 끝 이후면 빈 벡터를 준다.
    fn snapshot_since(&self, offset: u64) -> Vec<u8> {
        let ring_start = self.total - self.buf.len() as u64; // 링 안 첫 바이트의 누적 인덱스
        if offset <= ring_start {
            return self.snapshot();
        }
        let skip = (offset - ring_start) as usize;
        if skip >= self.buf.len() {
            return Vec::new();
        }
        self.buf.iter().skip(skip).copied().collect()
    }

    fn total(&self) -> u64 {
        self.total
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

// ── v2 상시 브로커 세션 ────────────────────────────────────────────────
//
// v1(SessionEntry)은 앱이 소유하던 fd를 넘겨받아 보관만 하지만, v2 브로커
// 세션은 데몬이 portable-pty로 직접 openpty+spawn해 PTY master와 자식을
// 소유한다. 세션당:
//   - reader 스레드: master 출력을 링버퍼에 쌓고, 활성 data conn이 있으면
//     같은 바이트를 그 소켓에도 흘린다(라이브).
//   - waiter 스레드: 자식을 reap(waitpid)해 종료 정보를 기록하고, 남은
//     출력을 전부 흘린 뒤 data conn을 닫는다.
//   - data conn: 앱이 DataAttach로 붙이는 raw 양방향 소켓. handle_connection
//     스레드 자신이 이 소켓의 raw 입력(앱->master)을 담당하고, reader 스레드가
//     출력(master->앱)을 담당한다. 세션당 1개만 활성 -- 새 DataAttach가 오면
//     기존 소켓을 shutdown해 교체한다.

/// 활성 data conn 핸들. `fd`는 handle_connection 스레드가 소유한 UnixStream의
/// 빌린 fd -- 그 스레드가 raw 입력 루프를 도는 동안 유효하다. `gen`은 교체
/// 판정용(오래된 conn이 자기 자리를 잘못 비우지 않게).
struct DataConn {
    fd: RawFd,
    gen: u64,
}

struct BrokerIo {
    ring: RingBuffer,
    conn: Option<DataConn>,
    /// 마지막 스냅샷이 반영하는 링 누적 오프셋(§P1/§P2-b). Some이면 DataAttach
    /// 리플레이가 이 오프셋 이후 바이트만 흘린다(앱은 스냅샷을 initial_output으로
    /// 별도 주입). None이면 링 전체 리플레이.
    snapshot_offset: Option<u64>,
    /// 자식이 이미 reap돼 이 세션의 data 스트림이 영구히 닫혔는지(§P2-b).
    /// waiter가 conn을 정리한 *뒤* 도착한 DataAttach가 새 conn을 설치하면 아무도
    /// 닫아주지 않아 앱 reader가 영원히 블록되는 레이스를 막는다 -- 같은 io 락
    /// 아래에서 waiter의 정리와 DataAttach의 설치를 직렬화한다.
    closed: bool,
}

/// 종료 정보(reap 결과). portable-pty의 `ExitStatus`는 exit code만 노출하므로
/// signal은 항상 None이다(v1 EofWaiter와 동일한 한계).
#[derive(Clone, Copy)]
struct ExitRecord {
    exit_code: Option<i32>,
    signal: Option<i32>,
}

struct BrokerSession {
    session_id: String,
    /// 현재 지오메트리 -- Resize 성공 시 갱신되어 List/Attach가 최신 값을 준다(§P2-c).
    rows: AtomicU16,
    cols: AtomicU16,
    cwd: String,
    pid: Option<i32>,
    cleanup_paths: Vec<String>,
    /// 링버퍼 + 활성 data conn(하나의 락으로 묶어 리플레이/교체를 원자화).
    io: Mutex<BrokerIo>,
    /// master로 raw 입력을 쓰는 라이터(data conn 스레드가 사용).
    input: Mutex<Box<dyn Write + Send>>,
    /// resize용 master(portable-pty MasterPty::resize).
    master: Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    /// 의도적 종료(Kill/KillAll)용 killer.
    killer: Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
    /// 자식 종료 정보. Wait는 여기에 값이 들어올 때까지 condvar로 블로킹한다.
    exit: Mutex<Option<ExitRecord>>,
    exit_cv: Condvar,
    exited: AtomicBool,
    /// 앱이 주기적으로 올리는 최신 화면 스냅샷 (불투명 바이트, 압축 여부).
    /// 데몬은 이 바이트를 해석하지 않고 Attach에 플래그와 함께 그대로 되돌려준다.
    snapshot: Mutex<(Vec<u8>, bool)>,
    /// data conn 세대 카운터.
    gen: AtomicU64,
}

impl BrokerSession {
    fn next_gen(&self) -> u64 {
        self.gen.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// data conn이 아직 `gen`이면 비운다(교체됐으면 no-op). 반환값은
    /// "우리가 실제로 떼어냈는가".
    fn detach_if(&self, gen: u64) -> bool {
        let mut io = self.io.lock().unwrap();
        if io.conn.as_ref().map(|c| c.gen) == Some(gen) {
            io.conn = None;
            true
        } else {
            false
        }
    }

    fn exit_status_msg(&self) -> Option<ExitStatusMsg> {
        self.exit
            .lock()
            .unwrap()
            .map(|e| ExitStatusMsg { exit_code: e.exit_code, signal: e.signal })
    }
}

type BrokerTable = Mutex<HashMap<String, Arc<BrokerSession>>>;

/// 브로커 세션 하나를 spawn한다 -- openpty + 자식 spawn 후 reader/waiter
/// 스레드를 띄우고, 테이블에 등록할 `Arc<BrokerSession>`을 돌려준다.
#[allow(clippy::too_many_arguments)]
fn spawn_broker_session(
    session_id: String,
    shell: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    rows: u16,
    cols: u16,
    cwd: String,
    cleanup_paths: Vec<String>,
) -> io::Result<Arc<BrokerSession>> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};

    let sys = native_pty_system();
    let pair = sys
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| io::Error::other(e.to_string()))?;

    let mut cmd = CommandBuilder::new(&shell);
    for a in &args {
        cmd.arg(a);
    }
    cmd.cwd(&cwd);
    for (k, v) in &env {
        cmd.env(k, v);
    }
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| io::Error::other(e.to_string()))?;
    drop(pair.slave); // slave는 spawn 후 즉시 닫는다(권장).

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let pid = child.process_id().map(|p| p as i32);

    let session = Arc::new(BrokerSession {
        session_id,
        rows: AtomicU16::new(rows),
        cols: AtomicU16::new(cols),
        cwd,
        pid,
        cleanup_paths,
        io: Mutex::new(BrokerIo {
            ring: RingBuffer::new(RING_CAPACITY),
            conn: None,
            snapshot_offset: None,
            closed: false,
        }),
        input: Mutex::new(writer),
        master: Mutex::new(pair.master),
        killer: Mutex::new(child.clone_killer()),
        exit: Mutex::new(None),
        exit_cv: Condvar::new(),
        exited: AtomicBool::new(false),
        snapshot: Mutex::new((Vec::new(), false)),
        gen: AtomicU64::new(0),
    });

    // reader 스레드: master 출력 -> 링버퍼 + 활성 data conn.
    let reader_session = session.clone();
    let reader_join = std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let mut io = reader_session.io.lock().unwrap();
                    io.ring.push(&buf[..n]);
                    if let Some(conn) = &io.conn {
                        // 소켓 쓰기 실패는 무시 -- data conn 정리는 입력 루프의
                        // detach가 책임진다(잔여 바이트는 링버퍼에 남아 재접속 시 리플레이).
                        let _ = write_all_raw(conn.fd, &buf[..n]);
                    }
                }
                Err(_) => break,
            }
        }
    });

    // waiter 스레드: 자식 reap -> 종료 기록 -> reader drain 대기 -> data conn 닫기.
    let waiter_session = session.clone();
    std::thread::spawn(move || {
        let mut child = child;
        let status = child.wait();
        let record = match status {
            Ok(s) => ExitRecord { exit_code: Some(s.exit_code() as i32), signal: None },
            Err(_) => ExitRecord { exit_code: None, signal: None },
        };
        // 자식이 죽으면 master가 곧 EOF -> reader 스레드가 남은 출력을 전부
        // 링버퍼/conn에 흘린 뒤 끝난다. 그걸 기다린 다음에 종료를 신호해,
        // Wait를 받은 앱이 모든 출력을 본 뒤에 Exited로 전이하게 한다.
        let _ = reader_join.join();
        // 활성 data conn을 닫아 앱 쪽 reader가 EOF를 보게 하고, `closed`를 세워
        // 이후 도착하는 DataAttach가 새 conn을 설치하지 못하게 한다(§P2-b) --
        // conn.take()와 closed=true를 같은 락 아래에서 수행해 run_data_conn의
        // 설치와 직렬화한다.
        let stale = {
            let mut io = waiter_session.io.lock().unwrap();
            io.closed = true;
            io.conn.take()
        };
        if let Some(conn) = stale {
            let _ = shutdown(conn.fd, Shutdown::Both);
        }
        // cleanup: 관찰자 설정 파일 등(자식이 죽었으니 더 필요 없다).
        cleanup_broker_paths(&waiter_session.cleanup_paths);
        {
            let mut exit = waiter_session.exit.lock().unwrap();
            *exit = Some(record);
        }
        waiter_session.exited.store(true, Ordering::SeqCst);
        waiter_session.exit_cv.notify_all();
    });

    Ok(session)
}

fn cleanup_broker_paths(paths: &[String]) {
    for p in paths {
        if let Err(e) = std::fs::remove_file(p) {
            if e.kind() != io::ErrorKind::NotFound {
                eprintln!("agent-office sessiond: broker cleanup failed for {p}: {e}");
            }
        }
    }
}

/// DataAttach 처리: 응답 프레임을 보낸 뒤 이 연결을 raw 스트림으로 전환한다.
/// 백로그 리플레이 + conn 설치를 io 락 아래에서 원자적으로 수행해, 리플레이와
/// 라이브 출력 사이에 이음새(유실/중복)가 생기지 않게 한다. 이후 이 스레드는
/// 소켓의 raw 입력(앱->master)을 자식이 살아있는 동안 계속 펌프한다.
fn run_data_conn(fd: RawFd, session: &Arc<BrokerSession>) {
    let gen = session.next_gen();

    // DataAttachOk(+stream_offset) 프레임, 백로그 전송, closed 판정, conn 설치를
    // 하나의 io 락 아래에서 수행한다: (1) stream_offset이 백로그 첫 바이트와 정합
    // (§P1), (2) closed(§P2-b)와 conn 설치가 waiter의 정리와 직렬화된다.
    enum Outcome {
        Installed(Option<DataConn>), // 정상 설치 -- 교체된 이전 conn(있으면)
        AlreadyClosed,               // 자식이 이미 reap됨 -> 입력 펌프 없이 백로그+EOF
        WriteFailed,                 // 프레임/백로그 전송 실패
    }
    let outcome = {
        let mut io = session.io.lock().unwrap();
        // 스냅샷이 업로드된 세션은 그 오프셋 이후 바이트만 리플레이한다(앱이
        // 스냅샷을 화면으로 별도 복원하므로 중복 방지). 한 번도 없으면 링 전체.
        let backlog = match io.snapshot_offset {
            Some(off) => io.ring.snapshot_since(off),
            None => io.ring.snapshot(),
        };
        // 이 백로그 첫 바이트의 절대 스트림 오프셋 -- 앱이 수신 카운터를 여기서
        // 시작해, 이후 UpdateSnapshot에 실제 수신 오프셋을 실어 보낸다(§P1).
        let stream_offset = io.ring.total() - backlog.len() as u64;
        if protocol::write_frame(fd, &Message::DataAttachOk { stream_offset }, None).is_err()
            || write_all_raw(fd, &backlog).is_err()
        {
            Outcome::WriteFailed
        } else if io.closed {
            Outcome::AlreadyClosed
        } else {
            Outcome::Installed(io.conn.replace(DataConn { fd, gen }))
        }
    };
    match outcome {
        Outcome::WriteFailed => return,
        Outcome::AlreadyClosed => {
            // 자식이 이미 죽어 master에 쓸 게 없다 -- 백로그는 이미 보냈으니 write
            // 쪽만 shutdown해 앱 reader가 EOF를 보게 하고 입력 펌프는 생략한다.
            let _ = shutdown(fd, Shutdown::Write);
            return;
        }
        Outcome::Installed(prev) => {
            if let Some(prev) = prev {
                let _ = shutdown(prev.fd, Shutdown::Both);
            }
        }
    }

    // raw 입력 펌프: 소켓 -> master. 앱이 끊거나(Ok(0)) 교체/종료로 shutdown되면 끝.
    let mut buf = [0u8; 8192];
    loop {
        match nix::unistd::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let _ = session.input.lock().unwrap().write_all(&buf[..n]);
            }
            Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => break,
        }
    }
    session.detach_if(gen);
}

/// 연결 하나(=하나의 fd, Hello 이후 여러 요청을 순차로 받을 수 있다)를
/// 소진될 때까지(연결 종료/오류) 처리한다. 실 accept 루프와 테스트
/// (socketpair) 양쪽에서 동일하게 쓰는 핵심 로직.
fn handle_connection(
    fd: RawFd,
    table: &Table,
    broker: &BrokerTable,
    ever_handoff: &AtomicBool,
) {
    loop {
        let (msg, recv_fd) = match protocol::read_frame(fd) {
            Ok(v) => v,
            Err(_) => return, // 연결 종료 또는 프로토콜 오류 -- 이 연결은 여기까지.
        };
        match msg {
            Message::Hello { proto } => {
                // additive 협상: 클라이언트가 요청한 proto를 데몬 상한으로 낮춰
                // 수락한다(HelloOk{min(proto, PROTO_VERSION)}). 구프로토(>=1)
                // 클라이언트는 그 버전의 메시지만 보내므로 안전하다. proto 0은
                // 유효한 버전이 아니므로 거부한다.
                let reply = if proto >= 1 {
                    Message::HelloOk { proto: proto.min(protocol::PROTO_VERSION) }
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
                snapshot_b64,
            } => {
                let Some(master_fd) = recv_fd else {
                    let _ = protocol::write_frame(
                        fd,
                        &Message::Error { message: "Handoff must carry a master fd".into() },
                        None,
                    );
                    continue;
                };
                // 디코딩 실패(손상된 base64 등)는 스냅샷 없음으로 취급한다 --
                // fd는 이미 받았으므로 핸드오프 자체를 거부하지 않는다(설계
                // 문서: "핸드오프 실패 시에도 세션 표시가 깨지면 안 된다"와
                // 같은 원칙).
                let snapshot = {
                    use base64::Engine;
                    base64::engine::general_purpose::STANDARD
                        .decode(&snapshot_b64)
                        .unwrap_or_default()
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
            Message::List => {
                let mut sessions: Vec<SessionInfo> = table
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
                        broker: false,
                    })
                    .collect();
                // v2 브로커 세션도 같은 List에 additive로 실어 준다(broker: true).
                sessions.extend(broker.lock().unwrap().iter().map(|(agent_id, s)| SessionInfo {
                    agent_id: agent_id.clone(),
                    session_id: s.session_id.clone(),
                    pid: s.pid,
                    rows: s.rows.load(Ordering::SeqCst),
                    cols: s.cols.load(Ordering::SeqCst),
                    cwd: s.cwd.clone(),
                    exited: s.exited.load(Ordering::SeqCst),
                    buffered_bytes: s.io.lock().unwrap().ring.len(),
                    broker: true,
                }));
                let _ = protocol::write_frame(fd, &Message::ListOk { sessions }, None);
            }
            // ── v2 브로커 메시지 ────────────────────────────────────────
            Message::Spawn {
                agent_id,
                session_id,
                shell,
                args,
                env,
                rows,
                cols,
                cwd,
                cleanup_paths,
            } => {
                match spawn_broker_session(
                    session_id, shell, args, env, rows, cols, cwd, cleanup_paths,
                ) {
                    Ok(session) => {
                        let pid = session.pid;
                        broker.lock().unwrap().insert(agent_id, session);
                        // 스폰도 first-activity로 인정 -- 고아 데몬 타임아웃 방지.
                        ever_handoff.store(true, Ordering::SeqCst);
                        let _ = protocol::write_frame(fd, &Message::SpawnOk { pid }, None);
                    }
                    Err(e) => {
                        let _ = protocol::write_frame(
                            fd,
                            &Message::Error { message: format!("spawn failed: {e}") },
                            None,
                        );
                    }
                }
            }
            Message::DataAttach { agent_id } => {
                let session = broker.lock().unwrap().get(&agent_id).cloned();
                match session {
                    Some(session) => {
                        // 응답 프레임 후 이 연결은 raw로 전환된다 -- 자식이 살아있는
                        // 동안(또는 교체될 때까지) 이 스레드가 raw 입력을 펌프하다가,
                        // 끝나면 연결도 끝난다(프레임 루프로 돌아가지 않는다).
                        run_data_conn(fd, &session);
                        return;
                    }
                    None => {
                        let _ = protocol::write_frame(
                            fd,
                            &Message::Error { message: format!("unknown agent_id: {agent_id}") },
                            None,
                        );
                    }
                }
            }
            Message::Attach { agent_id } => {
                let session = broker.lock().unwrap().get(&agent_id).cloned();
                let reply = match session {
                    Some(session) => {
                        let (snapshot_bytes, snapshot_compressed) = {
                            let g = session.snapshot.lock().unwrap();
                            (g.0.clone(), g.1)
                        };
                        let snapshot_b64 = {
                            use base64::Engine;
                            base64::engine::general_purpose::STANDARD.encode(snapshot_bytes)
                        };
                        Message::AttachOk {
                            rows: session.rows.load(Ordering::SeqCst),
                            cols: session.cols.load(Ordering::SeqCst),
                            pid: session.pid,
                            snapshot_b64,
                            snapshot_compressed,
                            exit: session.exit_status_msg(),
                        }
                    }
                    None => Message::Error { message: format!("unknown agent_id: {agent_id}") },
                };
                let _ = protocol::write_frame(fd, &reply, None);
            }
            Message::Resize { agent_id, rows, cols } => {
                let session = broker.lock().unwrap().get(&agent_id).cloned();
                if let Some(session) = session {
                    let ok = session
                        .master
                        .lock()
                        .unwrap()
                        .resize(portable_pty::PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
                        .is_ok();
                    // resize 성공 시 메타를 갱신 -- List/Attach가 최신 지오메트리를
                    // 반환하게 한다(§P2-c).
                    if ok {
                        session.rows.store(rows, Ordering::SeqCst);
                        session.cols.store(cols, Ordering::SeqCst);
                    }
                }
                let _ = protocol::write_frame(fd, &Message::ResizeOk, None);
            }
            Message::Wait { agent_id } => {
                let session = broker.lock().unwrap().get(&agent_id).cloned();
                let reply = match session {
                    Some(session) => {
                        let mut exit = session.exit.lock().unwrap();
                        while exit.is_none() {
                            exit = session.exit_cv.wait(exit).unwrap();
                        }
                        let rec = exit.unwrap();
                        Message::WaitOk { exit_code: rec.exit_code, signal: rec.signal }
                    }
                    None => Message::Error { message: format!("unknown agent_id: {agent_id}") },
                };
                let _ = protocol::write_frame(fd, &reply, None);
            }
            Message::KillAll => {
                let sessions: Vec<Arc<BrokerSession>> =
                    broker.lock().unwrap().drain().map(|(_, s)| s).collect();
                let killed = sessions.len();
                for session in &sessions {
                    let _ = session.killer.lock().unwrap().kill();
                }
                let _ = protocol::write_frame(fd, &Message::KillAllOk { killed }, None);
            }
            Message::UpdateSnapshot { agent_id, snapshot_b64, offset, compressed } => {
                if let Some(session) = broker.lock().unwrap().get(&agent_id) {
                    use base64::Engine;
                    if let Ok(bytes) =
                        base64::engine::general_purpose::STANDARD.decode(&snapshot_b64)
                    {
                        *session.snapshot.lock().unwrap() = (bytes, compressed);
                        // 스냅샷이 반영하는 스트림 오프셋. 앱이 data 연결 카운터로
                        // "실제 여기까지 수신했다"는 offset을 실어 보내면(§P1) 그걸
                        // 쓰고(링 상한으로 클램프 — 하한은 snapshot_since가 처리),
                        // 없으면 수신 시점 ring.total()로 폴백한다. 이후 DataAttach는
                        // 이 오프셋 이후 바이트만 리플레이한다.
                        let mut io = session.io.lock().unwrap();
                        let total = io.ring.total();
                        io.snapshot_offset = Some(offset.map_or(total, |o| o.min(total)));
                    }
                }
                let _ = protocol::write_frame(fd, &Message::UpdateSnapshotOk, None);
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
            Message::Kill { agent_id } => {
                if let Some(mut entry) = table.lock().unwrap().remove(&agent_id) {
                    if let Some(pgid) = entry.pgid.or(entry.pid) {
                        let _ = killpg(Pid::from_raw(pgid), Signal::SIGKILL);
                    }
                    entry.stop_reader();
                    let _ = nix::unistd::close(entry.master_fd);
                    entry.consumed = true;
                }
                // v2 브로커 세션도 같은 Kill로 의도적 종료할 수 있다.
                if let Some(session) = broker.lock().unwrap().remove(&agent_id) {
                    let _ = session.killer.lock().unwrap().kill();
                    if let Some(conn) = session.io.lock().unwrap().conn.take() {
                        let _ = shutdown(conn.fd, Shutdown::Both);
                    }
                    cleanup_broker_paths(&session.cleanup_paths);
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
    let broker: Arc<BrokerTable> = Arc::new(Mutex::new(HashMap::new()));
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
        let broker = broker.clone();
        let ever = ever_handoff.clone();
        let hook = on_shutdown.clone();
        std::thread::spawn(move || {
            handle_connection(stream.as_raw_fd(), &table, &broker, &ever);
            drop(stream);
            // 연결이 끊길 때마다 두 테이블(v1 핸드오프 + v2 브로커)이 모두 비어
            // 있으면 종료(§프로토콜 "데몬 수명").
            if table.lock().unwrap().is_empty() && broker.lock().unwrap().is_empty() {
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
        broker: Arc<BrokerTable>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl Harness {
        fn new() -> Self {
            let (client_fd, server_fd) =
                socketpair(AddressFamily::Unix, SockType::Stream, None, SockFlag::empty()).unwrap();
            let table: Arc<Table> = Arc::new(Mutex::new(HashMap::new()));
            let broker: Arc<BrokerTable> = Arc::new(Mutex::new(HashMap::new()));
            let ever = Arc::new(AtomicBool::new(false));
            let table_for_thread = table.clone();
            let broker_for_thread = broker.clone();
            let handle = std::thread::spawn(move || {
                handle_connection(server_fd, &table_for_thread, &broker_for_thread, &ever);
                let _ = close(server_fd);
            });
            Harness { client_fd, table, broker, handle: Some(handle) }
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

    /// 소켓에서 raw 바이트를 `needle`가 나타날 때까지(또는 타임아웃) 읽는다.
    /// poll로 블로킹 read가 테스트를 영원히 매달지 않게 한다.
    fn raw_read_until(fd: RawFd, needle: &[u8]) -> Vec<u8> {
        use nix::poll::{poll, PollFd, PollFlags};
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut acc = Vec::new();
        let mut buf = [0u8; 4096];
        while std::time::Instant::now() < deadline {
            let mut fds = [PollFd::new(fd, PollFlags::POLLIN)];
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            match poll(&mut fds, remaining.as_millis().min(200) as i32) {
                Ok(0) => continue,
                Ok(_) => {}
                Err(nix::errno::Errno::EINTR) => continue,
                Err(_) => break,
            }
            match nix_read(fd, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    acc.extend_from_slice(&buf[..n]);
                    if acc.windows(needle.len()).any(|w| w == needle) {
                        return acc;
                    }
                }
                Err(nix::errno::Errno::EINTR) | Err(nix::errno::Errno::EAGAIN) => continue,
                Err(_) => break,
            }
        }
        acc
    }

    /// 실 `UnixListener`(run_daemon_inner) 데몬을 백그라운드로 띄우고 소켓
    /// 경로/작업 디렉터리를 돌려준다. 브로커 테스트는 control/data/wait에
    /// 여러 연결을 열어야 하므로(단일 소켓쌍 Harness로는 불가) 실 소켓을 쓴다.
    fn start_real_daemon() -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ao-bk-{}", short_id()));
        let _ = std::fs::create_dir_all(&dir);
        let socket_path = dir.join("s.sock");
        let socket_for_daemon = socket_path.clone();
        let hook: ShutdownHook = Arc::new(|| {}); // 테스트에선 프로세스를 죽이지 않는다.
        std::thread::spawn(move || {
            let _ = run_daemon_inner(socket_for_daemon, Duration::from_secs(60), hook);
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !socket_path.exists() {
            assert!(std::time::Instant::now() < deadline, "daemon never bound the socket");
            std::thread::sleep(Duration::from_millis(10));
        }
        (socket_path, dir)
    }

    fn connect_hello(socket_path: &std::path::Path) -> std::os::unix::net::UnixStream {
        use std::os::unix::net::UnixStream;
        let stream = UnixStream::connect(socket_path).unwrap();
        let fd = stream.as_raw_fd();
        protocol::write_frame(fd, &Message::Hello { proto: protocol::PROTO_VERSION }, None).unwrap();
        assert!(matches!(protocol::read_frame(fd).unwrap().0, Message::HelloOk { .. }));
        stream
    }

    fn spawn_broker(
        control_fd: RawFd,
        agent_id: &str,
        script: &str,
    ) {
        protocol::write_frame(
            control_fd,
            &Message::Spawn {
                agent_id: agent_id.into(),
                session_id: format!("s-{agent_id}"),
                shell: "/bin/sh".into(),
                args: vec!["-c".into(), script.into()],
                env: vec![("TERM".into(), "xterm-256color".into())],
                rows: 24,
                cols: 80,
                cwd: "/tmp".into(),
                cleanup_paths: vec![],
            },
            None,
        )
        .unwrap();
        assert!(matches!(
            protocol::read_frame(control_fd).unwrap().0,
            Message::SpawnOk { .. }
        ));
    }

    #[test]
    fn broker_spawn_registers_and_data_attach_echoes() {
        // Harness 단일 연결로 Spawn -> (테이블 등록 확인) -> DataAttach -> echo.
        let h = Harness::new();
        h.send(&Message::Hello { proto: protocol::PROTO_VERSION }, None);
        assert!(matches!(h.recv().0, Message::HelloOk { .. }));

        spawn_broker(h.client_fd, "a1", "printf READY; cat");
        wait_until(|| h.broker.lock().unwrap().contains_key("a1"));

        h.send(&Message::DataAttach { agent_id: "a1".into() }, None);
        assert!(matches!(h.recv().0, Message::DataAttachOk { .. }));

        // 백로그로 "READY"가 리플레이되어야 한다(스폰 시점부터 수집).
        let backlog = raw_read_until(h.client_fd, b"READY");
        assert!(
            backlog.windows(5).any(|w| w == b"READY"),
            "spawn-time output must replay on DataAttach: {backlog:?}"
        );

        // raw 입력 -> master -> cat 에코가 돌아온다.
        protocol::write_all_raw(h.client_fd, b"ping\n").unwrap();
        let echoed = raw_read_until(h.client_fd, b"ping");
        assert!(
            echoed.windows(4).any(|w| w == b"ping"),
            "input must round-trip through the broker PTY: {echoed:?}"
        );

        // 세션을 정리(자식 kill)해 데몬 스레드가 매달리지 않게.
        h.send(&Message::Kill { agent_id: "a1".into() }, None);
        // Kill 응답은 raw 스트림 중이라 프레임으로 안 오지만, 자식은 죽는다.
        let _ = close(h.client_fd);
    }

    #[test]
    fn broker_backlog_replays_losslessly_across_reattach() {
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        spawn_broker(control.as_raw_fd(), "a1", "printf HELLO-BACKLOG; sleep 5");

        // 첫 DataAttach: 백로그 회수.
        let data1 = connect_hello(&socket_path);
        protocol::write_frame(data1.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(protocol::read_frame(data1.as_raw_fd()).unwrap().0, Message::DataAttachOk { .. }));
        let first = raw_read_until(data1.as_raw_fd(), b"HELLO-BACKLOG");
        assert!(first.windows(13).any(|w| w == b"HELLO-BACKLOG"));
        drop(data1); // detach(자식은 안 죽는다)

        // 재 DataAttach: 같은 백로그가 무손실 리플레이돼야 한다.
        let data2 = connect_hello(&socket_path);
        protocol::write_frame(data2.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(protocol::read_frame(data2.as_raw_fd()).unwrap().0, Message::DataAttachOk { .. }));
        let second = raw_read_until(data2.as_raw_fd(), b"HELLO-BACKLOG");
        assert!(
            second.windows(13).any(|w| w == b"HELLO-BACKLOG"),
            "reattach must replay the full backlog: {second:?}"
        );

        protocol::write_frame(control.as_raw_fd(), &Message::Kill { agent_id: "a1".into() }, None)
            .unwrap();
        let _ = protocol::read_frame(control.as_raw_fd());
        drop(control);
        drop(data2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_data_attach_after_snapshot_replays_only_post_snapshot_bytes() {
        // §P2-b: UpdateSnapshot이 그 시점 링 오프셋을 기록하면, 이후 DataAttach는
        // 스냅샷 이전 출력을 리플레이하지 않고 그 이후 바이트만 흘려야 한다
        // (앱은 스냅샷을 화면으로 별도 복원 -> 중복 없이 전체 스크롤백 복원).
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        // printf PRE -> read(입력 대기, 여기서 출력이 멎어 스냅샷 오프셋이 결정적) -> printf POST.
        spawn_broker(control.as_raw_fd(), "a1", "printf PREMARKER; read x; printf POSTMARKER");

        let data1 = connect_hello(&socket_path);
        protocol::write_frame(data1.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(protocol::read_frame(data1.as_raw_fd()).unwrap().0, Message::DataAttachOk { .. }));
        // PRE가 링에 전부 들어온 상태에서(셸이 read로 멎어 더 안 나옴) 스냅샷.
        let pre = raw_read_until(data1.as_raw_fd(), b"PREMARKER");
        assert!(pre.windows(9).any(|w| w == b"PREMARKER"));

        protocol::write_frame(
            control.as_raw_fd(),
            &Message::UpdateSnapshot {
                agent_id: "a1".into(),
                snapshot_b64: "c25hcA==".into(),
                offset: None,
                compressed: false,
            },
            None,
        )
        .unwrap();
        assert!(matches!(
            protocol::read_frame(control.as_raw_fd()).unwrap().0,
            Message::UpdateSnapshotOk
        ));

        // 입력으로 read를 풀어 POST를 만든다(입력 에코 "go"도 스냅샷 이후라 무해).
        protocol::write_all_raw(data1.as_raw_fd(), b"go\n").unwrap();
        let _ = raw_read_until(data1.as_raw_fd(), b"POSTMARKER");

        // 재접속: 스냅샷 이후 바이트만 와야 한다 -- POST 포함, PRE 제외.
        let data2 = connect_hello(&socket_path);
        protocol::write_frame(data2.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(protocol::read_frame(data2.as_raw_fd()).unwrap().0, Message::DataAttachOk { .. }));
        let backlog2 = raw_read_until(data2.as_raw_fd(), b"POSTMARKER");
        assert!(
            backlog2.windows(10).any(|w| w == b"POSTMARKER"),
            "post-snapshot output must replay: {backlog2:?}"
        );
        assert!(
            !backlog2.windows(9).any(|w| w == b"PREMARKER"),
            "pre-snapshot output must NOT replay (snapshot_offset excludes it): {backlog2:?}"
        );

        protocol::write_frame(control.as_raw_fd(), &Message::Kill { agent_id: "a1".into() }, None)
            .unwrap();
        let _ = protocol::read_frame(control.as_raw_fd());
        drop(control);
        drop(data1);
        drop(data2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_resize_updates_geometry_in_list_and_attach() {
        // §P2-c: Resize 성공 시 rows/cols 메타가 갱신되어 List/Attach가 최신
        // 지오메트리를 반환해야 한다.
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        spawn_broker(control.as_raw_fd(), "a1", "sleep 30"); // 스폰 시 24x80

        protocol::write_frame(
            control.as_raw_fd(),
            &Message::Resize { agent_id: "a1".into(), rows: 50, cols: 200 },
            None,
        )
        .unwrap();
        assert!(matches!(protocol::read_frame(control.as_raw_fd()).unwrap().0, Message::ResizeOk));

        protocol::write_frame(control.as_raw_fd(), &Message::Attach { agent_id: "a1".into() }, None)
            .unwrap();
        match protocol::read_frame(control.as_raw_fd()).unwrap().0 {
            Message::AttachOk { rows, cols, .. } => {
                assert_eq!((rows, cols), (50, 200), "Attach must reflect the resize");
            }
            other => panic!("unexpected: {other:?}"),
        }

        protocol::write_frame(control.as_raw_fd(), &Message::List, None).unwrap();
        match protocol::read_frame(control.as_raw_fd()).unwrap().0 {
            Message::ListOk { sessions } => {
                let s = sessions.iter().find(|s| s.agent_id == "a1").expect("session in list");
                assert_eq!((s.rows, s.cols), (50, 200), "List must reflect the resize");
            }
            other => panic!("unexpected: {other:?}"),
        }

        protocol::write_frame(control.as_raw_fd(), &Message::Kill { agent_id: "a1".into() }, None)
            .unwrap();
        let _ = protocol::read_frame(control.as_raw_fd());
        drop(control);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_data_attach_honors_app_supplied_offset() {
        // §P1: UpdateSnapshot{offset:Some(k)}가 오면 데몬 수신 시점 ring.total()이
        // 아니라 앱이 준 오프셋을 snapshot_offset으로 써야 한다 -- 리플레이가 그
        // 오프셋부터 시작한다. printf로 결정적 8바이트("ABCDEFGH")를 만들고
        // offset=3을 주면 재접속 백로그는 "DEFGH"여야 한다.
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        spawn_broker(control.as_raw_fd(), "a1", "printf ABCDEFGH; sleep 5");

        let data1 = connect_hello(&socket_path);
        protocol::write_frame(data1.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(
            protocol::read_frame(data1.as_raw_fd()).unwrap().0,
            Message::DataAttachOk { stream_offset: 0 }
        ));
        let _ = raw_read_until(data1.as_raw_fd(), b"ABCDEFGH");

        protocol::write_frame(
            control.as_raw_fd(),
            &Message::UpdateSnapshot {
                agent_id: "a1".into(),
                snapshot_b64: "c25hcA==".into(),
                offset: Some(3),
                compressed: false,
            },
            None,
        )
        .unwrap();
        assert!(matches!(
            protocol::read_frame(control.as_raw_fd()).unwrap().0,
            Message::UpdateSnapshotOk
        ));

        let data2 = connect_hello(&socket_path);
        protocol::write_frame(data2.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(
            protocol::read_frame(data2.as_raw_fd()).unwrap().0,
            Message::DataAttachOk { stream_offset: 3 }
        ));
        let backlog2 = raw_read_until(data2.as_raw_fd(), b"DEFGH");
        assert!(backlog2.windows(5).any(|w| w == b"DEFGH"));
        assert!(
            !backlog2.windows(3).any(|w| w == b"ABC"),
            "offset=3 must skip the first 3 bytes: {backlog2:?}"
        );

        protocol::write_frame(control.as_raw_fd(), &Message::Kill { agent_id: "a1".into() }, None)
            .unwrap();
        let _ = protocol::read_frame(control.as_raw_fd());
        drop(control);
        drop(data1);
        drop(data2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_data_attach_after_child_exit_gets_backlog_and_eof_without_blocking() {
        // §P2-b: waiter가 conn 정리를 마친(closed=true) 뒤 도착한 DataAttach는
        // 새 conn을 설치하지 않고 백로그+EOF만 주고 끝나야 한다(앱 reader 무한
        // 블록 방지). 즉시 종료하는 자식으로 재현.
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        spawn_broker(control.as_raw_fd(), "a1", "printf DONE-X");

        // Wait로 자식 reap + closed=true까지 확정적으로 기다린다.
        let waiter = connect_hello(&socket_path);
        protocol::write_frame(waiter.as_raw_fd(), &Message::Wait { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(
            protocol::read_frame(waiter.as_raw_fd()).unwrap().0,
            Message::WaitOk { .. }
        ));

        // 종료 후 DataAttach: 백로그("DONE-X")를 받고 EOF(Ok(0))로 끝나야 한다.
        let data = connect_hello(&socket_path);
        protocol::write_frame(data.as_raw_fd(), &Message::DataAttach { agent_id: "a1".into() }, None)
            .unwrap();
        assert!(matches!(
            protocol::read_frame(data.as_raw_fd()).unwrap().0,
            Message::DataAttachOk { .. }
        ));
        // EOF까지 읽는다 -- 무한 블록이면 raw_read_until의 5초 데드라인에 걸려
        // 아래 assert가 실패한다.
        let got = raw_read_until(data.as_raw_fd(), b"DONE-X");
        assert!(got.windows(6).any(|w| w == b"DONE-X"), "backlog must arrive: {got:?}");
        // 이어 read하면 EOF(0)여야 한다(블록 아님).
        {
            use nix::poll::{poll, PollFd, PollFlags};
            let mut fds = [PollFd::new(data.as_raw_fd(), PollFlags::POLLIN)];
            let ready = poll(&mut fds, 2000).unwrap();
            assert!(ready > 0, "reader must not block after backlog -- expected EOF");
            let mut buf = [0u8; 64];
            let n = nix_read(data.as_raw_fd(), &mut buf).unwrap();
            // "DONE-X"가 한 번에 안 왔을 수도 있으니 0이거나 남은 바이트.
            if n > 0 {
                let n2 = nix_read(data.as_raw_fd(), &mut buf).unwrap_or(0);
                assert_eq!(n2, 0, "must reach EOF shortly after backlog");
            }
        }

        drop(control);
        drop(waiter);
        drop(data);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_wait_returns_child_exit_code() {
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        spawn_broker(control.as_raw_fd(), "a1", "exit 7");

        // Wait는 전용 연결에서(§설계) -- 자식 종료까지 블로킹.
        let waiter = connect_hello(&socket_path);
        protocol::write_frame(waiter.as_raw_fd(), &Message::Wait { agent_id: "a1".into() }, None)
            .unwrap();
        match protocol::read_frame(waiter.as_raw_fd()).unwrap().0 {
            Message::WaitOk { exit_code, .. } => assert_eq!(exit_code, Some(7)),
            other => panic!("unexpected: {other:?}"),
        }

        // 종료 후 Attach는 exit 정보를 실어 준다.
        protocol::write_frame(control.as_raw_fd(), &Message::Attach { agent_id: "a1".into() }, None)
            .unwrap();
        match protocol::read_frame(control.as_raw_fd()).unwrap().0 {
            Message::AttachOk { exit: Some(e), .. } => assert_eq!(e.exit_code, Some(7)),
            other => panic!("unexpected AttachOk without exit: {other:?}"),
        }

        drop(control);
        drop(waiter);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_kill_all_kills_every_session() {
        let (socket_path, dir) = start_real_daemon();
        let control = connect_hello(&socket_path);
        spawn_broker(control.as_raw_fd(), "a1", "sleep 30");
        spawn_broker(control.as_raw_fd(), "a2", "sleep 30");

        protocol::write_frame(control.as_raw_fd(), &Message::KillAll, None).unwrap();
        match protocol::read_frame(control.as_raw_fd()).unwrap().0 {
            Message::KillAllOk { killed } => assert_eq!(killed, 2),
            other => panic!("unexpected: {other:?}"),
        }

        // 이제 List는 브로커 세션을 하나도 담지 않아야 한다.
        protocol::write_frame(control.as_raw_fd(), &Message::List, None).unwrap();
        match protocol::read_frame(control.as_raw_fd()).unwrap().0 {
            Message::ListOk { sessions } => {
                assert!(sessions.iter().all(|s| !s.broker), "KillAll must empty the broker table");
            }
            other => panic!("unexpected: {other:?}"),
        }

        drop(control);
        let _ = std::fs::remove_dir_all(&dir);
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
    fn hello_errors_on_invalid_proto_zero() {
        // proto 0은 유효한 버전이 아니므로 거부한다(그 외 >=1은 협상 수락).
        let h = Harness::new();
        h.send(&Message::Hello { proto: 0 }, None);
        let (reply, _) = h.recv();
        assert!(matches!(reply, Message::Error { .. }));
        h.finish();
    }

    #[test]
    fn hello_negotiates_down_to_older_client_proto() {
        // 구프로토(v1) 클라이언트가 Hello{1}을 보내면 데몬은 HelloOk{1}로 답해
        // 그 버전으로 협상한다 -- 앱 업데이트 직후 신데몬 ↔ 구클라이언트 호환.
        let h = Harness::new();
        h.send(&Message::Hello { proto: 1 }, None);
        let (reply, _) = h.recv();
        assert!(matches!(reply, Message::HelloOk { proto: 1 }));
        h.finish();
    }

    #[test]
    fn hello_clamps_future_proto_to_daemon_max() {
        // 미래 클라이언트(proto > PROTO_VERSION)는 데몬 상한으로 클램프된다 --
        // 그 클라이언트는 협상된 버전의 메시지만 보내므로 안전(forward-compat).
        let h = Harness::new();
        h.send(&Message::Hello { proto: 99 }, None);
        let (reply, _) = h.recv();
        assert!(matches!(reply, Message::HelloOk { proto } if proto == protocol::PROTO_VERSION));
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
                snapshot_b64: String::new(),
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
                snapshot_b64: String::new(),
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

    /// 종료 직전 화면 스냅샷(§실증에서 발견된 빈틈 수정) 회귀: Handoff의
    /// snapshot_b64가 그대로 테이블에 보관됐다가 Adopt 응답의 snapshot_b64로
    /// 되돌아오는지 검증한다. 데몬은 이 바이트열을 전혀 해석하지 않고
    /// 불투명하게 보관/반환만 한다.
    #[test]
    fn handoff_snapshot_is_stored_and_returned_via_adopt_ok() {
        let h = Harness::new();
        let (master_read, master_write) = pipe().unwrap();
        use base64::Engine;
        let snapshot_b64 =
            base64::engine::general_purpose::STANDARD.encode(b"SCREEN-BEFORE-QUIT\r\n$ ls\r\n");

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
                snapshot_b64: snapshot_b64.clone(),
            },
            Some(master_read),
        );
        let _ = close(master_read);
        let (reply, _) = h.recv();
        assert!(matches!(reply, Message::HandoffOk));

        h.send(&Message::Adopt { agent_id: "a1".into() }, None);
        let (reply, fd) = h.recv();
        let adopted_fd = fd.expect("AdoptOk must carry the master fd");
        match reply {
            Message::AdoptOk { snapshot_b64: returned, .. } => {
                assert_eq!(returned, snapshot_b64, "snapshot must round-trip unchanged");
            }
            other => panic!("unexpected reply: {other:?}"),
        }

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
                snapshot_b64: String::new(),
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
                snapshot_b64: String::new(),
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
                snapshot_b64: String::new(),
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
                snapshot_b64: String::new(),
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
