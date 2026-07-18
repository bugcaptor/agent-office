// src-tauri/src/session/manager.rs
//
// SessionManager: owns the PTY session lifecycle (reader thread / tokio
// output pump / wait thread), autostart stdin injection, and state
// transitions.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// std::sync::Mutex가 아니라 parking_lot::Mutex — poisoning이 없다. 실사고
// (2026-07-11): 출력 채널 콜백 패닉 → channel 뮤텍스 poison → detach가 sinks
// 락 보유 중 unwrap 패닉 → sinks poison → 이후 모든 create()가 sink_for에서
// 패닉(훅 파일만 남기고 invoke 영구 미해결) → 앱 재시작까지 어떤 터미널도
// 못 뜨는 벽돌 상태. parking_lot은 패닉한 스레드가 락을 풀고 지나가므로
// 오염이 전파되지 않는다. (session_layer_survives_a_panicking_output_channel
// 회귀 테스트 참조.)
use parking_lot::Mutex;

use tauri::ipc::Channel;
use uuid::Uuid;

use crate::notification::hub::NotificationHub;
use crate::observer::{CommandWrapperSpec, ObserverRuntime, ObserverSessionContext, WrapperArg};
use crate::session::output_batcher::{FlushSink, OutputBatcher, MAX_BYTES, WINDOW_MS};
use crate::session::pi_extension;
use crate::session::pty_factory::{ExitOutcome, PtyControl, PtyFactory, PtySpawnOptions, SpawnedPty};
use crate::session::shells;
use crate::session_events::types::{AgentEventProfile, SessionStartedEvent};
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
        Self {
            channel: Mutex::new(None),
            backlog: Mutex::new(Default::default()),
        }
    }
    fn attach(&self, ch: Channel<OutputChunk>) {
        // 락 순서 항상 channel → backlog (데드락 방지, emit과 동일 순서).
        let mut c = self.channel.lock();
        let mut b = self.backlog.lock();
        for chunk in b.drain(..) {
            let _ = ch.send(chunk);
        }
        *c = Some(ch);
    }
    fn detach(&self) {
        *self.channel.lock() = None;
    }
    /// 핸드오프 스냅샷 폴백(실증에서 발견된 빈틈): 프론트가 이 터미널을
    /// 한 번도 구독하지 않은 채 종료하면 xterm 쪽 직렬화 스냅샷이 없다 --
    /// 그 세션의 종료 전 출력은 여기 backlog에만 남아 있으므로, 원시
    /// 바이트를 이어붙여 스냅샷 대용으로 쓴다. **드레인하지 않고 복사만
    /// 한다** -- 핸드오프가 실패해도(데몬 연결 불가 등) 이 세션은 맵에
    /// 그대로 남아 출력이 이어져야 하므로 backlog를 비우면 안 된다.
    fn backlog_snapshot(&self) -> Vec<u8> {
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

struct Session {
    session_id: SessionId,
    agent_id: AgentId,
    state: Mutex<SessionState>,
    writer: Mutex<Box<dyn Write + Send>>,
    control: Arc<dyn PtyControl>,
    cleanup_paths: Vec<std::path::PathBuf>,
    kill_requested: AtomicBool,
    /// 시작 작업 디렉터리(세션 수명 동안 불변 -- `cd`는 추적하지 않는다).
    /// 핸드오프 시 Handoff 메시지의 진단/List용 메타데이터로 실어 보낸다.
    cwd: String,
    /// 현재 알려진 터미널 크기. resize()가 갱신 -- 핸드오프 시 Handoff
    /// 메시지에 실어 데몬에 보내고, 입양 응답의 AdoptedSessionInfo로
    /// 프론트에 되돌려줘 터미널 크기를 맞추는 데 쓴다.
    size: Mutex<(u16, u16)>, // (cols, rows)
    /// 세션 핸드오프(§핵심 3, 4). true면 on_exit/dispose가 즉시 return —
    /// 이 세션의 실제 수명은 sessiond가 넘겨받았다(또는 넘겨받는 중이다).
    /// 필드 자체는 크로스플랫폼으로 둬 cfg 분기를 최소화한다(Windows/Fake는
    /// 항상 false로 남는 no-op).
    handed_off: AtomicBool,
    /// unix 전용: 핸드오프 시 리더 스레드를 확정적으로 멈추는 스위치와,
    /// sessiond에 넘길 마스터 fd/pid/pgid. `create_with_profile`(팩토리
    /// spawn)과 `adopt_detached`(assemble_adopted) 양쪽이 채운다 — Fake로
    /// 만든 세션은 항상 None(핸드오프 불가능 세션).
    #[cfg(unix)]
    reader_interrupt: Mutex<Option<crate::session::poll_reader::ReaderInterrupt>>,
    #[cfg(unix)]
    handoff: Mutex<Option<crate::session::pty_factory::HandoffInfo>>,
    /// 입양된 세션 한정(§핵심 4의 AdoptedReader 정지 게이트) -- 재핸드오프
    /// 인터럽트 직전 true로 세팅해야 EofWaiter가 오발화하지 않는다.
    /// create() 경로(RealWaiter가 독립적으로 exit 판정)는 항상 None. 타입
    /// 자체는 크로스플랫폼이라 cfg 분기가 필요 없다(항상 컴파일되고, 비unix는
    /// 그냥 항상 None으로 남는다).
    eof_stop_gate: Option<Arc<AtomicBool>>,
    /// v2 브로커 데몬이 이 세션을 소유하는가(SpawnedPty에서 전파). 브로커 모드
    /// 매니저라도 팩토리 폴백으로 생긴 in-process 세션은 false다 — handoff_all이
    /// 이 플래그로 "스냅샷 업로드+detach"(true)와 "v1 fd 핸드오프"(false)를 가른다.
    broker_owned: bool,
    /// 브로커 data 연결의 누적 수신 오프셋 카운터(§P1). 스냅샷 업로드 시 현재
    /// 값을 offset으로 동봉해 데몬 수신 시점 ring.total()과의 간극(유실 창)을
    /// 없앤다. 브로커 세션만 Some.
    broker_stream_offset: Option<Arc<std::sync::atomic::AtomicU64>>,
    /// detach 시 data 소켓을 결정적으로 닫는 핸들(§#50 선결). broker_owned 세션의
    /// detach에서 shutdown하면 앱 reader 스레드가 EOF로 종료되고 데몬 conn이 정리돼
    /// List `attached`가 false로 돌아간다 — adopt가 라이브 원격 소유 세션을
    /// 가로채지 않게 하는 근거(현재 인스턴스가 살아 붙어 있으면 attached=true 유지).
    /// 브로커 세션만 Some(unix).
    #[cfg(unix)]
    broker_data_shutdown: Mutex<Option<crate::session::broker_pty::BrokerDataShutdown>>,
}

pub struct SessionManager {
    factory: Arc<dyn PtyFactory>,
    observer: Arc<ObserverRuntime>,
    get_observer_url: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    registry: Arc<SessionRegistry>,
    events: Arc<dyn AppEvents>,
    hub: Arc<NotificationHub>,
    sessions: Mutex<HashMap<AgentId, Arc<Session>>>,
    /// agentId별 출력 sink — 세션 수명과 독립. subscribe 이전 pending attach와
    /// 세션 재생성 시 채널 재사용을 위해 세션이 아니라 여기에 보관한다.
    sinks: Mutex<HashMap<AgentId, Arc<OutputSink>>>,
    shell_resolver:
        Arc<dyn Fn(Option<&str>, &[CommandWrapperSpec]) -> shells::ResolvedShell + Send + Sync>,
    /// 세션 핸드오프(unix 전용)와 `AGENT_OFFICE_APP_DATA` env 주입(§핵심 5)에
    /// 쓰는 앱 데이터 디렉터리. 프로덕션은 `lib.rs`가 `with_app_data_dir`로
    /// 채운다 — 미설정(None)이면 `handoff_all`/`adopt_detached`는 no-op(0/빈
    /// 벡터)이고 env 주입도 생략된다(기존 테스트가 앱 데이터 경로 없이도
    /// 그대로 통과해야 하므로 기본값은 None).
    app_data_dir: Option<std::path::PathBuf>,
    /// v2 상시 브로커 모드(unix 전용 opt-in, docs/session-broker-v2-design.md).
    /// true면 `handoff_all`/`adopt_detached`/스냅샷 업로드가 v1 fd 핸드오프
    /// 대신 브로커 RPC 경로를 탄다. 기본 false(v1 경로 보존). 팩토리 주입은
    /// `lib.rs`가 별도로 하고, 이 플래그는 앱 쪽 의미 분기에만 관여한다.
    broker_mode: bool,
}

impl SessionManager {
    pub fn new(
        factory: Arc<dyn PtyFactory>,
        observer: Arc<ObserverRuntime>,
        registry: Arc<SessionRegistry>,
        events: Arc<dyn AppEvents>,
        hub: Arc<NotificationHub>,
        get_observer_url: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    ) -> Self {
        Self {
            factory,
            observer,
            get_observer_url,
            registry,
            events,
            hub,
            sessions: Mutex::new(HashMap::new()),
            sinks: Mutex::new(HashMap::new()),
            shell_resolver: Arc::new(shells::resolve_observed),
            app_data_dir: None,
            broker_mode: false,
        }
    }

    /// v2 상시 브로커 모드를 켠다(unix opt-in). `lib.rs`가 플래그+unix일 때만
    /// 호출하고, 같은 조건에서 팩토리도 `BrokerPtyFactory`로 주입한다.
    pub fn with_broker_mode(mut self, on: bool) -> Self {
        self.broker_mode = on;
        self
    }

    /// 렌더러가 주기 스냅샷 업로드/모달 분기를 켤지 판단하는 데 쓰는 조회.
    pub fn broker_mode(&self) -> bool {
        self.broker_mode
    }

    /// 앱 데이터 디렉터리를 지정한다(세션 핸드오프 소켓/로그 경로,
    /// `AGENT_OFFICE_APP_DATA` env의 근거). `lib.rs`의 프로덕션 부트스트랩만
    /// 호출 — 테스트는 기본 None으로 이 기능을 건드리지 않는다.
    pub fn with_app_data_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.app_data_dir = Some(dir);
        self
    }

    fn find(&self, agent_id: &str) -> Option<Arc<Session>> {
        self.sessions.lock().get(agent_id).cloned()
    }

    /// agentId의 출력 sink를 반환(없으면 생성). attach_output이 세션보다 먼저
    /// 호출되면 여기서 sink가 만들어지고, create()는 같은 sink를 이어받아
    /// 이미 붙은 채널/백로그를 그대로 재사용한다.
    fn sink_for(&self, agent_id: &str) -> Arc<OutputSink> {
        self.sinks
            .lock()
            .entry(agent_id.to_string())
            .or_insert_with(|| Arc::new(OutputSink::new()))
            .clone()
    }

    pub fn session_id_for(&self, agent_id: &str) -> Option<SessionId> {
        self.find(agent_id).map(|s| s.session_id.clone())
    }

    pub fn create(
        self: &Arc<Self>,
        req: CreateSessionRequest,
    ) -> Result<CreateSessionResult, String> {
        let fallback = AgentEventProfile {
            name: req.agent_id.clone(),
            role: None,
        };
        self.create_with_profile(req, fallback)
    }

    /// 1 에이전트 1 세션 불변식. self: &Arc<Self>로 wait 스레드에 소유 이전.
    pub fn create_with_profile(
        self: &Arc<Self>,
        req: CreateSessionRequest,
        profile: AgentEventProfile,
    ) -> Result<CreateSessionResult, String> {
        // 살아있는 세션이 있으면 재사용, 새 PTY 안 만듦. 단, dispose()로 kill이
        // 요청된(=재시작 중인) 세션은 곧 사라질 예정이므로 재사용하지 않는다 —
        // 그러지 않으면 PowerShell처럼 프로세스 reap(→ on_exit)이 느린 플랫폼에서
        // 아직 Running으로 남은 "죽어가는 세션"을 재사용해 첫 재시작이 헛돌았다.
        //
        // 재사용하지 않을 세션은 이 임계구역 안에서 맵 슬롯을 즉시 비운다. 그래야
        // 뒤늦게 도는 그 세션의 on_exit이 "이미 교체됨(superseded)"을 보고 새
        // 세션의 맵 엔트리·sink를 지우지 않는다(아래 on_exit의 identity 가드 참조).
        {
            let mut map = self.sessions.lock();
            if let Some(s) = map.get(&req.agent_id) {
                let st = *s.state.lock();
                let reusable = matches!(st, SessionState::Running | SessionState::Starting)
                    && !s.kill_requested.load(Ordering::SeqCst);
                if reusable {
                    return Ok(CreateSessionResult {
                        session_id: s.session_id.clone(),
                        state: st,
                    });
                }
                map.remove(&req.agent_id);
            }
        }

        let session_id = Uuid::new_v4().to_string(); // uuid는 URL-safe → hook 라우팅 키로 안전
        let observer_url = (self.get_observer_url)();
        let mut plan = observer_url
            .as_deref()
            .map(|url| {
                self.observer
                    .prepare_session(&ObserverSessionContext::new(&session_id, url))
            })
            .unwrap_or_default();
        if observer_url.is_some() {
            match pi_extension::ensure_extension() {
                Ok(path) => {
                    plan.env.push((
                        "AGENT_OFFICE_PI_EXT".into(),
                        path.to_string_lossy().into_owned(),
                    ));
                    plan.wrappers.push(CommandWrapperSpec {
                        command: "pi".into(),
                        prefix_args: vec![
                            WrapperArg::Literal("-e".into()),
                            WrapperArg::Env("AGENT_OFFICE_PI_EXT".into()),
                        ],
                        skip_if_present: vec![],
                    });
                }
                Err(error) => eprintln!("agent-office: failed to write pi extension: {error}"),
            }
        }

        if let Some(personality_prompt) = req
            .personality_prompt
            .as_deref()
            .filter(|prompt| !prompt.trim().is_empty())
        {
            plan.env.push((
                "AGENT_OFFICE_PERSONA".into(),
                personality_prompt.to_string(),
            ));
            let persona_args = [
                WrapperArg::Literal("--append-system-prompt".into()),
                WrapperArg::Env("AGENT_OFFICE_PERSONA".into()),
            ];
            if let Some(claude) = plan
                .wrappers
                .iter_mut()
                .find(|wrapper| wrapper.command == "claude")
            {
                claude.prefix_args.extend(persona_args);
            } else {
                plan.wrappers.push(CommandWrapperSpec {
                    command: "claude".into(),
                    prefix_args: persona_args.into(),
                    skip_if_present: vec![],
                });
            }
        }

        // prepare_session이 파일을 만든 뒤 spawn이 Err 또는 panic으로 끝나도
        // observer 아티팩트가 남지 않게 한다. 세션 등록 성공 뒤에는 Session이
        // cleanup_paths를 인계받아 dispose/on_exit에서 정리한다.
        struct ObserverPlanGuard {
            paths: Vec<std::path::PathBuf>,
            armed: bool,
        }
        impl Drop for ObserverPlanGuard {
            fn drop(&mut self) {
                if self.armed {
                    cleanup_paths(&self.paths);
                }
            }
        }
        let mut observer_plan_guard = ObserverPlanGuard {
            paths: plan.cleanup_paths.clone(),
            armed: true,
        };

        let resolved = (self.shell_resolver)(req.shell.as_deref(), &plan.wrappers);
        let cwd = req.cwd.clone().map(expand_tilde).unwrap_or_else(home_dir);
        let mut env = vec![
            ("AGENT_OFFICE_SESSION".into(), session_id.clone()),
            ("TERM".into(), "xterm-256color".into()),
        ];
        if let Some(url) = observer_url {
            env.push(("AGENT_OFFICE_HOOK_URL".into(), url));
        }
        // §핵심 5: 재시작 후 입양된 세션의 훅이 스폰 시점의(죽은) 포트를
        // 때리는 문제 완화 -- forwarder가 이 경로의 observer-port 파일을
        // 읽어 재시도할 수 있게 셸 env에 앱 데이터 디렉터리를 실어 둔다.
        if let Some(dir) = &self.app_data_dir {
            env.push(("AGENT_OFFICE_APP_DATA".into(), dir.to_string_lossy().into_owned()));
        }
        env.extend(plan.env.iter().cloned());
        env.extend(resolved.extra_env.iter().cloned());
        let actual_shell = resolved.program.clone();
        let actual_cwd = cwd.clone();
        let settings_path = env
            .iter()
            .rev()
            .find(|(key, _)| key == "AGENT_OFFICE_SETTINGS")
            .map(|(_, value)| std::path::PathBuf::from(value));
        let spawned = match self.factory.spawn(PtySpawnOptions {
            shell: resolved.program,
            args: resolved.args,
            cols: req.cols.unwrap_or(80),
            rows: req.rows.unwrap_or(24),
            cwd,
            env,
            agent_id: req.agent_id.clone(),
            session_id: session_id.clone(),
            // 브로커 모드 데몬이 크래시-생존 정리에 쓸 경로(비브로커 팩토리는 무시).
            cleanup_paths: plan
                .cleanup_paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
        }) {
            Ok(s) => s,
            // spawn 실패: observer_plan_guard가 설정 파일을 정리한다.
            Err(e) => return Err(e.to_string()),
        };

        self.events.session_started(&SessionStartedEvent {
            agent_id: req.agent_id.clone(),
            session_id: session_id.clone(),
            agent_name: profile.name,
            agent_role: profile.role,
            cwd: actual_cwd.clone(),
            shell: actual_shell,
            at: now_ms(),
        });

        let size = (req.cols.unwrap_or(80), req.rows.unwrap_or(24));
        let (session, started) = self.install_session(
            session_id.clone(),
            req.agent_id.clone(),
            plan.cleanup_paths,
            actual_cwd,
            size,
            spawned,
            None, // eof_stop_gate: create() 경로는 RealWaiter가 독립적으로 exit 판정
            None, // initial_output: 새로 spawn한 세션엔 이어받을 과거 출력이 없다
        );
        // 세션이 맵에 들어갔다 — 이후의 수명은 dispose()/on_exit()가 책임지므로
        // observer 파일 정리 가드를 해제한다.
        observer_plan_guard.armed = false;

        // autostart(기본 false): 세션은 기본적으로 빈 로그인 셸만 띄운다. 사용자가
        // `claude --settings "$AGENT_OFFICE_SETTINGS"`로 직접 기동한다. 명시적으로
        // Some(true)를 요청한 경우에만 stdin 주입 — 단, 실제로 Running으로 전이했을
        // 때만(이미 종료됐다면 주입해봐야 의미 없음).
        if started && req.autostart_claude.unwrap_or(false) {
            // 훅 OFF면 --settings 없이 순수 claude 기동(주입할 설정 파일이 없음).
            // 줄 끝은 CR('\r') — 아래 startup_command와 같은 이유(PowerShell 제출).
            let line = match &settings_path {
                Some(p) => format!("claude --settings \"{}\"\r", p.display()),
                None => "claude\r".to_string(),
            };
            let _ = session.writer.lock().write_all(line.as_bytes());
        }

        // 사용자 지정 시작 명령어: 세션이 실제로 Running으로 전이한 경우에만, 트림 후
        // 빈 값이 아니면 셸 stdin에 한 줄 주입한다. autostart_claude와 동일한 stdin
        // 주입 구조 — autostart는 실무상 항상 false라 두 주입이 겹칠 일은 없다.
        if started {
            if let Some(cmd) = req.startup_command.as_deref() {
                let cmd = cmd.trim();
                if !cmd.is_empty() {
                    // 줄 끝은 LF가 아니라 CR('\r'). PowerShell/PSReadLine은 CR에서만
                    // 라인을 제출한다 — 바로 LF를 보내면 명령이 실행되지 않고 `>>`
                    // 연속 입력 프롬프트에 얹힌 채로 멈춘다. 실제 xterm의 Enter 키도
                    // CR이며, 유닉스 PTY는 ICRNL로 CR->LF를 매핑하므로 CR 하나면
                    // 모든 플랫폼에서 명령이 그대로 실행된다.
                    let line = format!("{cmd}\r");
                    let _ = session.writer.lock().write_all(line.as_bytes());
                }
            }
        }

        let state = *session.state.lock();
        Ok(CreateSessionResult { session_id, state })
    }

    /// spawn 이후 배선부 -- 세션 등록, sink 이어받기, reader/pump/wait 3스레드
    /// 기동, Running CAS(§핵심 4: "create_with_profile의 spawn 이후 배선부를
    /// install_session으로 추출해 create/adopt가 공유"). `create_with_profile`과
    /// `adopt_detached` 둘 다 이 메서드로 수렴한다 -- 상태 머신·sink 재사용
    /// 로직은 완전히 동일하게 유지된다.
    ///
    /// `initial_output`: 데몬이 보관해 둔 미전달 출력(입양 전용). reader
    /// 스레드가 시작되기 *전에* pump 채널로 먼저 흘려보내 순서를 보장한다
    /// (§핵심 4: "pump mpsc에 첫 ReaderMsg::Data로 주입").
    #[allow(clippy::too_many_arguments)]
    fn install_session(
        self: &Arc<Self>,
        session_id: SessionId,
        agent_id: AgentId,
        cleanup_paths: Vec<std::path::PathBuf>,
        cwd: String,
        size: (u16, u16),
        spawned: SpawnedPty,
        eof_stop_gate: Option<Arc<AtomicBool>>,
        initial_output: Option<Vec<u8>>,
    ) -> (Arc<Session>, bool) {
        // 세션 수명과 독립인 agentId sink 재사용: 이미 붙은 채널/백로그를
        // 그대로 이어받아 재생성/재입양 시 재구독이 필요 없다.
        let output = self.sink_for(&agent_id);
        let broker_owned = spawned.broker_owned;
        let broker_stream_offset = spawned.broker_stream_offset.clone();
        let session = Arc::new(Session {
            session_id: session_id.clone(),
            agent_id: agent_id.clone(),
            state: Mutex::new(SessionState::Starting),
            writer: Mutex::new(spawned.writer),
            control: spawned.control,
            cleanup_paths,
            kill_requested: AtomicBool::new(false),
            cwd,
            size: Mutex::new(size),
            handed_off: AtomicBool::new(false),
            #[cfg(unix)]
            reader_interrupt: Mutex::new(spawned.reader_interrupt),
            #[cfg(unix)]
            handoff: Mutex::new(spawned.handoff),
            eof_stop_gate,
            broker_owned,
            broker_stream_offset,
            #[cfg(unix)]
            broker_data_shutdown: Mutex::new(spawned.broker_data_shutdown),
        });

        self.sessions.lock().insert(agent_id.clone(), session.clone());
        self.registry
            .insert(&session_id, &agent_id, SessionState::Starting);
        self.emit_state(&session, SessionState::Starting, None);

        // 1) reader thread (블로킹 read → mpsc)
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<ReaderMsg>();
        if let Some(bytes) = initial_output.filter(|b| !b.is_empty()) {
            // 리더 스레드보다 먼저 보내야 한다 -- unbounded 채널은 send() 호출
            // 순서를 그대로 보존하므로, 아래 스레드 스폰보다 앞서 이 send가
            // happens-before로 확정되면 순서가 깨지지 않는다.
            let _ = tx.send(ReaderMsg::Data(bytes));
        }
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
        spawn_output_pump(session_id.clone(), agent_id.clone(), rx, output, self.hub.clone());

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
            let mut st = session.state.lock();
            if *st == SessionState::Starting {
                *st = SessionState::Running;
                self.registry.set_state(&session_id, SessionState::Running);
                self.emit_state(&session, SessionState::Running, None);
                true
            } else {
                false
            }
        };

        (session, started)
    }

    pub fn write_input(&self, agent_id: &str, data: &str) {
        if let Some(s) = self.find(agent_id) {
            if *s.state.lock() == SessionState::Running {
                let _ = s.writer.lock().write_all(data.as_bytes());
            }
        }
    }

    pub fn resize(&self, agent_id: &str, cols: u16, rows: u16) {
        if let Some(s) = self.find(agent_id) {
            if *s.state.lock() == SessionState::Running {
                let _ = s.control.resize(cols, rows);
                *s.size.lock() = (cols, rows);
            }
        }
    }

    /// 의도적 종료. 최종 Disposed 전이는 wait 스레드의 on_exit에서 확정.
    /// 핸드오프된 세션(§핵심 3)은 즉시 return — kill/cleanup 금지. 그
    /// 세션의 실제 수명은 이제 sessiond가 책임진다.
    pub fn dispose(&self, agent_id: &str) {
        if let Some(s) = self.find(agent_id) {
            if s.handed_off.load(Ordering::SeqCst) {
                return;
            }
            s.kill_requested.store(true, Ordering::SeqCst);
            let _ = s.control.kill();
            cleanup_paths(&s.cleanup_paths);
        }
    }

    /// 앱 quit: 모든 PTY kill + settings 정리(동기, 빠름). 브로커 모드에서도
    /// 세션별 `dispose`가 각자의 control로 죽인다 -- 브로커 세션은 Kill RPC
    /// (데몬이 SIGKILL+테이블 제거+cleanup)로, 데몬 접속 실패 시 폴백 스폰된
    /// 세션은 in-process kill로. 그래서 KillAll 특수 분기 없이 v1과 동일한
    /// 루프면 폴백 세션 누수 없이 전부 정리된다.
    pub fn dispose_all(&self) {
        let ids: Vec<AgentId> = self.sessions.lock().keys().cloned().collect();
        for a in ids {
            self.dispose(&a);
        }
    }

    /// 앱 quit(§핵심 3): Running 세션들을 sessiond로 넘긴다. `snapshots`는
    /// agentId -> 프론트가 종료 직전 직렬화한 xterm 화면(스크롤백 포함) --
    /// 데몬은 핸드오프 *이후* 출력만 링버퍼에 담으므로, 이게 없으면 재입양
    /// 후 종료 전 화면(예: ls 결과)이 사라진다(실증에서 발견된 빈틈).
    /// 반환값은 성공 개수 -- 프론트는 이 수와 무관하게 종료를 진행한다.
    /// `app_data_dir`이 없으면(테스트 등) 0.
    #[cfg(unix)]
    pub fn handoff_all(&self, snapshots: &std::collections::HashMap<String, String>) -> usize {
        if self.broker_mode {
            return self.handoff_all_broker(snapshots);
        }
        let Some(app_data_dir) = self.app_data_dir.clone() else {
            return 0;
        };
        let ids: Vec<AgentId> = {
            let map = self.sessions.lock();
            map.iter()
                .filter(|(_, s)| *s.state.lock() == SessionState::Running)
                .map(|(a, _)| a.clone())
                .collect()
        };
        if ids.is_empty() {
            return 0;
        }

        let socket_path = crate::sessiond::client::default_socket_path(&app_data_dir);
        let log_path = crate::sessiond::client::default_log_path(&app_data_dir);
        let exe_path = std::env::current_exe().unwrap_or_default();
        let client =
            match crate::sessiond::client::connect_or_spawn(&socket_path, &exe_path, &log_path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("agent-office: handoff_all could not reach sessiond: {e}");
                    return 0;
                }
            };

        ids.iter()
            .filter(|agent_id| {
                let snapshot = snapshots
                    .get(agent_id.as_str())
                    .map(|s| s.clone().into_bytes())
                    .unwrap_or_default();
                self.handoff_one(agent_id, &client, snapshot)
            })
            .count()
    }

    #[cfg(not(unix))]
    pub fn handoff_all(&self, _snapshots: &std::collections::HashMap<String, String>) -> usize {
        0
    }

    /// 세션 하나를 넘긴다. 설계 문서 §핵심 3의 순서 그대로: 리더 인터럽트 →
    /// handed_off set → 전송. 실패해도 세션은 그대로 둔다(맵에 남고
    /// handed_off=true) -- 앱은 어차피 곧 종료되므로 마스터 fd가 닫히며
    /// SIGHUP으로 자연 정리된다(설계 문서 "왜 이 방식인가" 참조).
    ///
    /// `snapshot`이 비어 있으면(프론트가 이 터미널을 한 번도 구독하지 않아
    /// 직렬화 대상이 없었던 경우 등) sink의 backlog를 폴백으로 쓴다 --
    /// 실증에서 발견된 빈틈 수정: 그래야 아직 한 번도 열지 않은 터미널도
    /// 재입양 후 종료 전 출력이 최소한 backlog 분량만큼은 보존된다.
    #[cfg(unix)]
    fn handoff_one(
        &self,
        agent_id: &str,
        client: &crate::sessiond::client::Client,
        snapshot: Vec<u8>,
    ) -> bool {
        let Some(sess) = self.find(agent_id) else {
            return false;
        };
        if sess.handed_off.load(Ordering::SeqCst) {
            return false;
        }
        let Some(handoff) = sess.handoff.lock().take() else {
            return false; // Fake/입양 조립 실패 등으로 handoff 정보가 없는 세션은 핸드오프 불가.
        };

        // 재핸드오프(입양 세션)라면 EofWaiter 오발화를 막는다.
        if let Some(gate) = &sess.eof_stop_gate {
            gate.store(true, Ordering::SeqCst);
        }
        if let Some(interrupt) = sess.reader_interrupt.lock().take() {
            interrupt.interrupt();
        }
        // poll 기반 리더는 인터럽트를 수 ms 내 관측한다 -- fd를 보내기 전에
        // 짧게 양보해 리더 스레드가 실제로 빠져나갈 시간을 준다(완료 채널을
        // 새로 두는 것보다 훨씬 단순하고, 실패해도 안전 — 최악의 경우 데몬이
        // 아주 잠깐 늦게 도착한 잔여 바이트를 이어 읽을 뿐 유실은 없다).
        std::thread::sleep(std::time::Duration::from_millis(20));

        sess.handed_off.store(true, Ordering::SeqCst);

        let pid = handoff.pid;
        let pgid = handoff.pgid;
        let master_fd = handoff.take_master_fd();
        let (cols, rows) = *sess.size.lock();
        let cleanup_paths = sess
            .cleanup_paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let snapshot = if snapshot.is_empty() {
            self.sink_for(agent_id).backlog_snapshot()
        } else {
            snapshot
        };

        let result = client.handoff(crate::sessiond::client::HandoffRequest {
            agent_id: agent_id.to_string(),
            session_id: sess.session_id.clone(),
            pid,
            pgid,
            rows,
            cols,
            cwd: sess.cwd.clone(),
            cleanup_paths,
            snapshot,
            master_fd,
        });

        match result {
            Ok(()) => {
                self.sessions.lock().remove(agent_id);
                self.registry.remove(&sess.session_id);
                true
            }
            Err(e) => {
                eprintln!("agent-office: handoff failed for {agent_id}: {e}");
                let _ = nix::unistd::close(master_fd);
                false
            }
        }
    }

    // ── v2 브로커 모드 앱 쪽 분기 ─────────────────────────────────────
    //
    // 브로커 모드 매니저라도 세션은 두 종류가 섞일 수 있다: 데몬이 소유하는
    // 브로커 세션(broker_owned)과, 데몬 접속 실패로 팩토리가 폴백 스폰한
    // in-process 세션. "유지하고 종료"를 세션 단위로 가른다:
    //   - broker_owned: 데몬이 자식을 이미 소유하므로 **스냅샷 업로드 후
    //     detach**(맵에서만 떼어내 dispose_all이 Kill하지 않게 함).
    //   - 폴백 세션: 앱이 fd를 쥐고 있으므로 **기존 v1 fd 핸드오프**로 넘긴다.
    // 하나의 connect_or_spawn 연결로 두 경로를 모두 처리한다 -- 데몬은 proto 2라
    // v1 Handoff와 v2 UpdateSnapshot을 같은 연결에서 받는다.
    #[cfg(unix)]
    fn handoff_all_broker(&self, snapshots: &std::collections::HashMap<String, String>) -> usize {
        let Some(app_data_dir) = self.app_data_dir.clone() else {
            return 0;
        };
        let ids: Vec<AgentId> = {
            let map = self.sessions.lock();
            map.iter()
                .filter(|(_, s)| *s.state.lock() == SessionState::Running)
                .map(|(a, _)| a.clone())
                .collect()
        };
        if ids.is_empty() {
            return 0;
        }
        let socket_path = crate::sessiond::client::default_socket_path(&app_data_dir);
        let log_path = crate::sessiond::client::default_log_path(&app_data_dir);
        let exe_path = std::env::current_exe().unwrap_or_default();
        let client =
            crate::sessiond::client::connect_or_spawn(&socket_path, &exe_path, &log_path).ok();
        let mut count = 0;
        for agent_id in ids {
            let Some(sess) = self.find(&agent_id) else { continue };
            if sess.handed_off.load(Ordering::SeqCst) {
                continue;
            }
            if sess.broker_owned {
                // 브로커 세션: 최신 스냅샷 업로드(best-effort) 후 detach. 데몬에
                // 못 닿아도 detach는 진행해야 dispose_all이 자식을 죽이지 않는다.
                if let (Some(client), Some(snap)) = (&client, snapshots.get(agent_id.as_str())) {
                    let offset = sess
                        .broker_stream_offset
                        .as_ref()
                        .map(|c| c.load(Ordering::SeqCst));
                    let _ = client.update_snapshot(&agent_id, snap.as_bytes(), offset);
                }
                sess.handed_off.store(true, Ordering::SeqCst);
                // data 소켓을 결정적으로 shutdown: reader 스레드를 EOF로 종료시키고
                // 데몬에 FIN을 보내 conn을 정리시킨다(§#50 선결). 이게 없으면 reader
                // 스레드가 clone fd를 프로세스 종료까지 쥐어 데몬 conn이 살아 있고
                // List `attached`가 stale-true로 고착돼, 다음 인스턴스가 라이브 소유로
                // 오판하거나 같은 프로세스 재입양이 깨진다.
                if let Some(sd) = sess.broker_data_shutdown.lock().take() {
                    sd.shutdown();
                }
                // 데몬이 FIN을 관측해 conn을 떼어낼 짧은 여유(handoff_one과 동일 패턴).
                // 실제 앱 종료 시엔 이후 프로세스가 죽어 무관하나, 같은 프로세스에서
                // 곧바로 재입양하는 경우 attached=false로 수렴할 시간을 준다.
                std::thread::sleep(std::time::Duration::from_millis(20));
                self.sessions.lock().remove(&agent_id);
                self.registry.remove(&sess.session_id);
                count += 1;
            } else if let Some(client) = &client {
                // 폴백(in-process) 세션: 기존 v1 fd 핸드오프(reader 인터럽트 →
                // fd 전송 → 맵 제거). 스냅샷이 없으면 handoff_one이 backlog로 폴백.
                let snapshot = snapshots
                    .get(agent_id.as_str())
                    .map(|s| s.clone().into_bytes())
                    .unwrap_or_default();
                if self.handoff_one(&agent_id, client, snapshot) {
                    count += 1;
                }
            }
        }
        count
    }

    /// 주기 스냅샷 업로드(브로커 모드 전용). 렌더러가 30초마다 직렬화한 화면을
    /// 데몬에 올려 앱 크래시 후에도 마지막 화면을 복원할 수 있게 한다.
    /// 브로커 모드가 아니거나 데몬에 못 닿으면 no-op.
    #[cfg(unix)]
    pub fn upload_snapshots(&self, snapshots: &std::collections::HashMap<String, String>) {
        if !self.broker_mode {
            return;
        }
        let Some(app_data_dir) = &self.app_data_dir else {
            return;
        };
        let Ok(client) = crate::session::broker_pty::connect(app_data_dir) else {
            return;
        };
        for (agent_id, snap) in snapshots {
            // 데몬 테이블에 없는 agentId면 no-op으로 무시된다(안전). 앱 쪽 세션의
            // 수신 오프셋을 offset으로 동봉해 유실 창을 없앤다(§P1) -- 세션이 없거나
            // 브로커가 아니면 None(데몬은 수신 시점 ring.total()로 폴백).
            let offset = self
                .find(agent_id)
                .and_then(|s| s.broker_stream_offset.as_ref().map(|c| c.load(Ordering::SeqCst)));
            let _ = client.update_snapshot(agent_id, snap.as_bytes(), offset);
        }
    }

    #[cfg(not(unix))]
    pub fn upload_snapshots(&self, _snapshots: &std::collections::HashMap<String, String>) {}

    /// 브로커 모드 재접속: List를 훑어 세션 종류별로 되찾는다 -- **broker=true는
    /// Attach+DataAttach(브로커 경로)로, broker=false(v1 핸드오프/폴백 세션)는
    /// 기존 v1 adopt(adopt_one, fd 회수)로** 입양한다. 후자는 이전 실행이 폴백
    /// 스폰한 세션을 v1 fd 핸드오프로 넘긴 경우나, 브로커로 업그레이드하기 전
    /// 남아 있던 세션을 커버한다(협상 p=1인 구데몬 상대로는 애초에 broker 항목이
    /// 없으니 자연히 v1만 처리된다). exited 항목은 스킵.
    #[cfg(unix)]
    fn adopt_detached_broker(
        self: &Arc<Self>,
        known_agent_ids: &std::collections::HashSet<String>,
    ) -> Vec<AdoptedSessionInfo> {
        let Some(app_data_dir) = self.app_data_dir.clone() else {
            return Vec::new();
        };
        if !crate::session::broker_pty::socket_exists(&app_data_dir) {
            return Vec::new();
        }
        let Ok(client) = crate::session::broker_pty::connect(&app_data_dir) else {
            return Vec::new();
        };
        let sessions = client.list().unwrap_or_default();
        let mut adopted = Vec::new();
        for info in sessions {
            if info.exited {
                // 종료된 브로커 세션은 best-effort Kill로 데몬 테이블에서 치운다
                // (§P2-a) -- detach 중 자식이 죽으면 exited 엔트리가 영원히 남아
                // 데몬의 table-empty 종료를 막는 누수가 된다. v1 exited 항목은
                // 기존대로 스킵(v1 Adopt/Kill 수명 규칙 유지).
                if info.broker {
                    let _ = client.kill(&info.agent_id);
                }
                continue;
            }
            // §#50: 다른 앱 인스턴스가 지금 활성 data conn을 붙여 둔 세션
            // (info.attached)은 입양하지 않는다 -- 입양하면 DataAttach 교체로
            // 데몬이 그 인스턴스의 data 소켓을 shutdown해 원본 터미널이 먹통이
            // 된다(앱은 단일 인스턴스 강제가 없다). detach가 이제 소켓을 결정적
            // shutdown하므로(broker_data_shutdown) 정상 재시작/크래시(프로세스
            // 종료로 OS가 fd를 닫음)면 데몬이 conn을 정리해 attached=false가 되어
            // 여기서 정상 입양된다. attached=true는 "살아 있는 다른 인스턴스 소유".
            // v1 세션은 데몬이 항상 attached=false로 주므로 영향 없다.
            // TOCTOU(List~DataAttach 창)는 수용: 두 인스턴스가 ms 창에서 같은
            // 미소유 세션을 경합해도 데몬 gen 직렬화로 크래시 없이 last-wins 수렴.
            if info.attached {
                eprintln!(
                    "agent-office: skip adopt of {} — attached by another live instance",
                    info.agent_id
                );
                continue;
            }
            if !known_agent_ids.contains(&info.agent_id) {
                let _ = client.kill(&info.agent_id); // 삭제된 에이전트의 고아 세션 정리.
                continue;
            }
            let result = if info.broker {
                self.adopt_one_broker(&app_data_dir, &info, &client)
            } else {
                // v1 핸드오프/폴백 세션은 기존 fd 회수 경로로 입양한다(공유 연결 사용).
                self.adopt_one(&info.agent_id, &client)
            };
            if let Some(r) = result {
                adopted.push(r);
            }
        }
        adopted
    }

    /// 브로커 세션 하나 입양: Attach로 메타/스냅샷을 회수하고 DataAttach로
    /// 백로그 리플레이 스트림을 붙인다. 종료는 BrokerWaiter(Wait RPC)가 실제
    /// exit code로 관측한다(v1 EofWaiter의 "exit code 소실" 제약 해소).
    #[cfg(unix)]
    fn adopt_one_broker(
        self: &Arc<Self>,
        app_data_dir: &std::path::Path,
        info: &crate::sessiond::protocol::SessionInfo,
        client: &crate::sessiond::client::Client,
    ) -> Option<AdoptedSessionInfo> {
        let (spawned, meta) =
            match crate::session::broker_pty::assemble_broker_adopted(app_data_dir, &info.agent_id) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("agent-office: broker adopt failed for {}: {e}", info.agent_id);
                    return None;
                }
            };
        // List와 Attach 사이에 자식이 죽었으면(경합) 입양하지 않는다. 이때
        // 데몬 테이블엔 exited 엔트리가 남아 table-empty 종료를 막으므로,
        // best-effort Kill로 치운다(입양은 boot 때 1회뿐이라 나중에 dispose할
        // 매니저 세션이 안 생겨 방치되면 영구 잔류한다).
        if meta.exit.is_some() {
            let _ = client.kill(&info.agent_id);
            return None;
        }
        // Attach가 준 라이브 크기를 우선(리사이즈 후 List가 낡았을 수 있다).
        let size = (meta.cols, meta.rows);
        // 화면 복원: 업로드된 스냅샷이 있으면 항상 initial_output으로 주입한다.
        // 데몬은 그 스냅샷 시점 이후의 링버퍼 바이트만 data 연결로 리플레이하므로
        // (snapshot_offset 기반), "스냅샷 + 이후 출력"이 되어 중복 없이 전체
        // 스크롤백이 복원된다. 스냅샷이 한 번도 업로드 안 됐으면 데몬이 링 전체를
        // 리플레이하고 meta.snapshot은 비어 있어 주입하지 않는다.
        let initial_output = (!meta.snapshot.is_empty()).then_some(meta.snapshot);
        // cleanup_paths는 데몬이 Spawn 때 받아 보관·정리하므로 앱 쪽은 비운다.
        let (session, _started) = self.install_session(
            info.session_id.clone(),
            info.agent_id.clone(),
            Vec::new(),
            info.cwd.clone(),
            size,
            spawned,
            None, // eof_stop_gate: 브로커는 Wait RPC로 종료를 관측한다.
            initial_output,
        );
        Some(AdoptedSessionInfo {
            agent_id: info.agent_id.clone(),
            session_id: session.session_id.clone(),
            rows: size.1,
            cols: size.0,
        })
    }

    /// 부트스트랩(§핵심 4): sessiond에 남아 있는 세션들을 되찾는다.
    /// `known_agent_ids`는 영속 프로필의 agentId 집합 -- 여기 없는 항목은
    /// Kill 지시(삭제된 에이전트의 고아 claude 방지), exited 항목은 스킵.
    /// 소켓이 없거나 연결 실패면 빈 벡터(데몬을 새로 스폰하지 않는다 --
    /// 입양할 게 없으면 없는 대로다).
    #[cfg(unix)]
    pub fn adopt_detached(
        self: &Arc<Self>,
        known_agent_ids: &std::collections::HashSet<String>,
    ) -> Vec<AdoptedSessionInfo> {
        if self.broker_mode {
            return self.adopt_detached_broker(known_agent_ids);
        }
        let Some(app_data_dir) = &self.app_data_dir else {
            return Vec::new();
        };
        let socket_path = crate::sessiond::client::default_socket_path(app_data_dir);
        if !socket_path.exists() {
            return Vec::new();
        }
        let client = match crate::sessiond::client::Client::connect(&socket_path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let sessions = match client.list() {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let mut adopted = Vec::new();
        for info in sessions {
            if info.exited {
                continue;
            }
            if !known_agent_ids.contains(&info.agent_id) {
                let _ = client.kill(&info.agent_id);
                continue;
            }
            if let Some(result) = self.adopt_one(&info.agent_id, &client) {
                adopted.push(result);
            }
        }
        adopted
    }

    #[cfg(not(unix))]
    pub fn adopt_detached(
        self: &Arc<Self>,
        _known_agent_ids: &std::collections::HashSet<String>,
    ) -> Vec<AdoptedSessionInfo> {
        Vec::new()
    }

    /// 세션 하나를 입양해 install_session으로 재배선한다. 실패하면 None --
    /// 그 세션은 데몬 테이블에 그대로 남아 다음 재시작에서 다시 시도할 수
    /// 있다(이번 연결에서 이미 Adopt를 보낸 뒤 실패했다면 데몬 쪽에선 이미
    /// 테이블에서 빠진 상태이므로 fd 자체는 유실 -- assemble_adopted 실패는
    /// 극히 드문 경로라 이 트레이드오프를 받아들인다).
    #[cfg(unix)]
    fn adopt_one(
        self: &Arc<Self>,
        agent_id: &str,
        client: &crate::sessiond::client::Client,
    ) -> Option<AdoptedSessionInfo> {
        let adopted = client.adopt(agent_id).ok()?;
        let (spawned, stop_gate) = match crate::session::pty_factory::assemble_adopted(
            adopted.master_fd,
            adopted.pid,
            adopted.pgid,
        ) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("agent-office: failed to assemble adopted session {agent_id}: {e}");
                let _ = nix::unistd::close(adopted.master_fd);
                return None;
            }
        };
        let cleanup_paths = adopted.cleanup_paths.iter().map(std::path::PathBuf::from).collect();
        let size = (adopted.cols, adopted.rows);
        // 종료 직전 화면 스냅샷 -> 핸드오프 이후 링버퍼 순으로 이어붙인다
        // (실증에서 발견된 빈틈 수정) -- 순서가 바뀌면 화면이 뒤죽박죽으로
        // 재생된다. install_session이 빈 벡터는 initial_output 주입 자체를
        // 건너뛰므로 둘 다 없을 때를 따로 가릴 필요가 없다.
        let mut initial_output = adopted.snapshot;
        initial_output.extend_from_slice(&adopted.buffer);
        let (session, _started) = self.install_session(
            adopted.session_id,
            agent_id.to_string(),
            cleanup_paths,
            adopted.cwd,
            size,
            spawned,
            Some(stop_gate),
            Some(initial_output),
        );
        Some(AdoptedSessionInfo {
            agent_id: agent_id.to_string(),
            session_id: session.session_id.clone(),
            rows: size.1,
            cols: size.0,
        })
    }

    /// subscribe_output 커맨드가 호출: agentId에 Channel 등록(+백로그 드레인).
    /// 세션이 아직 없어도 sink를 만들어 채널을 보관한다(pending attach) —
    /// 이후 create()가 같은 sink를 이어받아 재구독 없이 출력이 흐른다.
    pub fn attach_output(&self, agent_id: &str, channel: Channel<OutputChunk>) {
        self.sink_for(agent_id).attach(channel);
    }
    pub fn detach_output(&self, agent_id: &str) {
        if let Some(s) = self.sinks.lock().get(agent_id) {
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
        // 핸드오프된 세션(§핵심 3)은 즉시 return -- kill/cleanup/상태이벤트
        // 금지. 실제로는 create()의 RealWaiter가 앱 프로세스 종료와 함께
        // 죽으므로 프로덕션에서 이 가드가 실행 도달하는 일은 드물지만(핸드오프
        // 직후 앱이 곧장 종료), dispose()와 대칭을 이루는 안전망이다.
        if sess.handed_off.load(Ordering::SeqCst) {
            return;
        }
        cleanup_paths(&sess.cleanup_paths);
        let intentional = sess.kill_requested.load(Ordering::SeqCst);
        let exit = SessionExitInfo {
            session_id: sess.session_id.clone(),
            exit_code: outcome.exit_code,
            signal: outcome.signal,
            intentional,
        };
        let next = if intentional {
            SessionState::Disposed
        } else {
            SessionState::Exited
        };
        // state 락을 registry.set_state까지 계속 쥐어 create()의 Running CAS와 상호
        // 배제한다: 상태 전이는 Starting-게이트 CAS로 단조(monotonic) 보장 →
        // "Exited 이후 Running" 역전 차단. (emit은 아래 superseded 판정 뒤로 뺀다 —
        // 낡은 세션의 상태 이벤트가 프론트에서 새 세션을 덮어쓰지 않게 하기 위해.
        // state→sessions 락 중첩은 create()의 sessions→state와 데드락이 되므로
        // 여기서는 state 락을 먼저 놓고 sessions 락을 잡는다.)
        {
            let mut st = sess.state.lock();
            *st = next;
            self.registry.set_state(&sess.session_id, next);
        }

        // 미해결 알림 정리(session_id 스코프 — 교체 여부와 무관).
        self.hub.purge_session(&sess.session_id);

        // 재시작 레이스 가드: dispose 직후 create()가 같은 agentId에 새 세션을
        // 밀어넣었다면(create의 재사용 가드가 kill_requested 세션을 맵에서 떼어냄)
        // 이 세션은 이미 "교체됨". 그때 맵/상태이벤트를 건드리면 새 세션을
        // 오염시키므로 건드리지 않는다. 맵 확인과 (미교체 시의) 제거를 하나의
        // sessions 락 임계구역에서 수행 → create()의 맵 제거/삽입과 순서가 확정된다.
        //
        // sink는 여기서 절대 제거하지 않는다(2026-07-11 "터미널이 재시작해도
        // 영구히 안 뜸" 근본 원인). sink는 agentId 키의 세션-수명-독립 자원인데,
        // 세션 수명 이벤트인 on_exit이 지우면 — 재시작 중 on_exit(Disposed)이
        // 다음 create보다 먼저 완주하는(빠른 reap, macOS) 순서에서 — 프론트가
        // attach해 둔 채널이 sink째로 버려진다. 프론트는 재시작 중 재구독
        // IPC를 보내지 않으므로(사운드 매니저가 onData를 상시 구독) 이후의
        // 어떤 재시작에도 출력이 채널에 닿지 않아 터미널이 영구 blank가 된다.
        // 에이전트 삭제 후 남는 sink는 무해한 소량(detach된 채널 + 캡 있는
        // 백로그)이므로 세션 수명과 묶지 않고 그대로 둔다.
        let is_current = {
            let mut map = self.sessions.lock();
            let current = map
                .get(&sess.agent_id)
                .map(|s| s.session_id == sess.session_id)
                .unwrap_or(false);
            if current && next == SessionState::Disposed {
                // 재사용 안 함 → 맵에서 제거(레지스트리는 아래에서 제거).
                map.remove(&sess.agent_id);
            }
            current
        };

        // 여전히 이 agentId의 현재 세션일 때만 상태 이벤트를 방출한다 — 교체된
        // 낡은 세션의 Disposed/Exited가 프론트(agentId 키)에서 새 세션의 상태를
        // 덮어쓰지 않게 한다.
        if is_current {
            self.emit_state(sess, next, Some(exit));
        }

        if next == SessionState::Disposed {
            self.registry.remove(&sess.session_id);
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
    /// `shells::resolve_observed`) so tests can exercise zsh ZDOTDIR wiring in
    /// `create()` without depending on the host's actual `$SHELL`, or record
    /// what the resolver was invoked with. Must be called before wrapping in
    /// `Arc::new` (consumes `self` by value).
    fn with_shell_resolver(
        mut self,
        resolver: Arc<
            dyn Fn(Option<&str>, &[CommandWrapperSpec]) -> shells::ResolvedShell + Send + Sync,
        >,
    ) -> Self {
        self.shell_resolver = resolver;
        self
    }
}

fn cleanup_paths(paths: &[std::path::PathBuf]) {
    for path in paths {
        if let Err(error) = std::fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("observer cleanup failed for {}: {error}", path.display());
            }
        }
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
                    Some(ReaderMsg::Eof) | None => {
                        batcher.flush_final(&*sink); // 잔여 강제 방출
                        break;
                    }
                }
            }
        }
    });
}

pub(crate) fn home_dir() -> String {
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
/// pub(crate): open_in_vscode/open_in_terminal/pick_directory 커맨드가
/// 프로필 cwd를 그대로 받으므로 세션 생성과 동일한 확장을 공유한다.
pub(crate) fn expand_tilde(path: String) -> String {
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
    use crate::observer::claude::ClaudeAdapter;
    use crate::observer::{
        AdapterSessionPlan, CommandWrapperSpec, ObserverAdapter, ObserverAdapterError,
        ObserverEvent, ObserverProvider, ObserverRuntime, ObserverSessionContext, RawObserverHook,
    };
    use crate::session::pty_factory::fake::{
        AlwaysFailPtyFactory, FakeControl, FakePtyFactory, MultiFakePtyFactory,
    };
    use crate::state::fake::RecordingEvents;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use tauri::ipc::{Channel, InvokeResponseBody};

    fn registry() -> Arc<SessionRegistry> {
        Arc::new(SessionRegistry::new())
    }

    fn hub_for(registry: Arc<SessionRegistry>, events: Arc<dyn AppEvents>) -> Arc<NotificationHub> {
        Arc::new(NotificationHub::new(
            registry,
            events,
            Arc::new(SystemClock),
            Duration::from_millis(3000),
        ))
    }

    /// Unique tempdir per test so parallel `cargo test` runs never collide.
    fn scratch_observer_dir() -> PathBuf {
        std::env::temp_dir().join(format!("agent-office-manager-test-{}", Uuid::new_v4()))
    }

    fn claude_observer(hub: Arc<NotificationHub>, dir: PathBuf) -> Arc<ObserverRuntime> {
        Arc::new(ObserverRuntime::new(
            hub,
            vec![Arc::new(ClaudeAdapter::new(
                dir,
                std::env::current_exe().unwrap(),
            ))],
        ))
    }

    fn req(agent_id: &str, autostart: Option<bool>) -> CreateSessionRequest {
        CreateSessionRequest {
            agent_id: agent_id.into(),
            cols: None,
            rows: None,
            cwd: None,
            shell: None,
            startup_command: None,
            personality_prompt: None,
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
            startup_command: None,
            personality_prompt: None,
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
            startup_command: None,
            personality_prompt: None,
            autostart_claude: Some(false),
        }
    }

    fn req_with_startup(agent_id: &str, startup_command: Option<String>) -> CreateSessionRequest {
        CreateSessionRequest {
            agent_id: agent_id.into(),
            cols: None,
            rows: None,
            cwd: None,
            shell: None,
            startup_command,
            personality_prompt: None,
            // autostart OFF: startup_command 주입만 단독 검증(두 주입이 겹치지 않게).
            autostart_claude: Some(false),
        }
    }

    fn req_with_persona(
        agent_id: &str,
        personality_prompt: Option<String>,
    ) -> CreateSessionRequest {
        CreateSessionRequest {
            agent_id: agent_id.into(),
            cols: None,
            rows: None,
            cwd: None,
            shell: None,
            startup_command: None,
            personality_prompt,
            autostart_claude: Some(false),
        }
    }

    /// Polls `pred` until it's true, panicking after a generous timeout
    /// instead of hanging forever if the pump/wait thread wiring is broken.
    async fn wait_for<F: Fn() -> bool>(pred: F) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while !pred() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "condition not met within timeout"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// One `SessionManager` wired to a single-spawn `FakePtyFactory` (per
    /// the fake's own contract: one fake per session under test), with a
    /// caller-chosen observation state. Disabled sessions skip observer
    /// preparation; enabled sessions receive a deterministic endpoint.
    fn build_with_observer(
        enabled: bool,
    ) -> (
        Arc<SessionManager>,
        Arc<RecordingEvents>,
        Arc<FakeControl>,
        PathBuf,
    ) {
        let events = Arc::new(RecordingEvents::default());
        let reg = registry();
        let hub = hub_for(reg.clone(), events.clone() as Arc<dyn AppEvents>);
        let dir = scratch_observer_dir();
        let observer = claude_observer(hub.clone(), dir.clone());
        let (fac, ctl) = FakePtyFactory::new();
        let endpoint = enabled.then(|| "http://127.0.0.1:12345/hook".to_string());
        let mgr = Arc::new(SessionManager::new(
            Arc::new(fac),
            observer,
            reg,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(move || endpoint.clone()),
        ));
        (mgr, events, ctl, dir)
    }

    fn build() -> (
        Arc<SessionManager>,
        Arc<RecordingEvents>,
        Arc<FakeControl>,
        PathBuf,
    ) {
        build_with_observer(true)
    }

    fn cleanup(ctl: &FakeControl, dir: &PathBuf) {
        // Let the reader thread observe EOF so it doesn't block forever.
        ctl.close_output();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[derive(Clone)]
    struct PlanAdapter {
        provider: ObserverProvider,
        result: Result<AdapterSessionPlan, ObserverAdapterError>,
    }

    impl ObserverAdapter for PlanAdapter {
        fn provider(&self) -> ObserverProvider {
            self.provider
        }

        fn prepare_session(
            &self,
            _context: &ObserverSessionContext,
        ) -> Result<AdapterSessionPlan, ObserverAdapterError> {
            self.result.clone()
        }

        fn map_hook(&self, _raw: &RawObserverHook<'_>) -> Option<ObserverEvent> {
            None
        }
    }

    fn plan_adapter(provider: ObserverProvider, command: &str) -> Arc<dyn ObserverAdapter> {
        Arc::new(PlanAdapter {
            provider,
            result: Ok(AdapterSessionPlan {
                env: if provider == ObserverProvider::Codex {
                    vec![(
                        "AGENT_OFFICE_CODEX_HOOK_STOP".into(),
                        "hooks.Stop=[]".into(),
                    )]
                } else {
                    vec![]
                },
                wrappers: vec![CommandWrapperSpec {
                    command: command.into(),
                    prefix_args: vec![],
                    skip_if_present: vec![],
                }],
                cleanup_paths: vec![],
            }),
        })
    }

    fn build_observer_manager(
        enabled: bool,
        adapters: Vec<Arc<dyn ObserverAdapter>>,
    ) -> (
        Arc<SessionManager>,
        Arc<FakeControl>,
        Arc<Mutex<Vec<CommandWrapperSpec>>>,
        PathBuf,
    ) {
        let events = Arc::new(RecordingEvents::default());
        let registry = Arc::new(SessionRegistry::new());
        let hub = Arc::new(NotificationHub::new(
            registry.clone(),
            events.clone(),
            Arc::new(SystemClock),
            Duration::from_millis(3_000),
        ));
        let observer = Arc::new(ObserverRuntime::new(hub.clone(), adapters));
        let (factory, control) = FakePtyFactory::new();
        let endpoint = enabled.then(|| "http://127.0.0.1:43123/hook".to_string());
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let recorded_for_resolver = recorded.clone();
        let manager = SessionManager::new(
            Arc::new(factory),
            observer,
            registry,
            events,
            hub,
            Arc::new(move || endpoint.clone()),
        )
        .with_shell_resolver(Arc::new(move |_selected, wrappers| {
            *recorded_for_resolver.lock() = wrappers.to_vec();
            shells::ResolvedShell {
                program: "test-shell".into(),
                args: vec![],
                extra_env: vec![],
            }
        }));
        let scratch = std::env::temp_dir().join(format!(
            "agent-office-observer-manager-test-{}",
            Uuid::new_v4(),
        ));
        (Arc::new(manager), control, recorded, scratch)
    }

    fn cleanup_observer_fixture(control: &FakeControl, scratch: &Path) {
        control.close_output();
        let _ = std::fs::remove_dir_all(scratch);
    }

    #[tokio::test]
    async fn observer_off_spawns_without_observer_env_or_wrappers() {
        let (manager, control, recorded_wrappers, scratch) = build_observer_manager(false, vec![]);
        manager.create(req("a1", Some(false))).unwrap();
        let env = control.spawned_env();
        assert!(env.iter().all(|(key, _)| key != "AGENT_OFFICE_HOOK_URL"));
        assert!(env
            .iter()
            .all(|(key, _)| !key.starts_with("AGENT_OFFICE_CODEX_HOOK_")));
        assert!(recorded_wrappers.lock().is_empty());
        cleanup_observer_fixture(&control, &scratch);
    }

    #[tokio::test]
    async fn persona_merges_into_existing_claude_wrapper_when_observer_is_on() {
        let adapters: Vec<Arc<dyn ObserverAdapter>> = vec![Arc::new(PlanAdapter {
            provider: ObserverProvider::Claude,
            result: Ok(AdapterSessionPlan {
                env: vec![("AGENT_OFFICE_SETTINGS".into(), "settings.json".into())],
                wrappers: vec![CommandWrapperSpec {
                    command: "claude".into(),
                    prefix_args: vec![
                        WrapperArg::Literal("--settings".into()),
                        WrapperArg::Env("AGENT_OFFICE_SETTINGS".into()),
                    ],
                    skip_if_present: vec!["--settings".into()],
                }],
                cleanup_paths: vec![],
            }),
        })];
        let (manager, control, recorded_wrappers, scratch) = build_observer_manager(true, adapters);
        let prompt = "차분하게 답한다.\n근거를 먼저 제시한다.";
        manager
            .create(req_with_persona("a1", Some(prompt.into())))
            .unwrap();

        let wrappers = recorded_wrappers.lock();
        let claude_wrappers = wrappers
            .iter()
            .filter(|wrapper| wrapper.command == "claude")
            .collect::<Vec<_>>();
        assert_eq!(claude_wrappers.len(), 1);
        assert_eq!(
            claude_wrappers[0].prefix_args,
            vec![
                WrapperArg::Literal("--settings".into()),
                WrapperArg::Env("AGENT_OFFICE_SETTINGS".into()),
                WrapperArg::Literal("--append-system-prompt".into()),
                WrapperArg::Env("AGENT_OFFICE_PERSONA".into()),
            ]
        );
        assert_eq!(
            claude_wrappers[0].skip_if_present,
            vec!["--settings"]
        );
        drop(wrappers);
        assert!(control
            .spawned_env()
            .contains(&("AGENT_OFFICE_PERSONA".into(), prompt.into())));
        cleanup_observer_fixture(&control, &scratch);
    }

    #[tokio::test]
    async fn persona_pushes_one_claude_wrapper_when_observer_is_off() {
        let (manager, control, recorded_wrappers, scratch) = build_observer_manager(false, vec![]);
        manager
            .create(req_with_persona("a1", Some("해적처럼 말한다.".into())))
            .unwrap();

        let wrappers = recorded_wrappers.lock();
        assert_eq!(wrappers.len(), 1);
        assert_eq!(wrappers[0].command, "claude");
        assert_eq!(
            wrappers[0].prefix_args,
            vec![
                WrapperArg::Literal("--append-system-prompt".into()),
                WrapperArg::Env("AGENT_OFFICE_PERSONA".into()),
            ]
        );
        assert!(wrappers[0].skip_if_present.is_empty());
        drop(wrappers);
        assert!(control
            .spawned_env()
            .contains(&("AGENT_OFFICE_PERSONA".into(), "해적처럼 말한다.".into())));
        cleanup_observer_fixture(&control, &scratch);
    }

    #[tokio::test]
    async fn blank_persona_does_not_add_env_or_wrapper() {
        let (manager, control, recorded_wrappers, scratch) = build_observer_manager(false, vec![]);
        manager
            .create(req_with_persona("a1", Some(" \n\t ".into())))
            .unwrap();

        assert!(recorded_wrappers.lock().is_empty());
        assert!(control
            .spawned_env()
            .iter()
            .all(|(key, _)| key != "AGENT_OFFICE_PERSONA"));
        cleanup_observer_fixture(&control, &scratch);
    }

    #[tokio::test]
    async fn observed_session_merges_both_adapters_and_keeps_startup_command() {
        let adapters = vec![
            plan_adapter(ObserverProvider::Claude, "claude"),
            plan_adapter(ObserverProvider::Codex, "codex"),
        ];
        let (manager, control, recorded_wrappers, scratch) = build_observer_manager(true, adapters);
        manager
            .create(req_with_startup("a1", Some("codex resume --last".into())))
            .unwrap();
        let names = recorded_wrappers
            .lock()
            .iter()
            .map(|wrapper| wrapper.command.clone())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            names,
            std::collections::HashSet::from(["claude".into(), "codex".into(), "pi".into(),])
        );
        assert_eq!(control.writes_utf8(), "codex resume --last\r");
        cleanup_observer_fixture(&control, &scratch);
    }

    #[tokio::test]
    async fn adapter_preparation_failure_still_spawns_pty_with_successful_adapter() {
        let adapters: Vec<Arc<dyn ObserverAdapter>> = vec![
            Arc::new(PlanAdapter {
                provider: ObserverProvider::Claude,
                result: Err(ObserverAdapterError::new("injected Claude failure")),
            }),
            plan_adapter(ObserverProvider::Codex, "codex"),
        ];
        let (manager, control, recorded_wrappers, scratch) = build_observer_manager(true, adapters);
        assert!(manager.create(req("a1", Some(false))).is_ok());
        assert_eq!(recorded_wrappers.lock()[0].command, "codex");
        assert!(control
            .spawned_env()
            .iter()
            .any(|(key, _)| key.starts_with("AGENT_OFFICE_CODEX_HOOK_")));
        cleanup_observer_fixture(&control, &scratch);
    }

    #[cfg(windows)]
    struct ManagerGitBashProbe;

    #[cfg(windows)]
    impl shells::ShellProbe for ManagerGitBashProbe {
        fn exists(&self, path: &str) -> bool {
            path == r"C:\Program Files\Git\bin\bash.exe"
        }

        fn program_files(&self) -> Option<String> {
            Some(r"C:\Program Files".into())
        }

        fn program_files_x86(&self) -> Option<String> {
            None
        }

        fn system_root(&self) -> Option<String> {
            None
        }

        fn command_stdout(&self, _program: &str, _args: &[&str]) -> Option<String> {
            None
        }
    }

    #[cfg(windows)]
    struct ManagerFailingShims;

    #[cfg(windows)]
    impl shells::ObserverShimWriter for ManagerFailingShims {
        fn bashrc(&self, _wrappers: &[CommandWrapperSpec]) -> std::io::Result<PathBuf> {
            Err(std::io::Error::other("injected manager bash shim failure"))
        }

        fn zdotdir(&self, _wrappers: &[CommandWrapperSpec]) -> std::io::Result<PathBuf> {
            Err(std::io::Error::other("injected manager zsh shim failure"))
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn shell_shim_failure_still_reaches_session_manager_pty_spawn() {
        let adapters = vec![
            plan_adapter(ObserverProvider::Claude, "claude"),
            plan_adapter(ObserverProvider::Codex, "codex"),
        ];
        let events = Arc::new(RecordingEvents::default());
        let registry = Arc::new(SessionRegistry::new());
        let hub = Arc::new(NotificationHub::new(
            registry.clone(),
            events.clone(),
            Arc::new(SystemClock),
            Duration::from_millis(3_000),
        ));
        let observer = Arc::new(ObserverRuntime::new(hub.clone(), adapters));
        let (factory, control) = FakePtyFactory::new();
        let manager = Arc::new(
            SessionManager::new(
                Arc::new(factory),
                observer,
                registry,
                events,
                hub,
                Arc::new(|| Some("http://127.0.0.1:43123/hook".into())),
            )
            .with_shell_resolver(Arc::new(|selected, wrappers| {
                shells::resolve_observed_with_shims(
                    selected,
                    wrappers,
                    &ManagerGitBashProbe,
                    &ManagerFailingShims,
                )
            })),
        );
        let mut request = req("a1", Some(false));
        request.shell = Some("git-bash".into());

        assert!(manager.create(request).is_ok());
        assert!(control
            .spawned_env()
            .iter()
            .any(|(key, _)| key == "AGENT_OFFICE_HOOK_URL"));
        control.close_output();
    }

    #[tokio::test]
    async fn observer_toggle_changes_only_future_pty_preparation() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let enabled = Arc::new(AtomicBool::new(false));
        let events = Arc::new(RecordingEvents::default());
        let registry = Arc::new(SessionRegistry::new());
        let hub = Arc::new(NotificationHub::new(
            registry.clone(),
            events.clone(),
            Arc::new(SystemClock),
            Duration::from_millis(3_000),
        ));
        let observer = Arc::new(ObserverRuntime::new(
            hub.clone(),
            vec![
                plan_adapter(ObserverProvider::Claude, "claude"),
                plan_adapter(ObserverProvider::Codex, "codex"),
            ],
        ));
        let factory = Arc::new(MultiFakePtyFactory::new());
        let enabled_for_url = enabled.clone();
        let wrapper_calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let wrapper_calls_for_resolver = wrapper_calls.clone();
        let manager = Arc::new(
            SessionManager::new(
                factory.clone(),
                observer,
                registry,
                events,
                hub,
                Arc::new(move || {
                    enabled_for_url
                        .load(Ordering::SeqCst)
                        .then(|| "http://127.0.0.1:43123/hook".into())
                }),
            )
            .with_shell_resolver(Arc::new(move |_selected, wrappers| {
                wrapper_calls_for_resolver.lock().push(
                    wrappers
                        .iter()
                        .map(|wrapper| wrapper.command.clone())
                        .collect(),
                );
                shells::ResolvedShell {
                    program: "test-shell".into(),
                    args: vec![],
                    extra_env: vec![],
                }
            })),
        );

        manager.create(req("off-before", Some(false))).unwrap();
        enabled.store(true, Ordering::SeqCst);
        manager.create(req("on-after", Some(false))).unwrap();
        enabled.store(false, Ordering::SeqCst);
        manager.create(req("off-again", Some(false))).unwrap();

        let calls = wrapper_calls.lock();
        assert!(calls[0].is_empty());
        assert_eq!(
            calls[1]
                .iter()
                .cloned()
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from(["claude".into(), "codex".into(), "pi".into(),]),
        );
        assert!(calls[2].is_empty());
        drop(calls);
        let controls = factory.controls();
        assert!(controls[0]
            .spawned_env()
            .iter()
            .all(|(key, _)| key != "AGENT_OFFICE_HOOK_URL"));
        assert!(controls[1]
            .spawned_env()
            .iter()
            .any(|(key, _)| key == "AGENT_OFFICE_HOOK_URL"));
        assert!(controls[2]
            .spawned_env()
            .iter()
            .all(|(key, _)| key != "AGENT_OFFICE_HOOK_URL"));
        assert!(controls[0]
            .spawned_env()
            .iter()
            .all(|(key, _)| !key.starts_with("AGENT_OFFICE_CODEX_HOOK_")));

        for control in controls {
            control.close_output();
            control.fire_exit(0);
        }
    }

    #[tokio::test]
    async fn pty_spawn_failure_removes_real_claude_settings_file() {
        let settings_dir = std::env::temp_dir().join(format!(
            "agent-office-observer-spawn-failure-{}",
            Uuid::new_v4(),
        ));
        let events = Arc::new(RecordingEvents::default());
        let registry = Arc::new(SessionRegistry::new());
        let hub = Arc::new(NotificationHub::new(
            registry.clone(),
            events.clone(),
            Arc::new(SystemClock),
            Duration::from_millis(3_000),
        ));
        let observer = Arc::new(ObserverRuntime::new(
            hub.clone(),
            vec![Arc::new(ClaudeAdapter::new(
                settings_dir.clone(),
                std::env::current_exe().unwrap(),
            ))],
        ));
        let manager = Arc::new(
            SessionManager::new(
                Arc::new(AlwaysFailPtyFactory),
                observer,
                registry,
                events,
                hub,
                Arc::new(|| Some("http://127.0.0.1:43123/hook".into())),
            )
            .with_shell_resolver(Arc::new(|_, _| shells::ResolvedShell {
                program: "test-shell".into(),
                args: vec![],
                extra_env: vec![],
            })),
        );

        assert!(manager.create(req("a1", Some(false))).is_err());
        let remaining = std::fs::read_dir(&settings_dir)
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(
            remaining, 0,
            "spawn failure must remove adapter cleanup files"
        );
        let _ = std::fs::remove_dir_all(settings_dir);
    }

    // ---- T-A: state transitions + intentional flag ----

    #[tokio::test]
    async fn successful_spawn_emits_session_started_with_profile_and_resolved_context() {
        let events = Arc::new(RecordingEvents::default());
        let reg = registry();
        let hub = hub_for(reg.clone(), events.clone());
        let dir = scratch_observer_dir();
        let observer = claude_observer(hub.clone(), dir.clone());
        let (factory, control) = FakePtyFactory::new();
        let manager = Arc::new(
            SessionManager::new(
                Arc::new(factory),
                observer,
                reg,
                events.clone(),
                hub,
                Arc::new(|| None),
            )
            .with_shell_resolver(Arc::new(|_, _| shells::ResolvedShell {
                program: "/bin/test-shell".into(),
                args: Vec::new(),
                extra_env: Vec::new(),
            })),
        );
        manager
            .create_with_profile(
                req_with_cwd("a1", Some("/work".into())),
                crate::session_events::types::AgentEventProfile {
                    name: "Compiler".into(),
                    role: Some("Platform".into()),
                },
            )
            .unwrap();
        let starts = events.session_starts();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].agent_name, "Compiler");
        assert_eq!(starts[0].agent_role.as_deref(), Some("Platform"));
        assert_eq!(starts[0].cwd, "/work");
        assert_eq!(starts[0].shell, "/bin/test-shell");
        assert_eq!(
            &events.timeline()[..2],
            &[
                "session_started".to_string(),
                "session_state:Starting".to_string(),
            ],
        );
        manager
            .create_with_profile(
                req_with_cwd("a1", Some("/different-work".into())),
                crate::session_events::types::AgentEventProfile {
                    name: "Renamed".into(),
                    role: None,
                },
            )
            .unwrap();
        assert_eq!(
            events.session_starts().len(),
            1,
            "reusing a live session must not log a second start"
        );
        control.close_output();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn create_transitions_starting_running_then_exited_on_unexpected_exit() {
        let (mgr, events, ctl, dir) = build();

        let created = mgr.create(req("a1", Some(false))).unwrap();
        assert_eq!(created.state, SessionState::Running);
        assert_eq!(
            events.states(),
            vec![SessionState::Starting, SessionState::Running]
        );

        ctl.fire_exit(1);
        wait_for(|| events.states().len() == 3).await;

        assert_eq!(
            events.states(),
            vec![
                SessionState::Starting,
                SessionState::Running,
                SessionState::Exited
            ]
        );
        let last = events.last_state().exit.unwrap();
        assert!(
            !last.intentional,
            "unexpected exit must not be marked intentional"
        );
        assert_eq!(last.exit_code, Some(1));

        // unexpected exit keeps the session in bookkeeping (diagnosis/restart).
        assert_eq!(mgr.session_id_for("a1"), Some(created.session_id.clone()));
        assert_eq!(
            mgr.registry.resolve_agent(&created.session_id),
            Some("a1".to_string())
        );

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

        assert_eq!(
            ctl.writes_utf8(),
            "",
            "autostartClaude omitted must not write to stdin"
        );

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_autostart_true_injects_claude_stdin_with_settings_path() {
        let (mgr, _events, ctl, dir) = build();
        let created = mgr.create(req("a1", Some(true))).unwrap();

        let written = ctl.writes_utf8();
        assert!(
            written.starts_with("claude --settings \"") && written.ends_with("\"\r"),
            "unexpected stdin injection: {written:?}"
        );
        assert!(written.contains(&format!("{}.settings.json", created.session_id)));

        cleanup(&ctl, &dir);
    }

    // ---- 시작 명령어(startup_command) stdin 주입 ----

    #[tokio::test]
    async fn create_startup_command_injects_trimmed_line_to_stdin() {
        let (mgr, _events, ctl, dir) = build();
        mgr.create(req_with_startup("a1", Some("source ./init.sh".into())))
            .unwrap();

        assert_eq!(
            ctl.writes_utf8(),
            "source ./init.sh\r",
            "startup_command must be injected verbatim followed by a carriage return \
             (CR submits the line in PowerShell/PSReadLine; a bare LF leaves it at the \
             `>>` continuation prompt. A real xterm Enter is also CR, and a unix PTY's \
             ICRNL maps CR->LF, so CR runs the command on every platform.)",
        );

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_startup_command_blank_skips_injection() {
        let (mgr, _events, ctl, dir) = build();
        // 공백만 있는 명령어 -> 트림 후 빈 값 -> 주입하지 않는다.
        mgr.create(req_with_startup("a1", Some("   ".into())))
            .unwrap();

        assert_eq!(
            ctl.writes_utf8(),
            "",
            "blank startup_command must not write to stdin"
        );

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_startup_command_none_skips_injection() {
        let (mgr, _events, ctl, dir) = build();
        mgr.create(req_with_startup("a1", None)).unwrap();

        assert_eq!(
            ctl.writes_utf8(),
            "",
            "absent startup_command must not write to stdin"
        );

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

    // ---- Observer opt-in OFF skips wiring ----

    #[tokio::test]
    async fn create_with_hooks_disabled_skips_settings_file_and_hook_env() {
        // URL getter가 None을 주면(옵저버 opt-in OFF): --settings 파일을 쓰지
        // 않고, AGENT_OFFICE_SETTINGS/AGENT_OFFICE_HOOK_URL env도 없다.
        let (mgr, _events, ctl, dir) = build_with_observer(false);
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
        let (mgr, _events, ctl, dir) = build_with_observer(false);
        mgr.create(req("a1", Some(true))).unwrap();

        assert_eq!(
            ctl.writes_utf8(),
            "claude\r",
            "hooks-OFF autostart must inject a bare `claude` with no --settings"
        );

        cleanup(&ctl, &dir);
    }

    // ---- Task B: zsh ZDOTDIR shim wiring ----

    /// Like `build()`, but with an overridden `shell_resolver` so the test
    /// doesn't depend on the host's actual `$SHELL`.
    fn build_with_shell_resolver(
        resolver: Arc<
            dyn Fn(Option<&str>, &[CommandWrapperSpec]) -> shells::ResolvedShell + Send + Sync,
        >,
    ) -> (
        Arc<SessionManager>,
        Arc<RecordingEvents>,
        Arc<FakeControl>,
        PathBuf,
    ) {
        let events = Arc::new(RecordingEvents::default());
        let reg = registry();
        let hub = hub_for(reg.clone(), events.clone() as Arc<dyn AppEvents>);
        let dir = scratch_observer_dir();
        let observer = claude_observer(hub.clone(), dir.clone());
        let (fac, ctl) = FakePtyFactory::new();
        let mgr = Arc::new(
            SessionManager::new(
                Arc::new(fac),
                observer,
                reg,
                events.clone() as Arc<dyn AppEvents>,
                hub,
                Arc::new(|| Some("http://127.0.0.1:12345/hook".into())),
            )
            .with_shell_resolver(resolver),
        );
        (mgr, events, ctl, dir)
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn create_pushes_zdotdir_env_when_shell_resolver_returns_zsh() {
        let shim_dir = std::env::temp_dir().join(format!(
            "agent-office-manager-zdotdir-test-{}",
            Uuid::new_v4(),
        ));
        let shim_dir_for_resolver = shim_dir.clone();
        let (mgr, _events, ctl, dir) =
            build_with_shell_resolver(Arc::new(move |_selected, wrappers| {
                let path = crate::session::zsh_wrapper::write_observer_shim(
                    &shim_dir_for_resolver,
                    wrappers,
                )
                .unwrap();
                shells::ResolvedShell {
                    program: "/bin/zsh".to_string(),
                    args: vec!["-l".to_string(), "-i".to_string()],
                    extra_env: vec![("ZDOTDIR".into(), path.to_string_lossy().into_owned())],
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
        let _ = std::fs::remove_dir_all(shim_dir);
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn create_does_not_push_zdotdir_env_for_non_zsh_shells() {
        let (mgr, _events, ctl, dir) =
            build_with_shell_resolver(Arc::new(|_selected, _wrappers| shells::ResolvedShell {
                program: "/bin/bash".to_string(),
                args: vec!["-l".to_string(), "-i".to_string()],
                extra_env: vec![],
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
        mgr.create(req_with_cwd("a1", Some("~/some/dir".into())))
            .unwrap();

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
        mgr.create(req_with_cwd("a1", Some("~someuser/dir".into())))
            .unwrap();

        assert_eq!(ctl.spawned_cwd(), "~someuser/dir");

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_passes_through_absolute_cwd_unchanged() {
        let (mgr, _events, ctl, dir) = build();
        mgr.create(req_with_cwd("a1", Some("/abs/path".into())))
            .unwrap();

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

        assert_eq!(
            mgr.session_id_for("a1"),
            None,
            "disposed agent must not resolve to a session"
        );
        let _ = created;

        cleanup(&ctl, &dir);
    }

    // ---- dispose -> Disposed, bookkeeping removed ----

    #[tokio::test]
    async fn dispose_kills_pty_and_on_exit_transitions_to_disposed_and_removes_bookkeeping() {
        let (mgr, events, ctl, dir) = build();
        let created = mgr.create(req("a1", Some(false))).unwrap();
        let settings = dir.join(format!("{}.settings.json", created.session_id));
        assert!(
            settings.exists(),
            "settings file should exist while running"
        );

        mgr.dispose("a1");
        assert_eq!(ctl.kill_count(), 1, "dispose must call PtyControl::kill");
        assert!(
            !settings.exists(),
            "dispose must remove observer cleanup paths"
        );

        ctl.fire_exit(0);
        wait_for(|| events.states().len() == 3).await;

        let last = events.last_state();
        assert_eq!(last.state, SessionState::Disposed);
        assert!(
            last.exit.as_ref().unwrap().intentional,
            "kill-triggered exit must be intentional"
        );

        assert_eq!(
            mgr.session_id_for("a1"),
            None,
            "agentId must be removed from the sessions map"
        );
        assert_eq!(
            mgr.registry.resolve_agent(&created.session_id),
            None,
            "Disposed session must be removed from the registry (E8: later hooks are discarded)"
        );
        assert!(
            !settings.exists(),
            "intentional exit cleanup must remain idempotent"
        );

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn dispose_all_kills_every_live_session() {
        let (mgr, events, ctl, dir) = build();
        mgr.create(req("a1", Some(false))).unwrap();

        mgr.dispose_all();

        assert_eq!(ctl.kill_count(), 1);
        ctl.fire_exit(0);
        wait_for(|| events.states().last() == Some(&SessionState::Disposed)).await;
        cleanup(&ctl, &dir);
    }

    // ---- 세션 핸드오프(docs/session-handoff-design.md) 회귀: handed_off ----
    //
    // 실제 UDS/sessiond 왕복은 sessiond::protocol/daemon/client 유닛 테스트가
    // 커버한다. 여기서는 핸드오프가 "성공했다고 치고" 세션에 handed_off를
    // 직접 세팅해(Fake에는 handoff/reader_interrupt가 애초에 없으므로
    // handoff_one 자체는 구동할 수 없다 -- private 필드에 직접 접근하는 이
    // 시뮬레이션이 설계 문서가 말하는 "Fake에 handoff 시뮬레이션 훅") 그
    // 이후 dispose_all/on_exit이 정말로 손을 떼는지만 검증한다.

    #[tokio::test]
    async fn handed_off_session_is_skipped_by_dispose_all_and_on_exit() {
        let (mgr, events, ctl, dir) = build();
        mgr.create(req("a1", Some(false))).unwrap();
        let states_before = events.states().len();

        {
            let sess = mgr.find("a1").expect("session must exist right after create");
            sess.handed_off.store(true, Ordering::SeqCst);
        }

        mgr.dispose_all();
        assert_eq!(
            ctl.kill_count(),
            0,
            "dispose_all must not kill a handed-off session"
        );
        assert!(
            std::fs::read_dir(&dir)
                .map(|mut entries| entries.next().is_some())
                .unwrap_or(false),
            "dispose_all must not remove a handed-off session's cleanup_paths"
        );

        // wait 스레드가 나중에 완주해도(on_exit 진입) 상태 전이가 없어야 한다.
        ctl.fire_exit(0);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            events.states().len(),
            states_before,
            "on_exit must not emit a state transition for a handed-off session"
        );

        // 세션은 맵에 그대로 남는다 -- 제거는 handoff_one의 성공 경로 책임이지
        // dispose_all/on_exit의 책임이 아니다.
        assert!(mgr.find("a1").is_some());

        ctl.close_output();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dispose_ignores_handed_off_session_directly() {
        let (mgr, _events, ctl, dir) = build();
        mgr.create(req("a1", Some(false))).unwrap();
        mgr.find("a1").unwrap().handed_off.store(true, Ordering::SeqCst);

        mgr.dispose("a1");

        assert_eq!(ctl.kill_count(), 0, "dispose() must skip a handed-off session");
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
        assert!(
            ctl.resize_calls().is_empty(),
            "resize after exit must be a no-op"
        );

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn write_input_and_resize_on_unknown_agent_do_not_panic() {
        let (mgr, _events, ctl, dir) = build();
        mgr.write_input("ghost", "x");
        mgr.resize("ghost", 1, 1);
        cleanup(&ctl, &dir);
    }

    // ---- 패닉 격리: 세션 계층은 한 번의 패닉으로 벽돌이 되면 안 된다 ----

    /// create()가 observer 설정 파일을 쓴 뒤 어떤 이유로든(스폰 내부 패닉 포함)
    /// 완주하지 못하면 파일이 정리돼야 한다.
    #[tokio::test]
    async fn create_cleans_up_observer_plan_even_when_spawn_panics() {
        let events = Arc::new(RecordingEvents::default());
        let reg = registry();
        let hub = hub_for(reg.clone(), events.clone() as Arc<dyn AppEvents>);
        let dir = scratch_observer_dir();
        let observer = claude_observer(hub.clone(), dir.clone());
        let mgr = Arc::new(SessionManager::new(
            Arc::new(crate::session::pty_factory::fake::PanickingPtyFactory),
            observer,
            reg,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| Some("http://127.0.0.1:12345/hook".into())),
        ));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            mgr.create(req("a1", Some(false)))
        }));
        assert!(
            result.is_err(),
            "spawn panic must propagate (converted at the command layer)"
        );

        let leftover = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
        assert_eq!(
            leftover, 0,
            "observer cleanup file must be removed on the panic/unwind path too"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 2026-07-11 실사용 "터미널 영구 고착" 재현(메커니즘 검증): 출력 채널
    /// 콜백이 패닉하면(웹뷰 측 전송 실패의 대역) 그 패닉이
    ///   pump(emit, channel 락 보유 중 패닉 → channel 뮤텍스 poison)
    ///   → detach_output(sinks 락 보유 중 channel.lock() unwrap 패닉 → sinks poison)
    ///   → 이후 모든 create()가 sink_for의 sinks.lock()에서 패닉
    /// 으로 전파되어, 훅 설정 파일만 쓰고(누적 잔존) 세션은 맵에 못 들어가며
    /// invoke는 영원히 미해결 — 앱 재시작 전까지 어떤 에이전트도 터미널을 못
    /// 띄우는 실사고 시그니처와 일치한다. 세션 계층은 채널 패닉 한 번에
    /// 오염되지 말아야 한다: 이후의 detach/create는 정상 동작해야 한다.
    #[tokio::test]
    async fn session_layer_survives_a_panicking_output_channel() {
        let events = Arc::new(RecordingEvents::default());
        let reg = registry();
        let hub = hub_for(reg.clone(), events.clone() as Arc<dyn AppEvents>);
        let dir = scratch_observer_dir();
        let observer = claude_observer(hub.clone(), dir.clone());
        let factory = Arc::new(MultiFakePtyFactory::new());
        let mgr = Arc::new(SessionManager::new(
            factory.clone(),
            observer,
            reg,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| Some("http://127.0.0.1:12345/hook".into())),
        ));

        // 패닉하는 채널을 먼저 attach — 첫 emit(ch.send)에서 pump가 죽는다.
        let bad: Channel<OutputChunk> = Channel::new(|_| panic!("simulated channel-send failure"));
        mgr.attach_output("a1", bad);

        mgr.create(req("a1", Some(false)))
            .expect("first create succeeds");
        let ctl1 = factory.controls()[0].clone();
        ctl1.push_output(b"trigger-pump-panic");

        // pump가 emit 중 패닉할 시간을 준다(16ms flush 윈도 + 여유).
        tokio::time::sleep(Duration::from_millis(200)).await;

        // 실사고 경로 그대로: 프론트의 unsubscribe_output → detach_output.
        // (수정 전: channel 뮤텍스 poison → 여기서 sinks 락 보유 중 패닉)
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| mgr.detach_output("a1")));

        // 재시작 시나리오: dispose 후 재생성. 세션 계층이 오염됐다면 여기서
        // 패닉(= invoke 영구 미해결 = 터미널 영구 고착)한다.
        mgr.dispose("a1");
        let second = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            mgr.create(req("a1", Some(false)))
        }));
        assert!(
            second.is_ok(),
            "create() must survive a prior channel panic — a single panicking \
             channel callback must never brick session creation for the rest of the app run"
        );
        second
            .unwrap()
            .expect("recreate after channel panic should return Ok");

        // 멀쩡한 채널로 재구독하면 새 세션 출력도 정상 수신돼야 한다.
        let (good, captured) = recording_channel();
        let reattach = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            mgr.attach_output("a1", good)
        }));
        assert!(
            reattach.is_ok(),
            "attach_output must survive after the cascade"
        );
        let ctl2 = factory.controls()[1].clone();
        ctl2.push_output(b"recovered-output");
        wait_for(|| captured.lock().contains("recovered-output")).await;

        ctl1.close_output();
        ctl2.close_output();
        let _ = std::fs::remove_dir_all(&dir);
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
                        sink_for_cb.lock().push_str(data);
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

        wait_for(|| captured.lock().contains("hello-pending")).await;

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
        let dir = scratch_observer_dir();
        let observer = claude_observer(hub.clone(), dir.clone());
        let factory = Arc::new(MultiFakePtyFactory::new());
        let mgr = Arc::new(SessionManager::new(
            factory.clone(),
            observer,
            reg,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| Some("http://127.0.0.1:12345/hook".into())),
        ));

        let (channel, captured) = recording_channel();
        mgr.attach_output("a1", channel); // subscribe once, before anything

        // First session.
        mgr.create(req("a1", Some(false))).unwrap();
        let ctl1 = factory.controls()[0].clone();
        ctl1.push_output(b"from-first;");
        wait_for(|| captured.lock().contains("from-first;")).await;

        // Unexpected exit -> Exited (session kept for restart).
        ctl1.fire_exit(1);
        wait_for(|| events.states().contains(&SessionState::Exited)).await;
        ctl1.close_output(); // let the first pump wind down

        // Recreate for the same agentId (a genuine 2nd spawn).
        mgr.create(req("a1", Some(false))).unwrap();
        let ctl2 = factory.controls()[1].clone();
        ctl2.push_output(b"from-second");

        // Same channel receives the new session's output — no re-attach.
        wait_for(|| captured.lock().contains("from-second")).await;

        ctl2.close_output();
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- 재시작 레이스: dispose 직후 create (PowerShell 회귀) ----

    /// Windows/PowerShell 재시작 회귀. 증상: 첫 재시작은 세션을 종료만 하고 새
    /// 세션을 못 띄워, 한 번 더 재시작해야 떴다. 원인: dispose가 kill을 요청해도
    /// 프로세스 reap(→ on_exit)이 느린 플랫폼에서는 create의 재사용 가드가 아직
    /// Running으로 남은 "죽어가는 세션"을 재사용해버렸다. dispose로 kill이 요청된
    /// 세션은 곧 사라질 예정이므로 재사용하지 말고 새 PTY를 띄워야 한다.
    #[tokio::test]
    async fn recreate_after_dispose_before_reap_spawns_fresh_session_not_reuse() {
        let events = Arc::new(RecordingEvents::default());
        let reg = registry();
        let hub = hub_for(reg.clone(), events.clone() as Arc<dyn AppEvents>);
        let dir = scratch_observer_dir();
        let observer = claude_observer(hub.clone(), dir.clone());
        let factory = Arc::new(MultiFakePtyFactory::new());
        let mgr = Arc::new(SessionManager::new(
            factory.clone(),
            observer,
            reg,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| Some("http://127.0.0.1:12345/hook".into())),
        ));

        let first = mgr.create(req("a1", Some(false))).unwrap();

        // dispose: kill 요청(kill_requested=true) — 단, fire_exit는 하지 않는다.
        // 즉 프로세스가 아직 reap되지 않아 on_exit이 실행되기 전 상태(세션은
        // 맵에 Running으로 남아 있음)를 재현한다.
        mgr.dispose("a1");

        let second = mgr.create(req("a1", Some(false))).unwrap();

        assert_ne!(
            first.session_id, second.session_id,
            "kill이 요청된(죽어가는) 세션을 재사용하면 안 된다 — 새 세션을 만들어야 한다"
        );
        assert_eq!(
            factory.controls().len(),
            2,
            "재시작 시 새 PTY가 spawn돼야 한다"
        );
        assert_eq!(
            mgr.session_id_for("a1"),
            Some(second.session_id.clone()),
            "agentId는 새 세션으로 resolve돼야 한다"
        );

        // cleanup: 두 세션 다 reap + 리더 종료.
        for c in factory.controls() {
            c.fire_exit(0);
            c.close_output();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 재시작 레이스의 반대 순서(macOS처럼 reap이 빠른 플랫폼): dispose 후
    /// on_exit(Disposed)이 다음 create보다 **먼저** 완주한 경우에도, agentId에
    /// 붙어 있던 출력 채널은 살아남아 새 세션의 출력을 받아야 한다.
    ///
    /// 2026-07-11 실사용 "터미널이 재시작해도 영구히 안 뜸" 근본 원인:
    /// on_exit(Disposed, is_current)가 맵 엔트리와 함께 **sink까지 제거**해
    /// 프론트가 attach해 둔 채널이 고아가 됐다. 이후 create는 채널 없는 새
    /// sink를 만들고, 프론트(사운드 매니저가 onData를 상시 구독해 재시작 중
    /// 재구독 IPC가 없음)는 끊긴 걸 모른 채 고아 sink에 붙어 있어 — 이후 몇
    /// 번을 재시작해도 터미널이 blank(앱 재시작 전까지). sink는 설계상
    /// "세션 수명과 독립"(agentId 키)이므로 세션 수명 이벤트인 on_exit이
    /// 지워서는 안 된다. (실 PTY 병렬 부하에서
    /// real_shell_restart_mash_never_wedges_and_never_leaks_hook_files로도 재현.)
    #[tokio::test]
    async fn restart_where_on_exit_wins_the_race_keeps_the_attached_channel() {
        let events = Arc::new(RecordingEvents::default());
        let reg = registry();
        let hub = hub_for(reg.clone(), events.clone() as Arc<dyn AppEvents>);
        let dir = scratch_observer_dir();
        let observer = claude_observer(hub.clone(), dir.clone());
        let factory = Arc::new(MultiFakePtyFactory::new());
        let mgr = Arc::new(SessionManager::new(
            factory.clone(),
            observer,
            reg,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| Some("http://127.0.0.1:12345/hook".into())),
        ));

        let (channel, captured) = recording_channel();
        mgr.attach_output("a1", channel); // 프론트: 부팅 시 1회 구독, 이후 재구독 없음

        mgr.create(req("a1", Some(false))).unwrap();
        let ctl1 = factory.controls()[0].clone();

        // 재시작 ①: dispose → (macOS: reap이 빨라) on_exit(Disposed)이 다음
        // create보다 먼저 완주한다.
        mgr.dispose("a1");
        ctl1.fire_exit(0);
        wait_for(|| events.states().contains(&SessionState::Disposed)).await;
        ctl1.close_output();

        // 재시작 ④: 새 세션 생성 — 처음 attach한 채널이 그대로 출력을 받아야 한다.
        mgr.create(req("a1", Some(false))).unwrap();
        let ctl2 = factory.controls()[1].clone();
        ctl2.push_output(b"after-fast-reap-restart");

        wait_for(|| captured.lock().contains("after-fast-reap-restart")).await;

        ctl2.close_output();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 위 재시작 레이스의 후속: 뒤늦게 옛 세션이 reap돼 on_exit이 돌아도, 이미
    /// 슬롯을 차지한 새 세션의 맵 엔트리·sink·출력 채널을 오염(evict)시키면 안 된다.
    /// (on_exit은 자신이 여전히 해당 agentId의 현재 세션일 때만 맵/sink/이벤트를
    /// 건드리는 identity 가드를 가진다.)
    #[tokio::test]
    async fn stale_on_exit_after_recreate_does_not_evict_replacement() {
        let events = Arc::new(RecordingEvents::default());
        let reg = registry();
        let hub = hub_for(reg.clone(), events.clone() as Arc<dyn AppEvents>);
        let dir = scratch_observer_dir();
        let observer = claude_observer(hub.clone(), dir.clone());
        let factory = Arc::new(MultiFakePtyFactory::new());
        let mgr = Arc::new(SessionManager::new(
            factory.clone(),
            observer,
            reg,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| Some("http://127.0.0.1:12345/hook".into())),
        ));

        let (channel, captured) = recording_channel();
        mgr.attach_output("a1", channel); // 세션 수명과 독립인 agentId 채널.

        let first = mgr.create(req("a1", Some(false))).unwrap();
        mgr.dispose("a1"); // kill 요청, 아직 미reap.
        let second = mgr.create(req("a1", Some(false))).unwrap();
        let ctl1 = factory.controls()[0].clone();
        let ctl2 = factory.controls()[1].clone();

        // 옛 세션 뒤늦게 reap → on_exit(옛)이 실행된다. Disposed 경로이므로
        // 레지스트리에서 옛 session_id가 제거되는 것을 on_exit 완료 신호로 쓴다.
        ctl1.fire_exit(0);
        wait_for(|| mgr.registry.resolve_agent(&first.session_id).is_none()).await;

        // on_exit(옛)이 새 세션을 evict하지 않았다.
        assert_eq!(
            mgr.session_id_for("a1"),
            Some(second.session_id.clone()),
            "교체된 옛 세션의 on_exit이 새 세션의 맵 엔트리를 지우면 안 된다"
        );
        // 그리고 새 세션의 출력이 여전히 같은 채널로 흐른다(sink가 제거되지 않았다).
        ctl2.push_output(b"after-restart");
        wait_for(|| captured.lock().contains("after-restart")).await;

        assert_ne!(first.session_id, second.session_id);

        ctl2.fire_exit(0);
        ctl1.close_output();
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
                        *s.state.lock() = SessionState::Exited;
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
        let dir = scratch_observer_dir();
        let observer = claude_observer(hub.clone(), dir.clone());
        let (fac, ctl) = FakePtyFactory::new();
        let mgr = Arc::new(SessionManager::new(
            Arc::new(fac),
            observer,
            reg,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| Some("http://127.0.0.1:12345/hook".into())),
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
            mgr.find("a1").map(|s| *s.state.lock()),
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
        assert!(
            settings.exists(),
            "settings file should exist while running"
        );

        ctl.fire_exit(1); // unexpected -> Exited
        wait_for(|| events.states().contains(&SessionState::Exited)).await;
        wait_for(|| !settings.exists()).await;

        assert!(
            !settings.exists(),
            "unexpected exit must clean up the settings file"
        );
        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn spawn_failure_cleans_up_the_settings_file_it_pre_wrote() {
        let events = Arc::new(RecordingEvents::default());
        let reg = registry();
        let hub = hub_for(reg.clone(), events.clone() as Arc<dyn AppEvents>);
        let dir = scratch_observer_dir();
        let observer = claude_observer(hub.clone(), dir.clone());
        let mgr = Arc::new(SessionManager::new(
            Arc::new(AlwaysFailPtyFactory),
            observer,
            reg,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| Some("http://127.0.0.1:12345/hook".into())),
        ));

        let result = mgr.create(req("a1", Some(false)));
        assert!(result.is_err(), "spawn must fail with AlwaysFailPtyFactory");
        assert!(
            events.session_starts().is_empty(),
            "a failed spawn must not emit session_started"
        );

        // The --settings file write() happens before spawn(); on spawn failure
        // it must be cleaned up, leaving no leftover in the hook dir.
        let leftovers = std::fs::read_dir(&dir).map(|rd| rd.count()).unwrap_or(0);
        assert_eq!(
            leftovers, 0,
            "spawn failure must not leak the pre-written settings file"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- shell selection: resolver receives selected id + wrapper specs, extra_env is spliced into spawn env ----

    /// What a recording resolver captured from its one call.
    struct RecordedResolverCall {
        selected: Option<String>,
        wrappers: Vec<String>,
    }

    /// Builds a `shell_resolver` that copies the selected id and wrapper names
    /// into `captured` (owned, so it outlives the borrowed inputs)
    /// and always resolves to a fixed, harmless `ResolvedShell` carrying
    /// `extra_env` so both concerns (request plumbing + env splicing) can be
    /// asserted from the same fixture.
    fn recording_resolver(
        captured: Arc<Mutex<Option<RecordedResolverCall>>>,
        extra_env: Vec<(String, String)>,
    ) -> Arc<dyn Fn(Option<&str>, &[CommandWrapperSpec]) -> shells::ResolvedShell + Send + Sync>
    {
        Arc::new(move |selected, wrappers| {
            *captured.lock() = Some(RecordedResolverCall {
                selected: selected.map(str::to_owned),
                wrappers: wrappers
                    .iter()
                    .map(|wrapper| wrapper.command.clone())
                    .collect(),
            });
            shells::ResolvedShell {
                program: "/bin/sh".to_string(),
                args: vec![],
                extra_env: extra_env.clone(),
            }
        })
    }

    /// Like `build_with_shell_resolver`, but lets the caller choose whether
    /// observation is enabled so wrapped/unwrapped variants share one fixture.
    fn build_with_shell_resolver_and_observation(
        resolver: Arc<
            dyn Fn(Option<&str>, &[CommandWrapperSpec]) -> shells::ResolvedShell + Send + Sync,
        >,
        enabled: bool,
    ) -> (
        Arc<SessionManager>,
        Arc<RecordingEvents>,
        Arc<FakeControl>,
        PathBuf,
    ) {
        let events = Arc::new(RecordingEvents::default());
        let reg = registry();
        let hub = hub_for(reg.clone(), events.clone() as Arc<dyn AppEvents>);
        let dir = scratch_observer_dir();
        let observer = claude_observer(hub.clone(), dir.clone());
        let (fac, ctl) = FakePtyFactory::new();
        let endpoint = enabled.then(|| "http://127.0.0.1:12345/hook".to_string());
        let mgr = Arc::new(
            SessionManager::new(
                Arc::new(fac),
                observer,
                reg,
                events.clone() as Arc<dyn AppEvents>,
                hub,
                Arc::new(move || endpoint.clone()),
            )
            .with_shell_resolver(resolver),
        );
        (mgr, events, ctl, dir)
    }

    #[tokio::test]
    async fn create_passes_selected_shell_and_observer_wrappers_to_resolver() {
        let captured = Arc::new(Mutex::new(None));
        let resolver = recording_resolver(captured.clone(), vec![]);
        let (mgr, _events, ctl, dir) = build_with_shell_resolver(resolver);

        mgr.create(req_with_shell("a1", Some("git-bash".to_string())))
            .unwrap();

        let rec = captured.lock();
        let rec = rec.as_ref().expect("resolver must have been called");
        assert_eq!(rec.selected.as_deref(), Some("git-bash"));
        assert_eq!(rec.wrappers, vec!["claude", "pi"]);

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_passes_no_wrappers_to_resolver_when_observer_disabled() {
        let captured = Arc::new(Mutex::new(None));
        let resolver = recording_resolver(captured.clone(), vec![]);
        let (mgr, _events, ctl, dir) = build_with_shell_resolver_and_observation(resolver, false);

        mgr.create(req_with_shell("a1", Some("git-bash".to_string())))
            .unwrap();

        let rec = captured.lock();
        let rec = rec.as_ref().expect("resolver must have been called");
        assert_eq!(rec.selected.as_deref(), Some("git-bash"));
        assert!(rec.wrappers.is_empty());

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_pushes_pi_ext_env_when_hooks_on() {
        // hooks ON(기본 port Some) 세션은 AGENT_OFFICE_PI_EXT를 spawn env에 실어야
        // 한다 — `pi()` 셸 래퍼가 이 경로를 -e로 로드하는 신호.
        let captured = Arc::new(Mutex::new(None));
        let resolver = recording_resolver(captured, vec![]);
        let (mgr, _events, ctl, dir) = build_with_shell_resolver(resolver);

        mgr.create(req("a1", None)).unwrap();

        let env = ctl.spawned_env();
        let pair = env.iter().find(|(k, _)| k == "AGENT_OFFICE_PI_EXT");
        let (_, val) = pair.expect("AGENT_OFFICE_PI_EXT must be injected when hooks are ON");
        assert!(
            val.ends_with("agent-office-pi.ts"),
            "AGENT_OFFICE_PI_EXT must point at the extension file, got: {val}"
        );

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_does_not_push_pi_ext_env_when_hooks_off() {
        // observer OFF 세션은 AGENT_OFFICE_PI_EXT가 없어야 한다.
        let captured = Arc::new(Mutex::new(None));
        let resolver = recording_resolver(captured, vec![]);
        let (mgr, _events, ctl, dir) = build_with_shell_resolver_and_observation(resolver, false);

        mgr.create(req("a1", None)).unwrap();

        let env = ctl.spawned_env();
        assert!(
            !env.iter().any(|(k, _)| k == "AGENT_OFFICE_PI_EXT"),
            "AGENT_OFFICE_PI_EXT must NOT be injected when hooks are OFF: {env:?}"
        );

        cleanup(&ctl, &dir);
    }

    #[tokio::test]
    async fn create_appends_resolved_extra_env_to_spawn_env() {
        let captured = Arc::new(Mutex::new(None));
        let marker = (
            "AGENT_OFFICE_TEST_MARKER".to_string(),
            "shell-extra-env".to_string(),
        );
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
// pieces -- AppEvents/observer server/app handle -- are swapped for local
// doubles; PortablePtyFactory + SessionManager + ObserverRuntime are the
// real production types). Deliberately `#[ignore]`d: shell startup time and
// `$SHELL` quirks make this env-dependent and too slow/flaky for the default
// `cargo test` run. Run explicitly:
//   cargo test --manifest-path src-tauri/Cargo.toml -- --ignored real_shell
//
// This lives inside `manager.rs` (rather than `src-tauri/tests/`) because
// `mod session`/`mod state`/`mod notification` are private in `lib.rs` --
// an external integration test crate can't name `SessionManager`,
// `ObserverRuntime`, or `state::fake::RecordingEvents` at all. Widening
// those to `pub`/`pub(crate)` just for this one smoke would be a bigger
// surface change than necessary, so the smoke rides along as a sibling
// `#[cfg(test)]` module instead, reusing the same private items the
// `tests` module above already does via `use super::*`.
#[cfg(test)]
mod real_pty_smoke {
    use super::*;
    use crate::notification::hub::{NotificationHub, SystemClock};
    use crate::observer::claude::ClaudeAdapter;
    use crate::observer::server::ObserverServerState;
    use crate::observer::ObserverRuntime;
    use crate::session::pty_factory::PortablePtyFactory;
    use crate::state::fake::RecordingEvents;
    use std::path::{Path, PathBuf};
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

    #[cfg(windows)]
    fn observer_path_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    #[cfg(windows)]
    struct ObserverEnvGuard {
        saved: Vec<(std::ffi::OsString, Option<std::ffi::OsString>)>,
    }

    #[cfg(windows)]
    impl ObserverEnvGuard {
        fn set(values: &[(&str, std::ffi::OsString)]) -> Self {
            let mut saved = Vec::with_capacity(values.len());
            for (key, value) in values {
                saved.push(((*key).into(), std::env::var_os(key)));
                std::env::set_var(key, value);
            }
            Self { saved }
        }
    }

    #[cfg(windows)]
    impl Drop for ObserverEnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..).rev() {
                match value {
                    Some(value) => std::env::set_var(&key, value),
                    None => std::env::remove_var(&key),
                }
            }
        }
    }

    #[cfg(windows)]
    fn write_observer_fake_clis(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("codex.ps1"),
            r#"
[IO.File]::WriteAllLines($env:AO_FAKE_CODEX_ARGS, [string[]]$args)
[IO.File]::WriteAllText($env:AO_FAKE_CODEX_PID, "$PID")
if ($args -contains 'bypass-marker') {
    [IO.File]::WriteAllText($env:AO_FAKE_BYPASS, 'bypassed')
    return
}
$payloads = @(
    '{"hook_event_name":"UserPromptSubmit","prompt":"codex-marker","session_id":"native-codex"}',
    '{"hook_event_name":"PostToolUse","session_id":"native-codex"}',
    '{"hook_event_name":"PermissionRequest","tool_input":{"description":"codex-attention"},"session_id":"native-codex"}',
    '{"hook_event_name":"Stop","last_assistant_message":"must-not-surface","session_id":"native-codex"}',
    '{"hook_event_name":"SubagentStart","session_id":"native-codex"}',
    '{"hook_event_name":"SubagentStop","session_id":"native-codex"}'
)
foreach ($payload in $payloads) {
    $payload | & $env:AO_FAKE_FORWARDER --observer-forward codex
    if ($LASTEXITCODE -ne 0) { throw "forwarder failed: $LASTEXITCODE" }
}
return
"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("claude.ps1"),
            r#"
[IO.File]::WriteAllLines($env:AO_FAKE_CLAUDE_ARGS, [string[]]$args)
[IO.File]::WriteAllText($env:AO_FAKE_CLAUDE_PID, "$PID")
$settingsPath = $null
for ($i = 0; $i -lt ($args.Count - 1); $i++) {
    if ($args[$i] -eq '--settings') { $settingsPath = $args[$i + 1]; break }
}
if (-not $settingsPath) { throw 'missing --settings path' }
$settings = Get-Content -LiteralPath $settingsPath -Raw | ConvertFrom-Json
$events = @(
    [pscustomobject]@{ Name = 'UserPromptSubmit'; Body = '{"prompt":"claude-marker","session_id":"native-claude"}' },
    [pscustomobject]@{ Name = 'PostToolUse'; Body = '{"session_id":"native-claude"}' },
    [pscustomobject]@{ Name = 'Notification'; Body = '{"message":"claude-attention","session_id":"native-claude"}' },
    [pscustomobject]@{ Name = 'Stop'; Body = '{"message":"claude-stop","session_id":"native-claude"}' },
    [pscustomobject]@{ Name = 'SubagentStart'; Body = '{"session_id":"native-claude"}' },
    [pscustomobject]@{ Name = 'SubagentStop'; Body = '{"session_id":"native-claude"}' }
)
foreach ($event in $events) {
    $group = $settings.hooks.PSObject.Properties[$event.Name].Value
    $command = $group[0].hooks[0].command
    $event.Body | & cmd.exe /d /s /c $command
    if ($LASTEXITCODE -ne 0) { throw "hook command failed: $LASTEXITCODE" }
}
return
"#,
        )
        .unwrap();
    }

    #[cfg(windows)]
    fn decode_observer_powershell_command(args: &[String]) -> Option<String> {
        use base64::Engine;

        let encoded = args
            .windows(2)
            .find(|pair| pair[0] == "-EncodedCommand")?
            .get(1)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()?;
        let utf16 = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        String::from_utf16(&utf16).ok()
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

        let observer_dir = scratch_dir("observer");
        let observer = Arc::new(ObserverRuntime::new(
            hub.clone(),
            vec![Arc::new(ClaudeAdapter::new(
                observer_dir.clone(),
                std::env::current_exe().unwrap(),
            ))],
        ));

        let cwd_dir = scratch_dir("cwd");
        std::fs::create_dir_all(&cwd_dir).expect("create scratch cwd");

        let mgr = Arc::new(SessionManager::new(
            Arc::new(PortablePtyFactory),
            observer,
            registry,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| Some("http://127.0.0.1:45999/hook".into())),
        ));

        let created = mgr
            .create(CreateSessionRequest {
                agent_id: "smoke".into(),
                cols: Some(80),
                rows: Some(24),
                cwd: Some(cwd_dir.to_string_lossy().into_owned()),
                shell: None,
                startup_command: None,
                personality_prompt: None,
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
                        output_for_channel.lock().push_str(data);
                    }
                }
            }
            Ok(())
        });
        mgr.attach_output("smoke", channel);

        // 1) Real shell prompt bytes must arrive within 5s, and state must
        //    have gone Starting -> Running.
        wait_for_timeout(
            || !output.lock().is_empty(),
            Duration::from_secs(5),
            "no output arrived from the real shell within 5s -- check $SHELL / login-shell startup time",
        )
        .await;
        assert_eq!(
            events.states().first().copied(),
            Some(SessionState::Starting)
        );
        assert!(events.states().contains(&SessionState::Running));

        // 2) Echo round-trip through real stdin -> shell -> stdout.
        mgr.write_input("smoke", "echo smoke-ok-12345\n");
        wait_for_timeout(
            || output.lock().contains("smoke-ok-12345"),
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

        let _ = std::fs::remove_dir_all(&observer_dir);
        let _ = std::fs::remove_dir_all(&cwd_dir);
    }

    /// 실 PTY + 실 sessiond 프로세스로 handoff_all -> adopt_detached 왕복
    /// 전체를 검증한다(docs/session-handoff-design.md §핵심 3, 4). 데몬을
    /// `client::connect_or_spawn`의 스폰 경로에 맡기지 않고 미리 띄워 둔다
    /// -- `cargo test` 바이너리는 `--sessiond` 분기가 없는 별개의 실행
    /// 파일이라 `spawn_daemon`(현재 실행 파일 재실행)을 여기서 구동할 수
    /// 없다(그 경로 자체는 client.rs 유닛 테스트 + 수동 검증 항목이 커버).
    /// 데몬이 이미 떠 있으면 `connect_or_spawn`의 첫 connect가 곧바로
    /// 성공하므로, 이 테스트는 그 뒤의 실제 핸드오프/입양 배선(리더 인터럽트,
    /// fd 전달, install_session 재조립)만 순수하게 검증한다.
    ///
    /// 실증에서 발견된 빈틈(데몬은 핸드오프 *이후* 출력만 보관 -- 종료 전
    /// 화면은 스냅샷 없이는 사라진다) 회귀도 여기서 함께 검증한다: 세션
    /// "a1"은 snapshots 맵에 명시적 스냅샷을 실어 보내고(그 텍스트가
    /// initial_output의 맨 앞에 와야 한다), 세션 "a2"는 맵에서 빠뜨려
    /// backlog 폴백 경로를 태운다(핸드오프 전 출력을 한 번도 구독하지
    /// 않았을 때도 최소한 backlog 분량은 보존돼야 한다).
    #[cfg(unix)]
    #[tokio::test]
    async fn handoff_all_then_adopt_detached_round_trips_a_real_session() {
        use crate::session::pty_factory::PortablePtyFactory;
        use std::collections::{HashMap, HashSet};

        let app_data_dir = scratch_dir("appdata");
        std::fs::create_dir_all(&app_data_dir).expect("create scratch app_data_dir");
        let socket_path = crate::sessiond::client::default_socket_path(&app_data_dir);

        let (shutdown_tx, _shutdown_rx) = std::sync::mpsc::channel::<()>();
        let hook: crate::sessiond::daemon::ShutdownHook = Arc::new(move || {
            let _ = shutdown_tx.send(());
        });
        let daemon_socket = socket_path.clone();
        std::thread::spawn(move || {
            let _ = crate::sessiond::daemon::run_daemon_inner(
                daemon_socket,
                Duration::from_secs(60),
                hook,
            );
        });
        wait_for_timeout(
            || socket_path.exists(),
            Duration::from_secs(2),
            "sessiond never bound its socket",
        )
        .await;

        let events1 = Arc::new(RecordingEvents::default());
        let registry1 = Arc::new(SessionRegistry::new());
        let hub1 = Arc::new(NotificationHub::new(
            registry1.clone(),
            events1.clone() as Arc<dyn AppEvents>,
            Arc::new(SystemClock),
            Duration::from_millis(3000),
        ));
        let observer1 = Arc::new(ObserverRuntime::new(hub1.clone(), vec![]));
        let mgr1 = Arc::new(
            SessionManager::new(
                Arc::new(PortablePtyFactory),
                observer1,
                registry1,
                events1.clone() as Arc<dyn AppEvents>,
                hub1,
                Arc::new(|| None), // observer off -- 실 PTY 핸드오프 배선만 검증하면 충분
            )
            .with_shell_resolver(Arc::new(|_, _| shells::ResolvedShell {
                program: "/bin/sh".into(),
                args: vec![],
                extra_env: vec![],
            }))
            .with_app_data_dir(app_data_dir.clone()),
        );

        let created = mgr1
            .create(CreateSessionRequest {
                agent_id: "a1".into(),
                cols: Some(80),
                rows: Some(24),
                cwd: None,
                shell: None,
                startup_command: None,
                personality_prompt: None,
                autostart_claude: Some(false),
            })
            .expect("real PTY spawn should succeed");
        assert_eq!(created.state, SessionState::Running);

        // "a2": 출력 채널을 한 번도 구독하지 않은 채 핸드오프한다 -- 스냅샷
        // 폴백(backlog)이 실제로 쓰이는지 검증하기 위한 세션. 핸드오프 전에
        // echo를 흘려보내 backlog에 쌓아 둔다(구독이 없으니 emit()이 채널
        // 대신 backlog로 간다).
        mgr1.create(CreateSessionRequest {
            agent_id: "a2".into(),
            cols: Some(80),
            rows: Some(24),
            cwd: None,
            shell: None,
            startup_command: None,
            personality_prompt: None,
            autostart_claude: Some(false),
        })
        .expect("real PTY spawn should succeed for a2");
        mgr1.write_input("a2", "echo backlog-marker-24680\n");
        wait_for_timeout(
            || {
                mgr1.sink_for("a2")
                    .backlog_snapshot()
                    .windows(b"backlog-marker-24680".len())
                    .any(|w| w == b"backlog-marker-24680")
            },
            Duration::from_secs(5),
            "a2's pre-handoff echo never landed in the sink backlog",
        )
        .await;

        let mut snapshots = HashMap::new();
        snapshots.insert("a1".to_string(), "SNAPSHOT-MARKER-13579\r\n".to_string());
        // "a2"는 의도적으로 생략 -- 백로그 폴백 경로를 태운다.

        let handed = mgr1.handoff_all(&snapshots);
        assert_eq!(handed, 2, "both running real sessions must be handed off");
        assert!(
            mgr1.find("a1").is_none(),
            "a successfully handed-off session must leave the manager's map"
        );
        assert!(mgr1.find("a2").is_none());

        // "재시작": 새 매니저가 같은 app_data_dir/소켓을 상대로 되찾는다.
        let events2 = Arc::new(RecordingEvents::default());
        let registry2 = Arc::new(SessionRegistry::new());
        let hub2 = Arc::new(NotificationHub::new(
            registry2.clone(),
            events2.clone() as Arc<dyn AppEvents>,
            Arc::new(SystemClock),
            Duration::from_millis(3000),
        ));
        let observer2 = Arc::new(ObserverRuntime::new(hub2.clone(), vec![]));
        let mgr2 = Arc::new(
            SessionManager::new(
                Arc::new(PortablePtyFactory),
                observer2,
                registry2,
                events2.clone() as Arc<dyn AppEvents>,
                hub2,
                Arc::new(|| None),
            )
            .with_app_data_dir(app_data_dir.clone()),
        );

        let known: HashSet<String> = ["a1".to_string(), "a2".to_string()].into_iter().collect();
        let adopted = mgr2.adopt_detached(&known);
        assert_eq!(adopted.len(), 2);
        let adopted_ids: HashSet<String> = adopted.iter().map(|a| a.agent_id.clone()).collect();
        assert_eq!(adopted_ids, known);
        assert_eq!(mgr2.session_id_for("a1"), Some(created.session_id.clone()));
        assert!(events2.states().contains(&SessionState::Running));

        // 이어받은 세션들이 실제로 살아 있는지: echo 왕복으로 확인 + 스냅샷이
        // initial_output 맨 앞에 왔는지 검증.
        fn attach_collector(mgr: &Arc<SessionManager>, agent_id: &str) -> Arc<Mutex<String>> {
            let output = Arc::new(Mutex::new(String::new()));
            let output_for_channel = output.clone();
            let channel: Channel<OutputChunk> = Channel::new(move |body| {
                if let InvokeResponseBody::Json(s) = body {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                        if let Some(data) = v.get("data").and_then(|d| d.as_str()) {
                            output_for_channel.lock().push_str(data);
                        }
                    }
                }
                Ok(())
            });
            mgr.attach_output(agent_id, channel);
            output
        }

        let output_a1 = attach_collector(&mgr2, "a1");
        let output_a2 = attach_collector(&mgr2, "a2");
        mgr2.write_input("a1", "echo adopted-and-alive-98765\n");
        wait_for_timeout(
            || output_a1.lock().contains("adopted-and-alive-98765"),
            Duration::from_secs(5),
            "adopted a1 never echoed the write_input marker",
        )
        .await;
        assert!(
            output_a1.lock().starts_with("SNAPSHOT-MARKER-13579"),
            "a1's explicit snapshot must be replayed before any post-adopt output: {:?}",
            output_a1.lock()
        );

        // a2는 명시적 스냅샷이 없었으니 backlog 폴백으로 핸드오프 전 echo가
        // 보존돼야 한다(드레인되지 않고 복사만 됐어야 하므로 mgr1 쪽
        // backlog에도 영향이 없다 -- 여기서는 재입양된 mgr2 쪽만 확인).
        wait_for_timeout(
            || output_a2.lock().contains("backlog-marker-24680"),
            Duration::from_secs(5),
            "adopted a2 never replayed the backlog-fallback snapshot",
        )
        .await;

        mgr2.dispose("a1");
        mgr2.dispose("a2");
        wait_for_timeout(
            || {
                events2.states().iter().filter(|s| **s == SessionState::Disposed).count() == 2
            },
            Duration::from_secs(5),
            "both adopted sessions never reached Disposed within 5s after dispose()",
        )
        .await;

        let _ = std::fs::remove_dir_all(&app_data_dir);
    }

    /// v2 브로커 모드 라운드트립: 실 sessiond(run_daemon_inner 스레드)와
    /// `BrokerPtyFactory`로 create(=Spawn) → write/read(echo) → detach(handoff_all,
    /// 자식 안 죽임) → 새 매니저로 adopt(브로커 경로) → 출력 연속성 확인 →
    /// dispose(=Kill RPC)로 정리까지. v1 핸드오프 테스트의 브로커판이다.
    #[cfg(unix)]
    #[tokio::test]
    async fn broker_spawn_write_read_detach_adopt_round_trips_a_real_session() {
        use crate::session::broker_pty::BrokerPtyFactory;
        use crate::session::pty_factory::PortablePtyFactory;
        use std::collections::HashSet;
        use tauri::ipc::{Channel, InvokeResponseBody};

        let app_data_dir = scratch_dir("broker-appdata");
        std::fs::create_dir_all(&app_data_dir).expect("create scratch app_data_dir");
        let socket_path = crate::sessiond::client::default_socket_path(&app_data_dir);

        // 실 데몬을 스레드로 띄운다(테스트 바이너리는 --sessiond를 모르므로
        // connect_or_spawn의 자기재실행 대신 여기서 직접 구동한다 -- 팩토리의
        // connect_or_spawn은 이미 떠 있는 이 데몬에 그냥 connect한다).
        let hook: crate::sessiond::daemon::ShutdownHook = Arc::new(|| {});
        let daemon_socket = socket_path.clone();
        std::thread::spawn(move || {
            let _ = crate::sessiond::daemon::run_daemon_inner(
                daemon_socket,
                Duration::from_secs(60),
                hook,
            );
        });
        wait_for_timeout(
            || socket_path.exists(),
            Duration::from_secs(2),
            "sessiond never bound its socket",
        )
        .await;

        fn build_broker_manager(app_data_dir: &Path) -> (Arc<SessionManager>, Arc<RecordingEvents>) {
            let events = Arc::new(RecordingEvents::default());
            let registry = Arc::new(SessionRegistry::new());
            let hub = Arc::new(NotificationHub::new(
                registry.clone(),
                events.clone() as Arc<dyn AppEvents>,
                Arc::new(SystemClock),
                Duration::from_millis(3000),
            ));
            let observer = Arc::new(ObserverRuntime::new(hub.clone(), vec![]));
            let fallback: Arc<dyn crate::session::pty_factory::PtyFactory> =
                Arc::new(PortablePtyFactory);
            let factory = Arc::new(BrokerPtyFactory::new(app_data_dir, fallback));
            let mgr = Arc::new(
                SessionManager::new(
                    factory,
                    observer,
                    registry,
                    events.clone() as Arc<dyn AppEvents>,
                    hub,
                    Arc::new(|| None),
                )
                .with_shell_resolver(Arc::new(|_, _| shells::ResolvedShell {
                    program: "/bin/sh".into(),
                    args: vec![],
                    extra_env: vec![],
                }))
                .with_app_data_dir(app_data_dir.to_path_buf())
                .with_broker_mode(true),
            );
            (mgr, events)
        }

        fn attach_collector(mgr: &Arc<SessionManager>, agent_id: &str) -> Arc<Mutex<String>> {
            let output = Arc::new(Mutex::new(String::new()));
            let output_for_channel = output.clone();
            let channel: Channel<OutputChunk> = Channel::new(move |body| {
                if let InvokeResponseBody::Json(s) = body {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                        if let Some(data) = v.get("data").and_then(|d| d.as_str()) {
                            output_for_channel.lock().push_str(data);
                        }
                    }
                }
                Ok(())
            });
            mgr.attach_output(agent_id, channel);
            output
        }

        let (mgr1, _events1) = build_broker_manager(&app_data_dir);
        let created = mgr1
            .create(CreateSessionRequest {
                agent_id: "a1".into(),
                cols: Some(80),
                rows: Some(24),
                cwd: None,
                shell: None,
                startup_command: None,
                personality_prompt: None,
                autostart_claude: Some(false),
            })
            .expect("broker spawn should succeed");
        assert_eq!(created.state, SessionState::Running);

        // write -> read(echo) 왕복.
        let out1 = attach_collector(&mgr1, "a1");
        mgr1.write_input("a1", "echo broker-alive-11111\n");
        wait_for_timeout(
            || out1.lock().contains("broker-alive-11111"),
            Duration::from_secs(5),
            "broker session never echoed the write_input marker",
        )
        .await;

        // detach(유지하고 종료): 자식은 안 죽고 맵에서만 빠진다.
        let handed = mgr1.handoff_all(&std::collections::HashMap::new());
        assert_eq!(handed, 1, "the running broker session must be detached");
        assert!(mgr1.find("a1").is_none(), "detached session must leave the map");
        drop(mgr1); // 앱 "종료" -- data/control/wait 연결이 끊긴다(자식은 데몬 소유라 생존).

        // "재시작": 새 매니저가 브로커 경로로 되찾는다.
        let (mgr2, events2) = build_broker_manager(&app_data_dir);
        let known: HashSet<String> = ["a1".to_string()].into_iter().collect();
        let adopted = mgr2.adopt_detached(&known);
        assert_eq!(adopted.len(), 1, "the detached broker session must be adopted");
        assert_eq!(mgr2.session_id_for("a1"), Some(created.session_id.clone()));
        assert!(events2.states().contains(&SessionState::Running));

        // 출력 연속성: data 연결 백로그 리플레이로 detach 전 echo가 다시 보이고,
        // 새 write도 살아서 왕복한다.
        let out2 = attach_collector(&mgr2, "a1");
        wait_for_timeout(
            || out2.lock().contains("broker-alive-11111"),
            Duration::from_secs(5),
            "adopted broker session never replayed the pre-detach backlog",
        )
        .await;
        mgr2.write_input("a1", "echo broker-still-alive-22222\n");
        wait_for_timeout(
            || out2.lock().contains("broker-still-alive-22222"),
            Duration::from_secs(5),
            "adopted broker session is not live after adopt",
        )
        .await;

        // dispose(=Kill RPC): 실제 exit code를 관측하는 BrokerWaiter가 Disposed로 전이.
        mgr2.dispose("a1");
        wait_for_timeout(
            || events2.states().contains(&SessionState::Disposed),
            Duration::from_secs(5),
            "disposed broker session never reached Disposed",
        )
        .await;

        let _ = std::fs::remove_dir_all(&app_data_dir);
    }

    /// §#50: 다중 인스턴스 안전화. 첫 매니저(mgr1)가 라이브로 붙어 있는(attached)
    /// 브로커 세션을 둘째 매니저(mgr2, = 2번째 앱 인스턴스)의 adopt가 가로채지
    /// 않고 스킵하는지, 그리고 mgr1이 detach(handoff_all -> data 소켓 결정적
    /// shutdown)하면 conn이 정리돼 attached=false가 되어 mgr2가 그제서야 정상
    /// 입양하는지(§P0 결정적 reader-close 의존)를 실 데몬으로 검증한다.
    #[cfg(unix)]
    #[tokio::test]
    async fn broker_adopt_skips_live_attached_session_then_adopts_after_detach() {
        use crate::session::broker_pty::BrokerPtyFactory;
        use crate::session::pty_factory::PortablePtyFactory;
        use std::collections::HashSet;
        use tauri::ipc::{Channel, InvokeResponseBody};

        let app_data_dir = scratch_dir("broker-hijack-appdata");
        std::fs::create_dir_all(&app_data_dir).expect("create scratch app_data_dir");
        let socket_path = crate::sessiond::client::default_socket_path(&app_data_dir);

        let hook: crate::sessiond::daemon::ShutdownHook = Arc::new(|| {});
        let daemon_socket = socket_path.clone();
        std::thread::spawn(move || {
            let _ = crate::sessiond::daemon::run_daemon_inner(
                daemon_socket,
                Duration::from_secs(60),
                hook,
            );
        });
        wait_for_timeout(
            || socket_path.exists(),
            Duration::from_secs(2),
            "sessiond never bound its socket",
        )
        .await;

        fn build_broker_manager(app_data_dir: &Path) -> Arc<SessionManager> {
            let events = Arc::new(RecordingEvents::default());
            let registry = Arc::new(SessionRegistry::new());
            let hub = Arc::new(NotificationHub::new(
                registry.clone(),
                events.clone() as Arc<dyn AppEvents>,
                Arc::new(SystemClock),
                Duration::from_millis(3000),
            ));
            let observer = Arc::new(ObserverRuntime::new(hub.clone(), vec![]));
            let fallback: Arc<dyn crate::session::pty_factory::PtyFactory> =
                Arc::new(PortablePtyFactory);
            let factory = Arc::new(BrokerPtyFactory::new(app_data_dir, fallback));
            Arc::new(
                SessionManager::new(
                    factory,
                    observer,
                    registry,
                    events.clone() as Arc<dyn AppEvents>,
                    hub,
                    Arc::new(|| None),
                )
                .with_shell_resolver(Arc::new(|_, _| shells::ResolvedShell {
                    program: "/bin/sh".into(),
                    args: vec![],
                    extra_env: vec![],
                }))
                .with_app_data_dir(app_data_dir.to_path_buf())
                .with_broker_mode(true),
            )
        }

        fn attach_collector(mgr: &Arc<SessionManager>, agent_id: &str) -> Arc<Mutex<String>> {
            let output = Arc::new(Mutex::new(String::new()));
            let output_for_channel = output.clone();
            let channel: Channel<OutputChunk> = Channel::new(move |body| {
                if let InvokeResponseBody::Json(s) = body {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                        if let Some(data) = v.get("data").and_then(|d| d.as_str()) {
                            output_for_channel.lock().push_str(data);
                        }
                    }
                }
                Ok(())
            });
            mgr.attach_output(agent_id, channel);
            output
        }

        let known: HashSet<String> = ["a1".to_string()].into_iter().collect();

        // mgr1: 세션 생성 후 라이브로 붙어 있다(attached=true).
        let mgr1 = build_broker_manager(&app_data_dir);
        mgr1.create(CreateSessionRequest {
            agent_id: "a1".into(),
            cols: Some(80),
            rows: Some(24),
            cwd: None,
            shell: None,
            startup_command: None,
            personality_prompt: None,
            autostart_claude: Some(false),
        })
        .expect("broker spawn should succeed");
        let out1 = attach_collector(&mgr1, "a1");
        mgr1.write_input("a1", "echo mgr1-alive-11111\n");
        wait_for_timeout(
            || out1.lock().contains("mgr1-alive-11111"),
            Duration::from_secs(5),
            "mgr1 broker session never echoed",
        )
        .await;

        // mgr2(= 2번째 인스턴스): mgr1이 살아 붙어 있는 세션을 입양하려 하면
        // attached=true라 스킵해야 한다(하이재킹 금지).
        let mgr2 = build_broker_manager(&app_data_dir);
        let adopted = mgr2.adopt_detached(&known);
        assert!(
            adopted.is_empty(),
            "must NOT adopt a session attached by a live instance (hijack): {adopted:?}"
        );
        assert!(mgr2.find("a1").is_none(), "skipped session must not enter mgr2");

        // mgr1은 여전히 살아 동작한다(가로채기가 없었으므로 스트림 유지).
        mgr1.write_input("a1", "echo mgr1-still-alive-22222\n");
        wait_for_timeout(
            || out1.lock().contains("mgr1-still-alive-22222"),
            Duration::from_secs(5),
            "mgr1 must stay live after mgr2's skipped adopt",
        )
        .await;

        // mgr1 detach: data 소켓을 결정적 shutdown -> 데몬 conn 정리 -> attached=false.
        let handed = mgr1.handoff_all(&std::collections::HashMap::new());
        assert_eq!(handed, 1, "mgr1 must detach its broker session");
        assert!(mgr1.find("a1").is_none());

        // 이제 mgr2가 입양 가능해야 한다(P0 결정적 close로 attached가 풀렸다).
        // 데몬이 FIN을 관측할 짧은 시간을 주며 재시도.
        let mut adopted2 = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while adopted2.is_empty() && std::time::Instant::now() < deadline {
            adopted2 = mgr2.adopt_detached(&known);
            if adopted2.is_empty() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        assert_eq!(
            adopted2.len(),
            1,
            "after mgr1 detached, mgr2 must adopt the now-unattached session"
        );
        let out2 = attach_collector(&mgr2, "a1");
        wait_for_timeout(
            || out2.lock().contains("mgr1-alive-11111"),
            Duration::from_secs(5),
            "adopted session must replay pre-detach backlog",
        )
        .await;

        mgr2.dispose("a1");
        let _ = std::fs::remove_dir_all(&app_data_dir);
    }

    /// §P1-a 혼합 상황: 브로커 모드 매니저에 브로커 세션(broker_owned) 1개 +
    /// 팩토리 폴백으로 만든 in-process 세션 1개가 섞여 있을 때, handoff_all이
    /// 세션 단위로 경로를 갈라 -- 브로커는 스냅샷 업로드+detach, 폴백은 v1 fd
    /// 핸드오프 -- 둘 다 데몬에 남기고, 새 매니저 adopt가 둘 다 복구하는지.
    #[cfg(unix)]
    #[tokio::test]
    async fn broker_mode_handoff_mixes_broker_detach_and_v1_fd_handoff() {
        use crate::session::broker_pty::BrokerPtyFactory;
        use crate::session::pty_factory::{PortablePtyFactory, PtyFactory, PtySpawnOptions, SpawnedPty};
        use std::collections::HashSet;
        use tauri::ipc::{Channel, InvokeResponseBody};

        // agent_id가 "f"로 시작하면 폴백(PortablePtyFactory), 아니면 브로커.
        struct MixedFactory {
            broker: BrokerPtyFactory,
            portable: PortablePtyFactory,
        }
        impl PtyFactory for MixedFactory {
            fn spawn(&self, o: PtySpawnOptions) -> std::io::Result<SpawnedPty> {
                if o.agent_id.starts_with('f') {
                    self.portable.spawn(o)
                } else {
                    self.broker.spawn(o)
                }
            }
        }

        let app_data_dir = scratch_dir("mixed-appdata");
        std::fs::create_dir_all(&app_data_dir).unwrap();
        let socket_path = crate::sessiond::client::default_socket_path(&app_data_dir);
        let hook: crate::sessiond::daemon::ShutdownHook = Arc::new(|| {});
        let daemon_socket = socket_path.clone();
        std::thread::spawn(move || {
            let _ =
                crate::sessiond::daemon::run_daemon_inner(daemon_socket, Duration::from_secs(60), hook);
        });
        wait_for_timeout(|| socket_path.exists(), Duration::from_secs(2), "daemon socket").await;

        fn build(
            app_data_dir: &Path,
            factory: Arc<dyn crate::session::pty_factory::PtyFactory>,
        ) -> (Arc<SessionManager>, Arc<RecordingEvents>) {
            let events = Arc::new(RecordingEvents::default());
            let registry = Arc::new(SessionRegistry::new());
            let hub = Arc::new(NotificationHub::new(
                registry.clone(),
                events.clone() as Arc<dyn AppEvents>,
                Arc::new(SystemClock),
                Duration::from_millis(3000),
            ));
            let observer = Arc::new(ObserverRuntime::new(hub.clone(), vec![]));
            let mgr = Arc::new(
                SessionManager::new(
                    factory,
                    observer,
                    registry,
                    events.clone() as Arc<dyn AppEvents>,
                    hub,
                    Arc::new(|| None),
                )
                .with_shell_resolver(Arc::new(|_, _| shells::ResolvedShell {
                    program: "/bin/sh".into(),
                    args: vec![],
                    extra_env: vec![],
                }))
                .with_app_data_dir(app_data_dir.to_path_buf())
                .with_broker_mode(true),
            );
            (mgr, events)
        }

        fn collect(mgr: &Arc<SessionManager>, agent_id: &str) -> Arc<Mutex<String>> {
            let out = Arc::new(Mutex::new(String::new()));
            let out2 = out.clone();
            let channel: Channel<OutputChunk> = Channel::new(move |body| {
                if let InvokeResponseBody::Json(s) = body {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                        if let Some(d) = v.get("data").and_then(|d| d.as_str()) {
                            out2.lock().push_str(d);
                        }
                    }
                }
                Ok(())
            });
            mgr.attach_output(agent_id, channel);
            out
        }

        fn mkreq(agent_id: &str) -> CreateSessionRequest {
            CreateSessionRequest {
                agent_id: agent_id.into(),
                cols: Some(80),
                rows: Some(24),
                cwd: None,
                shell: None,
                startup_command: None,
                personality_prompt: None,
                autostart_claude: Some(false),
            }
        }

        // manager1: 혼합 팩토리. "b1"=브로커, "f1"=폴백(in-process).
        let fallback: Arc<dyn crate::session::pty_factory::PtyFactory> = Arc::new(PortablePtyFactory);
        let mixed: Arc<dyn crate::session::pty_factory::PtyFactory> = Arc::new(MixedFactory {
            broker: BrokerPtyFactory::new(&app_data_dir, fallback),
            portable: PortablePtyFactory,
        });
        let (mgr1, _e1) = build(&app_data_dir, mixed);
        let b1 = mgr1.create(mkreq("b1")).unwrap();
        let f1 = mgr1.create(mkreq("f1")).unwrap();
        assert_eq!(b1.state, SessionState::Running);
        assert_eq!(f1.state, SessionState::Running);

        // 둘 다 살아 있음(echo 왕복).
        let ob1 = collect(&mgr1, "b1");
        let of1 = collect(&mgr1, "f1");
        mgr1.write_input("b1", "echo B-ALIVE-1\n");
        mgr1.write_input("f1", "echo F-ALIVE-1\n");
        wait_for_timeout(|| ob1.lock().contains("B-ALIVE-1"), Duration::from_secs(5), "b1 echo").await;
        wait_for_timeout(|| of1.lock().contains("F-ALIVE-1"), Duration::from_secs(5), "f1 echo").await;

        // handoff_all: b1=detach, f1=v1 fd 핸드오프. 반환 카운트는 둘 합.
        let handed = mgr1.handoff_all(&std::collections::HashMap::new());
        assert_eq!(handed, 2, "both sessions must be handed off via their own path");
        assert!(mgr1.find("b1").is_none());
        assert!(mgr1.find("f1").is_none());

        // 데몬 List: v1 핸드오프 1건(broker=false) + 브로커 1건(broker=true).
        {
            let client = crate::sessiond::client::Client::connect(&socket_path).unwrap();
            let listed = client.list().unwrap();
            let b = listed.iter().find(|s| s.agent_id == "b1").expect("b1 in daemon list");
            let f = listed.iter().find(|s| s.agent_id == "f1").expect("f1 in daemon list");
            assert!(b.broker, "b1 must be a broker session");
            assert!(!f.broker, "f1 must be a v1 handoff session");
        }
        drop(mgr1);

        // 새 매니저(브로커 모드)로 둘 다 복구.
        let (mgr2, _e2) = build(&app_data_dir, Arc::new(PortablePtyFactory));
        let known: HashSet<String> = ["b1".to_string(), "f1".to_string()].into_iter().collect();
        let adopted = mgr2.adopt_detached(&known);
        let ids: HashSet<String> = adopted.iter().map(|a| a.agent_id.clone()).collect();
        assert_eq!(ids, known, "both broker and v1 sessions must be adopted in broker mode");

        // 둘 다 여전히 살아 왕복.
        let ob2 = collect(&mgr2, "b1");
        let of2 = collect(&mgr2, "f1");
        mgr2.write_input("b1", "echo B-ALIVE-2\n");
        mgr2.write_input("f1", "echo F-ALIVE-2\n");
        wait_for_timeout(|| ob2.lock().contains("B-ALIVE-2"), Duration::from_secs(5), "b1 post-adopt").await;
        wait_for_timeout(|| of2.lock().contains("F-ALIVE-2"), Duration::from_secs(5), "f1 post-adopt").await;

        mgr2.dispose("b1");
        mgr2.dispose("f1");
        let _ = std::fs::remove_dir_all(&app_data_dir);
    }

    /// §P2-a: detach된 브로커 세션의 자식이 나중에 스스로 죽으면 데몬 테이블에
    /// exited 엔트리가 남는데, 이후 adopt_detached가 그걸 best-effort Kill로
    /// 치워 데몬의 table-empty 종료 누수를 막아야 한다. 짧게 사는 자식으로 재현.
    #[cfg(unix)]
    #[tokio::test]
    async fn broker_adopt_reaps_exited_detached_session_from_daemon_table() {
        use crate::session::broker_pty::BrokerPtyFactory;
        use crate::session::pty_factory::PortablePtyFactory;
        use std::collections::HashSet;

        let app_data_dir = scratch_dir("reap-appdata");
        std::fs::create_dir_all(&app_data_dir).unwrap();
        let socket_path = crate::sessiond::client::default_socket_path(&app_data_dir);
        let hook: crate::sessiond::daemon::ShutdownHook = Arc::new(|| {});
        let daemon_socket = socket_path.clone();
        std::thread::spawn(move || {
            let _ =
                crate::sessiond::daemon::run_daemon_inner(daemon_socket, Duration::from_secs(60), hook);
        });
        wait_for_timeout(|| socket_path.exists(), Duration::from_secs(2), "daemon socket").await;

        fn build_short_lived(app_data_dir: &Path) -> Arc<SessionManager> {
            let events = Arc::new(RecordingEvents::default());
            let registry = Arc::new(SessionRegistry::new());
            let hub = Arc::new(NotificationHub::new(
                registry.clone(),
                events.clone() as Arc<dyn AppEvents>,
                Arc::new(SystemClock),
                Duration::from_millis(3000),
            ));
            let observer = Arc::new(ObserverRuntime::new(hub.clone(), vec![]));
            let fallback: Arc<dyn crate::session::pty_factory::PtyFactory> =
                Arc::new(PortablePtyFactory);
            let factory = Arc::new(BrokerPtyFactory::new(app_data_dir, fallback));
            Arc::new(
                SessionManager::new(
                    factory,
                    observer,
                    registry,
                    events.clone() as Arc<dyn AppEvents>,
                    hub,
                    Arc::new(|| None),
                )
                // 자식이 0.4초 후 스스로 종료하도록 셸을 sh -c 'sleep 0.4'로 고정.
                .with_shell_resolver(Arc::new(|_, _| shells::ResolvedShell {
                    program: "/bin/sh".into(),
                    args: vec!["-c".into(), "sleep 0.4".into()],
                    extra_env: vec![],
                }))
                .with_app_data_dir(app_data_dir.to_path_buf())
                .with_broker_mode(true),
            )
        }

        let mgr1 = build_short_lived(&app_data_dir);
        mgr1
            .create(CreateSessionRequest {
                agent_id: "a1".into(),
                cols: Some(80),
                rows: Some(24),
                cwd: None,
                shell: None,
                startup_command: None,
                personality_prompt: None,
                autostart_claude: Some(false),
            })
            .unwrap();
        // detach: 자식은 데몬 소유로 남는다(곧 sleep이 끝나 스스로 죽는다).
        assert_eq!(mgr1.handoff_all(&std::collections::HashMap::new()), 1);
        drop(mgr1);

        // 데몬 List에 a1이 exited로 남을 때까지 대기(자식 reap 확인).
        wait_for_timeout(
            || {
                crate::sessiond::client::Client::connect(&socket_path)
                    .and_then(|c| c.list())
                    .map(|list| list.iter().any(|s| s.agent_id == "a1" && s.exited))
                    .unwrap_or(false)
            },
            Duration::from_secs(3),
            "detached child never showed up as exited in the daemon table",
        )
        .await;

        // adopt_detached: exited 브로커 세션을 Kill로 치운다. exited라 입양은 0건.
        let mgr2 = build_short_lived(&app_data_dir);
        let known: HashSet<String> = ["a1".to_string()].into_iter().collect();
        let adopted = mgr2.adopt_detached(&known);
        assert!(adopted.is_empty(), "exited session must not be adopted");

        // 데몬 테이블에서 a1이 사라져야 한다(누수 방지).
        wait_for_timeout(
            || {
                crate::sessiond::client::Client::connect(&socket_path)
                    .and_then(|c| c.list())
                    .map(|list| !list.iter().any(|s| s.agent_id == "a1"))
                    .unwrap_or(false)
            },
            Duration::from_secs(3),
            "exited broker session was never reaped from the daemon table",
        )
        .await;

        let _ = std::fs::remove_dir_all(&app_data_dir);
    }

    /// 실기기 재현 프로브: 프론트의 attach(1회) → create →
    /// { dispose → 즉시 create } 반복에서도 create가 멈추지 않고, 최초 출력
    /// 채널과 observer cleanup 계약이 유지되어야 한다.
    #[tokio::test]
    #[ignore = "real PTY; run explicitly"]
    async fn real_shell_restart_mash_never_wedges_and_never_leaks_observer_files() {
        let events = Arc::new(RecordingEvents::default());
        let registry = Arc::new(SessionRegistry::new());
        let hub = Arc::new(NotificationHub::new(
            registry.clone(),
            events.clone() as Arc<dyn AppEvents>,
            Arc::new(SystemClock),
            Duration::from_millis(3_000),
        ));
        let observer_dir = scratch_dir("observer-mash");
        let observer = Arc::new(ObserverRuntime::new(
            hub.clone(),
            vec![Arc::new(ClaudeAdapter::new(
                observer_dir.clone(),
                std::env::current_exe().unwrap(),
            ))],
        ));
        let manager = Arc::new(SessionManager::new(
            Arc::new(PortablePtyFactory),
            observer,
            registry,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| Some("http://127.0.0.1:45999/hook".into())),
        ));

        let output = Arc::new(Mutex::new(String::new()));
        let output_for_channel = output.clone();
        manager.attach_output(
            "mash",
            Channel::new(move |body| {
                if let InvokeResponseBody::Json(json) = body {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) {
                        if let Some(data) = value.get("data").and_then(|data| data.as_str()) {
                            output_for_channel.lock().push_str(data);
                        }
                    }
                }
                Ok(())
            }),
        );

        let request = || CreateSessionRequest {
            agent_id: "mash".into(),
            cols: Some(80),
            rows: Some(24),
            cwd: Some("/definitely/not/a/real/dir".into()),
            shell: None,
            startup_command: Some("echo mash-marker".into()),
            personality_prompt: None,
            autostart_claude: Some(false),
        };
        let create_with_watchdog = |manager: Arc<SessionManager>, label: String| async move {
            let handle = tokio::task::spawn_blocking(move || manager.create(request()));
            match tokio::time::timeout(Duration::from_secs(10), handle).await {
                Err(_) => panic!("create() wedged (>10s) at {label}"),
                Ok(join) => join
                    .unwrap_or_else(|error| panic!("create() panicked at {label}: {error:?}"))
                    .unwrap_or_else(|error| panic!("create() returned Err at {label}: {error}")),
            }
        };

        create_with_watchdog(manager.clone(), "initial".into()).await;
        for index in 0..6 {
            manager.dispose("mash");
            create_with_watchdog(manager.clone(), format!("restart#{index}")).await;
        }

        output.lock().clear();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            manager.write_input("mash", "echo final-alive-98765\r");
            tokio::time::sleep(Duration::from_millis(500)).await;
            if output.lock().contains("final-alive-98765") {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "final session output never reached the originally attached channel"
            );
        }

        let leftovers = std::fs::read_dir(&observer_dir)
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert!(
            leftovers <= 1,
            "observer files accumulated across restarts: {leftovers}"
        );

        manager.dispose("mash");
        wait_for_timeout(
            || matches!(events.states().last(), Some(SessionState::Disposed)),
            Duration::from_secs(5),
            "final dispose never completed",
        )
        .await;
        let _ = std::fs::remove_dir_all(observer_dir);
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "real PowerShell PTY and built forwarder; no model call"]
    async fn observed_powershell_fake_clis_cross_the_complete_local_boundary() {
        let _path_lock = observer_path_lock().lock().unwrap();
        let root = std::env::temp_dir().join(format!(
            "Agent Office observer PTY test {}",
            uuid::Uuid::new_v4(),
        ));
        let fake_dir = root.join("fake cli bin");
        let settings_dir = root.join("settings with spaces");
        let forwarder_dir = root.join("forwarder with spaces");
        std::fs::create_dir_all(&forwarder_dir).unwrap();
        write_observer_fake_clis(&fake_dir);

        let built = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("debug")
            .join("agent-office.exe");
        assert!(
            built.is_file(),
            "run cargo build before this ignored test: {}",
            built.display()
        );
        let forwarder = forwarder_dir.join("agent-office.exe");
        std::fs::copy(&built, &forwarder).unwrap();

        let codex_args = root.join("codex args.txt");
        let claude_args = root.join("claude args.txt");
        let codex_pid = root.join("codex pid.txt");
        let claude_pid = root.join("claude pid.txt");
        let bypass = root.join("bypass marker.txt");
        let shell_pid = root.join("shell pid.txt");
        let shell_env = root.join("shell env.txt");
        let command_resolution = root.join("command resolution.txt");
        let inherited_path = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(fake_dir.as_os_str().to_os_string())
                .chain(std::env::split_paths(&inherited_path).map(|p| p.into_os_string())),
        )
        .unwrap();
        let _env = ObserverEnvGuard::set(&[
            ("PATH", path),
            ("AO_FAKE_FORWARDER", forwarder.as_os_str().to_os_string()),
            ("AO_FAKE_CODEX_ARGS", codex_args.as_os_str().to_os_string()),
            (
                "AO_FAKE_CLAUDE_ARGS",
                claude_args.as_os_str().to_os_string(),
            ),
            ("AO_FAKE_CODEX_PID", codex_pid.as_os_str().to_os_string()),
            ("AO_FAKE_CLAUDE_PID", claude_pid.as_os_str().to_os_string()),
            ("AO_FAKE_BYPASS", bypass.as_os_str().to_os_string()),
            ("AO_FAKE_SHELL_PID", shell_pid.as_os_str().to_os_string()),
            ("AO_FAKE_SHELL_ENV", shell_env.as_os_str().to_os_string()),
            (
                "AO_FAKE_RESOLUTION",
                command_resolution.as_os_str().to_os_string(),
            ),
        ]);

        let events = Arc::new(RecordingEvents::default());
        let registry = Arc::new(SessionRegistry::new());
        let hub = Arc::new(NotificationHub::new(
            registry.clone(),
            events.clone() as Arc<dyn AppEvents>,
            Arc::new(SystemClock),
            Duration::from_millis(3_000),
        ));
        let observer = Arc::new(ObserverRuntime::production(
            hub.clone(),
            settings_dir.clone(),
            forwarder.clone(),
        ));
        let server = Arc::new(ObserverServerState::default());
        assert!(server.ensure(observer.clone()).await.is_some());
        let server_url = server.current_url();
        let server_for_getter = server.clone();
        let resolved_shell = Arc::new(Mutex::new(None));
        let resolved_shell_for_resolver = resolved_shell.clone();
        let manager = Arc::new(
            SessionManager::new(
                Arc::new(PortablePtyFactory),
                observer,
                registry,
                events.clone() as Arc<dyn AppEvents>,
                hub,
                Arc::new(move || server_for_getter.current_url()),
            )
            .with_shell_resolver(Arc::new(move |selected, wrappers| {
                let resolved = shells::resolve_observed(selected, wrappers);
                *resolved_shell_for_resolver.lock() = Some((
                    resolved.program.clone(),
                    resolved.args.clone(),
                    resolved.extra_env.clone(),
                ));
                resolved
            })),
        );

        let created = manager
            .create(CreateSessionRequest {
                agent_id: "observer-pty".into(),
                cols: Some(100),
                rows: Some(40),
                cwd: Some(root.to_string_lossy().into_owned()),
                shell: Some("powershell".into()),
                startup_command: None,
                autostart_claude: Some(false),
            })
            .unwrap();

        let output = Arc::new(Mutex::new(String::new()));
        let output_for_channel = output.clone();
        manager.attach_output(
            "observer-pty",
            Channel::new(move |body| {
                if let InvokeResponseBody::Json(json) = body {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) {
                        if let Some(data) = value.get("data").and_then(|data| data.as_str()) {
                            output_for_channel.lock().push_str(data);
                        }
                    }
                }
                Ok(())
            }),
        );

        let (shell_program, shell_args, shell_extra_env) = resolved_shell.lock().clone().unwrap();
        let decoded_wrapper = decode_observer_powershell_command(&shell_args).unwrap();
        assert!(decoded_wrapper.contains("function global:claude"));
        assert!(decoded_wrapper.contains("function global:codex"));
        assert!(shell_extra_env.is_empty());
        let mut wrapper_hash = sha1_smol::Sha1::new();
        wrapper_hash.update(decoded_wrapper.as_bytes());
        let wrapper_hash = wrapper_hash.digest().to_string();

        let shell_marker_command = "[IO.File]::WriteAllText($env:AO_FAKE_SHELL_PID, \"$PID\")\r";
        manager.write_input("observer-pty", shell_marker_command);
        wait_for_timeout(
            || shell_pid.is_file(),
            Duration::from_secs(5),
            "PowerShell PTY did not execute the minimal marker",
        )
        .await;

        let resolution_command = concat!(
            "$ao = Get-Command codex -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1; ",
            "[IO.File]::WriteAllText($env:AO_FAKE_RESOLUTION, [string]$ao.Source); ",
            "[IO.File]::WriteAllText($env:AO_FAKE_SHELL_ENV, ($env:PATH + \"`n\" + $env:AO_FAKE_FORWARDER + \"`n\" + $env:AGENT_OFFICE_HOOK_URL + \"`n\" + $env:AGENT_OFFICE_SESSION))\r",
        );
        manager.write_input("observer-pty", resolution_command);
        wait_for_timeout(
            || command_resolution.is_file() && shell_env.is_file(),
            Duration::from_secs(5),
            "PowerShell PTY did not record command resolution and environment",
        )
        .await;
        let resolved_command = std::fs::read_to_string(&command_resolution).unwrap();
        let expected_fake = fake_dir
            .join("codex.ps1")
            .to_string_lossy()
            .to_ascii_lowercase();
        assert_eq!(
            resolved_command.trim().to_ascii_lowercase(),
            expected_fake,
            "refusing to invoke codex because PowerShell did not resolve the fake CLI: {resolved_command:?}"
        );
        let shell_env_contents = std::fs::read_to_string(&shell_env).unwrap();
        let mut shell_env_lines = shell_env_contents.lines();
        assert!(shell_env_lines
            .next()
            .unwrap()
            .to_ascii_lowercase()
            .contains(&fake_dir.to_string_lossy().to_ascii_lowercase()));
        assert_eq!(
            shell_env_lines.next(),
            Some(forwarder.to_string_lossy().as_ref())
        );
        assert_eq!(shell_env_lines.next(), server_url.as_deref());
        assert_eq!(shell_env_lines.next(), Some(created.session_id.as_str()));
        eprintln!(
            "observer-pty boundary session={} serverUrl={:?} shellPid={} shellProgram={:?} wrapperSha1={} commandResolution={:?}",
            created.session_id,
            server_url,
            std::fs::read_to_string(&shell_pid).unwrap().trim(),
            shell_program,
            wrapper_hash,
            resolved_command,
        );

        manager.write_input("observer-pty", "codex resume --last\r");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while {
            let activities = events.activities();
            let notifications = events.notifications();
            activities.len() < 4
                || !activities
                    .iter()
                    .any(|event| event.text.as_deref() == Some("codex-marker"))
                || !notifications
                    .iter()
                    .any(|event| event.message == "codex-attention")
                || !notifications
                    .iter()
                    .any(|event| event.message == "작업이 완료되었습니다.")
        } {
            if tokio::time::Instant::now() >= deadline {
                let pid = std::fs::read_to_string(&shell_pid).unwrap();
                let process_status = std::process::Command::new("tasklist.exe")
                    .args(["/FI", &format!("PID eq {}", pid.trim())])
                    .output()
                    .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                    .unwrap_or_else(|error| format!("tasklist failed: {error}"));
                eprintln!(
                    "observer-pty failure shellProcess={:?} rawPtyOutput={:?} artifacts={{codexArgs:{},codexPid:{},bypass:{},settingsFiles:{}}} activities={:?} notifications={:?}",
                    process_status,
                    output.lock().clone(),
                    codex_args.is_file(),
                    codex_pid.is_file(),
                    bypass.is_file(),
                    std::fs::read_dir(&settings_dir)
                        .map(|entries| entries.count())
                        .unwrap_or(0),
                    events.activities(),
                    events.notifications(),
                );
                panic!("Codex fake did not cross wrapper/forwarder/server");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            events.activities().len(),
            4,
            "Codex fake must emit the complete four-activity boundary before Claude starts",
        );
        let codex_argv = std::fs::read_to_string(&codex_args)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(&codex_argv[codex_argv.len() - 2..], ["resume", "--last"]);
        assert_eq!(
            codex_argv.iter().filter(|arg| arg.as_str() == "-c").count(),
            6
        );
        assert_eq!(
            codex_argv
                .iter()
                .filter(
                    |arg| arg.contains("powershell.exe -NoProfile -NonInteractive -EncodedCommand")
                )
                .count(),
            6,
        );
        let rendered_codex_argv = codex_argv.join("\0");
        for forbidden in [
            "dangerously-bypass-hook-trust",
            "approval_policy",
            "--approval-policy",
            "sandbox_mode",
            "--sandbox",
            "model=",
            "--model",
            "model_reasoning_effort",
            "--ignore-user-config",
            "--ignore-rules",
        ] {
            assert!(
                !rendered_codex_argv.contains(forbidden),
                "captured Codex argv contained forbidden override {forbidden}: {codex_argv:?}"
            );
        }

        manager.write_input("observer-pty", "claude user-suffix\r");
        wait_for_timeout(
            || {
                let activities = events.activities();
                let notifications = events.notifications();
                activities.len() >= 8
                    && activities
                        .iter()
                        .any(|event| event.text.as_deref() == Some("claude-marker"))
                    && notifications
                        .iter()
                        .any(|event| event.message == "claude-attention")
                    && notifications
                        .iter()
                        .any(|event| event.message == "claude-stop")
            },
            Duration::from_secs(10),
            "Claude fake did not cross wrapper/settings/curl/server",
        )
        .await;
        let claude_argv = std::fs::read_to_string(&claude_args)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(claude_argv.last().map(String::as_str), Some("user-suffix"));
        let settings_index = claude_argv
            .iter()
            .position(|arg| arg == "--settings")
            .unwrap();
        assert!(Path::new(&claude_argv[settings_index + 1]).is_file());

        let activities = events.activities();
        let notifications = events.notifications();
        assert_eq!(
            activities.len(),
            8,
            "Codex and Claude fakes must emit eight activities total",
        );
        assert_eq!(
            activities
                .iter()
                .filter(|event| event.kind == ActivityKind::SubStart)
                .count(),
            2,
        );
        assert_eq!(
            activities
                .iter()
                .filter(|event| event.kind == ActivityKind::SubStop)
                .count(),
            2,
        );
        assert!(activities
            .iter()
            .all(|event| event.session_id == created.session_id));
        assert!(notifications
            .iter()
            .all(|event| event.session_id == created.session_id));
        assert!(activities
            .iter()
            .any(|event| event.text.as_deref() == Some("codex-marker")));
        assert!(activities
            .iter()
            .any(|event| event.text.as_deref() == Some("claude-marker")));
        assert!(notifications
            .iter()
            .any(|event| event.message == "codex-attention"));
        assert!(notifications
            .iter()
            .any(|event| event.message == "claude-attention"));
        assert!(notifications
            .iter()
            .any(|event| event.message == "작업이 완료되었습니다."));
        assert!(notifications
            .iter()
            .any(|event| event.message == "claude-stop"));
        assert!(!notifications
            .iter()
            .any(|event| event.message.contains("must-not-surface")));
        assert!(codex_pid.is_file() && claude_pid.is_file());
        let codex_host_pid = std::fs::read_to_string(&codex_pid).unwrap();
        let claude_host_pid = std::fs::read_to_string(&claude_pid).unwrap();
        let mut config_hash = sha1_smol::Sha1::new();
        config_hash.update(codex_argv.join("\0").as_bytes());
        let config_hash = config_hash.digest().to_string();
        eprintln!(
            "observer-pty session={} codexHostPid={} claudeHostPid={} configSha1={} codexArgv={:?} claudeArgv={:?}",
            created.session_id,
            codex_host_pid.trim(),
            claude_host_pid.trim(),
            config_hash,
            codex_argv,
            claude_argv,
        );

        let before = (activities.len(), notifications.len());
        manager.write_input(
            "observer-pty",
            "$ao = Get-Command codex -CommandType Application,ExternalScript | Select-Object -First 1; & $ao.Source bypass-marker\r",
        );
        wait_for_timeout(
            || bypass.is_file(),
            Duration::from_secs(5),
            "explicit external-command bypass did not execute",
        )
        .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            (events.activities().len(), events.notifications().len()),
            before
        );

        manager.dispose("observer-pty");
        wait_for_timeout(
            || matches!(events.states().last(), Some(SessionState::Disposed)),
            Duration::from_secs(5),
            "observed real PTY did not dispose",
        )
        .await;
        server.shutdown();
        drop(_env);
        let _ = std::fs::remove_dir_all(root);
    }
}
