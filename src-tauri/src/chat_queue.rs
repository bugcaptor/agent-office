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
    pub async fn send(&self, ctx: &ControlContext, agent_id: &str, text: &str) -> Result<SendOutcome, String> {
        check_text(text)?;
        let session_id = ctx
            .manager
            .session_id_for(agent_id)
            .filter(|_| ctx.manager.is_running(agent_id))
            .ok_or("not-running")?;
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
                let manager = ctx.manager.clone();
                let agent = agent_id.to_string();
                let cli = tokio::task::spawn_blocking(move || {
                    crate::webdash::foreground_cli(&manager, &agent, manager.foreground_command(&agent).as_deref())
                })
                .await
                .ok()
                .flatten()
                .is_some();
                if !cli {
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
        let result = deliver(ctx, agent_id, &session_id, text).await;
        let mut agents = self.agents.lock().unwrap();
        if let Some(q) = agents.get_mut(agent_id) {
            q.delivering = false;
            if result.is_ok() {
                q.sent_at = Some(crate::types::now_ms());
                q.error = None;
            }
        }
        result.map(|_| SendOutcome::Sent)
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
            let result = deliver(ctx, &agent_id, &session_id, &msg.text).await;
            let mut agents = self.agents.lock().unwrap();
            let Some(q) = agents.get_mut(&agent_id) else { continue };
            q.delivering = false;
            match result {
                Ok(()) => {
                    q.sent_at = Some(crate::types::now_ms());
                    q.error = None;
                }
                Err(e) => {
                    // 넣지 못한 메시지는 맨 앞에 되돌린다. 일시적이면 잠시 뒤, 아니면 다음 턴 끝에 다시 해 본다.
                    if q.session_id == session_id {
                        q.items.push_front(msg);
                    }
                    q.retry_at = crate::types::now_ms() + if transient(&e) { RETRY_MS } else { RETRY_MS * 5 };
                    q.error = Some(e);
                }
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

async fn deliver(ctx: &ControlContext, agent_id: &str, session_id: &str, text: &str) -> Result<(), String> {
    ctx.work_send
        .follow_up(
            &ctx.gate,
            WorkFollowUpRequest { agent_id: agent_id.to_string(), session_id: session_id.to_string(), text: text.to_string() },
        )
        .await
        .map(|_| ())
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

    #[test]
    fn text_rules_match_the_web_input() {
        assert_eq!(check_text("  \n"), Err("empty".into()));
        assert_eq!(check_text("a\u{1b}b"), Err("control-char".into()));
        assert!(check_text("줄\n바꿈\t탭").is_ok());
        assert_eq!(check_text(&"a".repeat(MAX_TEXT_BYTES + 1)), Err("too-large".into()));
    }
}
