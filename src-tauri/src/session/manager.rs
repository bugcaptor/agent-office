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
use crate::session::kilo_plugin;
use crate::session::pi_extension;
use crate::session::pty_factory::{ExitOutcome, PtyControl, PtyFactory, PtySpawnOptions, SpawnedPty};
use crate::session::shells;
use crate::session_events::types::{AgentEventProfile, SessionStartedEvent};
use crate::state::{AppEvents, SessionRegistry};
use crate::types::*;

/// pub(super): handoff_v1.rs(세션 핸드오프 형제 모듈)가
/// 필드에 직접 접근한다 -- 최소 가시성으로 `session` 모듈 트리 전체에만 연다.
pub(super) struct Session {
    pub(super) session_id: SessionId,
    agent_id: AgentId,
    pub(super) state: Mutex<SessionState>,
    writer: Mutex<Box<dyn Write + Send>>,
    pub(super) control: Arc<dyn PtyControl>,
    pub(super) cleanup_paths: Vec<std::path::PathBuf>,
    pub(super) kill_requested: AtomicBool,
    /// 시작 작업 디렉터리(세션 수명 동안 불변 -- `cd`는 추적하지 않는다).
    /// 핸드오프 시 Handoff 메시지의 진단/List용 메타데이터로 실어 보낸다.
    pub(super) cwd: String,
    /// 자동화 CLI 전환이 실제로 제출할 셸 프로그램. 직접 관리하는 PTY를 만들 때만
    /// 기록한다. 외부 attach·입양·tmux 호스팅은 반환 receipt를 증명할 수 없으므로
    /// None으로 남겨 자동 반환 확인 capability를 광고하지 않는다.
    shell_path: Mutex<Option<String>>,
    /// 세션 로그 기록 핸들(docs/session-log-design.md). 저장소 루트가 없거나
    /// 파일을 못 열면 None -- 로그는 부가 기능이라 세션 동작을 막지 않는다.
    pub(super) log: Option<Arc<crate::session_log::SessionLogHandle>>,
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
    /// pub(super): handoff_v1.rs의 adopt_one이
    /// `restore_session_artifacts`를 직접 호출한다.
    pub(super) observer: Arc<ObserverRuntime>,
    get_observer_url: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    /// 관찰이 꺼진 세션에 물릴 사무실 shim 권한 전용 설정 파일. sid를 주면 그
    /// 파일 경로를 돌려준다. 승인할 shim이 없으면 None.
    get_office_settings: Arc<dyn Fn(&str) -> Option<std::path::PathBuf> + Send + Sync>,
    /// pub(super): 핸드오프/입양 형제 모듈이 세션 제거 시 레지스트리도 함께 정리한다.
    pub(super) registry: Arc<SessionRegistry>,
    /// pub(super): external.rs가 외부(논리) 세션의 started/state 이벤트를 낸다.
    pub(super) events: Arc<dyn AppEvents>,
    /// pub(super): external.rs가 detach 시 미해결 알림을 purge한다.
    pub(super) hub: Arc<NotificationHub>,
    /// pub(super): 핸드오프/입양 형제 모듈이 Running 세션 목록 조회·맵 제거에 쓴다.
    pub(super) sessions: Mutex<HashMap<AgentId, Arc<Session>>>,
    /// create·dispose·on_exit를 한 줄로 세운다 — 옛 PTY의 마지막 상태 이벤트가
    /// 같은 캐릭터의 새 세션 Running보다 뒤에 도착하지 않게 한다.
    lifecycle: Mutex<()>,
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
    /// `office-task` shim 경로와 shim이 실행할 앱 바이너리(docs/work-panels-design.md §업무 수신함).
    /// None이면 shim과 `AGENT_OFFICE_TASK`를 생략한다 -- 테스트 기본값이다.
    task_cli: Option<(std::path::PathBuf, std::path::PathBuf)>,
    /// 업무 패널 설정(`workPanelsEnabled`) 조회. 켜졌을 때만 공통 안내문과 Codex
    /// 수신함 쓰기 허용을 싣는다. 세션을 띄울 때 고정된다.
    work_panels_enabled: Arc<dyn Fn() -> bool + Send + Sync>,
    /// 사용자 Codex `config.toml` 본문 조회(writable_roots 합치기). 테스트는 주입한다.
    codex_user_config: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    /// 세션 로그 저장소 루트(`<app_data>/session-logs/v1`). None이면 기록하지
    /// 않는다 -- 테스트 기본값이다.
    session_log_root: Option<std::path::PathBuf>,
    /// 설정 `sessionLogEnabled`의 런타임 미러. 설정 변경이 즉시 반영되도록
    /// 원자 플래그를 세션들과 공유한다.
    session_log_enabled: Arc<AtomicBool>,
    /// 캐릭터별 Claude native 세션 ID 조회기(프로덕션은 `ClaudeResumeStore`).
    /// 세션 로그가 Claude 자체 JSONL 전사를 찾아 붙는 데 쓴다. None이면 PTY
    /// 전사만 남는다 -- 테스트 기본값이다.
    agent_session_lookup: Option<Arc<dyn crate::session_log::agent_transcript::AgentSessionLookup>>,
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
            get_office_settings: Arc::new(|_| None),
            registry,
            events,
            hub,
            sessions: Mutex::new(HashMap::new()),
            lifecycle: Mutex::new(()),
            sinks: Mutex::new(HashMap::new()),
            shell_resolver: Arc::new(shells::resolve_observed),
            app_data_dir: None,
            task_cli: None,
            work_panels_enabled: Arc::new(|| false),
            codex_user_config: Arc::new(|| {
                let path = crate::agent_paths::codex_home_from_env()?.join("config.toml");
                std::fs::read_to_string(path).ok()
            }),
            session_log_root: None,
            session_log_enabled: Arc::new(AtomicBool::new(true)),
            agent_session_lookup: None,
        }
    }

    /// 세션 로그 기록을 켠다(docs/session-log-design.md). `lib.rs`가 앱 데이터
    /// 디렉터리를 알게 된 시점에 호출한다. `enabled`는 설정 토글과 공유하는
    /// 원자 플래그 -- 세션이 도는 중에 꺼도 즉시 반영된다.
    pub fn with_session_log(
        mut self,
        root: std::path::PathBuf,
        enabled: Arc<AtomicBool>,
    ) -> Self {
        self.session_log_root = Some(root);
        self.session_log_enabled = enabled;
        self
    }

    /// 세션 로그가 Claude 자체 JSONL 전사를 찾아 붙일 수 있게 조회기를 준다
    /// (docs/session-log-design.md §3.4). 없으면 대체 화면 안의 대화는 로그에
    /// 남지 않는다.
    pub fn with_agent_session_lookup(
        mut self,
        lookup: Arc<dyn crate::session_log::agent_transcript::AgentSessionLookup>,
    ) -> Self {
        self.agent_session_lookup = Some(lookup);
        self
    }

    /// 사무실 shim 권한 설정 주입(lib.rs). 기본은 no-op이라 테스트는 신경 쓰지 않는다.
    pub fn with_office_settings(
        mut self,
        provider: Arc<dyn Fn(&str) -> Option<std::path::PathBuf> + Send + Sync>,
    ) -> Self {
        self.get_office_settings = provider;
        self
    }

    /// 앱 데이터 디렉터리를 지정한다(세션 핸드오프 소켓/로그 경로,
    /// `AGENT_OFFICE_APP_DATA` env의 근거). `lib.rs`의 프로덕션 부트스트랩만
    /// 호출 — 테스트는 기본 None으로 이 기능을 건드리지 않는다.
    pub fn with_app_data_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.app_data_dir = Some(dir);
        self
    }

    /// 세션 준비마다 `office-task` shim을 이 바이너리로 (재)작성하고 경로를 env로 싣는다.
    pub fn with_task_cli(mut self, exe: std::path::PathBuf) -> Self {
        self.task_cli = Some((crate::work_inbox_cli::shim_path(), exe));
        self
    }

    /// 업무 패널 설정 조회기를 준다(lib.rs). 기본은 꺼짐.
    pub fn with_work_panels_flag(mut self, enabled: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        self.work_panels_enabled = enabled;
        self
    }

    /// pub(super): handoff_v1.rs가 세션 조회에 직접 쓴다.
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

    /// agentId의 현재 sid.
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

    /// sid 하나분의 observer 훅 배선 + pi 확장 + persona 래퍼를 준비한다.
    /// `create_with_profile`(PTY 세션)과 `attach_external`(외부 논리 세션)이
    /// 공유한다 — 어느 쪽으로 붙어도 같은 훅 settings 파일과 같은 persona
    /// 주입을 받는다.
    ///
    /// 부작용: 훅이 켜져 있으면 claude 설정 파일을 디스크에 쓴다(어댑터의
    /// temp+rename 멱등 기록이라 같은 sid로 다시 불러도 안전하다). 어댑터
    /// 준비 실패는 로그만 남고 비관찰로 강등된다(ObserverRuntime 계약).
    ///
    /// pub(super): external.rs의 `attach_external`이 재사용한다.
    pub(super) fn prepare_session_plan(
        &self,
        session_id: &str,
        agent_id: &str,
        personality_prompt: Option<&str>,
    ) -> PreparedPlan {
        let observer_url = (self.get_observer_url)();
        let mut plan = observer_url
            .as_deref()
            .map(|url| {
                self.observer
                    .prepare_session(&ObserverSessionContext::new(session_id, url))
            })
            .unwrap_or_default();
        if observer_url.is_some() {
            match pi_extension::ensure_extension(self.app_data_dir.as_deref()) {
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
                        // 확장 파일은 app_data(`<app_data>/observer/pi`)에 있고
                        // 부팅 때마다 다시 쓰이지만, app_data가 없는 구성은 여전히
                        // OS temp로 떨어지고 사용자가 파일을 지울 수도 있다.
                        // `pi -e <없는 경로>`는 경고가 아니라 **하드 실패**다
                        // ("Extension path does not exist" 후 즉시 종료, pi v0.84.2
                        // 실측) — claude와 같은 강등 가드를 걸어, 파일이 사라지면
                        // 관찰만 포기하고 pi 실행은 보장한다(이슈 #40과 동일 취지).
                        skip_prefix_if_env_file_missing: Some("AGENT_OFFICE_PI_EXT".into()),
                        ..Default::default()
                    });
                }
                Err(error) => eprintln!("agent-office: failed to write pi extension: {error}"),
            }
        }

        // Kilo Code CLI(kilo/kilocode, docs/kilo-support-design.md): 플러그인
        // 설정 JSON을 `KILO_CONFIG` env로 가리킨다(전역 `~/.config/kilo/kilo.jsonc`와
        // 병합, 스파이크 실측). 사용자가 이미 자기 `KILO_CONFIG`를 쓰고 있으면
        // (skip_if_env_set) 덮어쓰지 않고 관찰만 포기한다 — pi의 파일-부재 강등과
        // 같은 취지지만 여기서는 "사용자 설정과 충돌"이 강등 사유다. 두 커맨드
        // 이름(kilo/kilocode) 모두에 같은 배선을 건다.
        if observer_url.is_some() {
            match kilo_plugin::ensure_plugin(self.app_data_dir.as_deref()) {
                Ok(path) => {
                    plan.env.push((
                        "AGENT_OFFICE_KILO_CONFIG".into(),
                        path.to_string_lossy().into_owned(),
                    ));
                    for command in ["kilo", "kilocode"] {
                        plan.wrappers.push(CommandWrapperSpec {
                            command: command.into(),
                            prefix_args: vec![],
                            skip_if_present: vec![],
                            set_env_from_env: vec![(
                                "KILO_CONFIG".into(),
                                "AGENT_OFFICE_KILO_CONFIG".into(),
                            )],
                            skip_if_env_set: Some("KILO_CONFIG".into()),
                            // 설정 파일이 사라졌으면(app_data 없는 구성이 OS temp를
                            // 쓰다 청소된 경우 등) env 대입 없이 원본 명령을 실행한다
                            // — pi의 `pi -e <없는 경로>` 하드 실패 가드와 같은 취지.
                            skip_prefix_if_env_file_missing: Some(
                                "AGENT_OFFICE_KILO_CONFIG".into(),
                            ),
                            ..Default::default()
                        });
                    }
                }
                Err(error) => eprintln!("agent-office: failed to write kilo plugin: {error}"),
            }
        }

        // Antigravity CLI(agy, docs/antigravity-support-design.md): 전역
        // hooks.json 병합(부팅 시 1회, lib.rs)이 관찰의 전부다 -- agy() 래퍼는
        // prefix 인자가 없다. 유일한 역할은 §4 스파이크 실측(workspacePaths가
        // 빈 배열로 올 수 있음)의 cwd fallback으로, 호출 한 번에만 붙는 앞자리
        // 대입으로 `$PWD`를 `AGENT_OFFICE_AGY_CWD`에 실어 보내는 것 -- hook.sh가
        // 이 값을 `X-Agent-Office-Cwd` **헤더**로 서버 POST에 얹는다(쿼리
        // 문자열이 아니다 -- POSIX sh에 표준 percent-encoding이 없어 공백·
        // 비ASCII 경로가 깨진다, observer/server.rs AGY_CWD_HEADER 참고).
        if observer_url.is_some() {
            plan.wrappers.push(CommandWrapperSpec {
                command: "agy".into(),
                prefix_args: vec![],
                skip_if_present: vec![],
                export_cwd_env: Some("AGENT_OFFICE_AGY_CWD".into()),
                ..Default::default()
            });
        }

        // 사무실 shim(office-task) 권한 사전 승인: 관찰이 꺼져 있으면 훅 설정
        // 파일 자체가 없으므로 권한 조각만 담은 설정 파일을 따로 만들어
        // `--settings`로 물린다(켜져 있으면 어댑터가 같은 조각을 훅 설정에 합쳐
        // 놓았다).
        let has_settings = plan
            .env
            .iter()
            .any(|(key, _)| key == "AGENT_OFFICE_SETTINGS");
        if !has_settings {
            if let Some(path) = (self.get_office_settings)(session_id) {
                plan.env.push((
                    "AGENT_OFFICE_SETTINGS".into(),
                    path.to_string_lossy().into_owned(),
                ));
                let claude_args = vec![
                    WrapperArg::Literal("--settings".into()),
                    WrapperArg::Env("AGENT_OFFICE_SETTINGS".into()),
                ];
                plan.cleanup_paths.push(path);
                if let Some(claude) = plan
                    .wrappers
                    .iter_mut()
                    .find(|wrapper| wrapper.command == "claude")
                {
                    claude.prefix_args.extend(claude_args);
                } else {
                    plan.wrappers.push(CommandWrapperSpec {
                        command: "claude".into(),
                        prefix_args: claude_args,
                        skip_if_present: vec![],
                        // 설정 파일이 사라졌으면 인자만 포기하고 claude는 띄운다
                        // (없는 경로를 주면 claude가 하드 실패한다).
                        skip_prefix_if_env_file_missing: Some("AGENT_OFFICE_SETTINGS".into()),
                        ..Default::default()
                    });
                }
            }
        }

        // 업무 수신함 shim(docs/work-panels-design.md §업무 수신함). OS temp는 청소될 수 있어 매번 쓴다.
        let task_shim = self.task_cli.as_ref().and_then(|(shim, exe)| {
            crate::work_inbox_cli::ensure_shim(shim, exe)
                .map_err(|error| eprintln!("agent-office: failed to write office-task shim: {error}"))
                .ok()
                .map(|()| shim.clone())
        });
        let work_inbox = self.app_data_dir.as_ref().map(|dir| dir.join("work-inbox"));
        let office_enabled = task_shim.is_some() && (self.work_panels_enabled)();
        let office_instructions = task_shim
            .as_deref()
            .filter(|_| office_enabled)
            .map(crate::office_instructions::task_instructions);

        // 덧붙일 지시문 = persona + 빈 줄 + 사무실 안내문(있는 것만). env 이름
        // `AGENT_OFFICE_PERSONA`는 래퍼 함수 호환과 IDE 페르소나 경로 때문에 유지한다.
        if let Some(appended) = crate::office_instructions::appended_prompt(
            personality_prompt,
            office_instructions.as_deref(),
        ) {
            let personality_prompt = appended.as_str();
            plan.env.push((
                "AGENT_OFFICE_PERSONA".into(),
                personality_prompt.to_string(),
            ));
            let persona_args = [
                WrapperArg::Literal("--append-system-prompt".into()),
                WrapperArg::Env("AGENT_OFFICE_PERSONA".into()),
            ];
            merge_wrapper_args(&mut plan.wrappers, "claude", persona_args.into());

            // Codex에는 `--append-system-prompt`가 없다. 같은 자리(기본
            // 지침을 덮지 않고 **덧붙이는** 채널)는 `developer_instructions`
            // 설정값이다 — `-c` 한 방으로 주입되고, codex 자신의 정체성·기본
            // 지침은 그대로 둔 채 지시만 얹힌다(codex-cli 0.149 실측).
            // 참고: `instructions`는 기본 지침을 통째로 **대체**하므로 쓰면
            // 안 된다.
            plan.env.push((
                "AGENT_OFFICE_CODEX_PERSONA".into(),
                codex_persona_config(personality_prompt),
            ));
            merge_wrapper_args(
                &mut plan.wrappers,
                "codex",
                vec![
                    WrapperArg::Literal("-c".into()),
                    WrapperArg::Env("AGENT_OFFICE_CODEX_PERSONA".into()),
                ],
            );
        }

        // Pi·Kilo는 persona를 받지 않으므로 안내문만 따로 싣는다. Pi는 덧붙이기 플래그
        // (pi 1.1.0 `--append-system-prompt`), Kilo는 앱 소유 플러그인의
        // `experimental.chat.system.transform` 훅이 이 env를 읽는다(플러그인은 관찰이
        // 켜졌을 때만 실린다). agy는 덧붙이기 채널이 없어 `AGENT_OFFICE_TASK`만 준다.
        if let Some(office) = &office_instructions {
            plan.env.push(("AGENT_OFFICE_OFFICE_PROMPT".into(), office.clone()));
            merge_wrapper_args(
                &mut plan.wrappers,
                "pi",
                vec![
                    WrapperArg::Literal("--append-system-prompt".into()),
                    WrapperArg::Env("AGENT_OFFICE_OFFICE_PROMPT".into()),
                ],
            );
        }

        // Codex 기본 workspace-write 샌드박스는 작업 폴더 밖 쓰기를 막는다. 안내문을
        // 실은 세션에서는 수신함을 쓰기 허용 경로에 더한다.
        if let Some(inbox) = work_inbox.as_deref().filter(|_| office_enabled) {
            plan.env.push((
                "AGENT_OFFICE_CODEX_WRITABLE_ROOTS".into(),
                crate::office_instructions::codex_writable_roots_config(
                    (self.codex_user_config)().as_deref(),
                    inbox,
                ),
            ));
            merge_wrapper_args(
                &mut plan.wrappers,
                "codex",
                vec![
                    WrapperArg::Literal("-c".into()),
                    WrapperArg::Env("AGENT_OFFICE_CODEX_WRITABLE_ROOTS".into()),
                ],
            );
        }

        let mut env = vec![("AGENT_OFFICE_SESSION".into(), session_id.to_string())];
        if let Some(url) = observer_url {
            env.push(("AGENT_OFFICE_HOOK_URL".into(), url));
        }
        // §핵심 5: 재시작 후 입양된 세션의 훅이 스폰 시점의(죽은) 포트를
        // 때리는 문제 완화 -- forwarder가 이 경로의 observer-port 파일을
        // 읽어 재시도할 수 있게 셸 env에 앱 데이터 디렉터리를 실어 둔다.
        if let Some(dir) = &self.app_data_dir {
            env.push((
                "AGENT_OFFICE_APP_DATA".into(),
                dir.to_string_lossy().into_owned(),
            ));
        }
        // 업무 수신함(docs/work-panels-design.md §업무 수신함): 업무 패널 설정과 무관하게 싣는다.
        // 수신함 규약은 패널이 꺼져 있어도 유효하다.
        env.push(("AGENT_OFFICE_AGENT_ID".into(), agent_id.to_string()));
        if let Some(inbox) = &work_inbox {
            env.push((
                "AGENT_OFFICE_WORK_INBOX".into(),
                inbox.to_string_lossy().into_owned(),
            ));
        }
        if let Some(shim) = &task_shim {
            env.push(("AGENT_OFFICE_TASK".into(), shim.to_string_lossy().into_owned()));
        }
        env.extend(plan.env.iter().cloned());
        let settings_path = env
            .iter()
            .rev()
            .find(|(key, _)| key == "AGENT_OFFICE_SETTINGS")
            .map(|(_, value)| std::path::PathBuf::from(value));

        PreparedPlan {
            env,
            wrappers: plan.wrappers,
            cleanup_paths: plan.cleanup_paths,
            settings_path,
        }
    }

    /// 1 에이전트 1 세션 불변식. self: &Arc<Self>로 wait 스레드에 소유 이전.
    pub fn create_with_profile(
        self: &Arc<Self>,
        req: CreateSessionRequest,
        profile: AgentEventProfile,
    ) -> Result<CreateSessionResult, String> {
        let _lifecycle = self.lifecycle.lock();

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
        let prepared =
            self.prepare_session_plan(&session_id, &req.agent_id, req.personality_prompt.as_deref());

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
            paths: prepared.cleanup_paths.clone(),
            armed: true,
        };

        let resolved = (self.shell_resolver)(req.shell.as_deref(), &prepared.wrappers);
        let cwd = req.cwd.clone().map(expand_tilde).unwrap_or_else(home_dir);
        let PreparedPlan {
            mut env,
            cleanup_paths: plan_cleanup_paths,
            settings_path,
            ..
        } = prepared;
        // TERM은 PTY 세션 전용이라 공용 plan에 넣지 않는다(외부 세션은 자기
        // 터미널의 TERM을 그대로 쓴다). 기존 env 순서(SESSION, TERM, ...)를
        // 그대로 보존하려고 여기서 두 번째 자리에 끼워 넣는다.
        env.insert(1, ("TERM".into(), "xterm-256color".into()));
        env.extend(resolved.extra_env.iter().cloned());
        let actual_shell = resolved.program.clone();
        let actual_cwd = cwd.clone();

        let spawned = match self.factory.spawn(PtySpawnOptions {
            shell: resolved.program,
            args: resolved.args,
            cols: req.cols.unwrap_or(80),
            rows: req.rows.unwrap_or(24),
            cwd,
            env,
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
            shell: actual_shell.clone(),
            at: now_ms(),
        });

        let size = (req.cols.unwrap_or(80), req.rows.unwrap_or(24));
        let (session, started) = self.install_session(
            session_id.clone(),
            req.agent_id.clone(),
            plan_cleanup_paths,
            actual_cwd,
            size,
            spawned,
            None, // eof_stop_gate: create() 경로는 RealWaiter가 독립적으로 exit 판정
            None, // initial_output: 입양 전용
        );
        // `ResolvedShell`은 이 직접 생성 경로에서만 실제로 PTY에 전달된 값이다.
        // 외부 attach, 재입양은 이 근거를 갖지 않으므로 auto receipt 경로를 열지
        // 않는다.
        if !req.startup_command.as_deref().is_some_and(|command| !command.trim().is_empty())
            && !req.autostart_claude.unwrap_or(false)
        {
            *session.shell_path.lock() = Some(actual_shell);
        }
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
            if let Err(error) = session.writer.lock().write_all(line.as_bytes()) {
                eprintln!("agent-office: autostart write failed for {session_id}: {error}");
            }
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
                    if let Err(error) = session.writer.lock().write_all(line.as_bytes()) {
                        eprintln!(
                            "agent-office: startup command write failed for {session_id}: {error}"
                        );
                    }
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
    /// pub(super): handoff_v1.rs의 adopt_one이
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
        // 셸 작업중 감시(kbm #2f9)에 필요한 두 조각을 Session이 소유하기 전에
        // 떠 둔다. control은 Arc라 복제해도 같은 마스터를 가리킨다.
        #[cfg(unix)]
        let watch_control = spawned.control.clone();
        // 셸 자신의 pgid. spawn 시점의 포그라운드 그룹이 곧 셸이다 -- 알면
        // 주지 않아도 tracker가 첫 샘플에서 래치하지만, 명령이 이미 도는 채로
        // 입양된 세션에서 오판하지 않도록 아는 값을 우선한다.
        #[cfg(unix)]
        let watch_shell_pgid = spawned.handoff.as_ref().and_then(|h| h.pgid);
        // 세션 로그 기록 시작(§3.1). 같은 sessionId의 파일이 이미 있으면
        // 이어 쓴다 -- 앱 재시작 후 입양된 세션이 한 파일로 이어진다.
        //
        // 설정이 꺼져 있으면 핸들 자체를 만들지 않는다: 껐는데도 헤더(작업 폴더
        // 경로 포함)만 든 파일이 세션마다 생기면 "끔"이 아니다. 대신 세션이 도는
        // 중에 끄는 것은 원자 플래그로 즉시 반영된다(다음 세션부터 다시 켜진다).
        let log = self
            .session_log_root
            .as_ref()
            .filter(|_| self.session_log_enabled.load(Ordering::Relaxed))
            .and_then(|root| {
            crate::session_log::SessionLogHandle::spawn(
                root,
                &agent_id,
                &session_id,
                &cwd,
                size.1,
                self.session_log_enabled.clone(),
                self.agent_session_lookup.clone(),
            )
            .map(Arc::new)
        });
        let session = Arc::new(Session {
            session_id: session_id.clone(),
            agent_id: agent_id.clone(),
            state: Mutex::new(SessionState::Starting),
            writer: Mutex::new(spawned.writer),
            control: spawned.control,
            cleanup_paths,
            kill_requested: AtomicBool::new(false),
            log: log.clone(),
            cwd,
            shell_path: Mutex::new(None),
            size: Mutex::new(size),
            handed_off: AtomicBool::new(false),
            #[cfg(unix)]
            reader_interrupt: Mutex::new(spawned.reader_interrupt),
            #[cfg(unix)]
            handoff: Mutex::new(spawned.handoff),
            eof_stop_gate,
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

        // 2) output pump task (배칭 + BEL 감지 + Channel 방출 + 세션 로그 tee)
        spawn_output_pump(
            session_id.clone(),
            agent_id.clone(),
            rx,
            output,
            self.hub.clone(),
            log,
        );

        // 3) wait thread (블로킹 wait → 상태 전이)
        let me = Arc::clone(self);
        let sess = session.clone();
        let waiter = spawned.waiter;
        std::thread::spawn(move || {
            let outcome = waiter.wait();
            me.on_exit(&sess, outcome);
        });

        // 4) 셸 작업중 감시 스레드(kbm #2f9, unix + 앱이 마스터 fd를 쥔 세션 한정).
        // 포그라운드 프로세스 그룹을 못 보는 컨트롤(브로커 데몬 소유·Fake)에서는
        // 아예 띄우지 않는다 -- 감시가 없으면 기존 동작 그대로다.
        #[cfg(unix)]
        if watch_control.foreground_sample().is_some() {
            let weak = Arc::downgrade(&session);
            crate::session::shell_activity::spawn_watcher(
                session_id.clone(),
                watch_control,
                self.hub.clone(),
                watch_shell_pgid,
                Arc::new(move || {
                    weak.upgrade().is_some_and(|s| {
                        !s.handed_off.load(Ordering::SeqCst)
                            && matches!(
                                *s.state.lock(),
                                SessionState::Starting | SessionState::Running
                            )
                    })
                }),
            );
        }

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
                // 실패해도 IPC 계약상 돌려줄 곳이 없다(반환값 없음). 그래도
                // 조용히 삼키면 "키를 눌렀는데 아무 일도 없다"가 되므로 최소한
                // 로그는 남긴다 -- 세션이 방금 죽었거나 PTY가 닫힌 경우다.
                if let Err(error) = s.writer.lock().write_all(data.as_bytes()) {
                    eprintln!(
                        "agent-office: input write failed for {}: {error}",
                        s.session_id
                    );
                }
            }
        }
    }

    /// 자동 입력은 찾은 세션 객체에 고정한다. 확인 직후 같은 탭에 새 세션이
    /// 생겨도 다시 agent_id로 조회해서 새 세션에 쓰는 일이 없어야 한다.
    pub fn write_input_for_session(&self, agent_id: &str, session_id: &str, data: &str) -> bool {
        let Some(session) = self.find(agent_id) else { return false; };
        if session.session_id != session_id || *session.state.lock() != SessionState::Running {
            return false;
        }
        let written = match session.writer.lock().write_all(data.as_bytes()) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("agent-office: input write failed for {}: {error}", session.session_id);
                false
            }
        };
        written
    }

    /// 세션이 살아서 입력을 받을 수 있는 상태(Running)인지.
    pub fn is_running(&self, agent_id: &str) -> bool {
        self.find(agent_id)
            .map(|s| *s.state.lock() == SessionState::Running)
            .unwrap_or(false)
    }

    /// 원래 셸이 foreground인지를 확인한다(Windows는 셸의 자손 프로세스 유무). 브로커/미지원 조회는 None이며
    /// 보내기 UI는 기존 세션에 대해 재시작 확인으로 처리한다.
    pub fn is_shell_prompt(&self, agent_id: &str) -> Option<bool> {
        #[cfg(unix)]
        {
            let session = self.find(agent_id)?;
            session.shell_path.lock().as_ref()?;
            let shell_pgid = session.handoff.lock().as_ref()?.pgid?;
            let fg = session.control.foreground_sample()?.fg_pgid?;
            if shell_pgid != fg {
                return Some(false);
            }
            // `set +m`에서는 CLI도 셸과 같은 pgid를 쓸 수 있다. 같은 그룹에
            // 자식이 남아 있으면 빈 셸로 단정하지 않는다. 조회 실패도 확인 대상으로 둔다.
            let members = std::process::Command::new("pgrep")
                .args(["-g", &shell_pgid.to_string()])
                .output()
                .ok()?;
            if !members.status.success() {
                return None;
            }
            let count = crate::session::shell_activity::count_pgrep_pids(&String::from_utf8_lossy(&members.stdout));
            Some(count == 1)
        }
        // ConPTY엔 foreground 그룹이 없다. 셸 프로세스에 자손이 없을 때만 빈 셸이다.
        #[cfg(windows)]
        {
            let session = self.find(agent_id)?;
            let shell_path = session.shell_path.lock().clone()?;
            let pid = session.control.process_id()?;
            crate::session::win_process::shell_is_idle(pid, &shell_path)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = agent_id;
            None
        }
    }

    /// 직접 관리하는 PTY의 실제 셸 실행 경로. attach/입양/tmux는 None이다.
    pub fn shell_path_for(&self, agent_id: &str) -> Option<String> {
        self.find(agent_id)
            .and_then(|session| session.shell_path.lock().clone())
    }

    pub fn resize(&self, agent_id: &str, cols: u16, rows: u16) {
        if let Some(s) = self.find(agent_id) {
            if *s.state.lock() == SessionState::Running {
                if let Err(error) = s.control.resize(cols, rows) {
                    eprintln!("agent-office: resize failed for {}: {error}", s.session_id);
                }
                *s.size.lock() = (cols, rows);
                // 전사 필터의 라이브 영역은 화면 높이에서 나온다(§3.3).
                if let Some(log) = s.log.as_ref() {
                    log.resize(rows);
                }
            }
        }
    }

    /// 의도적 종료. 최종 Disposed 전이는 wait 스레드의 on_exit에서 확정.
    /// 핸드오프된 세션(§핵심 3)은 즉시 return — kill/cleanup 금지. 그
    /// 세션의 실제 수명은 이제 sessiond가 책임진다.
    pub fn dispose(&self, agent_id: &str) {
        let _ = self.dispose_matching(agent_id, None);
    }

    /// 확인받은 세션만 폐기한다. create와 같은 lifecycle 락 아래 검사/폐기를 묶는다.
    pub fn dispose_matching(&self, agent_id: &str, expected: Option<&str>) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        if expected.is_some_and(|sid| self.find(agent_id).as_ref().is_none_or(|s| s.session_id != sid)) {
            return Err("work-send-session-changed".into());
        }
        if let Some(s) = self.find(agent_id) {
            if s.handed_off.load(Ordering::SeqCst) {
                return Err("work-send-session-changed".into());
            }
            s.kill_requested.store(true, Ordering::SeqCst);
            if let Err(error) = s.control.kill() {
                // 이미 죽은 자식이면 흔한 실패다 -- 그래도 남겨 둬야 "kill했는데
                // 계속 살아 있다"를 사후에 구분할 수 있다.
                eprintln!("agent-office: kill failed for {}: {error}", s.session_id);
            }
            cleanup_paths(&s.cleanup_paths);
        }
        Ok(())
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


    /// 세션의 시작 작업 디렉터리(자동화 점검용, kbm). 외부(논리) 세션·없는
    /// 세션은 None. `expand_tilde`가 이미 적용된 실제 경로다.
    pub fn cwd_of(&self, agent_id: &str) -> Option<String> {
        self.find(agent_id).map(|s| s.cwd.clone())
    }

    pub fn pending_notifications(&self, agent_id: &str) -> Vec<NotificationEvent> {
        match self.session_id_for(agent_id) {
            Some(sid) => self.hub.pending(&sid),
            None => Vec::new(),
        }
    }

    fn on_exit(&self, sess: &Arc<Session>, outcome: ExitOutcome) {
        // Keep the old PTY's final state event ordered with replacement by a
        // new PTY: a stale exit must not follow the new Running event.
        let _lifecycle = self.lifecycle.lock();
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

    pub(super) fn emit_state(&self, sess: &Arc<Session>, state: SessionState, exit: Option<SessionExitInfo>) {
        self.events.session_state(&SessionStateEvent {
            session_id: sess.session_id.clone(),
            agent_id: sess.agent_id.clone(),
            state,
            exit,
            at: now_ms(),
            // PTY 세션 -- 외부(논리) 세션 표시는 external.rs가 붙인다.
            external: None,
        });
    }
}

/// `prepare_session_plan`의 산출물 -- 세션 하나에 필요한 observer/persona 배선.
pub struct PreparedPlan {
    /// 셸에 넣을 env. 순서는 AGENT_OFFICE_SESSION → HOOK_URL(훅 ON) →
    /// APP_DATA(설정 시) → 어댑터 plan.env. PTY 경로는 여기에 TERM과 셸
    /// 리졸버의 extra_env를 더한다.
    pub env: Vec<(String, String)>,
    /// 셸 함수로 렌더할 명령 래퍼(claude/pi ...).
    pub wrappers: Vec<CommandWrapperSpec>,
    /// 세션 종료 시 지울 observer 아티팩트(claude 훅 settings 파일 등).
    pub cleanup_paths: Vec<std::path::PathBuf>,
    /// `AGENT_OFFICE_SETTINGS` 값(훅 OFF면 None). autostart 주입 줄이 쓴다.
    pub settings_path: Option<std::path::PathBuf>,
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

/// 같은 command의 래퍼가 이미 있으면 prefix_args를 이어 붙이고, 없으면 새
/// 래퍼를 하나 만든다. persona 주입이 관찰 배선(어댑터가 만든 래퍼) 위에
/// 얹히는 자리라 claude·codex 두 갈래가 같은 규칙을 쓴다.
fn merge_wrapper_args(
    wrappers: &mut Vec<CommandWrapperSpec>,
    command: &str,
    args: Vec<WrapperArg>,
) {
    if let Some(existing) = wrappers.iter_mut().find(|wrapper| wrapper.command == command) {
        existing.prefix_args.extend(args);
    } else {
        wrappers.push(CommandWrapperSpec {
            command: command.into(),
            prefix_args: args,
            skip_if_present: vec![],
            ..Default::default()
        });
    }
}

/// persona 텍스트 → codex `-c` 인자 하나(`developer_instructions="..."`).
///
/// codex는 `-c key=value`의 value를 TOML로 파싱하고 실패하면 원문 문자열로
/// 쓰지만, 그 폴백에 기대면 따옴표로 시작하는 persona가 어중간하게 파싱될
/// 수 있다. 그래서 **항상 유효한 TOML 기본 문자열**로 만든다: 역슬래시·
/// 큰따옴표를 이스케이프하고 개행·탭은 `\n`·`\t`로, 나머지 제어문자는
/// 버린다(개행이 그대로 들어가면 TOML 기본 문자열이 깨진다).
fn codex_persona_config(prompt: &str) -> String {
    format!(
        "developer_instructions={}",
        crate::office_instructions::toml_basic_string(prompt)
    )
}

