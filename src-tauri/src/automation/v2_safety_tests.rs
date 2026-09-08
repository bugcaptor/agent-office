//! Deterministic transition decisions with real receipt writers and a recording PTY.
use super::{store::AutomationStore, AutomationContext, AutomationRuntime};
use crate::session::inject::{
    InjectGate, InjectSource, PendingReason, RecordingSink, SubmitOutcome,
};
use crate::state::BotPromptArms;
use crate::types::*;
use std::{collections::BTreeMap, sync::Arc, time::Duration};

struct Harness {
    runtime: AutomationRuntime,
    ctx: Arc<AutomationContext>,
    sink: Arc<RecordingSink>,
    store: Arc<AutomationStore>,
    root: tempfile::TempDir,
}
impl Harness {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a", true);
        sink.set_session("a", "s");
        sink.set_shell_path("a", "/bin/bash");
        let gate = Arc::new(InjectGate::new(
            sink.clone(),
            Arc::new(BotPromptArms::new()),
        ));
        let ctx = Arc::new(AutomationContext {
            sink: sink.clone(),
            gate,
            bot_runtime: Arc::new(crate::bot::BotRuntime::default()),
        });
        let store = Arc::new(AutomationStore::new(root.path().join("앱 데이터")));
        Self {
            runtime: AutomationRuntime::default(),
            ctx,
            sink,
            store,
            root,
        }
    }
    fn start(&self, mut steps: Vec<AutomationStep>, cycles: u32, manual: bool) {
        if manual {
            for step in &mut steps {
                if let AutomationStep::ExitCli { return_mode, .. } = step {
                    *return_mode = Some(AutomationCliReturnMode::Manual);
                }
            }
        }
        self.runtime
            .start_definition(
                self.ctx.clone(),
                "a".into(),
                AutomationDefinition {
                    schema_version: 2,
                    id: "test".into(),
                    revision: 1,
                    name: "test".into(),
                    steps,
                    inputs: vec![],
                    repeat: Some(AutomationRepeat { max_cycles: cycles }),
                    workspace: None,
                    input_values: None,
                },
                BTreeMap::new(),
                self.root.path().display().to_string(),
                self.store.clone(),
            )
            .unwrap();
    }
    fn status(&self) -> AutomationAgentStatus {
        self.runtime.status().agents["a"].clone()
    }
    async fn until(&self, f: impl Fn(&AutomationAgentStatus) -> bool) -> AutomationAgentStatus {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                self.acknowledge_pending_launch();
                let s = self.status();
                if f(&s) {
                    return s;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "unexpected state {:?}; writes {:?}",
                self.status(),
                self.sink.recorded_writes()
            )
        })
    }
    async fn until_raw(&self, f: impl Fn(&AutomationAgentStatus) -> bool) -> AutomationAgentStatus {
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                let s = self.status();
                if f(&s) {
                    return s;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap()
    }
    fn acknowledge_pending_launch(&self) {
        let Some(run_id) = self
            .runtime
            .status()
            .agents
            .get("a")
            .and_then(|s| s.run_id.clone())
        else {
            return;
        };
        let root = self
            .store
            .app_data_dir()
            .join("automation-cli-returns")
            .join(run_id);
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let started = entry.path().join("started.json");
            if started.exists() {
                continue;
            }
            let Ok(script) = std::fs::read_to_string(entry.path().join("launch.sh")) else {
                continue;
            };
            if let Some(ack) = script
                .split("printf '%s\\n' '")
                .nth(1)
                .and_then(|s| s.split("' >").next())
            {
                Self::write_atomic(&started, ack.as_bytes());
            }
        }
    }
    fn write_atomic(path: &std::path::Path, contents: &[u8]) {
        let temporary = path.with_extension("fixture.tmp");
        std::fs::write(&temporary, contents).unwrap();
        std::fs::rename(temporary, path).unwrap();
    }
    fn decide(&self, s: &AutomationAgentStatus, choice: AutomationDecisionChoice) -> bool {
        self.runtime
            .decide(
                "a",
                s.run_id.as_deref().unwrap(),
                s.step_execution_id.as_deref().unwrap(),
                s.decision_id.as_deref().unwrap(),
                choice,
            )
            .unwrap()
    }
    async fn ready(&self) {
        let s = self
            .until(|s| s.decision_reason == Some(AutomationDecisionReason::ShellReady))
            .await;
        assert!(self.sink.recorded_writes().is_empty());
        assert!(self.decide(&s, AutomationDecisionChoice::Continue));
        assert!(!self.decide(&s, AutomationDecisionChoice::Continue));
    }
    fn launches(&self) -> Vec<String> {
        self.sink
            .recorded_writes()
            .into_iter()
            .filter_map(|(_, s, _)| s.starts_with(". '").then_some(s))
            .collect()
    }
    fn exits(&self) -> Vec<String> {
        self.sink
            .recorded_writes()
            .into_iter()
            .filter_map(|(_, s, _)| s.starts_with("/exit-").then_some(s))
            .collect()
    }
    fn receipt(&self, code: i32) {
        self.acknowledge_pending_launch();
        let launch = self.runtime.status().agents["a"].run_id.clone().unwrap();
        let root = self
            .store
            .app_data_dir()
            .join("automation-cli-returns")
            .join(launch);
        let launch_dir = std::fs::read_dir(root)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| !path.join("return.json").exists())
            .unwrap();
        let script = std::fs::read_to_string(launch_dir.join("launch.sh")).unwrap();
        let ack = script
            .split("printf '%s\\n' '")
            .nth(1)
            .unwrap()
            .split("' >")
            .next()
            .unwrap();
        let receipt = script
            .split("printf '%s%s%s\\n' '")
            .nth(1)
            .unwrap()
            .split("' \"$__ao_cli_rc\"")
            .next()
            .unwrap();
        Self::write_atomic(&launch_dir.join("started.json"), ack.as_bytes());
        Self::write_atomic(
            &launch_dir.join("return.json"),
            format!("{receipt}{code}}}").as_bytes(),
        );
    }
    fn result(&self, s: &AutomationAgentStatus, early: bool) {
        std::fs::write(
            &s.file_path,
            serde_json::to_vec(&serde_json::json!({
                "version":1,"runId":s.run_id,"stepExecutionId":s.step_execution_id,
                "status":if early {"complete"} else {"done"},"summary":"handoff {{cycle}}",
            }))
            .unwrap(),
        )
        .unwrap();
    }
    async fn finish(&self) {
        let s = self.until(|s| !s.running).await;
        assert_eq!(s.phase, AutomationPhase::Completed, "{:?}", s.error);
        assert_eq!(s.cli_context, Some(AutomationCliContext::Shell));
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.runtime.stop_all();
    }
}
fn segment(name: &str, cli: &str, task: bool) -> Vec<AutomationStep> {
    let mut steps = vec![AutomationStep::LaunchCli {
        id: format!("launch-{name}"),
        cli_profile_id: cli.into(),
        model: None,
        effort: None,
        startup_wait_ms: Some(30),
    }];
    if task {
        steps.push(AutomationStep::LlmTask {
            id: format!("task-{name}"),
            label: name.into(),
            prompt_template: "read {{previousResult}}".into(),
            wait_timeout_ms: Some(2000),
            completion_grace_ms: Some(10),
            allow_early_complete: Some(true),
        });
    }
    steps.push(AutomationStep::ExitCli {
        id: format!("exit-{name}"),
        command: format!("/exit-{name}"),
        return_mode: None,
        exit_wait_ms: Some(40),
    });
    steps
}

#[tokio::test]
async fn return_timeout_requires_extend_and_never_resends_exit() {
    let h = Harness::new();
    let mut steps = segment("one", "claude", false);
    steps.extend(segment("two", "agy", false));
    h.start(steps, 1, false);
    h.ready().await;
    let timeout = h
        .until(|s| s.decision_reason == Some(AutomationDecisionReason::CliExitTimeout))
        .await;
    assert_eq!(h.launches().len(), 1);
    assert_eq!(h.exits(), vec!["/exit-one"]);
    assert!(!h.decide(&timeout, AutomationDecisionChoice::Continue));
    h.receipt(0);
    let observed = h.until(|s| s.cli_return_observed_at_ms.is_some()).await;
    assert_eq!(observed.decision_id, timeout.decision_id);
    assert_eq!(h.launches().len(), 1);
    assert!(h.decide(&observed, AutomationDecisionChoice::Extend));
    let second = h
        .until(|s| {
            s.decision_reason == Some(AutomationDecisionReason::CliExitTimeout)
                && s.step_id.as_deref() == Some("exit-two")
        })
        .await;
    assert_eq!(h.exits(), vec!["/exit-one", "/exit-two"]);
    h.receipt(0);
    assert!(h.decide(&second, AutomationDecisionChoice::Extend));
    h.finish().await;
    assert_eq!(h.launches().len(), 2);
}

#[tokio::test]
async fn early_complete_exits_only_the_matching_cli_and_waits_for_return() {
    for early_second in [false, true] {
        let h = Harness::new();
        let mut steps = segment("one", "claude", true);
        steps.extend(segment("two", "agy", true));
        h.start(steps, 2, false);
        h.ready().await;
        let first = h
            .until(|s| s.phase == AutomationPhase::Watching && !s.file_path.is_empty())
            .await;
        h.result(&first, !early_second);
        let exit = h
            .until(|s| s.decision_reason == Some(AutomationDecisionReason::CliExitTimeout))
            .await;
        assert!(exit.running);
        assert_eq!(h.exits(), vec!["/exit-one"]);
        h.receipt(0);
        assert!(h.decide(&exit, AutomationDecisionChoice::Extend));
        if early_second {
            let second = h
                .until(|s| {
                    s.phase == AutomationPhase::Watching
                        && !s.file_path.is_empty()
                        && s.step_id.as_deref() == Some("task-two")
                })
                .await;
            let prompt = std::path::Path::new(&second.file_path).with_file_name("prompt.md");
            assert!(std::fs::read_to_string(prompt)
                .unwrap()
                .contains("handoff {{cycle}}"));
            h.result(&second, true);
            let exit = h
                .until(|s| s.decision_reason == Some(AutomationDecisionReason::CliExitTimeout))
                .await;
            assert_eq!(h.exits(), vec!["/exit-one", "/exit-two"]);
            h.receipt(0);
            assert!(h.decide(&exit, AutomationDecisionChoice::Extend));
        }
        h.finish().await;
        assert_eq!(h.launches().len(), if early_second { 2 } else { 1 });
    }
}

#[tokio::test]
async fn startup_return_never_sends_an_llm_prompt() {
    for rc in [0, 127] {
        let h = Harness::new();
        let mut steps = segment("one", "claude", true);
        if let AutomationStep::LaunchCli {
            startup_wait_ms, ..
        } = &mut steps[0]
        {
            *startup_wait_ms = Some(500);
        }
        h.start(steps, 1, false);
        h.ready().await;
        h.until(|s| s.phase == AutomationPhase::WaitingStartup)
            .await;
        h.receipt(rc);
        let stopped = h.until(|s| !s.running).await;
        assert_eq!(stopped.phase, AutomationPhase::Failed);
        assert!(stopped
            .error
            .unwrap()
            .starts_with("automation-cli-returned-before-task:"));
        assert!(h.exits().is_empty());
        assert!(!h
            .sink
            .recorded_writes()
            .iter()
            .any(|(_, s, _)| s.starts_with("Read ")));
    }
}

#[tokio::test]
async fn missing_launch_ack_fails_without_cli_context_or_retry() {
    let h = Harness::new();
    h.start(segment("one", "claude", true), 1, false);
    h.ready().await;
    let launching = h.until_raw(|s| {
        let writes = h.sink.recorded_writes();
        let source = writes.iter().position(|(_, text, _)| text.starts_with(". '"));
        s.phase == AutomationPhase::Launching && source.is_some_and(|i| writes[i + 1..].iter().any(|(_, text, _)| text == "\r"))
    }).await;
    assert_eq!(launching.cli_context, Some(AutomationCliContext::Unknown));
    let failed = h.until_raw(|s| !s.running).await;
    assert_eq!(failed.phase, AutomationPhase::Failed);
    assert_eq!(
        failed.error.as_deref(),
        Some("automation-cli-launch-ack-timeout")
    );
    assert_eq!(h.launches().len(), 1);
    assert!(h.exits().is_empty());
}

#[tokio::test]
async fn delayed_launch_ack_starts_readiness_only_after_the_ack() {
    let h = Harness::new();
    h.start(segment("one", "claude", false), 1, false);
    h.ready().await;
    let launching = h.until_raw(|s| {
        let writes = h.sink.recorded_writes();
        let source = writes.iter().position(|(_, text, _)| text.starts_with(". '"));
        s.phase == AutomationPhase::Launching && source.is_some_and(|i| writes[i + 1..].iter().any(|(_, text, _)| text == "\r"))
    }).await;
    assert_eq!(launching.cli_context, Some(AutomationCliContext::Unknown));
    h.until_raw(|_| h.sink.recorded_writes().iter().any(|(_, text, _)| text.starts_with(". '"))).await;
    let writes_before_ack = h.sink.recorded_writes().len();
    tokio::time::sleep(Duration::from_millis(80)).await; // exceeds startupWaitMs=30
    assert_eq!(h.status().phase, AutomationPhase::Launching);
    assert_eq!(h.sink.recorded_writes().len(), writes_before_ack);
    let acknowledged_at = tokio::time::Instant::now();
    h.acknowledge_pending_launch();
    tokio::time::sleep(Duration::from_millis(15)).await;
    assert_ne!(h.status().phase, AutomationPhase::Exiting);
    h.until(|s| s.phase == AutomationPhase::Exiting).await;
    assert!(acknowledged_at.elapsed() >= Duration::from_millis(30));
    assert_eq!(h.launches().len(), 1);
}

#[tokio::test]
async fn startup_human_confirmation_never_relaunches_and_keeps_other_producers_deferred() {
    let h = Harness::new();
    let mut steps = segment("one", "claude", true);
    if let AutomationStep::LaunchCli {
        startup_wait_ms, ..
    } = &mut steps[0]
    {
        *startup_wait_ms = Some(500);
    }
    h.start(steps, 1, false);
    h.ready().await;
    h.until(|s| s.phase == AutomationPhase::WaitingStartup)
        .await;
    h.ctx.gate.note_human("a", "\x03");
    let confirm = h
        .until(|s| s.decision_reason == Some(AutomationDecisionReason::CliExitUnconfirmed))
        .await;
    assert_eq!(
        h.ctx.gate.submit("a", "s", "bot", InjectSource::Bot).await,
        SubmitOutcome::Deferred(PendingReason::AnotherProducer)
    );
    assert!(h.decide(&confirm, AutomationDecisionChoice::Continue));
    h.until(|s| s.phase == AutomationPhase::Watching && !s.file_path.is_empty())
        .await;
    assert_eq!(h.launches().len(), 1);
    h.runtime.stop("a");
}

#[tokio::test]
async fn manual_return_and_nonzero_return_require_explicit_confirmation() {
    for manual in [false, true] {
        let h = Harness::new();
        h.start(segment("one", "claude", false), 1, manual);
        h.ready().await;
        if !manual {
            let timeout = h
                .until(|s| s.decision_reason == Some(AutomationDecisionReason::CliExitTimeout))
                .await;
            h.receipt(3);
            assert!(h.decide(&timeout, AutomationDecisionChoice::Extend));
        }
        let confirm = h
            .until(|s| s.decision_reason == Some(AutomationDecisionReason::CliExitUnconfirmed))
            .await;
        assert!(confirm.running);
        assert_eq!(h.exits().len(), 1);
        assert!(!h.decide(&confirm, AutomationDecisionChoice::Extend));
        assert!(h.decide(&confirm, AutomationDecisionChoice::Continue));
        h.finish().await;
    }
}

#[tokio::test]
async fn human_edit_after_return_invalidates_the_next_launch_confirmation() {
    let h = Harness::new();
    let mut steps = segment("one", "claude", false);
    steps.extend(segment("two", "agy", false));
    h.start(steps, 1, false);
    h.ready().await;
    let timeout = h
        .until(|s| s.decision_reason == Some(AutomationDecisionReason::CliExitTimeout))
        .await;
    h.receipt(0);
    h.ctx.gate.note_human("a", "\x1a");
    assert!(h.decide(&timeout, AutomationDecisionChoice::Extend));
    let confirm = h
        .until(|s| s.decision_reason == Some(AutomationDecisionReason::ShellReady))
        .await;
    assert_eq!(h.launches().len(), 1);
    h.ctx.gate.note_human("a", "new text");
    assert!(h.decide(&confirm, AutomationDecisionChoice::Continue));
    let again = h
        .until(|s| {
            s.decision_reason == Some(AutomationDecisionReason::ShellReady)
                && s.decision_id.is_some()
                && s.decision_id != confirm.decision_id
        })
        .await;
    assert_eq!(h.launches().len(), 1);
    assert!(h.decide(&again, AutomationDecisionChoice::Stop));
}

#[tokio::test]
async fn stop_during_return_wait_releases_owner_without_more_input() {
    let h = Harness::new();
    h.start(segment("one", "claude", false), 1, false);
    h.ready().await;
    h.until(|s| s.decision_reason == Some(AutomationDecisionReason::CliExitTimeout))
        .await;
    let before = h.sink.recorded_writes().len();
    h.runtime.stop("a");
    h.receipt(0);
    h.until(|s| !s.running).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(h.sink.recorded_writes().len(), before);
    assert_eq!(
        h.ctx.gate.submit("a", "s", "bot", InjectSource::Bot).await,
        SubmitOutcome::Submitted
    );
}

#[tokio::test]
async fn terminal_queries_during_startup_do_not_pause_automation_as_human_input() {
    let h = Harness::new();
    let mut steps = segment("one", "claude", true);
    if let AutomationStep::LaunchCli {
        startup_wait_ms, ..
    } = &mut steps[0]
    {
        *startup_wait_ms = Some(200);
    }
    h.start(steps, 1, false);
    h.ready().await;
    h.until(|s| s.phase == AutomationPhase::WaitingStartup)
        .await;
    for reply in ["\x1b[12;34R", "\x1b]11;rgb:0000/0000/0000\x1b\\", "\x1b[I"] {
        h.ctx.gate.note_terminal_response("a", reply);
    }
    let ready = h
        .until(|s| s.phase == AutomationPhase::Watching && !s.file_path.is_empty())
        .await;
    assert_eq!(ready.pending_reason, None);
    assert_eq!(h.ctx.gate.human_input_revision("a"), 0);
    assert_eq!(h.launches().len(), 1);
    assert!(h
        .sink
        .recorded_writes()
        .iter()
        .any(|(_, data, _)| data == "\x1b[12;34R"));
    h.runtime.reset_session("a").await;
}
