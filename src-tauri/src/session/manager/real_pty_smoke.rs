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

        let handed = mgr1.handoff_all(&snapshots, &HashMap::new());
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
        let handed = mgr1.handoff_all(&std::collections::HashMap::new(), &std::collections::HashMap::new());
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
        let handed = mgr1.handoff_all(&std::collections::HashMap::new(), &std::collections::HashMap::new());
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
        let handed = mgr1.handoff_all(&std::collections::HashMap::new(), &std::collections::HashMap::new());
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
        assert_eq!(mgr1.handoff_all(&std::collections::HashMap::new(), &std::collections::HashMap::new()), 1);
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
                personality_prompt: None,
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
