use super::{store::AutomationStore, AutomationContext, AutomationRuntime};
use crate::session::inject::{InjectGate, RecordingSink};
use crate::state::BotPromptArms;
use crate::types::*;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

struct Harness {
    runtime: AutomationRuntime,
    context: Arc<AutomationContext>,
    sink: Arc<RecordingSink>,
    store: Arc<AutomationStore>,
    root: tempfile::TempDir,
}
impl Harness {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let sink = Arc::new(RecordingSink::default());
        sink.set_session("a", "original");
        sink.set_running("a", true);
        sink.set_cwd("a", root.path().to_str().unwrap());
        // v2 deliberately enables automatic return only for a directly managed
        // POSIX shell.  Unit tests use the same capability as the real sink.
        sink.set_shell_path("a", "/bin/bash");
        let context = Arc::new(AutomationContext {
            gate: Arc::new(InjectGate::new(
                sink.clone(),
                Arc::new(BotPromptArms::new()),
            )),
            sink: sink.clone(),
            bot_runtime: Arc::new(crate::bot::BotRuntime::default()),
        });
        let store = Arc::new(AutomationStore::new(root.path().join("data")));
        Self {
            runtime: AutomationRuntime::default(),
            context,
            sink,
            store,
            root,
        }
    }
    fn start(&self, steps: Vec<AutomationStep>) -> Result<AutomationAgentStatus, String> {
        self.start_definition(AutomationDefinition {
            schema_version: 1,
            id: "definition".into(),
            revision: 1,
            name: "integration".into(),
            inputs: vec![],
            steps,
            repeat: None,
            workspace: None,
            input_values: None,
        })
    }
    fn start_definition(
        &self,
        definition: AutomationDefinition,
    ) -> Result<AutomationAgentStatus, String> {
        self.runtime.start_definition(
            self.context.clone(),
            "a".into(),
            definition,
            BTreeMap::new(),
            self.root.path().display().to_string(),
            self.store.clone(),
        )
    }
    fn status(&self) -> AutomationAgentStatus {
        self.runtime.status().agents["a"].clone()
    }
    async fn until(
        &self,
        predicate: impl Fn(&AutomationAgentStatus) -> bool,
    ) -> AutomationAgentStatus {
        let end = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            self.acknowledge_pending_launch();
            let status = self.status();
            if predicate(&status) {
                return status;
            }
            assert!(
                tokio::time::Instant::now() < end,
                "unexpected status: {status:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    fn acknowledge_pending_launch(&self) {
        let Some(run_id) = self.runtime.status().agents.get("a").and_then(|s| s.run_id.clone()) else { return; };
        let root = self.store.app_data_dir().join("automation-cli-returns").join(run_id);
        let Ok(entries) = std::fs::read_dir(root) else { return; };
        for entry in entries.flatten() {
            let started = entry.path().join("started.json");
            if started.exists() { continue; }
            let Ok(script) = std::fs::read_to_string(entry.path().join("launch.sh")) else { continue; };
            if let Some(ack) = script.split("printf '%s\\n' '").nth(1).and_then(|s| s.split("' >").next()) {
                Self::write_atomic(&started, ack.as_bytes());
            }
        }
    }
    fn write_atomic(path: &std::path::Path, contents: &[u8]) {
        let temporary = path.with_extension("fixture.tmp");
        std::fs::write(&temporary, contents).unwrap();
        std::fs::rename(temporary, path).unwrap();
    }
    fn decide(&self, status: &AutomationAgentStatus, choice: AutomationDecisionChoice) -> bool {
        self.runtime
            .decide(
                "a",
                status.run_id.as_deref().unwrap(),
                status.step_execution_id.as_deref().unwrap(),
                status.decision_id.as_deref().unwrap(),
                choice,
            )
            .unwrap()
    }
    fn result(&self, status: &AutomationAgentStatus, result: &str, version: u32) {
        std::fs::write(&status.file_path, serde_json::to_vec(&serde_json::json!({
            "version": version, "runId": status.run_id, "stepExecutionId": status.step_execution_id,
            "status": result, "summary": "integration marker",
        })).unwrap()).unwrap();
    }
    async fn final_record(&self, expected: &str) {
        let end = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let records = self.store.runs().unwrap();
            if records.len() == 1
                && records[0].status == expected
                && records[0]
                    .events
                    .last()
                    .is_some_and(|event| event.kind == expected)
            {
                assert!(
                    records[0].events.iter().any(|e| e.kind == "started"),
                    "started event was overwritten"
                );
                assert!(
                    records[0].events.len() > 1,
                    "transition history must accumulate"
                );
                return;
            }
            assert!(
                tokio::time::Instant::now() < end,
                "expected {expected} record, got {records:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

#[test]
fn stale_expected_session_rejects_before_creating_history_or_writing() {
    let h = Harness::new();
    let definition = AutomationDefinition {
        schema_version: 1,
        id: "definition".into(),
        revision: 1,
        name: "integration".into(),
        inputs: vec![],
        steps: vec![AutomationStep::LaunchCli {
            id: "launch".into(),
            cli_profile_id: "claude".into(),
            model: None,
            effort: None,
            startup_wait_ms: None,
        }],
        repeat: None,
        workspace: None,
        input_values: None,
    };
    assert_eq!(
        h.runtime.start_definition_for_session(
            h.context.clone(),
            "a".into(),
            definition,
            BTreeMap::new(),
            h.root.path().display().to_string(),
            h.store.clone(),
            "stale-session",
        ),
        Err("automation-session-changed".into())
    );
    assert!(h.sink.recorded_writes().is_empty());
    assert!(h.store.runs().unwrap().is_empty());
    assert!(h.runtime.status().agents.is_empty());
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.runtime.stop_all();
    }
}
fn confirm() -> AutomationStep {
    AutomationStep::Confirm {
        id: "confirm".into(),
        message: "Continue?".into(),
    }
}
fn task(timeout: u64, early: bool) -> AutomationStep {
    AutomationStep::LlmTask {
        id: "task".into(),
        label: "task".into(),
        prompt_template: "preserve\nmultiple lines".into(),
        wait_timeout_ms: Some(timeout),
        completion_grace_ms: Some(20),
        allow_early_complete: Some(early),
    }
}

fn v2_definition(steps: Vec<AutomationStep>) -> AutomationDefinition {
    AutomationDefinition {
        schema_version: 2,
        id: "definition-v2".into(),
        revision: 1,
        name: "v2 integration".into(),
        inputs: vec![],
        steps,
        repeat: None,
        workspace: None,
        input_values: None,
    }
}

fn v2_segment(task_timeout_ms: u64) -> Vec<AutomationStep> {
    vec![
        AutomationStep::LaunchCli {
            id: "launch".into(),
            cli_profile_id: "codex".into(),
            model: None,
            effort: None,
            startup_wait_ms: Some(1),
        },
        AutomationStep::LlmTask {
            id: "task".into(),
            label: "v2 task".into(),
            prompt_template: "v2 prompt {{previousResult}}".into(),
            wait_timeout_ms: Some(task_timeout_ms),
            completion_grace_ms: Some(1),
            allow_early_complete: Some(false),
        },
        AutomationStep::ExitCli {
            id: "exit".into(),
            command: "/exit".into(),
            return_mode: Some(AutomationCliReturnMode::Manual),
            exit_wait_ms: None,
        },
    ]
}

#[tokio::test]
async fn kilo_profile_launches_in_v1_and_v2_with_the_kilo_status() {
    let v1 = Harness::new();
    v1.start(vec![
        AutomationStep::LaunchCli {
            id: "launch".into(),
            cli_profile_id: "kilo".into(),
            model: Some("kilo/openai/gpt-latest".into()),
            effort: None,
            startup_wait_ms: Some(1),
        },
        task(1_000, false),
    ])
    .unwrap();
    let watching = v1
        .until(|status| status.phase == AutomationPhase::Watching)
        .await;
    assert_eq!(watching.cli, AutomationCli::Kilo);
    assert!(v1.sink.recorded_writes().iter().any(|(_, text, _)| {
        text == "kilo --auto --model 'kilo/openai/gpt-latest'"
    }));
    v1.result(&watching, "done", 1);
    v1.until(|status| !status.running).await;
    v1.final_record("completed").await;

    let v2 = Harness::new();
    let mut steps = v2_segment(1_000);
    if let AutomationStep::LaunchCli { cli_profile_id, .. } = &mut steps[0] {
        *cli_profile_id = "kilo".into();
    }
    v2.start_definition(v2_definition(steps)).unwrap();
    let shell_ready = v2
        .until(|status| {
            status.decision_reason == Some(AutomationDecisionReason::ShellReady)
                && status.cli == AutomationCli::Kilo
        })
        .await;
    assert!(v2.decide(&shell_ready, AutomationDecisionChoice::Continue));
    let launching = v2
        .until(|status| {
            status.phase == AutomationPhase::Watching && status.cli == AutomationCli::Kilo
        })
        .await;
    assert_eq!(launching.cli, AutomationCli::Kilo);
    v2.runtime.stop("a");
    v2.until(|status| !status.running).await;
    v2.final_record("cancelled").await;
}

/// pi must use its own `--thinking` flag while preserving the normal result
/// marker completion path in both schema versions.
#[tokio::test]
async fn pi_profile_launches_and_completes_in_v1_and_v2() {
    let v1 = Harness::new();
    v1.start(vec![
        AutomationStep::LaunchCli {
            id: "launch".into(),
            cli_profile_id: "pi".into(),
            model: Some("openai/gpt-5".into()),
            effort: Some("minimal".into()),
            startup_wait_ms: Some(1),
        },
        task(1_000, false),
    ])
    .unwrap();
    let watching = v1.until(|status| status.phase == AutomationPhase::Watching).await;
    assert_eq!(watching.cli, AutomationCli::Pi);
    assert!(v1.sink.recorded_writes().iter().any(|(_, text, _)| {
        text == "pi --approve --model 'openai/gpt-5' --thinking 'minimal'"
    }));
    v1.result(&watching, "done", 1);
    v1.until(|status| !status.running && status.phase == AutomationPhase::Completed).await;
    v1.final_record("completed").await;

    let v2 = Harness::new();
    let mut steps = v2_segment(1_000);
    if let AutomationStep::LaunchCli { cli_profile_id, model, effort, .. } = &mut steps[0] {
        *cli_profile_id = "pi".into();
        *model = Some("anthropic/claude-sonnet-4".into());
        *effort = Some("max".into());
    }
    if let Some(AutomationStep::ExitCli { command, .. }) = steps.last_mut() {
        *command = "/quit".into();
    }
    v2.start_definition(v2_definition(steps)).unwrap();
    let shell_ready = v2.until(|status| {
        status.decision_reason == Some(AutomationDecisionReason::ShellReady)
            && status.cli == AutomationCli::Pi
    }).await;
    assert!(v2.decide(&shell_ready, AutomationDecisionChoice::Continue));
    let watching = v2.until(|status| {
        status.phase == AutomationPhase::Watching
            && status.cli == AutomationCli::Pi
            && !status.file_path.is_empty()
    }).await;
    v2.result(&watching, "done", 1);
    let exit = v2.until(|status| {
        status.decision_reason == Some(AutomationDecisionReason::CliExitUnconfirmed)
    }).await;
    assert!(v2.sink.recorded_writes().iter().any(|(_, text, _)| text == "/quit"));
    assert!(v2.decide(&exit, AutomationDecisionChoice::Continue));
    v2.until(|status| !status.running && status.phase == AutomationPhase::Completed).await;
    v2.final_record("completed").await;
}

#[tokio::test]
async fn first_confirm_is_actionable_and_duplicate_decision_is_ignored() {
    let h = Harness::new();
    h.start(vec![confirm()]).unwrap();
    let decision = h.until(|s| s.phase == AutomationPhase::Confirming).await;
    assert!(decision.step_execution_id.is_some());
    assert_eq!(decision.decision_message.as_deref(), Some("Continue?"));
    assert!(h.sink.recorded_writes().is_empty());
    assert!(h.decide(&decision, AutomationDecisionChoice::Continue));
    assert!(!h.decide(&decision, AutomationDecisionChoice::Continue));
    h.until(|s| s.phase == AutomationPhase::Completed && !s.running)
        .await;
    h.final_record("completed").await;
}

#[tokio::test]
async fn complete_without_early_completion_reaches_next_confirmation() {
    let h = Harness::new();
    h.start(vec![task(1000, false), confirm()]).unwrap();
    let watching = h
        .until(|s| s.phase == AutomationPhase::Watching && s.step_id.as_deref() == Some("task"))
        .await;
    h.result(&watching, "complete", 1);
    let decision = h.until(|s| s.phase == AutomationPhase::Confirming).await;
    assert_ne!(watching.step_execution_id, decision.step_execution_id);
    assert_eq!(decision.step_index, Some(1));
    assert!(h.decide(&decision, AutomationDecisionChoice::Stop));
    h.until(|s| !s.running && s.phase == AutomationPhase::Cancelled)
        .await;
    h.final_record("cancelled").await;
}

#[tokio::test]
async fn timeout_observes_marker_without_advancing_until_extension() {
    let h = Harness::new();
    h.start(vec![task(40, false)]).unwrap();
    let decision = h
        .until(|s| s.phase == AutomationPhase::TimeoutDecision)
        .await;
    h.until(|_| {
        h.store.runs().unwrap()[0]
            .events
            .iter()
            .any(|event| event.kind == "decision-timeout")
    })
    .await;
    let writes = h.sink.recorded_writes().len();
    h.result(&decision, "done", 1);
    let observed = h.until(|s| s.marker_observed_at_ms.is_some()).await;
    assert_eq!(observed.phase, AutomationPhase::TimeoutDecision);
    assert_eq!(observed.decision_id, decision.decision_id);
    assert_eq!(h.sink.recorded_writes().len(), writes);
    assert!(h.decide(&observed, AutomationDecisionChoice::Extend));
    h.until(|s| !s.running && s.phase == AutomationPhase::Completed)
        .await;
    h.final_record("completed").await;
    let record = &h.store.runs().unwrap()[0];
    for kind in ["step", "marker-observed", "decision-extend", "completed"] {
        assert!(
            record.events.iter().any(|event| event.kind == kind),
            "missing persisted event {kind}: {:?}",
            record.events
        );
    }
}

#[tokio::test]
async fn stop_between_body_and_enter_writes_no_enter_and_finalizes_history() {
    let h = Harness::new();
    h.start(vec![task(1000, false)]).unwrap();
    h.until(|_| !h.sink.recorded_writes().is_empty()).await;
    h.runtime.stop("a");
    h.until(|s| !s.running && s.phase == AutomationPhase::Cancelled)
        .await;
    tokio::time::sleep(Duration::from_millis(180)).await;
    assert!(!h
        .sink
        .recorded_writes()
        .iter()
        .any(|(_, text, _)| text == "\r"));
    h.final_record("cancelled").await;
}

#[tokio::test]
async fn duplicate_start_does_not_create_a_phantom_running_record() {
    let h = Harness::new();
    h.start(vec![confirm()]).unwrap();
    assert!(h.start(vec![confirm()]).is_err());
    assert_eq!(h.store.runs().unwrap().len(), 1);
}

#[tokio::test]
async fn changed_session_during_wait_fails_without_writing_to_new_session() {
    let h = Harness::new();
    h.start(vec![
        AutomationStep::Wait {
            id: "wait".into(),
            duration_ms: 200,
        },
        task(1000, false),
    ])
    .unwrap();
    h.sink.set_session("a", "replacement");
    h.until(|s| !s.running && s.phase == AutomationPhase::Failed)
        .await;
    assert!(h.sink.recorded_writes().is_empty());
    h.final_record("failed").await;
}

#[tokio::test]
async fn blocked_can_be_resolved_without_replaying_the_prompt() {
    let h = Harness::new();
    h.start(vec![task(1000, false), confirm()]).unwrap();
    let watching = h.until(|s| s.phase == AutomationPhase::Watching).await;
    let writes = h.sink.recorded_writes().len();
    h.result(&watching, "blocked", 1);
    let blocked = h
        .until(|s| s.decision_reason == Some(AutomationDecisionReason::Blocked))
        .await;
    h.result(&watching, "done", 1);
    assert!(h.decide(&blocked, AutomationDecisionChoice::Extend));
    let next = h
        .until(|s| s.decision_reason == Some(AutomationDecisionReason::Confirm))
        .await;
    assert_eq!(h.sink.recorded_writes().len(), writes);
    assert!(h.decide(&next, AutomationDecisionChoice::Continue));
    h.until(|s| !s.running).await;
    h.final_record("completed").await;
}

#[tokio::test]
async fn blocked_continue_explicitly_skips_to_the_next_step() {
    let h = Harness::new();
    h.start(vec![task(2000, false), confirm()]).unwrap();
    let watching = h.until(|s| s.phase == AutomationPhase::Watching).await;
    h.result(&watching, "blocked", 1);
    let blocked = h
        .until(|s| s.decision_reason == Some(AutomationDecisionReason::Blocked))
        .await;
    let writes = h.sink.recorded_writes().len();
    assert!(h.decide(&blocked, AutomationDecisionChoice::Continue));
    let next = h.until(|s| s.phase == AutomationPhase::Confirming).await;
    assert_eq!(h.sink.recorded_writes().len(), writes);
    assert!(h.decide(&next, AutomationDecisionChoice::Continue));
    h.until(|s| !s.running).await;
    h.final_record("completed").await;
}

#[tokio::test]
async fn protocol_error_can_be_repaired_in_the_same_run() {
    let h = Harness::new();
    h.start(vec![task(1000, false)]).unwrap();
    let watching = h.until(|s| s.phase == AutomationPhase::Watching).await;
    h.result(&watching, "done", 99);
    let error = h
        .until(|s| s.decision_reason == Some(AutomationDecisionReason::ProtocolError))
        .await;
    assert!(error.running);
    assert_eq!(error.run_id, watching.run_id);
    h.result(&watching, "done", 1);
    assert!(h.decide(&error, AutomationDecisionChoice::Extend));
    h.until(|s| !s.running && s.phase == AutomationPhase::Completed)
        .await;
    h.final_record("completed").await;
}

#[tokio::test]
async fn human_submission_during_wait_after_marker_requires_continue() {
    let h = Harness::new();
    let mut second = task(1000, false);
    if let AutomationStep::LlmTask { id, .. } = &mut second {
        *id = "second".into();
    }
    h.start(vec![
        task(1000, false),
        AutomationStep::Wait {
            id: "pause".into(),
            duration_ms: 300,
        },
        second,
    ])
    .unwrap();
    let first = h.until(|s| s.phase == AutomationPhase::Watching).await;
    h.result(&first, "done", 1);
    h.until(|s| s.step_id.as_deref() == Some("pause")).await;
    h.context.gate.note_human("a", "manual follow-up\r");
    let writes = h.sink.recorded_writes().len();
    let decision = h
        .until(|s| s.phase == AutomationPhase::WaitingHumanInput)
        .await;
    assert_eq!(h.sink.recorded_writes().len(), writes);
    assert!(h.decide(&decision, AutomationDecisionChoice::Continue));
    let second = h
        .until(|s| s.step_id.as_deref() == Some("second") && s.phase == AutomationPhase::Watching)
        .await;
    h.result(&second, "done", 1);
    h.until(|s| !s.running && s.phase == AutomationPhase::Completed)
        .await;
}

#[tokio::test]
async fn early_complete_waits_for_grace_and_human_choice_before_exit() {
    let h = Harness::new();
    let mut early = task(1000, true);
    if let AutomationStep::LlmTask {
        completion_grace_ms,
        ..
    } = &mut early
    {
        *completion_grace_ms = Some(300);
    }
    h.start(vec![
        early,
        AutomationStep::ExitCli {
            id: "exit".into(),
            command: "/exit".into(),
            return_mode: None,
            exit_wait_ms: None,
        },
    ])
    .unwrap();
    let watching = h.until(|s| s.phase == AutomationPhase::Watching).await;
    h.result(&watching, "complete", 1);
    h.until(|s| s.phase == AutomationPhase::Settling).await;
    h.context.gate.note_human("a", "manual\r");
    let decision = h
        .until(|s| s.phase == AutomationPhase::WaitingHumanInput)
        .await;
    assert!(!h
        .sink
        .recorded_writes()
        .iter()
        .any(|(_, text, _)| text == "/exit"));
    assert!(h.decide(&decision, AutomationDecisionChoice::Continue));
    h.until(|s| !s.running && s.phase == AutomationPhase::Completed)
        .await;
    assert_eq!(
        h.sink
            .recorded_writes()
            .iter()
            .filter(|(_, text, _)| text == "/exit")
            .count(),
        1
    );
    h.final_record("completed").await;
}

#[tokio::test]
async fn previous_result_flows_between_steps_without_evaluating_its_tokens() {
    let h = Harness::new();
    let mut first = task(2000, false);
    let mut second = task(2000, false);
    if let AutomationStep::LlmTask {
        prompt_template, ..
    } = &mut first
    {
        *prompt_template = "First: {{previousResult}}".into();
    }
    if let AutomationStep::LlmTask {
        id,
        prompt_template,
        ..
    } = &mut second
    {
        *id = "review".into();
        *prompt_template = "Review: {{previousResult}}".into();
    }
    h.start(vec![first, second]).unwrap();
    let first = h.until(|s| s.phase == AutomationPhase::Watching).await;
    let prompt = std::path::Path::new(&first.file_path).with_file_name("prompt.md");
    assert_eq!(std::fs::read_to_string(prompt).unwrap(), "First: ");
    std::fs::write(
        &first.file_path,
        serde_json::to_vec(&serde_json::json!({
            "version": 1, "runId": first.run_id, "stepExecutionId": first.step_execution_id,
            "status": "done", "summary": "검토 사항 {{workspace}}",
        }))
        .unwrap(),
    )
    .unwrap();
    let review = h
        .until(|s| s.phase == AutomationPhase::Watching && s.step_id.as_deref() == Some("review"))
        .await;
    let prompt = std::path::Path::new(&review.file_path).with_file_name("prompt.md");
    assert_eq!(
        std::fs::read_to_string(prompt).unwrap(),
        "Review: 검토 사항 {{workspace}}"
    );
    h.result(&review, "done", 1);
    h.until(|s| !s.running).await;
    h.final_record("completed").await;
}

#[tokio::test]
async fn enter_during_deferred_next_submission_requires_human_choice() {
    let h = Harness::new();
    let mut first = task(2000, false);
    if let AutomationStep::LlmTask {
        completion_grace_ms,
        ..
    } = &mut first
    {
        *completion_grace_ms = Some(200);
    }
    let mut second = task(2000, false);
    if let AutomationStep::LlmTask { id, .. } = &mut second {
        *id = "second".into();
    }
    h.start(vec![first, second]).unwrap();
    let watching = h.until(|s| s.phase == AutomationPhase::Watching).await;
    h.result(&watching, "done", 1);
    h.until(|s| s.phase == AutomationPhase::Settling).await;
    h.context.gate.note_human("a", "unfinished follow-up");
    h.until(|s| s.pending_reason.is_some() && s.step_id.as_deref() == Some("second"))
        .await;
    h.context.gate.note_human("a", "\r");
    let writes = h.sink.recorded_writes().len();
    let decision = h
        .until(|s| s.phase == AutomationPhase::WaitingHumanInput)
        .await;
    assert_eq!(h.sink.recorded_writes().len(), writes);
    assert!(h.decide(&decision, AutomationDecisionChoice::Continue));
    let watching = h.until(|s| s.phase == AutomationPhase::Watching).await;
    h.result(&watching, "done", 1);
    h.until(|s| !s.running).await;
    h.final_record("completed").await;
}

#[tokio::test]
async fn timeout_marker_epoch_is_preserved_across_extension() {
    let h = Harness::new();
    h.start(vec![task(40, false), confirm()]).unwrap();
    let timeout = h
        .until(|s| s.phase == AutomationPhase::TimeoutDecision)
        .await;
    h.result(&timeout, "done", 1);
    h.until(|s| s.marker_observed_at_ms.is_some()).await;
    h.context.gate.note_human("a", "manual after marker\r");
    assert!(h.decide(&timeout, AutomationDecisionChoice::Extend));
    let human = h
        .until(|s| s.phase == AutomationPhase::WaitingHumanInput)
        .await;
    assert!(h.decide(&human, AutomationDecisionChoice::Continue));
    let confirm = h.until(|s| s.phase == AutomationPhase::Confirming).await;
    assert!(h.decide(&confirm, AutomationDecisionChoice::Continue));
    h.until(|s| !s.running).await;
    h.final_record("completed").await;
}

#[tokio::test]
async fn stop_while_submission_is_deferred_never_writes_a_new_body() {
    let h = Harness::new();
    h.context.gate.note_human("a", "unfinished");
    h.start(vec![task(2000, false)]).unwrap();
    h.until(|s| s.pending_reason.is_some()).await;
    h.runtime.stop("a");
    h.context.gate.note_human("a", "\r");
    let writes = h.sink.recorded_writes().len();
    h.until(|s| !s.running).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(h.sink.recorded_writes().len(), writes);
    h.final_record("cancelled").await;
}

#[tokio::test]
async fn forced_human_fragment_submission_is_recorded() {
    let mut h = Harness::new();
    h.context = Arc::new(AutomationContext {
        gate: Arc::new(InjectGate::with_hold_max(
            h.sink.clone(),
            Arc::new(BotPromptArms::new()),
            50,
        )),
        sink: h.sink.clone(),
        bot_runtime: h.context.bot_runtime.clone(),
    });
    h.context.gate.note_human("a", "unfinished");
    h.start(vec![task(2000, false)]).unwrap();
    let watching = h.until(|s| s.phase == AutomationPhase::Watching).await;
    assert!(h.store.runs().unwrap()[0]
        .events
        .iter()
        .any(|e| e.kind == "submitted-over-human-fragment"));
    h.result(&watching, "done", 1);
    h.until(|s| !s.running).await;
    h.final_record("completed").await;
}

#[tokio::test]
async fn unknown_result_status_can_be_repaired_without_replaying_prompt() {
    let h = Harness::new();
    h.start(vec![task(2000, false)]).unwrap();
    let watching = h.until(|s| s.phase == AutomationPhase::Watching).await;
    h.result(&watching, "invalid-status", 1);
    let decision = h
        .until(|s| s.decision_reason == Some(AutomationDecisionReason::ProtocolError))
        .await;
    let writes = h.sink.recorded_writes().len();
    h.result(&watching, "done", 1);
    assert!(h.decide(&decision, AutomationDecisionChoice::Extend));
    h.until(|s| !s.running).await;
    assert_eq!(h.sink.recorded_writes().len(), writes);
    h.final_record("completed").await;
}

#[tokio::test]
async fn continuing_human_input_still_guards_later_input_before_next_submission() {
    let h = Harness::new();
    let mut first = task(2000, false);
    if let AutomationStep::LlmTask {
        completion_grace_ms,
        ..
    } = &mut first
    {
        *completion_grace_ms = Some(200);
    }
    let mut second = task(2000, false);
    if let AutomationStep::LlmTask { id, .. } = &mut second {
        *id = "second".into();
    }
    h.start(vec![
        first,
        AutomationStep::Wait {
            id: "pause".into(),
            duration_ms: 200,
        },
        second,
    ])
    .unwrap();
    let watching = h.until(|s| s.phase == AutomationPhase::Watching).await;
    h.result(&watching, "done", 1);
    h.until(|s| s.phase == AutomationPhase::Settling).await;
    h.context.gate.note_human("a", "first manual\r");
    let first_choice = h
        .until(|s| s.phase == AutomationPhase::WaitingHumanInput)
        .await;
    assert!(h.decide(&first_choice, AutomationDecisionChoice::Continue));
    h.until(|s| s.step_id.as_deref() == Some("pause")).await;
    h.context.gate.note_human("a", "second manual\r");
    let second_choice = h
        .until(|s| s.phase == AutomationPhase::WaitingHumanInput)
        .await;
    assert_ne!(first_choice.decision_id, second_choice.decision_id);
    assert!(h.decide(&second_choice, AutomationDecisionChoice::Continue));
    let watching = h
        .until(|s| {
            s.phase == AutomationPhase::Watching
                && s.step_id.as_deref() == Some("second")
                && !s.file_path.is_empty()
        })
        .await;
    h.result(&watching, "done", 1);
    h.until(|s| !s.running).await;
    h.final_record("completed").await;
}

#[tokio::test]
async fn v2_rejects_an_unverified_shell_before_launching_or_creating_a_task() {
    let h = Harness::new();
    h.sink.set_shell_path("a", "/bin/sh");
    assert_eq!(
        h.start_definition(v2_definition(v2_segment(1_000))),
        Err("automation-cli-return-shell-unsupported".into())
    );
    assert!(h.sink.recorded_writes().is_empty());
    assert!(h.store.runs().unwrap().is_empty());
}

#[tokio::test]
async fn v2_human_input_before_launch_requires_choices_and_launch_is_not_resent() {
    let h = Harness::new();
    h.context.gate.note_human("a", "manual before launch\r");
    h.start_definition(v2_definition(v2_segment(1_000)))
        .unwrap();

    assert!(h
        .sink
        .recorded_writes()
        .iter()
        .any(|(_, text, _)| text == "manual before launch\r"));
    let confirm = h
        .until(|s| s.decision_reason == Some(AutomationDecisionReason::ShellReady))
        .await;
    assert!(h.decide(&confirm, AutomationDecisionChoice::Continue));
    let watching = h
        .until(|s| s.phase == AutomationPhase::Watching && s.step_id.as_deref() == Some("task"))
        .await;
    h.until(|_| {
        h.sink
            .recorded_writes()
            .iter()
            .any(|(_, text, _)| text.contains("automation-cli-returns"))
    })
    .await;
    let launch_writes = h
        .sink
        .recorded_writes()
        .iter()
        .filter(|(_, text, _)| text.contains("automation-cli-returns"))
        .count();
    assert_eq!(launch_writes, 1);
    assert_eq!(watching.step_id.as_deref(), Some("task"));
    h.runtime.stop("a");
    h.until(|s| !s.running && s.phase == AutomationPhase::Cancelled)
        .await;
    h.final_record("cancelled").await;
}

#[tokio::test]
async fn v2_timeout_does_not_resend_the_active_cli_launch() {
    let h = Harness::new();
    h.start_definition(v2_definition(v2_segment(40))).unwrap();
    let confirm = h
        .until(|s| s.decision_reason == Some(AutomationDecisionReason::ShellReady))
        .await;
    assert!(h.decide(&confirm, AutomationDecisionChoice::Continue));
    h.until(|s| s.phase == AutomationPhase::Watching).await;
    h.until(|_| {
        h.sink
            .recorded_writes()
            .iter()
            .any(|(_, text, _)| text.contains("automation-cli-returns"))
    })
    .await;
    let launches_before = h
        .sink
        .recorded_writes()
        .iter()
        .filter(|(_, text, _)| text.contains("automation-cli-returns"))
        .count();
    let timeout = h
        .until(|s| s.phase == AutomationPhase::TimeoutDecision)
        .await;
    assert_eq!(
        timeout.decision_reason,
        Some(AutomationDecisionReason::Timeout)
    );
    assert_eq!(
        h.sink
            .recorded_writes()
            .iter()
            .filter(|(_, text, _)| text.contains("automation-cli-returns"))
            .count(),
        launches_before
    );
    assert!(h.decide(&timeout, AutomationDecisionChoice::Stop));
    h.until(|s| !s.running && s.phase == AutomationPhase::Cancelled)
        .await;
    h.final_record("cancelled").await;
}

#[tokio::test]
async fn terminal_reset_cancels_the_run_preserves_history_and_allows_a_fresh_session() {
    let h = Harness::new();
    h.start(vec![confirm(), task(1_000, false)]).unwrap();
    h.until(|s| s.phase == AutomationPhase::Confirming).await;
    let old = h.status();
    h.runtime.reset_session("a").await;
    h.context.gate.remove_agent("a");
    h.sink.set_session("a", "replacement");
    assert!(h.runtime.status().agents.is_empty());
    assert!(h.sink.recorded_writes().is_empty());
    h.final_record("cancelled").await;
    assert!(!h.decide(&old, AutomationDecisionChoice::Continue));

    let fresh = h.start(vec![confirm()]).unwrap();
    assert_eq!(fresh.session_id.as_deref(), Some("replacement"));
    assert_ne!(fresh.run_id, old.run_id);
    h.runtime.reset_session("a").await;
    assert!(h.sink.recorded_writes().is_empty());
    assert_eq!(h.store.runs().unwrap().len(), 2);
}
