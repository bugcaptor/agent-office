// src-tauri/src/session/pty_factory.rs
//
// PTY spawn abstraction (test injection point). `SessionManager` only knows
// the `PtyFactory` trait; production wires `PortablePtyFactory`
// (portable-pty, real OS process), tests wire `FakePtyFactory` (in-memory,
// no process). Shell selection (`$SHELL -l -i` / Windows powershell) is a
// SessionManager concern (its `default_shell`) — this module only spawns
// whatever `PtySpawnOptions` it is given.

use std::io::{self, Read, Write};
use std::sync::Arc;

// parking_lot::Mutex(poisoning 없음) — manager.rs와 같은 이유. RealControl의
// resize/kill 경로가 다른 스레드의 패닉에 오염되지 않게 한다.
// (테스트 전용 fake 모듈은 자체적으로 std::sync::Mutex를 임포트해 그대로 쓴다.)
use parking_lot::Mutex;

/// PTY spawn 결과 번들. 리더/라이터/제어/대기를 분리해 페이크 주입을 쉽게 한다.
pub struct SpawnedPty {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub control: Arc<dyn PtyControl>, // resize + kill (여러 스레드 공유)
    pub waiter: Box<dyn PtyWaiter>,   // 블로킹 wait, 소유 이전
    /// 세션 핸드오프(§핵심 1) — 리더 스레드를 확정적으로 멈추는 스위치.
    /// unix에서만 Some. Fake/Windows는 None(핸드오프 자체가 unix 전용 기능).
    #[cfg(unix)]
    pub reader_interrupt: Option<crate::session::poll_reader::ReaderInterrupt>,
    #[cfg(not(unix))]
    pub reader_interrupt: Option<()>,
    /// 세션 핸드오프(§핵심 2) — sessiond에 넘길 마스터 fd(dup 소유)/pid/pgid.
    /// unix에서만 Some.
    #[cfg(unix)]
    pub handoff: Option<HandoffInfo>,
    #[cfg(not(unix))]
    pub handoff: Option<()>,
    /// 이 세션을 v2 브로커 데몬이 소유하는가. `BrokerPtyFactory`의 성공 경로와
    /// 브로커 재접속(`assemble_broker_adopted`)만 true다 — `PortablePtyFactory`,
    /// 팩토리 폴백, v1 fd 입양(`assemble_adopted`)은 전부 false. 브로커 모드
    /// 매니저에서도 폴백으로 생긴 in-process 세션이 섞일 수 있으므로, 전역
    /// `broker_mode`가 아니라 이 **세션 단위 소유 플래그**로 handoff/adopt 경로를
    /// 가른다(브로커 세션=스냅샷 업로드+detach, 폴백 세션=v1 fd 핸드오프).
    pub broker_owned: bool,
    /// 브로커 data 연결의 누적 수신 바이트 카운터(절대 스트림 오프셋). 스냅샷
    /// 업로드 시 "앱이 실제 여기까지 받았다"는 offset으로 동봉해 유실 창을 없앤다
    /// (§P1). 브로커 세션만 Some, 그 외(폴백/PortablePty/v1 입양/Fake)는 None.
    pub broker_stream_offset: Option<Arc<std::sync::atomic::AtomicU64>>,
    /// detach 시 data 소켓을 결정적으로 닫는 핸들(§#50 선결). detach가 이걸
    /// shutdown하면 앱 reader 스레드가 EOF로 종료되고 데몬 conn이 정리돼
    /// List의 `attached`가 false로 돌아간다 — 그래야 재시작/크래시 후 다음
    /// 인스턴스가 안전히 입양한다. 브로커 세션만 Some(unix).
    #[cfg(unix)]
    pub broker_data_shutdown: Option<crate::session::broker_pty::BrokerDataShutdown>,
    #[cfg(not(unix))]
    pub broker_data_shutdown: Option<()>,
}

/// 핸드오프 시 sessiond에 전달할 마스터 fd(및 프로세스 식별자). `master_fd`는
/// spawn 시점에 dup한 이 구조체만의 소유 fd — `RealControl`이 쥔 원본
/// `MasterPty`의 수명과 분리해 두어야, 세션 맵에서 제거되며 `RealControl`이
/// 드롭돼도(핸드오프 완료 후) 이 fd는 살아남는다. `take_master_fd()`로
/// 소유권을 넘기지 않으면 Drop에서 닫힌다(핸드오프 안 하고 세션이 정상
/// 종료되는 경우의 fd 누수 방지).
#[cfg(unix)]
pub struct HandoffInfo {
    master_fd: std::os::unix::io::RawFd,
    pub pid: Option<i32>,
    pub pgid: Option<i32>,
}

#[cfg(unix)]
impl HandoffInfo {
    /// 소유권을 호출자에게 넘기고 Drop에서 닫지 않게 한다 — 이후 fd를 닫는
    /// 책임은 전적으로 호출자(handoff_all의 sessiond 전송 경로)에게 있다.
    pub fn take_master_fd(mut self) -> std::os::unix::io::RawFd {
        let fd = self.master_fd;
        self.master_fd = -1;
        fd
    }
}

#[cfg(unix)]
impl Drop for HandoffInfo {
    fn drop(&mut self) {
        if self.master_fd >= 0 {
            let _ = nix::unistd::close(self.master_fd);
        }
    }
}

pub trait PtyControl: Send + Sync {
    fn resize(&self, cols: u16, rows: u16) -> io::Result<()>;
    fn kill(&self) -> io::Result<()>;
}

pub trait PtyWaiter: Send {
    /// 블로킹. 프로세스 종료까지 대기 후 결과 반환.
    fn wait(self: Box<Self>) -> ExitOutcome;
}

#[derive(Debug, Clone, Copy)]
pub struct ExitOutcome {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
}

pub struct PtySpawnOptions {
    pub shell: String,
    pub args: Vec<String>,
    pub cols: u16,
    pub rows: u16,
    pub cwd: String,
    pub env: Vec<(String, String)>,
    /// 세션을 소유할 에이전트/세션 식별자. `PortablePtyFactory`/Fake는 무시하지만
    /// `BrokerPtyFactory`(v2)는 이걸 데몬 Spawn 메시지의 테이블 키/세션 id로 쓴다.
    pub agent_id: String,
    pub session_id: String,
    /// 관찰자 설정 파일 등 세션 종료 시 지울 경로. `BrokerPtyFactory`가 데몬에
    /// 넘겨, 앱 크래시 후 자식을 kill할 때 데몬이 정리할 수 있게 한다(정상
    /// 경로에서는 앱 쪽 Session이 on_exit/dispose에서도 지운다 -- 이중 정리는
    /// NotFound 무시라 무해).
    pub cleanup_paths: Vec<String>,
}

/// 부작용 경계. SessionManager는 이 트레잇만 안다. 테스트는 FakePtyFactory 주입.
pub trait PtyFactory: Send + Sync {
    fn spawn(&self, opts: PtySpawnOptions) -> io::Result<SpawnedPty>;
}

// ── 실제 portable-pty 구현 ─────────────────────────────────────────────
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

pub struct PortablePtyFactory;

struct RealControl {
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
}
impl PtyControl for RealControl {
    fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        self.master
            .lock()
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| io::Error::other(e.to_string()))
    }
    fn kill(&self) -> io::Result<()> {
        self.killer.lock().kill()
    }
}

struct RealWaiter {
    child: Box<dyn portable_pty::Child + Send + Sync>,
}
impl PtyWaiter for RealWaiter {
    fn wait(mut self: Box<Self>) -> ExitOutcome {
        match self.child.wait() {
            Ok(status) => ExitOutcome { exit_code: Some(status.exit_code() as i32), signal: None },
            Err(_) => ExitOutcome { exit_code: None, signal: None },
        }
    }
}

impl PtyFactory for PortablePtyFactory {
    fn spawn(&self, o: PtySpawnOptions) -> io::Result<SpawnedPty> {
        let sys = native_pty_system();
        let pair = sys
            .openpty(PtySize { rows: o.rows, cols: o.cols, pixel_width: 0, pixel_height: 0 })
            .map_err(to_io)?;

        let mut cmd = CommandBuilder::new(&o.shell);
        // portable-pty 0.8.1 rebuilds the Windows base environment from the
        // registry after reading the live process environment, which replaces
        // process-only overrides such as PATH. Re-apply the actual parent env
        // so live parent values win just as they do for ordinary child processes.
        #[cfg(windows)]
        for (key, value) in std::env::vars_os() {
            cmd.env(key, value);
        }
        for a in &o.args {
            cmd.arg(a);
        }
        cmd.cwd(&o.cwd);
        // Session-specific values take precedence over the inherited snapshot.
        for (k, v) in &o.env {
            cmd.env(k, v);
        }

        let child = pair.slave.spawn_command(cmd).map_err(to_io)?;
        drop(pair.slave); // slave는 spawn 후 즉시 닫는다(권장).

        #[cfg(unix)]
        let (reader, reader_interrupt, handoff): (
            Box<dyn Read + Send>,
            Option<crate::session::poll_reader::ReaderInterrupt>,
            Option<HandoffInfo>,
        ) = {
            // try_clone_reader() 대신 as_raw_fd()를 poll 기반 리더로 직접 읽는다
            // (§핵심 1) — 핸드오프 시 이 스레드를 확정적으로 멈춰야 커널 tty
            // 버퍼에 남은 바이트를 데몬이 무손실로 이어받는다.
            let raw_fd = pair
                .master
                .as_raw_fd()
                .ok_or_else(|| io::Error::other("master pty has no raw fd"))?;
            let (poll_reader, interrupt) = crate::session::poll_reader::spawn(raw_fd)?;
            let dup_fd = nix::unistd::dup(raw_fd).map_err(to_io)?;
            let handoff = HandoffInfo {
                master_fd: dup_fd,
                pid: child.process_id().map(|p| p as i32),
                pgid: pair.master.process_group_leader(),
            };
            (Box::new(poll_reader), Some(interrupt), Some(handoff))
        };
        #[cfg(not(unix))]
        let (reader, reader_interrupt, handoff): (Box<dyn Read + Send>, Option<()>, Option<()>) =
            (pair.master.try_clone_reader().map_err(to_io)?, None, None);

        let writer = pair.master.take_writer().map_err(to_io)?;
        let killer = child.clone_killer(); // wait 스레드가 child를 소유해도 별도로 kill 가능
        let control = Arc::new(RealControl {
            master: Mutex::new(pair.master),
            killer: Mutex::new(killer),
        });
        let waiter = Box::new(RealWaiter { child });
        Ok(SpawnedPty {
            reader,
            writer,
            control,
            waiter,
            reader_interrupt,
            handoff,
            broker_owned: false, // 프로세스 내 직접 스폰 -- 브로커 소유 아님(v1 fd 핸드오프 대상).
            broker_stream_offset: None,
            broker_data_shutdown: None,
        })
    }
}

fn to_io(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

// ── 입양된(adopted) 세션 구성요소 (unix 전용, §핵심 4) ──────────────────
//
// sessiond가 되돌려준 fd로 `SpawnedPty`와 동형인 번들을 재조립한다. 자식을
// 직접 spawn한 게 아니라 waitpid가 불가능하므로 `RealWaiter`(child.wait())
// 대신 마스터 fd의 EOF를 종료 신호로 쓰는 `EofWaiter`를 쓴다.

#[cfg(unix)]
pub struct AdoptedControl {
    master_fd: std::sync::atomic::AtomicI32, // -1 == 이미 닫힘
    pgid: Option<libc::pid_t>,
    pid: Option<libc::pid_t>,
}

#[cfg(unix)]
impl PtyControl for AdoptedControl {
    fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let fd = self.master_fd.load(std::sync::atomic::Ordering::SeqCst);
        if fd < 0 {
            return Ok(());
        }
        let ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
        // portable-pty의 unix.rs PtyFd::resize와 동일한 저수준 호출 -- 우리는
        // MasterPty 트레잇 객체가 아니라 fd 정수 하나만 쥐고 있어 그쪽 구현을
        // 재사용할 수 없다.
        let rc = unsafe { libc::ioctl(fd, libc::TIOCSWINSZ as _, &ws as *const _) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn kill(&self) -> io::Result<()> {
        use nix::sys::signal::{kill, killpg, Signal};
        use nix::unistd::Pid;
        // pgid 없으면 pid로 폴백(설계 문서 §핵심 4).
        if let Some(pgid) = self.pgid {
            let _ = killpg(Pid::from_raw(pgid), Signal::SIGKILL);
        } else if let Some(pid) = self.pid {
            let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
        }
        Ok(())
    }
}

#[cfg(unix)]
impl Drop for AdoptedControl {
    fn drop(&mut self) {
        let fd = self.master_fd.swap(-1, std::sync::atomic::Ordering::SeqCst);
        if fd >= 0 {
            let _ = nix::unistd::close(fd);
        }
    }
}

/// 마스터 fd의 EOF까지 블로킹하는 대기자. 자식을 spawn하지 않았으므로
/// waitpid는 불가 -- `ExitOutcome`은 항상 `{ None, None }`(exit code/signal
/// 모름), on_exit이 kill_requested로 intentional 여부를 가린다.
#[cfg(unix)]
pub struct EofWaiter {
    rx: std::sync::mpsc::Receiver<()>,
}

#[cfg(unix)]
impl PtyWaiter for EofWaiter {
    fn wait(self: Box<Self>) -> ExitOutcome {
        let _ = self.rx.recv();
        ExitOutcome { exit_code: None, signal: None }
    }
}

/// 재핸드오프(입양된 세션을 다시 핸드오프)를 위한 의도적 인터럽트와, 진짜
/// 프로세스 종료(마스터 EOF)를 구분하는 게이트. `stopping`이 true인 채로
/// Ok(0)을 보면 EofWaiter에 신호를 보내지 않는다 -- 그러지 않으면 재핸드오프
/// 인터럽트가 매번 세션을 "Exited"로 오판시킨다.
#[cfg(unix)]
struct AdoptedReader {
    inner: crate::session::poll_reader::PollReader,
    stopping: Arc<std::sync::atomic::AtomicBool>,
    exit_tx: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

#[cfg(unix)]
impl Read for AdoptedReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n == 0 && !self.stopping.load(std::sync::atomic::Ordering::SeqCst) {
            if let Some(tx) = self.exit_tx.lock().take() {
                let _ = tx.send(());
            }
        }
        Ok(n)
    }
}

/// `master_fd`(데몬에게서 SCM_RIGHTS로 받은, 호출자 소유의 fd)로부터
/// `SpawnedPty`와 동형인 번들을 만든다. 반환된 `Arc<AtomicBool>`은
/// "재핸드오프 정지 게이트" -- 이 세션을 다시 핸드오프할 때 `reader_interrupt`를
/// 발화하기 *직전*에 반드시 `true`로 세팅해야 EofWaiter가 오발화하지 않는다.
#[cfg(unix)]
pub fn assemble_adopted(
    master_fd: std::os::unix::io::RawFd,
    pid: Option<i32>,
    pgid: Option<i32>,
) -> io::Result<(SpawnedPty, Arc<std::sync::atomic::AtomicBool>)> {
    use std::os::unix::io::FromRawFd;

    let (poll_reader, interrupt) = crate::session::poll_reader::spawn(master_fd)?;
    let stopping = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (exit_tx, exit_rx) = std::sync::mpsc::channel();
    let reader: Box<dyn Read + Send> = Box::new(AdoptedReader {
        inner: poll_reader,
        stopping: stopping.clone(),
        exit_tx: Mutex::new(Some(exit_tx)),
    });

    let writer_fd = nix::unistd::dup(master_fd).map_err(io::Error::from)?;
    let writer: Box<dyn Write + Send> = Box::new(unsafe { std::fs::File::from_raw_fd(writer_fd) });

    let control = Arc::new(AdoptedControl {
        master_fd: std::sync::atomic::AtomicI32::new(master_fd),
        pgid,
        pid,
    });
    let waiter: Box<dyn PtyWaiter> = Box::new(EofWaiter { rx: exit_rx });

    let handoff_fd = nix::unistd::dup(master_fd).map_err(io::Error::from)?;
    let handoff = Some(HandoffInfo { master_fd: handoff_fd, pid, pgid });

    Ok((
        SpawnedPty {
            reader,
            writer,
            control,
            waiter,
            reader_interrupt: Some(interrupt),
            handoff,
            broker_owned: false, // v1 fd 입양 세션 -- 재핸드오프도 v1 경로.
            broker_stream_offset: None,
            broker_data_shutdown: None,
        },
        stopping,
    ))
}

// ── 테스트용 Fake ──────────────────────────────────────────────────────
//
// 인메모리 파이프 리더, 기록용 writer(Arc<Mutex<Vec<u8>>>), 테스트가
// firing하는 exit 채널. `FakePtyFactory::new()`는 (factory, control) 쌍을
// 돌려주고, `control`은 트레잇 밖의 테스트 전용 메서드(write 조회, 출력 주입,
// exit 발화, resize/kill 관찰)를 노출한다. 한 factory는 단발성으로, `spawn`은
// 최초 1회만 완전한 `SpawnedPty`를 만든다(2번째 호출은 채널이 이미 소비돼 에러).
#[cfg(test)]
pub mod fake {
    use super::{ExitOutcome, PtyControl, PtyFactory, PtySpawnOptions, PtyWaiter, SpawnedPty};
    use std::io::{self, Read, Write};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    /// Blocking in-memory "pipe" reader. `push` on the paired `FakeControl`
    /// sends a chunk; `close` sends `None` to signal EOF. `recv()` blocks
    /// the reader thread exactly like a real PTY fd would.
    struct PipeReader {
        rx: mpsc::Receiver<Option<Vec<u8>>>,
        buf: Vec<u8>,
        pos: usize,
        eof: bool,
    }

    impl Read for PipeReader {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            // Loop until we hold a non-empty chunk: an injected empty chunk
            // must never surface as Ok(0), which downstream reader loops
            // (`Ok(0) => break`) would misread as EOF.
            while self.pos >= self.buf.len() {
                if self.eof {
                    return Ok(0);
                }
                match self.rx.recv() {
                    Ok(Some(chunk)) => {
                        self.buf = chunk;
                        self.pos = 0;
                    }
                    Ok(None) | Err(_) => {
                        self.eof = true;
                        return Ok(0);
                    }
                }
            }
            let n = out.len().min(self.buf.len() - self.pos);
            out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    /// Writer whose bytes are recorded for test assertions instead of sent
    /// anywhere real.
    struct RecordingWriter {
        buf: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.buf.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct FakeWaiter {
        rx: mpsc::Receiver<ExitOutcome>,
    }

    impl PtyWaiter for FakeWaiter {
        fn wait(self: Box<Self>) -> ExitOutcome {
            self.rx
                .recv()
                .unwrap_or(ExitOutcome { exit_code: None, signal: None })
        }
    }

    /// Test-facing handle for one fake spawn. Implements `PtyControl` for
    /// `SessionManager`, plus extra methods (not on the trait) that let a
    /// test inspect/drive the fake session.
    pub struct FakeControl {
        writes: Arc<Mutex<Vec<u8>>>,
        output_tx: mpsc::Sender<Option<Vec<u8>>>,
        exit_tx: Mutex<Option<mpsc::Sender<ExitOutcome>>>,
        resizes: Mutex<Vec<(u16, u16)>>,
        kills: Mutex<u32>,
        env: Mutex<Vec<(String, String)>>,
        cwd: Mutex<String>,
    }

    impl PtyControl for FakeControl {
        fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
            self.resizes.lock().unwrap().push((cols, rows));
            Ok(())
        }
        fn kill(&self) -> io::Result<()> {
            *self.kills.lock().unwrap() += 1;
            Ok(())
        }
    }

    impl FakeControl {
        /// Bytes the session under test has written to the PTY (stdin),
        /// decoded as UTF-8 lossily-free (panics on invalid UTF-8, which
        /// would itself indicate a test bug for our text-only protocol).
        pub fn writes_utf8(&self) -> String {
            String::from_utf8(self.writes.lock().unwrap().clone()).expect("writes were not valid UTF-8")
        }

        /// Raw recorded writes, for tests that care about exact bytes.
        ///
        /// No current test needs this (all existing assertions go through
        /// `writes_utf8`), but it's kept as part of `FakeControl`'s public
        /// test-fixture surface for future non-UTF-8 assertions. Previously
        /// silenced crate-wide by `lib.rs`'s scaffold `#[allow(dead_code)]`
        /// on `mod session`; a later change replaced that scaffold with the real
        /// bootstrap (no blanket allow), which is what surfaced this
        /// warning under `cargo clippy --all-targets` / `cargo test`.
        #[allow(dead_code)]
        pub fn writes_raw(&self) -> Vec<u8> {
            self.writes.lock().unwrap().clone()
        }

        /// Inject a chunk of "PTY output" (stdout/stderr) that the
        /// session's reader thread will observe on its next `read`.
        pub fn push_output(&self, bytes: &[u8]) {
            let _ = self.output_tx.send(Some(bytes.to_vec()));
        }

        /// Signal EOF on the reader side (as if the child process closed
        /// its output).
        pub fn close_output(&self) {
            let _ = self.output_tx.send(None);
        }

        /// Fire process exit with the given exit code; unblocks the
        /// session's waiter thread. No-op if already fired.
        pub fn fire_exit(&self, exit_code: i32) {
            if let Some(tx) = self.exit_tx.lock().unwrap().take() {
                let _ = tx.send(ExitOutcome { exit_code: Some(exit_code), signal: None });
            }
        }

        /// Fire process exit via signal (no exit code) instead of a normal
        /// exit code.
        pub fn fire_exit_signal(&self, signal: i32) {
            if let Some(tx) = self.exit_tx.lock().unwrap().take() {
                let _ = tx.send(ExitOutcome { exit_code: None, signal: Some(signal) });
            }
        }

        pub fn resize_calls(&self) -> Vec<(u16, u16)> {
            self.resizes.lock().unwrap().clone()
        }

        pub fn kill_count(&self) -> u32 {
            *self.kills.lock().unwrap()
        }

        /// The `PtySpawnOptions.env` the factory was spawned with (recorded
        /// by `FakePtyFactory::spawn`), for tests that assert on env
        /// plumbing (e.g. `AGENT_OFFICE_SETTINGS`).
        pub fn spawned_env(&self) -> Vec<(String, String)> {
            self.env.lock().unwrap().clone()
        }

        fn record_env(&self, env: Vec<(String, String)>) {
            *self.env.lock().unwrap() = env;
        }

        /// The `PtySpawnOptions.cwd` the factory was spawned with (recorded
        /// by `FakePtyFactory::spawn`), for tests that assert on cwd
        /// plumbing (e.g. leading-`~` expansion).
        pub fn spawned_cwd(&self) -> String {
            self.cwd.lock().unwrap().clone()
        }

        fn record_cwd(&self, cwd: String) {
            *self.cwd.lock().unwrap() = cwd;
        }
    }

    /// Builds one fresh `FakeControl` + its paired reader/waiter channels.
    /// Shared by the single-spawn `FakePtyFactory` and the multi-spawn
    /// `MultiFakePtyFactory` so their fakes behave identically.
    fn fresh_control() -> (
        Arc<FakeControl>,
        mpsc::Receiver<Option<Vec<u8>>>,
        mpsc::Receiver<ExitOutcome>,
    ) {
        let (output_tx, output_rx) = mpsc::channel();
        let (exit_tx, exit_rx) = mpsc::channel();
        let control = Arc::new(FakeControl {
            writes: Arc::new(Mutex::new(Vec::new())),
            output_tx,
            exit_tx: Mutex::new(Some(exit_tx)),
            resizes: Mutex::new(Vec::new()),
            kills: Mutex::new(0),
            env: Mutex::new(Vec::new()),
            cwd: Mutex::new(String::new()),
        });
        (control, output_rx, exit_rx)
    }

    /// Assembles a `SpawnedPty` bundle from a control + its channels.
    fn spawned_from(
        control: Arc<FakeControl>,
        output_rx: mpsc::Receiver<Option<Vec<u8>>>,
        exit_rx: mpsc::Receiver<ExitOutcome>,
    ) -> SpawnedPty {
        let reader = Box::new(PipeReader { rx: output_rx, buf: Vec::new(), pos: 0, eof: false });
        let writer = Box::new(RecordingWriter { buf: control.writes.clone() });
        let waiter = Box::new(FakeWaiter { rx: exit_rx });
        // 핸드오프는 unix 전용 실제 PTY 기능 -- 페이크는 항상 None(설계 문서
        // "Fake/Windows는 None").
        SpawnedPty {
            reader,
            writer,
            control,
            waiter,
            reader_interrupt: None,
            handoff: None,
            broker_owned: false,
            broker_stream_offset: None,
            broker_data_shutdown: None,
        }
    }

    /// In-memory `PtyFactory` for unit tests. See module docs above.
    pub struct FakePtyFactory {
        control: Arc<FakeControl>,
        output_rx: Mutex<Option<mpsc::Receiver<Option<Vec<u8>>>>>,
        exit_rx: Mutex<Option<mpsc::Receiver<ExitOutcome>>>,
    }

    impl FakePtyFactory {
        /// Returns the factory plus the control handle a test uses to
        /// drive/inspect the (single) session it will spawn.
        pub fn new() -> (Self, Arc<FakeControl>) {
            let (control, output_rx, exit_rx) = fresh_control();
            let factory = FakePtyFactory {
                control: control.clone(),
                output_rx: Mutex::new(Some(output_rx)),
                exit_rx: Mutex::new(Some(exit_rx)),
            };
            (factory, control)
        }
    }

    impl PtyFactory for FakePtyFactory {
        fn spawn(&self, opts: PtySpawnOptions) -> io::Result<SpawnedPty> {
            self.control.record_env(opts.env);
            self.control.record_cwd(opts.cwd);
            let output_rx = self
                .output_rx
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| io::Error::other("FakePtyFactory::spawn called more than once"))?;
            let exit_rx = self
                .exit_rx
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| io::Error::other("FakePtyFactory::spawn called more than once"))?;

            Ok(spawned_from(self.control.clone(), output_rx, exit_rx))
        }
    }

    /// Multi-spawn in-memory `PtyFactory`: hands out a *fresh* `FakeControl`
    /// on every `spawn`, recording each so a test can drive/inspect the Nth
    /// session. Needed for same-agent session-recreate tests, where
    /// a single agent legitimately spawns more than one PTY over its lifetime.
    #[derive(Default)]
    pub struct MultiFakePtyFactory {
        controls: Mutex<Vec<Arc<FakeControl>>>,
    }

    impl MultiFakePtyFactory {
        pub fn new() -> Self {
            Self::default()
        }

        /// All controls handed out so far, in spawn order.
        pub fn controls(&self) -> Vec<Arc<FakeControl>> {
            self.controls.lock().unwrap().clone()
        }
    }

    impl PtyFactory for MultiFakePtyFactory {
        fn spawn(&self, opts: PtySpawnOptions) -> io::Result<SpawnedPty> {
            let (control, output_rx, exit_rx) = fresh_control();
            control.record_env(opts.env);
            control.record_cwd(opts.cwd);
            self.controls.lock().unwrap().push(control.clone());
            Ok(spawned_from(control, output_rx, exit_rx))
        }
    }

    /// `PtyFactory` whose `spawn` always fails — for exercising create()'s
    /// spawn-error cleanup path.
    pub struct AlwaysFailPtyFactory;

    impl PtyFactory for AlwaysFailPtyFactory {
        fn spawn(&self, _opts: PtySpawnOptions) -> io::Result<SpawnedPty> {
            Err(io::Error::other("AlwaysFailPtyFactory: spawn always fails"))
        }
    }

    /// `PtyFactory` whose `spawn` PANICS — for exercising create()'s
    /// panic-safety (훅 설정 파일이 unwind 경로에서도 정리되는지).
    pub struct PanickingPtyFactory;

    impl PtyFactory for PanickingPtyFactory {
        fn spawn(&self, _opts: PtySpawnOptions) -> io::Result<SpawnedPty> {
            panic!("PanickingPtyFactory: simulated panic inside spawn")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakePtyFactory;
    use super::{PtyFactory, PtySpawnOptions};
    use std::io::{Read, Write};

    #[cfg(windows)]
    static PROCESS_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(windows)]
    struct PathRestore(Option<std::ffi::OsString>);

    #[cfg(windows)]
    impl Drop for PathRestore {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("PATH", value),
                None => std::env::remove_var("PATH"),
            }
        }
    }

    fn opts() -> PtySpawnOptions {
        PtySpawnOptions {
            shell: "/bin/sh".into(),
            args: vec![],
            cols: 80,
            rows: 24,
            cwd: ".".into(),
            env: vec![],
            agent_id: "test-agent".into(),
            session_id: "test-session".into(),
            cleanup_paths: vec![],
        }
    }

    #[test]
    fn fake_records_writes() {
        let (fac, ctl) = FakePtyFactory::new();
        let mut spawned = fac.spawn(opts()).unwrap();
        spawned.writer.write_all(b"claude --settings \"x\"\n").unwrap();
        assert_eq!(ctl.writes_utf8(), "claude --settings \"x\"\n");
    }

    #[test]
    fn fake_injected_output_is_readable() {
        let (fac, ctl) = FakePtyFactory::new();
        let mut spawned = fac.spawn(opts()).unwrap();
        ctl.push_output(b"hello ");
        ctl.push_output(b"world");
        ctl.close_output();

        let mut out = Vec::new();
        spawned.reader.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"hello world");
    }

    #[test]
    fn fake_output_read_blocks_until_pushed_then_returns_eof_after_close() {
        let (fac, ctl) = FakePtyFactory::new();
        let mut spawned = fac.spawn(opts()).unwrap();

        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 16];
            let n = spawned.reader.read(&mut buf).unwrap();
            (buf[..n].to_vec(), spawned)
        });

        // Give the reader thread a moment to block on recv() before we push.
        std::thread::sleep(std::time::Duration::from_millis(20));
        ctl.push_output(b"abc");
        let (first, mut spawned) = handle.join().unwrap();
        assert_eq!(first, b"abc");

        ctl.close_output();
        let mut buf = [0u8; 16];
        let n = spawned.reader.read(&mut buf).unwrap();
        assert_eq!(n, 0, "reader should observe EOF after close_output");
    }

    #[test]
    fn fake_exit_is_observable_by_waiter() {
        let (fac, ctl) = FakePtyFactory::new();
        let spawned = fac.spawn(opts()).unwrap();

        let handle = std::thread::spawn(move || spawned.waiter.wait());
        std::thread::sleep(std::time::Duration::from_millis(20));
        ctl.fire_exit(7);

        let outcome = handle.join().unwrap();
        assert_eq!(outcome.exit_code, Some(7));
        assert_eq!(outcome.signal, None);
    }

    #[test]
    fn fake_exit_via_signal_has_no_exit_code() {
        let (fac, ctl) = FakePtyFactory::new();
        let spawned = fac.spawn(opts()).unwrap();
        ctl.fire_exit_signal(9);
        let outcome = spawned.waiter.wait();
        assert_eq!(outcome.exit_code, None);
        assert_eq!(outcome.signal, Some(9));
    }

    #[test]
    fn fake_control_records_resize_and_kill() {
        let (fac, ctl) = FakePtyFactory::new();
        let spawned = fac.spawn(opts()).unwrap();

        spawned.control.resize(100, 40).unwrap();
        spawned.control.kill().unwrap();

        assert_eq!(ctl.resize_calls(), vec![(100, 40)]);
        assert_eq!(ctl.kill_count(), 1);
    }

    #[test]
    fn fake_empty_chunk_is_skipped_not_treated_as_eof() {
        // Regression: an injected zero-length chunk must not surface as
        // Ok(0) — downstream reader loops treat Ok(0) as EOF and
        // would silently terminate the simulated output stream.
        let (fac, ctl) = FakePtyFactory::new();
        let mut spawned = fac.spawn(opts()).unwrap();

        ctl.push_output(b"");
        ctl.push_output(b"data");

        let mut buf = [0u8; 16];
        let n = spawned.reader.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"data", "read must skip the empty chunk and deliver the next data");

        // Only a genuine close yields Ok(0).
        ctl.push_output(b"");
        ctl.close_output();
        let n = spawned.reader.read(&mut buf).unwrap();
        assert_eq!(n, 0, "Ok(0) only at real EOF, even with a trailing empty chunk");
    }

    #[test]
    fn fake_spawn_can_only_be_called_once() {
        let (fac, _ctl) = FakePtyFactory::new();
        fac.spawn(opts()).unwrap();
        assert!(fac.spawn(opts()).is_err());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "spawns a real Windows PTY"]
    fn portable_factory_preserves_live_parent_path() {
        use super::PortablePtyFactory;

        let _env_guard = PROCESS_ENV_LOCK.lock().unwrap();
        let old_path = std::env::var_os("PATH");
        let _restore = PathRestore(old_path.clone());
        let marker = format!(
            r"C:\agent-office-parent-path-sentinel-{}",
            std::process::id()
        );
        let mut paths = vec![std::path::PathBuf::from(&marker)];
        if let Some(old_path) = old_path {
            paths.extend(std::env::split_paths(&old_path));
        }
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());

        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let shell = std::path::Path::new(&system_root)
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
            .to_string_lossy()
            .into_owned();
        let output_path = std::env::temp_dir().join(format!(
            "agent-office-parent-path-{}.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&output_path);
        let output_path_ps = output_path.to_string_lossy().replace('\'', "''");
        let spawned = PortablePtyFactory
            .spawn(PtySpawnOptions {
                shell,
                args: vec![
                    "-NoLogo".into(),
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    format!("$env:Path | Set-Content -LiteralPath '{output_path_ps}' -NoNewline"),
                ],
                cols: 240,
                rows: 24,
                cwd: std::env::current_dir()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                env: vec![],
                agent_id: "test-agent".into(),
                session_id: "test-session".into(),
                cleanup_paths: vec![],
            })
            .unwrap();

        let outcome = spawned.waiter.wait();
        assert_eq!(outcome.exit_code, Some(0));
        let output = std::fs::read_to_string(&output_path).unwrap();
        assert!(
            output.contains(&marker),
            "live parent PATH marker was lost before the PTY child: {output:?}"
        );
        std::fs::remove_file(output_path).unwrap();
    }

    /// 설계 문서 §테스트: "poll reader: 실제 openpty + /bin/cat으로 인터럽트 시
    /// 무손실 검증" — cat 대신 `yes | head -c`로 두 개의 독립된 청크(사이에
    /// idle 구간)를 생성해, 첫 청크 도착 후 앱 쪽 poll 리더를 인터럽트하고
    /// 나머지를 핸드오프 fd로 직접(데몬처럼) 이어 읽는다. 두 구간을 이어붙인
    /// 결과가 기대 바이트열과 정확히 같아야 한다 — 유실도 중복도 없어야
    /// "이중 리더 금지"(§핵심 1) 계약이 성립한다.
    #[cfg(unix)]
    #[test]
    fn handoff_interrupt_then_raw_fd_continuation_loses_no_bytes() {
        use super::PortablePtyFactory;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        const CHUNK: usize = 100_000;
        let script = format!(
            "yes AGENTOFFICEHANDOFFTEST | head -c {CHUNK}; sleep 0.3; yes AGENTOFFICEHANDOFFTEST | head -c {CHUNK}"
        );
        let mut spawned = PortablePtyFactory
            .spawn(PtySpawnOptions {
                shell: "/bin/sh".into(),
                args: vec!["-c".into(), script],
                cols: 80,
                rows: 24,
                cwd: ".".into(),
                env: vec![],
                agent_id: "test-agent".into(),
                session_id: "test-session".into(),
                cleanup_paths: vec![],
            })
            .unwrap();

        let interrupt = spawned
            .reader_interrupt
            .take()
            .expect("unix spawn must produce a reader_interrupt");
        let handoff = spawned
            .handoff
            .take()
            .expect("unix spawn must produce handoff info");

        // 자식을 백그라운드에서 reap -- 테스트가 좀비를 남기지 않게.
        let waiter = spawned.waiter;
        std::thread::spawn(move || {
            waiter.wait();
        });

        let read_total = Arc::new(AtomicUsize::new(0));
        let read_total_for_thread = read_total.clone();
        let mut reader = spawned.reader;
        let reader_handle = std::thread::spawn(move || {
            let mut acc = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        acc.extend_from_slice(&buf[..n]);
                        read_total_for_thread.store(acc.len(), Ordering::SeqCst);
                    }
                    Err(_) => break,
                }
            }
            acc
        });

        // 첫 청크가 완전히 도착할 때까지 대기 -- 스크립트의 `sleep 0.3`이
        // 만드는 idle 구간에서 인터럽트해야 새 바이트 도착과 경합하지 않는다.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while read_total.load(Ordering::SeqCst) < CHUNK {
            assert!(
                std::time::Instant::now() < deadline,
                "first chunk never fully arrived"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        interrupt.interrupt();
        let app_side = reader_handle.join().unwrap();

        // 핸드오프 fd를 데몬처럼 그대로 이어 읽는다.
        let master_fd = handoff.take_master_fd();
        let mut daemon_side = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match nix::unistd::read(master_fd, &mut buf) {
                Ok(0) => break,
                Ok(n) => daemon_side.extend_from_slice(&buf[..n]),
                Err(nix::errno::Errno::EIO) => break, // macOS: 슬레이브 전부 닫힘 == EOF
                Err(e) => panic!("daemon-side read failed: {e}"),
            }
        }
        let _ = nix::unistd::close(master_fd);

        let mut combined = app_side;
        combined.extend_from_slice(&daemon_side);

        // `head -c CHUNK`가 만드는 원본 바이트열(줄바꿈은 LF)에, pty 라인
        // 디시플린의 기본 출력 후처리(OPOST/ONLCR: LF -> CRLF)를 그대로
        // 시뮬레이션해야 마스터에서 실제로 읽히는 바이트열과 일치한다.
        let raw_chunk = |n: usize| -> Vec<u8> {
            let pattern = b"AGENTOFFICEHANDOFFTEST\n";
            let mut out = Vec::with_capacity(n);
            while out.len() < n {
                out.extend_from_slice(pattern);
            }
            out.truncate(n);
            out
        };
        let onlcr = |raw: &[u8]| -> Vec<u8> {
            let mut out = Vec::with_capacity(raw.len());
            for &b in raw {
                if b == b'\n' {
                    out.push(b'\r');
                }
                out.push(b);
            }
            out
        };
        let mut expected = onlcr(&raw_chunk(CHUNK));
        expected.extend_from_slice(&onlcr(&raw_chunk(CHUNK)));

        assert_eq!(
            combined, expected,
            "handoff must not lose or duplicate bytes across the interrupt boundary"
        );
    }
}
