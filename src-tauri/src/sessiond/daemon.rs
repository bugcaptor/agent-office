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
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use nix::sys::signal::{killpg, Signal};
use nix::sys::socket::{shutdown, Shutdown};
use nix::unistd::Pid;

use super::protocol::{self, write_all_raw, ExitStatusMsg, Message, SessionInfo};

/// 데몬이 세션당 보관하는 미전달 출력 상한(§프로토콜). base64 팽창은 전송
/// 시점(AdoptOk)에만 계산 — 보관은 원본 바이트로.
const RING_CAPACITY: usize = 512 * 1024;

/// data conn별 writer 스레드로 흘리는 출력 프레임 큐의 상한(프레임 수, §#48).
/// 블로킹 소켓 write를 io 락 밖(전용 writer 스레드)으로 빼되, 앱이 멈춰 큐가
/// 이만큼 차면 그 conn을 버린다(링버퍼가 진실원본이라 재접속 시 backlog로 복구).
/// 프레임 하나는 reader buf 상한(8KiB)이므로 상한 메모리는 ~2MiB.
const WRITER_QUEUE_CAP: usize = 256;

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
///
/// §#48: 앱 방향 출력 write는 이제 전용 writer 스레드가 담당하고, reader는 이
/// `tx`로 프레임을 넘길 뿐이다(블로킹 write가 io 락을 물지 않게). conn이
/// 슬롯에서 제거(take/replace)되면 `tx`가 드롭돼 writer의 `recv`가 끝난다.
struct DataConn {
    fd: RawFd,
    gen: u64,
    /// 출력 프레임을 writer 스레드로 넘기는 바운드 채널(꽉 차면 conn 폐기).
    tx: SyncSender<Vec<u8>>,
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
                    // §#48: 블로킹 write 대신 non-blocking try_send로 writer 스레드에
                    // 넘긴다 -- io 락을 쥔 채 소켓 write가 멈추던 결함을 없앤다. 큐가
                    // 꽉 찼거나(느린/멈춘 앱) writer가 죽어 채널이 끊겼으면 이 conn을
                    // 버린다: fd를 shutdown해 입력 펌프/writer를 깨우고 슬롯을 비운다.
                    // 잔여/유실 바이트는 링버퍼에 남아 재접속 시 backlog로 복구된다.
                    let send_failed = match &io.conn {
                        Some(conn) => conn.tx.try_send(buf[..n].to_vec()).is_err(),
                        None => false,
                    };
                    if send_failed {
                        if let Some(conn) = io.conn.take() {
                            let _ = shutdown(conn.fd, Shutdown::Both);
                        }
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

/// 한 data conn의 앱 방향 출력(master->앱)을 전담하는 writer 스레드를 띄운다
/// (§#48). 블로킹 소켓 write를 io 락 밖으로 완전히 빼는 핵심: 이 스레드만 fd에
/// write하고, reader는 `tx`로 프레임을 넘길 뿐이다.
///
/// **원자성 seam**: DataAttachOk 프레임과 backlog는 채널이 아니라 이 스레드의
/// *초기 상태*로 받아 큐보다 **먼저** 쓴다. 호출자가 io 락 아래에서 backlog를
/// 캡처하고 conn(=tx)을 설치하므로, 그 이후 reader가 `tx`로 넣는 라이브 프레임은
/// 반드시 backlog 뒤에 온다 -- enqueue가 새로운 직렬화 지점이 되어 기존의
/// "락 안 원자성"이 그대로 보존된다(유실/중복 없음).
///
/// `close_write_after_backlog`(=자식이 이미 reap됨, §P2-b)면 backlog만 보내고
/// write 쪽을 shutdown해 앱이 EOF를 보게 하고 끝낸다(라이브 출력이 없으므로 큐
/// 드레인 없음). fd는 호출자(run_data_conn을 부른 handle_connection)가 소유하며,
/// run_data_conn이 이 스레드를 join한 **뒤** 반환하므로 fd가 닫히기 전에 이
/// 스레드가 확정 종료된다(fd 재사용 레이스 방지).
fn spawn_conn_writer(
    fd: RawFd,
    stream_offset: u64,
    backlog: Vec<u8>,
    rx: Receiver<Vec<u8>>,
    close_write_after_backlog: bool,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        if protocol::write_frame(fd, &Message::DataAttachOk { stream_offset }, None).is_err()
            || write_all_raw(fd, &backlog).is_err()
        {
            // 초기 전송 실패(앱이 이미 끊김 등) -- conn 정리는 입력 펌프의
            // detach_if가 담당하므로 여기선 그냥 끝낸다.
            return;
        }
        if close_write_after_backlog {
            let _ = shutdown(fd, Shutdown::Write);
            return;
        }
        // 라이브 프레임 드레인: conn이 슬롯에서 제거되면 tx가 드롭돼 recv가 끝난다.
        while let Ok(chunk) = rx.recv() {
            if write_all_raw(fd, &chunk).is_err() {
                break;
            }
        }
    })
}

/// DataAttach 처리: 이 연결을 raw 스트림으로 전환한다. backlog 캡처 + conn(tx)
/// 설치를 io 락 아래에서 원자적으로 수행하고(§원자성 seam은 spawn_conn_writer
/// 참고), 실제 소켓 write는 전용 writer 스레드로 넘긴다 -- 블로킹 write가 io
/// 락을 물어 List/Kill 등을 얼리던 결함(§#48)을 없앤다. 이후 이 스레드는 소켓의
/// raw 입력(앱->master)을 펌프하다가, 끝나면 writer를 join한 뒤 반환한다.
fn run_data_conn(fd: RawFd, session: &Arc<BrokerSession>) {
    let gen = session.next_gen();

    // io 락 아래에서: backlog 캡처 + writer 스레드 생성 + conn 설치. writer는 락
    // 밖에서 backlog/라이브를 write하므로 블로킹이 락을 물지 않는다.
    let (writer_join, prev, run_pump) = {
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
        let closed = io.closed; // 자식이 이미 reap됨(§P2-b) -> 라이브 출력 없음.
        let (tx, rx) = sync_channel::<Vec<u8>>(WRITER_QUEUE_CAP);
        // writer 생성은 락 아래(큐가 빈 시점). 이후 reader의 enqueue는 이 conn
        // 설치 뒤에만 가능하므로 backlog 뒤 순서가 보장된다.
        let writer_join = spawn_conn_writer(fd, stream_offset, backlog, rx, closed);
        if closed {
            // 설치하지 않는다(라이브 출력 없음). tx는 여기서 드롭되지만 writer는
            // closed 경로라 rx를 보지 않는다. 입력 펌프도 돌리지 않는다.
            (writer_join, None, false)
        } else {
            (writer_join, io.conn.replace(DataConn { fd, gen, tx }), true)
        }
    };

    // 교체된 이전 conn: 소켓을 shutdown해 그 입력 펌프/writer를 깨우고, prev를
    // 드롭해 그 tx를 떨어뜨린다(이전 writer의 recv 종료).
    if let Some(prev) = prev {
        let _ = shutdown(prev.fd, Shutdown::Both);
    }

    if run_pump {
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
    }

    // conn을 슬롯에서 비운다(우리 gen이면). 이게 우리 tx를 드롭해 writer의 recv를
    // 끝낸다 -- join보다 **먼저** 해야 writer가 깨어 종료하고 join이 안 막힌다.
    session.detach_if(gen);
    // fd가 닫히기(handle_connection 반환) 전에 writer가 확정 종료하도록 join한다.
    let _ = writer_join.join();
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
                        attached: false,
                    })
                    .collect();
                // v2 브로커 세션도 같은 List에 additive로 실어 준다(broker: true).
                sessions.extend(broker.lock().unwrap().iter().map(|(agent_id, s)| {
                    // buffered_bytes와 attached를 한 번의 io 락으로 함께 읽는다.
                    let (buffered_bytes, attached) = {
                        let io = s.io.lock().unwrap();
                        (io.ring.len(), io.conn.is_some())
                    };
                    SessionInfo {
                        agent_id: agent_id.clone(),
                        session_id: s.session_id.clone(),
                        pid: s.pid,
                        rows: s.rows.load(Ordering::SeqCst),
                        cols: s.cols.load(Ordering::SeqCst),
                        cwd: s.cwd.clone(),
                        exited: s.exited.load(Ordering::SeqCst),
                        buffered_bytes,
                        broker: true,
                        attached,
                    }
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
                            // 이슈 #40: 입양 앱이 설정 파일을 복구할 수 있게 데몬이
                            // 보관 중인 cleanup_paths를 함께 돌려준다.
                            cleanup_paths: session.cleanup_paths.clone(),
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
                // v2 브로커 세션도 같은 Kill로 의도적 종료할 수 있다. 테이블에서
                // 먼저 꺼내(락 즉시 해제) 이후 killer/io 락은 broker 테이블 락 밖에서
                // 잡는다 -- if-let scrutinee의 임시 가드가 body 끝까지 broker 락을
                // 쥐면, 그 사이 session.io.lock()을 기다리는 동안 다른 브로커
                // 요청(Spawn/List/DataAttach)이 전부 막힌다(edition 2021).
                let killed = broker.lock().unwrap().remove(&agent_id);
                if let Some(session) = killed {
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
mod tests;
