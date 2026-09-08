use crate::automation::marker::{self, AutomationResultStatus, ResultRead};
use crate::automation::runner::{cancel_status, fail, now_ms, DecisionCommand};
use crate::automation::{store::AutomationStore, AutomationContext};
use crate::session::inject::{InjectSource, SubmitOutcome};
use crate::types::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

struct RunHistoryGuard {
    store: Arc<AutomationStore>,
    record: AutomationRunRecord,
    status: Arc<Mutex<AutomationAgentStatus>>,
    finished: Arc<Mutex<Option<Instant>>>,
    cancel: Arc<AtomicBool>,
}
impl RunHistoryGuard {
    fn event(&mut self, kind: &str, details: Option<String>) {
        self.record.events.push(AutomationRunEvent {
            at: now_ms(),
            kind: kind.into(),
            details,
        });
        let snapshot = self.status.lock().unwrap().clone();
        self.record.status = if snapshot.running {
            "running"
        } else {
            terminal_name(snapshot.phase)
        }
        .into();
        self.record.outcome = snapshot.outcome;
        if let Err(error) = self.store.append_run(&self.record) {
            eprintln!("automation history write failed: {error}");
        }
    }
    fn phase(&mut self) {
        let snapshot = self.status.lock().unwrap().clone();
        self.event(
            "phase",
            Some(
                serde_json::json!({
                    "phase": snapshot.phase, "stepId": snapshot.step_id,
                    "stepExecutionId": snapshot.step_execution_id, "cycle": snapshot.cycle,
                })
                .to_string(),
            ),
        );
    }
}
fn terminal_name(phase: AutomationPhase) -> &'static str {
    match phase {
        AutomationPhase::Completed | AutomationPhase::Done => "completed",
        AutomationPhase::Cancelled | AutomationPhase::Cancelling => "cancelled",
        AutomationPhase::Failed => "failed",
        _ => "interrupted",
    }
}
impl Drop for RunHistoryGuard {
    fn drop(&mut self) {
        let (phase, error) = {
            let mut s = self.status.lock().unwrap();
            if s.running {
                s.phase = if self.cancel.load(Ordering::Acquire) {
                    AutomationPhase::Cancelled
                } else {
                    AutomationPhase::Interrupted
                };
            }
            s.running = false;
            s.completed_at_ms.get_or_insert(now_ms());
            s.deadline_ms = None;
            s.decision_id = None;
            s.pending_reason = None;
            s.pending_since_ms = None;
            let outcome = terminal_name(s.phase).to_string();
            s.outcome.get_or_insert(outcome);
            (s.phase, s.error.clone())
        };
        self.finished
            .lock()
            .unwrap()
            .get_or_insert_with(Instant::now);
        if self.cancel.load(Ordering::Acquire)
            && !self.record.events.iter().any(|e| e.kind == "decision-stop")
        {
            self.event("decision-stop", None);
        }
        self.event(terminal_name(phase), error);
    }
}

fn subst(
    s: &str,
    inputs: &std::collections::BTreeMap<String, String>,
    workspace: &str,
    run: &str,
    cycle: u32,
) -> String {
    // 입력값 자체의 `{{...}}`를 다시 평가하지 않는다. 순차 replace는 값 안의
    // 토큰까지 다음 루프에서 바꿔 버려 사용자 입력을 코드처럼 해석하게 된다.
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let key = &after[..end];
        let value = match key {
            "workspace" => Some(workspace),
            "run" => Some(run),
            "cycle" => None,
            _ => inputs.get(key).map(String::as_str),
        };
        if key == "cycle" {
            out.push_str(&cycle.to_string());
        } else if let Some(value) = value {
            out.push_str(value);
        } else {
            out.push_str("{{");
            out.push_str(key);
            out.push_str("}}");
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(crate) fn launch_command(
    cli: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<String, String> {
    crate::automation::definition::validate_launch_effort(cli, effort)?;
    let mut command = match cli {
        "claude" => "claude".to_string(),
        "codex" => "codex".to_string(),
        "agy" => "agy --dangerously-skip-permissions".to_string(),
        // Kilo's interactive TUI accepts provider/model via --model. --auto
        // is required for unattended automation to progress through tool use.
        "kilo" => "kilo --auto".to_string(),
        // `--approve` is pi's unattended approval mode. Automation otherwise
        // stalls at tool-use approval prompts before it can write result.json.
        "pi" => "pi --approve".to_string(),
        _ => return Err("automation-cli-profile-unsupported".into()),
    };
    match cli {
        "claude" => {
            if let Some(model) = model {
                command.push_str(" --model ");
                command.push_str(&shell_quote(model));
            }
            if let Some(effort) = effort {
                command.push_str(" --effort ");
                command.push_str(&shell_quote(effort));
            }
        }
        "codex" => {
            if let Some(model) = model {
                command.push_str(" --model ");
                command.push_str(&shell_quote(model));
            }
            if let Some(effort) = effort {
                command.push_str(" -c ");
                command.push_str(&shell_quote(&format!(
                    "model_reasoning_effort=\"{effort}\""
                )));
            }
        }
        "agy" => {
            if let Some(model) = model {
                command.push_str(" --model ");
                command.push_str(&shell_quote(model));
            }
            if let Some(effort) = effort {
                command.push_str(" --effort ");
                command.push_str(&shell_quote(effort));
            }
        }
        "kilo" => {
            if let Some(model) = model {
                command.push_str(" --model ");
                command.push_str(&shell_quote(model));
            }
        }
        "pi" => {
            if let Some(model) = model {
                command.push_str(" --model ");
                command.push_str(&shell_quote(model));
            }
            if let Some(effort) = effort {
                command.push_str(" --thinking ");
                command.push_str(&shell_quote(effort));
            }
        }
        _ => unreachable!(),
    }
    Ok(command)
}
struct DefinitionRunner {
    ctx: Arc<AutomationContext>,
    agent: String,
    sid: String,
    shutdown: oneshot::Receiver<()>,
    decisions: mpsc::UnboundedReceiver<DecisionCommand>,
    history: RunHistoryGuard,
    // Fixed at the first observation, including while a decision panel is open.
    last_marker_epoch: Option<u64>,
    transition: Option<crate::automation::cli_transition::CliTransition>,
    launch: Option<crate::automation::cli_transition::PreparedLaunch>,
    transition_owner: Option<crate::session::inject::TransitionOwnerGuard>,
    confirmed_revision: Option<u64>,
    generation: u64,
    last_transition_submitted: bool,
}
impl DefinitionRunner {
    fn active(&self) -> bool {
        if self.history.cancel.load(Ordering::Acquire) {
            cancel_status(&self.history.status, &self.history.finished);
            return false;
        }
        if !self.ctx.sink.is_running(&self.agent)
            || self.ctx.sink.session_id_for(&self.agent).as_deref() != Some(&self.sid)
        {
            fail(
                &self.history.status,
                &self.history.finished,
                "automation-session-changed".into(),
            );
            return false;
        }
        true
    }
    fn phase(&mut self, phase: AutomationPhase) {
        let changed = {
            let mut s = self.history.status.lock().unwrap();
            let changed = s.phase != phase;
            s.phase = phase;
            changed
        };
        if changed {
            self.history.phase();
        }
    }
    async fn pause(&mut self, duration: Duration) -> bool {
        let end = tokio::time::Instant::now() + duration;
        while self.active() {
            if tokio::time::Instant::now() >= end {
                return true;
            }
            tokio::select! {
                biased;
                _ = &mut self.shutdown => return false,
                _ = tokio::time::sleep_until(end.min(tokio::time::Instant::now() + Duration::from_millis(50))) => {}
            }
        }
        false
    }
    fn observe_marker(&mut self, result: &marker::AutomationResultFile) {
        let first = self
            .history
            .status
            .lock()
            .unwrap()
            .marker_observed_at_ms
            .is_none();
        if first {
            self.last_marker_epoch = Some(self.ctx.gate.human_input_epoch(&self.agent));
            self.history.status.lock().unwrap().marker_observed_at_ms = Some(now_ms());
            self.history.event(
                "marker-observed",
                Some(serde_json::to_string(result).unwrap()),
            );
        }
    }
    async fn decision(
        &mut self,
        reason: AutomationDecisionReason,
        message: Option<String>,
        result: Option<(&std::path::Path, &str)>,
    ) -> Option<DecisionCommand> {
        if !self.active() {
            return None;
        }
        {
            let mut s = self.history.status.lock().unwrap();
            s.phase = match reason {
                AutomationDecisionReason::Confirm => AutomationPhase::Confirming,
                AutomationDecisionReason::HumanInput => AutomationPhase::WaitingHumanInput,
                _ => AutomationPhase::TimeoutDecision,
            };
            s.decision_id = Some(Uuid::new_v4().to_string());
            s.decision_reason = Some(reason);
            s.decision_message = message;
            s.deadline_ms = None;
        }
        self.history.phase();
        let kind = match reason {
            AutomationDecisionReason::Confirm => "decision-confirm",
            AutomationDecisionReason::HumanInput => "decision-human-input",
            AutomationDecisionReason::Blocked => "decision-blocked",
            AutomationDecisionReason::ProtocolError => "decision-protocol-error",
            AutomationDecisionReason::Timeout => "decision-timeout",
            AutomationDecisionReason::ShellReady => "decision-shell-ready",
            AutomationDecisionReason::CliExitTimeout => "decision-cli-exit-timeout",
            AutomationDecisionReason::CliExitUnconfirmed => "decision-cli-exit-unconfirmed",
        };
        self.history.event(kind, None);
        loop {
            if !self.active() {
                return None;
            }
            if reason == AutomationDecisionReason::CliExitTimeout {
                if matches!(
                    self.receipt(),
                    crate::automation::cli_transition::ReceiptRead::Returned { exit_code: 0 }
                ) {
                    self.history
                        .status
                        .lock()
                        .unwrap()
                        .cli_return_observed_at_ms
                        .get_or_insert(now_ms());
                }
            }
            if let Some((path, exec)) = result {
                if let ResultRead::Accepted(value) =
                    marker::read_result_file(path, &self.history.record.run_id, exec)
                {
                    if matches!(
                        value.status,
                        AutomationResultStatus::Done | AutomationResultStatus::Complete
                    ) {
                        self.observe_marker(&value);
                    }
                }
            }
            let choice = tokio::select! {
                biased;
                _ = &mut self.shutdown => return None,
                choice = self.decisions.recv() => choice,
                _ = tokio::time::sleep(Duration::from_millis(50)) => continue,
            };
            let choice = choice?;
            self.history.event(
                match choice {
                    DecisionCommand::Extend => "decision-extend",
                    DecisionCommand::Continue => "decision-continue",
                    DecisionCommand::Stop => "decision-stop",
                },
                None,
            );
            if matches!(choice, DecisionCommand::Stop) {
                cancel_status(&self.history.status, &self.history.finished);
                return None;
            }
            {
                let mut s = self.history.status.lock().unwrap();
                s.decision_id = None;
                s.decision_reason = None;
                s.decision_message = None;
            }
            return Some(choice);
        }
    }
    async fn human_checkpoint(&mut self) -> bool {
        if self
            .last_marker_epoch
            .is_some_and(|epoch| self.ctx.gate.human_input_epoch(&self.agent) > epoch)
        {
            if self
                .decision(AutomationDecisionReason::HumanInput, None, None)
                .await
                .is_none()
            {
                return false;
            }
            self.last_marker_epoch = Some(self.ctx.gate.human_input_epoch(&self.agent));
        }
        self.active()
    }
    async fn submit(&mut self, text: &str) -> bool {
        loop {
            if !self.active() || !self.human_checkpoint().await {
                return false;
            }
            if self.transition.is_some() && !self.cli_still_active() {
                return false;
            }
            self.phase(AutomationPhase::Injecting);
            let before = self.ctx.gate.forced_submission_count(&self.agent);
            let outcome = tokio::select! {
                biased;
                _ = &mut self.shutdown => return false,
                outcome = self.ctx.gate.submit(&self.agent, &self.sid, text, InjectSource::Automation) => outcome,
            };
            if self.ctx.gate.forced_submission_count(&self.agent) != before {
                self.history.event("submitted-over-human-fragment", None);
            }
            match outcome {
                SubmitOutcome::Submitted => {
                    let mut s = self.history.status.lock().unwrap();
                    s.pending_reason = None;
                    s.pending_since_ms = None;
                    return true;
                }
                SubmitOutcome::Deferred(reason) => {
                    {
                        let mut s = self.history.status.lock().unwrap();
                        s.pending_reason = Some(reason);
                        s.pending_since_ms.get_or_insert(now_ms());
                    }
                    if !self.pause(Duration::from_millis(100)).await {
                        return false;
                    }
                }
                _ => {
                    fail(
                        &self.history.status,
                        &self.history.finished,
                        "automation-session-changed".into(),
                    );
                    return false;
                }
            }
        }
    }
    fn complete(&mut self) {
        if !self.active() {
            return;
        }
        {
            let mut s = self.history.status.lock().unwrap();
            s.running = false;
            s.phase = AutomationPhase::Completed;
            s.outcome = Some("complete".into());
            s.completed_at_ms = Some(now_ms());
            s.deadline_ms = None;
            s.decision_id = None;
        }
        self.history.phase();
    }
    async fn llm_task(
        &mut self,
        step: &AutomationStep,
        cycle: u32,
        previous_result: &mut String,
    ) -> Option<bool> {
        let AutomationStep::LlmTask {
            prompt_template,
            wait_timeout_ms,
            completion_grace_ms,
            allow_early_complete,
            ..
        } = step
        else {
            unreachable!()
        };
        let exec = self
            .history
            .status
            .lock()
            .unwrap()
            .step_execution_id
            .clone()
            .unwrap();
        let workspace = self.history.record.workspace.clone();
        let run_id = self.history.record.run_id.clone();
        let dir = match marker::step_dir(std::path::Path::new(&workspace), &run_id, &exec) {
            Ok(dir) => dir,
            Err(error) => {
                fail(&self.history.status, &self.history.finished, error);
                return None;
            }
        };
        if std::fs::create_dir_all(&dir).is_err()
            || marker::verify_within_workspace(std::path::Path::new(&workspace), &dir).is_err()
        {
            fail(
                &self.history.status,
                &self.history.finished,
                "automation-path-invalid".into(),
            );
            return None;
        }
        marker::ensure_ignored(std::path::Path::new(&workspace));
        let prompt_path = dir.join("prompt.md");
        let result_path = dir.join("result.json");
        let mut inputs = self.history.record.inputs.clone();
        inputs.insert("previousResult".into(), previous_result.clone());
        if std::fs::write(
            &prompt_path,
            subst(prompt_template, &inputs, &workspace, &run_id, cycle),
        )
        .is_err()
        {
            fail(
                &self.history.status,
                &self.history.finished,
                "automation-prompt-write-failed".into(),
            );
            return None;
        }
        self.history.status.lock().unwrap().file_path = result_path.display().to_string();
        let status_help = if allow_early_complete.unwrap_or(false) {
            "done (continue), complete (finish this automation early), or blocked (ask for help)"
        } else {
            "done (continue) or blocked (ask for help); do not use complete"
        };
        let command = format!("Read '{}' and finish by writing '{}' JSON {{\"version\":1,\"runId\":\"{}\",\"stepExecutionId\":\"{}\",\"status\":\"...\",\"summary\":\"...\"}}. Allowed status values: {}.", prompt_path.display(), result_path.display(), run_id, exec, status_help);
        if !self.submit(&command).await {
            return None;
        }
        // A new task has no completion marker yet. Retain the prior epoch until
        // its submission succeeds, so human input during deferred submission wins.
        self.last_marker_epoch = None;
        let timeout = wait_timeout_ms.unwrap_or(1_800_000);
        let mut deadline = now_ms().saturating_add(timeout);
        let mut ignored_blocked: Option<marker::AutomationResultFile> = None;
        let mut ignored_protocol: Option<ResultRead> = None;
        let early;
        loop {
            if !self.active() {
                return None;
            }
            self.history.status.lock().unwrap().deadline_ms = Some(deadline);
            self.phase(AutomationPhase::Watching);
            let read = marker::read_result_file(&result_path, &run_id, &exec);
            match read.clone() {
                ResultRead::Accepted(value) => match value.status {
                    AutomationResultStatus::Done | AutomationResultStatus::Complete => {
                        self.observe_marker(&value);
                        early = value.status == AutomationResultStatus::Complete
                            && allow_early_complete.unwrap_or(false);
                        *previous_result = value.summary.unwrap_or_default();
                        break;
                    }
                    AutomationResultStatus::Blocked if ignored_blocked.as_ref() != Some(&value) => {
                        let choice = self
                            .decision(
                                AutomationDecisionReason::Blocked,
                                value.summary.clone(),
                                Some((&result_path, &exec)),
                            )
                            .await?;
                        if matches!(choice, DecisionCommand::Continue) {
                            return Some(false);
                        }
                        ignored_blocked = Some(value);
                        deadline = now_ms().saturating_add(timeout);
                        continue;
                    }
                    _ => {}
                },
                ResultRead::UnsupportedVersion(_) | ResultRead::ProtocolError(_)
                    if ignored_protocol.as_ref() != Some(&read) =>
                {
                    self.decision(
                        AutomationDecisionReason::ProtocolError,
                        None,
                        Some((&result_path, &exec)),
                    )
                    .await?;
                    ignored_protocol = Some(read);
                    deadline = now_ms().saturating_add(timeout);
                    continue;
                }
                _ => {}
            }
            if now_ms() >= deadline {
                self.decision(
                    AutomationDecisionReason::Timeout,
                    None,
                    Some((&result_path, &exec)),
                )
                .await?;
                deadline = now_ms().saturating_add(timeout);
                continue;
            }
            if !self.pause(Duration::from_millis(50)).await {
                return None;
            }
        }
        self.history.status.lock().unwrap().deadline_ms = None;
        self.phase(AutomationPhase::Settling);
        if !self
            .pause(Duration::from_millis(completion_grace_ms.unwrap_or(10_000)))
            .await
            || !self.human_checkpoint().await
        {
            return None;
        }
        Some(early)
    }
    async fn execute(&mut self) {
        if self.history.record.definition_snapshot.schema_version == 2 {
            self.execute_v2().await;
            return;
        }
        let definition = self.history.record.definition_snapshot.clone();
        let cycles = definition
            .repeat
            .as_ref()
            .map(|r| r.max_cycles)
            .unwrap_or(1);
        let mut previous_result = String::new();
        for cycle in 1..=cycles {
            for (index, step) in definition.steps.iter().enumerate() {
                if matches!(step, AutomationStep::LaunchCli { .. }) && cycle != 1 {
                    continue;
                }
                if matches!(step, AutomationStep::ExitCli { .. }) && cycle != cycles {
                    continue;
                }
                if !self.active() || !self.human_checkpoint().await {
                    return;
                }
                let id = match step {
                    AutomationStep::LaunchCli { id, .. }
                    | AutomationStep::LlmTask { id, .. }
                    | AutomationStep::Wait { id, .. }
                    | AutomationStep::Confirm { id, .. }
                    | AutomationStep::ExitCli { id, .. } => id,
                };
                {
                    let mut s = self.history.status.lock().unwrap();
                    s.cycle = Some(cycle);
                    s.step_index = Some(index);
                    s.step_id = Some(id.clone());
                    s.step_execution_id = Some(Uuid::new_v4().to_string());
                    s.marker_observed_at_ms = None;
                    s.deadline_ms = None;
                    s.decision_id = None;
                    s.decision_reason = None;
                    s.decision_message = None;
                    s.file_path.clear();
                }
                self.history.event(
                    "step",
                    Some(
                        serde_json::json!({"cycle":cycle,"stepIndex":index,"stepId":id})
                            .to_string(),
                    ),
                );
                match step {
                    AutomationStep::LaunchCli {
                        cli_profile_id,
                        model,
                        effort,
                        startup_wait_ms,
                        ..
                    } => {
                        self.phase(AutomationPhase::Launching);
                        let command = match launch_command(
                            cli_profile_id,
                            model.as_deref(),
                            effort.as_deref(),
                        ) {
                            Ok(command) => command,
                            Err(error) => {
                                fail(&self.history.status, &self.history.finished, error);
                                return;
                            }
                        };
                        if !self.submit(&command).await {
                            return;
                        }
                        self.phase(AutomationPhase::WaitingStartup);
                        if !self
                            .pause(Duration::from_millis(startup_wait_ms.unwrap_or(10_000)))
                            .await
                        {
                            return;
                        }
                    }
                    AutomationStep::Wait { duration_ms, .. } => {
                        self.phase(AutomationPhase::WaitingStartup);
                        if !self.pause(Duration::from_millis(*duration_ms)).await {
                            return;
                        }
                    }
                    AutomationStep::Confirm { message, .. } => {
                        let mut inputs = self.history.record.inputs.clone();
                        inputs.insert("previousResult".into(), previous_result.clone());
                        let message = subst(
                            message,
                            &inputs,
                            &self.history.record.workspace,
                            &self.history.record.run_id,
                            cycle,
                        );
                        if self
                            .decision(AutomationDecisionReason::Confirm, Some(message), None)
                            .await
                            .is_none()
                        {
                            return;
                        }
                    }
                    AutomationStep::ExitCli { command, .. } => {
                        if !self.submit(command).await {
                            return;
                        }
                    }
                    AutomationStep::LlmTask { .. } => {
                        let Some(early) = self.llm_task(step, cycle, &mut previous_result).await
                        else {
                            return;
                        };
                        if early {
                            if let Some(AutomationStep::ExitCli { id, command, .. }) =
                                definition.steps.last()
                            {
                                {
                                    let mut s = self.history.status.lock().unwrap();
                                    s.step_index = Some(definition.steps.len() - 1);
                                    s.step_id = Some(id.clone());
                                    s.step_execution_id = Some(Uuid::new_v4().to_string());
                                }
                                self.history.event(
                                    "step",
                                    Some(
                                        serde_json::json!({"cycle":cycle,"stepId":id}).to_string(),
                                    ),
                                );
                                if !self.submit(command).await {
                                    return;
                                }
                            }
                            self.complete();
                            return;
                        }
                    }
                }
            }
        }
        self.complete();
    }
}

impl DefinitionRunner {
    fn transition_error(&self, error: impl Into<String>) {
        fail(&self.history.status, &self.history.finished, error.into());
    }

    fn receipt(&self) -> crate::automation::cli_transition::ReceiptRead {
        match (&self.transition, &self.launch) {
            (Some(adapter), Some(launch)) => adapter.poll_receipt(launch),
            _ => crate::automation::cli_transition::ReceiptRead::Pending,
        }
    }

    fn launch_started(&self) -> crate::automation::cli_transition::StartedRead {
        match (&self.transition, &self.launch) {
            (Some(adapter), Some(launch)) => adapter.poll_started(launch),
            _ => crate::automation::cli_transition::StartedRead::Pending,
        }
    }

    // A returned synchronous call is never an active LLM input target.
    fn cli_still_active(&self) -> bool {
        use crate::automation::cli_transition::ReceiptRead;
        match self.receipt() {
            ReceiptRead::Pending => true,
            ReceiptRead::Returned { exit_code } => {
                self.history.status.lock().unwrap().cli_context = Some(AutomationCliContext::Shell);
                self.transition_error(format!("automation-cli-returned-before-task:{exit_code}"));
                false
            }
            ReceiptRead::Invalid(error) => {
                self.transition_error(error);
                false
            }
        }
    }

    async fn own_transition(&mut self) -> bool {
        if self.transition_owner.is_some() {
            return true;
        }
        loop {
            if !self.active() {
                return false;
            }
            match self.ctx.gate.try_acquire_transition_owner(
                &self.agent,
                &self.sid,
                self.generation,
            ) {
                Ok(owner) => {
                    self.transition_owner = Some(owner);
                    return true;
                }
                Err(crate::session::inject::TransitionAcquireOutcome::Deferred(reason)) => {
                    self.history.status.lock().unwrap().pending_reason = Some(reason);
                    if !self.pause(Duration::from_millis(50)).await {
                        return false;
                    }
                }
                Err(_) => {
                    self.transition_error("automation-session-changed");
                    return false;
                }
            }
        }
    }

    async fn confirm_transition(&mut self, shell: bool, message: Option<String>) -> bool {
        loop {
            let revision = self.ctx.gate.human_input_revision(&self.agent);
            self.history.status.lock().unwrap().cli_context = Some(AutomationCliContext::Unknown);
            if self
                .decision(
                    if shell {
                        AutomationDecisionReason::ShellReady
                    } else {
                        AutomationDecisionReason::CliExitUnconfirmed
                    },
                    message.clone(),
                    None,
                )
                .await
                .is_none()
            {
                return false;
            }
            if !self.active() {
                return false;
            }
            if self
                .ctx
                .gate
                .clear_uncommitted_if_revision(&self.agent, revision)
            {
                self.confirmed_revision = Some(revision);
                if shell {
                    self.history.status.lock().unwrap().cli_context =
                        Some(AutomationCliContext::Shell);
                }
                return true;
            }
            // A decision rendered before another human edit cannot authorize that edit.
        }
    }

    async fn transition_submit(&mut self, text: &str, launching: bool) -> bool {
        use crate::session::inject::{StrictOutcome, StrictSubmitRequest};
        self.last_transition_submitted = false;
        loop {
            if !self.active() {
                return false;
            }
            if !launching
                && !matches!(
                    self.receipt(),
                    crate::automation::cli_transition::ReceiptRead::Pending
                )
            {
                return true;
            }
            if self.confirmed_revision != Some(self.ctx.gate.human_input_revision(&self.agent)) {
                if !self
                    .confirm_transition(
                        launching,
                        (!launching).then(|| "automation-cli-input-before-exit".into()),
                    )
                    .await
                {
                    return false;
                }
                continue;
            }
            let owner = self.transition_owner.as_ref().unwrap();
            let outcome = self
                .ctx
                .gate
                .submit_strict(
                    owner,
                    StrictSubmitRequest {
                        text,
                        source: InjectSource::Automation,
                        expected_generation: self.generation,
                        expected_human_input_revision: self.confirmed_revision.unwrap(),
                        cancelled: &self.history.cancel,
                    },
                )
                .await;
            match outcome {
                StrictOutcome::Submitted => {
                    self.last_transition_submitted = true;
                    let mut status = self.history.status.lock().unwrap();
                    status.pending_reason = None;
                    status.pending_since_ms = None;
                    return true;
                }
                StrictOutcome::HumanInputChangedBeforeBody
                | StrictOutcome::Deferred(crate::session::inject::PendingReason::HumanTyping) => {
                    if !self
                        .confirm_transition(
                            launching,
                            (!launching).then(|| "automation-cli-input-before-exit".into()),
                        )
                        .await
                    {
                        return false;
                    }
                }
                StrictOutcome::Deferred(reason) => {
                    self.history.status.lock().unwrap().pending_reason = Some(reason);
                    if !self.pause(Duration::from_millis(25)).await {
                        return false;
                    }
                }
                StrictOutcome::CancelledBeforeCr
                | StrictOutcome::HumanInputChangedBeforeCr
                | StrictOutcome::SessionChangedBeforeCr
                | StrictOutcome::NotRunningBeforeCr
                | StrictOutcome::GenerationChangedBeforeCr => {
                    self.history.status.lock().unwrap().cli_context =
                        Some(AutomationCliContext::Unknown);
                    self.transition_error("automation-transition-partial-submission");
                    self.history.event("cli-partial-submission-no-retry", None);
                    return false;
                }
                StrictOutcome::CancelledBeforeBody => {
                    return false;
                }
                _ => {
                    self.transition_error("automation-session-changed");
                    return false;
                }
            }
        }
    }

    fn set_step(&mut self, index: usize, step: &AutomationStep, cycle: u32) {
        let id = match step {
            AutomationStep::LaunchCli { id, .. }
            | AutomationStep::ExitCli { id, .. }
            | AutomationStep::LlmTask { id, .. }
            | AutomationStep::Wait { id, .. }
            | AutomationStep::Confirm { id, .. } => id,
        };
        {
            let mut s = self.history.status.lock().unwrap();
            s.cycle = Some(cycle);
            s.step_index = Some(index);
            s.step_id = Some(id.clone());
            s.step_execution_id = Some(Uuid::new_v4().to_string());
            s.marker_observed_at_ms = None;
            s.cli_return_observed_at_ms = None;
            s.deadline_ms = None;
            s.decision_id = None;
            s.decision_reason = None;
            s.decision_message = None;
            s.file_path.clear();
        }
        self.history.event(
            "step",
            Some(serde_json::json!({"cycle":cycle,"stepIndex":index,"stepId":id}).to_string()),
        );
    }

    fn cli_event(
        &mut self,
        kind: &str,
        segment: &crate::automation::definition::CliSegment,
        early: bool,
        exit_code: Option<i32>,
    ) {
        let status = self.history.status.lock().unwrap().clone();
        self.history.event(kind, Some(serde_json::json!({
            "cli":status.cli,"cycle":status.cycle,"launchStepId":segment.launch_step_id,
            "exitStepId":segment.exit_step_id,"launchId":self.launch.as_ref().map(|l| &l.launch_id),
            "reason":if early {"earlyComplete"} else {"normal"},"exitCode":exit_code,
        }).to_string()));
    }

    async fn launch_v2(
        &mut self,
        step: &AutomationStep,
        segment: &crate::automation::definition::CliSegment,
    ) -> bool {
        let AutomationStep::LaunchCli {
            id,
            cli_profile_id,
            model,
            effort,
            startup_wait_ms,
        } = step
        else {
            unreachable!()
        };
        if !self.own_transition().await {
            return false;
        }
        if self.confirmed_revision != Some(self.ctx.gate.human_input_revision(&self.agent))
            && !self.confirm_transition(true, None).await
        {
            return false;
        }
        let command = match launch_command(cli_profile_id, model.as_deref(), effort.as_deref()) {
            Ok(command) => command,
            Err(error) => {
                self.transition_error(error);
                return false;
            }
        };
        let exec = self
            .history
            .status
            .lock()
            .unwrap()
            .step_execution_id
            .clone()
            .unwrap();
        let prepared = match self.transition.as_ref().unwrap().prepare_posix_launch(
            &self.sid,
            &self.history.record.run_id,
            id,
            &exec,
            &command,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.transition_error(error);
                return false;
            }
        };
        let rendered = prepared.rendered_command.clone();
        self.launch = Some(prepared);
        {
            let mut status = self.history.status.lock().unwrap();
            status.cli = match cli_profile_id.as_str() {
                "codex" => AutomationCli::Codex,
                "agy" => AutomationCli::Agy,
                "kilo" => AutomationCli::Kilo,
                "pi" => AutomationCli::Pi,
                _ => AutomationCli::Claude,
            };
        }
        self.phase(AutomationPhase::Launching);
        if !self.transition_submit(&rendered, true).await {
            return false;
        }
        self.history.status.lock().unwrap().cli_context = Some(AutomationCliContext::Unknown);
        // A submitted CR is not evidence that canonical-mode input survived.
        // Do not enter CLI context or start its readiness timer before the
        // sourced app-owned script atomically acknowledges execution.
        let ack_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            if !self.active() {
                return false;
            }
            match self.launch_started() {
                crate::automation::cli_transition::StartedRead::Started => break,
                crate::automation::cli_transition::StartedRead::Invalid(error) => {
                    self.transition_error(error);
                    return false;
                }
                crate::automation::cli_transition::StartedRead::Pending => {}
            }
            if tokio::time::Instant::now() >= ack_deadline {
                self.transition_error("automation-cli-launch-ack-timeout");
                return false;
            }
            if !self.pause(Duration::from_millis(25)).await {
                return false;
            }
        }
        self.history.status.lock().unwrap().cli_context = Some(AutomationCliContext::Cli {
            cli: self
                .history
                .record
                .definition_snapshot
                .steps
                .get(segment.launch_step_index)
                .and_then(|s| {
                    if let AutomationStep::LaunchCli { cli_profile_id, .. } = s {
                        Some(match cli_profile_id.as_str() {
                            "codex" => AutomationCli::Codex,
                            "agy" => AutomationCli::Agy,
                            "kilo" => AutomationCli::Kilo,
                            "pi" => AutomationCli::Pi,
                            _ => AutomationCli::Claude,
                        })
                    } else {
                        None
                    }
                })
                .unwrap(),
            launch_step_id: id.clone(),
        });
        self.cli_event("cli-launch", segment, false, None);
        let deadline =
            tokio::time::Instant::now() + Duration::from_millis(startup_wait_ms.unwrap_or(10_000));
        loop {
            if !self.active() || !self.cli_still_active() {
                return false;
            }
            if self.confirmed_revision != Some(self.ctx.gate.human_input_revision(&self.agent)) {
                // CR already succeeded: confirming the current CLI never resends Launch.
                if !self
                    .confirm_transition(false, Some("automation-cli-input-after-launch".into()))
                    .await
                {
                    return false;
                }
                let cli = self.history.status.lock().unwrap().cli;
                self.history.status.lock().unwrap().cli_context = Some(AutomationCliContext::Cli {
                    cli,
                    launch_step_id: id.clone(),
                });
            }
            self.phase(AutomationPhase::WaitingStartup);
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            if !self.pause(Duration::from_millis(25)).await {
                return false;
            }
        }
        self.transition_owner = None;
        self.confirmed_revision = None;
        true
    }

    async fn exit_v2(
        &mut self,
        step: &AutomationStep,
        segment: &crate::automation::definition::CliSegment,
        early: bool,
    ) -> bool {
        use crate::automation::cli_transition::ReceiptRead;
        let AutomationStep::ExitCli {
            command,
            return_mode,
            exit_wait_ms,
            ..
        } = step
        else {
            unreachable!()
        };
        if !self.own_transition().await || !self.human_checkpoint().await {
            return false;
        }
        self.confirmed_revision = Some(self.ctx.gate.human_input_revision(&self.agent));
        self.phase(AutomationPhase::Exiting);
        match self.receipt() {
            ReceiptRead::Pending => {
                if !self.transition_submit(command, false).await {
                    return false;
                }
                self.cli_event(
                    if self.last_transition_submitted {
                        "cli-exit-submitted"
                    } else {
                        "cli-already-returned"
                    },
                    segment,
                    early,
                    None,
                );
            }
            _ => {
                self.cli_event("cli-already-returned", segment, early, None);
            }
        }
        let manual = *return_mode == Some(AutomationCliReturnMode::Manual);
        if manual {
            if !self
                .confirm_transition(false, Some("automation-cli-manual-return".into()))
                .await
            {
                return false;
            }
            self.cli_event("cli-return-manual", segment, early, None);
        } else {
            let timeout = exit_wait_ms.unwrap_or(30_000);
            let mut deadline = now_ms().saturating_add(timeout);
            loop {
                if !self.active() {
                    return false;
                }
                self.phase(AutomationPhase::Exiting);
                self.history.status.lock().unwrap().deadline_ms = Some(deadline);
                match self.receipt() {
                    ReceiptRead::Returned { exit_code: 0 } => {
                        self.history
                            .status
                            .lock()
                            .unwrap()
                            .cli_return_observed_at_ms = Some(now_ms());
                        if self.confirmed_revision
                            != Some(self.ctx.gate.human_input_revision(&self.agent))
                            && !self.confirm_transition(true, None).await
                        {
                            return false;
                        }
                        self.cli_event("cli-returned", segment, early, Some(0));
                        break;
                    }
                    ReceiptRead::Returned { exit_code } => {
                        if !self
                            .confirm_transition(
                                false,
                                Some(format!("automation-cli-return-code:{exit_code}")),
                            )
                            .await
                        {
                            return false;
                        }
                        self.cli_event("cli-return-manual", segment, early, Some(exit_code));
                        break;
                    }
                    ReceiptRead::Invalid(error) => {
                        if !self.confirm_transition(false, Some(error)).await {
                            return false;
                        }
                        self.cli_event("cli-return-manual", segment, early, None);
                        break;
                    }
                    ReceiptRead::Pending => {}
                }
                if now_ms() >= deadline {
                    if self
                        .decision(AutomationDecisionReason::CliExitTimeout, None, None)
                        .await
                        .is_none()
                    {
                        return false;
                    }
                    deadline = now_ms().saturating_add(timeout);
                }
                if !self.pause(Duration::from_millis(25)).await {
                    return false;
                }
            }
        }
        self.launch = None; // consume this launch's receipt exactly once
        let mut status = self.history.status.lock().unwrap();
        status.cli_context = Some(AutomationCliContext::Shell);
        status.deadline_ms = None;
        true
    }

    async fn execute_v2(&mut self) {
        use crate::automation::definition::{compile, DefinitionPlan};
        let definition = self.history.record.definition_snapshot.clone();
        let segments = match compile(&definition) {
            Ok(DefinitionPlan::V2 { segments }) => segments,
            _ => {
                self.transition_error("automation-v2-plan-invalid");
                return;
            }
        };
        let shell = self.ctx.sink.shell_path_for(&self.agent);
        let capability =
            crate::automation::cli_transition::CliTransition::capability_for_shell_path(
                shell.as_deref(),
            );
        if !capability.auto_return_supported {
            self.transition_error(
                capability
                    .unavailable_reason
                    .unwrap_or_else(|| "automation-cli-shell-unsupported".into()),
            );
            return;
        }
        self.generation = self.ctx.gate.begin_generation(&self.agent);
        self.transition = Some(crate::automation::cli_transition::CliTransition::new(
            self.history.store.app_data_dir().to_path_buf(),
        ));
        let cycles = definition
            .repeat
            .as_ref()
            .map(|r| r.max_cycles)
            .unwrap_or(1);
        let mut previous_result = String::new();
        for cycle in 1..=cycles {
            for (index, step) in definition.steps.iter().enumerate() {
                if !self.active() || !self.human_checkpoint().await {
                    return;
                }
                self.set_step(index, step, cycle);
                match step {
                    AutomationStep::LaunchCli { .. } => {
                        let segment = segments
                            .iter()
                            .find(|s| s.launch_step_index == index)
                            .unwrap();
                        if !self.launch_v2(step, segment).await {
                            return;
                        }
                    }
                    AutomationStep::ExitCli { .. } => {
                        let segment = segments
                            .iter()
                            .find(|s| s.exit_step_index == index)
                            .unwrap();
                        if !self.exit_v2(step, segment, false).await {
                            return;
                        }
                        if cycle == cycles && index == segments.last().unwrap().exit_step_index {
                            self.transition_owner = None;
                        }
                    }
                    AutomationStep::Wait { duration_ms, .. } => {
                        self.phase(AutomationPhase::WaitingStartup);
                        if !self.pause(Duration::from_millis(*duration_ms)).await {
                            return;
                        }
                    }
                    AutomationStep::Confirm { message, .. } => {
                        let mut inputs = self.history.record.inputs.clone();
                        inputs.insert("previousResult".into(), previous_result.clone());
                        let message = subst(
                            message,
                            &inputs,
                            &self.history.record.workspace,
                            &self.history.record.run_id,
                            cycle,
                        );
                        if self
                            .decision(AutomationDecisionReason::Confirm, Some(message), None)
                            .await
                            .is_none()
                        {
                            return;
                        }
                    }
                    AutomationStep::LlmTask { .. } => {
                        if !self.cli_still_active() {
                            return;
                        }
                        let Some(early) = self.llm_task(step, cycle, &mut previous_result).await
                        else {
                            return;
                        };
                        if early {
                            let segment = segments
                                .iter()
                                .find(|s| s.launch_step_index < index && index < s.exit_step_index)
                                .unwrap();
                            self.cli_event("early-complete", segment, true, None);
                            let exit = &definition.steps[segment.exit_step_index];
                            self.set_step(segment.exit_step_index, exit, cycle);
                            if !self.exit_v2(exit, segment, true).await {
                                return;
                            }
                            self.transition_owner = None;
                            self.complete();
                            return;
                        }
                    }
                }
            }
        }
        self.transition_owner = None;
        self.complete();
    }
}
pub async fn run(
    ctx: Arc<AutomationContext>,
    agent: String,
    session_id: String,
    definition: AutomationDefinition,
    inputs: std::collections::BTreeMap<String, String>,
    workspace: String,
    status: Arc<Mutex<AutomationAgentStatus>>,
    shutdown: oneshot::Receiver<()>,
    decisions: mpsc::UnboundedReceiver<DecisionCommand>,
    cancel: Arc<AtomicBool>,
    finished: Arc<Mutex<Option<Instant>>>,
    store: Arc<AutomationStore>,
    run_id: String,
) {
    let started_at = status.lock().unwrap().started_at_ms;
    let history = RunHistoryGuard {
        store,
        record: AutomationRunRecord {
            run_id,
            definition_snapshot: definition,
            inputs,
            workspace,
            agent_id: agent.clone(),
            status: "running".into(),
            outcome: None,
            events: vec![AutomationRunEvent {
                at: started_at,
                kind: "started".into(),
                details: None,
            }],
        },
        status,
        finished,
        cancel,
    };
    DefinitionRunner {
        ctx,
        agent,
        sid: session_id,
        shutdown,
        decisions,
        history,
        last_marker_epoch: None,
        transition: None,
        launch: None,
        transition_owner: None,
        confirmed_revision: None,
        generation: 0,
        last_transition_submitted: false,
    }
    .execute()
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn template_values_are_not_re_evaluated() {
        let mut inputs = std::collections::BTreeMap::new();
        inputs.insert("a".to_string(), "{{b}}".to_string());
        inputs.insert("b".to_string(), "changed".to_string());
        assert_eq!(
            subst("{{a}} {{workspace}}", &inputs, "/repo", "r", 1),
            "{{b}} /repo"
        );
    }
    #[test]
    fn launch_commands_preserve_model_and_effort() {
        assert_eq!(
            launch_command("claude", Some("opus"), Some("high")).unwrap(),
            "claude --model 'opus' --effort 'high'"
        );
        assert_eq!(
            launch_command("codex", Some("gpt-5"), Some("xhigh")).unwrap(),
            "codex --model 'gpt-5' -c 'model_reasoning_effort=\"xhigh\"'"
        );
        assert_eq!(
            launch_command("agy", Some("gemini-3.1-pro-low"), Some("high")).unwrap(),
            "agy --dangerously-skip-permissions --model 'gemini-3.1-pro-low' --effort 'high'"
        );
        assert_eq!(
            launch_command("agy", Some("gemini-3.8-flash-medium"), Some("medium")).unwrap(),
            "agy --dangerously-skip-permissions --model 'gemini-3.8-flash-medium' --effort 'medium'"
        );
        assert_eq!(
            launch_command("kilo", Some("openai/gpt-5"), None).unwrap(),
            "kilo --auto --model 'openai/gpt-5'"
        );
        assert_eq!(
            launch_command("pi", Some("openai/gpt-5"), Some("xhigh")).unwrap(),
            "pi --approve --model 'openai/gpt-5' --thinking 'xhigh'"
        );
        assert_eq!(
            launch_command("agy", None, Some("xhigh")),
            Err("automation-agy-effort-invalid".into())
        );
        assert!(launch_command("claude", None, Some("unsafe")).is_err());
    }
}
