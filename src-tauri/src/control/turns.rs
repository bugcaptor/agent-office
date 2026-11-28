// src-tauri/src/control/turns.rs
//
// 세션별 턴 상태(작업 중·사람 대기·턴 끝·종료)를 백엔드에서도 쥔다. 렌더러의
// `workMonitor.ts`와 같은 신호(activity·notification·turn-usage·session-state)를
// 같은 규칙으로 읽는다 — `ctl state`·`ctl ask`·`autorun status`가 렌더러 없이
// 답해야 하기 때문이다. 두 구현이 갈라지면 화면과 CLI가 다른 말을 하므로,
// 규칙을 바꿀 때는 `workMonitor.ts`의 `observe`와 함께 바꾼다.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::watch;

use crate::session_events::types::SessionStartedEvent;
use crate::state::AppEvents;
use crate::types::{
    ActivityEvent, ActivityKind, NotificationEvent, NotificationSource, SessionState,
    SessionStateEvent, TurnUsageEvent,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TurnPhase {
    /// 턴이 돌고 있다.
    Working,
    /// 권한 승인·질문처럼 사람이 답해야 다음으로 간다.
    NeedsHuman,
    /// 턴이 끝나 입력을 기다린다.
    Idle,
    /// 세션이 끝났다.
    Ended,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnObservation {
    pub phase: TurnPhase,
    pub at: u64,
    /// 이 상태가 이어진 시작 시각. 작업 중·사람 대기는 한 턴으로 보아 턴이 시작된 때를 지킨다
    /// (웹·대화 보기의 "생각 중 N초").
    pub since: u64,
    pub agent_id: String,
    /// 사람 대기일 때 훅이 준 문구(도구·명령 설명).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// 토의 세션의 턴 토큰 하나(머신 연동: 호스트가 진행하는 토의의 비용을 그 머신에서 올려 보낸다).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscussionUsage {
    pub seq: u64,
    pub at: u64,
    pub tokens: crate::types::SessionEventTokens,
    pub partial: bool,
}

/// 세션마다 들고 있는 토의 토큰 기록 수와 세션 수.
const MAX_USAGE_PER_SESSION: usize = 256;
const MAX_USAGE_SESSIONS: usize = 32;

/// 세션 id → 마지막 관측. 바뀔 때마다 `version`을 올려 기다리는 쪽을 깨운다.
pub struct TurnTracker {
    sessions: Mutex<HashMap<String, TurnObservation>>,
    version: watch::Sender<u64>,
    /// 토의 세션 id → (다음 seq, 기록). 토의 세션만 모은다.
    usage: Mutex<HashMap<String, (u64, std::collections::VecDeque<DiscussionUsage>)>>,
}

impl Default for TurnTracker {
    fn default() -> Self {
        Self { sessions: Mutex::new(HashMap::new()), version: watch::channel(0).0, usage: Mutex::new(HashMap::new()) }
    }
}

impl TurnTracker {
    /// 그 토의 세션의 `after`보다 뒤 토큰 기록.
    pub fn discussion_usage_after(&self, session_id: &str, after: u64) -> Vec<DiscussionUsage> {
        let usage = self.usage.lock().unwrap();
        usage.get(session_id).map(|(_, list)| list.iter().filter(|u| u.seq > after).cloned().collect()).unwrap_or_default()
    }

    fn record_usage(&self, ev: &TurnUsageEvent) {
        if !crate::types::is_discussion_session_id(&ev.agent_id) {
            return;
        }
        let mut usage = self.usage.lock().unwrap();
        if !usage.contains_key(&ev.session_id) && usage.len() >= MAX_USAGE_SESSIONS {
            let oldest = usage.iter().min_by_key(|(_, (_, l))| l.back().map_or(0, |u| u.at)).map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                usage.remove(&k);
            }
        }
        let (next, list) = usage.entry(ev.session_id.clone()).or_insert((1, Default::default()));
        list.push_back(DiscussionUsage { seq: *next, at: ev.at, tokens: ev.tokens.clone(), partial: ev.partial });
        *next += 1;
        while list.len() > MAX_USAGE_PER_SESSION {
            list.pop_front();
        }
    }

    pub fn get(&self, session_id: &str) -> Option<TurnObservation> {
        self.sessions.lock().unwrap().get(session_id).cloned()
    }

    /// 턴이 도는 중(작업 중·사람 대기)인 세션들.
    pub fn in_turn(&self) -> Vec<(String, TurnObservation)> {
        let sessions = self.sessions.lock().unwrap();
        sessions
            .iter()
            .filter(|(_, o)| matches!(o.phase, TurnPhase::Working | TurnPhase::NeedsHuman))
            .map(|(sid, o)| (sid.clone(), o.clone()))
            .collect()
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.version.subscribe()
    }

    fn observe(&self, session_id: &str, agent_id: &str, phase: TurnPhase, at: u64, message: Option<String>) {
        let mut sessions = self.sessions.lock().unwrap();
        let mut since = at;
        if let Some(old) = sessions.get(session_id) {
            // 늦게 도착한 옛 신호는 버린다.
            if old.at > at {
                return;
            }
            // 턴이 끝난 뒤의 질문 신호는 Claude idle_prompt(입력 대기 리마인더)다.
            if phase == TurnPhase::NeedsHuman && matches!(old.phase, TurnPhase::Idle | TurnPhase::Ended) {
                return;
            }
            let in_turn = |p: TurnPhase| matches!(p, TurnPhase::Working | TurnPhase::NeedsHuman);
            if old.phase == phase || (in_turn(old.phase) && in_turn(phase)) {
                since = old.since;
            }
        }
        sessions.insert(
            session_id.to_string(),
            TurnObservation { phase, at, since, agent_id: agent_id.to_string(), message },
        );
        drop(sessions);
        self.version.send_modify(|v| *v += 1);
    }
}

/// 안쪽 `AppEvents`로 모두 넘기면서 `TurnTracker`를 갱신하는 데코레이터.
pub struct TrackingEvents {
    inner: Arc<dyn AppEvents>,
    tracker: Arc<TurnTracker>,
}

impl TrackingEvents {
    pub fn new(inner: Arc<dyn AppEvents>, tracker: Arc<TurnTracker>) -> Self {
        Self { inner, tracker }
    }
}

impl AppEvents for TrackingEvents {
    fn session_started(&self, ev: &SessionStartedEvent) {
        self.inner.session_started(ev);
    }
    fn session_state(&self, ev: &SessionStateEvent) {
        if matches!(ev.state, SessionState::Exited | SessionState::Disposed) {
            self.tracker.observe(&ev.session_id, &ev.agent_id, TurnPhase::Ended, ev.at, None);
        }
        self.inner.session_state(ev);
    }
    fn notification_new(&self, ev: &NotificationEvent) {
        match ev.source {
            NotificationSource::Hook => self.tracker.observe(
                &ev.session_id,
                &ev.agent_id,
                TurnPhase::NeedsHuman,
                ev.at,
                Some(ev.message.clone()).filter(|m| !m.trim().is_empty()),
            ),
            NotificationSource::Stop => {
                self.tracker.observe(&ev.session_id, &ev.agent_id, TurnPhase::Idle, ev.at, None)
            }
            NotificationSource::Bell => {}
        }
        self.inner.notification_new(ev);
    }
    fn notification_cleared(&self, agent_id: &str, ids: &[String]) {
        self.inner.notification_cleared(agent_id, ids);
    }
    fn activity_event(&self, ev: &ActivityEvent) {
        if matches!(ev.kind, ActivityKind::Prompt | ActivityKind::Tool | ActivityKind::Resume) {
            self.tracker.observe(&ev.session_id, &ev.agent_id, TurnPhase::Working, ev.at, None);
        }
        // 알림 없이 턴을 정산하는 신호: 셸 명령 끝, Stop 훅 없이 끝난 Claude 턴(사용자가 끊음·API 오류,
        // `turn_halt.rs`). 렌더러도 같은 신호로 턴을 정산한다(`appStore`·`workMonitor.ts`).
        if ev.kind == ActivityKind::Idle {
            self.tracker.observe(&ev.session_id, &ev.agent_id, TurnPhase::Idle, ev.at, None);
        }
        self.inner.activity_event(ev);
    }
    fn turn_usage(&self, ev: &TurnUsageEvent) {
        self.tracker.record_usage(ev);
        if ev.ends_turn() {
            self.tracker.observe(&ev.session_id, &ev.agent_id, TurnPhase::Idle, ev.at, None);
        }
        self.inner.turn_usage(ev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::fake::RecordingEvents;

    fn activity(kind: ActivityKind, at: u64) -> ActivityEvent {
        ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind,
            at,
            text: None,
            assistant_text: None,
            cwd: None,
            count: None,
        }
    }

    fn notification(source: NotificationSource, at: u64, message: &str) -> NotificationEvent {
        NotificationEvent {
            id: format!("n{at}"),
            session_id: "s1".into(),
            agent_id: "a1".into(),
            source,
            message: message.into(),
            dedup_key: String::new(),
            at,
        }
    }

    fn setup() -> (Arc<TurnTracker>, TrackingEvents, Arc<RecordingEvents>) {
        let tracker = Arc::new(TurnTracker::default());
        let inner = Arc::new(RecordingEvents::default());
        let events = TrackingEvents::new(inner.clone(), tracker.clone());
        (tracker, events, inner)
    }

    #[test]
    fn turn_runs_waits_for_permission_and_ends() {
        let (tracker, events, _) = setup();
        events.activity_event(&activity(ActivityKind::Prompt, 10));
        assert_eq!(tracker.get("s1").unwrap().phase, TurnPhase::Working);
        events.notification_new(&notification(NotificationSource::Hook, 20, "Claude needs your permission to use Bash"));
        let waiting = tracker.get("s1").unwrap();
        assert_eq!(waiting.phase, TurnPhase::NeedsHuman);
        assert_eq!(waiting.message.as_deref(), Some("Claude needs your permission to use Bash"));
        events.activity_event(&activity(ActivityKind::Tool, 30));
        assert_eq!(tracker.get("s1").unwrap().phase, TurnPhase::Working);
        assert_eq!(tracker.get("s1").unwrap().since, 10, "권한 대기를 지나도 턴 시작 시각을 지킨다");
        events.notification_new(&notification(NotificationSource::Stop, 40, "done"));
        let idle = tracker.get("s1").unwrap();
        assert_eq!((idle.phase, idle.since), (TurnPhase::Idle, 40));
        events.activity_event(&activity(ActivityKind::Prompt, 50));
        assert_eq!(tracker.get("s1").unwrap().since, 50);
    }

    #[test]
    fn idle_activity_settles_the_turn_without_a_stop() {
        let (tracker, events, _) = setup();
        events.activity_event(&activity(ActivityKind::Prompt, 10));
        events.activity_event(&activity(ActivityKind::Tool, 20));
        assert_eq!(tracker.in_turn().len(), 1);
        events.activity_event(&activity(ActivityKind::Idle, 30));
        let idle = tracker.get("s1").unwrap();
        assert_eq!((idle.phase, idle.since), (TurnPhase::Idle, 30));
        assert!(tracker.in_turn().is_empty());
        events.activity_event(&activity(ActivityKind::Prompt, 40));
        assert_eq!(tracker.get("s1").unwrap().phase, TurnPhase::Working);
    }

    #[test]
    fn idle_reminder_after_turn_end_and_stale_signals_are_ignored() {
        let (tracker, events, _) = setup();
        events.notification_new(&notification(NotificationSource::Stop, 40, ""));
        events.notification_new(&notification(NotificationSource::Hook, 50, "Claude is waiting for your input"));
        assert_eq!(tracker.get("s1").unwrap().phase, TurnPhase::Idle);
        events.activity_event(&activity(ActivityKind::Prompt, 30));
        assert_eq!(tracker.get("s1").unwrap().phase, TurnPhase::Idle);
    }

    #[test]
    fn discussion_session_tokens_are_kept_for_the_host_to_read_after_a_seq() {
        let (tracker, events, _) = setup();
        let usage = |agent: &str, at: u64, partial: bool| TurnUsageEvent {
            agent_id: agent.into(),
            session_id: if agent == "a1" { "s1".into() } else { "ds1".into() },
            at,
            tokens: Default::default(),
            partial,
            subagents_running: false,
        };
        events.turn_usage(&usage("a1", 5, false));
        assert!(tracker.discussion_usage_after("s1", 0).is_empty(), "캐릭터 세션은 모으지 않는다");
        events.turn_usage(&usage("discuss-d1.ada", 10, true));
        events.turn_usage(&usage("discuss-d1.ada", 20, false));
        let all = tracker.discussion_usage_after("ds1", 0);
        assert_eq!(all.iter().map(|u| (u.seq, u.at, u.partial)).collect::<Vec<_>>(), vec![(1, 10, true), (2, 20, false)]);
        assert_eq!(tracker.discussion_usage_after("ds1", 1).len(), 1, "이미 읽은 것은 건너뛴다");
        for n in 0..MAX_USAGE_PER_SESSION + 10 {
            events.turn_usage(&usage("discuss-d1.ada", 100 + n as u64, true));
        }
        let kept = tracker.discussion_usage_after("ds1", 0);
        assert_eq!(kept.len(), MAX_USAGE_PER_SESSION);
        assert_eq!(kept.last().unwrap().seq, (MAX_USAGE_PER_SESSION + 12) as u64, "seq는 버린 뒤에도 이어진다");
    }

    #[test]
    fn final_turn_usage_and_exit_update_phase_and_wake_waiters() {
        let (tracker, events, _) = setup();
        let mut rx = tracker.subscribe();
        events.activity_event(&activity(ActivityKind::Prompt, 10));
        events.turn_usage(&TurnUsageEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            at: 15,
            tokens: Default::default(),
            partial: true,
            subagents_running: false,
        });
        assert_eq!(tracker.get("s1").unwrap().phase, TurnPhase::Working);
        events.turn_usage(&TurnUsageEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            at: 20,
            tokens: Default::default(),
            partial: false,
            subagents_running: true,
        });
        // 백그라운드 서브에이전트가 남은 Stop은 턴 끝이 아니다.
        assert_eq!(tracker.get("s1").unwrap().phase, TurnPhase::Working);
        events.turn_usage(&TurnUsageEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            at: 20,
            tokens: Default::default(),
            partial: false,
            subagents_running: false,
        });
        assert_eq!(tracker.get("s1").unwrap().phase, TurnPhase::Idle);
        assert!(rx.has_changed().unwrap());
        rx.mark_unchanged();
        events.session_state(&SessionStateEvent {
            session_id: "s1".into(),
            agent_id: "a1".into(),
            state: SessionState::Exited,
            exit: None,
            at: 30,
            external: None,
        });
        assert_eq!(tracker.get("s1").unwrap().phase, TurnPhase::Ended);
        assert!(rx.has_changed().unwrap());
    }
}
