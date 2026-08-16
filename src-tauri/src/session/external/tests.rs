// src-tauri/src/session/external/tests.rs
//
// 외부(논리) 세션의 계약: 등록되면 기존 훅 파이프라인이 그대로 캐릭터 알림을
// 내고, 끊기면 그 sid의 훅은 폐기되며 아티팩트가 남지 않는다.

use super::*;
use crate::notification::hub::{NotificationHub, SystemClock};
use crate::observer::claude::ClaudeAdapter;
use crate::observer::{ObserverProvider, ObserverRuntime, RawObserverHook, WrapperArg};
use crate::session::pty_factory::fake::{FakeControl, FakePtyFactory};
use crate::state::fake::RecordingEvents;
use crate::state::{AppEvents, SessionRegistry};
use std::sync::Arc;
use std::time::Duration;

/// 훅이 켜진(=claude 어댑터가 settings 파일을 쓰는) SessionManager 한 벌.
/// PTY 팩토리는 단일 스폰 Fake라 세션을 만드는 테스트는 한 번만 만든다.
struct Fixture {
    manager: Arc<SessionManager>,
    events: Arc<RecordingEvents>,
    observer: Arc<ObserverRuntime>,
    registry: Arc<SessionRegistry>,
    control: Arc<FakeControl>,
    settings_dir: PathBuf,
}

impl Fixture {
    fn cleanup(&self) {
        // 리더 스레드가 EOF를 보게 해 영원히 블로킹되지 않게 한다.
        self.control.close_output();
        let _ = std::fs::remove_dir_all(&self.settings_dir);
    }

    /// forwarder가 `/hook`으로 보내는 것과 같은 경로로 훅을 흘려 넣는다.
    fn ingest(&self, session_id: &str, event_name: &str, body: &[u8]) {
        self.observer.ingest(
            ObserverProvider::Claude,
            session_id,
            RawObserverHook { event_name, body },
        );
    }
}

fn build() -> Fixture {
    let events = Arc::new(RecordingEvents::default());
    let registry = Arc::new(SessionRegistry::new());
    let hub = Arc::new(NotificationHub::new(
        registry.clone(),
        events.clone() as Arc<dyn AppEvents>,
        Arc::new(SystemClock),
        Duration::from_millis(3_000),
    ));
    // 테스트마다 고유 디렉터리 — 병렬 실행에서 settings 파일이 겹치지 않게.
    let settings_dir =
        std::env::temp_dir().join(format!("agent-office-external-test-{}", Uuid::new_v4()));
    let observer = Arc::new(ObserverRuntime::new(
        hub.clone(),
        vec![Arc::new(ClaudeAdapter::new(
            settings_dir.clone(),
            std::env::current_exe().unwrap(),
        ))],
    ));
    let (factory, control) = FakePtyFactory::new();
    let endpoint = Some("http://127.0.0.1:12345/hook".to_string());
    let manager = Arc::new(SessionManager::new(
        Arc::new(factory),
        observer.clone(),
        registry.clone(),
        events.clone() as Arc<dyn AppEvents>,
        hub,
        Arc::new(move || endpoint.clone()),
    ));
    Fixture {
        manager,
        events,
        observer,
        registry,
        control,
        settings_dir,
    }
}

fn req(agent_id: &str) -> CreateSessionRequest {
    CreateSessionRequest {
        agent_id: agent_id.into(),
        cols: None,
        rows: None,
        cwd: None,
        shell: None,
        startup_command: None,
        personality_prompt: None,
        autostart_claude: Some(false),
    }
}

#[tokio::test]
async fn attach_external_routes_hooks_to_the_agent() {
    let f = build();
    let outcome = f.manager.attach_external("a1", None, None, None).unwrap();
    assert!(outcome.is_new());
    let sid = outcome.session_id().to_string();

    assert_eq!(f.registry.resolve_agent(&sid).as_deref(), Some("a1"));
    assert_eq!(f.manager.session_id_for("a1").as_deref(), Some(sid.as_str()));

    f.ingest(&sid, "UserPromptSubmit", r#"{"prompt":"버그 고쳐줘"}"#.as_bytes());
    f.ingest(&sid, "Stop", b"{}");

    // Prompt 하나 + Stop이 동반하는 서브에이전트 카운트 하나. 둘 다 이 캐릭터로
    // 해석돼야 한다(레지스트리 해석 실패면 아예 방출되지 않는다).
    let activities = f.events.activities();
    assert_eq!(
        activities.iter().map(|a| a.kind).collect::<Vec<_>>(),
        vec![ActivityKind::Prompt, ActivityKind::SubCount]
    );
    assert!(activities
        .iter()
        .all(|a| a.agent_id == "a1" && a.session_id == sid));

    let notifications = f.events.notifications();
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].agent_id, "a1");
    assert_eq!(notifications[0].session_id, sid);
    // 큐에 쌓인 알림은 sid 폴백을 통해 캐릭터 이름으로도 조회된다.
    assert_eq!(f.manager.pending_notifications("a1").len(), 1);

    // session_started는 PTY 없음을 shell="external"로 알린다.
    let starts = f.events.session_starts();
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0].shell, "external");
    assert_eq!(starts[0].session_id, sid);

    let state = f.events.last_state();
    assert_eq!(state.state, SessionState::Running);
    assert_eq!(state.external, Some(true));

    f.cleanup();
}

#[tokio::test]
async fn attach_external_writes_the_same_hook_settings_and_persona_wiring() {
    let f = build();
    let outcome = f
        .manager
        .attach_external("a1", None, None, Some("차분하게 답한다."))
        .unwrap();
    let plan = outcome.plan();

    let settings = plan.settings_path.clone().expect("훅 ON이면 settings 경로");
    assert!(settings.exists());
    assert!(plan
        .env
        .contains(&("AGENT_OFFICE_SESSION".into(), outcome.session_id().into())));
    assert!(plan.env.contains(&(
        "AGENT_OFFICE_HOOK_URL".into(),
        "http://127.0.0.1:12345/hook".into()
    )));
    assert!(plan
        .env
        .contains(&("AGENT_OFFICE_PERSONA".into(), "차분하게 답한다.".into())));
    // PTY 전용 env는 공용 plan에 없다.
    assert!(plan.env.iter().all(|(key, _)| key != "TERM"));

    let claude = plan
        .wrappers
        .iter()
        .find(|wrapper| wrapper.command == "claude")
        .expect("claude 래퍼");
    assert_eq!(
        claude.prefix_args,
        vec![
            WrapperArg::Literal("--settings".into()),
            WrapperArg::Env("AGENT_OFFICE_SETTINGS".into()),
            WrapperArg::Literal("--append-system-prompt".into()),
            WrapperArg::Env("AGENT_OFFICE_PERSONA".into()),
        ]
    );

    f.cleanup();
}

#[tokio::test]
async fn attach_with_profile_puts_the_character_name_role_and_cwd_on_the_timeline() {
    let f = build();
    f.manager
        .attach_external_with_profile(
            "a1",
            None,
            Some("/tmp/proj"),
            None,
            crate::session_events::types::AgentEventProfile {
                name: "에이다".into(),
                role: Some("backend".into()),
            },
        )
        .unwrap();

    let starts = f.events.session_starts();
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0].agent_name, "에이다");
    assert_eq!(starts[0].agent_role.as_deref(), Some("backend"));
    assert_eq!(starts[0].cwd, "/tmp/proj");

    f.cleanup();
}

#[tokio::test]
async fn detach_external_stops_hook_routing_and_removes_the_settings_file() {
    let f = build();
    let outcome = f.manager.attach_external("a1", None, None, None).unwrap();
    let sid = outcome.session_id().to_string();
    let settings = outcome.plan().settings_path.clone().expect("settings 경로");
    assert!(settings.exists());

    assert!(f.manager.detach_external("a1", ExternalDetachReason::Detach));

    assert!(!settings.exists());
    assert_eq!(f.registry.resolve_agent(&sid), None);
    assert_eq!(f.manager.session_id_for("a1"), None);
    // 낡은 sid로 오는 훅은 해석 실패로 폐기된다.
    f.ingest(&sid, "Stop", b"{}");
    assert!(f.events.notifications().is_empty());
    // 두 번째 detach는 no-op(이벤트도 더 나오지 않는다).
    assert!(!f.manager.detach_external("a1", ExternalDetachReason::Detach));

    let states = f.events.states();
    assert_eq!(states, vec![SessionState::Running, SessionState::Disposed]);
    let last = f.events.last_state();
    assert_eq!(last.external, Some(true));
    assert!(last.exit.expect("exit 동반").intentional);

    f.cleanup();
}

#[tokio::test]
async fn attach_binds_to_a_live_in_app_session_without_registering_an_external() {
    let f = build();
    let created = f.manager.create(req("a1")).unwrap();

    let outcome = f
        .manager
        .attach_external("a1", Some(std::process::id()), None, None)
        .unwrap();
    assert!(!outcome.is_new());
    assert_eq!(outcome.session_id(), created.session_id);
    // BindExisting에서도 셸 스크립트를 렌더할 수 있게 plan은 준다.
    assert!(outcome
        .plan()
        .env
        .contains(&("AGENT_OFFICE_SESSION".into(), created.session_id.clone())));

    // externals에는 아무것도 안 들어갔다 → 끊을 외부 세션이 없다.
    assert!(!f.manager.detach_external("a1", ExternalDetachReason::Detach));
    // 상태 이벤트도 PTY 세션의 것뿐이다(외부 Running이 끼어들지 않는다).
    assert_eq!(
        f.events.states(),
        vec![SessionState::Starting, SessionState::Running]
    );

    f.cleanup();
}

#[tokio::test]
async fn create_session_detaches_a_previously_attached_external() {
    let f = build();
    let outcome = f.manager.attach_external("a1", None, None, None).unwrap();
    let external_sid = outcome.session_id().to_string();
    let external_settings = outcome.plan().settings_path.clone().expect("settings 경로");

    let created = f.manager.create(req("a1")).unwrap();

    assert_ne!(created.session_id, external_sid);
    assert_eq!(f.registry.resolve_agent(&external_sid), None);
    assert_eq!(
        f.manager.session_id_for("a1").as_deref(),
        Some(created.session_id.as_str())
    );
    // 외부 세션의 훅 아티팩트만 지워지고, 새 PTY 세션 것은 살아 있다.
    assert!(!external_settings.exists());
    assert_eq!(
        f.events.states(),
        vec![
            SessionState::Running,  // external attach
            SessionState::Disposed, // external 자동 detach
            SessionState::Starting, // PTY 세션
            SessionState::Running,
        ]
    );

    f.cleanup();
}

#[tokio::test]
async fn attaching_twice_reissues_a_fresh_session_id() {
    let f = build();
    let first = f.manager.attach_external("a1", None, None, None).unwrap();
    let first_sid = first.session_id().to_string();
    let first_settings = first.plan().settings_path.clone().expect("settings 경로");

    let second = f.manager.attach_external("a1", None, None, None).unwrap();
    assert!(second.is_new());
    assert_ne!(second.session_id(), first_sid);

    assert_eq!(f.registry.resolve_agent(&first_sid), None);
    assert!(!first_settings.exists());
    assert_eq!(
        f.registry.resolve_agent(second.session_id()).as_deref(),
        Some("a1")
    );

    f.cleanup();
}

#[tokio::test]
async fn external_session_never_looks_running_to_bot_mode() {
    let f = build();
    f.manager.attach_external("a1", None, None, None).unwrap();
    // write_input이 닿지 않는 세션이므로 봇 게이트는 false여야 한다.
    assert!(!f.manager.is_running("a1"));
    f.cleanup();
}

#[tokio::test]
async fn dispose_delegates_to_detach_for_an_external_session() {
    let f = build();
    let outcome = f.manager.attach_external("a1", None, None, None).unwrap();
    let settings = outcome.plan().settings_path.clone().expect("settings 경로");

    f.manager.dispose("a1");

    assert_eq!(f.manager.session_id_for("a1"), None);
    assert!(!settings.exists());
    assert_eq!(f.events.last_state().state, SessionState::Disposed);
    // PTY가 없으니 kill은 한 번도 불리지 않는다.
    assert_eq!(f.control.kill_count(), 0);

    f.cleanup();
}

#[cfg(unix)]
#[tokio::test]
async fn sweep_detaches_the_external_whose_shell_is_gone() {
    let f = build();
    // 확실히 죽은 PID: 자식을 띄우고 reap까지 끝낸 뒤 그 PID를 쓴다(임의의
    // 큰 수를 찍으면 호스트에 따라 살아 있는 프로세스일 수 있다).
    let mut child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("exit 0")
        .spawn()
        .expect("테스트용 단명 자식 프로세스 스폰");
    let dead_pid = child.id();
    let _ = child.wait();

    f.manager
        .attach_external("gone", Some(dead_pid), None, None)
        .unwrap();
    let alive = f
        .manager
        .attach_external("alive", Some(std::process::id()), None, None)
        .unwrap();
    let alive_sid = alive.session_id().to_string();

    f.manager.sweep_externals();

    assert_eq!(f.manager.session_id_for("gone"), None);
    assert_eq!(
        f.manager.session_id_for("alive").as_deref(),
        Some(alive_sid.as_str())
    );

    let last = f.events.last_state();
    assert_eq!(last.state, SessionState::Exited);
    assert_eq!(last.agent_id, "gone");
    assert_eq!(last.external, Some(true));
    assert!(!last.exit.expect("exit 동반").intentional);

    f.cleanup();
}

#[cfg(unix)]
#[tokio::test]
async fn sweep_leaves_externals_without_a_shell_pid_alone() {
    let f = build();
    let outcome = f.manager.attach_external("a1", None, None, None).unwrap();
    let sid = outcome.session_id().to_string();

    f.manager.sweep_externals();

    assert_eq!(f.manager.session_id_for("a1").as_deref(), Some(sid.as_str()));
    f.cleanup();
}
