    // Assert each command *body* delegates correctly into
    // manager/hub/store. `tauri::State<'_, AppState>` cannot be constructed
    // standalone (it borrows from a live `tauri::App`/`AppHandle`), so
    // instead of driving the `#[tauri::command]`-wrapped async fns directly,
    // these tests build a real `AppState` (fakes for PtyFactory/AppEvents,
    // tempdir-backed ProfileStore/ObserverRuntime -- the same seams
    // other test modules use) and call the exact `app_state.manager` /
    // `app_state.hub` / `app_state.store` method sequence each command body
    // above executes. Every command function is a one-line, non-`await`ing
    // delegation, so exercising the delegation target through a real
    // `AppState` proves the wiring without needing a Tauri runtime.
    // `subscribe_output`/`unsubscribe_output` (need a live `Channel`) and
    // `set_badge_count` (needs a live `AppHandle`/webview window) are left
    // to manual/E2E verification -- there is no seam for either without a
    // running Tauri app.
    use super::*;
    use crate::notification::hub::{NotificationHub, SystemClock};
    use crate::observer::server::ObserverServerState;
    use crate::observer::ObserverRuntime;
    use crate::persistence::profile_store::ProfileStore;
    use crate::persistence::settings_store::SummaryProvider;
    use crate::session::manager::SessionManager;
    use crate::session::pty_factory::fake::FakePtyFactory;
    use crate::state::fake::RecordingEvents;
    use crate::state::{AppEvents, SessionRegistry};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;
    use uuid::Uuid;

    // summarize_text은 `State<'_, AppState>`를 받으므로 (State는 살아있는
    // App/AppHandle 없이 standalone 구성 불가) 다른 stateful command들과 같은
    // 패턴으로, 본문의 게이트 로직을 real AppState를 통해 그대로 재현해
    // 검증한다.

    #[test]
    fn summarize_text_command_accepts_provider_snapshot() {
        fn assert_signature<F, Fut>(_command: F)
        where
            F: Fn(
                State<'static, AppState>,
                SummaryProvider,
                String,
                String,
                Option<crate::summarizer::SummaryPurpose>,
            ) -> Fut,
        {
        }

        assert_signature(summarize_text);
    }

    #[tokio::test]
    async fn summarize_text_gate_rejects_when_disabled() {
        let (state, ctl, dir, profile_dir) = build("summarize-disabled");
        // AppSettings::default()의 summarizer_enabled == false 전제.
        assert!(!state.settings.read().unwrap().summarizer_enabled);

        // summarize_text 본문과 동일한 게이트: OFF면 CLI 호출 전에 거절.
        let result: Result<String, String> = if !state.settings.read().unwrap().summarizer_enabled {
            Err("summarizer-disabled".to_string())
        } else {
            crate::summarizer::summarize(
                SummaryProvider::Codex,
                crate::summarizer::SummaryPurpose::Label,
                "요약하라",
                "text",
            )
            .await
        };

        assert_eq!(result.unwrap_err(), "summarizer-disabled");
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn summarize_text_proceeds_to_selected_summarizer_when_enabled() {
        let (state, ctl, dir, profile_dir) = build("summarize-enabled");
        *state.settings.write().unwrap() = crate::persistence::settings_store::AppSettings {
            version: 1,
            summarizer_enabled: true,
            summary_provider: SummaryProvider::Codex,
            diary_enabled: false,
            observer_enabled: false,
            sound_enabled: true,
            sound_volume: 0.5,
            external_terminal: Default::default(),
            external_editor: Default::default(),
            attention_hold_ms: 5000,
            git_status_enabled: true,
            cli_enabled: false,
        };

        // ON이면 게이트를 통과해 캡처된 provider로 위임된다 -- 빈 텍스트라서
        // 실 프로세스 spawn 없이 그쪽의 자체 검증 에러로 되돌아오는 것으로 확인.
        let result: Result<String, String> = if !state.settings.read().unwrap().summarizer_enabled {
            Err("summarizer-disabled".to_string())
        } else {
            crate::summarizer::summarize(
                SummaryProvider::Codex,
                crate::summarizer::SummaryPurpose::Label,
                "요약하라",
                "   ",
            )
            .await
        };

        assert_eq!(result.unwrap_err(), "validation: text is empty");
        cleanup(&ctl, &dir, &profile_dir);
    }

    // set_app_settings 본문(저장 -> 캐시 갱신 -> first_run 플래그 내림)을
    // 그대로 재현해 검증한다. 회귀 대상: 첫 실행 완료 후 웹뷰가 리로드돼도
    // get_app_settings가 firstRun: true를 다시 주면 안 된다(Minor #3).
    #[tokio::test]
    async fn set_app_settings_clears_first_run_flag_after_success() {
        let (state, ctl, dir, profile_dir) = build("first-run-flag");
        assert!(
            state
                .settings_first_run
                .load(std::sync::atomic::Ordering::SeqCst),
            "build() 헬퍼는 부팅 시 settings.json 부재 상태를 흉내내므로 초기값은 true"
        );

        let new_settings = crate::persistence::settings_store::AppSettings {
            version: 1,
            summarizer_enabled: true,
            summary_provider: SummaryProvider::Claude,
            diary_enabled: false,
            observer_enabled: false,
            sound_enabled: true,
            sound_volume: 0.5,
            external_terminal: Default::default(),
            external_editor: Default::default(),
            attention_hold_ms: 5000,
            git_status_enabled: true,
            cli_enabled: false,
        };
        // set_app_settings 본문과 동일한 순서: write 가드를 쥔 채 저장 후 캐시
        // 갱신, 가드 해제 -- 그다음 first_run을 false로 내린다.
        {
            let mut guard = state.settings.write().unwrap();
            state
                .settings_store
                .save(&new_settings)
                .expect("save into tempdir should succeed");
            *guard = new_settings;
        }
        state
            .settings_first_run
            .store(false, std::sync::atomic::Ordering::SeqCst);

        assert!(!state
            .settings_first_run
            .load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(*state.settings.read().unwrap(), new_settings);
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn set_app_settings_keeps_enabled_setting_when_observer_server_is_unavailable() {
        let (state, ctl, dir, profile_dir) = build("settings-observer-fail-open");
        state.observer_server.shutdown();
        let settings = crate::persistence::settings_store::AppSettings {
            version: 1,
            summarizer_enabled: false,
            summary_provider: SummaryProvider::Claude,
            diary_enabled: false,
            observer_enabled: true,
            sound_enabled: true,
            sound_volume: 0.5,
            external_terminal: Default::default(),
            external_editor: Default::default(),
            attention_hold_ms: 5000,
            git_status_enabled: true,
            cli_enabled: false,
        };

        assert!(set_app_settings_inner(&state, settings).await.is_ok());
        assert_eq!(*state.settings.read().unwrap(), settings);
        assert_eq!(state.settings_store.load().0, settings);
        assert!(!state
            .settings_first_run
            .load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(state.observer_server.current_url(), None);

        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn create_session_ensures_observer_server_before_preparing_new_pty() {
        let (state, ctl, dir, profile_dir) = build("create-observer-ensure");
        state.settings.write().unwrap().observer_enabled = true;

        let created = create_session_inner(&state, "a1".into(), None)
            .await
            .unwrap();

        assert_eq!(created.state, SessionState::Running);
        assert!(state.observer_server.current_url().is_some());
        assert!(ctl
            .spawned_env()
            .iter()
            .any(|(key, _)| key == "AGENT_OFFICE_HOOK_URL"));
        state.observer_server.shutdown();
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn create_session_still_spawns_when_observer_server_cannot_start() {
        let (state, ctl, dir, profile_dir) = build("create-observer-fail-open");
        state.settings.write().unwrap().observer_enabled = true;
        state.observer_server.shutdown();

        let created = create_session_inner(&state, "a1".into(), None)
            .await
            .unwrap();

        assert_eq!(created.state, SessionState::Running);
        assert!(ctl
            .spawned_env()
            .iter()
            .all(|(key, _)| key != "AGENT_OFFICE_HOOK_URL"));
        cleanup(&ctl, &dir, &profile_dir);
    }

    /// Unique tempdir per test, matching the convention used throughout the
    /// other modules' tests (no `tempfile` dependency needed).
    fn scratch_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-commands-test-{tag}-{}",
            Uuid::new_v4()
        ))
    }

    /// Builds a real `AppState` wired to fakes, mirroring `lib.rs`'s setup
    /// wiring but with `FakePtyFactory`/`RecordingEvents` standing in for
    /// the PTY/Tauri-event side effects.
    fn build(tag: &str) -> (AppState, Arc<FakePtyFactoryControl>, PathBuf, PathBuf) {
        let events: Arc<RecordingEvents> = Arc::new(RecordingEvents::default());
        let events_dyn: Arc<dyn AppEvents> = events.clone();
        let registry = Arc::new(SessionRegistry::new());
        let hub = Arc::new(NotificationHub::new(
            registry.clone(),
            events_dyn.clone(),
            Arc::new(SystemClock),
            Duration::from_millis(3000),
        ));
        let observer_dir = scratch_dir(&format!("{tag}-observer"));
        let observer = Arc::new(ObserverRuntime::production(
            hub.clone(),
            observer_dir.clone(),
            std::env::current_exe().unwrap(),
        ));
        let observer_server = Arc::new(ObserverServerState::default());
        let settings = Arc::new(std::sync::RwLock::new(
            crate::persistence::settings_store::AppSettings::default(),
        ));
        let get_observer_url =
            crate::make_observer_url_getter(settings.clone(), observer_server.clone());
        let (fac, ctl) = FakePtyFactory::new();
        let manager = Arc::new(SessionManager::new(
            Arc::new(fac),
            observer.clone(),
            registry.clone(),
            events_dyn,
            hub.clone(),
            get_observer_url,
        ));
        let profile_dir = scratch_dir(&format!("{tag}-profiles"));
        let store = ProfileStore::new(profile_dir.join("profiles.json"));
        let portrait_store = crate::persistence::png_store::PngStore::new(
            profile_dir.join("portraits"),
            crate::persistence::png_store::MAX_PORTRAIT_BYTES,
        );
        let sprite_store = crate::persistence::png_store::PngStore::new(
            profile_dir.join("sprites"),
            crate::persistence::png_store::MAX_SPRITE_BYTES,
        );

        let settings_store = crate::persistence::settings_store::SettingsStore::new(
            profile_dir.join("settings.json"),
        );
        let session_time_store = crate::persistence::session_time_store::SessionTimeStore::new(
            profile_dir.join("session-times.jsonl"),
        );
        let diary_store =
            crate::persistence::diary_store::DiaryStore::new(profile_dir.join("diaries"));
        let work_log_store =
            crate::persistence::work_log_store::WorkLogStore::new(profile_dir.join("worklogs"));
        let claude_resume_store =
            Arc::new(crate::persistence::claude_resume_store::ClaudeResumeStore::new(
                profile_dir.join("claude-resume.json"),
            ));

        let control_server = Arc::new(crate::control::ControlServerState::default());
        control_server.set_app_data_dir(profile_dir.clone());
        let control_ctx = Arc::new(crate::control::ControlContext {
            manager: manager.clone(),
            observer: observer.clone(),
            observer_server: observer_server.clone(),
            hub: hub.clone(),
            registry: registry.clone(),
            store: store.clone(),
            settings: settings.clone(),
            settings_store: settings_store.clone(),
            app_data_dir: profile_dir.clone(),
        });

        let bot_runtime = std::sync::Arc::new(crate::bot::BotRuntime::default());
        let bot_ctx = std::sync::Arc::new(crate::bot::runner::BotContext {
            manager: manager.clone(),
            store: store.clone(),
            state_store: crate::bot::state_store::BotStateStore::new(
                profile_dir.join("bot-state.json"),
            ),
            state_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
        });
        let state = AppState {
            manager,
            hub,
            observer,
            observer_server,
            store,
            portrait_store,
            sprite_store,
            session_time_store,
            diary_store,
            work_log_store,
            claude_resume_store,
            settings_store,
            settings,
            settings_first_run: std::sync::atomic::AtomicBool::new(true),
            session_event_root: profile_dir.join("session-events").join("v1"),
            live_usage: crate::usage::LiveUsageState::new(),
            control_server,
            control_ctx,
            bot_runtime,
            bot_ctx,
        };
        (state, ctl, observer_dir, profile_dir)
    }

    type FakePtyFactoryControl = crate::session::pty_factory::fake::FakeControl;

    fn req(agent_id: &str) -> CreateSessionRequest {
        CreateSessionRequest {
            agent_id: agent_id.into(),
            cols: None,
            rows: None,
            cwd: None,
            shell: None,
            startup_command: None,
            personality_prompt: None,
            autostart_claude: None,
        }
    }

    fn cleanup(ctl: &FakePtyFactoryControl, hook_dir: &PathBuf, profile_dir: &PathBuf) {
        ctl.close_output();
        let _ = std::fs::remove_dir_all(hook_dir);
        let _ = std::fs::remove_dir_all(profile_dir);
    }

    // ---- create_session ----

    #[test]
    fn create_session_opts_profile_snapshot_flows_to_manager() {
        let opts = SessionOpts {
            cols: None,
            rows: None,
            cwd: None,
            shell: None,
            startup_command: None,
            personality_prompt: None,
            agent_name: Some("Compiler".into()),
            agent_role: Some("Platform".into()),
        };
        assert_eq!(
            event_profile("a1", &opts),
            crate::session_events::types::AgentEventProfile {
                name: "Compiler".into(),
                role: Some("Platform".into()),
            },
        );
        assert_eq!(
            event_profile("a1", &SessionOpts::default()),
            crate::session_events::types::AgentEventProfile {
                name: "a1".into(),
                role: None,
            },
        );
    }

    #[tokio::test]
    async fn create_session_delegates_to_manager_create_with_opts_and_default_autostart() {
        let (state, ctl, dir, profile_dir) = build("create");

        let result = state
            .manager
            .create(CreateSessionRequest {
                agent_id: "a1".into(),
                cols: Some(100),
                rows: Some(30),
                cwd: None,
                shell: None,
                startup_command: None,
                personality_prompt: None,
                autostart_claude: None, // command body always passes None -> manager defaults to false
            })
            .unwrap();

        assert_eq!(result.state, SessionState::Running);
        // autostart defaulted false (plain shell) -> no stdin injection.
        assert_eq!(ctl.writes_utf8(), "");

        cleanup(&ctl, &dir, &profile_dir);
    }

    // create_session 본문(SessionOpts -> CreateSessionRequest)이 `shell`
    // 필드를 그대로 전달하는지 검증 -- cols/rows/cwd와 동일한 delegation
    // 패턴이지만 opts.shell 값이 유실되지 않는지가 회귀 지점.
    #[test]
    fn create_session_opts_shell_flows_into_create_session_request() {
        let opts = SessionOpts {
            cols: None,
            rows: None,
            cwd: None,
            shell: Some("git-bash".into()),
            startup_command: None,
            personality_prompt: None,
            agent_name: None,
            agent_role: None,
        };
        // create_session 본문과 동일한 매핑.
        let request = CreateSessionRequest {
            agent_id: "a1".into(),
            cols: opts.cols,
            rows: opts.rows,
            cwd: opts.cwd,
            shell: opts.shell.clone(),
            startup_command: opts.startup_command.clone(),
            personality_prompt: opts.personality_prompt.clone(),
            autostart_claude: None,
        };
        assert_eq!(request.shell, Some("git-bash".to_string()));
    }

    // create_session 본문이 opts.startup_command를 유실 없이 전달하는지 검증
    // (shell 회귀 테스트와 동일 패턴 -- sessionOptsFor -> SessionOpts -> 요청).
    #[test]
    fn create_session_opts_startup_command_flows_into_create_session_request() {
        let opts = SessionOpts {
            cols: None,
            rows: None,
            cwd: None,
            shell: None,
            startup_command: Some("source ./init.sh".into()),
            personality_prompt: None,
            agent_name: None,
            agent_role: None,
        };
        let request = CreateSessionRequest {
            agent_id: "a1".into(),
            cols: opts.cols,
            rows: opts.rows,
            cwd: opts.cwd,
            shell: opts.shell.clone(),
            startup_command: opts.startup_command.clone(),
            personality_prompt: opts.personality_prompt.clone(),
            autostart_claude: None,
        };
        assert_eq!(
            request.startup_command,
            Some("source ./init.sh".to_string())
        );
    }

    // ---- list_available_shells ----

    #[tokio::test]
    async fn list_available_shells_returns_ok_without_panicking() {
        let result = list_available_shells().await;
        assert!(result.is_ok());
    }

    // ---- dispose_session ----

    #[tokio::test]
    async fn dispose_session_delegates_to_manager_dispose() {
        let (state, ctl, dir, profile_dir) = build("dispose");
        state.manager.create(req("a1")).unwrap();

        state.manager.dispose("a1");

        assert_eq!(
            ctl.kill_count(),
            1,
            "dispose_session must reach PtyControl::kill via manager.dispose"
        );
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- write_input ----

    #[tokio::test]
    async fn write_input_delegates_to_manager_write_input() {
        let (state, ctl, dir, profile_dir) = build("write");
        state.manager.create(req("a1")).unwrap();
        let claude_launch_len = ctl.writes_utf8().len();

        state.manager.write_input("a1", "echo hi\n");

        assert_eq!(&ctl.writes_utf8()[claude_launch_len..], "echo hi\n");
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- resize_session ----

    #[tokio::test]
    async fn resize_session_delegates_to_manager_resize() {
        let (state, ctl, dir, profile_dir) = build("resize");
        state.manager.create(req("a1")).unwrap();

        state.manager.resize("a1", 120, 40);

        assert_eq!(ctl.resize_calls(), vec![(120, 40)]);
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- list_notifications ----

    #[tokio::test]
    async fn list_notifications_delegates_to_manager_pending_notifications() {
        let (state, ctl, dir, profile_dir) = build("list-notif");
        state.manager.create(req("a1")).unwrap();
        let sid = state.manager.session_id_for("a1").unwrap();
        state.hub.ingest_hook(
            &sid,
            crate::types::NotificationSource::Hook,
            br#"{"message":"hi"}"#,
        );

        let listed = state.manager.pending_notifications("a1");

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].message, "hi");
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn list_notifications_for_unknown_agent_returns_empty() {
        let (state, ctl, dir, profile_dir) = build("list-notif-unknown");
        assert!(state.manager.pending_notifications("ghost").is_empty());
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- clear_notifications ----

    #[tokio::test]
    async fn clear_notifications_resolves_session_id_then_delegates_to_hub_clear() {
        let (state, ctl, dir, profile_dir) = build("clear-notif");
        state.manager.create(req("a1")).unwrap();
        let sid = state.manager.session_id_for("a1").unwrap();
        state.hub.ingest_hook(
            &sid,
            crate::types::NotificationSource::Hook,
            br#"{"message":"hi"}"#,
        );

        // Mirrors `clear_notifications`'s body exactly: session_id_for then hub.clear.
        if let Some(resolved_sid) = state.manager.session_id_for("a1") {
            state.hub.clear(&resolved_sid, None);
        }

        assert!(state.hub.pending(&sid).is_empty());
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn clear_notifications_for_unknown_agent_is_a_harmless_noop() {
        let (state, ctl, dir, profile_dir) = build("clear-notif-unknown");
        // Must not panic when session_id_for resolves to None.
        if let Some(sid) = state.manager.session_id_for("ghost") {
            state.hub.clear(&sid, None);
        }
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- load_state / save_state ----

    #[tokio::test]
    async fn load_state_delegates_to_store_load() {
        let (state, ctl, dir, profile_dir) = build("load-state");
        let loaded = state.store.load();
        assert_eq!(loaded.version, 1);
        assert!(loaded.agents.is_empty());
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn save_state_delegates_to_store_save_and_maps_io_error_to_string() {
        let (state, ctl, dir, profile_dir) = build("save-state");
        let persisted = PersistedState {
            agents: vec![AgentProfile {
                id: "p1".into(),
                name: "Ada".into(),
                role: "backend".into(),
                note: "".into(),
                seed: "seed".into(),
                created_at: 1,
                desk_index: 0,
                assigned_desk_index: None,
                cwd: None,
                appearance: None,
                portrait_updated_at: None,
                sprite_request: None,
                sprite_updated_at: None,
                archetype: None,
                shell: None,
                startup_command: None,
                personality_prompt: None,
                clocked_out: None,
                keyboard_sound: None,
                bot: None,
            }],
            version: 1,
            vacation_mode: None,
        };

        // Mirrors `save_state`'s body: `store.save(&state).map_err(|e| e.to_string())`.
        let result: Result<(), String> = state.store.save(&persisted).map_err(|e| e.to_string());
        assert!(result.is_ok());

        let reloaded = state.store.load();
        assert_eq!(reloaded.agents.len(), 1);
        assert_eq!(reloaded.agents[0].name, "Ada");
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- portrait commands ----

    fn tiny_png_b64() -> String {
        use base64::Engine;
        let mut v = vec![0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend_from_slice(b"body");
        base64::engine::general_purpose::STANDARD.encode(v)
    }

    #[tokio::test]
    async fn save_then_load_then_delete_portrait_through_app_state() {
        let (state, ctl, dir, profile_dir) = build("portrait");
        // 프로필 존재 검증을 위해 profiles.json에 p1 저장.
        let persisted = PersistedState {
            agents: vec![AgentProfile {
                id: "p1".into(),
                name: "Ada".into(),
                role: "backend".into(),
                note: "".into(),
                seed: "seed".into(),
                created_at: 1,
                desk_index: 0,
                assigned_desk_index: None,
                cwd: None,
                appearance: None,
                portrait_updated_at: None,
                sprite_request: None,
                sprite_updated_at: None,
                archetype: None,
                shell: None,
                startup_command: None,
                personality_prompt: None,
                clocked_out: None,
                keyboard_sound: None,
                bot: None,
            }],
            version: 1,
            vacation_mode: None,
        };
        state.store.save(&persisted).unwrap();
        let ids: Vec<String> = state
            .store
            .load()
            .agents
            .iter()
            .map(|a| a.id.clone())
            .collect();
        let encoded = tiny_png_b64();

        // save_portrait 본문과 동일한 delegation.
        state.portrait_store.save("p1", &encoded, &ids).unwrap();
        // load_portrait 본문.
        let loaded = state.portrait_store.load("p1").unwrap();
        assert_eq!(loaded, Some(encoded));
        // delete_portrait 본문.
        state.portrait_store.delete("p1").unwrap();
        assert_eq!(state.portrait_store.load("p1").unwrap(), None);

        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn save_portrait_maps_unknown_agent_to_err() {
        let (state, ctl, dir, profile_dir) = build("portrait-unknown");
        let ids: Vec<String> = state
            .store
            .load()
            .agents
            .iter()
            .map(|a| a.id.clone())
            .collect();
        let result: Result<(), String> = state
            .portrait_store
            .save("ghost", &tiny_png_b64(), &ids)
            .map_err(|e| e.to_string());
        assert!(result.is_err());
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn save_then_load_then_delete_sprite_through_app_state() {
        let (state, ctl, dir, profile_dir) = build("sprite");
        let persisted = PersistedState {
            agents: vec![AgentProfile {
                id: "p1".into(),
                name: "Ada".into(),
                role: "backend".into(),
                note: "".into(),
                seed: "seed".into(),
                created_at: 1,
                desk_index: 0,
                assigned_desk_index: None,
                cwd: None,
                appearance: None,
                portrait_updated_at: None,
                sprite_request: None,
                sprite_updated_at: None,
                archetype: None,
                shell: None,
                startup_command: None,
                personality_prompt: None,
                clocked_out: None,
                keyboard_sound: None,
                bot: None,
            }],
            version: 1,
            vacation_mode: None,
        };
        state.store.save(&persisted).unwrap();
        let ids: Vec<String> = state
            .store
            .load()
            .agents
            .iter()
            .map(|a| a.id.clone())
            .collect();
        let encoded = tiny_png_b64();

        // save_sprite / load_sprite / delete_sprite 본문과 동일한 delegation.
        state.sprite_store.save("p1", &encoded, &ids).unwrap();
        assert_eq!(state.sprite_store.load("p1").unwrap(), Some(encoded));
        // portraits와 sprites는 별도 디렉터리다.
        assert_eq!(state.portrait_store.load("p1").unwrap(), None);
        state.sprite_store.delete("p1").unwrap();
        assert_eq!(state.sprite_store.load("p1").unwrap(), None);

        cleanup(&ctl, &dir, &profile_dir);
    }

    // generate_sprite_image: 네트워크 이전의 검증 로직만 테스트한다.
    // (실 API 호출 테스트 금지 — 빈 description은 HTTP 전에 걸러져야 한다.)
    #[tokio::test]
    async fn generate_sprite_image_rejects_empty_description() {
        let err = generate_sprite_image("   ".to_string()).await.unwrap_err();
        assert_eq!(err, "validation: description is empty");
    }

    // ---- append_session_turn / load_session_turns ----

    #[tokio::test]
    async fn append_then_load_session_turn_through_app_state() {
        let (state, ctl, dir, profile_dir) = build("session-turn");
        let record = SessionTurnRecord {
            agent_id: "a1".into(),
            started_at: 1_000,
            ended_at: 4_000,
            total_ms: 3_000,
            worked_ms: 2_000,
            waited_ms: 1_000,
        };

        // append_session_turn / load_session_turns 본문과 동일한 delegation.
        state
            .session_time_store
            .append(&record)
            .map_err(|e: std::io::Error| e.to_string())
            .unwrap();
        let loaded = state.session_time_store.load();

        assert_eq!(loaded, vec![record]);
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn load_session_turns_on_no_prior_appends_returns_empty() {
        let (state, ctl, dir, profile_dir) = build("session-turn-empty");
        assert!(state.session_time_store.load().is_empty());
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- append_diary_entry / load_diary ----

    #[tokio::test]
    async fn append_then_load_diary_through_app_state() {
        let (state, ctl, dir, profile_dir) = build("diary");
        let entry = crate::types::DiaryEntry {
            at: 1_000,
            session_id: "s1".into(),
            body: "오늘은 이슈 하나를 해치웠다. 뿌듯.".into(),
        };

        // append_diary_entry / load_diary 본문과 동일한 delegation.
        state.diary_store.append("a1", &entry).unwrap();
        let loaded = state.diary_store.load("a1").unwrap();

        assert_eq!(loaded, vec![entry]);
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn load_diary_on_no_prior_appends_returns_empty() {
        let (state, ctl, dir, profile_dir) = build("diary-empty");
        assert!(state.diary_store.load("nobody").unwrap().is_empty());
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- load_session_events ----

    #[tokio::test]
    async fn load_session_events_reads_the_configured_root_in_range() {
        use crate::session_events::types::{SessionEventKind, SessionEventRecord};
        let (state, ctl, dir, profile_dir) = build("session-events");

        // 커맨드 본문과 동일한 root를 써서 v1 디렉터리에 하루치 파일을 만든다.
        std::fs::create_dir_all(&state.session_event_root).unwrap();
        let record = SessionEventRecord {
            schema_version: 1,
            run_id: "r".into(),
            seq: 1,
            at: 1_783_728_000_000, // 2026-07-11 00:00 UTC
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: SessionEventKind::Tool,
            agent_name: None,
            agent_role: None,
            cwd: None,
            shell: None,
            state: None,
        };
        let mut line = serde_json::to_string(&record).unwrap();
        line.push('\n');
        std::fs::write(
            state.session_event_root.join("2026-07-11.jsonl"),
            line,
        )
        .unwrap();

        // load_session_events 본문과 동일한 delegation.
        let loaded = crate::session_events::reader::load_session_events(
            &state.session_event_root,
            1_783_728_000_000,
            1_783_728_000_001,
        );

        assert_eq!(loaded, vec![record]);
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn load_session_events_on_missing_root_returns_empty() {
        let (state, ctl, dir, profile_dir) = build("session-events-empty");
        let loaded = crate::session_events::reader::load_session_events(
            &state.session_event_root,
            0,
            u64::MAX / 2,
        );
        assert!(loaded.is_empty());
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- resolve_usage_roots ----
    //
    // 전역 `std::env::var` 없이 순수 계산만 검증(load_usage_snapshot 본문과
    // 동일 로직). docs/usage-limits-design.md §2 CODEX_HOME/CLAUDE_CONFIG_DIR
    // 오버라이드 계약.

    #[test]
    fn resolve_usage_roots_defaults_to_home_when_no_env_set() {
        let home = PathBuf::from("/home/u");
        let (codex_root, claude_root) = resolve_usage_roots(&home, None, None);
        assert_eq!(codex_root, PathBuf::from("/home/u/.codex"));
        assert_eq!(claude_root, PathBuf::from("/home/u"));
    }

    #[test]
    fn resolve_usage_roots_prefers_codex_home_env_when_set() {
        let home = PathBuf::from("/home/u");
        let (codex_root, claude_root) =
            resolve_usage_roots(&home, Some("/custom/codex"), None);
        assert_eq!(codex_root, PathBuf::from("/custom/codex"));
        assert_eq!(claude_root, PathBuf::from("/home/u"));
    }

    #[test]
    fn resolve_usage_roots_prefers_claude_config_dir_env_when_set() {
        let home = PathBuf::from("/home/u");
        let (codex_root, claude_root) =
            resolve_usage_roots(&home, None, Some("/custom/claude"));
        assert_eq!(codex_root, PathBuf::from("/home/u/.codex"));
        assert_eq!(claude_root, PathBuf::from("/custom/claude"));
    }

    #[test]
    fn resolve_usage_roots_honors_both_env_vars_independently() {
        let home = PathBuf::from("/home/u");
        let (codex_root, claude_root) =
            resolve_usage_roots(&home, Some("/custom/codex"), Some("/custom/claude"));
        assert_eq!(codex_root, PathBuf::from("/custom/codex"));
        assert_eq!(claude_root, PathBuf::from("/custom/claude"));
    }

    #[test]
    fn resolve_usage_roots_treats_empty_string_env_as_unset() {
        let home = PathBuf::from("/home/u");
        let (codex_root, claude_root) = resolve_usage_roots(&home, Some(""), Some(""));
        assert_eq!(codex_root, PathBuf::from("/home/u/.codex"));
        assert_eq!(claude_root, PathBuf::from("/home/u"));
    }

    // ---- resolve_claude_config_dir ----
    //
    // 자격증명 디렉터리는 claude_root와 달리 미설정 시 ~/.claude로 내려간다
    // (docs/claude-usage-live-fetch-design.md §2.2).

    #[test]
    fn resolve_claude_config_dir_defaults_to_dot_claude_under_home() {
        let home = PathBuf::from("/home/u");
        assert_eq!(
            resolve_claude_config_dir(&home, None),
            PathBuf::from("/home/u/.claude")
        );
    }

    #[test]
    fn resolve_claude_config_dir_prefers_env_when_set() {
        let home = PathBuf::from("/home/u");
        assert_eq!(
            resolve_claude_config_dir(&home, Some("/custom/claude")),
            PathBuf::from("/custom/claude")
        );
    }

    #[test]
    fn resolve_claude_config_dir_treats_empty_env_as_unset() {
        let home = PathBuf::from("/home/u");
        assert_eq!(
            resolve_claude_config_dir(&home, Some("")),
            PathBuf::from("/home/u/.claude")
        );
    }

    // ---- load_usage_snapshot_body (스로틀 상태 위임) ----
    //
    // 커맨드 본체가 AppState 없이 호출 가능하고 파일 캐시 미러 폴백으로 항상
    // 성공하는지만 확인한다. 개발 머신엔 실 자격증명이 있을 수 있으므로 live
    // 경로(Keychain 자식 프로세스·실 API 호출)는 스로틀 선점으로 결정적으로
    // 차단한다 — 이 테스트는 네트워크·Keychain에 절대 닿지 않아야 한다.
    #[tokio::test]
    async fn load_usage_snapshot_body_delegates_and_never_errors() {
        let live = crate::usage::LiveUsageState::new();
        let now = 1_784_281_391_475;
        // 스로틀 선점: 직전에 시도한 것으로 기록해 5분 하한에 걸리게 한다.
        assert!(live.begin_attempt_if_due(now - 1));
        // 두 번 호출해도(폴링 흉내) 패닉/에러 없이 스냅샷을 돌려준다.
        let _snap = load_usage_snapshot_body(&live, now).await;
        let _snap2 = load_usage_snapshot_body(&live, now + 60_000).await;
    }
