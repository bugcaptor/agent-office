//! Explicit real-PTY smoke coverage. The deterministic CLI double tests the
//! entire definition runner without consuming an authenticated LLM session.
//! The opt-in live test uses an installed, already configured CLI.

#[cfg(unix)]
mod unix {
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use crate::automation::cli_transition::{CliTransition, ReceiptRead, StartedRead};
    use crate::automation::{store::AutomationStore, AutomationContext, AutomationRuntime};
    use crate::session::inject::{InjectGate, InputSink};
    use crate::state::BotPromptArms;
    use crate::types::{AutomationDefinition, AutomationPhase, AutomationRepeat, AutomationStep};
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};

    struct PtySink {
        writer: Mutex<Box<dyn Write + Send>>,
        output: Arc<Mutex<String>>,
        cwd: String,
    }

    impl InputSink for PtySink {
        fn write(&self, _: &str, data: &str) {
            let mut writer = self.writer.lock().unwrap();
            writer.write_all(data.as_bytes()).unwrap();
            writer.flush().unwrap();
        }
        fn session_id_for(&self, _: &str) -> Option<String> {
            Some("pty-session".into())
        }
        fn is_running(&self, _: &str) -> bool {
            true
        }
        fn cwd_of(&self, _: &str) -> Option<String> {
            Some(self.cwd.clone())
        }
        fn shell_path_for(&self, _: &str) -> Option<String> {
            Some("/bin/bash".into())
        }
    }

    struct Shell {
        sink: Arc<PtySink>,
        child: Box<dyn portable_pty::Child + Send + Sync>,
        _master: Box<dyn portable_pty::MasterPty + Send>,
    }
    impl Drop for Shell {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn shell(workspace: &Path, bin: Option<&Path>) -> Shell {
        shell_with_program(workspace, bin, "/bin/bash")
    }

    fn shell_with_program(workspace: &Path, bin: Option<&Path>, program: &str) -> Shell {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 32,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(program);
        if program.ends_with("bash") {
            command.args(["--noprofile", "--norc"]);
        } else {
            command.arg("-f");
        }
        command.cwd(workspace);
        command.env("TERM", "xterm-256color");
        command.env("PS1", "AO_SMOKE_READY> ");
        if let Some(bin) = bin {
            command.env(
                "PATH",
                format!(
                    "{}:{}",
                    bin.display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            );
        }
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let output = Arc::new(Mutex::new(String::new()));
        let reader_output = output.clone();
        std::thread::spawn(move || {
            let mut bytes = [0u8; 4096];
            while let Ok(n) = reader.read(&mut bytes) {
                if n == 0 {
                    break;
                }
                reader_output
                    .lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&bytes[..n]));
            }
        });
        let sink = Arc::new(PtySink {
            writer: Mutex::new(pair.master.take_writer().unwrap()),
            output,
            cwd: workspace.display().to_string(),
        });
        Shell {
            sink,
            child,
            _master: pair.master,
        }
    }

    async fn wait_until(
        mut predicate: impl FnMut() -> bool,
        limit: Duration,
        description: &str,
        sink: &PtySink,
    ) {
        let deadline = tokio::time::Instant::now() + limit;
        while !predicate() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "{}\nPTY output:\n{}",
                description,
                sink.output.lock().unwrap()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn context(sink: Arc<PtySink>) -> Arc<AutomationContext> {
        let gate = Arc::new(InjectGate::new(
            sink.clone(),
            Arc::new(BotPromptArms::new()),
        ));
        Arc::new(AutomationContext {
            sink,
            gate,
            bot_runtime: Arc::new(crate::bot::BotRuntime::default()),
        })
    }

    fn definition(cli: &str, startup_ms: u64, cycles: u32) -> AutomationDefinition {
        AutomationDefinition {
            schema_version: 1, id: "pty-smoke".into(), revision: 1, name: "PTY smoke".into(), inputs: vec![],
            steps: vec![
                AutomationStep::LaunchCli { id: "launch".into(), cli_profile_id: cli.into(), model: None, effort: None, startup_wait_ms: Some(startup_ms) },
                AutomationStep::LlmTask { id: "task".into(), label: "smoke".into(), prompt_template: "This is a smoke test. Do no project work. Output a short greeting and write the required result JSON as the final action. Preserve the provided version, runId and stepExecutionId. Use status done.".into(), wait_timeout_ms: Some(120_000), completion_grace_ms: Some(30) , allow_early_complete: Some(false)},
                AutomationStep::ExitCli { id: "exit".into(), command: if cli == "pi" { "/quit" } else { "/exit" }.into(), return_mode: None, exit_wait_ms: None },
            ], repeat: Some(AutomationRepeat { max_cycles: cycles }), workspace: None, input_values: None,
        }
    }

    #[tokio::test]
    async fn canonical_pty_queues_next_short_source_during_return_prompt_delay() {
        for program in ["/bin/bash", "/bin/zsh"] {
            if !Path::new(program).is_file() { continue; }
            let root = tempfile::tempdir().unwrap();
            let shell = shell_with_program(root.path(), None, program);
            wait_until(|| shell.sink.output.lock().unwrap().contains("AO_SMOKE_READY"), Duration::from_secs(5), "shell startup", &shell.sink).await;
            let marker = root.path().join("prompt-hook-entered");
            let release = root.path().join("prompt-hook-release");
            let quote = |path: &Path| format!("'{}'", path.display().to_string().replace('\'', "'\\''"));
            let marker_q = quote(&marker);
            let release_q = quote(&release);
            let hook = if program.ends_with("bash") {
                format!("__ao_prompt_count=0; ao_first() {{ :; }}; ao_second() {{ :; }}; PROMPT_COMMAND='__ao_prompt_count=$((__ao_prompt_count+1)); if [ \"$__ao_prompt_count\" -gt 1 ]; then stty icanon; : > {marker_q}; __ao_i=0; while [ ! -f {release_q} ] && [ \"$__ao_i\" -lt 200 ]; do sleep 0.01; __ao_i=$((__ao_i+1)); done; fi'")
            } else {
                format!("__ao_prompt_count=0; ao_first() {{ :; }}; ao_second() {{ :; }}; precmd() {{ (( __ao_prompt_count++ )); if (( __ao_prompt_count > 1 )); then stty icanon; : > {marker_q}; integer __ao_i=0; while [[ ! -f {release_q} && $__ao_i -lt 200 ]]; do sleep 0.01; (( __ao_i++ )); done; fi }}")
            };
            shell.sink.write("smoke", &format!("{hook}\r"));
            let adapter = CliTransition::new(root.path().join("app-data"));
            let first = adapter.prepare_posix_launch("pty-session", "run-1", "first", "step-1", "ao_first").unwrap();
            let second = adapter.prepare_posix_launch("pty-session", "run-1", "second", "step-2", "ao_second").unwrap();
            assert!(first.command().as_bytes().len() + 1 <= 512);
            assert!(second.command().as_bytes().len() + 1 <= 512);
            shell.sink.write("smoke", &format!("{}\r", first.command()));
            wait_until(|| adapter.poll_receipt(&first) == ReceiptRead::Returned { exit_code: 0 }, Duration::from_secs(3), "first return receipt", &shell.sink).await;
            wait_until(|| marker.exists(), Duration::from_secs(1), "canonical prompt hook after return", &shell.sink).await;
            // The shell is in canonical input mode but is still running its prompt
            // hook. Queue the next transition now; it must execute after the hook.
            shell.sink.write("smoke", &format!("{}\r", second.command()));
            tokio::time::sleep(Duration::from_millis(80)).await;
            assert_eq!(adapter.poll_started(&second), StartedRead::Pending, "queued source ran before prompt hook released");
            std::fs::write(&release, "release").unwrap();
            wait_until(|| adapter.poll_started(&second) == StartedRead::Started, Duration::from_secs(3), "queued source ACK", &shell.sink).await;
            wait_until(|| adapter.poll_receipt(&second) == ReceiptRead::Returned { exit_code: 0 }, Duration::from_secs(3), "queued source return", &shell.sink).await;
        }
    }

    fn v2_definition(cycles: u32) -> AutomationDefinition {
        let segment = |cli: &str, prefix: &str| {
            vec![
                AutomationStep::LaunchCli {
                    id: format!("{prefix}-launch"),
                    cli_profile_id: cli.into(),
                    model: None,
                    effort: None,
                    startup_wait_ms: Some(20),
                },
                AutomationStep::LlmTask {
                    id: format!("{prefix}-task"),
                    label: prefix.into(),
                    prompt_template: format!("{prefix}: {{{{previousResult}}}}"),
                    wait_timeout_ms: Some(5_000),
                    completion_grace_ms: Some(1),
                    allow_early_complete: Some(false),
                },
                AutomationStep::ExitCli {
                    id: format!("{prefix}-exit"),
                    command: "/exit".into(),
                    return_mode: Some(crate::types::AutomationCliReturnMode::Auto),
                    exit_wait_ms: Some(5_000),
                },
            ]
        };
        let mut steps = segment("claude", "first");
        steps.extend(segment("agy", "second"));
        AutomationDefinition {
            schema_version: 2,
            id: "pty-smoke-v2".into(),
            revision: 1,
            name: "PTY v2 smoke".into(),
            inputs: vec![],
            steps,
            repeat: Some(AutomationRepeat { max_cycles: cycles }),
            workspace: None,
            input_values: None,
        }
    }

    #[tokio::test]
    #[ignore = "real PTY; run explicitly"]
    async fn definition_roundtrip_in_real_shell_with_cli_double() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace with spaces");
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        let script = r#"#!/usr/bin/env python3
import json, pathlib, re, sys
log = pathlib.Path('smoke.log')
def record(value):
    with log.open('a') as f: f.write(value + '\n')
record('launch')
for line in sys.stdin:
    if line.strip() == ('/quit' if pathlib.Path(sys.argv[0]).name == 'pi' else '/exit'):
        record('exit')
        break
    match = re.search(r"['\"]([^'\"]*/prompt\.md)['\"]", line)
    if not match: continue
    prompt = pathlib.Path(match.group(1))
    record('task:' + prompt.parent.name)
    (prompt.parent / 'result.json').write_text(json.dumps({'version': 1, 'runId': prompt.parent.parent.name, 'stepExecutionId': prompt.parent.name, 'status': 'done', 'summary': 'PTY complete'}))
"#;
        for cli in ["claude", "codex", "agy", "kilo", "pi"] {
            let path = bin.join(cli);
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
            let shell = shell(&workspace, Some(&bin));
            wait_until(
                || shell.sink.output.lock().unwrap().contains("AO_SMOKE_READY"),
                Duration::from_secs(5),
                "shell startup",
                &shell.sink,
            )
            .await;
            let runtime = AutomationRuntime::default();
            let store = Arc::new(AutomationStore::new(
                root.path().join(format!("data-{cli}")),
            ));
            let status = runtime
                .start_definition(
                    context(shell.sink.clone()),
                    "smoke".into(),
                    definition(cli, 100, 2),
                    BTreeMap::new(),
                    workspace.display().to_string(),
                    store.clone(),
                )
                .unwrap();
            assert!(status.running);
            wait_until(
                || {
                    runtime
                        .status()
                        .agents
                        .get("smoke")
                        .is_some_and(|s| !s.running)
                },
                Duration::from_secs(20),
                "definition run must finish",
                &shell.sink,
            )
            .await;
            assert_eq!(
                runtime.status().agents["smoke"].phase,
                AutomationPhase::Completed
            );
            wait_until(
                || {
                    std::fs::read_to_string(workspace.join("smoke.log"))
                        .unwrap_or_default()
                        .lines()
                        .filter(|line| *line == "exit")
                        .count()
                        >= 1
                },
                Duration::from_secs(2),
                "explicit ExitCli was delivered",
                &shell.sink,
            )
            .await;
            let log = std::fs::read_to_string(workspace.join("smoke.log")).unwrap();
            assert_eq!(log.lines().filter(|line| *line == "launch").count(), 1);
            assert_eq!(log.lines().filter(|line| *line == "exit").count(), 1);
            let tasks: Vec<_> = log
                .lines()
                .filter(|line| line.starts_with("task:"))
                .collect();
            assert_eq!(tasks.len(), 2);
            assert_ne!(
                tasks[0], tasks[1],
                "each cycle must get a unique execution directory"
            );
            assert_eq!(store.runs().unwrap()[0].status, "completed");
            runtime.stop_all();
            drop(shell);
            std::fs::remove_file(workspace.join("smoke.log")).unwrap();
        }
    }

    #[tokio::test]
    #[ignore = "real PTY v2 CLI-transition double; run explicitly"]
    async fn v2_two_cli_two_cycles_roundtrip_in_real_shell_with_cli_double() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("작업 공간 with spaces");
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        let script = r##"#!/usr/bin/env python3
import json, pathlib, re, sys, time
log = pathlib.Path('v2-smoke.log')
def record(value):
    with log.open('a') as f: f.write(value + '\n')
record('launch:' + pathlib.Path(sys.argv[0]).name)
assert sys.argv[1:] == ['--model', "모델 ' with spaces", '--effort', 'high']
for line in sys.stdin:
    if line.strip() == '/exit':
        record('exit:' + pathlib.Path(sys.argv[0]).name)
        time.sleep(0.25)
        record('returned:' + pathlib.Path(sys.argv[0]).name)
        break
    match = re.search(r"['\"]([^'\"]*/prompt\.md)['\"]", line)
    if not match: continue
    prompt = pathlib.Path(match.group(1))
    text = prompt.read_text()
    record('task:' + pathlib.Path(sys.argv[0]).name + ':' + prompt.parent.name + ':' + text.replace('\n', '|'))
    handoff = pathlib.Path(text.split('\nhandoff=', 1)[1])
    if pathlib.Path(sys.argv[0]).name == 'claude':
        handoff.parent.mkdir(parents=True, exist_ok=True)
        handoff.write_text('design for ' + handoff.parent.name)
    else:
        assert handoff.read_text() == 'design for ' + handoff.parent.name
        handoff.with_name('implementation.txt').write_text(handoff.read_text())
    (prompt.parent / 'result.json').write_text(json.dumps({'version': 1, 'runId': prompt.parent.parent.name, 'stepExecutionId': prompt.parent.name, 'status': 'done', 'summary': '완료 ' + pathlib.Path(sys.argv[0]).name}))
"##;
        for cli in ["claude", "agy"] {
            let path = bin.join(cli);
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let shell = shell(&workspace, Some(&bin));
        wait_until(
            || shell.sink.output.lock().unwrap().contains("AO_SMOKE_READY"),
            Duration::from_secs(5),
            "shell startup",
            &shell.sink,
        )
        .await;
        let runtime = AutomationRuntime::default();
        let store = Arc::new(AutomationStore::new(root.path().join("data")));
        let mut flow = v2_definition(2);
        for step in &mut flow.steps {
            match step {
                AutomationStep::LaunchCli { model, effort, .. } => {
                    *model = Some("모델 ' with spaces".into());
                    *effort = Some("high".into());
                }
                AutomationStep::LlmTask {
                    prompt_template, ..
                } => {
                    prompt_template.push_str("\nhandoff={{workspace}}/.agent-office/automation-runs/{{run}}/handoff/cycle-{{cycle}}/design.md");
                }
                _ => {}
            }
        }
        runtime
            .start_definition(
                context(shell.sink.clone()),
                "smoke".into(),
                flow,
                BTreeMap::new(),
                workspace.display().to_string(),
                store.clone(),
            )
            .unwrap();
        wait_until(
            || {
                runtime.status().agents.get("smoke").is_some_and(|status| {
                    status.decision_reason
                        == Some(crate::types::AutomationDecisionReason::ShellReady)
                })
            },
            Duration::from_secs(5),
            "initial shell-ready confirmation",
            &shell.sink,
        )
        .await;
        let shell_ready = runtime.status().agents["smoke"].clone();
        assert!(runtime
            .decide(
                "smoke",
                shell_ready.run_id.as_deref().unwrap(),
                shell_ready.step_execution_id.as_deref().unwrap(),
                shell_ready.decision_id.as_deref().unwrap(),
                crate::types::AutomationDecisionChoice::Continue,
            )
            .unwrap());
        wait_until(
            || {
                runtime
                    .status()
                    .agents
                    .get("smoke")
                    .is_some_and(|s| !s.running)
            },
            Duration::from_secs(30),
            "v2 definition run must finish",
            &shell.sink,
        )
        .await;
        assert_eq!(
            runtime.status().agents["smoke"].phase,
            AutomationPhase::Completed
        );
        let log = std::fs::read_to_string(workspace.join("v2-smoke.log")).unwrap();
        for cli in ["claude", "agy"] {
            assert_eq!(
                log.lines()
                    .filter(|line| *line == format!("launch:{cli}"))
                    .count(),
                2
            );
            assert_eq!(
                log.lines()
                    .filter(|line| *line == format!("exit:{cli}"))
                    .count(),
                2
            );
        }
        let tasks: Vec<_> = log
            .lines()
            .filter(|line| line.starts_with("task:"))
            .collect();
        assert_eq!(tasks.len(), 4);
        assert!(
            tasks
                .iter()
                .any(|line| line.contains("second:") && line.contains("완료 claude")),
            "previousResult was not handed to the next CLI: {tasks:?}"
        );
        let records = store.runs().unwrap();
        let record = &records[0];
        assert_eq!(record.status, "completed");
        let lifecycle: Vec<_> = log
            .lines()
            .filter(|line| !line.starts_with("task:"))
            .collect();
        assert_eq!(
            lifecycle,
            [
                "launch:claude",
                "exit:claude",
                "returned:claude",
                "launch:agy",
                "exit:agy",
                "returned:agy",
                "launch:claude",
                "exit:claude",
                "returned:claude",
                "launch:agy",
                "exit:agy",
                "returned:agy"
            ]
        );
        let returned: Vec<_> = record
            .events
            .iter()
            .filter(|e| e.kind == "cli-returned")
            .collect();
        let launched: Vec<_> = record
            .events
            .iter()
            .filter(|e| e.kind == "cli-launch")
            .collect();
        let exits: Vec<_> = record
            .events
            .iter()
            .filter(|e| e.kind == "cli-exit-submitted")
            .collect();
        assert_eq!(returned.len(), 4);
        let mut ids = std::collections::HashSet::new();
        for (i, event) in returned.iter().enumerate() {
            assert!(
                event.at >= exits[i].at + 200,
                "return must wait for delayed CLI exit"
            );
            if i < 3 {
                assert!(launched[i + 1].at >= event.at);
            }
            let details: serde_json::Value =
                serde_json::from_str(event.details.as_ref().unwrap()).unwrap();
            assert_eq!(details["exitCode"], 0);
            ids.insert(details["launchId"].as_str().unwrap().to_string());
        }
        assert_eq!(ids.len(), 4);
        for cycle in [1, 2] {
            let handoff = workspace
                .join(".agent-office/automation-runs")
                .join(&record.run_id)
                .join("handoff")
                .join(format!("cycle-{cycle}"));
            assert_eq!(
                std::fs::read_to_string(handoff.join("implementation.txt")).unwrap(),
                format!("design for cycle-{cycle}")
            );
        }
        runtime.stop_all();
    }

    #[tokio::test]
    #[ignore = "authenticated Claude and agy; explicit live run only, no permission changes"]
    async fn v2_handoff_with_installed_claude_and_agy() {
        let root = tempfile::tempdir().unwrap();
        let shell = shell(root.path(), None);
        wait_until(
            || shell.sink.output.lock().unwrap().contains("AO_SMOKE_READY"),
            Duration::from_secs(5),
            "shell startup",
            &shell.sink,
        )
        .await;
        let runtime = AutomationRuntime::default();
        let ctx = context(shell.sink.clone());
        let store = Arc::new(AutomationStore::new(root.path().join("app-data")));
        let mut flow = v2_definition(1);
        for step in &mut flow.steps {
            match step {
                AutomationStep::LaunchCli {
                    startup_wait_ms, ..
                } => *startup_wait_ms = Some(10_000),
                AutomationStep::LlmTask {
                    prompt_template,
                    wait_timeout_ms,
                    ..
                } => {
                    *wait_timeout_ms = Some(60_000);
                    *prompt_template = if prompt_template.contains("first") {
                        "Create only {{workspace}}/design.md containing exactly 'handoff verified'. Do no other project work. Report done after the file exists."
                    } else {
                        "Read {{workspace}}/design.md, then create only {{workspace}}/implementation.txt containing the same text. If design.md is absent, report blocked. Do no other project work. Report done after the file exists."
                    }.into();
                }
                _ => {}
            }
        }
        runtime
            .start_definition(
                ctx,
                "live-multi".into(),
                flow,
                BTreeMap::new(),
                root.path().display().to_string(),
                store,
            )
            .unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(100);
        loop {
            let status = runtime.status().agents["live-multi"].clone();
            if status.decision_reason == Some(crate::types::AutomationDecisionReason::ShellReady)
                && status.decision_id.is_some()
            {
                runtime
                    .decide(
                        "live-multi",
                        status.run_id.as_deref().unwrap(),
                        status.step_execution_id.as_deref().unwrap(),
                        status.decision_id.as_deref().unwrap(),
                        crate::types::AutomationDecisionChoice::Continue,
                    )
                    .unwrap();
            }
            if !status.running {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                runtime.stop_all();
                panic!("Live CLI handoff did not finish; authentication/trust/tool approval may be required. No CLI permissions were changed. State: {:?}. PTY output: {}", status, shell.sink.output.lock().unwrap());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(
            runtime.status().agents["live-multi"].phase,
            AutomationPhase::Completed
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("design.md")).unwrap(),
            std::fs::read_to_string(root.path().join("implementation.txt")).unwrap()
        );
        runtime.stop_all();
    }

    #[tokio::test]
    #[ignore = "authenticated Claude/Codex/agy/Kilo/pi CLI; AGENT_OFFICE_REAL_AUTOMATION_CLI required"]
    async fn definition_roundtrip_with_installed_cli() {
        let cli = std::env::var("AGENT_OFFICE_REAL_AUTOMATION_CLI")
            .expect("choose claude, codex, agy, kilo, or pi explicitly");
        assert!(matches!(cli.as_str(), "claude" | "codex" | "agy" | "kilo" | "pi"));
        let root = tempfile::tempdir().unwrap();
        let shell = shell(root.path(), None);
        wait_until(
            || shell.sink.output.lock().unwrap().contains("AO_SMOKE_READY"),
            Duration::from_secs(5),
            "shell startup",
            &shell.sink,
        )
        .await;
        let runtime = AutomationRuntime::default();
        let store = Arc::new(AutomationStore::new(root.path().join("app-data")));
        let mut definition = definition(&cli, 10_000, 1);
        // The smoke owns this disposable PTY. CLI exit is tested separately with
        // the double; a live provider may still be finishing its TUI response.
        definition.steps.pop();
        let ctx = context(shell.sink.clone());
        runtime
            .start_definition(
                ctx.clone(),
                "live-smoke".into(),
                definition,
                BTreeMap::new(),
                root.path().display().to_string(),
                store.clone(),
            )
            .unwrap();
        if cli == "claude" || cli == "codex" {
            // This test created and owns the empty temporary workspace. Exercise
            // the normal human-input path to accept its first-use trust prompt;
            // do not change CLI permission settings or bypass tool approvals.
            wait_until(
                || {
                    let output = shell.sink.output.lock().unwrap();
                    output.contains("trust")
                        && (output.contains("folder") || output.contains("directory"))
                },
                Duration::from_secs(8),
                "CLI trust prompt for disposable workspace",
                &shell.sink,
            )
            .await;
            tokio::time::sleep(Duration::from_millis(300)).await;
            if cli == "claude" {
                ctx.gate.note_human("live-smoke", "\x1b[B");
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            ctx.gate.note_human("live-smoke", "\r");
        }
        wait_until(
            || {
                runtime
                    .status()
                    .agents
                    .get("live-smoke")
                    .is_some_and(|s| !s.running)
            },
            Duration::from_secs(150),
            "live CLI must finish the result protocol (trust/auth approval may be required)",
            &shell.sink,
        )
        .await;
        assert_eq!(
            runtime.status().agents["live-smoke"].phase,
            AutomationPhase::Completed
        );
        runtime.stop_all();
    }
}
