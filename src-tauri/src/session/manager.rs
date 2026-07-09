// src-tauri/src/session/manager.rs
//
// SessionManager: owns the PTY session lifecycle (reader thread / tokio
// output pump / wait thread), autostart stdin injection, and state
// transitions.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::ipc::Channel;
use uuid::Uuid;

use crate::notification::hook_settings::HookSettingsWriter;
use crate::notification::hub::NotificationHub;
use crate::session::output_batcher::{FlushSink, OutputBatcher, MAX_BYTES, WINDOW_MS};
use crate::session::pty_factory::{ExitOutcome, PtyControl, PtyFactory, PtySpawnOptions};
use crate::session::shells;
#[cfg(not(windows))]
use crate::session::zsh_wrapper;
use crate::state::{AppEvents, SessionRegistry};
use crate::types::*;

const BACKLOG_CAP: usize = 256;

enum ReaderMsg {
    Data(Vec<u8>),
    Eof,
}

/// agentId당 출력 Channel + 등록 이전 백로그. FlushSink 구현체.
pub struct OutputSink {
    channel: Mutex<Option<Channel<OutputChunk>>>,
    backlog: Mutex<std::collections::VecDeque<OutputChunk>>,
}
impl OutputSink {
    fn new() -> Self {
        Self { channel: Mutex::new(None), backlog: Mutex::new(Default::default()) }
    }
    fn attach(&self, ch: Channel<OutputChunk>) {
        // 락 순서 항상 channel → backlog (데드락 방지, emit과 동일 순서).
        let mut c = self.channel.lock().unwrap();
        let mut b = self.backlog.lock().unwrap();
        for chunk in b.drain(..) {
            let _ = ch.send(chunk);
        }
        *c = Some(ch);
    }
    fn detach(&self) {
        *self.channel.lock().unwrap() = None;
    }
}
impl FlushSink for OutputSink {
    fn emit(&self, chunk: OutputChunk) {
        let c = self.channel.lock().unwrap();
        if let Some(ch) = c.as_ref() {
            let _ = ch.send(chunk); // Channel 전송 실패(웹뷰 소멸)는 무시
        } else {
            let mut b = self.backlog.lock().unwrap();
            if b.len() >= BACKLOG_CAP {
                b.pop_front();
            }
            b.push_back(chunk);
        }
    }
}

struct Session {
    session_id: SessionId,
    agent_id: AgentId,
    state: Mutex<SessionState>,
    writer: Mutex<Box<dyn Write + Send>>,
    control: Arc<dyn PtyControl>,
    settings_path: Option<std::path::PathBuf>,
    kill_requested: AtomicBool,
}

pub struct SessionManager {
    factory: Arc<dyn PtyFactory>,
    hook_writer: HookSettingsWriter, // Clone 가능(PathBuf만 보유)
    registry: Arc<SessionRegistry>,
    events: Arc<dyn AppEvents>,
    hub: Arc<NotificationHub>,
    sessions: Mutex<HashMap<AgentId, Arc<Session>>>,
    /// agentId별 출력 sink — 세션 수명과 독립. subscribe 이전 pending attach와
    /// 세션 재생성 시 채널 재사용을 위해 세션이 아니라 여기에 보관한다.
    sinks: Mutex<HashMap<AgentId, Arc<OutputSink>>>,
    get_hook_port: Arc<dyn Fn() -> Option<u16> + Send + Sync>,
    shell_resolver: Arc<dyn Fn(shells::ShellRequest) -> shells::ResolvedShell + Send + Sync>,
}

impl SessionManager {
    pub fn new(
        factory: Arc<dyn PtyFactory>,
        hook_writer: HookSettingsWriter,
        registry: Arc<SessionRegistry>,
        events: Arc<dyn AppEvents>,
        hub: Arc<NotificationHub>,
        get_hook_port: Arc<dyn Fn() -> Option<u16> + Send + Sync>,
    ) -> Self {
        Self {
            factory,
            hook_writer,
            registry,
            events,
            hub,
            sessions: Mutex::new(HashMap::new()),
            sinks: Mutex::new(HashMap::new()),
            get_hook_port,
            shell_resolver: Arc::new(shells::resolve),
        }
    }

    fn find(&self, agent_id: &str) -> Option<Arc<Session>> {
        self.sessions.lock().unwrap().get(agent_id).cloned()
    }

    /// agentId의 출력 sink를 반환(없으면 생성). attach_output이 세션보다 먼저
    /// 호출되면 여기서 sink가 만들어지고, create()는 같은 sink를 이어받아
    /// 이미 붙은 채널/백로그를 그대로 재사용한다.
    fn sink_for(&self, agent_id: &str) -> Arc<OutputSink> {
        self.sinks
            .lock()
            .unwrap()
            .entry(agent_id.to_string())
            .or_insert_with(|| Arc::new(OutputSink::new()))
            .clone()
    }

    pub fn session_id_for(&self, agent_id: &str) -> Option<SessionId> {
        self.find(agent_id).map(|s| s.session_id.clone())
    }

    /// 1 에이전트 1 세션 불변식. self: &Arc<Self>로 wait 스레드에 소유 이전.
    pub fn create(self: &Arc<Self>, req: CreateSessionRequest) -> Result<CreateSessionResult, String> {
        // 살아있는 세션이 있으면 재사용, 새 PTY 안 만듦.
        if let Some(s) = self.find(&req.agent_id) {
            let st = *s.state.lock().unwrap();
            if matches!(st, SessionState::Running | SessionState::Starting) {
                return Ok(CreateSessionResult { session_id: s.session_id.clone(), state: st });
            }
        }

        let session_id = Uuid::new_v4().to_string(); // uuid는 URL-safe → hook 라우팅 키로 안전
        // 훅 opt-in: 포트가 None이면(설정 OFF 또는 서버 미기동) --settings 파일·
        // 훅 env·zsh ZDOTDIR 심을 전부 생략한다 — 세션은 순수한 셸로 뜨고,
        // 알림/시간측정 훅은 발화하지 않는다(새 세션부터 적용 정책).
        let port = (self.get_hook_port)();
        let settings_path: Option<std::path::PathBuf> = match port {
            Some(p) => Some(self.hook_writer.write(&session_id, p).map_err(|e| e.to_string())?),
            None => None,
        };

        // hooks_on: 이번 세션에 AGENT_OFFICE_SETTINGS가 실제로 주입되는지 —
        // 아래 env 주입 조건(port + settings_path 둘 다 Some)과 동일한 신호를
        // 셸 리졸버에 미리 전달해, git-bash 분기가 --rcfile 심 설치 여부를
        // 결정할 수 있게 한다.
        let hooks_on = port.is_some() && settings_path.is_some();
        let resolved = (self.shell_resolver)(shells::ShellRequest { selected: req.shell.as_deref(), hooks_on });
        let cwd = req.cwd.clone().map(expand_tilde).unwrap_or_else(home_dir);
        let mut env = vec![
            ("AGENT_OFFICE_SESSION".into(), session_id.clone()),
            ("TERM".into(), "xterm-256color".into()),
        ];
        if let (Some(p), Some(sp)) = (port, settings_path.as_ref()) {
            env.push(("AGENT_OFFICE_HOOK_URL".into(), format!("http://127.0.0.1:{p}/hook")));
            env.push(("AGENT_OFFICE_SETTINGS".into(), sp.to_string_lossy().into_owned()));
            // macOS/Linux zsh time-tracking fix (Task B): inject a ZDOTDIR shim so
            // the spawned zsh defines a `claude` wrapper that transparently adds
            // `--settings $AGENT_OFFICE_SETTINGS` (see session::zsh_wrapper and
            // the Windows CLAUDE_WRAPPER_PS sibling in session::shells). Windows
            // PowerShell/pwsh and Git Bash get their own wrapper injection inside
            // `session::shells::resolve_with`. 훅 OFF면 심을 설치하지 않는다
            // (래퍼가 주입할 settings 파일 자체가 없음).
            #[cfg(not(windows))]
            if zsh_wrapper::is_zsh(&resolved.program) {
                match zsh_wrapper::ensure_zdotdir() {
                    Ok(dir) => env.push(("ZDOTDIR".into(), dir.to_string_lossy().into_owned())),
                    Err(e) => eprintln!("agent-office: failed to write zsh ZDOTDIR shim: {e}"),
                }
            }
        }
        env.extend(resolved.extra_env.iter().cloned());
        let spawned = match self.factory.spawn(PtySpawnOptions {
            shell: resolved.program,
            args: resolved.args,
            cols: req.cols.unwrap_or(80),
            rows: req.rows.unwrap_or(24),
            cwd,
            env,
        }) {
            Ok(s) => s,
            Err(e) => {
                // spawn 실패: 이미 디스크에 쓴 --settings 파일이 새지 않게 정리.
                self.hook_writer.cleanup(&session_id);
                return Err(e.to_string());
            }
        };

        // 세션 수명과 독립인 agentId sink 재사용: 이미 붙은 채널/백로그를
        // 그대로 이어받아 재생성 시 재구독이 필요 없다.
        let output = self.sink_for(&req.agent_id);
        let session = Arc::new(Session {
            session_id: session_id.clone(),
            agent_id: req.agent_id.clone(),
            state: Mutex::new(SessionState::Starting),
            writer: Mutex::new(spawned.writer),
            control: spawned.control,
            settings_path,
            kill_requested: AtomicBool::new(false),
        });

        self.sessions.lock().unwrap().insert(req.agent_id.clone(), session.clone());
        self.registry.insert(&session_id, &req.agent_id, SessionState::Starting);
        self.emit_state(&session, SessionState::Starting, None);

        // 1) reader thread (블로킹 read → mpsc)
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<ReaderMsg>();
        let mut reader = spawned.reader;
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(ReaderMsg::Data(buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = tx.send(ReaderMsg::Eof);
        });

        // 2) output pump task (배칭 + BEL 감지 + Channel 방출)
        spawn_output_pump(session_id.clone(), req.agent_id.clone(), rx, output, self.hub.clone());

        // 3) wait thread (블로킹 wait → 상태 전이)
        let me = Arc::clone(self);
        let sess = session.clone();
        let waiter = spawned.waiter;
        std::thread::spawn(move || {
            let outcome = waiter.wait();
            me.on_exit(&sess, outcome);
        });

        // Running 전이 (CAS): wait 스레드가 이미 Exited/Disposed로 옮겼다면
        // 덮어쓰지 않는다. state 락을 registry.set_state/emit까지 계속 쥐어
        // on_exit의 전이와 상호 배제 → "Exited 이후 Running" 역전을 원천 차단.
        let started = {
            let mut st = session.state.lock().unwrap();
            if *st == SessionState::Starting {
                *st = SessionState::Running;
                self.registry.set_state(&session_id, SessionState::Running);
                self.emit_state(&session, SessionState::Running, None);
                true
            } else {
                false
            }
        };

        // autostart(기본 false): 세션은 기본적으로 빈 로그인 셸만 띄운다. 사용자가
        // `claude --settings "$AGENT_OFFICE_SETTINGS"`로 직접 기동한다. 명시적으로
        // Some(true)를 요청한 경우에만 stdin 주입 — 단, 실제로 Running으로 전이했을
        // 때만(이미 종료됐다면 주입해봐야 의미 없음).
        if started && req.autostart_claude.unwrap_or(false) {
            // 훅 OFF면 --settings 없이 순수 claude 기동(주입할 설정 파일이 없음).
            let line = match &session.settings_path {
                Some(p) => format!("claude --settings \"{}\"\n", p.display()),
                None => "claude\n".to_string(),
            };
            let _ = session.writer.lock().unwrap().write_all(line.as_bytes());
        }

        let state = *session.state.lock().unwrap();
        Ok(CreateSessionResult { session_id, state })
    }

    pub fn write_input(&self, agent_id: &str, data: &str) {
        if let Some(s) = self.find(agent_id) {
            if *s.state.lock().unwrap() == SessionState::Running {
                let _ = s.writer.lock().unwrap().write_all(data.as_bytes());
            }
        }
    }

    pub fn resize(&self, agent_id: &str, cols: u16, rows: u16) {
        if let Some(s) = self.find(agent_id) {
            if *s.state.lock().unwrap() == SessionState::Running {
                let _ = s.control.resize(cols, rows);
            }
        }
    }

    /// 의도적 종료. 최종 Disposed 전이는 wait 스레드의 on_exit에서 확정.
    pub fn dispose(&self, agent_id: &str) {
        if let Some(s) = self.find(agent_id) {
            s.kill_requested.store(true, Ordering::SeqCst);
            let _ = s.control.kill();
            self.hook_writer.cleanup(&s.session_id);
        }
    }

    /// 앱 quit: 모든 PTY kill + settings 정리(동기, 빠름).
    pub fn dispose_all(&self) {
        let ids: Vec<AgentId> = self.sessions.lock().unwrap().keys().cloned().collect();
        for a in ids {
            self.dispose(&a);
        }
    }

    /// subscribe_output 커맨드가 호출: agentId에 Channel 등록(+백로그 드레인).
    /// 세션이 아직 없어도 sink를 만들어 채널을 보관한다(pending attach) —
    /// 이후 create()가 같은 sink를 이어받아 재구독 없이 출력이 흐른다.
    pub fn attach_output(&self, agent_id: &str, channel: Channel<OutputChunk>) {
        self.sink_for(agent_id).attach(channel);
    }
    pub fn detach_output(&self, agent_id: &str) {
        if let Some(s) = self.sinks.lock().unwrap().get(agent_id) {
            s.detach();
        }
    }

    pub fn pending_notifications(&self, agent_id: &str) -> Vec<NotificationEvent> {
        match self.session_id_for(agent_id) {
            Some(sid) => self.hub.pending(&sid),
            None => Vec::new(),
        }
    }

    fn on_exit(&self, sess: &Arc<Session>, outcome: ExitOutcome) {
        let intentional = sess.kill_requested.load(Ordering::SeqCst);
        let exit = SessionExitInfo {
            session_id: sess.session_id.clone(),
            exit_code: outcome.exit_code,
            signal: outcome.signal,
            intentional,
        };
        let next = if intentional { SessionState::Disposed } else { SessionState::Exited };
        // state 락을 registry/emit까지 계속 쥐어 create()의 Running CAS와 상호
        // 배제한다: 둘 중 하나만 완주 → 상태·이벤트 순서 일관성 보장.
        {
            let mut st = sess.state.lock().unwrap();
            *st = next;
            self.registry.set_state(&sess.session_id, next);
            self.emit_state(sess, next, Some(exit));
        }

        // 미해결 알림 정리.
        self.hub.purge_session(&sess.session_id);

        if next == SessionState::Disposed {
            // 재사용 안 함 → 맵/레지스트리/sink에서 제거(이후 hook은 폐기).
            self.sessions.lock().unwrap().remove(&sess.agent_id);
            self.registry.remove(&sess.session_id);
            self.sinks.lock().unwrap().remove(&sess.agent_id);
        } else {
            // Exited(예기치 않은 종료)는 진단/재기동 위해 레지스트리에 유지하되,
            // 죽은 세션의 --settings 파일은 정리한다. 재기동 시 create()가
            // 새 sessionId로 새 파일을 쓴다.
            self.hook_writer.cleanup(&sess.session_id);
        }
    }

    fn emit_state(&self, sess: &Arc<Session>, state: SessionState, exit: Option<SessionExitInfo>) {
        self.events.session_state(&SessionStateEvent {
            session_id: sess.session_id.clone(),
            agent_id: sess.agent_id.clone(),
            state,
            exit,
            at: now_ms(),
        });
    }
}

#[cfg(test)]
impl SessionManager {
    /// Test-only hook to override `shell_resolver` (normally always
    /// `shells::resolve`) so tests can exercise the zsh ZDOTDIR wiring in
    /// `create()` without depending on the host's actual `$SHELL`, or record
    /// what the resolver was invoked with. Must be called before wrapping in
    /// `Arc::new` (consumes `self` by value).
    fn with_shell_resolver(
        mut self,
        resolver: Arc<dyn Fn(shells::ShellRequest) -> shells::ResolvedShell + Send + Sync>,
    ) -> Self {
        self.shell_resolver = resolver;
        self
    }
}

fn spawn_output_pump(
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
                        batcher.push(&bytes);
                        if batcher.pending_bytes() >= MAX_BYTES {
                            batcher.flush(&*sink);
                            deadline = None;
                        } else if deadline.is_none() {
                            deadline = Some(tokio::time::Instant::now()
                                + std::time::Duration::from_millis(WINDOW_MS));
                        }
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

fn home_dir() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into())
}

/// Expand a leading `~` in a profile's configured cwd against the same
/// home-directory source `home_dir()` uses -- portable_pty spawns with a
/// literal `~` path fail (session immediately exits with no explanation),
/// but the 시작 폴더 UI invites `~/dev/foo`-style input. Only bare `~` and
/// `~/...` are expanded; `~user/...` forms are left untouched (rare, and we
/// have no portable way to resolve another user's home).
fn expand_tilde(path: String) -> String {
    if path == "~" {
        home_dir()
    } else if let Some(rest) = path.strip_prefix("~/") {
        format!("{}/{rest}", home_dir())
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification::hub::{NotificationHub, SystemClock};
    use crate::session::pty_factory::fake::{
        AlwaysFailPtyFactory, FakeControl, FakePtyFactory, MultiFakePtyFactory,
    };
    use crate::state::fake::RecordingEvents;
    use std::path::PathBuf;
    use std::time::Duration;
    use tauri::ipc::{Channel, InvokeResponseBody};

    fn registry() -> Arc<SessionRegistry> {
        Arc::new(SessionRegistry::new())
    }

    fn hub_for(registry: Arc<SessionRegistry>, events: Arc<dyn AppEvents>) -> Arc<NotificationHub> {
        Arc::new(NotificationHub::new(registry, events, Arc::new(SystemClock), Duration::from_millis(3000)))
    }

    /// Unique tempdir per test so parallel `cargo test` runs never collide.
    fn scratch_hook_writer() -> (HookSettingsWriter, PathBuf) {
        let dir = std::env::temp_dir().join(format!("agent-office-manager-test-{}", Uuid::new_v4()));
        (HookSettingsWriter::new(dir.clone()), dir)
    }

    fn req(agent_id: &str, autostart: Option<bool>) -> CreateSessionRequest {
        CreateSessionRequest {
            agent_id: agent_id.into(),
            cols: None,
            rows: None,
            cwd: None,
            shell: None,
            autostart_claude: autostart,
        }
    }

    fn req_with_cwd(agent_id: &str, cwd: Option<String>) -> CreateSessionRequest {
        CreateSessionRequest {
            agent_id: agent_id.into(),
            cols: None,
            rows: None,
            cwd,
            shell: None,
            autostart_claude: Some(false),
        }
    }

    fn req_with_shell(agent_id: &str, shell: Option<String>) -> CreateSessionRequest {
        CreateSessionRequest {
            agent_id: agent_id.into(),
            cols: None,
            rows: None,
            cwd: None,
            shell,
            autostart_claude: Some(false),
        }
    }

    /// Polls `pred` until it's true, panicking after a generous timeout
    /// instead of hanging forever if the pump/wait thread wiring is broken.
    async fn wait_for<F: Fn() -> bool>(pred: F) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while !pred() {
            assert!(tokio::time::Instant::now() < deadline, "condition not met within timeout");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// One `SessionManager` wired to a single-spawn `FakePtyFactory` (per
    /// the fake's own contract: one fake per session under test), with a
    /// caller-chosen `get_hook_port` result — `None` exercises the hooks-OFF
    /// (opt-in disabled) path, `Some(port)` the normal hooks-ON path.
    fn build_with_port(port: Option<u16>) -> (Arc<SessionManager>, Arc<RecordingEvents>, Arc<FakeControl>, PathBuf) {
        let events = Arc::new(RecordingEvents::default());
        let reg = registry();
        let hub = hub_for(reg.clone(), events.clone() as Arc<dyn AppEvents>);
        let (writer, dir) = scratch_hook_writer();
        let (fac, ctl) = FakePtyFactory::new();
        let mgr = Arc::new(SessionManager::new(
            Arc::new(fac),
            writer,
            reg,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(move || port),
        ));
        (mgr, events, ctl, dir)
    }

    fn build() -> (Arc<SessionManager>, Arc<RecordingEvents>, Arc<FakeControl>, PathBuf) {
        build_with_port(Some(12345))
    }

    fn cleanup(ctl: &FakeControl, dir: &PathBuf) {
        // Let the reader thread observe EOF so it doesn't block forever.
        ctl.close_output();
        let _ = std::fs::remove_dir_all(dir);
    }

    // ---- T-A: state transitions + intentional flag ----

    #[tokio::test]
    async fn create_transitions_starting_running_then_exited_on_unexpected_exit() {
        let (mgr, events, ctl, dir) = build();

        let created = mgr.create(req("a1", Some(false))).unwrap();
        assert_eq!(created.state, SessionState::Running);
        assert_eq!(events.states(), vec![SessionState::Starting, SessionState::Running]);

        ctl.fire_exit(1);
        wait_for(|| events.states().len() == 3).await;

        assert_eq!(
            events.states(),
            vec![SessionState::Starting, SessionState::Running, SessionState::Exited]
        );
        let last = events.last_state().exit.unwrap();
        assert!(!last.intentional, "unexpected exit must not be marked intentional");
        assert_eq!(last.exit_code, Some(1));

        // unexpected exit keeps the session in bookkeeping (diagnosis/restart).
        assert_eq!(mgr.session_id_for("a1"), Some(created.session_id.clone()));
        assert_eq!(mgr.registry.resolve_agent(&created.session_id), Some("a1".to_string()));

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_exit_via_signal_is_reported_with_no_exit_code() {
        let (mgr, events, ctl, dir) = build();
        mgr.create(req("a1", Some(false))).unwrap();

        ctl.fire_exit_signal(9);
        wait_for(|| events.states().len() == 3).await;

        let last = events.last_state().exit.unwrap();
        assert!(!last.intentional);
        assert_eq!(last.exit_code, None);
        assert_eq!(last.signal, Some(9));

        cleanup(&ctl, &dir);
    }

    // ---- T-B: autostart stdin injection ----

    #[tokio::test]
    async fn create_autostart_default_skips_stdin_injection() {
        let (mgr, _events, ctl, dir) = build();
        // autostart_claude omitted -> defaults to false (plain shell session);
        // the user runs `claude --settings "$AGENT_OFFICE_SETTINGS"` manually.
        mgr.create(req("a1", None)).unwrap();

        assert_eq!(ctl.writes_utf8(), "", "autostartClaude omitted must not write to stdin");

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_autostart_true_injects_claude_stdin_with_settings_path() {
        let (mgr, _events, ctl, dir) = build();
        let created = mgr.create(req("a1", Some(true))).unwrap();

        let written = ctl.writes_utf8();
        assert!(
            written.starts_with("claude --settings \"") && written.ends_with("\"\n"),
            "unexpected stdin injection: {written:?}"
        );
        assert!(written.contains(&format!("{}.settings.json", created.session_id)));

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_env_includes_agent_office_settings_path() {
        let (mgr, _events, ctl, dir) = build();
        let created = mgr.create(req("a1", None)).unwrap();

        let env = ctl.spawned_env();
        let settings_env = env
            .iter()
            .find(|(k, _)| k == "AGENT_OFFICE_SETTINGS")
            .map(|(_, v)| v.clone())
            .expect("AGENT_OFFICE_SETTINGS must be present in spawn env");
        assert!(
            settings_env.contains(&format!("{}.settings.json", created.session_id)),
            "unexpected AGENT_OFFICE_SETTINGS value: {settings_env:?}"
        );

        cleanup(&ctl, &dir);
    }

    // ---- Task 7: hooks opt-in OFF (get_hook_port -> None) skips wiring ----

    #[tokio::test]
    async fn create_with_hooks_disabled_skips_settings_file_and_hook_env() {
        // get_hook_port가 None을 주면(훅 opt-in OFF): --settings 파일을 쓰지
        // 않고, AGENT_OFFICE_SETTINGS/AGENT_OFFICE_HOOK_URL env도 없다.
        let (mgr, _events, ctl, dir) = build_with_port(None);
        mgr.create(req("a1", None)).unwrap();

        // 훅 설정 파일이 안 쓰였다.
        assert!(
            !dir.exists() || std::fs::read_dir(&dir).unwrap().next().is_none(),
            "no settings file should be written when hooks are disabled"
        );
        // env에 훅 관련 키가 없다.
        let env = ctl.spawned_env();
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"AGENT_OFFICE_SESSION"));
        assert!(!keys.contains(&"AGENT_OFFICE_SETTINGS"));
        assert!(!keys.contains(&"AGENT_OFFICE_HOOK_URL"));
        assert!(!keys.contains(&"ZDOTDIR"));

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_autostart_with_hooks_disabled_injects_plain_claude() {
        let (mgr, _events, ctl, dir) = build_with_port(None);
        mgr.create(req("a1", Some(true))).unwrap();

        assert_eq!(ctl.writes_utf8(), "claude\n", "hooks-OFF autostart must inject a bare `claude` with no --settings");

        cleanup(&ctl, &dir);
    }

    // ---- Task B: zsh ZDOTDIR shim wiring ----

    /// Like `build()`, but with an overridden `shell_resolver` so the test
    /// doesn't depend on the host's actual `$SHELL`.
    fn build_with_shell_resolver(
        resolver: Arc<dyn Fn(shells::ShellRequest) -> shells::ResolvedShell + Send + Sync>,
    ) -> (Arc<SessionManager>, Arc<RecordingEvents>, Arc<FakeControl>, PathBuf) {
        let events = Arc::new(RecordingEvents::default());
        let reg = registry();
        let hub = hub_for(reg.clone(), events.clone() as Arc<dyn AppEvents>);
        let (writer, dir) = scratch_hook_writer();
        let (fac, ctl) = FakePtyFactory::new();
        let mgr = Arc::new(
            SessionManager::new(
                Arc::new(fac),
                writer,
                reg,
                events.clone() as Arc<dyn AppEvents>,
                hub,
                Arc::new(|| Some(12345u16)),
            )
            .with_shell_resolver(resolver),
        );
        (mgr, events, ctl, dir)
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn create_pushes_zdotdir_env_when_shell_resolver_returns_zsh() {
        let (mgr, _events, ctl, dir) = build_with_shell_resolver(Arc::new(|_req: shells::ShellRequest| {
            shells::ResolvedShell {
                program: "/bin/zsh".to_string(),
                args: vec!["-l".to_string(), "-i".to_string()],
                extra_env: vec![],
            }
        }));
        mgr.create(req("a1", None)).unwrap();

        let env = ctl.spawned_env();
        let zdotdir = env
            .iter()
            .find(|(k, _)| k == "ZDOTDIR")
            .map(|(_, v)| v.clone())
            .expect("ZDOTDIR must be present in spawn env for a zsh session");
        assert!(
            PathBuf::from(&zdotdir).join(".zshrc").is_file(),
            "ZDOTDIR must point at a directory containing the written .zshrc shim: {zdotdir}"
        );

        cleanup(&ctl, &dir);
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn create_does_not_push_zdotdir_env_for_non_zsh_shells() {
        let (mgr, _events, ctl, dir) = build_with_shell_resolver(Arc::new(|_req: shells::ShellRequest| {
            shells::ResolvedShell {
                program: "/bin/bash".to_string(),
                args: vec!["-l".to_string(), "-i".to_string()],
                extra_env: vec![],
            }
        }));
        mgr.create(req("a1", None)).unwrap();

        let env = ctl.spawned_env();
        assert!(
            !env.iter().any(|(k, _)| k == "ZDOTDIR"),
            "ZDOTDIR must not be set for a non-zsh shell: {env:?}"
        );

        cleanup(&ctl, &dir);
    }

    // ---- cwd: leading `~` expansion ----

    #[tokio::test]
    async fn create_expands_leading_tilde_slash_in_cwd() {
        let (mgr, _events, ctl, dir) = build();
        mgr.create(req_with_cwd("a1", Some("~/some/dir".into()))).unwrap();

        assert_eq!(ctl.spawned_cwd(), format!("{}/some/dir", home_dir()));

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_expands_bare_tilde_in_cwd() {
        let (mgr, _events, ctl, dir) = build();
        mgr.create(req_with_cwd("a1", Some("~".into()))).unwrap();

        assert_eq!(ctl.spawned_cwd(), home_dir());

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_does_not_expand_tilde_user_form() {
        // `~someuser/dir` is left untouched -- only bare `~` and `~/...` expand.
        let (mgr, _events, ctl, dir) = build();
        mgr.create(req_with_cwd("a1", Some("~someuser/dir".into()))).unwrap();

        assert_eq!(ctl.spawned_cwd(), "~someuser/dir");

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_passes_through_absolute_cwd_unchanged() {
        let (mgr, _events, ctl, dir) = build();
        mgr.create(req_with_cwd("a1", Some("/abs/path".into()))).unwrap();

        assert_eq!(ctl.spawned_cwd(), "/abs/path");

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_defaults_cwd_to_home_dir_when_omitted() {
        let (mgr, _events, ctl, dir) = build();
        mgr.create(req_with_cwd("a1", None)).unwrap();

        assert_eq!(ctl.spawned_cwd(), home_dir());

        cleanup(&ctl, &dir);
    }

    // ---- same agentId reuse ----

    #[tokio::test]
    async fn create_reuses_existing_session_for_same_agent_id_while_alive() {
        let (mgr, events, ctl, dir) = build();
        let first = mgr.create(req("a1", Some(false))).unwrap();
        // A 2nd real spawn would panic (FakePtyFactory allows exactly one
        // spawn), so a successful reuse call here proves no new PTY was made.
        let second = mgr.create(req("a1", Some(false))).unwrap();

        assert_eq!(first.session_id, second.session_id);
        assert_eq!(second.state, SessionState::Running);
        assert_eq!(
            events.states(),
            vec![SessionState::Starting, SessionState::Running],
            "reuse must not re-run the Starting/Running pipeline"
        );

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_spawns_a_new_session_for_the_same_agent_id_after_disposal() {
        // A disposed session must NOT be reused (only Running/Starting are)
        // -- but we can't spawn a 2nd real PTY on the same single-spawn fake,
        // so this asserts the negative space via the removal side: once
        // Disposed, the manager's own bookkeeping no longer considers "a1"
        // alive, which is exactly the condition `create`'s reuse check relies
        // on to decide whether to reuse.
        let (mgr, events, ctl, dir) = build();
        let created = mgr.create(req("a1", Some(false))).unwrap();
        mgr.dispose("a1");
        ctl.fire_exit(0);
        wait_for(|| events.states().len() == 3).await;

        assert_eq!(mgr.session_id_for("a1"), None, "disposed agent must not resolve to a session");
        let _ = created;

        cleanup(&ctl, &dir);
    }

    // ---- dispose -> Disposed, bookkeeping removed ----

    #[tokio::test]
    async fn dispose_kills_pty_and_on_exit_transitions_to_disposed_and_removes_bookkeeping() {
        let (mgr, events, ctl, dir) = build();
        let created = mgr.create(req("a1", Some(false))).unwrap();

        mgr.dispose("a1");
        assert_eq!(ctl.kill_count(), 1, "dispose must call PtyControl::kill");

        ctl.fire_exit(0);
        wait_for(|| events.states().len() == 3).await;

        let last = events.last_state();
        assert_eq!(last.state, SessionState::Disposed);
        assert!(last.exit.as_ref().unwrap().intentional, "kill-triggered exit must be intentional");

        assert_eq!(mgr.session_id_for("a1"), None, "agentId must be removed from the sessions map");
        assert_eq!(
            mgr.registry.resolve_agent(&created.session_id),
            None,
            "Disposed session must be removed from the registry (E8: later hooks are discarded)"
        );

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn dispose_all_kills_every_live_session() {
        let (mgr, _events, ctl, dir) = build();
        mgr.create(req("a1", Some(false))).unwrap();

        mgr.dispose_all();

        assert_eq!(ctl.kill_count(), 1);
        ctl.fire_exit(0);
        cleanup(&ctl, &dir);
    }

    // ---- write/resize: Running guard ----

    #[tokio::test]
    async fn write_input_and_resize_apply_while_running() {
        let (mgr, _events, ctl, dir) = build();
        mgr.create(req("a1", Some(false))).unwrap();

        mgr.write_input("a1", "echo hi\n");
        mgr.resize("a1", 120, 40);

        assert_eq!(ctl.writes_utf8(), "echo hi\n");
        assert_eq!(ctl.resize_calls(), vec![(120, 40)]);

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn write_input_and_resize_are_noop_once_session_has_exited() {
        let (mgr, events, ctl, dir) = build();
        mgr.create(req("a1", Some(false))).unwrap();

        ctl.fire_exit(2);
        wait_for(|| events.states().len() == 3).await;

        mgr.write_input("a1", "should not appear");
        mgr.resize("a1", 10, 10);

        assert_eq!(ctl.writes_utf8(), "", "write after exit must be a no-op");
        assert!(ctl.resize_calls().is_empty(), "resize after exit must be a no-op");

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn write_input_and_resize_on_unknown_agent_do_not_panic() {
        let (mgr, _events, ctl, dir) = build();
        mgr.write_input("ghost", "x");
        mgr.resize("ghost", 1, 1);
        cleanup(&ctl, &dir);
    }

    // ---- agentId-keyed output sinks (pending attach + recreate reuse) ----

    /// A `tauri::ipc::Channel<OutputChunk>` that accumulates every emitted
    /// `data` string into a shared buffer (no Tauri runtime needed — `Channel`
    /// just wraps a callback).
    fn recording_channel() -> (Channel<OutputChunk>, Arc<Mutex<String>>) {
        let sink = Arc::new(Mutex::new(String::new()));
        let sink_for_cb = sink.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Json(s) = body {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                    if let Some(data) = v.get("data").and_then(|d| d.as_str()) {
                        sink_for_cb.lock().unwrap().push_str(data);
                    }
                }
            }
            Ok(())
        });
        (channel, sink)
    }

    #[tokio::test]
    async fn attach_before_create_delivers_output_once_the_session_starts() {
        // A channel attached BEFORE any session exists (pending attach) must
        // be honored by the session create() later binds to that agentId.
        let (mgr, _events, ctl, dir) = build();
        let (channel, captured) = recording_channel();

        // No session yet for "a1" — attach creates a pending sink.
        assert_eq!(mgr.session_id_for("a1"), None);
        mgr.attach_output("a1", channel);

        mgr.create(req("a1", Some(false))).unwrap();
        ctl.push_output(b"hello-pending");

        wait_for(|| captured.lock().unwrap().contains("hello-pending")).await;

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn recreating_a_session_for_the_same_agent_reuses_the_attached_channel() {
        // Multi-spawn fake: the same agentId spawns two PTYs over its life.
        // The channel is attached once; after the first session Exits and a
        // new one is created, output must still flow to that same channel with
        // NO re-subscribe from the renderer.
        let events = Arc::new(RecordingEvents::default());
        let reg = registry();
        let hub = hub_for(reg.clone(), events.clone() as Arc<dyn AppEvents>);
        let (writer, dir) = scratch_hook_writer();
        let factory = Arc::new(MultiFakePtyFactory::new());
        let mgr = Arc::new(SessionManager::new(
            factory.clone(),
            writer,
            reg,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| Some(12345u16)),
        ));

        let (channel, captured) = recording_channel();
        mgr.attach_output("a1", channel); // subscribe once, before anything

        // First session.
        mgr.create(req("a1", Some(false))).unwrap();
        let ctl1 = factory.controls()[0].clone();
        ctl1.push_output(b"from-first;");
        wait_for(|| captured.lock().unwrap().contains("from-first;")).await;

        // Unexpected exit -> Exited (session kept for restart).
        ctl1.fire_exit(1);
        wait_for(|| events.states().contains(&SessionState::Exited)).await;
        ctl1.close_output(); // let the first pump wind down

        // Recreate for the same agentId (a genuine 2nd spawn).
        mgr.create(req("a1", Some(false))).unwrap();
        let ctl2 = factory.controls()[1].clone();
        ctl2.push_output(b"from-second");

        // Same channel receives the new session's output — no re-attach.
        wait_for(|| captured.lock().unwrap().contains("from-second")).await;

        ctl2.close_output();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- create() Running transition is a compare-and-set ----

    /// AppEvents wrapper that deterministically simulates the resurrection
    /// race: the instant create() emits `Starting` (synchronously, on create's
    /// own thread, right before the post-spawn transition), it flips the
    /// session's state to `Exited` — exactly as if the wait thread's on_exit
    /// had already won. create()'s transition must then see "not Starting" and
    /// skip the Running write (CAS). Without the fix it unconditionally sets
    /// Running, resurrecting the dead session.
    struct ExitDuringStarting {
        inner: Arc<RecordingEvents>,
        mgr: std::sync::OnceLock<std::sync::Weak<SessionManager>>,
        fired: AtomicBool,
    }
    impl AppEvents for ExitDuringStarting {
        fn session_state(&self, ev: &SessionStateEvent) {
            self.inner.session_state(ev);
            if ev.state == SessionState::Starting && !self.fired.swap(true, Ordering::SeqCst) {
                if let Some(mgr) = self.mgr.get().and_then(|w| w.upgrade()) {
                    if let Some(s) = mgr.find(&ev.agent_id) {
                        *s.state.lock().unwrap() = SessionState::Exited;
                    }
                }
            }
        }
        fn notification_new(&self, ev: &NotificationEvent) {
            self.inner.notification_new(ev);
        }
        fn notification_cleared(&self, agent_id: &str, ids: &[String]) {
            self.inner.notification_cleared(agent_id, ids);
        }
        fn activity_event(&self, ev: &ActivityEvent) {
            self.inner.activity_event(ev);
        }
    }

    #[tokio::test]
    async fn running_transition_does_not_overwrite_a_session_already_exited() {
        let inner = Arc::new(RecordingEvents::default());
        let events = Arc::new(ExitDuringStarting {
            inner: inner.clone(),
            mgr: std::sync::OnceLock::new(),
            fired: AtomicBool::new(false),
        });
        let reg = registry();
        let hub = hub_for(reg.clone(), events.clone() as Arc<dyn AppEvents>);
        let (writer, dir) = scratch_hook_writer();
        let (fac, ctl) = FakePtyFactory::new();
        let mgr = Arc::new(SessionManager::new(
            Arc::new(fac),
            writer,
            reg,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| Some(12345u16)),
        ));
        events.mgr.set(Arc::downgrade(&mgr)).ok();

        // During the Starting emit, `events` flips the session to Exited; the
        // CAS transition must then skip Running.
        let created = mgr.create(req("a1", Some(false))).unwrap();

        assert_eq!(
            created.state,
            SessionState::Exited,
            "create() must not resurrect a session that exited during Starting"
        );
        assert_eq!(
            mgr.find("a1").map(|s| *s.state.lock().unwrap()),
            Some(SessionState::Exited),
            "session state must stay Exited, never overwritten to Running"
        );
        // No Running was ever emitted (the transition was skipped).
        assert!(
            !inner.states().contains(&SessionState::Running),
            "Running must never be emitted after the session already Exited: {:?}",
            inner.states()
        );

        cleanup(&ctl, &dir);
    }

    // ---- settings-file cleanup on unexpected exit & spawn failure ----

    #[tokio::test]
    async fn unexpected_exit_cleans_up_the_settings_file() {
        let (mgr, events, ctl, dir) = build();
        let created = mgr.create(req("a1", Some(false))).unwrap();
        let settings = dir.join(format!("{}.settings.json", created.session_id));
        assert!(settings.exists(), "settings file should exist while running");

        ctl.fire_exit(1); // unexpected -> Exited
        wait_for(|| events.states().contains(&SessionState::Exited)).await;
        wait_for(|| !settings.exists()).await;

        assert!(!settings.exists(), "unexpected exit must clean up the settings file");
        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn spawn_failure_cleans_up_the_settings_file_it_pre_wrote() {
        let events = Arc::new(RecordingEvents::default());
        let reg = registry();
        let hub = hub_for(reg.clone(), events.clone() as Arc<dyn AppEvents>);
        let (writer, dir) = scratch_hook_writer();
        let mgr = Arc::new(SessionManager::new(
            Arc::new(AlwaysFailPtyFactory),
            writer,
            reg,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| Some(12345u16)),
        ));

        let result = mgr.create(req("a1", Some(false)));
        assert!(result.is_err(), "spawn must fail with AlwaysFailPtyFactory");

        // The --settings file write() happens before spawn(); on spawn failure
        // it must be cleaned up, leaving no leftover in the hook dir.
        let leftovers = std::fs::read_dir(&dir).map(|rd| rd.count()).unwrap_or(0);
        assert_eq!(leftovers, 0, "spawn failure must not leak the pre-written settings file");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- shell selection: resolver receives selected id + hooks_on, extra_env is spliced into spawn env ----

    /// What a recording resolver captured from its one `ShellRequest` call.
    struct RecordedShellRequest {
        selected: Option<String>,
        hooks_on: bool,
    }

    /// Builds a `shell_resolver` that copies `req.selected`/`req.hooks_on`
    /// into `captured` (owned, so it outlives the borrowed `ShellRequest`)
    /// and always resolves to a fixed, harmless `ResolvedShell` carrying
    /// `extra_env` so both concerns (request plumbing + env splicing) can be
    /// asserted from the same fixture.
    fn recording_resolver(
        captured: Arc<Mutex<Option<RecordedShellRequest>>>,
        extra_env: Vec<(String, String)>,
    ) -> Arc<dyn Fn(shells::ShellRequest) -> shells::ResolvedShell + Send + Sync> {
        Arc::new(move |req: shells::ShellRequest| {
            *captured.lock().unwrap() =
                Some(RecordedShellRequest { selected: req.selected.map(|s| s.to_string()), hooks_on: req.hooks_on });
            shells::ResolvedShell {
                program: "/bin/sh".to_string(),
                args: vec![],
                extra_env: extra_env.clone(),
            }
        })
    }

    /// Like `build_with_shell_resolver`, but lets the caller also choose the
    /// hook port (so hooks-on/hooks-off variants can share one fixture).
    fn build_with_shell_resolver_and_port(
        resolver: Arc<dyn Fn(shells::ShellRequest) -> shells::ResolvedShell + Send + Sync>,
        port: Option<u16>,
    ) -> (Arc<SessionManager>, Arc<RecordingEvents>, Arc<FakeControl>, PathBuf) {
        let events = Arc::new(RecordingEvents::default());
        let reg = registry();
        let hub = hub_for(reg.clone(), events.clone() as Arc<dyn AppEvents>);
        let (writer, dir) = scratch_hook_writer();
        let (fac, ctl) = FakePtyFactory::new();
        let mgr = Arc::new(
            SessionManager::new(
                Arc::new(fac),
                writer,
                reg,
                events.clone() as Arc<dyn AppEvents>,
                hub,
                Arc::new(move || port),
            )
            .with_shell_resolver(resolver),
        );
        (mgr, events, ctl, dir)
    }

    #[tokio::test]
    async fn create_passes_selected_shell_and_hooks_on_true_to_resolver() {
        let captured = Arc::new(Mutex::new(None));
        let resolver = recording_resolver(captured.clone(), vec![]);
        // build_with_shell_resolver defaults get_hook_port to Some(12345) -> hooks ON.
        let (mgr, _events, ctl, dir) = build_with_shell_resolver(resolver);

        mgr.create(req_with_shell("a1", Some("git-bash".to_string()))).unwrap();

        let rec = captured.lock().unwrap();
        let rec = rec.as_ref().expect("resolver must have been called");
        assert_eq!(rec.selected.as_deref(), Some("git-bash"));
        assert!(rec.hooks_on, "hooks were enabled for this session -> hooks_on must be true");

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_passes_hooks_on_false_to_resolver_when_hooks_disabled() {
        let captured = Arc::new(Mutex::new(None));
        let resolver = recording_resolver(captured.clone(), vec![]);
        // get_hook_port -> None mirrors the hooks-opt-in-OFF path exercised by
        // build_with_port(None) elsewhere in this file.
        let (mgr, _events, ctl, dir) = build_with_shell_resolver_and_port(resolver, None);

        mgr.create(req_with_shell("a1", Some("git-bash".to_string()))).unwrap();

        let rec = captured.lock().unwrap();
        let rec = rec.as_ref().expect("resolver must have been called");
        assert_eq!(rec.selected.as_deref(), Some("git-bash"));
        assert!(!rec.hooks_on, "hooks are OFF for this session -> hooks_on must be false");

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_appends_resolved_extra_env_to_spawn_env() {
        let captured = Arc::new(Mutex::new(None));
        let marker = ("AGENT_OFFICE_TEST_MARKER".to_string(), "shell-extra-env".to_string());
        let resolver = recording_resolver(captured, vec![marker.clone()]);
        let (mgr, _events, ctl, dir) = build_with_shell_resolver(resolver);

        mgr.create(req("a1", None)).unwrap();

        let env = ctl.spawned_env();
        assert!(
            env.contains(&marker),
            "resolved.extra_env pair must be appended to the spawned env: {env:?}"
        );

        cleanup(&ctl, &dir);
    }
}

// ---------------------------------------------------------------------
// Phase 2 sign-off smoke: REAL PTY, end-to-end through the exact same
// SessionManager wiring `lib.rs::run()` builds (only Tauri-runtime-bound
// pieces -- AppEvents/hook server/app handle -- are swapped for local
// doubles; PortablePtyFactory + SessionManager + HookSettingsWriter are the
// real production types). Deliberately `#[ignore]`d: shell startup time and
// `$SHELL` quirks make this env-dependent and too slow/flaky for the default
// `cargo test` run. Run explicitly:
//   cargo test --manifest-path src-tauri/Cargo.toml -- --ignored real_shell
//
// This lives inside `manager.rs` (rather than `src-tauri/tests/`) because
// `mod session`/`mod state`/`mod notification` are private in `lib.rs` --
// an external integration test crate can't name `SessionManager`,
// `HookSettingsWriter`, or `state::fake::RecordingEvents` at all. Widening
// those to `pub`/`pub(crate)` just for this one smoke would be a bigger
// surface change than necessary, so the smoke rides along as a sibling
// `#[cfg(test)]` module instead, reusing the same private items the
// `tests` module above already does via `use super::*`.
#[cfg(test)]
mod real_pty_smoke {
    use super::*;
    use crate::notification::hub::{NotificationHub, SystemClock};
    use crate::session::pty_factory::PortablePtyFactory;
    use crate::state::fake::RecordingEvents;
    use std::time::Duration;
    use tauri::ipc::{Channel, InvokeResponseBody};

    /// Poll `pred` until true, panicking with `msg` after `timeout` instead
    /// of hanging forever if the real shell never produces the expected
    /// bytes (misconfigured `$SHELL`, a hung profile script, etc).
    async fn wait_for_timeout<F: Fn() -> bool>(pred: F, timeout: Duration, msg: &str) {
        let deadline = tokio::time::Instant::now() + timeout;
        while !pred() {
            assert!(tokio::time::Instant::now() < deadline, "{msg}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn scratch_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("agent-office-smoke-{label}-{}", Uuid::new_v4()))
    }

    #[tokio::test]
    #[ignore = "real PTY; run explicitly"]
    async fn real_shell_output_flows_end_to_end_and_disposes_cleanly() {
        let events = Arc::new(RecordingEvents::default());
        let registry = Arc::new(SessionRegistry::new());
        let hub = Arc::new(NotificationHub::new(
            registry.clone(),
            events.clone() as Arc<dyn AppEvents>,
            Arc::new(SystemClock),
            Duration::from_millis(3000),
        ));

        let hook_dir = scratch_dir("hooks");
        let hook_writer = HookSettingsWriter::new(hook_dir.clone());

        let cwd_dir = scratch_dir("cwd");
        std::fs::create_dir_all(&cwd_dir).expect("create scratch cwd");

        let mgr = Arc::new(SessionManager::new(
            Arc::new(PortablePtyFactory),
            hook_writer,
            registry,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| Some(45999u16)), // fake hook port; no real hook server needed for this smoke
        ));

        let created = mgr
            .create(CreateSessionRequest {
                agent_id: "smoke".into(),
                cols: Some(80),
                rows: Some(24),
                cwd: Some(cwd_dir.to_string_lossy().into_owned()),
                shell: None,
                autostart_claude: Some(false),
            })
            .expect("real PTY spawn should succeed");
        assert_eq!(created.state, SessionState::Running);

        // Collect OutputChunk.data via a real tauri::ipc::Channel (no Tauri
        // runtime/webview needed -- Channel::new() just wraps a callback).
        let output = Arc::new(Mutex::new(String::new()));
        let output_for_channel = output.clone();
        let channel: Channel<OutputChunk> = Channel::new(move |body| {
            if let InvokeResponseBody::Json(s) = body {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                    if let Some(data) = v.get("data").and_then(|d| d.as_str()) {
                        output_for_channel.lock().unwrap().push_str(data);
                    }
                }
            }
            Ok(())
        });
        mgr.attach_output("smoke", channel);

        // 1) Real shell prompt bytes must arrive within 5s, and state must
        //    have gone Starting -> Running.
        wait_for_timeout(
            || !output.lock().unwrap().is_empty(),
            Duration::from_secs(5),
            "no output arrived from the real shell within 5s -- check $SHELL / login-shell startup time",
        )
        .await;
        assert_eq!(events.states().first().copied(), Some(SessionState::Starting));
        assert!(events.states().contains(&SessionState::Running));

        // 2) Echo round-trip through real stdin -> shell -> stdout.
        mgr.write_input("smoke", "echo smoke-ok-12345\n");
        wait_for_timeout(
            || output.lock().unwrap().contains("smoke-ok-12345"),
            Duration::from_secs(5),
            "echoed marker 'smoke-ok-12345' never appeared in PTY output within 5s",
        )
        .await;

        // 3) Dispose -> real process killed -> Disposed(intentional=true).
        mgr.dispose("smoke");
        wait_for_timeout(
            || matches!(events.states().last(), Some(SessionState::Disposed)),
            Duration::from_secs(5),
            "session never reached Disposed within 5s after dispose()",
        )
        .await;
        let last = events.last_state();
        assert_eq!(last.state, SessionState::Disposed);
        assert!(
            last.exit.as_ref().unwrap().intentional,
            "dispose()-triggered exit must be reported intentional=true"
        );

        let _ = std::fs::remove_dir_all(&hook_dir);
        let _ = std::fs::remove_dir_all(&cwd_dir);
    }
}
