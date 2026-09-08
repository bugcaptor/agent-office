// src-tauri/src/automation/mod.rs
//
// 사용자 정의 터미널 입력 자동화 런타임 (kbm #2t9).
//
// 런타임 구조:
// - 탭별로 tokio 태스크를 띄우고 상태를 Arc<Mutex>로 공유한다.
// - Watching 타임아웃 시 CLI 입력을 중단하지 않고 TimeoutDecision으로 전이.
// - `automation_decide` 커맨드로 연장/중단을 원자적으로 1회만 처리한다.

pub mod cli_transition;
pub mod definition;
#[cfg(test)]
mod definition_integration_tests;
#[cfg(all(test, unix))]
mod v2_safety_tests;
pub mod definition_runner;
pub mod marker;
#[cfg(test)]
mod pty_smoke;
pub mod runner;
pub mod store;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::async_runtime::JoinHandle;
use tokio::sync::{mpsc, oneshot};

use crate::automation::runner::{
    automation_loop, cancel_status, now_ms, AutomationTimeouts, DecisionCommand,
};
use crate::session::inject::{InjectGate, InputSink};
use crate::types::{
    AutomationAgentStatus, AutomationCli, AutomationDecisionChoice, AutomationPhase,
    AutomationStatus,
};

/// 자동화 태스크가 공유하는 앱 상태 클론.
pub struct AutomationContext {
    pub sink: Arc<dyn InputSink>,
    pub gate: Arc<InjectGate>,
    /// 봇 모드와의 상호 배제 확인용.
    pub bot_runtime: Arc<crate::bot::BotRuntime>,
}

/// 끝난(running=false) 항목을 맵에서 청소하기까지의 유예.
const FINISHED_SWEEP_MS: u64 = 60_000;

/// 한 탭(agentId)의 살아있는 자동화 태스크.
struct RunningAutomation {
    shutdown: Option<oneshot::Sender<()>>,
    decision_tx: mpsc::UnboundedSender<DecisionCommand>,
    handle: Option<JoinHandle<()>>,
    status: Arc<Mutex<AutomationAgentStatus>>,
    cancel: Arc<AtomicBool>,
    finished_at: Arc<Mutex<Option<Instant>>>,
}

/// 자동화 태스크들의 소유자. AppState가 Arc로 보유한다.
#[derive(Default)]
pub struct AutomationRuntime {
    tasks: Mutex<HashMap<String, RunningAutomation>>,
}

impl AutomationRuntime {
    /// 정의 실행 진입점. legacy 점검 API와 같은 입력 관문/시간 계약을 재사용한다.
    /// 정의의 첫 LaunchCli가 지정한 내장 CLI만 허용하며, 실행 스냅샷은 시작 즉시
    /// app-data 기록으로 남겨 앱이 죽어도 자동 재전송하지 않는다.
    pub fn start_definition(
        &self,
        ctx: Arc<AutomationContext>,
        agent_id: String,
        definition: crate::types::AutomationDefinition,
        inputs: std::collections::BTreeMap<String, String>,
        workspace: String,
        store: Arc<crate::automation::store::AutomationStore>,
    ) -> Result<AutomationAgentStatus, String> {
        self.start_definition_inner(ctx, agent_id, definition, inputs, workspace, store, None)
    }

    /// 이미 비동기 사전 점검이 잡아 둔 세션에서만 정의를 시작한다. 모델 카탈로그
    /// 조회처럼 await를 사이에 둔 호출자가 새 PTY에 옛 정의를 붙이는 것을 막는다.
    pub fn start_definition_for_session(
        &self,
        ctx: Arc<AutomationContext>,
        agent_id: String,
        definition: crate::types::AutomationDefinition,
        inputs: std::collections::BTreeMap<String, String>,
        workspace: String,
        store: Arc<crate::automation::store::AutomationStore>,
        expected_session_id: &str,
    ) -> Result<AutomationAgentStatus, String> {
        self.start_definition_inner(
            ctx,
            agent_id,
            definition,
            inputs,
            workspace,
            store,
            Some(expected_session_id),
        )
    }

    fn start_definition_inner(
        &self,
        ctx: Arc<AutomationContext>,
        agent_id: String,
        definition: crate::types::AutomationDefinition,
        mut inputs: std::collections::BTreeMap<String, String>,
        workspace: String,
        store: Arc<crate::automation::store::AutomationStore>,
        expected_session_id: Option<&str>,
    ) -> Result<AutomationAgentStatus, String> {
        crate::automation::definition::validate(&definition)?;
        let workspace = std::fs::canonicalize(&workspace)
            .map_err(|_| "automation-workspace-invalid".to_string())?;
        if !workspace.is_dir() {
            return Err("automation-workspace-invalid".into());
        }
        const RESERVED_INPUT_KEYS: [&str; 4] = ["workspace", "run", "cycle", "previousResult"];
        if definition
            .inputs
            .iter()
            .any(|input| RESERVED_INPUT_KEYS.contains(&input.key.as_str()))
        {
            return Err("automation-input-key-reserved".into());
        }
        if inputs
            .keys()
            .any(|key| !definition.inputs.iter().any(|input| &input.key == key))
        {
            return Err("automation-input-unknown".into());
        }
        for input in &definition.inputs {
            if !inputs.contains_key(&input.key) {
                let Some(default) = &input.default else {
                    return Err(format!("automation-input-required:{}", input.key));
                };
                inputs.insert(input.key.clone(), default.clone());
            }
        }
        if !ctx.sink.is_running(&agent_id) {
            return Err("automation-session-not-running".into());
        }
        let session_id = ctx
            .sink
            .session_id_for(&agent_id)
            .ok_or_else(|| "automation-session-lost".to_string())?;
        if expected_session_id.is_some_and(|expected| expected != session_id) {
            return Err("automation-session-changed".into());
        }
        if definition.schema_version == 2 {
            let shell = ctx.sink.shell_path_for(&agent_id);
            let capability = crate::automation::cli_transition::CliTransition::capability_for_shell_path(shell.as_deref());
            if !capability.auto_return_supported {
                return Err(capability.unavailable_reason.unwrap_or_else(|| "automation-cli-shell-unsupported".into()));
            }
        }
        let cli = definition
            .steps
            .iter()
            .find_map(|s| match s {
                crate::types::AutomationStep::LaunchCli { cli_profile_id, .. } => {
                    match cli_profile_id.as_str() {
                        "claude" => Some(AutomationCli::Claude),
                        "codex" => Some(AutomationCli::Codex),
                        "agy" => Some(AutomationCli::Agy),
                        "kilo" => Some(AutomationCli::Kilo),
                        "pi" => Some(AutomationCli::Pi),
                        _ => None,
                    }
                }
                _ => None,
            })
            .unwrap_or_default();
        if definition.steps.iter().any(|s| matches!(s, crate::types::AutomationStep::LaunchCli { cli_profile_id, .. } if !matches!(cli_profile_id.as_str(), "claude" | "codex" | "agy" | "kilo" | "pi"))) { return Err("automation-cli-profile-unsupported".into()); }
        if ctx.bot_runtime.is_running(&agent_id) {
            return Err("automation-bot-running".into());
        }
        let mut tasks = self.tasks.lock().unwrap();
        if tasks
            .get(&agent_id)
            .is_some_and(|r| r.status.lock().unwrap().running)
        {
            return Err("automation-already-running".into());
        }
        let run_id = uuid::Uuid::new_v4().to_string();
        let record = crate::types::AutomationRunRecord {
            run_id: run_id.clone(),
            definition_snapshot: definition,
            inputs,
            workspace: workspace.display().to_string(),
            agent_id: agent_id.clone(),
            status: "running".into(),
            outcome: None,
            events: vec![crate::types::AutomationRunEvent {
                at: now_ms(),
                kind: "started".into(),
                details: None,
            }],
        };
        store.append_run(&record)?;
        let status = Arc::new(Mutex::new(AutomationAgentStatus {
            running: true,
            phase: AutomationPhase::Launching,
            cli,
            file_path: String::new(),
            started_at_ms: now_ms(),
            run_id: Some(run_id.clone()),
            step_execution_id: None,
            deadline_ms: None,
            decision_id: None,
            extension_count: None,
            marker_observed_at_ms: None,
            pending_reason: None,
            pending_since_ms: None,
            error: None,
            definition_id: Some(record.definition_snapshot.id.clone()),
            definition_name: Some(record.definition_snapshot.name.clone()),
            workspace: Some(record.workspace.clone()),
            session_id: Some(session_id.clone()),
            cycle: Some(1),
            step_index: Some(0),
            step_id: None,
            outcome: None,
            completed_at_ms: None,
            decision_message: None,
            decision_reason: None,
            cli_context: if record.definition_snapshot.schema_version == 2 { Some(crate::types::AutomationCliContext::Unknown) } else { None },
            cli_return_observed_at_ms: None,
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let finished_at = Arc::new(Mutex::new(None));
        let (tx, rx) = oneshot::channel();
        let (dtx, drx) = mpsc::unbounded_channel();
        let handle = tauri::async_runtime::spawn(crate::automation::definition_runner::run(
            ctx,
            agent_id.clone(),
            session_id,
            record.definition_snapshot,
            record.inputs,
            record.workspace,
            status.clone(),
            rx,
            drx,
            cancel.clone(),
            finished_at.clone(),
            store,
            run_id,
        ));
        let snapshot = status.lock().unwrap().clone();
        tasks.insert(
            agent_id,
            RunningAutomation {
                shutdown: Some(tx),
                decision_tx: dtx,
                handle: Some(handle),
                status,
                cancel,
                finished_at,
            },
        );
        Ok(snapshot)
    }
    /// 이 탭의 자동화를 시작한다. 기본 타임아웃 계약을 적용한다.
    pub fn start(
        &self,
        ctx: Arc<AutomationContext>,
        agent_id: String,
        cli: AutomationCli,
    ) -> AutomationAgentStatus {
        self.start_with_timeouts(ctx, agent_id, cli, AutomationTimeouts::default())
    }

    /// 타임아웃 설정을 지정하여 자동화를 시작한다 (테스트 및 커스텀 설정용).
    pub fn start_with_timeouts(
        &self,
        ctx: Arc<AutomationContext>,
        agent_id: String,
        cli: AutomationCli,
        timeouts: AutomationTimeouts,
    ) -> AutomationAgentStatus {
        let mut tasks = self.tasks.lock().unwrap();
        sweep_finished(&mut tasks);
        if let Some(ra) = tasks.get_mut(&agent_id) {
            let snapshot = ra.status.lock().unwrap().clone();
            if snapshot.running {
                return snapshot;
            }
            // 끝났다고 표시됐어도 태스크가 아직 안 죽었을 수 있다(중단 직후 재시작).
            // 새 것을 넣기 전에 옛 것을 확실히 내린다 -- 안 그러면 같은 탭에 둘이
            // 잠깐 공존하고, 옛 쪽이 마지막 한 줄을 더 쳐 넣을 수 있다.
            ra.cancel.store(true, Ordering::Relaxed);
            if let Some(tx) = ra.shutdown.take() {
                let _ = tx.send(());
            }
        }
        if ctx.bot_runtime.is_running(&agent_id) {
            return AutomationAgentStatus {
                running: false,
                phase: AutomationPhase::Failed,
                cli,
                file_path: String::new(),
                started_at_ms: now_ms(),
                run_id: None,
                step_execution_id: None,
                deadline_ms: None,
                decision_id: None,
                extension_count: None,
                marker_observed_at_ms: None,
                pending_reason: None,
                pending_since_ms: None,
                error: Some("automation-bot-running".to_string()),
                ..AutomationAgentStatus::empty()
            };
        }
        let status = Arc::new(Mutex::new(AutomationAgentStatus {
            running: true,
            phase: AutomationPhase::Launching,
            cli,
            file_path: String::new(),
            started_at_ms: now_ms(),
            run_id: None,
            step_execution_id: None,
            deadline_ms: None,
            decision_id: None,
            extension_count: None,
            marker_observed_at_ms: None,
            pending_reason: None,
            pending_since_ms: None,
            error: None,
            ..AutomationAgentStatus::empty()
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let finished_at = Arc::new(Mutex::new(None));
        let (tx, rx) = oneshot::channel();
        let (decision_tx, decision_rx) = mpsc::unbounded_channel();
        let handle = tauri::async_runtime::spawn(automation_loop(
            ctx,
            agent_id.clone(),
            cli,
            status.clone(),
            rx,
            decision_rx,
            cancel.clone(),
            finished_at.clone(),
            timeouts,
        ));
        let snapshot = status.lock().unwrap().clone();
        tasks.insert(
            agent_id,
            RunningAutomation {
                shutdown: Some(tx),
                decision_tx,
                handle: Some(handle),
                status,
                cancel,
                finished_at,
            },
        );
        snapshot
    }

    /// 이 탭의 자동화를 중단한다.
    ///
    /// Phase 2 계약: cancelling -> 태스크 종료 대기 -> cancelled + finished_at.
    /// IPC를 막지 않기 위해 cancelling으로 즉시 표시하고 백그라운드에서 await한다.
    pub fn stop(&self, agent_id: &str) {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(ra) = tasks.get_mut(agent_id) {
            ra.cancel.store(true, Ordering::Relaxed);
            if let Some(tx) = ra.shutdown.take() {
                let _ = tx.send(());
            }
            let still_running = ra.status.lock().unwrap().running;
            if still_running {
                {
                    let mut s = ra.status.lock().unwrap();
                    s.phase = AutomationPhase::Cancelling;
                }
                if let Some(handle) = ra.handle.take() {
                    let status = ra.status.clone();
                    let finished_at = ra.finished_at.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = handle.await;
                        // 기다리는 사이 러너가 스스로 마감했을 수 있다(Done/Failed).
                        // 그 결과를 Cancelled 로 덮지 않는다 — 아직 Cancelling 인
                        // 것만 우리가 마감한다.
                        let still_cancelling =
                            status.lock().unwrap().phase == AutomationPhase::Cancelling;
                        if still_cancelling {
                            cancel_status(&status, &finished_at);
                        } else if finished_at.lock().unwrap().is_none() {
                            *finished_at.lock().unwrap() = Some(Instant::now());
                        }
                    });
                } else {
                    cancel_status(&ra.status, &ra.finished_at);
                }
            } else if ra.finished_at.lock().unwrap().is_none() {
                *ra.finished_at.lock().unwrap() = Some(Instant::now());
            }
        }
    }

    /// 터미널 재시작 경계에서 동기적으로 취소하고 상태를 분리한다.
    /// 반환 퓨처는 러너의 정상 취소/이력 저장을 기다린다. 호출자는 취소 뒤
    /// PTY를 폐기하고 이 퓨처를 기다린 다음 새 세션을 만든다.
    pub fn reset_session(&self, agent_id: &str) -> impl std::future::Future<Output = ()> + Send + 'static {
        let mut task = self.tasks.lock().unwrap().remove(agent_id);
        if let Some(task) = task.as_mut() {
            task.cancel.store(true, Ordering::Relaxed);
            if let Some(tx) = task.shutdown.take() {
                let _ = tx.send(());
            }
        }
        async move {
            if let Some(mut task) = task {
                if let Some(handle) = task.handle.take() {
                    let _ = handle.await;
                }
                // stop()이 이미 handle을 기다리는 경우에도 그 상태 Arc는 맵에서
                // 분리됐다. 늦은 Cancelled 갱신은 새 세션 상태를 되살릴 수 없다.
            }
        }
    }

    /// 종료 훅에서 모든 자동화 태스크를 내린다.
    pub fn stop_all(&self) {
        let mut tasks = self.tasks.lock().unwrap();
        for (_, mut ra) in tasks.drain() {
            ra.cancel.store(true, Ordering::Relaxed);
            if let Some(tx) = ra.shutdown.take() {
                let _ = tx.send(());
            }
            // 종료 이벤트 루프는 동기라 JoinHandle을 await할 수 없다. abort는
            // submit의 CR 전 대기도 취소하며 HolderGuard가 입력 큐를 해제한다.
            if let Some(handle) = ra.handle.take() {
                handle.abort();
            }
        }
    }

    /// 자동화가 도는(또는 방금 끝난) 탭들의 상태 스냅샷.
    pub fn status(&self) -> AutomationStatus {
        let mut tasks = self.tasks.lock().unwrap();
        sweep_finished(&mut tasks);
        let mut agents = std::collections::BTreeMap::new();
        for (id, ra) in tasks.iter() {
            agents.insert(id.clone(), ra.status.lock().unwrap().clone());
        }
        AutomationStatus { agents }
    }

    /// 타임아웃 결정(연장 / 중단)을 원자적으로 한 번만 적용한다.
    /// decisionId가 일치하고 아직 미적용 상태일 때만 true를 반환한다.
    pub fn decide(
        &self,
        agent_id: &str,
        run_id: &str,
        step_execution_id: &str,
        decision_id: &str,
        choice: AutomationDecisionChoice,
    ) -> Result<bool, String> {
        let tasks = self.tasks.lock().unwrap();
        let Some(ra) = tasks.get(agent_id) else {
            return Ok(false);
        };

        let mut status = ra.status.lock().unwrap();
        if !status.running {
            return Ok(false);
        }
        if !matches!(
            status.phase,
            AutomationPhase::TimeoutDecision
                | AutomationPhase::Confirming
                | AutomationPhase::WaitingHumanInput
        ) {
            return Ok(false);
        }
        if status.run_id.as_deref() != Some(run_id) {
            return Ok(false);
        }
        if status.step_execution_id.as_deref() != Some(step_execution_id) {
            return Ok(false);
        }
        if status.decision_id.as_deref() != Some(decision_id) {
            // 이미 처리되었거나 다른 결정 ID
            return Ok(false);
        }

        // Return timeouts cannot be bypassed by the generic Continue command.
        if status.decision_reason == Some(crate::types::AutomationDecisionReason::CliExitTimeout)
            && choice == AutomationDecisionChoice::Continue
        {
            return Ok(false);
        }
        if matches!(status.decision_reason, Some(crate::types::AutomationDecisionReason::ShellReady | crate::types::AutomationDecisionReason::CliExitUnconfirmed))
            && choice == AutomationDecisionChoice::Extend
        {
            return Ok(false);
        }

        // 원자적으로 decision_id 소모 (중복 클릭 방어)
        status.decision_id = None;

        match choice {
            AutomationDecisionChoice::Extend | AutomationDecisionChoice::Continue => {
                let count = status.extension_count.unwrap_or(0) + 1;
                status.extension_count = Some(count);
                status.phase = AutomationPhase::Watching;
                // 새 deadline 은 러너가 **누른 시각 + 이 실행의 설정 대기시간**으로 정한다.
                // 여기서 30분을 다시 적으면 단계별 설정을 무시한 두 번째 진실이 생긴다.
                status.deadline_ms = None;
                let _ = ra
                    .decision_tx
                    .send(if choice == AutomationDecisionChoice::Continue {
                        DecisionCommand::Continue
                    } else {
                        DecisionCommand::Extend
                    });
            }
            AutomationDecisionChoice::Stop => {
                drop(status);
                ra.cancel.store(true, Ordering::Relaxed);
                let _ = ra.decision_tx.send(DecisionCommand::Stop);
                // 러너가 이미 죽었어도 상태가 running 인 채 남지 않도록 여기서도 마감한다.
                cancel_status(&ra.status, &ra.finished_at);
            }
        }

        Ok(true)
    }
}

/// 끝난 지 `FINISHED_SWEEP_MS` 넘은 항목을 맵에서 지운다.
fn sweep_finished(tasks: &mut HashMap<String, RunningAutomation>) {
    tasks.retain(|_, ra| {
        let finished_at = *ra.finished_at.lock().unwrap();
        match finished_at {
            Some(at) => at.elapsed() < Duration::from_millis(FINISHED_SWEEP_MS),
            None => true,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automation_cli_command_strings() {
        assert_eq!(AutomationCli::Claude.command(), "claude");
        assert_eq!(AutomationCli::Codex.command(), "codex");
        assert_eq!(AutomationCli::Agy.command(), "agy");
        assert_eq!(AutomationCli::Kilo.command(), "kilo");
        assert_eq!(AutomationCli::Pi.command(), "pi");
    }

    #[test]
    fn sweep_finished_keeps_running_and_recent() {
        let mut tasks: HashMap<String, RunningAutomation> = HashMap::new();
        let (tx, _rx) = oneshot::channel();
        let (dtx, _drx) = mpsc::unbounded_channel();
        let handle = tauri::async_runtime::spawn(async {});
        tasks.insert(
            "still-running".to_string(),
            RunningAutomation {
                shutdown: Some(tx),
                decision_tx: dtx,
                handle: Some(handle),
                status: Arc::new(Mutex::new(AutomationAgentStatus {
                    running: true,
                    phase: AutomationPhase::Watching,
                    cli: AutomationCli::Claude,
                    file_path: String::new(),
                    started_at_ms: 0,
                    run_id: None,
                    step_execution_id: None,
                    deadline_ms: None,
                    decision_id: None,
                    extension_count: None,
                    marker_observed_at_ms: None,
                    pending_reason: None,
                    pending_since_ms: None,
                    error: None,
                    ..AutomationAgentStatus::empty()
                })),
                cancel: Arc::new(AtomicBool::new(false)),
                finished_at: Arc::new(Mutex::new(None)),
            },
        );
        sweep_finished(&mut tasks);
        assert!(tasks.contains_key("still-running"));
    }

    #[test]
    fn sweep_finished_removes_old_finished() {
        let mut tasks: HashMap<String, RunningAutomation> = HashMap::new();
        let (tx, _rx) = oneshot::channel();
        let (dtx, _drx) = mpsc::unbounded_channel();
        let handle = tauri::async_runtime::spawn(async {});
        let old = Instant::now()
            .checked_sub(Duration::from_millis(FINISHED_SWEEP_MS + 1_000))
            .unwrap();
        tasks.insert(
            "long-done".to_string(),
            RunningAutomation {
                shutdown: Some(tx),
                decision_tx: dtx,
                handle: Some(handle),
                status: Arc::new(Mutex::new(AutomationAgentStatus {
                    running: false,
                    phase: AutomationPhase::Done,
                    cli: AutomationCli::Claude,
                    file_path: String::new(),
                    started_at_ms: 0,
                    run_id: None,
                    step_execution_id: None,
                    deadline_ms: None,
                    decision_id: None,
                    extension_count: None,
                    marker_observed_at_ms: None,
                    pending_reason: None,
                    pending_since_ms: None,
                    error: None,
                    ..AutomationAgentStatus::empty()
                })),
                cancel: Arc::new(AtomicBool::new(false)),
                finished_at: Arc::new(Mutex::new(Some(old))),
            },
        );
        sweep_finished(&mut tasks);
        assert!(!tasks.contains_key("long-done"));
    }

    #[test]
    fn decide_atomic_single_application() {
        let runtime = AutomationRuntime::default();
        let (tx, _rx) = oneshot::channel();
        let (dtx, mut drx) = mpsc::unbounded_channel();
        let handle = tauri::async_runtime::spawn(async {});
        let status = Arc::new(Mutex::new(AutomationAgentStatus {
            running: true,
            phase: AutomationPhase::TimeoutDecision,
            cli: AutomationCli::Claude,
            file_path: "/tmp/result.json".to_string(),
            started_at_ms: 1000,
            run_id: Some("run-1".to_string()),
            step_execution_id: Some("step-1".to_string()),
            deadline_ms: None,
            decision_id: Some("dec-123".to_string()),
            extension_count: None,
            marker_observed_at_ms: None,
            pending_reason: None,
            pending_since_ms: None,
            error: None,
            ..AutomationAgentStatus::empty()
        }));

        runtime.tasks.lock().unwrap().insert(
            "agent-1".to_string(),
            RunningAutomation {
                shutdown: Some(tx),
                decision_tx: dtx,
                handle: Some(handle),
                status: status.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
                finished_at: Arc::new(Mutex::new(None)),
            },
        );

        // 첫 번째 결정: 성공해야 함
        let res1 = runtime.decide(
            "agent-1",
            "run-1",
            "step-1",
            "dec-123",
            AutomationDecisionChoice::Extend,
        );
        assert_eq!(res1.unwrap(), true);

        // 첫 번째 결정 후 채널로 커맨드가 와야 함
        assert!(drx.try_recv().is_ok());

        // status는 Watching으로 전이되고 extension_count = 1, decision_id = None.
        // deadline 은 여기서 정하지 않는다 — 러너가 이 실행의 설정 대기시간으로 다시 잡는다.
        {
            let s = status.lock().unwrap();
            assert_eq!(s.phase, AutomationPhase::Watching);
            assert_eq!(s.extension_count, Some(1));
            assert_eq!(s.decision_id, None);
            assert_eq!(s.deadline_ms, None);
        }

        // 두 번째 결정(중복 클릭): decision_id가 없으므로 false 반환해야 함
        let res2 = runtime.decide(
            "agent-1",
            "run-1",
            "step-1",
            "dec-123",
            AutomationDecisionChoice::Extend,
        );
        assert_eq!(res2.unwrap(), false);
    }

    /// 헬퍼: TimeoutDecision 상태로 등록된 자동화 하나를 만든다.
    fn seed_timeout_decision(runtime: &AutomationRuntime) -> Arc<Mutex<AutomationAgentStatus>> {
        let (tx, mut rx) = oneshot::channel();
        let (dtx, mut drx) = mpsc::unbounded_channel();
        let handle = tauri::async_runtime::spawn(async move {
            tokio::select! {
                _ = &mut rx => {}
                _ = drx.recv() => {}
            }
        });
        let status = Arc::new(Mutex::new(AutomationAgentStatus {
            running: true,
            phase: AutomationPhase::TimeoutDecision,
            cli: AutomationCli::Claude,
            file_path: String::new(),
            started_at_ms: 1000,
            run_id: Some("run-1".to_string()),
            step_execution_id: Some("step-1".to_string()),
            deadline_ms: None,
            decision_id: Some("dec-1".to_string()),
            extension_count: None,
            marker_observed_at_ms: None,
            pending_reason: None,
            pending_since_ms: None,
            error: None,
            ..AutomationAgentStatus::empty()
        }));
        runtime.tasks.lock().unwrap().insert(
            "agent-1".to_string(),
            RunningAutomation {
                shutdown: Some(tx),
                decision_tx: dtx,
                handle: Some(handle),
                status: status.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
                finished_at: Arc::new(Mutex::new(None)),
            },
        );
        status
    }

    #[tokio::test]
    async fn reset_session_removes_snapshot_and_rejects_old_decisions() {
        let runtime = AutomationRuntime::default();
        seed_timeout_decision(&runtime);
        runtime.reset_session("agent-1").await;
        assert!(!runtime.status().agents.contains_key("agent-1"));
        assert!(!runtime.decide("agent-1", "run-1", "step-1", "dec-1", AutomationDecisionChoice::Continue).unwrap());
        // 재시작 뒤의 새 실행은 이전 정리의 영향을 받지 않는다.
        let fresh = seed_timeout_decision(&runtime);
        assert!(runtime.status().agents.contains_key("agent-1"));
        assert!(fresh.lock().unwrap().running);
        runtime.reset_session("agent-1").await;
    }

    #[tokio::test]
    async fn reset_session_after_stop_cannot_restore_cancelled_snapshot() {
        let runtime = AutomationRuntime::default();
        seed_timeout_decision(&runtime);
        runtime.stop("agent-1");
        runtime.reset_session("agent-1").await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!runtime.status().agents.contains_key("agent-1"));
        runtime.reset_session("missing").await;
    }

    /// 사용자 중단은 실패가 아니다 — Cancelled 로 끝나고 에러 코드를 남기지 않는다.
    #[test]
    fn decide_stop_ends_as_cancelled_without_error() {
        let runtime = AutomationRuntime::default();
        let status = seed_timeout_decision(&runtime);

        let ok = runtime
            .decide(
                "agent-1",
                "run-1",
                "step-1",
                "dec-1",
                AutomationDecisionChoice::Stop,
            )
            .unwrap();
        assert!(ok);

        let s = status.lock().unwrap();
        assert!(!s.running);
        assert_eq!(s.phase, AutomationPhase::Cancelled);
        assert_eq!(s.error, None);
        assert_eq!(s.decision_id, None);
    }

    /// 중단한 항목은 맵에서 곧바로 사라지지 않는다 — 화면이 결말을 봐야 하고,
    /// finished_at 이 박혀 있어야 나중에 sweep 된다.
    #[tokio::test]
    async fn stop_marks_cancelled_and_keeps_entry_for_sweep() {
        let runtime = AutomationRuntime::default();
        let status = seed_timeout_decision(&runtime);

        runtime.stop("agent-1");

        // Phase 2: stop 직후에는 Cancelling 전이
        {
            let s = status.lock().unwrap();
            assert_eq!(s.phase, AutomationPhase::Cancelling);
        }

        // 태스크 종료 await 후 Cancelled + finished_at 설정됨
        tokio::time::sleep(Duration::from_millis(20)).await;
        {
            let s = status.lock().unwrap();
            assert!(!s.running);
            assert_eq!(s.phase, AutomationPhase::Cancelled);
        }
        let snapshot = runtime.status();
        assert!(snapshot.agents.contains_key("agent-1"));

        // finished_at 이 박혔으므로 유예가 지나면 sweep 대상이다.
        {
            let mut tasks = runtime.tasks.lock().unwrap();
            let ra = tasks.get_mut("agent-1").unwrap();
            let old = Instant::now()
                .checked_sub(Duration::from_millis(FINISHED_SWEEP_MS + 1_000))
                .unwrap();
            *ra.finished_at.lock().unwrap() = Some(old);
            sweep_finished(&mut tasks);
            assert!(!tasks.contains_key("agent-1"));
        }
    }
}
