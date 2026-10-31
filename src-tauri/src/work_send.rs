//! 업무 보내기: CLI 기동/ACK/10초 대기/첫 프롬프트만 수행한다.
//! 결과 파일 감시·CLI 종료는 없다. 세션과 사람 revision을 모든 입력 전에 고정한다.
pub(crate) mod cli_transition;
mod launch;

use cli_transition::{CliTransition, CommandSyntax, LaunchShell, ReceiptRead, StartedRead};
use crate::session::inject::{
    InjectGate, InjectSource, InputSink, StrictOutcome, StrictSubmitRequest,
};
use crate::types::{WorkAgent, WorkCli, WorkFollowUpRequest, WorkSendRequest, WorkSendResult, WorkSendTarget};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

/// 새 세션 셸이 시작 스크립트를 마치고 빈 프롬프트가 되기를 기다리는 최대 시간.
const FRESH_SHELL_SETTLE: Duration = Duration::from_secs(10);

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
    /// 자동 진행의 후속 지시: 이미 돌고 있는 CLI에 프롬프트 하나를 넣는다.
    /// 기동은 하지 않고, 보내기와 같은 캐릭터별
    /// 상호배제·전환 소유권·사람 입력 검사를 거친다.
    pub async fn follow_up(
        &self,
        gate: &InjectGate,
        request: WorkFollowUpRequest,
    ) -> Result<WorkSendResult, String> {
        let prompt = work_prompt("", &request.text)?;
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
        if !sink.is_running(&request.agent_id)
            || sink.session_id_for(&request.agent_id).as_deref() != Some(&request.session_id)
        {
            return Err("work-send-session-changed".into());
        }
        // 셸 프롬프트로 돌아왔다면 CLI가 끝난 것이다. 셸에 프롬프트를 실행시키지 않는다.
        if sink.is_shell_prompt(&request.agent_id) == Some(true) {
            return Err("work-send-cli-returned".into());
        }
        if gate.has_uncommitted(&request.agent_id) {
            return Err("work-send-human-input".into());
        }
        let generation = gate.generation(&request.agent_id);
        let revision = gate.human_input_revision(&request.agent_id);
        let owner = gate
            .try_acquire_transition_owner(&request.agent_id, &request.session_id, generation)
            .map_err(|_| "work-send-busy".to_string())?;
        let submitted_at = crate::types::now_ms();
        strict_result(
            gate.submit_strict_paste(
                &owner,
                StrictSubmitRequest {
                    text: &prompt,
                    source: InjectSource::WorkSend,
                    expected_generation: generation,
                    expected_human_input_revision: revision,
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
    async fn send_with_wait(
        &self,
        gate: &InjectGate,
        root: PathBuf,
        request: WorkSendRequest,
        startup: Duration,
    ) -> Result<WorkSendResult, String> {
        // 셸과 무관한 모델·effort 검증을 먼저 끝낸다. 인용은 셸을 확인한 뒤 다시 만든다.
        validated_command(&request.agent, CommandSyntax::Posix)?;
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
        let shell = require_supported_shell(sink.as_ref(), &request.agent_id)?;
        let command = validated_command(&request.agent, shell.syntax())?;
        if self.attempted.lock().unwrap().contains(&request.session_id) {
            return Err("work-send-restart-required".into());
        }
        // 조회 불가인 새 브로커 셸도 ACK 없이는 프롬프트를 절대 보내지 않는다.
        // 막 띄운 셸은 rc 파일을 실행하는 동안 자식이 foreground라 사용 중으로 보인다.
        // 새 세션만 빈 프롬프트가 될 때까지 잠시 기다리고, 기존 세션은 바로 거절한다.
        let settle = tokio::time::Instant::now() + FRESH_SHELL_SETTLE;
        loop {
            let shell_prompt = sink.is_shell_prompt(&request.agent_id);
            if shell_prompt == Some(true) || (shell_prompt.is_none() && request.fresh_session) {
                break;
            }
            if !request.fresh_session || tokio::time::Instant::now() >= settle {
                return Err("work-send-restart-required".into());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
            check()?;
        }
        let owner = gate
            .try_acquire_transition_owner(&request.agent_id, &request.session_id, generation)
            .map_err(|_| "work-send-busy".to_string())?;
        check()?;
        // agy는 실제 설치 CLI의 정확한 모델 ID만 허용한다(기존 자동화와 동일).
        if let WorkCli::Agy = request.agent.cli {
            if let Some(model) = &request.agent.model {
                let models = crate::summarizer::list_agy_models().await;
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
        let launch = transition.prepare_launch(
            shell,
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
                    source: InjectSource::WorkSend,
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
                    source: InjectSource::WorkSend,
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
/// 업무 보내기는 Unix bash/zsh와 Windows Git Bash·PowerShell을 지원한다.
fn require_supported_shell(sink: &dyn InputSink, agent_id: &str) -> Result<LaunchShell, String> {
    sink.shell_path_for(agent_id)
        .as_deref()
        .and_then(LaunchShell::for_shell_path)
        .ok_or_else(|| "work-send-shell-unsupported".into())
}
pub fn validated_command(agent: &WorkAgent, syntax: CommandSyntax) -> Result<String, String> {
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
    launch::launch_command_for(
        cli,
        agent.model.as_deref(),
        agent.effort.as_deref(),
        syntax,
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
        sink.set_shell_path("a", SUPPORTED_SHELL);
        sink
    }
    #[cfg(unix)]
    const SUPPORTED_SHELL: &str = "/bin/bash";
    #[cfg(windows)]
    const SUPPORTED_SHELL: &str = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";
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
        assert!(validated_command(&agent, CommandSyntax::Posix).is_err());
        agent.model = Some("$(touch injected)'".into());
        assert_eq!(
            validated_command(&agent, CommandSyntax::Posix).unwrap(),
            "claude --model '$(touch injected)'\\'''"
        );
        assert_eq!(
            validated_command(&agent, CommandSyntax::PowerShell).unwrap(),
            "claude --model '$(touch injected)'''"
        );
    }
    #[test]
    fn windows_shells_are_supported_but_wsl_and_cmd_are_not() {
        let sink = recording();
        for (path, expected) in [
            (r"C:\Program Files\Git\bin\bash.exe", Some(LaunchShell::GitBash)),
            (r"C:\Program Files\PowerShell\7\pwsh.exe", Some(LaunchShell::PowerShell)),
            ("powershell.exe", Some(LaunchShell::PowerShell)),
            ("wsl.exe", None),
            (r"C:\Windows\System32\cmd.exe", None),
            ("bash.exe", None),
        ] {
            sink.set_shell_path("a", path);
            let actual = require_supported_shell(sink.as_ref(), "a").ok();
            assert_eq!(actual, if cfg!(windows) { expected } else { None }, "{path}");
        }
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
    struct ExecutingSink {
        recorded: Arc<RecordingSink>,
        launch: Mutex<Option<String>>,
        children: Mutex<Vec<std::process::Child>>,
        exit_immediately: bool,
        /// 남은 '사용 중' 응답 수. 0이면 조회 불가(None)로 답한다.
        busy_polls: std::sync::atomic::AtomicUsize,
    }
    impl ExecutingSink {
        fn new(exit_immediately: bool) -> Arc<Self> {
            Arc::new(Self {
                recorded: recording(),
                launch: Mutex::new(None),
                children: Mutex::new(vec![]),
                exit_immediately,
                busy_polls: Default::default(),
            })
        }
    }
    impl Drop for ExecutingSink {
        fn drop(&mut self) {
            for child in self.children.get_mut().unwrap() {
                let _ = child.wait();
            }
        }
    }
    impl InputSink for ExecutingSink {
        fn write(&self, agent: &str, data: &str) {
            self.recorded.write(agent, data);
            if data.starts_with(". '") || data.starts_with("& ([scriptblock]") {
                *self.launch.lock().unwrap() = Some(data.into());
            }
            if data == "\r" {
                if let Some(command) = self.launch.lock().unwrap().take() {
                    #[cfg(unix)]
                    let mut process = {
                        let body = if self.exit_immediately { "return 127" } else { "sleep 1" };
                        let mut process = std::process::Command::new("/bin/bash");
                        process.arg("-c").arg(format!("claude() {{ {body}; }}; {command}"));
                        process
                    };
                    #[cfg(windows)]
                    let mut process = {
                        let body = if self.exit_immediately {
                            "cmd /c exit 127"
                        } else {
                            "Start-Sleep -Seconds 1"
                        };
                        let mut process = std::process::Command::new(SUPPORTED_SHELL);
                        process
                            .args(["-NoProfile", "-NonInteractive", "-Command"])
                            .arg(format!("function claude {{ {body} }}; {command}"));
                        process
                    };
                    let child = process
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
            self.busy_polls
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
                .then_some(false)
        }
    }
    #[tokio::test(start_paused = true)]
    async fn fresh_session_cannot_override_a_known_busy_terminal() {
        let root = tempfile::tempdir().unwrap();
        let sink = ExecutingSink::new(false);
        sink.busy_polls.store(usize::MAX, Ordering::SeqCst);
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
    #[tokio::test]
    async fn fresh_session_waits_for_shell_startup_before_launch() {
        let root = tempfile::tempdir().unwrap();
        let sink = ExecutingSink::new(false);
        sink.busy_polls.store(3, Ordering::SeqCst);
        let gate = InjectGate::new(sink.clone(), Arc::new(BotPromptArms::default()));
        let runtime = WorkSendRuntime::default();
        runtime
            .send_with_wait(&gate, root.path().into(), request(), Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(sink.recorded.recorded_writes().len(), 4);
    }
    #[tokio::test]
    async fn existing_session_busy_shell_is_rejected_without_waiting() {
        let root = tempfile::tempdir().unwrap();
        let sink = ExecutingSink::new(false);
        sink.busy_polls.store(1, Ordering::SeqCst);
        let gate = InjectGate::new(sink.clone(), Arc::new(BotPromptArms::default()));
        let runtime = WorkSendRuntime::default();
        let mut req = request();
        req.fresh_session = false;
        assert_eq!(
            runtime
                .send_with_wait(&gate, root.path().into(), req, Duration::ZERO)
                .await
                .unwrap_err(),
            "work-send-restart-required"
        );
        assert!(sink.recorded.recorded_writes().is_empty());
    }
    async fn wait_for_launch(sink: &ExecutingSink) {
        for _ in 0..100 {
            if sink.recorded.recorded_writes().len() >= 2 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("launch not submitted");
    }
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
                    // PowerShell 기동·cmd 반환 시간을 넉넉히 둔다. receipt를 보면 바로 끝난다.
                    Duration::from_secs(5)
                )
                .await
                .unwrap_err(),
            "work-send-cli-returned"
        );
        assert_eq!(sink.recorded.recorded_writes().len(), 2);
    }
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
                gate.submit("a", "s", "other", InjectSource::WorkSend).await,
                SubmitOutcome::Deferred(PendingReason::AnotherProducer)
            );
            runtime.stop("a");
        };
        let (result, _) = tokio::join!(send, intervene);
        assert_eq!(result.unwrap_err(), "work-send-cancelled");
        assert_eq!(sink.recorded.recorded_writes().len(), 2);
    }
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
    fn follow_up_request(text: &str) -> WorkFollowUpRequest {
        WorkFollowUpRequest { agent_id: "a".into(), session_id: "s".into(), text: text.into() }
    }
    #[tokio::test]
    async fn follow_up_pastes_one_prompt_into_the_running_cli() {
        let sink = recording();
        let gate = InjectGate::new(sink.clone(), Arc::new(BotPromptArms::default()));
        let runtime = WorkSendRuntime::default();
        let result = runtime
            .follow_up(&gate, follow_up_request("Run the tests\n\nthen report"))
            .await
            .unwrap();
        assert_eq!(result.session_id, "s");
        let writes: Vec<_> = sink.recorded_writes().into_iter().map(|(_, text, _)| text).collect();
        assert_eq!(writes, ["\x1b[200~Run the tests\n\nthen report\x1b[201~", "\r"]);
        // 끝난 뒤에는 상호배제가 풀려 다음 지시를 받을 수 있다.
        assert!(runtime.follow_up(&gate, follow_up_request("again")).await.is_ok());
    }
    #[tokio::test]
    async fn follow_up_never_types_over_human_input_or_into_a_replaced_session() {
        let sink = recording();
        let gate = InjectGate::new(sink.clone(), Arc::new(BotPromptArms::default()));
        let runtime = WorkSendRuntime::default();
        sink.set_session("a", "other");
        assert_eq!(
            runtime.follow_up(&gate, follow_up_request("go")).await.unwrap_err(),
            "work-send-session-changed"
        );
        sink.set_session("a", "s");
        gate.note_human("a", "half typed");
        assert_eq!(
            runtime.follow_up(&gate, follow_up_request("go")).await.unwrap_err(),
            "work-send-human-input"
        );
        assert_eq!(
            runtime.follow_up(&gate, follow_up_request("bad \x1b[31m")).await.unwrap_err(),
            "work-send-prompt-invalid"
        );
        // 사람 입력은 그대로 전달되지만 자동 프롬프트는 한 글자도 쓰지 않는다.
        assert!(sink.recorded_writes().iter().all(|(_, text, _)| text == "half typed"));
    }
}

#[cfg(all(test, unix))]
mod pty_smoke;

#[cfg(all(test, windows))]
mod pty_smoke_windows;
