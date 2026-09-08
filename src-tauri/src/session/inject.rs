// src-tauri/src/session/inject.rs
//
// 사용자 정의 터미널 입력 자동화 관문 (kbm #2t9 Phase 2).
//
// 역할:
// - 자동 입력 생산자(Bot, Talk, WebRemote, Automation) 간 상호 배타 보장.
// - 본문 -> 150ms -> CR 직렬화 구간 동안 들어오는 사람 입력을 버리지 않고 큐잉 후 전달.
// - 미제출(uncommitted) 사람 입력이 있을 때 자동 제출 보류(최대 2분).
// - 사람이 직접 입력·제출한 횟수(human_input_epoch) 추적으로 마커 뒤 예약 제출 무효화 지원.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::session::manager::SessionManager;
use crate::state::BotPromptArms;

/// 미제출 사람 입력 보류 상한 (기본 2분 = 120,000ms).
pub const DEFAULT_HUMAN_HOLD_MAX_MS: u64 = 120_000;

/// 현재 시각(epoch ms).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 자동 입력의 출처.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectSource {
    Bot,
    Talk,
    WebRemote,
    Automation,
}

/// 제출을 미룬 이유 (배너 문구로 노출).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PendingReason {
    HumanTyping,
    AnotherProducer,
}

/// submit 시도 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitOutcome {
    Submitted,
    Deferred(PendingReason),
    SessionChanged,
    NotRunning,
}

/// CLI 전환이 일반 자동 입력 생산자를 막고 있는 동안의 소유권.
///
/// 이 가드는 전환 전체(셸 확인부터 다음 CLI 준비 대기까지)를 살아 있어야 한다.
/// 사람 입력은 막지 않고 전달하며, Drop에서 어떤 종료 경로든 소유권을 푼다.
pub struct TransitionOwnerGuard {
    gate: AgentGate,
    sink: Arc<dyn InputSink>,
    agent_id: String,
    session_id: String,
    generation: u64,
    _lock: tokio::sync::OwnedMutexGuard<()>,
}

/// 전환 소유권 취득 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionAcquireOutcome {
    Deferred(PendingReason),
    SessionChanged,
    NotRunning,
    GenerationChanged,
}

/// strict 제출에서 일반 submit과 구별해야 하는 결과.
///
/// `*BeforeCr`는 본문 조각은 이미 PTY에 썼지만 자동 CR은 보내지 않았음을 뜻한다.
/// 이 경우 뒤이어 방출되는 사람 Enter가 그 조각을 제출할 수도 있으므로, 실행이
/// 취소됐다고 해석하면 안 된다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrictOutcome {
    Submitted,
    Deferred(PendingReason),
    SessionChangedBeforeBody,
    SessionChangedBeforeCr,
    NotRunningBeforeBody,
    NotRunningBeforeCr,
    GenerationChangedBeforeBody,
    GenerationChangedBeforeCr,
    CancelledBeforeBody,
    CancelledBeforeCr,
    HumanInputChangedBeforeBody,
    HumanInputChangedBeforeCr,
}

/// transition owner가 strict 본문/CR 직전에 검증할 불변 근거.
pub struct StrictSubmitRequest<'a> {
    pub text: &'a str,
    pub source: InjectSource,
    pub expected_generation: u64,
    pub expected_human_input_revision: u64,
    pub cancelled: &'a AtomicBool,
}

/// 실제 쓰기 트레이트. 테스트에서 RecordingSink로 교체한다.
pub trait InputSink: Send + Sync {
    fn write(&self, agent_id: &str, data: &str);
    fn write_for_session(&self, agent_id: &str, session_id: &str, data: &str) -> bool {
        if !self.is_running(agent_id)
            || self.session_id_for(agent_id).as_deref() != Some(session_id)
        {
            return false;
        }
        self.write(agent_id, data);
        true
    }
    fn session_id_for(&self, agent_id: &str) -> Option<String>;
    fn is_running(&self, agent_id: &str) -> bool;
    fn cwd_of(&self, _agent_id: &str) -> Option<String> {
        None
    }
    /// 직접 관리 세션에서 실제로 해석된 셸 실행 경로. 외부 attach 등은 None.
    fn shell_path_for(&self, _agent_id: &str) -> Option<String> {
        None
    }
}

/// SessionManager를 감싼 기본 InputSink 구현.
pub struct ManagerSink {
    manager: Arc<SessionManager>,
}

impl ManagerSink {
    pub fn new(manager: Arc<SessionManager>) -> Self {
        Self { manager }
    }
}

impl InputSink for ManagerSink {
    fn write(&self, agent_id: &str, data: &str) {
        self.manager.write_input(agent_id, data);
    }
    fn write_for_session(&self, agent_id: &str, session_id: &str, data: &str) -> bool {
        self.manager
            .write_input_for_session(agent_id, session_id, data)
    }
    fn session_id_for(&self, agent_id: &str) -> Option<String> {
        self.manager.session_id_for(agent_id)
    }
    fn is_running(&self, agent_id: &str) -> bool {
        self.manager.is_running(agent_id)
    }
    fn cwd_of(&self, agent_id: &str) -> Option<String> {
        self.manager.cwd_of(agent_id)
    }
    fn shell_path_for(&self, agent_id: &str) -> Option<String> {
        self.manager.shell_path_for(agent_id)
    }
}

/// 테스트용 기록 싱크.
#[derive(Default)]
pub struct RecordingSink {
    pub writes: Mutex<Vec<(String, String, u64)>>,
    pub sessions: Mutex<HashMap<String, String>>,
    pub running: Mutex<HashMap<String, bool>>,
    pub cwds: Mutex<HashMap<String, String>>,
    pub shell_paths: Mutex<HashMap<String, String>>,
}

impl RecordingSink {
    pub fn set_session(&self, agent_id: &str, session_id: &str) {
        self.sessions
            .lock()
            .insert(agent_id.to_string(), session_id.to_string());
    }

    pub fn set_running(&self, agent_id: &str, running: bool) {
        self.running.lock().insert(agent_id.to_string(), running);
    }

    pub fn set_cwd(&self, agent_id: &str, cwd: &str) {
        self.cwds
            .lock()
            .insert(agent_id.to_string(), cwd.to_string());
    }

    pub fn set_shell_path(&self, agent_id: &str, shell_path: &str) {
        self.shell_paths
            .lock()
            .insert(agent_id.to_string(), shell_path.to_string());
    }

    pub fn recorded_writes(&self) -> Vec<(String, String, u64)> {
        self.writes.lock().clone()
    }
}

impl InputSink for RecordingSink {
    fn write(&self, agent_id: &str, data: &str) {
        self.writes
            .lock()
            .push((agent_id.to_string(), data.to_string(), now_ms()));
    }
    fn session_id_for(&self, agent_id: &str) -> Option<String> {
        self.sessions.lock().get(agent_id).cloned()
    }
    fn is_running(&self, agent_id: &str) -> bool {
        self.running.lock().get(agent_id).copied().unwrap_or(false)
    }
    fn cwd_of(&self, agent_id: &str) -> Option<String> {
        self.cwds.lock().get(agent_id).cloned()
    }
    fn shell_path_for(&self, agent_id: &str) -> Option<String> {
        self.shell_paths.lock().get(agent_id).cloned()
    }
}

#[derive(Default)]
struct AgentGateInner {
    /// CR 없이 들어온 사람 입력이 있다.
    human_uncommitted: bool,
    /// 그 표시가 선 시각(epoch ms).
    uncommitted_since_ms: u64,
    /// 사람이 제출(CR)한 횟수.
    human_input_epoch: u64,
    /// CR 여부와 관계 없이 사람이 입력한 모든 조각의 단조 revision.
    human_input_revision: u64,
    /// 이 agent의 자동화 실행 세대. 전환 제출은 이 값을 매번 확인한다.
    generation: u64,
    /// 2분 상한을 지나 미제출 조각 위로 자동 제출한 횟수(실행 이력용).
    forced_submission_count: u64,
    /// 관문이 잠긴 동안 도착한 사람 입력.
    queued_human: Vec<(Option<String>, String)>,
    /// 지금 잠금을 쥔 생산자.
    holder: Option<InjectSource>,
    /// true이면 본문/CR 원자 구간이라 사람 입력을 잠시 큐잉한다.
    /// 전환 소유권만 들고 기다리는 동안에는 false라서 사람 입력이 바로 전달된다.
    queue_human_while_held: bool,
}

struct AgentGate {
    lock: Arc<tokio::sync::Mutex<()>>,
    inner: Arc<Mutex<AgentGateInner>>,
}

impl AgentGate {
    fn new() -> Self {
        Self {
            lock: Arc::new(tokio::sync::Mutex::new(())),
            inner: Arc::new(Mutex::new(AgentGateInner::default())),
        }
    }
}

fn flush_queued_human(
    inner: &mut AgentGateInner,
    sink: &Arc<dyn InputSink>,
    agent_id: &str,
    session_id: &str,
) {
    let queued = std::mem::take(&mut inner.queued_human);
    // 세션 교체 뒤 도착한 새 입력도 있으므로 입력 당시 세션별로 방출한다.
    for (queued_session_id, data) in queued {
        sink.write_for_session(
            agent_id,
            queued_session_id.as_deref().unwrap_or(session_id),
            &data,
        );
    }
}

impl TransitionOwnerGuard {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

impl Drop for TransitionOwnerGuard {
    fn drop(&mut self) {
        let mut inner = self.gate.inner.lock();
        inner.holder = None;
        inner.queue_human_while_held = false;
        flush_queued_human(&mut inner, &self.sink, &self.agent_id, &self.session_id);
    }
}

/// strict 제출 중에만 사람 입력을 큐잉하고, 어떤 return/cancel 경로에서도 즉시 푼다.
struct StrictSubmissionGuard<'a> {
    owner: &'a TransitionOwnerGuard,
}

impl Drop for StrictSubmissionGuard<'_> {
    fn drop(&mut self) {
        let mut inner = self.owner.gate.inner.lock();
        inner.queue_human_while_held = false;
        flush_queued_human(
            &mut inner,
            &self.owner.sink,
            &self.owner.agent_id,
            &self.owner.session_id,
        );
    }
}

/// `holder` 표식을 반드시 내리는 RAII 가드.
///
/// submit 퓨처가 150ms 대기 중에 통째로 드롭되면(연결 종료, 태스크 취소) 표식이
/// 선 채로 남고, 그 뒤 사람 입력이 전부 `queued_human`에 쌓여 터미널이 먹통이
/// 된다. 그 경로를 막으려고 표식 해제와 큐 방출을 Drop에 둔다.
struct HolderGuard {
    inner: Arc<Mutex<AgentGateInner>>,
    sink: Arc<dyn InputSink>,
    agent_id: String,
    session_id: String,
}

impl Drop for HolderGuard {
    fn drop(&mut self) {
        let mut inner = self.inner.lock();
        inner.holder = None;
        inner.queue_human_while_held = false;
        flush_queued_human(&mut inner, &self.sink, &self.agent_id, &self.session_id);
    }
}

/// 자동 입력 관문.
pub struct InjectGate {
    sink: Arc<dyn InputSink>,
    arms: Arc<BotPromptArms>,
    agents: Mutex<HashMap<String, AgentGate>>,
    human_hold_max_ms: u64,
}

impl InjectGate {
    pub fn new(sink: Arc<dyn InputSink>, arms: Arc<BotPromptArms>) -> Self {
        Self {
            sink,
            arms,
            agents: Mutex::new(HashMap::new()),
            human_hold_max_ms: DEFAULT_HUMAN_HOLD_MAX_MS,
        }
    }

    pub fn with_hold_max(
        sink: Arc<dyn InputSink>,
        arms: Arc<BotPromptArms>,
        human_hold_max_ms: u64,
    ) -> Self {
        Self {
            sink,
            arms,
            agents: Mutex::new(HashMap::new()),
            human_hold_max_ms,
        }
    }

    pub fn sink(&self) -> &Arc<dyn InputSink> {
        &self.sink
    }

    pub fn arms(&self) -> &Arc<BotPromptArms> {
        &self.arms
    }

    fn get_agent_gate(&self, agent_id: &str) -> AgentGate {
        let mut map = self.agents.lock();
        if let Some(gate) = map.get(agent_id) {
            AgentGate {
                lock: gate.lock.clone(),
                inner: gate.inner.clone(),
            }
        } else {
            let gate = AgentGate::new();
            let cloned = AgentGate {
                lock: gate.lock.clone(),
                inner: gate.inner.clone(),
            };
            map.insert(agent_id.to_string(), gate);
            cloned
        }
    }

    /// 새 자동화 실행의 세대를 발급한다. 이전 실행이 남긴 strict 제출은 더 이상
    /// 이 세대에서 admission되지 않는다.
    pub fn begin_generation(&self, agent_id: &str) -> u64 {
        let gate = self.get_agent_gate(agent_id);
        let mut inner = gate.inner.lock();
        inner.generation = inner.generation.wrapping_add(1);
        inner.generation
    }

    pub fn generation(&self, agent_id: &str) -> u64 {
        let gate = self.get_agent_gate(agent_id);
        let generation = gate.inner.lock().generation;
        generation
    }

    /// strict 전환의 장기 소유권을 비차단으로 얻는다.
    pub fn try_acquire_transition_owner(
        &self,
        agent_id: &str,
        session_id: &str,
        expected_generation: u64,
    ) -> Result<TransitionOwnerGuard, TransitionAcquireOutcome> {
        if !self.sink.is_running(agent_id) {
            return Err(TransitionAcquireOutcome::NotRunning);
        }
        if self.sink.session_id_for(agent_id).as_deref() != Some(session_id) {
            return Err(TransitionAcquireOutcome::SessionChanged);
        }
        let gate = self.get_agent_gate(agent_id);
        let lock = match gate.lock.clone().try_lock_owned() {
            Ok(lock) => lock,
            Err(_) => {
                return Err(TransitionAcquireOutcome::Deferred(
                    PendingReason::AnotherProducer,
                ))
            }
        };
        let mut inner = gate.inner.lock();
        if inner.generation != expected_generation {
            return Err(TransitionAcquireOutcome::GenerationChanged);
        }
        inner.holder = Some(InjectSource::Automation);
        // 전환을 기다리는 동안 사람 입력은 직접 전달한다.
        inner.queue_human_while_held = false;
        drop(inner);
        Ok(TransitionOwnerGuard {
            gate,
            sink: self.sink.clone(),
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
            generation: expected_generation,
            _lock: lock,
        })
    }

    /// strict 전환의 본문/CR 제출. 일반 submit과 달리 오래된 사람 조각 위에
    /// 강행하지 않으며, 모든 admission 근거를 본문과 CR 직전에 다시 검사한다.
    pub async fn submit_strict(
        &self,
        owner: &TransitionOwnerGuard,
        request: StrictSubmitRequest<'_>,
    ) -> StrictOutcome {
        let validate = |before_cr: bool, inner: &AgentGateInner| {
            if request.cancelled.load(Ordering::Relaxed) {
                return Some(if before_cr {
                    StrictOutcome::CancelledBeforeCr
                } else {
                    StrictOutcome::CancelledBeforeBody
                });
            }
            if !self.sink.is_running(&owner.agent_id) {
                return Some(if before_cr {
                    StrictOutcome::NotRunningBeforeCr
                } else {
                    StrictOutcome::NotRunningBeforeBody
                });
            }
            if self.sink.session_id_for(&owner.agent_id).as_deref() != Some(&owner.session_id) {
                return Some(if before_cr {
                    StrictOutcome::SessionChangedBeforeCr
                } else {
                    StrictOutcome::SessionChangedBeforeBody
                });
            }
            if inner.generation != request.expected_generation
                || owner.generation != request.expected_generation
            {
                return Some(if before_cr {
                    StrictOutcome::GenerationChangedBeforeCr
                } else {
                    StrictOutcome::GenerationChangedBeforeBody
                });
            }
            if inner.human_input_revision != request.expected_human_input_revision {
                return Some(if before_cr {
                    StrictOutcome::HumanInputChangedBeforeCr
                } else {
                    StrictOutcome::HumanInputChangedBeforeBody
                });
            }
            None
        };

        let submission_guard = StrictSubmissionGuard { owner };
        {
            // Serialize the revision/generation check and the write with note_human.
            let mut inner = owner.gate.inner.lock();
            if let Some(outcome) = validate(false, &inner) {
                return outcome;
            }
            if inner.human_uncommitted {
                return StrictOutcome::Deferred(PendingReason::HumanTyping);
            }
            inner.holder = Some(request.source);
            inner.queue_human_while_held = true;
            self.arms.arm(&owner.agent_id, now_ms());
            let single = crate::bot::runner::single_line(request.text);
            if !self
                .sink
                .write_for_session(&owner.agent_id, &owner.session_id, &single)
            {
                return StrictOutcome::SessionChangedBeforeBody;
            }
        }
        tokio::time::sleep(Duration::from_millis(
            crate::bot::runner::INJECT_SUBMIT_DELAY_MS,
        ))
        .await;
        {
            let inner = owner.gate.inner.lock();
            if let Some(outcome) = validate(true, &inner) {
                return outcome;
            }
            if !self
                .sink
                .write_for_session(&owner.agent_id, &owner.session_id, "\r")
            {
                return StrictOutcome::SessionChangedBeforeCr;
            }
        }
        drop(submission_guard);
        StrictOutcome::Submitted
    }

    /// 자동 입력 한 건. 본문 -> 150ms -> CR을 이 안에서 직렬화해 수행한다.
    pub async fn submit(
        &self,
        agent_id: &str,
        session_id: &str,
        text: &str,
        source: InjectSource,
    ) -> SubmitOutcome {
        // 1. 실행 중인지 확인
        if !self.sink.is_running(agent_id) {
            return SubmitOutcome::NotRunning;
        }

        // 2. 세션 ID 검증
        match self.sink.session_id_for(agent_id) {
            Some(ref sid) if sid == session_id => {}
            _ => return SubmitOutcome::SessionChanged,
        }

        let gate = self.get_agent_gate(agent_id);

        // 3. 비동기 락 try_lock — 다른 생산자가 쥐고 있으면 즉시 AnotherProducer 반환
        let guard = match gate.lock.clone().try_lock_owned() {
            Ok(g) => g,
            Err(_) => return SubmitOutcome::Deferred(PendingReason::AnotherProducer),
        };

        // 4. 사람 입력 보류 판정
        {
            let mut inner = gate.inner.lock();
            if inner.human_uncommitted {
                let now = now_ms();
                let elapsed = now.saturating_sub(inner.uncommitted_since_ms);
                if elapsed < self.human_hold_max_ms {
                    // 2분 미만: 보류
                    drop(guard);
                    return SubmitOutcome::Deferred(PendingReason::HumanTyping);
                } else {
                    // 2분 초과: 보류 해제하고 그대로 진행
                    inner.human_uncommitted = false;
                    inner.forced_submission_count = inner.forced_submission_count.wrapping_add(1);
                    eprintln!(
                        "agent-office: [inject] {agent_id} submitted-over-human-fragment after {elapsed}ms"
                    );
                }
            }
            inner.holder = Some(source);
            inner.queue_human_while_held = true;
        }
        // 여기서부터 표식 해제는 가드가 책임진다(중간에 드롭돼도 반드시 풀린다).
        let holder_guard = HolderGuard {
            inner: gate.inner.clone(),
            sink: self.sink.clone(),
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
        };

        if !self.sink.is_running(agent_id) {
            return SubmitOutcome::NotRunning;
        }
        if self.sink.session_id_for(agent_id).as_deref() != Some(session_id) {
            return SubmitOutcome::SessionChanged;
        }
        // 5. 출처 표식 arm
        self.arms.arm(agent_id, now_ms());

        // 6. 본문 -> 150ms -> CR 쓰기
        let single = crate::bot::runner::single_line(text);
        if !self.sink.write_for_session(agent_id, session_id, &single) {
            return SubmitOutcome::SessionChanged;
        }
        tokio::time::sleep(Duration::from_millis(
            crate::bot::runner::INJECT_SUBMIT_DELAY_MS,
        ))
        .await;
        // 150ms 사이 세션이 교체되면 새 셸에 Enter를 보내지 않는다.
        if !self.sink.is_running(agent_id) {
            return SubmitOutcome::NotRunning;
        }
        if self.sink.session_id_for(agent_id).as_deref() != Some(session_id) {
            return SubmitOutcome::SessionChanged;
        }
        if !self.sink.write_for_session(agent_id, session_id, "\r") {
            return SubmitOutcome::SessionChanged;
        }

        // 7. 잠긴 동안 도착한 사람 입력 방출. 다른 생산자가 끼어들기 전에
        //    먼저 비워야 하므로 관문 잠금보다 가드를 먼저 내린다.
        drop(holder_guard);
        drop(guard);
        SubmitOutcome::Submitted
    }

    /// 사람 입력 경로가 부른다.
    pub fn note_human(&self, agent_id: &str, data: &str) {
        if data.is_empty() {
            return;
        }
        let gate = self.get_agent_gate(agent_id);
        let mut inner = gate.inner.lock();

        // 한 청크에 Enter와 다음 입력이 함께 올 수 있다. 마지막 미제출
        // 조각을 놓치지 않도록 바이트 순서로 반영한다.
        for byte in data.bytes() {
            match byte {
                b'\r' | b'\n' => {
                    inner.human_uncommitted = false;
                    inner.human_input_epoch = inner.human_input_epoch.wrapping_add(1);
                }
                0x03 | 0x15 | 0x1a => inner.human_uncommitted = false,
                _ if !inner.human_uncommitted => {
                    inner.human_uncommitted = true;
                    inner.uncommitted_since_ms = now_ms();
                }
                _ => {}
            }
        }

        inner.human_input_revision = inner.human_input_revision.wrapping_add(1);

        if inner.holder.is_some() && inner.queue_human_while_held {
            // 관문이 잠겨 있는 동안은 모았다가 CR 뒤에 내보낸다
            inner
                .queued_human
                .push((self.sink.session_id_for(agent_id), data.to_string()));
        } else {
            // holder 검사와 쓰기 사이에 자동 제출이 끼어들지 않게 한다.
            self.sink.write(agent_id, data);
        }
    }

    /// xterm이 PTY의 질의에 답하는 제어 응답 경로다. 이 바이트는 프로그램에는
    /// 즉시 전달해야 하지만, 사람이 편집하거나 자동화를 중단한 것은 아니다.
    pub fn note_terminal_response(&self, agent_id: &str, data: &str) {
        if !data.is_empty() {
            self.sink.write(agent_id, data);
        }
    }

    /// 사람이 제출(CR)한 횟수. 마커 뒤 예약 제출 무효화에 사용.
    pub fn human_input_epoch(&self, agent_id: &str) -> u64 {
        let gate = self.get_agent_gate(agent_id);
        let epoch = gate.inner.lock().human_input_epoch;
        epoch
    }

    /// 사람이 입력한 모든 조각의 revision. strict 전환의 확인 근거에 쓴다.
    pub fn human_input_revision(&self, agent_id: &str) -> u64 {
        let gate = self.get_agent_gate(agent_id);
        let revision = gate.inner.lock().human_input_revision;
        revision
    }

    pub fn forced_submission_count(&self, agent_id: &str) -> u64 {
        let gate = self.get_agent_gate(agent_id);
        let count = gate.inner.lock().forced_submission_count;
        count
    }

    /// 배너에 띄울 보류 이유.
    pub fn pending_reason(&self, agent_id: &str) -> Option<PendingReason> {
        let gate = self.get_agent_gate(agent_id);
        let reason = {
            let inner = gate.inner.lock();
            if inner.holder.is_some() {
                Some(PendingReason::AnotherProducer)
            } else if inner.human_uncommitted {
                let elapsed = now_ms().saturating_sub(inner.uncommitted_since_ms);
                if elapsed < self.human_hold_max_ms {
                    Some(PendingReason::HumanTyping)
                } else {
                    None
                }
            } else {
                None
            }
        };
        reason
    }

    /// 보류가 시작된 시각(epoch ms). 알림 유예 판정 등에 사용.
    pub fn pending_since_ms(&self, agent_id: &str) -> Option<u64> {
        let gate = self.get_agent_gate(agent_id);
        let since = {
            let inner = gate.inner.lock();
            // 다른 생산자가 쥔 보류는 시작 시각을 따로 재지 않는다. 미제출 입력의
            // `uncommitted_since_ms`를 빌려 쓰면 0(=1970년)이 새어 나간다.
            if inner.human_uncommitted {
                let elapsed = now_ms().saturating_sub(inner.uncommitted_since_ms);
                if elapsed < self.human_hold_max_ms {
                    Some(inner.uncommitted_since_ms)
                } else {
                    None
                }
            } else {
                None
            }
        };
        since
    }

    /// 배너의 `계속` 버튼 클릭 시 미제출 플래그만 해제.
    pub fn clear_uncommitted(&self, agent_id: &str) {
        let gate = self.get_agent_gate(agent_id);
        let mut inner = gate.inner.lock();
        inner.human_uncommitted = false;
    }

    /// 결정 패널을 띄운 뒤 새 사람 입력이 없었을 때만 미제출 표식을 해제한다.
    /// 오래된 사용자 응답이 새 입력을 지우지 않게 하는 compare-and-clear 연산이다.
    pub fn clear_uncommitted_if_revision(
        &self,
        agent_id: &str,
        expected_human_input_revision: u64,
    ) -> bool {
        let gate = self.get_agent_gate(agent_id);
        let mut inner = gate.inner.lock();
        if inner.human_input_revision != expected_human_input_revision {
            return false;
        }
        inner.human_uncommitted = false;
        true
    }

    /// 세션 종료 시 상태 정리.
    pub fn remove_agent(&self, agent_id: &str) {
        if let Some(gate) = self.agents.lock().remove(agent_id) {
            let mut inner = gate.inner.lock();
            inner.generation = inner.generation.wrapping_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_response_reaches_pty_without_changing_human_input_state() {
        let sink = Arc::new(RecordingSink::default());
        let gate = InjectGate::new(sink.clone(), Arc::new(BotPromptArms::new()));

        gate.note_terminal_response("a1", "\x1b[12;34R");

        assert_eq!(sink.recorded_writes()[0].1, "\x1b[12;34R");
        assert_eq!(gate.human_input_epoch("a1"), 0);
        assert_eq!(gate.human_input_revision("a1"), 0);
        assert_eq!(gate.pending_reason("a1"), None);
    }

    #[tokio::test]
    async fn session_replaced_during_submit_gets_no_enter_or_old_queue() {
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", true);
        sink.set_session("a1", "s1");
        let gate = Arc::new(InjectGate::new(
            sink.clone(),
            Arc::new(BotPromptArms::new()),
        ));
        let submit_gate = gate.clone();
        let handle = tokio::spawn(async move {
            submit_gate
                .submit("a1", "s1", "prompt", InjectSource::Automation)
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while sink.recorded_writes().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        gate.note_human("a1", "old input");
        sink.set_session("a1", "s2");
        gate.note_human("a1", "new input during old submit");
        assert_eq!(handle.await.unwrap(), SubmitOutcome::SessionChanged);
        let writes = sink.recorded_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0].1, "prompt");
        assert_eq!(writes[1].1, "new input during old submit");
        gate.note_human("a1", "new input");
        assert_eq!(sink.recorded_writes()[2].1, "new input");
    }

    #[tokio::test]
    async fn enter_followed_by_partial_input_in_same_chunk_still_defers() {
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", true);
        sink.set_session("a1", "s1");
        let gate = InjectGate::new(sink.clone(), Arc::new(BotPromptArms::new()));
        gate.note_human("a1", "first\rsecond");
        assert_eq!(gate.human_input_epoch("a1"), 1);
        assert_eq!(gate.pending_reason("a1"), Some(PendingReason::HumanTyping));
        assert_eq!(
            gate.submit("a1", "s1", "auto", InjectSource::Automation)
                .await,
            SubmitOutcome::Deferred(PendingReason::HumanTyping)
        );
        assert_eq!(sink.recorded_writes().len(), 1);
        gate.note_human("a1", "\x15third");
        assert_eq!(gate.pending_reason("a1"), Some(PendingReason::HumanTyping));
        gate.note_human("a1", "\x15");
        assert_eq!(gate.pending_reason("a1"), None);
    }

    #[tokio::test]
    async fn concurrent_submit_one_succeeds_one_deferred() {
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", true);
        sink.set_session("a1", "s1");
        let arms = Arc::new(BotPromptArms::new());
        let gate = Arc::new(InjectGate::new(sink.clone(), arms));

        // 두 비동기 태스크에서 동시에 submit 호출
        let gate1 = gate.clone();
        let gate2 = gate.clone();

        let h1 = tokio::spawn(async move {
            gate1
                .submit("a1", "s1", "cmd1", InjectSource::Automation)
                .await
        });
        let h2 = tokio::spawn(async move {
            // 아주 미세한 지연으로 h1이 먼저 lock을 쥐도록 유도
            tokio::time::sleep(Duration::from_millis(5)).await;
            gate2.submit("a1", "s1", "cmd2", InjectSource::Bot).await
        });

        let r1 = h1.await.unwrap();
        let r2 = h2.await.unwrap();

        assert_eq!(r1, SubmitOutcome::Submitted);
        assert_eq!(r2, SubmitOutcome::Deferred(PendingReason::AnotherProducer));

        let writes = sink.recorded_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0].1, "cmd1");
        assert_eq!(writes[1].1, "\r");
    }

    #[tokio::test]
    async fn session_changed_writes_nothing() {
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", true);
        sink.set_session("a1", "old-session");
        let arms = Arc::new(BotPromptArms::new());
        let gate = InjectGate::new(sink.clone(), arms);

        let res = gate
            .submit("a1", "new-session", "echo hi", InjectSource::Automation)
            .await;
        assert_eq!(res, SubmitOutcome::SessionChanged);
        assert!(sink.recorded_writes().is_empty());
    }

    #[tokio::test]
    async fn not_running_writes_nothing() {
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", false);
        sink.set_session("a1", "s1");
        let arms = Arc::new(BotPromptArms::new());
        let gate = InjectGate::new(sink.clone(), arms);

        let res = gate
            .submit("a1", "s1", "echo hi", InjectSource::Automation)
            .await;
        assert_eq!(res, SubmitOutcome::NotRunning);
        assert!(sink.recorded_writes().is_empty());
    }

    #[tokio::test]
    async fn human_uncommitted_defers_until_clear_or_timeout() {
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", true);
        sink.set_session("a1", "s1");
        let arms = Arc::new(BotPromptArms::new());
        // hold_max = 50ms로 짧게 설정
        let gate = InjectGate::with_hold_max(sink.clone(), arms, 50);

        // 사람이 "hello"를 침 (CR 없음)
        gate.note_human("a1", "hello");
        assert_eq!(gate.pending_reason("a1"), Some(PendingReason::HumanTyping));

        // submit 시도 -> Deferred(HumanTyping)
        let res = gate
            .submit("a1", "s1", "auto-cmd", InjectSource::Automation)
            .await;
        assert_eq!(res, SubmitOutcome::Deferred(PendingReason::HumanTyping));
        // 자동 입력 쓰기 0회 (사람 입력 1회만 존재)
        let writes = sink.recorded_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].1, "hello");

        // 사용자가 배너에서 "계속"을 누름 (clear_uncommitted)
        gate.clear_uncommitted("a1");
        let res2 = gate
            .submit("a1", "s1", "auto-cmd", InjectSource::Automation)
            .await;
        assert_eq!(res2, SubmitOutcome::Submitted);
        let writes2 = sink.recorded_writes();
        assert_eq!(writes2.len(), 3); // hello, auto-cmd, \r

        // 다시 사람이 글자 입력 후 50ms 이상 방치 -> 보류 상한 만료 후 자동 제출됨
        gate.note_human("a1", "typing...");
        tokio::time::sleep(Duration::from_millis(60)).await;
        let res3 = gate
            .submit("a1", "s1", "next-cmd", InjectSource::Automation)
            .await;
        assert_eq!(res3, SubmitOutcome::Submitted);
    }

    #[tokio::test]
    async fn queued_human_input_delivered_after_submit_cr() {
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", true);
        sink.set_session("a1", "s1");
        let arms = Arc::new(BotPromptArms::new());
        let gate = Arc::new(InjectGate::new(sink.clone(), arms));

        let gate_clone = gate.clone();
        let handle = tokio::spawn(async move {
            gate_clone
                .submit("a1", "s1", "long-prompt", InjectSource::Automation)
                .await
        });

        // submit 실행 도중(150ms 대기 구간) 사람이 키를 침
        tokio::time::sleep(Duration::from_millis(30)).await;
        gate.note_human("a1", "human-interruption\r");

        let res = handle.await.unwrap();
        assert_eq!(res, SubmitOutcome::Submitted);

        let writes = sink.recorded_writes();
        // 순서: 1) "long-prompt", 2) "\r", 3) "human-interruption\r"
        assert_eq!(writes.len(), 3);
        assert_eq!(writes[0].1, "long-prompt");
        assert_eq!(writes[1].1, "\r");
        assert_eq!(writes[2].1, "human-interruption\r");
    }

    // submit 퓨처가 150ms 대기 중에 드롭돼도 holder 표식이 남지 않아야 한다.
    // 남으면 그 뒤의 사람 입력이 전부 큐에 갇혀 터미널이 먹통이 된다.
    #[tokio::test]
    async fn dropped_submit_releases_holder_and_flushes_queue() {
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", true);
        sink.set_session("a1", "s1");
        let arms = Arc::new(BotPromptArms::new());
        let gate = Arc::new(InjectGate::new(sink.clone(), arms));

        let gate_clone = gate.clone();
        let handle = tokio::spawn(async move {
            gate_clone
                .submit("a1", "s1", "long-prompt", InjectSource::Automation)
                .await
        });

        // 본문은 나갔고 CR 전 대기 중인 시점에 태스크를 죽인다.
        tokio::time::sleep(Duration::from_millis(30)).await;
        gate.note_human("a1", "x");
        handle.abort();
        let _ = handle.await;

        // 가드가 풀리며 갇혀 있던 "x"가 나간다.
        let writes = sink.recorded_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0].1, "long-prompt");
        assert_eq!(writes[1].1, "x");

        // 이후 입력은 바로 통과하고, 다음 submit도 막히지 않는다.
        gate.note_human("a1", "\r");
        assert_eq!(sink.recorded_writes().len(), 3);
        let res = gate
            .submit("a1", "s1", "next", InjectSource::Automation)
            .await;
        assert_eq!(res, SubmitOutcome::Submitted);
    }

    #[tokio::test]
    async fn transition_owner_defers_other_producers_but_passes_human_input() {
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", true);
        sink.set_session("a1", "s1");
        let gate = InjectGate::new(sink.clone(), Arc::new(BotPromptArms::new()));
        let generation = gate.begin_generation("a1");
        let owner = gate
            .try_acquire_transition_owner("a1", "s1", generation)
            .unwrap();

        gate.note_human("a1", "human\r");
        assert_eq!(sink.recorded_writes()[0].1, "human\r");
        assert_eq!(gate.human_input_revision("a1"), 1);
        assert_eq!(
            gate.submit("a1", "s1", "bot", InjectSource::Bot).await,
            SubmitOutcome::Deferred(PendingReason::AnotherProducer)
        );

        drop(owner);
        assert_eq!(
            gate.submit("a1", "s1", "bot", InjectSource::Bot).await,
            SubmitOutcome::Submitted,
        );
    }

    #[tokio::test]
    async fn strict_never_forces_over_old_human_fragment() {
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", true);
        sink.set_session("a1", "s1");
        let gate = InjectGate::with_hold_max(sink.clone(), Arc::new(BotPromptArms::new()), 1);
        let generation = gate.begin_generation("a1");
        let owner = gate
            .try_acquire_transition_owner("a1", "s1", generation)
            .unwrap();
        gate.note_human("a1", "unfinished");
        tokio::time::sleep(Duration::from_millis(5)).await;
        let cancelled = AtomicBool::new(false);

        assert_eq!(
            gate.submit_strict(
                &owner,
                StrictSubmitRequest {
                    text: "launch",
                    source: InjectSource::Automation,
                    expected_generation: generation,
                    expected_human_input_revision: gate.human_input_revision("a1"),
                    cancelled: &cancelled,
                },
            )
            .await,
            StrictOutcome::Deferred(PendingReason::HumanTyping),
        );
        assert_eq!(sink.recorded_writes().len(), 1);
    }

    #[tokio::test]
    async fn strict_human_input_between_body_and_cr_is_partial_outcome_and_is_flushed() {
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", true);
        sink.set_session("a1", "s1");
        let gate = Arc::new(InjectGate::new(
            sink.clone(),
            Arc::new(BotPromptArms::new()),
        ));
        let generation = gate.begin_generation("a1");
        let owner = gate
            .try_acquire_transition_owner("a1", "s1", generation)
            .unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let submit_gate = gate.clone();
        let submit_cancelled = cancelled.clone();
        let handle = tokio::spawn(async move {
            submit_gate
                .submit_strict(
                    &owner,
                    StrictSubmitRequest {
                        text: "launch",
                        source: InjectSource::Automation,
                        expected_generation: generation,
                        expected_human_input_revision: 0,
                        cancelled: &submit_cancelled,
                    },
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while sink.recorded_writes().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        gate.note_human("a1", "x");
        assert_eq!(
            handle.await.unwrap(),
            StrictOutcome::HumanInputChangedBeforeCr
        );
        let writes = sink.recorded_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0].1, "launch");
        assert_eq!(writes[1].1, "x");
    }

    #[test]
    fn clear_uncommitted_requires_matching_human_revision() {
        let sink = Arc::new(RecordingSink::default());
        let gate = InjectGate::new(sink, Arc::new(BotPromptArms::new()));
        gate.note_human("a1", "first");
        let revision = gate.human_input_revision("a1");
        gate.note_human("a1", "second");
        assert!(!gate.clear_uncommitted_if_revision("a1", revision));
        assert_eq!(gate.pending_reason("a1"), Some(PendingReason::HumanTyping));
        assert!(gate.clear_uncommitted_if_revision("a1", revision + 1));
        assert_eq!(gate.pending_reason("a1"), None);
    }

    #[test]
    fn ctrl_c_and_ctrl_u_clear_uncommitted() {
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a1", true);
        let arms = Arc::new(BotPromptArms::new());
        let gate = InjectGate::new(sink, arms);

        gate.note_human("a1", "partial");
        assert_eq!(gate.pending_reason("a1"), Some(PendingReason::HumanTyping));

        // Ctrl-C
        gate.note_human("a1", "\x03");
        assert_eq!(gate.pending_reason("a1"), None);
        assert_eq!(gate.human_input_epoch("a1"), 0); // Ctrl-C는 epoch 안 올림

        gate.note_human("a1", "partial2");
        assert_eq!(gate.pending_reason("a1"), Some(PendingReason::HumanTyping));

        // Ctrl-U
        gate.note_human("a1", "\x15");
        assert_eq!(gate.pending_reason("a1"), None);
        assert_eq!(gate.human_input_epoch("a1"), 0);

        // CR은 epoch 올림
        gate.note_human("a1", "cmd\r");
        assert_eq!(gate.pending_reason("a1"), None);
        assert_eq!(gate.human_input_epoch("a1"), 1);
    }

    #[test]
    fn every_human_input_kind_advances_strict_revision() {
        let sink = Arc::new(RecordingSink::default());
        let gate = InjectGate::new(sink, Arc::new(BotPromptArms::new()));
        for (index, input) in ["typed", "\x03", "\x15", "\x1a", "\r"].iter().enumerate() {
            gate.note_human("a1", input);
            assert_eq!(gate.human_input_revision("a1"), (index + 1) as u64);
        }
    }
    #[tokio::test]
    async fn removed_agent_invalidates_an_outstanding_transition_owner() {
        let sink = Arc::new(RecordingSink::default());
        sink.set_running("a", true);
        sink.set_session("a", "s");
        let gate = InjectGate::new(sink.clone(), Arc::new(BotPromptArms::new()));
        let generation = gate.begin_generation("a");
        let owner = gate
            .try_acquire_transition_owner("a", "s", generation)
            .unwrap();
        gate.remove_agent("a");
        let cancelled = AtomicBool::new(false);
        assert_eq!(
            gate.submit_strict(
                &owner,
                StrictSubmitRequest {
                    text: "old launch",
                    source: InjectSource::Automation,
                    expected_generation: generation,
                    expected_human_input_revision: 0,
                    cancelled: &cancelled,
                }
            )
            .await,
            StrictOutcome::GenerationChangedBeforeBody
        );
        assert!(sink.recorded_writes().is_empty());
    }
}
