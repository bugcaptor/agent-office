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
                    ..Default::default()
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
                    ..Default::default()
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
