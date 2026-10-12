//! 업무 보내기: CLI 기동/ACK/10초 대기/첫 프롬프트만 수행한다.
//! 결과 파일 감시·CLI 종료는 없다. 세션과 사람 revision을 모든 입력 전에 고정한다.
use crate::automation::cli_transition::{CliTransition, ReceiptRead, StartedRead};
use crate::session::inject::{
    InjectGate, InjectSource, InputSink, StrictOutcome, StrictSubmitRequest,
};
use crate::types::{WorkAgent, WorkCli, WorkSendRequest, WorkSendResult, WorkSendTarget};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

#[derive(Default)]
pub struct WorkSendRuntime {
    active: Mutex<HashMap<String, Arc<AtomicBool>>>,
    attempted: Mutex<HashSet<String>>,
}
struct SendGuard<'a> {
    runtime: &'a WorkSendRuntime,
    agent_id: String,
    cancel: Arc<AtomicBool>,
}
impl Drop for SendGuard<'_> {
    fn drop(&mut self) {
        let mut active = self.runtime.active.lock().unwrap();
        if active
            .get(&self.agent_id)
            .is_some_and(|c| Arc::ptr_eq(c, &self.cancel))
        {
            active.remove(&self.agent_id);
        }
    }
}
impl WorkSendRuntime {
    pub fn stop(&self, agent_id: &str) {
        if let Some(cancel) = self.active.lock().unwrap().remove(agent_id) {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    pub fn stop_all(&self) {
        for (_, cancel) in self.active.lock().unwrap().drain() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    pub fn inspect(&self, gate: &InjectGate, agent_id: &str) -> Result<WorkSendTarget, String> {
        if self.active.lock().unwrap().contains_key(agent_id) {
            return Err("work-send-busy".into());
        }
        let sink = gate.sink();
        let revision = gate.human_input_revision(agent_id);
        let has_uncommitted = gate.has_uncommitted(agent_id);
        let sid = sink
            .session_id_for(agent_id)
            .filter(|_| sink.is_running(agent_id));
        if sid.is_some() {
            require_supported_shell(sink.as_ref(), agent_id)?;
        }
        let restart_required = sid.as_ref().is_some_and(|sid| {
            self.attempted.lock().unwrap().contains(sid)
                || has_uncommitted
                || sink.is_shell_prompt(agent_id) != Some(true)
        });
        Ok(WorkSendTarget {
            session_id: sid,
            human_input_revision: revision,
            restart_required,
            has_uncommitted,
        })
    }
    pub async fn send(
        &self,
        gate: &InjectGate,
        root: PathBuf,
        request: WorkSendRequest,
    ) -> Result<WorkSendResult, String> {
        self.send_with_wait(gate, root, request, Duration::from_secs(10))
            .await
    }
    async fn send_with_wait(
        &self,
        gate: &InjectGate,
        root: PathBuf,
        request: WorkSendRequest,
        startup: Duration,
    ) -> Result<WorkSendResult, String> {
        let command = validated_command(&request.agent)?;
        let prompt = work_prompt(&request.title, &request.body)?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut active = self.active.lock().unwrap();
            if active.contains_key(&request.agent_id) {
                return Err("work-send-busy".into());
            }
            active.insert(request.agent_id.clone(), cancel.clone());
        }
        let _guard = SendGuard {
            runtime: self,
            agent_id: request.agent_id.clone(),
            cancel: cancel.clone(),
        };
        let sink = gate.sink();
        let generation = gate.generation(&request.agent_id);
        let check = || -> Result<(), String> {
            if cancel.load(Ordering::Relaxed) {
                return Err("work-send-cancelled".into());
            }
            if !sink.is_running(&request.agent_id)
                || sink.session_id_for(&request.agent_id).as_deref() != Some(&request.session_id)
                || gate.generation(&request.agent_id) != generation
            {
                return Err("work-send-session-changed".into());
            }
            if gate.human_input_revision(&request.agent_id) != request.human_input_revision
                || gate.has_uncommitted(&request.agent_id)
            {
                return Err("work-send-human-input".into());
            }
            Ok(())
        };
        check()?;
        if request.fresh_session && request.human_input_revision != 0 {
            return Err("work-send-human-input".into());
        }
        require_supported_shell(sink.as_ref(), &request.agent_id)?;
        if self.attempted.lock().unwrap().contains(&request.session_id) {
            return Err("work-send-restart-required".into());
        }
        // 조회 불가인 새 브로커 셸도 ACK 없이는 프롬프트를 절대 보내지 않는다.
        let shell_prompt = sink.is_shell_prompt(&request.agent_id);
        if shell_prompt != Some(true)
            && !(shell_prompt.is_none()
                && request.fresh_session
                && request.human_input_revision == 0)
        {
            return Err("work-send-restart-required".into());
        }
        let owner = gate
            .try_acquire_transition_owner(&request.agent_id, &request.session_id, generation)
            .map_err(|_| "work-send-busy".to_string())?;
        check()?;
        // agy는 실제 설치 CLI의 정확한 모델 ID만 허용한다(기존 자동화와 동일).
        if let WorkCli::Agy = request.agent.cli {
            if let Some(model) = &request.agent.model {
                let models = crate::summarizer::list_automation_agy_models().await;
                if models.is_empty() {
                    return Err("automation-agy-model-catalog-unavailable".into());
                }
                if !models.contains(model) {
                    return Err(format!("automation-agy-model-unavailable:{model}"));
                }
            }
        }
        check()?;
        let transition = CliTransition::new(root);
        let run_id = uuid::Uuid::new_v4().to_string();
        let launch = transition.prepare_posix_launch(
            &request.session_id,
            &run_id,
            "work",
            &run_id,
            &command,
        )?;
        // 실패한 부분 제출/ACK 시간초과도 기동을 자동 재전송하지 않는다.
        self.attempted
            .lock()
            .unwrap()
            .insert(request.session_id.clone());
        strict_result(
            gate.submit_strict(
                &owner,
                StrictSubmitRequest {
                    text: launch.command(),
                    source: InjectSource::Automation,
                    expected_generation: generation,
                    expected_human_input_revision: request.human_input_revision,
                    cancelled: &cancel,
                },
            )
            .await,
        )?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            check()?;
            match transition.poll_started(&launch) {
                StartedRead::Started => break,
                StartedRead::Invalid(error) => return Err(error),
                StartedRead::Pending if tokio::time::Instant::now() >= deadline => {
                    return Err("work-send-launch-timeout".into())
                }
                StartedRead::Pending => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
        let ready = tokio::time::Instant::now() + startup;
        loop {
            check()?;
            match transition.poll_receipt(&launch) {
                ReceiptRead::Returned { .. } => return Err("work-send-cli-returned".into()),
                ReceiptRead::Invalid(error) => return Err(error),
                ReceiptRead::Pending => (),
            }
            if tokio::time::Instant::now() >= ready {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let submitted_at = crate::types::now_ms();
        strict_result(
            gate.submit_strict_paste(
                &owner,
                StrictSubmitRequest {
                    text: &prompt,
                    source: InjectSource::Automation,
                    expected_generation: generation,
                    expected_human_input_revision: request.human_input_revision,
                    cancelled: &cancel,
                },
            )
            .await,
        )?;
        Ok(WorkSendResult {
            session_id: request.session_id,
            submitted_at,
        })
    }
}
fn require_supported_shell(sink: &dyn InputSink, agent_id: &str) -> Result<(), String> {
    let shell = sink.shell_path_for(agent_id);
    if !CliTransition::capability_for_shell_path(shell.as_deref()).auto_return_supported {
        return Err("work-send-shell-unsupported".into());
    }
    Ok(())
}
pub fn validated_command(agent: &WorkAgent) -> Result<String, String> {
    if agent
        .model
        .as_ref()
        .is_some_and(|m| m.is_empty() || m.chars().any(char::is_control))
    {
        return Err("work-send-model-invalid".into());
    }
    let cli = match agent.cli {
        WorkCli::Claude => "claude",
        WorkCli::Codex => "codex",
        WorkCli::Agy => "agy",
        WorkCli::Kilo => "kilo",
        WorkCli::Pi => "pi",
    };
    crate::automation::definition_runner::launch_command(
        cli,
        agent.model.as_deref(),
        agent.effort.as_deref(),
    )
}
fn work_prompt(title: &str, body: &str) -> Result<String, String> {
    if (title.trim().is_empty() && body.trim().is_empty()) || title.contains(['\r', '\n']) {
        return Err("work-send-prompt-invalid".into());
    }
    let prompt = if title.trim().is_empty() {
        body.replace("\r\n", "\n")
    } else if body.trim().is_empty() {
        title.trim().into()
    } else {
        format!("{}\n\n{}", title.trim(), body.replace("\r\n", "\n"))
    };
    if prompt.len() > 256 * 1024
        || prompt
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    {
        return Err("work-send-prompt-invalid".into());
    }
    Ok(prompt)
}
fn strict_result(result: StrictOutcome) -> Result<(), String> {
    use StrictOutcome::*;
    match result {
        Submitted => Ok(()),
        Deferred(_) => Err("work-send-human-input".into()),
        HumanInputChangedBeforeBody => Err("work-send-human-input".into()),
        HumanInputChangedBeforeCr
        | CancelledBeforeCr
        | SessionChangedBeforeCr
        | NotRunningBeforeCr
        | GenerationChangedBeforeCr => Err("work-send-partial-submission".into()),
        CancelledBeforeBody => Err("work-send-cancelled".into()),
        _ => Err("work-send-session-changed".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        session::inject::{PendingReason, RecordingSink, SubmitOutcome},
        state::BotPromptArms,
    };
    fn request() -> WorkSendRequest {
        WorkSendRequest {
            agent_id: "a".into(),
            session_id: "s".into(),
            human_input_revision: 0,
            fresh_session: true,
            agent: WorkAgent {
                cli: WorkCli::Claude,
                model: None,
                effort: None,
            },
            title: "Implement task".into(),
            body: "Line one\n\n```text\nLine two\n```".into(),
        }
    }
    fn recording() -> Arc<RecordingSink> {
        let sink = Arc::new(RecordingSink::default());
        sink.set_session("a", "s");
        sink.set_running("a", true);
        sink.set_shell_path("a", "/bin/bash");
        sink
    }
    #[test]
    fn validation_preserves_multiline_and_rejects_terminal_control_sequences() {
        assert_eq!(
            work_prompt(" Task ", "first\r\nsecond").unwrap(),
            "Task\n\nfirst\nsecond"
        );
        assert_eq!(
            work_prompt(" ", "first\r\n\r\nsecond").unwrap(),
            "first\n\nsecond"
        );
        assert!(work_prompt(" ", "\n\t ").is_err());
        assert!(work_prompt("", "\x1b[201~echo injected").is_err());
        assert!(work_prompt("", &"a".repeat(256 * 1024 + 1)).is_err());
        for text in ["\x1b[201~echo injected", "a\rb", "\0"] {
            assert!(work_prompt("Task", text).is_err());
        }
        assert!(work_prompt("a\nb", "").is_err());
        assert!(work_prompt("Task", &"a".repeat(256 * 1024)).is_err());
        let mut agent = request().agent;
        agent.model = Some("x\nshutdown".into());
        assert!(validated_command(&agent).is_err());
        agent.model = Some("$(touch injected)'".into());
        assert_eq!(
            validated_command(&agent).unwrap(),
            "claude --model '$(touch injected)'\\'''"
        );
    }
    #[test]
    fn unknown_foreground_or_unsubmitted_input_requires_restart() {
        let sink = recording();
        let gate = InjectGate::new(sink.clone(), Arc::new(BotPromptArms::default()));
        let runtime = WorkSendRuntime::default();
        assert!(runtime.inspect(&gate, "a").unwrap().restart_required);
        gate.note_human("a", "draft");
        assert!(runtime.inspect(&gate, "a").unwrap().has_uncommitted);
        sink.set_shell_path("a", "/bin/fish");
        assert_eq!(
            runtime.inspect(&gate, "a").unwrap_err(),
            "work-send-shell-unsupported"
        );
        sink.set_running("a", false);
        assert_eq!(runtime.inspect(&gate, "a").unwrap().session_id, None);
    }
    #[tokio::test]
    async fn stale_session_and_old_human_fragment_produce_no_input() {
        let root = tempfile::tempdir().unwrap();
        let sink = recording();
        let gate = InjectGate::with_hold_max(sink.clone(), Arc::new(BotPromptArms::default()), 0);
        let runtime = WorkSendRuntime::default();
        sink.set_session("a", "replacement");
        assert_eq!(
            runtime
                .send_with_wait(&gate, root.path().into(), request(), Duration::ZERO)
                .await
                .unwrap_err(),
            "work-send-session-changed"
        );
        sink.set_session("a", "s");
        gate.note_human("a", "draft");
        let mut req = request();
        req.human_input_revision = gate.human_input_revision("a");
        assert_eq!(
            runtime
                .send_with_wait(&gate, root.path().into(), req, Duration::ZERO)
                .await
                .unwrap_err(),
            "work-send-human-input"
        );
        assert_eq!(
            sink.recorded_writes()
                .iter()
                .map(|(_, data, _)| data.as_str())
                .collect::<Vec<_>>(),
            vec!["draft"]
        );
    }
    #[tokio::test(start_paused = true)]
    async fn missing_launch_ack_never_submits_the_prompt_or_retries_launch() {
        let root = tempfile::tempdir().unwrap();
        let sink = recording();
        let gate = InjectGate::new(sink.clone(), Arc::new(BotPromptArms::default()));
        let runtime = WorkSendRuntime::default();
        assert_eq!(
            runtime
                .send_with_wait(&gate, root.path().into(), request(), Duration::ZERO)
                .await
                .unwrap_err(),
            "work-send-launch-timeout"
        );
        assert_eq!(sink.recorded_writes().len(), 2);
        assert_eq!(
            runtime
                .send_with_wait(&gate, root.path().into(), request(), Duration::ZERO)
                .await
                .unwrap_err(),
            "work-send-restart-required"
        );
        assert_eq!(sink.recorded_writes().len(), 2);
    }
    // Execute only the generated launch script in a real shell with a CLI function double.
    // This verifies ACK/return receipts without launching an installed/authenticated CLI.
    #[cfg(unix)]
    struct ExecutingSink {
        recorded: Arc<RecordingSink>,
        launch: Mutex<Option<String>>,
        children: Mutex<Vec<std::process::Child>>,
        exit_immediately: bool,
        known_busy: bool,
    }
    #[cfg(unix)]
    impl ExecutingSink {
        fn new(exit_immediately: bool) -> Arc<Self> {
            Arc::new(Self {
                recorded: recording(),
                launch: Mutex::new(None),
                children: Mutex::new(vec![]),
                exit_immediately,
                known_busy: false,
            })
        }
    }
    #[cfg(unix)]
    impl Drop for ExecutingSink {
        fn drop(&mut self) {
            for child in self.children.get_mut().unwrap() {
                let _ = child.wait();
            }
        }
    }
    #[cfg(unix)]
    impl InputSink for ExecutingSink {
        fn write(&self, agent: &str, data: &str) {
            self.recorded.write(agent, data);
            if data.starts_with(". '") {
                *self.launch.lock().unwrap() = Some(data.into());
            }
            if data == "\r" {
                if let Some(command) = self.launch.lock().unwrap().take() {
                    let body = if self.exit_immediately {
                        "return 127"
                    } else {
                        "sleep 1"
                    };
                    let child = std::process::Command::new("/bin/bash")
                        .arg("-c")
                        .arg(format!("claude() {{ {body}; }}; {command}"))
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                        .unwrap();
                    self.children.lock().unwrap().push(child);
                }
            }
        }
        fn session_id_for(&self, agent: &str) -> Option<String> {
            self.recorded.session_id_for(agent)
        }
        fn is_running(&self, agent: &str) -> bool {
            self.recorded.is_running(agent)
        }
        fn shell_path_for(&self, agent: &str) -> Option<String> {
            self.recorded.shell_path_for(agent)
        }
        fn is_shell_prompt(&self, _: &str) -> Option<bool> {
            self.known_busy.then_some(false)
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn fresh_session_cannot_override_a_known_busy_terminal() {
        let root = tempfile::tempdir().unwrap();
        let mut sink = ExecutingSink::new(false);
        Arc::get_mut(&mut sink).unwrap().known_busy = true;
        let gate = InjectGate::new(sink.clone(), Arc::new(BotPromptArms::default()));
        let runtime = WorkSendRuntime::default();
        assert_eq!(
            runtime
                .send_with_wait(&gate, root.path().into(), request(), Duration::ZERO)
                .await
                .unwrap_err(),
            "work-send-restart-required"
        );
        assert!(sink.recorded.recorded_writes().is_empty());
    }
    #[cfg(unix)]
    async fn wait_for_launch(sink: &ExecutingSink) {
        for _ in 0..100 {
            if sink.recorded.recorded_writes().len() >= 2 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("launch not submitted");
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn confirmed_launch_submits_multiline_prompt_once_and_keeps_cli_running() {
        let root = tempfile::tempdir().unwrap();
        let sink = ExecutingSink::new(false);
        let gate = InjectGate::new(sink.clone(), Arc::new(BotPromptArms::default()));
        let runtime = WorkSendRuntime::default();
        let result = runtime
            .send_with_wait(&gate, root.path().into(), request(), Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(result.session_id, "s");
        let writes = sink.recorded.recorded_writes();
        assert_eq!(writes.len(), 4);
        assert_eq!(
            writes[2].1,
            format!(
                "\x1b[200~{}\x1b[201~",
                work_prompt(&request().title, &request().body).unwrap()
            )
        );
        assert_eq!(writes[3].1, "\r");
        assert!(!writes.iter().any(|(_, data, _)| data == "/exit"));
        assert!(runtime.inspect(&gate, "a").unwrap().restart_required);
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn cli_return_before_readiness_never_sends_a_prompt_to_shell() {
        let root = tempfile::tempdir().unwrap();
        let sink = ExecutingSink::new(true);
        let gate = InjectGate::new(sink.clone(), Arc::new(BotPromptArms::default()));
        let runtime = WorkSendRuntime::default();
        assert_eq!(
            runtime
                .send_with_wait(
                    &gate,
                    root.path().into(),
                    request(),
                    Duration::from_millis(200)
                )
                .await
                .unwrap_err(),
            "work-send-cli-returned"
        );
        assert_eq!(sink.recorded.recorded_writes().len(), 2);
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn duplicate_send_is_rejected_and_other_producers_wait_until_cancelled() {
        let root = tempfile::tempdir().unwrap();
        let sink = ExecutingSink::new(false);
        let gate = InjectGate::new(sink.clone(), Arc::new(BotPromptArms::default()));
        let runtime = WorkSendRuntime::default();
        let send = runtime.send_with_wait(
            &gate,
            root.path().into(),
            request(),
            Duration::from_secs(10),
        );
        let intervene = async {
            wait_for_launch(&sink).await;
            assert_eq!(
                runtime
                    .send_with_wait(&gate, root.path().into(), request(), Duration::ZERO)
                    .await
                    .unwrap_err(),
                "work-send-busy"
            );
            assert_eq!(
                gate.submit("a", "s", "bot", InjectSource::Bot).await,
                SubmitOutcome::Deferred(PendingReason::AnotherProducer)
            );
            runtime.stop("a");
        };
        let (result, _) = tokio::join!(send, intervene);
        assert_eq!(result.unwrap_err(), "work-send-cancelled");
        assert_eq!(sink.recorded.recorded_writes().len(), 2);
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn input_or_session_change_during_readiness_prevents_first_prompt() {
        for swap_session in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let sink = ExecutingSink::new(false);
            let gate = InjectGate::new(sink.clone(), Arc::new(BotPromptArms::default()));
            let runtime = WorkSendRuntime::default();
            let send = runtime.send_with_wait(
                &gate,
                root.path().into(),
                request(),
                Duration::from_secs(10),
            );
            let intervene = async {
                wait_for_launch(&sink).await;
                if swap_session {
                    sink.recorded.set_session("a", "new");
                } else {
                    gate.note_human("a", "human");
                }
            };
            let (result, _) = tokio::join!(send, intervene);
            assert_eq!(
                result.unwrap_err(),
                if swap_session {
                    "work-send-session-changed"
                } else {
                    "work-send-human-input"
                }
            );
            assert_eq!(
                sink.recorded
                    .recorded_writes()
                    .iter()
                    .filter(|(_, text, _)| text.starts_with("\x1b[200~"))
                    .count(),
                0
            );
        }
    }
}

#[cfg(all(test, unix))]
mod pty_smoke;
