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
        for a in &o.args {
            cmd.arg(a);
        }
        cmd.cwd(&o.cwd);
        // portable-pty CommandBuilder는 기본적으로 부모 env를 상속한다. 우리는 override만 얹는다.
        for (k, v) in &o.env {
            cmd.env(k, v);
        }

        let child = pair.slave.spawn_command(cmd).map_err(to_io)?;
        drop(pair.slave); // slave는 spawn 후 즉시 닫는다(권장).

        let reader = pair.master.try_clone_reader().map_err(to_io)?;
        let writer = pair.master.take_writer().map_err(to_io)?;
        let killer = child.clone_killer(); // wait 스레드가 child를 소유해도 별도로 kill 가능
        let control = Arc::new(RealControl {
            master: Mutex::new(pair.master),
            killer: Mutex::new(killer),
        });
        let waiter = Box::new(RealWaiter { child });
        Ok(SpawnedPty { reader, writer, control, waiter })
    }
}

fn to_io(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
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
        SpawnedPty { reader, writer, control, waiter }
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
        fn spawn(&self, _opts: PtySpawnOptions) -> io::Result<SpawnedPty> {
            let (control, output_rx, exit_rx) = fresh_control();
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

    fn opts() -> PtySpawnOptions {
        PtySpawnOptions {
            shell: "/bin/sh".into(),
            args: vec![],
            cols: 80,
            rows: 24,
            cwd: ".".into(),
            env: vec![],
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
}
