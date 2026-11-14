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
    pub agent_id: String,
    /// 사람 대기일 때 훅이 준 문구(도구·명령 설명).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// 세션 id → 마지막 관측. 바뀔 때마다 `version`을 올려 기다리는 쪽을 깨운다.
pub struct TurnTracker {
    sessions: Mutex<HashMap<String, TurnObservation>>,
    version: watch::Sender<u64>,
}

impl Default for TurnTracker {
    fn default() -> Self {
        Self { sessions: Mutex::new(HashMap::new()), version: watch::channel(0).0 }
    }
}

impl TurnTracker {
    pub fn get(&self, session_id: &str) -> Option<TurnObservation> {
        self.sessions.lock().unwrap().get(session_id).cloned()
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.version.subscribe()
    }

    fn observe(&self, session_id: &str, agent_id: &str, phase: TurnPhase, at: u64, message: Option<String>) {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(old) = sessions.get(session_id) {
            // 늦게 도착한 옛 신호는 버린다.
            if old.at > at {
                return;
            }
            // 턴이 끝난 뒤의 질문 신호는 Claude idle_prompt(입력 대기 리마인더)다.
            if phase == TurnPhase::NeedsHuman && matches!(old.phase, TurnPhase::Idle | TurnPhase::Ended) {
                return;
            }
        }
        sessions.insert(
            session_id.to_string(),
            TurnObservation { phase, at, agent_id: agent_id.to_string(), message },
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
        self.inner.activity_event(ev);
    }
    fn turn_usage(&self, ev: &TurnUsageEvent) {
        if !ev.partial {
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
        events.notification_new(&notification(NotificationSource::Stop, 40, "done"));
        assert_eq!(tracker.get("s1").unwrap().phase, TurnPhase::Idle);
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
        });
        assert_eq!(tracker.get("s1").unwrap().phase, TurnPhase::Working);
        events.turn_usage(&TurnUsageEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            at: 20,
            tokens: Default::default(),
            partial: false,
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
