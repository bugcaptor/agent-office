// src-tauri/src/chat_queue.rs
//
// 대화 입력과 보낼 메시지 대기열. 터미널 탭 "대화 보기"와 웹 상태판이 함께 쓴다.
//
// CLI가 턴을 마치고 입력을 기다리면(턴 관측 Idle) 바로 같은 세션에 넣고, 작업 중·사람 대기면
// 대기열에 쌓아 두었다가 턴이 끝날 때마다 하나씩 넣는다. 예외: Claude가 작업 중이고 답을 기다리는
// 권한 요청·질문이 없으면 작업 중에도 바로 넣는다(Claude Code가 다음 도구 호출 뒤에 반영한다).
// 다른 CLI는 작업 중 Enter를 다르게 다루고, 대화상자가 떠 있으면 글이 거기 들어가므로 기다린다. 넣는 길은 업무 후속 지시 관문
// (`WorkSendRuntime::follow_up`)이라 사람이 쓰다 만 입력·바뀐 세션·셸로 돌아온 CLI에는 넣지 않는다.
// 대기열은 세션에 묶인다 — 세션이 바뀌면(재시작·새 세션) 옛 대화를 향한 메시지는 버리고 알린다.
// 앱을 끄면 대기열도 사라진다(메모리에만 둔다).
//
// 작업 중 넣은 글은 Claude 자기 대기열에 들어가 다음 도구 호출 뒤에 반영된다. 그 사이 Esc가 오면 Claude는
// 턴을 끊지 않고 대기 글을 입력창으로 되돌린다(전사 `queue-operation` `popAll`) — 그대로 두면 끝내
// 제출되지 않는다. 그래서 작업 중 넣은 글은 전사를 지켜보다 되돌려졌으면 다시 제출한다
// (`watch_steered`): 웹·앱에서 작업 중단을 누른 직후면 "지금 보내기"(턴을 끊고 바로 보냄), 아니면 Enter
// (다시 Claude 대기열로). 사람이 터미널에서 직접 되돌렸으면(사람 입력이 있었다) 건드리지 않고 알린다.
//
// "중단하고 바로 보내기"(`send_now`): 대기열을 건너뛰고 지금 넣는다. Claude는 글을 붙여 넣고 Claude Code의
// 지금 보내기 키(`chat:sendNow`, 기본 Ctrl-X Ctrl-S)를 눌러 도는 턴을 끊고 바로 보낸다. 다른 CLI는 Esc로
// 끊고 잠시 뒤 넣는다. 권한 요청·질문이 떠 있으면 거절한다(글이 대화상자로 들어간다).
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;

use crate::control::turns::{TurnObservation, TurnPhase};
use crate::control::ControlContext;
use crate::types::WorkFollowUpRequest;

/// 캐릭터마다 쌓아 둘 수 있는 메시지 수.
pub const MAX_QUEUED: usize = 20;
/// 메시지 하나의 바이트 상한. 웹 상태판이 받는 글(업무 본문·토의 답·권한 답)도 같다.
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
/// 넣은 뒤 턴 관측이 따라오지 않아도(슬래시 명령 등) 이만큼 지나면 다음을 넣는다.
const SENT_FALLBACK_MS: u64 = 60_000;
/// 넣기가 일시적으로 막혔을 때(사람 입력·다른 보내기) 다시 해 보기까지.
const RETRY_MS: u64 = 3_000;
const PUMP_INTERVAL: Duration = Duration::from_secs(1);
/// 화면에 보일 "작업 중에도 바로 들어감" 표시용 포그라운드 CLI 조회를 다시 하기 전 간격.
const CLI_HINT_TTL_MS: u64 = 2_000;
/// Claude Code "지금 보내기"(`chat:sendNow`) 기본 키 Ctrl-X Ctrl-S: 입력창 글을 넣고 도는 턴을 끊어 바로 보낸다.
const CLAUDE_SEND_NOW: &str = "\x18\x13";
/// Claude 아닌 CLI를 Esc로 끊은 뒤 입력창이 돌아오기를 기다리는 시간.
const INTERRUPT_SETTLE: Duration = Duration::from_millis(1_000);
/// 작업 중 넣은 글을 전사에서 지켜보는 간격과 최대 시간.
const STEER_POLL_MS: u64 = 2_000;
const STEER_WATCH_MS: u64 = 30 * 60_000;
/// 작업 중단(웹·앱 Esc) 뒤 이 안에 Claude가 대기 글을 입력창으로 되돌렸으면 턴을 끊고 바로 보낸다.
const INTERRUPT_WINDOW_MS: u64 = 15_000;
/// 전사 끝에서 읽을 바이트.
const TRANSCRIPT_TAIL_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedMessage {
    pub id: String,
    pub text: String,
    /// 쌓은 시각(유닉스 ms).
    pub at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SendOutcome {
    /// 지금 넣었다.
    Sent,
    /// 대기열에 쌓았다. 턴이 끝나면 넣는다.
    Queued,
}

/// 작업 중 넣어 Claude 대기열에 있는 글.
#[derive(Debug, Clone)]
struct Steered {
    text: String,
    /// 넣기 시작한 시각(유닉스 ms).
    at: u64,
    /// 넣을 때의 사람 입력 revision. 바뀌었으면 그 뒤 사람이 터미널을 만졌다.
    revision: u64,
}

#[derive(Default)]
struct AgentQueue {
    session_id: String,
    items: VecDeque<QueuedMessage>,
    /// 마지막으로 넣은 시각. 그 뒤의 턴 관측이 와야(작업 → 턴 끝) 다음을 넣는다.
    sent_at: Option<u64>,
    /// 지금 넣는 중(바로 보내기와 펌프가 겹치지 않게).
    delivering: bool,
    retry_at: u64,
    /// 마지막으로 넣지 못한 까닭(오류 코드). 넣으면 지운다.
    error: Option<String>,
    /// 작업 중 넣어 Claude가 아직 가져가지 않았을 수 있는 글(`watch_steered`).
    steered: Vec<Steered>,
    steer_checked_at: u64,
    /// 이미 처리한 전사 `popAll`의 마지막 시각.
    popped_seen: u64,
    /// 웹·앱에서 작업 중단(Esc)을 보낸 시각.
    interrupt_at: Option<u64>,
}

/// 화면에 보일 이 캐릭터의 대기열.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueView {
    pub items: Vec<QueuedMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 작업 중이지만 지금 보내면 쌓지 않고 바로 넣는다(Claude, 기다리는 권한 요청·질문 없음).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub steer: bool,
}

#[derive(Default)]
pub struct ChatQueue {
    agents: Mutex<HashMap<String, AgentQueue>>,
    seq: AtomicU64,
    /// 캐릭터 → (조회한 때, 포그라운드가 Claude인가). 화면 표시용 캐시다.
    claude_fg: Mutex<HashMap<String, (u64, bool)>>,
    probing: Mutex<HashSet<String>>,
}

/// 보내기 전 글 검사: 빈 글을 거부하고 `check_limits`를 본다.
pub fn check_text(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("empty".into());
    }
    check_limits(text)
}

/// 길이·제어 문자 검사(빈 글은 허용). 웹 상태판이 받는 글도 이 규칙을 쓴다.
pub fn check_limits(text: &str) -> Result<(), String> {
    if text.len() > MAX_TEXT_BYTES {
        return Err("too-large".into());
    }
    if text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err("control-char".into());
    }
    Ok(())
}

/// 지금 넣어도 되는가. 넣은 뒤로는 새 턴 관측(작업 → 끝)이 와야 다음 차례다.
fn ready(turn: Option<&TurnObservation>, sent_at: Option<u64>, now: u64) -> bool {
    let Some(turn) = turn.filter(|t| t.phase == TurnPhase::Idle) else {
        return false;
    };
    match sent_at {
        None => true,
        Some(sent) => turn.at > sent || now.saturating_sub(sent) >= SENT_FALLBACK_MS,
    }
}

/// 작업 중에도 바로 넣어도 되는가. Claude Code만 작업 중 입력을 받아 다음 도구 호출 뒤에 반영하고,
/// 권한 요청·질문 대화상자가 떠 있으면 친 글이 그 대화상자로 들어가므로 넣지 않는다.
fn steerable(turn: Option<&TurnObservation>, claude: bool, asks_pending: bool) -> bool {
    claude && !asks_pending && turn.is_some_and(|t| t.phase == TurnPhase::Working)
}

/// 터미널 포그라운드가 Claude인가(블로킹 — 프로세스 명령줄을 읽는다). Windows는 알 수 없어 false다.
fn claude_in_foreground(manager: &crate::session::manager::SessionManager, agent_id: &str) -> bool {
    crate::webdash::foreground_cli(manager, agent_id, manager.foreground_command(agent_id).as_deref()) == Some("claude")
}

/// 넣기 실패가 잠깐 기다리면 풀리는 것인가(아니면 사람이 볼 때까지 붙들고 있는다).
fn transient(error: &str) -> bool {
    matches!(error, "work-send-busy" | "work-send-human-input" | "busy")
}

impl ChatQueue {
    /// 화면에 보일 대기열. 작업 중이면 바로 넣을 수 있는지(`steer`)도 싣는다.
    pub fn view(&self, ctx: &ControlContext, agent_id: &str, session_id: Option<&str>) -> QueueView {
        let mut view = self.items_view(agent_id, session_id);
        view.steer = session_id.is_some_and(|sid| {
            let turn = ctx.turns.get(sid);
            // 작업 중일 때만 포그라운드를 본다(짧은 순서로 ps를 아낀다).
            turn.as_ref().is_some_and(|t| t.phase == TurnPhase::Working)
                && steerable(turn.as_ref(), self.claude_hint(ctx, agent_id), !ctx.observer.permissions().list(sid).is_empty())
        });
        view
    }

    fn items_view(&self, agent_id: &str, session_id: Option<&str>) -> QueueView {
        let agents = self.agents.lock().unwrap();
        let Some(q) = agents.get(agent_id) else {
            return QueueView::default();
        };
        // 세션이 바뀌었으면 펌프가 곧 비운다. 그 전에도 옛 메시지는 보이지 않는다.
        let same = session_id.is_some_and(|sid| sid == q.session_id);
        QueueView {
            items: if same { q.items.iter().cloned().collect() } else { Vec::new() },
            error: q.error.clone(),
            steer: false,
        }
    }

    /// 캐시한 "포그라운드가 Claude인가". 오래됐으면 뒤에서 다시 본다(화면 조회를 막지 않는다).
    fn claude_hint(&self, ctx: &ControlContext, agent_id: &str) -> bool {
        let now = crate::types::now_ms();
        let cached = self.claude_fg.lock().unwrap().get(agent_id).copied();
        let stale = cached.is_none_or(|(at, _)| now.saturating_sub(at) >= CLI_HINT_TTL_MS);
        if stale && self.probing.lock().unwrap().insert(agent_id.to_string()) {
            let (me, manager, agent) = (ctx.chat_queue.clone(), ctx.manager.clone(), agent_id.to_string());
            std::thread::spawn(move || {
                let claude = claude_in_foreground(&manager, &agent);
                me.claude_fg.lock().unwrap().insert(agent.clone(), (crate::types::now_ms(), claude));
                me.probing.lock().unwrap().remove(&agent);
            });
        }
        cached.is_some_and(|(_, claude)| claude)
    }

    /// 지금 작업 중에도 바로 넣어도 되는가(포그라운드를 새로 본다).
    async fn steer_now(&self, ctx: &ControlContext, agent_id: &str, session_id: &str) -> bool {
        let turn = ctx.turns.get(session_id);
        // 터미널에서 답한 것을 출력으로만 짐작했으면 대화상자가 아직 떠 있을 수 있다. 끼워 넣은
        // 글이 선택지 키로 읽히지 않게, 훅이 확인할 때까지는 쌓아 둔다.
        if ctx.hub.answer_inferred(session_id) {
            return false;
        }
        if !steerable(turn.as_ref(), true, !ctx.observer.permissions().list(session_id).is_empty()) {
            return false;
        }
        let (manager, agent) = (ctx.manager.clone(), agent_id.to_string());
        let claude = tokio::task::spawn_blocking(move || claude_in_foreground(&manager, &agent)).await.unwrap_or(false);
        self.claude_fg.lock().unwrap().insert(agent_id.to_string(), (crate::types::now_ms(), claude));
        claude
    }

    /// 쌓아 둔 메시지 하나를 뺀다. 이미 넣었거나 없으면 false.
    pub fn cancel(&self, agent_id: &str, id: &str) -> bool {
        let mut agents = self.agents.lock().unwrap();
        let Some(q) = agents.get_mut(agent_id) else {
            return false;
        };
        let before = q.items.len();
        q.items.retain(|m| m.id != id);
        if q.items.is_empty() {
            q.error = None;
        }
        q.items.len() != before
    }

    /// 입력 대기면 바로 넣고, 작업 중이면 쌓는다. 앞서 쌓인 것이 있으면 순서를 지켜 뒤에 쌓는다.
    pub async fn send(&self, ctx: &Arc<ControlContext>, agent_id: &str, text: &str) -> Result<SendOutcome, String> {
        check_text(text)?;
        let session_id = running_session(ctx, agent_id)?;
        let turn = ctx.turns.get(&session_id);
        let now = crate::types::now_ms();
        let mut steer = false;
        let deliver_now = match turn.as_ref().map(|t| t.phase) {
            Some(TurnPhase::Idle) => true,
            Some(TurnPhase::Working) => {
                steer = self.steer_now(ctx, agent_id, &session_id).await;
                steer
            }
            Some(TurnPhase::NeedsHuman) => false,
            Some(TurnPhase::Ended) => return Err("not-running".into()),
            // 막 띄운 CLI는 아직 턴 관측이 없다. 포그라운드가 CLI면 입력 대기로 본다.
            None => {
                if foreground_cli_now(ctx, agent_id).await.is_none() {
                    return Err("chat-view-no-cli".into());
                }
                true
            }
        };
        {
            let mut agents = self.agents.lock().unwrap();
            let q = agents.entry(agent_id.to_string()).or_default();
            if q.session_id != session_id {
                *q = AgentQueue { session_id: session_id.clone(), ..Default::default() };
            }
            let idle_known = steer || turn.is_none() || ready(turn.as_ref(), q.sent_at, now);
            if !(deliver_now && idle_known && q.items.is_empty() && !q.delivering) {
                if q.items.len() >= MAX_QUEUED {
                    return Err("queue-full".into());
                }
                let id = format!("q{}", self.seq.fetch_add(1, Ordering::Relaxed) + 1);
                q.items.push_back(QueuedMessage { id, text: text.to_string(), at: now });
                return Ok(SendOutcome::Queued);
            }
            q.delivering = true;
        }
        let how = if steer { Delivery::Steer } else { Delivery::Enter };
        deliver_detached(ctx, agent_id, &session_id, text, how).await.map(|_| SendOutcome::Sent)
    }

    /// 대기열을 건너뛰고 지금 넣는다. 작업 중이면 도는 턴을 끊고 바로 보낸다(Claude는 지금 보내기 키,
    /// 다른 CLI는 Esc 뒤). 권한 요청·질문·사람 대기 중이면 거절한다 — 글이 대화상자로 들어간다.
    pub async fn send_now(&self, ctx: &Arc<ControlContext>, agent_id: &str, text: &str) -> Result<SendOutcome, String> {
        check_text(text)?;
        let session_id = running_session(ctx, agent_id)?;
        if !ctx.observer.permissions().list(&session_id).is_empty() || ctx.hub.answer_inferred(&session_id) {
            return Err("send-now-ask-pending".into());
        }
        // 끊는 키(Esc)를 보내기 전에 막는다 — 넣기 관문도 쓰다 만 사람 입력 위에는 넣지 않는다.
        if ctx.gate.has_uncommitted(agent_id) {
            return Err("work-send-human-input".into());
        }
        let phase = ctx.turns.get(&session_id).map(|t| t.phase);
        let how = match phase {
            Some(TurnPhase::Ended) => return Err("not-running".into()),
            Some(TurnPhase::NeedsHuman) => return Err("send-now-ask-pending".into()),
            Some(TurnPhase::Idle) => Delivery::Enter,
            // 포그라운드를 알 수 없으면(Windows 등) Esc로 끊고 넣는다 — Claude도 그렇게 끊긴다.
            Some(TurnPhase::Working) => match foreground_cli_now(ctx, agent_id).await {
                Some("claude") => Delivery::ClaudeSendNow,
                _ => Delivery::EscThenEnter,
            },
            None => {
                if foreground_cli_now(ctx, agent_id).await.is_none() {
                    return Err("chat-view-no-cli".into());
                }
                Delivery::Enter
            }
        };
        {
            let mut agents = self.agents.lock().unwrap();
            let q = agents.entry(agent_id.to_string()).or_default();
            if q.session_id != session_id {
                *q = AgentQueue { session_id: session_id.clone(), ..Default::default() };
            }
            if q.delivering {
                return Err("work-send-busy".into());
            }
            q.delivering = true;
        }
        deliver_detached(ctx, agent_id, &session_id, text, how).await.map(|_| SendOutcome::Sent)
    }

    /// 웹·앱에서 작업 중단(Esc)을 보냈다. 곧 Claude가 대기 글을 입력창으로 되돌리면 끊고 바로 보낸다.
    pub fn note_interrupt(&self, agent_id: &str) {
        let mut agents = self.agents.lock().unwrap();
        if let Some(q) = agents.get_mut(agent_id) {
            q.interrupt_at = Some(crate::types::now_ms());
        }
    }

    /// 넣기 하나가 끝났다. 작업 중 Claude 대기열로 넣었으면 지켜볼 목록에 단다.
    fn delivered(&self, agent_id: &str, session_id: &str, steered: Option<Steered>, result: &Result<(), String>) {
        let mut agents = self.agents.lock().unwrap();
        let Some(q) = agents.get_mut(agent_id) else { return };
        q.delivering = false;
        if result.is_ok() {
            q.sent_at = Some(crate::types::now_ms());
            q.error = None;
            if let Some(s) = steered.filter(|_| q.session_id == session_id) {
                q.steered.push(s);
            }
        }
    }

    /// 쌓인 메시지를 넣을 차례인 캐릭터마다 하나씩 넣는다.
    pub async fn pump_once(&self, ctx: &ControlContext) {
        let now = crate::types::now_ms();
        // 작업 중인 Claude에는 턴 끝을 기다리지 않고 넣는다. 포그라운드 조회는 잠금 밖에서 한다.
        let working: Vec<(String, String)> = {
            let agents = self.agents.lock().unwrap();
            agents
                .iter()
                .filter(|(_, q)| !q.items.is_empty() && !q.delivering && now >= q.retry_at)
                .filter(|(_, q)| ctx.turns.get(&q.session_id).is_some_and(|t| t.phase == TurnPhase::Working))
                .map(|(agent, q)| (agent.clone(), q.session_id.clone()))
                .collect()
        };
        let mut steer = HashSet::new();
        for (agent, sid) in working {
            if self.steer_now(ctx, &agent, &sid).await {
                steer.insert(agent);
            }
        }
        let mut due: Vec<(String, String, QueuedMessage)> = Vec::new();
        {
            let mut agents = self.agents.lock().unwrap();
            for (agent_id, q) in agents.iter_mut() {
                if q.items.is_empty() || q.delivering || now < q.retry_at {
                    continue;
                }
                let live = ctx.manager.session_id_for(agent_id).filter(|_| ctx.manager.is_running(agent_id));
                match live {
                    Some(sid) if sid == q.session_id => {}
                    Some(_) => {
                        // 새 세션이다. 옛 대화를 향한 메시지는 넣지 않는다.
                        q.items.clear();
                        q.sent_at = None;
                        q.error = Some("queue-dropped-session".into());
                        continue;
                    }
                    None => {
                        q.error = Some("not-running".into());
                        continue;
                    }
                }
                let turn = ctx.turns.get(&q.session_id);
                if turn.as_ref().is_some_and(|t| t.phase == TurnPhase::Ended) {
                    q.items.clear();
                    q.error = Some("queue-dropped-session".into());
                    continue;
                }
                if !ready(turn.as_ref(), q.sent_at, now) && !steer.contains(agent_id) {
                    continue;
                }
                let Some(msg) = q.items.pop_front() else { continue };
                q.delivering = true;
                due.push((agent_id.clone(), q.session_id.clone(), msg));
            }
        }
        for (agent_id, session_id, msg) in due {
            let steered = steer.contains(&agent_id).then(|| Steered {
                text: msg.text.clone(),
                at: crate::types::now_ms(),
                revision: ctx.gate.human_input_revision(&agent_id),
            });
            let result = deliver(ctx, &agent_id, &session_id, &msg.text, "\r").await;
            if result.is_ok() {
                self.delivered(&agent_id, &session_id, steered, &result);
                continue;
            }
            let mut agents = self.agents.lock().unwrap();
            let Some(q) = agents.get_mut(&agent_id) else { continue };
            q.delivering = false;
            if let Err(e) = result {
                // 넣지 못한 메시지는 맨 앞에 되돌린다. 일시적이면 잠시 뒤, 아니면 다음 턴 끝에 다시 해 본다.
                if q.session_id == session_id {
                    q.items.push_front(msg);
                }
                q.retry_at = crate::types::now_ms() + if transient(&e) { RETRY_MS } else { RETRY_MS * 5 };
                q.error = Some(e);
            }
        }
        self.watch_steered(ctx).await;
    }

    /// 작업 중 넣은 글이 Claude 대기열에서 입력창으로 되돌려졌는지(전사 `popAll`) 보고, 그랬으면 다시 제출한다.
    async fn watch_steered(&self, ctx: &ControlContext) {
        let now = crate::types::now_ms();
        let targets: Vec<(String, String)> = {
            let mut agents = self.agents.lock().unwrap();
            agents
                .iter_mut()
                .filter_map(|(agent, q)| {
                    q.steered.retain(|s| now.saturating_sub(s.at) < STEER_WATCH_MS);
                    if q.steered.is_empty() || now.saturating_sub(q.steer_checked_at) < STEER_POLL_MS {
                        return None;
                    }
                    q.steer_checked_at = now;
                    Some((agent.clone(), q.session_id.clone()))
                })
                .collect()
        };
        for (agent_id, session_id) in targets {
            let Some(path) = ctx.agent_session_lookup.latest_session(&agent_id).and_then(|s| s.transcript_path) else {
                continue;
            };
            let Ok(Some(tail)) = tokio::task::spawn_blocking(move || read_tail(&path)).await else { continue };
            let ops = queue_ops(&tail);
            let turn = ctx.turns.get(&session_id);
            let live = ctx.manager.session_id_for(&agent_id).filter(|_| ctx.manager.is_running(&agent_id));
            let (human_revision, uncommitted) = (ctx.gate.human_input_revision(&agent_id), ctx.gate.has_uncommitted(&agent_id));
            let press = {
                let mut agents = self.agents.lock().unwrap();
                let Some(q) = agents.get_mut(&agent_id) else { continue };
                if q.session_id != session_id || live.as_deref() != Some(session_id.as_str()) {
                    q.steered.clear();
                    continue;
                }
                let verdict = judge_steered(&q.steered, &ops, q.popped_seen, now, turn.as_ref());
                q.steered.retain(|s| !verdict.settled.iter().any(|t| t == &s.text));
                let Some(pop) = verdict.popped else { continue };
                q.popped_seen = pop.at;
                q.steered.retain(|s| !pop.texts.contains(&s.text));
                // 사람이 그 뒤 터미널을 만졌으면 직접 되돌린 것이다(Claude: Esc로 대기 글 고치기). 두고 알린다.
                let human = uncommitted || pop.revisions.iter().any(|r| *r != human_revision);
                if human || q.delivering {
                    q.error = Some("queue-returned-to-input".into());
                    continue;
                }
                let interrupted = q.interrupt_at.is_some_and(|t| t >= pop.first_at && now.saturating_sub(t) < INTERRUPT_WINDOW_MS);
                if interrupted {
                    q.interrupt_at = None;
                } else {
                    // Enter는 작업 중이면 다시 Claude 대기열로, 턴이 끝났으면 바로 제출이다. 다시 지켜본다.
                    q.steered.extend(pop.texts.iter().map(|text| Steered { text: text.clone(), at: now, revision: human_revision }));
                }
                Some(if interrupted { CLAUDE_SEND_NOW } else { "\r" })
            };
            if let Some(keys) = press {
                if !ctx.gate.sink().write_for_session(&agent_id, &session_id, keys) {
                    continue;
                }
                eprintln!("agent-office: [chat-queue] {agent_id} resubmitted a queued message Claude put back into the input");
            }
        }
    }

    pub fn spawn(me: Arc<ChatQueue>, ctx: Arc<ControlContext>) {
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(PUMP_INTERVAL).await;
                me.pump_once(&ctx).await;
            }
        });
    }
}

/// 넣는 방법.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delivery {
    /// 붙여 넣고 Enter.
    Enter,
    /// 작업 중인 Claude에 Enter — Claude 대기열로 들어가 다음 도구 호출 뒤 반영된다. 지켜본다.
    Steer,
    /// 붙여 넣고 Claude 지금 보내기 키 — 도는 턴을 끊고 바로 보낸다.
    ClaudeSendNow,
    /// Esc로 끊고 잠시 뒤 붙여 넣고 Enter(Claude 아닌 CLI).
    EscThenEnter,
}

/// 넣기를 따로 띄운 일로 한다 — 웹 요청이 끊겨 이 future가 버려져도 붙여 넣기와 Enter 사이에서 멈추거나
/// `delivering`이 남아 대기열이 영영 서지 않게. 부르기 전에 `delivering`을 세워 둔다.
async fn deliver_detached(
    ctx: &Arc<ControlContext>,
    agent_id: &str,
    session_id: &str,
    text: &str,
    how: Delivery,
) -> Result<(), String> {
    let (ctx, agent, sid, text) = (ctx.clone(), agent_id.to_string(), session_id.to_string(), text.to_string());
    let task = tauri::async_runtime::spawn(async move {
        let steered = (how == Delivery::Steer).then(|| Steered {
            text: text.clone(),
            at: crate::types::now_ms(),
            revision: ctx.gate.human_input_revision(&agent),
        });
        if how == Delivery::EscThenEnter {
            ctx.gate.sink().write(&agent, "\x1b");
            tokio::time::sleep(INTERRUPT_SETTLE).await;
        }
        let submit = if how == Delivery::ClaudeSendNow { CLAUDE_SEND_NOW } else { "\r" };
        let result = deliver(&ctx, &agent, &sid, &text, submit).await;
        ctx.chat_queue.delivered(&agent, &sid, steered, &result);
        result
    });
    task.await.unwrap_or_else(|_| Err("busy".into()))
}

async fn deliver(ctx: &ControlContext, agent_id: &str, session_id: &str, text: &str, submit: &str) -> Result<(), String> {
    ctx.work_send
        .follow_up_with(
            &ctx.gate,
            WorkFollowUpRequest { agent_id: agent_id.to_string(), session_id: session_id.to_string(), text: text.to_string() },
            submit,
        )
        .await
        .map(|_| ())
}

/// 돌고 있는 세션 id.
fn running_session(ctx: &ControlContext, agent_id: &str) -> Result<String, String> {
    ctx.manager
        .session_id_for(agent_id)
        .filter(|_| ctx.manager.is_running(agent_id))
        .ok_or_else(|| "not-running".into())
}

/// 터미널 포그라운드의 CLI 이름(블로킹 조회를 따로 돌린다).
async fn foreground_cli_now(ctx: &ControlContext, agent_id: &str) -> Option<&'static str> {
    let (manager, agent) = (ctx.manager.clone(), agent_id.to_string());
    tokio::task::spawn_blocking(move || crate::webdash::foreground_cli(&manager, &agent, manager.foreground_command(&agent).as_deref()))
        .await
        .ok()
        .flatten()
}

/// 전사의 Claude 대기열 기록 하나.
#[derive(Debug, Clone, PartialEq)]
struct QueueOp {
    op: String,
    /// 유닉스 ms.
    at: u64,
    content: String,
}

/// 전사 꼬리에서 `queue-operation` 항목들(시각 순).
fn queue_ops(tail: &str) -> Vec<QueueOp> {
    tail.lines()
        .filter(|l| l.contains("\"queue-operation\""))
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("queue-operation"))
        .filter_map(|v| {
            let at = chrono::DateTime::parse_from_rfc3339(v.get("timestamp")?.as_str()?).ok()?.timestamp_millis();
            Some(QueueOp {
                op: v.get("operation")?.as_str()?.to_string(),
                at: u64::try_from(at).ok()?,
                content: v.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// 되돌려진 글 묶음(`popAll` 하나).
#[derive(Debug, Default, PartialEq)]
struct Popped {
    at: u64,
    /// 우리가 넣은 것 중 가장 이른 시각.
    first_at: u64,
    texts: Vec<String>,
    revisions: Vec<u64>,
}

#[derive(Debug, Default, PartialEq)]
struct SteerVerdict {
    /// 더 지켜볼 필요 없는 글(Claude가 가져갔거나 턴이 끝났다).
    settled: Vec<String>,
    popped: Option<Popped>,
}

/// 전사 시각과 앱 시각은 같은 머신이지만 쓰는 순서가 어긋날 수 있어 이만큼 앞까지 본다.
const CLOCK_SLACK_MS: u64 = 2_000;

/// 지켜보는 글들의 운명: Claude가 가져갔는가(`remove`), 입력창으로 되돌렸는가(`popAll`, 아직 처리 안 한 것).
fn judge_steered(steered: &[Steered], ops: &[QueueOp], popped_seen: u64, now: u64, turn: Option<&TurnObservation>) -> SteerVerdict {
    let mut verdict = SteerVerdict::default();
    let after = |s: &Steered, op: &QueueOp| op.at + CLOCK_SLACK_MS >= s.at;
    for s in steered {
        let text = s.text.trim();
        let taken = ops.iter().any(|op| op.op == "remove" && after(s, op) && op.content.trim() == text);
        // 턴이 끝난 뒤 한참 지났으면 대기 글은 다음 프롬프트로 들어갔다(`dequeue`는 내용을 싣지 않는다).
        let turn_over = turn.is_some_and(|t| t.phase == TurnPhase::Idle && t.at > s.at && now.saturating_sub(t.at) > STEER_POLL_MS * 2);
        if taken || turn_over {
            verdict.settled.push(s.text.clone());
        }
    }
    let pop = ops.iter().find(|op| {
        op.op == "popAll"
            && op.at > popped_seen
            && steered.iter().any(|s| after(s, op) && !verdict.settled.contains(&s.text) && op.content.contains(s.text.trim()))
    });
    if let Some(op) = pop {
        let hit: Vec<&Steered> = steered.iter().filter(|s| after(s, op) && op.content.contains(s.text.trim())).collect();
        verdict.popped = Some(Popped {
            at: op.at,
            first_at: hit.iter().map(|s| s.at).min().unwrap_or(op.at),
            texts: hit.iter().map(|s| s.text.clone()).collect(),
            revisions: hit.iter().map(|s| s.revision).collect(),
        });
        verdict.settled.retain(|t| !hit.iter().any(|s| &s.text == t));
    }
    verdict
}

/// 파일 끝 `TRANSCRIPT_TAIL_BYTES`(첫 줄은 잘렸을 수 있어 버린다).
fn read_tail(path: &str) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(TRANSCRIPT_TAIL_BYTES);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    f.read_to_end(&mut buf).ok()?;
    let mut text = String::from_utf8_lossy(&buf).into_owned();
    if start > 0 {
        let cut = text.find('\n').map_or(text.len(), |i| i + 1);
        text.drain(..cut);
    }
    Some(text)
}

/// 지금 에이전트가 무엇을 하는가 — "생각 중"과 "멈춤"을 가르는 재료. 판정은 화면이 한다
/// (턴이 작업 중인데 PTY 출력이 오래 끊겼으면 멈췄을 수 있다).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveStatus {
    /// 세션이 돌고 있는가.
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<TurnPhase>,
    /// 이 상태가 시작된 시각(작업 중이면 턴 시작).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    /// 마지막 PTY 출력 시각.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_at: Option<u64>,
    /// 서버 시각(기기 시계 차이 없이 경과 시간을 잰다).
    pub now: u64,
    pub queue: QueueView,
}

pub fn live_status(ctx: &ControlContext, agent_id: &str) -> LiveStatus {
    let session_id = ctx.manager.session_id_for(agent_id).filter(|_| ctx.manager.is_running(agent_id));
    let turn = session_id.as_deref().and_then(|sid| ctx.turns.get(sid));
    LiveStatus {
        running: session_id.is_some(),
        phase: turn.as_ref().map(|t| t.phase),
        since: turn.as_ref().map(|t| t.since),
        output_at: ctx.manager.last_output_ms(agent_id),
        now: crate::types::now_ms(),
        queue: ctx.chat_queue.view(ctx, agent_id, session_id.as_deref()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(phase: TurnPhase, at: u64) -> TurnObservation {
        TurnObservation { phase, at, since: at, agent_id: "a1".into(), message: None }
    }

    #[test]
    fn delivers_only_after_a_fresh_turn_end() {
        assert!(ready(Some(&turn(TurnPhase::Idle, 10)), None, 20));
        assert!(!ready(Some(&turn(TurnPhase::Working, 10)), None, 20));
        assert!(!ready(Some(&turn(TurnPhase::NeedsHuman, 10)), None, 20));
        assert!(!ready(None, None, 20));
        // 넣은 뒤 아직 그 턴의 관측이 안 왔다 — 옛 Idle로 다음을 넣지 않는다.
        assert!(!ready(Some(&turn(TurnPhase::Idle, 10)), Some(15), 20));
        assert!(ready(Some(&turn(TurnPhase::Idle, 30)), Some(15), 40));
        // 관측이 끝내 오지 않으면(슬래시 명령 등) 시간이 지나 넣는다.
        assert!(ready(Some(&turn(TurnPhase::Idle, 10)), Some(15), 15 + SENT_FALLBACK_MS));
    }

    #[test]
    fn steers_only_a_working_claude_without_pending_asks() {
        assert!(steerable(Some(&turn(TurnPhase::Working, 10)), true, false));
        assert!(!steerable(Some(&turn(TurnPhase::Working, 10)), false, false));
        assert!(!steerable(Some(&turn(TurnPhase::Working, 10)), true, true));
        assert!(!steerable(Some(&turn(TurnPhase::NeedsHuman, 10)), true, false));
        assert!(!steerable(Some(&turn(TurnPhase::Idle, 10)), true, false));
        assert!(!steerable(None, true, false));
    }

    #[test]
    fn view_hides_messages_for_another_session_and_cancel_removes_one() {
        let queue = ChatQueue::default();
        {
            let mut agents = queue.agents.lock().unwrap();
            let q = agents.entry("a1".into()).or_default();
            q.session_id = "s1".into();
            for id in ["q1", "q2"] {
                q.items.push_back(QueuedMessage { id: id.into(), text: id.into(), at: 1 });
            }
        }
        assert_eq!(queue.items_view("a1", Some("s1")).items.len(), 2);
        assert!(queue.items_view("a1", Some("s2")).items.is_empty());
        assert!(queue.cancel("a1", "q1"));
        assert!(!queue.cancel("a1", "q1"));
        assert_eq!(queue.items_view("a1", Some("s1")).items[0].id, "q2");
    }

    fn op_line(op: &str, ts: &str, content: &str) -> String {
        serde_json::json!({ "type": "queue-operation", "operation": op, "timestamp": ts, "sessionId": "s", "content": content })
            .to_string()
    }

    fn ms(ts: &str) -> u64 {
        chrono::DateTime::parse_from_rfc3339(ts).unwrap().timestamp_millis() as u64
    }

    fn steered(text: &str, ts: &str) -> Steered {
        Steered { text: text.into(), at: ms(ts), revision: 7 }
    }

    #[test]
    fn reads_claude_queue_operations_from_a_transcript_tail() {
        let tail = [
            r#"{"type":"user","message":{"content":"hi"}}"#.to_string(),
            op_line("enqueue", "2026-11-27T15:28:06.211Z", "폴더도 보여 줘"),
            op_line("popAll", "2026-11-27T15:28:11.063Z", "폴더도 보여 줘"),
            "not json".into(),
        ]
        .join("\n");
        let ops = queue_ops(&tail);
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[1], QueueOp { op: "popAll".into(), at: ms("2026-11-27T15:28:11.063Z"), content: "폴더도 보여 줘".into() });
    }

    #[test]
    fn finds_a_steered_message_claude_put_back_into_the_input() {
        let s = [steered("폴더도 보여 줘", "2026-11-27T15:28:06.000Z")];
        let ops = queue_ops(&[
            op_line("enqueue", "2026-11-27T15:28:06.211Z", "폴더도 보여 줘"),
            op_line("popAll", "2026-11-27T15:28:11.063Z", "폴더도 보여 줘"),
        ].join("\n"));
        let now = ms("2026-11-27T15:28:12.000Z");
        let v = judge_steered(&s, &ops, 0, now, None);
        let pop = v.popped.expect("popped");
        assert_eq!((pop.at, pop.texts.clone(), pop.revisions.clone()), (ms("2026-11-27T15:28:11.063Z"), vec!["폴더도 보여 줘".to_string()], vec![7]));
        assert!(v.settled.is_empty());
        // 이미 처리한 popAll은 다시 보지 않는다.
        assert_eq!(judge_steered(&s, &ops, pop.at, now, None).popped, None);
    }

    #[test]
    fn a_taken_message_or_an_older_pop_is_not_resubmitted() {
        let s = [steered("다음 것", "2026-11-27T15:30:00.000Z")];
        let ops = queue_ops(&[
            // 넣기 전의 popAll(같은 글이라도)은 우리 것이 아니다.
            op_line("popAll", "2026-11-27T15:20:00.000Z", "다음 것"),
            op_line("enqueue", "2026-11-27T15:30:00.100Z", "다음 것"),
            op_line("remove", "2026-11-27T15:30:09.000Z", "다음 것"),
        ].join("\n"));
        let v = judge_steered(&s, &ops, 0, ms("2026-11-27T15:30:10.000Z"), None);
        assert_eq!(v.popped, None);
        assert_eq!(v.settled, vec!["다음 것".to_string()]);
    }

    #[test]
    fn a_finished_turn_settles_what_was_waiting() {
        let s = [steered("남은 것", "2026-11-27T15:30:00.000Z")];
        let end = ms("2026-11-27T15:31:00.000Z");
        let idle = turn(TurnPhase::Idle, end);
        assert!(judge_steered(&s, &[], 0, end + 1_000, Some(&idle)).settled.is_empty());
        assert_eq!(judge_steered(&s, &[], 0, end + 10_000, Some(&idle)).settled, vec!["남은 것".to_string()]);
        let working = turn(TurnPhase::Working, end);
        assert!(judge_steered(&s, &[], 0, end + 10_000, Some(&working)).settled.is_empty());
    }

    #[test]
    fn text_rules_match_the_web_input() {
        assert_eq!(check_text("  \n"), Err("empty".into()));
        assert_eq!(check_text("a\u{1b}b"), Err("control-char".into()));
        assert!(check_text("줄\n바꿈\t탭").is_ok());
        assert_eq!(check_text(&"a".repeat(MAX_TEXT_BYTES + 1)), Err("too-large".into()));
    }
}
