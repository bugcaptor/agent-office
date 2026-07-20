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
    /// adopt 복원 스냅샷(화면 이미지). 스트림 바이트로 계수하지 않는다(§#49 함정 2):
    /// base가 이미 이 지점을 가리키므로 offset에 잡히면 그만큼 데이터가 유실된다.
    /// 렌더러 누적 회계에 안 잡히도록 bytes=0 청크로 방출된다.
    Restore(Vec<u8>),
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
    /// 마지막 PTY 활동(출력 수신 또는 stdin 주입) 시각(epoch ms). 봇 모드의
    /// turn-taking 유휴 판정에 쓴다 — 봇은 이 값이 일정 시간 이상 정체됐을 때만
    /// 다음 지시를 주입한다(이슈 #57, docs/bot-mode-design.md). 0이면 아직 활동
    /// 없음. reader 스레드와 write_input이 갱신한다.
    last_activity_ms: Arc<std::sync::atomic::AtomicU64>,
}

/// epoch(UTC) 밀리초. 봇 turn-taking 유휴 계산용(단조성보다 벽시계 기준이면
/// 충분 — 폴링 주기가 초 단위라 시계 점프에 민감하지 않다).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
                        ..Default::default()
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
                    ..Default::default()
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
            last_activity_ms: Arc::new(std::sync::atomic::AtomicU64::new(0)),
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
            // §#49: 복원 스냅샷은 스트림 바이트가 아니므로 Restore로 보내 offset
            // 회계에서 제외한다(bytes=0 청크로 방출됨). Data로 보내면 offset이
            // 스냅샷 길이만큼 부풀어 그만큼 유실된다(이 버그의 재발).
            let _ = tx.send(ReaderMsg::Restore(bytes));
        }
        let mut reader = spawned.reader;
        let reader_activity = session.last_activity_ms.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        // 봇 turn-taking 유휴 판정용 활동 시각 갱신(출력 수신).
                        reader_activity.store(now_ms(), Ordering::Relaxed);
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
                // stdin 주입도 활동으로 기록 — 봇이 방금 넣은 프롬프트 직후
                // 곧바로 다음 지시를 밀어넣지 않게(turn-taking 유휴 리셋).
                s.last_activity_ms.store(now_ms(), Ordering::Relaxed);
                let _ = s.writer.lock().write_all(data.as_bytes());
            }
        }
    }

    /// 세션이 마지막 활동(출력/입력) 이후 유휴로 있었던 시간(ms). 아직 활동이
    /// 없었거나 세션이 없으면 None. 봇 turn-taking이 릴레이 주입 타이밍을 잡는 데
    /// 쓴다.
    pub fn idle_ms(&self, agent_id: &str) -> Option<u64> {
        let s = self.find(agent_id)?;
        let last = s.last_activity_ms.load(Ordering::Relaxed);
        if last == 0 {
            return None;
        }
        Some(now_ms().saturating_sub(last))
    }

    /// 세션이 살아서 입력을 받을 수 있는 상태(Running)인지. 봇이 잡을 이어갈 수
    /// 있는지 판단한다.
    pub fn is_running(&self, agent_id: &str) -> bool {
        self.find(agent_id)
            .map(|s| *s.state.lock() == SessionState::Running)
            .unwrap_or(false)
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
    pub fn handoff_all(
        &self,
        snapshots: &std::collections::HashMap<String, String>,
        rendered_bytes: &std::collections::HashMap<String, u64>,
    ) -> usize {
        if self.broker_mode {
            return self.handoff_all_broker(snapshots, rendered_bytes);
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
    pub fn handoff_all(
        &self,
        _snapshots: &std::collections::HashMap<String, String>,
        _rendered_bytes: &std::collections::HashMap<String, u64>,
    ) -> usize {
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
    fn handoff_all_broker(
        &self,
        snapshots: &std::collections::HashMap<String, String>,
        rendered_bytes: &std::collections::HashMap<String, u64>,
    ) -> usize {
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
                    let offset = snapshot_offset(&sess, rendered_bytes.get(agent_id.as_str()).copied());
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
    pub fn upload_snapshots(
        &self,
        snapshots: &std::collections::HashMap<String, String>,
        rendered_bytes: &std::collections::HashMap<String, u64>,
    ) {
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
            // 데몬 테이블에 없는 agentId면 no-op으로 무시된다(안전). 스냅샷 offset은
            // base(attach 시 stream_offset) + 렌더러가 실제 렌더한 raw 바이트 누적치로
            // 동봉해 유실 창을 없앤다(§#49) -- 렌더러 누적치가 없으면 None(데몬은
            // 수신 시점 ring.total()로 폴백).
            let offset = self
                .find(agent_id)
                .and_then(|s| snapshot_offset(&s, rendered_bytes.get(agent_id).copied()));
            let _ = client.update_snapshot(agent_id, snap.as_bytes(), offset);
        }
    }

    #[cfg(not(unix))]
    pub fn upload_snapshots(
        &self,
        _snapshots: &std::collections::HashMap<String, String>,
        _rendered_bytes: &std::collections::HashMap<String, u64>,
    ) {
    }

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
        // 이슈 #40: 삭제 소유권은 데몬이 유지하되(앱 install_session엔 빈 벡터를
        // 넘긴다), 앱이 꺼진 사이 사라졌을 수 있는 observer 설정 파일은 데몬이
        // 돌려준 cleanup_paths로 입양 시점에 멱등 재작성한다.
        let restore_paths: Vec<std::path::PathBuf> =
            meta.cleanup_paths.iter().map(std::path::PathBuf::from).collect();
        self.observer
            .restore_session_artifacts(&info.session_id, &restore_paths);
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
        let cleanup_paths: Vec<std::path::PathBuf> =
            adopted.cleanup_paths.iter().map(std::path::PathBuf::from).collect();
        // 이슈 #40: 앱이 꺼진 사이 사라졌을 수 있는 observer 설정 파일을 입양
        // 시점에 멱등 재작성한다. cleanup_paths가 비면(observer OFF 세션) no-op.
        self.observer
            .restore_session_artifacts(&adopted.session_id, &cleanup_paths);
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

/// 스냅샷 업로드/핸드오프에 실을 절대 offset을 계산한다(§#49).
/// `base`(attach 시 DataAttachOk가 준 stream_offset, 세션당 고정) + `rendered`
/// (렌더러가 실제 렌더/소비한 raw 스트림 바이트 누적치). 렌더러 누적치가
/// 없으면(프론트가 그 세션 값을 안 실어 보냄) None을 반환해 데몬이 수신 시점
/// ring.total()로 폴백하게 한다(trim 과다 위험은 값이 아예 없을 때만).
#[cfg(unix)]
fn snapshot_offset(sess: &Session, rendered: Option<u64>) -> Option<u64> {
    let rendered = rendered?;
    let base = sess
        .broker_stream_offset
        .as_ref()
        .map(|c| c.load(Ordering::SeqCst))
        .unwrap_or(0);
    Some(base + rendered)
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
                    Some(ReaderMsg::Restore(bytes)) => {
                        // §#49 함정 2: adopt 복원 스냅샷(화면 이미지)은 실시간
                        // 스트림 출력이 아니라 화면 복원이다. batcher를 거치면
                        // consumed>0으로 계수돼 offset이 부풀므로, bytes=0인 청크로
                        // 직접 방출한다. 순서 보존을 위해 혹시 남아 있을 pending을
                        // 먼저 flush한다(Restore는 항상 첫 메시지라 실제로는 없음).
                        // BEL/on_output 휴리스틱도 적용하지 않는다(실시간 출력 아님).
                        batcher.flush(&*sink);
                        deadline = None;
                        batcher.emit_uncounted(String::from_utf8_lossy(&bytes).into_owned(), &*sink);
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
mod tests;

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
mod real_pty_smoke;
