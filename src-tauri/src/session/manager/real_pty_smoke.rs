    use super::*;
    use crate::notification::hub::{NotificationHub, SystemClock};
    use crate::observer::claude::ClaudeAdapter;
    use crate::observer::server::ObserverServerState;
    use crate::observer::ObserverRuntime;
    use crate::session::pty_factory::PortablePtyFactory;
    use crate::state::fake::RecordingEvents;
    #[cfg(windows)]
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
                task_chat: false,
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
    #[ignore = "real PTY; run explicitly"]
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
                task_chat: false,
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
            task_chat: false,
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
            task_chat: false,
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
                personality_prompt: None,
                autostart_claude: Some(false),
                task_chat: false,
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

    /// 셸 작업중 감지(kbm #2f9) 종단 확인: 진짜 로그인 셸 세션에서 긴 명령을
    /// 돌리면 `SessionManager`가 띄운 감시 스레드가 훅 없이도
    /// prompt(턴 open) → sub-count(미니미) → idle(턴 정산)을 방출한다.
    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "real PTY; run explicitly"]
    async fn real_shell_foreground_command_drives_activity_without_hooks() {
        use crate::types::ActivityKind;

        let events = Arc::new(RecordingEvents::default());
        let registry = Arc::new(SessionRegistry::new());
        let hub = Arc::new(NotificationHub::new(
            registry.clone(),
            events.clone() as Arc<dyn AppEvents>,
            Arc::new(SystemClock),
            Duration::from_millis(3000),
        ));
        // 훅 URL 없음 = observer 완전 OFF. 이 테스트가 보는 신호는 전부
        // 포그라운드 감시에서만 나온 것이다.
        let observer = Arc::new(ObserverRuntime::new(hub.clone(), vec![]));
        let mgr = Arc::new(SessionManager::new(
            Arc::new(PortablePtyFactory),
            observer,
            registry,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| None),
        ));

        mgr.create(CreateSessionRequest {
            agent_id: "shellwatch".into(),
            cols: Some(80),
            rows: Some(24),
            cwd: None,
            shell: None,
            startup_command: None,
            personality_prompt: None,
            autostart_claude: Some(false),
            task_chat: false,
        })
        .expect("create real session");
        // 출력을 계속 빨아들여 셸이 막히지 않게 한다.
        let _sink = attach_string_collector(&mgr, "shellwatch");
        tokio::time::sleep(Duration::from_millis(1_500)).await; // 프롬프트 대기

        // 프롬프트에 머무는 동안에는 아무 신호도 없어야 한다.
        assert!(
            events.activities().is_empty(),
            "프롬프트 대기 중 activity: {:?}",
            events.activities()
        );

        mgr.write_input("shellwatch", "sleep 4\r");
        wait_for_timeout(
            || {
                events
                    .activities()
                    .iter()
                    .any(|a| a.kind == ActivityKind::Prompt)
            },
            Duration::from_secs(6),
            "셸 포그라운드 명령이 prompt activity를 내지 않았다",
        )
        .await;

        let prompt = events
            .activities()
            .into_iter()
            .find(|a| a.kind == ActivityKind::Prompt)
            .expect("prompt activity");
        assert_eq!(prompt.agent_id, "shellwatch");
        assert_eq!(prompt.text.as_deref(), Some("sleep 4"));

        // 미니미 카운트는 prompt 직후 별도로 방출된다(pgrep 조회 한 박자 뒤).
        wait_for_timeout(
            || {
                events
                    .activities()
                    .iter()
                    .any(|a| a.kind == ActivityKind::SubCount && a.count.unwrap_or(0) >= 1)
            },
            Duration::from_secs(4),
            "작업중인데 미니미 카운트가 오지 않았다",
        )
        .await;

        // 명령이 끝나고 최소 표시시간이 지나면 정산 신호가 온다.
        wait_for_timeout(
            || {
                events
                    .activities()
                    .iter()
                    .any(|a| a.kind == ActivityKind::Idle)
            },
            Duration::from_secs(12),
            "명령 종료 후 idle activity가 오지 않았다",
        )
        .await;
        // 정산과 함께 미니미도 0으로 거둔다.
        assert!(events
            .activities()
            .iter()
            .any(|a| a.kind == ActivityKind::SubCount && a.count == Some(0)));
        // 셸 명령은 완료 알림을 만들지 않는다(알림 목록 오염 금지). 셸이 스스로
        // 울린 BEL은 기존 폴백 경로라 여기서 따지지 않는다.
        assert!(
            !events
                .notifications()
                .iter()
                .any(|n| n.source == NotificationSource::Stop),
            "{:?}",
            events.notifications()
        );

        mgr.dispose("shellwatch");
    }

    /// 태스크 대화 세션 종단 확인(docs/task-chat-design.md §관찰을 끈다): 관찰이 켜진
    /// 매니저에서도 대화 세션의 진짜 로그인 셸에는 `AGENT_OFFICE_AGENT_ID`·
    /// `AGENT_OFFICE_HOOK_URL`이 없고, `claude` 래퍼는 훅 설정 대신 대화용 설정 파일을
    /// 붙이며, 포그라운드 명령이 셸 작업중 신호를 내지 않는다.
    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "real PTY; run explicitly"]
    async fn real_shell_task_chat_session_runs_unobserved() {
        // 세션 env는 부모 프로세스 env 위에 덧쓰기만 한다. 이 테스트를 Agent Office 터미널
        // 안에서 돌리면 부모 세션의 값이 그대로 상속되므로 먼저 지운다(앱을 Finder·Dock으로
        // 띄우면 이 값들이 없다).
        for key in ["AGENT_OFFICE_AGENT_ID", "AGENT_OFFICE_HOOK_URL"] {
            std::env::remove_var(key);
        }
        let events = Arc::new(RecordingEvents::default());
        let registry = Arc::new(SessionRegistry::new());
        let hub = Arc::new(NotificationHub::new(
            registry.clone(),
            events.clone() as Arc<dyn AppEvents>,
            Arc::new(SystemClock),
            Duration::from_millis(3000),
        ));
        let root = scratch_dir("task-chat");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("claude");
        std::fs::write(
            &fake,
            "#!/bin/sh\nprintf 'FAKE-ARGS[%s]\\n' \"$*\"\nwhile [ $# -gt 0 ]; do\n  if [ \"$1\" = --settings ]; then printf 'FAKE-SETTINGS[%s]\\n' \"$(cat \"$2\")\"; fi\n  shift\ndone\n",
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let settings_dir = root.join("settings");
        std::fs::create_dir_all(&settings_dir).unwrap();
        let observer = Arc::new(ObserverRuntime::new(
            hub.clone(),
            vec![Arc::new(ClaudeAdapter::new(
                root.join("observer"),
                std::env::current_exe().unwrap(),
            ))],
        ));
        let mgr = Arc::new(
            SessionManager::new(
                Arc::new(PortablePtyFactory),
                observer,
                registry,
                events.clone() as Arc<dyn AppEvents>,
                hub,
                Arc::new(|| Some("http://127.0.0.1:45999/hook".into())),
            )
            .with_task_chat_settings(Arc::new(move |sid| {
                let path = settings_dir.join(format!("{sid}.settings.json"));
                std::fs::write(&path, r#"{"permissions":{"allow":["Read(//chat-marker/**)"]}}"#).ok()?;
                Some(path)
            })),
        );

        let agent_id = "task-chat-smokeTask1";
        mgr.create(CreateSessionRequest {
            agent_id: agent_id.into(),
            cols: Some(200),
            rows: Some(24),
            cwd: Some(root.to_string_lossy().into_owned()),
            shell: None,
            startup_command: None,
            personality_prompt: Some("persona-must-not-load".into()),
            autostart_claude: Some(false),
            task_chat: true,
        })
        .expect("create real task chat session");
        let output = attach_string_collector(&mgr, agent_id);
        tokio::time::sleep(Duration::from_millis(1_500)).await; // 프롬프트 대기

        mgr.write_input(
            agent_id,
            "echo \"ENV-AID[${AGENT_OFFICE_AGENT_ID-unset}] ENV-HOOK[${AGENT_OFFICE_HOOK_URL-unset}] ENV-SES[${AGENT_OFFICE_SESSION:+set}]\"\r",
        );
        mgr.write_input(agent_id, &format!("export PATH='{}':\"$PATH\"; claude hello\r", bin.display()));
        wait_for_timeout(
            || output.lock().contains("FAKE-SETTINGS[{"),
            Duration::from_secs(8),
            "claude 래퍼가 대화용 설정 파일을 붙이지 않았다",
        )
        .await;
        let text = output.lock().clone();
        assert!(text.contains("ENV-AID[unset] ENV-HOOK[unset] ENV-SES[set]"), "{text}");
        assert!(text.contains("FAKE-SETTINGS[{\"permissions\":{\"allow\":[\"Read(//chat-marker/**)\"]}}]"), "{text}");
        assert!(!text.contains("persona-must-not-load"), "{text}");

        // 셸 작업중 감시가 꺼져 있으므로 긴 명령도 activity를 만들지 않는다.
        mgr.write_input(agent_id, "sleep 3; echo SLEPT-$((40+2))\r");
        wait_for_timeout(
            || output.lock().contains("SLEPT-42"),
            Duration::from_secs(8),
            "sleep 명령이 끝나지 않았다",
        )
        .await;
        tokio::time::sleep(Duration::from_millis(1_000)).await;
        assert!(events.activities().is_empty(), "대화 세션 activity: {:?}", events.activities());
        assert_eq!(mgr.live_task_chat_count(), 1);

        mgr.dispose(agent_id);
        assert_eq!(mgr.live_task_chat_count(), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    /// 출력을 문자열로 모으는 최소 수집기(위 스모크들의 `attach_collector`와
    /// 같은 배선 — 이 테스트만 쓰는 사본이라 이름을 분리해 둔다).
    fn attach_string_collector(
        mgr: &Arc<SessionManager>,
        agent_id: &str,
    ) -> Arc<Mutex<String>> {
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
