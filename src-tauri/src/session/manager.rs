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
use crate::session::output::{spawn_output_pump, OutputSink, ReaderMsg};
use crate::session::pi_extension;
use crate::session::pty_factory::{ExitOutcome, PtyControl, PtyFactory, PtySpawnOptions, SpawnedPty};
use crate::session::shells;
use crate::session_events::types::{AgentEventProfile, SessionStartedEvent};
use crate::state::{AppEvents, SessionRegistry};
use crate::types::*;

/// pub(super): handoff_v1.rs/handoff_broker.rs(세션 핸드오프 형제 모듈)가
/// 필드에 직접 접근한다 -- 최소 가시성으로 `session` 모듈 트리 전체에만 연다.
pub(super) struct Session {
    pub(super) session_id: SessionId,
    agent_id: AgentId,
    pub(super) state: Mutex<SessionState>,
    writer: Mutex<Box<dyn Write + Send>>,
    control: Arc<dyn PtyControl>,
    pub(super) cleanup_paths: Vec<std::path::PathBuf>,
    kill_requested: AtomicBool,
    /// 시작 작업 디렉터리(세션 수명 동안 불변 -- `cd`는 추적하지 않는다).
    /// 핸드오프 시 Handoff 메시지의 진단/List용 메타데이터로 실어 보낸다.
    pub(super) cwd: String,
    /// 현재 알려진 터미널 크기. resize()가 갱신 -- 핸드오프 시 Handoff
    /// 메시지에 실어 데몬에 보내고, 입양 응답의 AdoptedSessionInfo로
    /// 프론트에 되돌려줘 터미널 크기를 맞추는 데 쓴다.
    pub(super) size: Mutex<(u16, u16)>, // (cols, rows)
    /// 세션 핸드오프(§핵심 3, 4). true면 on_exit/dispose가 즉시 return —
    /// 이 세션의 실제 수명은 sessiond가 넘겨받았다(또는 넘겨받는 중이다).
    /// 필드 자체는 크로스플랫폼으로 둬 cfg 분기를 최소화한다(Windows/Fake는
    /// 항상 false로 남는 no-op).
    pub(super) handed_off: AtomicBool,
    /// unix 전용: 핸드오프 시 리더 스레드를 확정적으로 멈추는 스위치와,
    /// sessiond에 넘길 마스터 fd/pid/pgid. `create_with_profile`(팩토리
    /// spawn)과 `adopt_detached`(assemble_adopted) 양쪽이 채운다 — Fake로
    /// 만든 세션은 항상 None(핸드오프 불가능 세션).
    #[cfg(unix)]
    pub(super) reader_interrupt: Mutex<Option<crate::session::poll_reader::ReaderInterrupt>>,
    #[cfg(unix)]
    pub(super) handoff: Mutex<Option<crate::session::pty_factory::HandoffInfo>>,
    /// 입양된 세션 한정(§핵심 4의 AdoptedReader 정지 게이트) -- 재핸드오프
    /// 인터럽트 직전 true로 세팅해야 EofWaiter가 오발화하지 않는다.
    /// create() 경로(RealWaiter가 독립적으로 exit 판정)는 항상 None. 타입
    /// 자체는 크로스플랫폼이라 cfg 분기가 필요 없다(항상 컴파일되고, 비unix는
    /// 그냥 항상 None으로 남는다).
    pub(super) eof_stop_gate: Option<Arc<AtomicBool>>,
    /// v2 브로커 데몬이 이 세션을 소유하는가(SpawnedPty에서 전파). 브로커 모드
    /// 매니저라도 팩토리 폴백으로 생긴 in-process 세션은 false다 — handoff_all이
    /// 이 플래그로 "스냅샷 업로드+detach"(true)와 "v1 fd 핸드오프"(false)를 가른다.
    pub(super) broker_owned: bool,
    /// 브로커 data 연결의 누적 수신 오프셋 카운터(§P1). 스냅샷 업로드 시 현재
    /// 값을 offset으로 동봉해 데몬 수신 시점 ring.total()과의 간극(유실 창)을
    /// 없앤다. 브로커 세션만 Some.
    pub(super) broker_stream_offset: Option<Arc<std::sync::atomic::AtomicU64>>,
    /// detach 시 data 소켓을 결정적으로 닫는 핸들(§#50 선결). broker_owned 세션의
    /// detach에서 shutdown하면 앱 reader 스레드가 EOF로 종료되고 데몬 conn이 정리돼
    /// List `attached`가 false로 돌아간다 — adopt가 라이브 원격 소유 세션을
    /// 가로채지 않게 하는 근거(현재 인스턴스가 살아 붙어 있으면 attached=true 유지).
    /// 브로커 세션만 Some(unix).
    #[cfg(unix)]
    pub(super) broker_data_shutdown: Mutex<Option<crate::session::broker_pty::BrokerDataShutdown>>,
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
    /// pub(super): handoff_v1.rs의 adopt_one/handoff_broker.rs의 adopt_one_broker가
    /// `restore_session_artifacts`를 직접 호출한다.
    pub(super) observer: Arc<ObserverRuntime>,
    get_observer_url: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    /// pub(super): 핸드오프/입양 형제 모듈이 세션 제거 시 레지스트리도 함께 정리한다.
    pub(super) registry: Arc<SessionRegistry>,
    events: Arc<dyn AppEvents>,
    hub: Arc<NotificationHub>,
    /// pub(super): 핸드오프/입양 형제 모듈이 Running 세션 목록 조회·맵 제거에 쓴다.
    pub(super) sessions: Mutex<HashMap<AgentId, Arc<Session>>>,
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
    /// pub(super): 핸드오프/입양 형제 모듈이 소켓/로그 경로 계산에 직접 읽는다.
    pub(super) app_data_dir: Option<std::path::PathBuf>,
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

    /// pub(super): handoff_v1.rs/handoff_broker.rs가 세션 조회에 직접 쓴다.
    pub(super) fn find(&self, agent_id: &str) -> Option<Arc<Session>> {
        self.sessions.lock().get(agent_id).cloned()
    }

    /// agentId의 출력 sink를 반환(없으면 생성). attach_output이 세션보다 먼저
    /// 호출되면 여기서 sink가 만들어지고, create()는 같은 sink를 이어받아
    /// 이미 붙은 채널/백로그를 그대로 재사용한다.
    ///
    /// pub(super): handoff_v1.rs의 handoff_one이 backlog 스냅샷 폴백에 쓴다.
    pub(super) fn sink_for(&self, agent_id: &str) -> Arc<OutputSink> {
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
    ///
    /// pub(super): handoff_v1.rs의 adopt_one/handoff_broker.rs의 adopt_one_broker가
    /// 입양 재배선에 재사용한다.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn install_session(
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
