// src-tauri/src/automation/runner.rs
//
// 자동화 실행 루프 본체 (Phase 1):
// - 사전 확인 -> Launching -> WaitingStartup (10초) -> Injecting ->
//   Watching (30분) -> (타임아웃 시 TimeoutDecision -> 선택 대기) ->
//   마커 관측 시 Settling (10초) -> Done.
//
// 마지막 단계 뒤에 종료 입력(`/exit`)을 넣지 않는다 — 종료는 사용자가 정의에
// 넣었을 때만 하는 일이고(설계 §1·§2), Phase 1에는 아직 정의가 없다.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::automation::marker::{
    self, prompt_path, read_result_file, result_path, AutomationResultStatus, ResultRead,
};
use crate::automation::AutomationContext;
use crate::session::inject::{InjectSource, SubmitOutcome};
use crate::types::{AutomationAgentStatus, AutomationCli, AutomationPhase};

/// 실행 시간 설정 파라미터.
#[derive(Debug, Clone, Copy)]
pub struct AutomationTimeouts {
    /// CLI 기동 대기 (기본 10초).
    pub startup_wait_ms: u64,
    /// 한 작업의 대기 상한 (기본 30분).
    pub wait_timeout_ms: u64,
    /// 결과 파일 폴링 주기 (기본 1초).
    pub poll_interval_ms: u64,
    /// 마커 관측 후 마무리 대기 (기본 10초).
    pub settling_wait_ms: u64,
}

impl Default for AutomationTimeouts {
    fn default() -> Self {
        Self {
            startup_wait_ms: 10_000,
            wait_timeout_ms: 30 * 60 * 1_000,
            poll_interval_ms: 1_000,
            settling_wait_ms: 10_000,
        }
    }
}

pub use crate::session::inject::now_ms;

/// 타임아웃 결정 명령.
#[derive(Debug, Clone, Copy)]
pub enum DecisionCommand {
    /// 더 기다린다. 새 deadline은 **누른 시각 + 이 실행의 설정 대기시간**이며,
    /// 계산은 러너가 한다 — deadline의 주인을 한 곳으로 모은다.
    Extend,
    Continue,
    Stop,
}

/// 상태를 실패로 마감한다.
pub fn fail(
    status: &Arc<Mutex<AutomationAgentStatus>>,
    finished_at: &Arc<Mutex<Option<Instant>>>,
    error: String,
) {
    {
        let mut s = status.lock().unwrap();
        s.running = false;
        s.phase = AutomationPhase::Failed;
        s.error = Some(error);
        s.deadline_ms = None;
        s.decision_id = None;
        s.pending_reason = None;
        s.pending_since_ms = None;
    }
    *finished_at.lock().unwrap() = Some(Instant::now());
}

/// 사용자 중단으로 상태를 마감한다. **실패가 아니다** — CLI도 터미널도 작업 내용도
/// 그대로 두고 감시만 멈춘 것이라, 에러 코드를 남기지 않고 `Cancelled` 로 끝낸다.
pub fn cancel_status(
    status: &Arc<Mutex<AutomationAgentStatus>>,
    finished_at: &Arc<Mutex<Option<Instant>>>,
) {
    {
        let mut s = status.lock().unwrap();
        s.running = false;
        s.phase = AutomationPhase::Cancelled;
        s.deadline_ms = None;
        s.decision_id = None;
        s.pending_reason = None;
        s.pending_since_ms = None;
    }
    *finished_at.lock().unwrap() = Some(Instant::now());
}

/// `millis`만큼 대기하되, 그 사이 종료 신호가 오면 즉시 true(중단됨)를 반환한다.
pub async fn wait_or_cancelled(
    shutdown_rx: &mut oneshot::Receiver<()>,
    cancel: &Arc<AtomicBool>,
    millis: u64,
) -> bool {
    if cancel.load(Ordering::Relaxed) {
        return true;
    }
    tokio::select! {
        _ = &mut *shutdown_rx => true,
        _ = tokio::time::sleep(Duration::from_millis(millis)) => false,
    }
}

/// 관문을 통한 자동 입력 제출. 보류(Deferred) 시 재시도 루프를 돈다.
async fn submit_or_wait(
    ctx: &AutomationContext,
    agent_id: &str,
    session_id: &str,
    text: &str,
    status: &Arc<Mutex<AutomationAgentStatus>>,
    finished_at: &Arc<Mutex<Option<Instant>>>,
    shutdown_rx: &mut oneshot::Receiver<()>,
    cancel: &Arc<AtomicBool>,
    poll_interval_ms: u64,
) -> bool {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        // CR 앞 150ms 대기도 종료 신호보다 우선하면 안 된다. submit future를
        // 드롭하면 InjectGate의 HolderGuard가 사람 입력 큐를 즉시 방출한다.
        let submitted = tokio::select! {
            biased;
            _ = &mut *shutdown_rx => return false,
            outcome = ctx.gate.submit(agent_id, session_id, text, InjectSource::Automation) => outcome,
        };
        match submitted {
            SubmitOutcome::Submitted => {
                let mut s = status.lock().unwrap();
                s.pending_reason = None;
                s.pending_since_ms = None;
                return true;
            }
            SubmitOutcome::Deferred(reason) => {
                {
                    let mut s = status.lock().unwrap();
                    s.pending_reason = Some(reason);
                    if s.pending_since_ms.is_none() {
                        s.pending_since_ms = Some(now_ms());
                    }
                }
                if wait_or_cancelled(shutdown_rx, cancel, poll_interval_ms).await {
                    return false;
                }
            }
            SubmitOutcome::SessionChanged => {
                fail(
                    status,
                    finished_at,
                    "automation-session-changed".to_string(),
                );
                return false;
            }
            SubmitOutcome::NotRunning => {
                fail(status, finished_at, "automation-session-lost".to_string());
                return false;
            }
        }
    }
}

/// 자동화 루프 본체.
pub async fn automation_loop(
    ctx: Arc<AutomationContext>,
    agent_id: String,
    cli: AutomationCli,
    status: Arc<Mutex<AutomationAgentStatus>>,
    mut shutdown_rx: oneshot::Receiver<()>,
    mut decision_rx: mpsc::UnboundedReceiver<DecisionCommand>,
    cancel: Arc<AtomicBool>,
    finished_at: Arc<Mutex<Option<Instant>>>,
    timeouts: AutomationTimeouts,
) {
    // 0) 사전 확인
    if !ctx.sink.is_running(&agent_id) {
        fail(
            &status,
            &finished_at,
            "automation-session-not-running".to_string(),
        );
        return;
    }
    let Some(session_id) = ctx.sink.session_id_for(&agent_id) else {
        fail(&status, &finished_at, "automation-session-lost".to_string());
        return;
    };
    let Some(cwd) = ctx.sink.cwd_of(&agent_id) else {
        fail(&status, &finished_at, "automation-cwd-unknown".to_string());
        return;
    };

    let run_id = Uuid::new_v4().to_string();
    let step_execution_id = Uuid::new_v4().to_string();
    let workspace = Path::new(&cwd);

    let res_path = match result_path(workspace, &run_id, &step_execution_id) {
        Ok(p) => p,
        Err(e) => {
            fail(
                &status,
                &finished_at,
                format!("automation-path-invalid: {e}"),
            );
            return;
        }
    };
    let pr_path = match prompt_path(workspace, &run_id, &step_execution_id) {
        Ok(p) => p,
        Err(e) => {
            fail(
                &status,
                &finished_at,
                format!("automation-path-invalid: {e}"),
            );
            return;
        }
    };

    // 실행 폴더 준비
    if let Some(parent) = res_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            fail(
                &status,
                &finished_at,
                format!("automation-file-reset-failed: {e}"),
            );
            return;
        }
    }
    // 만든 폴더가 정말 workspace 안인지 확인한다 -- `.agent-office` 가 심링크면
    // 여기서 걸린다(경로 검증은 우리가 만든 식별자만 보므로 그 위는 못 막는다).
    if let Some(parent) = res_path.parent() {
        if let Err(e) = marker::verify_within_workspace(workspace, parent) {
            fail(
                &status,
                &finished_at,
                format!("automation-path-invalid: {e}"),
            );
            return;
        }
    }
    // 실행 산출물은 사용자 저장소의 작업 폴더 안에 쌓인다. 남의 리포에
    // 우리 제어 파일이 커밋 후보로 뜨지 않도록 `.agent-office/` 를 자기-무시로 둔다.
    marker::ensure_ignored(workspace);

    // 프롬프트 본문 파일 작성 (개행과 형식 보존)
    let prompt_content = format!(
        "# 자동화 점검 작업\n\n\
        오늘 날짜(YYYY-MM-DD)와 짧은 인사말 한 줄을 출력해라.\n\
        작업을 모두 마친 뒤 반드시 아래 결과 파일에 완료 JSON을 작성해라:\n\
        {}\n",
        res_path.display()
    );
    if let Err(e) = std::fs::write(&pr_path, prompt_content) {
        fail(
            &status,
            &finished_at,
            format!("automation-prompt-write-failed: {e}"),
        );
        return;
    }

    // 상태 초기화
    {
        let mut s = status.lock().unwrap();
        s.run_id = Some(run_id.clone());
        s.step_execution_id = Some(step_execution_id.clone());
        s.file_path = res_path.to_string_lossy().into_owned();
        s.phase = AutomationPhase::Launching;
    }

    // 1) Launching — CLI 실행
    if !submit_or_wait(
        &ctx,
        &agent_id,
        &session_id,
        cli.command(),
        &status,
        &finished_at,
        &mut shutdown_rx,
        &cancel,
        timeouts.poll_interval_ms,
    )
    .await
    {
        return;
    }

    // 2) WaitingStartup — 기동 대기 (기본 10초)
    {
        status.lock().unwrap().phase = AutomationPhase::WaitingStartup;
    }
    if wait_or_cancelled(&mut shutdown_rx, &cancel, timeouts.startup_wait_ms).await {
        return;
    }

    // 3) Injecting — 짧은 한 줄 지시만 전달
    {
        status.lock().unwrap().phase = AutomationPhase::Injecting;
    }
    let instruction = format!(
        "cat '{}' 지시를 수행해라. 작업을 마친 뒤 마지막에 '{}' 파일에 JSON으로 {{\"version\":1,\"runId\":\"{}\",\"stepExecutionId\":\"{}\",\"status\":\"done\",\"summary\":\"완료\"}} 를 써라.",
        pr_path.display(),
        res_path.display(),
        run_id,
        step_execution_id,
    );
    if !submit_or_wait(
        &ctx,
        &agent_id,
        &session_id,
        &instruction,
        &status,
        &finished_at,
        &mut shutdown_rx,
        &cancel,
        timeouts.poll_interval_ms,
    )
    .await
    {
        return;
    }

    // 4) Watching — 결과 파일 완료 마커 감시 (기본 30분)
    let mut deadline_ms = now_ms() + timeouts.wait_timeout_ms;
    {
        let mut s = status.lock().unwrap();
        s.phase = AutomationPhase::Watching;
        s.deadline_ms = Some(deadline_ms);
    }

    loop {
        // Watching 폴링 루프
        loop {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            if !ctx.sink.is_running(&agent_id) {
                fail(&status, &finished_at, "automation-session-lost".to_string());
                return;
            }
            if ctx.sink.session_id_for(&agent_id).as_deref() != Some(&session_id) {
                fail(
                    &status,
                    &finished_at,
                    "automation-session-changed".to_string(),
                );
                return;
            }

            // 결과 파일 검사
            match read_result_file(&res_path, &run_id, &step_execution_id) {
                ResultRead::Accepted(res)
                    if res.status == AutomationResultStatus::Done
                        || res.status == AutomationResultStatus::Complete =>
                {
                    // 마커 관측 완료!
                    break;
                }
                ResultRead::UnsupportedVersion(v) => {
                    fail(
                        &status,
                        &finished_at,
                        format!("automation-result-version-unsupported: v{v}"),
                    );
                    return;
                }
                _ => {}
            }

            // 타임아웃 판정
            if now_ms() >= deadline_ms {
                // 타임아웃 발생! CLI에 아무런 입력도 넣지 않음.
                break;
            }

            if wait_or_cancelled(&mut shutdown_rx, &cancel, timeouts.poll_interval_ms).await {
                return;
            }
        }

        if cancel.load(Ordering::Relaxed) {
            return;
        }

        // 마커를 관측했는지 확인
        let marker_detected = match read_result_file(&res_path, &run_id, &step_execution_id) {
            ResultRead::Accepted(res) => {
                res.status == AutomationResultStatus::Done
                    || res.status == AutomationResultStatus::Complete
            }
            _ => false,
        };

        if marker_detected {
            break;
        }

        // 아직 마커가 없는데 루프를 빠져나왔다면 타임아웃임 -> TimeoutDecision 전이
        let decision_id = Uuid::new_v4().to_string();
        {
            let mut s = status.lock().unwrap();
            s.phase = AutomationPhase::TimeoutDecision;
            s.decision_id = Some(decision_id.clone());
            s.deadline_ms = None;
        }

        // TimeoutDecision 대기 루프 (사용자 선택을 기다린다)
        let mut decided = false;
        while !decided {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            if !ctx.sink.is_running(&agent_id) {
                fail(&status, &finished_at, "automation-session-lost".to_string());
                return;
            }

            // 대기 중 마커가 도착하는지 확인 (패널에 감지 표시)
            match read_result_file(&res_path, &run_id, &step_execution_id) {
                ResultRead::Accepted(res)
                    if res.status == AutomationResultStatus::Done
                        || res.status == AutomationResultStatus::Complete =>
                {
                    let mut s = status.lock().unwrap();
                    if s.marker_observed_at_ms.is_none() {
                        s.marker_observed_at_ms = Some(now_ms());
                    }
                }
                ResultRead::UnsupportedVersion(v) => {
                    fail(
                        &status,
                        &finished_at,
                        format!("automation-result-version-unsupported: v{v}"),
                    );
                    return;
                }
                _ => {}
            }

            // 사용자 결정 신호 또는 정지 신호 확인
            tokio::select! {
                _ = &mut shutdown_rx => {
                    return;
                }
                cmd = decision_rx.recv() => {
                    match cmd {
                        Some(DecisionCommand::Extend | DecisionCommand::Continue) => {
                            // 누른 시각 + **이 실행의 설정 대기시간**. 원래 만료 시각 기준이 아니다.
                            deadline_ms = now_ms() + timeouts.wait_timeout_ms;
                            {
                                let mut s = status.lock().unwrap();
                                s.phase = AutomationPhase::Watching;
                                s.deadline_ms = Some(deadline_ms);
                            }
                            decided = true;
                        }
                        Some(DecisionCommand::Stop) => {
                            cancel_status(&status, &finished_at);
                            return;
                        }
                        None => {
                            return;
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(timeouts.poll_interval_ms)) => {}
            }
        }
    }

    // 5) Settling — 마커 관측 후 마무리 대기 (기본 10초)
    // 계획서 5절: 마커를 본 시점에 human_input_epoch을 읽어 둔다
    let marker_epoch = ctx.gate.human_input_epoch(&agent_id);
    {
        let mut s = status.lock().unwrap();
        s.phase = AutomationPhase::Settling;
        if s.marker_observed_at_ms.is_none() {
            s.marker_observed_at_ms = Some(now_ms());
        }
        s.deadline_ms = None;
        s.decision_id = None;
    }
    if wait_or_cancelled(&mut shutdown_rx, &cancel, timeouts.settling_wait_ms).await {
        return;
    }

    // Settling 끝난 직후 사람의 직접 입력(CR) 여부 확인
    let current_epoch = ctx.gate.human_input_epoch(&agent_id);
    if current_epoch > marker_epoch {
        eprintln!(
            "agent-office: [automation] {agent_id} human intervention detected (epoch {marker_epoch} -> {current_epoch}), subsequent automated submits invalidated"
        );
    }

    // 6) Done — 종료 입력은 넣지 않는다. 사용자가 그 CLI에서 계속 일하고 있을 수 있고,
    //    종료는 정의에 `ExitCli` 단계가 있을 때만 하는 일이다(설계 §1·§2).
    {
        let mut s = status.lock().unwrap();
        s.running = false;
        s.phase = AutomationPhase::Done;
    }
    *finished_at.lock().unwrap() = Some(Instant::now());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::inject::RecordingSink;
    use crate::state::BotPromptArms;

    #[test]
    fn default_timeouts_match_design_contracts() {
        let timeouts = AutomationTimeouts::default();
        assert_eq!(timeouts.startup_wait_ms, 10_000);
        assert_eq!(timeouts.wait_timeout_ms, 30 * 60 * 1_000);
        assert_eq!(timeouts.settling_wait_ms, 10_000);
        assert_eq!(timeouts.poll_interval_ms, 1_000);
    }

    #[tokio::test]
    async fn wait_or_cancelled_respects_duration() {
        let (_tx, mut rx) = oneshot::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let start = Instant::now();
        let cancelled = wait_or_cancelled(&mut rx, &cancel, 50).await;
        assert!(!cancelled);
        assert!(start.elapsed() >= Duration::from_millis(45));
    }

    #[tokio::test]
    async fn wait_or_cancelled_early_exit_on_cancel() {
        let (_tx, mut rx) = oneshot::channel();
        let cancel = Arc::new(AtomicBool::new(true));
        let start = Instant::now();
        let cancelled = wait_or_cancelled(&mut rx, &cancel, 5_000).await;
        assert!(cancelled);
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    #[tokio::test]
    async fn wait_or_cancelled_early_exit_on_shutdown() {
        let (tx, mut rx) = oneshot::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let _ = tx.send(());
        let start = Instant::now();
        let cancelled = wait_or_cancelled(&mut rx, &cancel, 5_000).await;
        assert!(cancelled);
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    fn test_context(
        sink: Arc<dyn crate::session::inject::InputSink>,
        arms: Arc<BotPromptArms>,
    ) -> Arc<AutomationContext> {
        let gate = Arc::new(crate::session::inject::InjectGate::new(sink.clone(), arms));
        Arc::new(AutomationContext {
            sink,
            gate,
            bot_runtime: Arc::new(crate::bot::BotRuntime::default()),
        })
    }

    // 1. startup_wait_blocks_task_prompt: 기동 +9,999ms까지 작업 프롬프트 쓰기 0회
    #[tokio::test]
    async fn startup_wait_blocks_task_prompt() {
        let tmp = tempfile::tempdir().unwrap();
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", true);
        sink.set_session("a1", "s1");
        sink.set_cwd("a1", tmp.path().to_str().unwrap());

        let arms = Arc::new(BotPromptArms::new());
        let ctx = test_context(sink.clone(), arms);

        let status = Arc::new(Mutex::new(AutomationAgentStatus {
            running: true,
            phase: AutomationPhase::Launching,
            cli: AutomationCli::Claude,
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
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (_dtx, drx) = mpsc::unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let finished_at = Arc::new(Mutex::new(None));

        let timeouts = AutomationTimeouts {
            startup_wait_ms: 300, // 300ms 동안 기동 대기
            wait_timeout_ms: 500,
            poll_interval_ms: 10,
            settling_wait_ms: 50,
        };

        let loop_handle = tokio::spawn(automation_loop(
            ctx,
            "a1".to_string(),
            AutomationCli::Claude,
            status.clone(),
            shutdown_rx,
            drx,
            cancel.clone(),
            finished_at,
            timeouts,
        ));

        // 200ms 시점: Launching(150ms) 직후 WaitingStartup 단계여야 하며, 작업 프롬프트(cat 지시)는 아직 들어가지 않아야 함
        tokio::time::sleep(Duration::from_millis(200)).await;
        {
            let s = status.lock().unwrap();
            assert_eq!(s.phase, AutomationPhase::WaitingStartup);
        }
        let writes = sink.recorded_writes();
        // CLI 실행 ("claude", "\r") 외에 추가 작업 프롬프트 쓰기는 0회여야 함
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0].1, "claude");

        let _ = shutdown_tx.send(());
        let _ = loop_handle.await;
    }

    // 2. timeout_writes_nothing_to_cli: 타임아웃 뒤 쓰기 0회
    #[tokio::test]
    async fn timeout_writes_nothing_to_cli() {
        let tmp = tempfile::tempdir().unwrap();
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", true);
        sink.set_session("a1", "s1");
        sink.set_cwd("a1", tmp.path().to_str().unwrap());

        let arms = Arc::new(BotPromptArms::new());
        let ctx = test_context(sink.clone(), arms);

        let status = Arc::new(Mutex::new(AutomationAgentStatus {
            running: true,
            phase: AutomationPhase::Launching,
            cli: AutomationCli::Claude,
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
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (_dtx, drx) = mpsc::unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let finished_at = Arc::new(Mutex::new(None));

        let timeouts = AutomationTimeouts {
            startup_wait_ms: 10,
            wait_timeout_ms: 20, // 20ms 후 타임아웃
            poll_interval_ms: 10,
            settling_wait_ms: 50,
        };

        let loop_handle = tokio::spawn(automation_loop(
            ctx,
            "a1".to_string(),
            AutomationCli::Claude,
            status.clone(),
            shutdown_rx,
            drx,
            cancel.clone(),
            finished_at,
            timeouts,
        ));

        // Launching(150ms) + WaitingStartup(10ms) + Injecting(150ms) + Watching(20ms) = ~330ms
        // 400ms 대기 시 TimeoutDecision 전이 완료
        tokio::time::sleep(Duration::from_millis(400)).await;
        {
            let s = status.lock().unwrap();
            assert_eq!(s.phase, AutomationPhase::TimeoutDecision);
            assert!(s.decision_id.is_some());
        }

        // 프롬프트 입력 완료(2건) + 작업 프롬프트 완료(2건) = 총 4건 이후 타임아웃에 따른 CLI 쓰기 0회!
        let writes = sink.recorded_writes();
        assert_eq!(writes.len(), 4);
        assert!(!writes.iter().any(|(_, text, _)| text.contains("/exit")));

        let _ = shutdown_tx.send(());
        let _ = loop_handle.await;
    }

    // 3. extend_deadline_is_press_time_plus_configured_wait
    #[tokio::test]
    async fn extend_deadline_is_press_time_plus_configured_wait() {
        let tmp = tempfile::tempdir().unwrap();
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", true);
        sink.set_session("a1", "s1");
        sink.set_cwd("a1", tmp.path().to_str().unwrap());

        let arms = Arc::new(BotPromptArms::new());
        let ctx = test_context(sink.clone(), arms);

        let status = Arc::new(Mutex::new(AutomationAgentStatus {
            running: true,
            phase: AutomationPhase::Launching,
            cli: AutomationCli::Claude,
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
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (dtx, drx) = mpsc::unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let finished_at = Arc::new(Mutex::new(None));

        let timeouts = AutomationTimeouts {
            startup_wait_ms: 10,
            wait_timeout_ms: 100, // 100ms 작업 대기
            poll_interval_ms: 10,
            settling_wait_ms: 50,
        };

        let loop_handle = tokio::spawn(automation_loop(
            ctx,
            "a1".to_string(),
            AutomationCli::Claude,
            status.clone(),
            shutdown_rx,
            drx,
            cancel.clone(),
            finished_at,
            timeouts,
        ));

        // 150(launch) + 10(startup) + 150(inject) + 100(watch) = 410ms
        // 450ms 대기 시 확실히 TimeoutDecision 도달
        tokio::time::sleep(Duration::from_millis(450)).await;
        {
            let s = status.lock().unwrap();
            assert_eq!(s.phase, AutomationPhase::TimeoutDecision);
        }

        // Extend 명령 전송
        let press_time = now_ms();
        dtx.send(DecisionCommand::Extend).unwrap();

        tokio::time::sleep(Duration::from_millis(15)).await;
        {
            let s = status.lock().unwrap();
            assert_eq!(s.phase, AutomationPhase::Watching);
            let deadline = s.deadline_ms.expect("deadline set on extend");
            // 새 deadline은 누른 시각 + 설정 대기시간(100ms) 근처여야 함 (오차 50ms 이내)
            let diff = (deadline as i64) - ((press_time + timeouts.wait_timeout_ms) as i64);
            assert!(diff.abs() < 50, "deadline diff is {diff}");
        }

        let _ = shutdown_tx.send(());
        let _ = loop_handle.await;
    }

    // 4. no_writes_after_marker: 마커 뒤 쓰기 무기한 0회
    #[tokio::test]
    async fn no_writes_after_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", true);
        sink.set_session("a1", "s1");
        sink.set_cwd("a1", tmp.path().to_str().unwrap());

        let arms = Arc::new(BotPromptArms::new());
        let ctx = test_context(sink.clone(), arms);

        let status = Arc::new(Mutex::new(AutomationAgentStatus {
            running: true,
            phase: AutomationPhase::Launching,
            cli: AutomationCli::Claude,
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
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        let (_dtx, drx) = mpsc::unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let finished_at = Arc::new(Mutex::new(None));

        let timeouts = AutomationTimeouts {
            startup_wait_ms: 10,
            wait_timeout_ms: 1_000,
            poll_interval_ms: 10,
            settling_wait_ms: 30, // 30ms 마무리 대기
        };

        let loop_handle = tokio::spawn(automation_loop(
            ctx,
            "a1".to_string(),
            AutomationCli::Claude,
            status.clone(),
            shutdown_rx,
            drx,
            cancel.clone(),
            finished_at,
            timeouts,
        ));

        // Launching(150ms) + 10ms + Injecting(150ms) = 310ms -> Watching 진입 대기
        tokio::time::sleep(Duration::from_millis(350)).await;

        // 결과 파일 작성 (Done 마커)
        let (file_path, run_id, step_execution_id) = {
            let s = status.lock().unwrap();
            (
                s.file_path.clone(),
                s.run_id.clone().unwrap(),
                s.step_execution_id.clone().unwrap(),
            )
        };
        let res_json = format!(
            "{{\"version\":1,\"runId\":\"{}\",\"stepExecutionId\":\"{}\",\"status\":\"done\",\"summary\":\"끝\"}}",
            run_id, step_execution_id
        );
        std::fs::write(&file_path, res_json).unwrap();

        // Settling(30ms) 지나서 루프 종료될 때까지 대기
        let _ = loop_handle.await;
        {
            let s = status.lock().unwrap();
            assert_eq!(s.phase, AutomationPhase::Done);
            assert!(!s.running);
        }

        let writes_count = sink.recorded_writes().len();
        // 종료 후 추가 시간 대기
        tokio::time::sleep(Duration::from_millis(50)).await;
        // 마커 뒤 추가 쓰기는 0회!
        assert_eq!(sink.recorded_writes().len(), writes_count);
    }

    // 5. stop_stops_all_writes: 중단 뒤 새 쓰기 0회
    #[tokio::test]
    async fn stop_stops_all_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", true);
        sink.set_session("a1", "s1");
        sink.set_cwd("a1", tmp.path().to_str().unwrap());

        let arms = Arc::new(BotPromptArms::new());
        let ctx = test_context(sink.clone(), arms);

        let status = Arc::new(Mutex::new(AutomationAgentStatus {
            running: true,
            phase: AutomationPhase::Launching,
            cli: AutomationCli::Claude,
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
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        let (_dtx, drx) = mpsc::unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let finished_at = Arc::new(Mutex::new(None));

        let timeouts = AutomationTimeouts {
            startup_wait_ms: 10,
            wait_timeout_ms: 1_000,
            poll_interval_ms: 10,
            settling_wait_ms: 50,
        };

        let cancel_clone = cancel.clone();
        let loop_handle = tokio::spawn(automation_loop(
            ctx,
            "a1".to_string(),
            AutomationCli::Claude,
            status.clone(),
            shutdown_rx,
            drx,
            cancel_clone,
            finished_at,
            timeouts,
        ));

        // 기동 중 즉시 cancel
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancel.store(true, Ordering::Relaxed);

        let _ = loop_handle.await;
        let count_at_stop = sink.recorded_writes().len();

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(sink.recorded_writes().len(), count_at_stop);
    }
}
