// src-tauri/src/state.rs
//
// AppEvents event-emission boundary + SessionRegistry (agentId<->sessionId
// bookkeeping), plus `AppState` which wires
// SessionManager/NotificationHub/ProfileStore together for the Tauri
// `.manage()` call in `lib.rs`.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use tauri::{AppHandle, Emitter};

use crate::notification::hub::NotificationHub;
use crate::observer::server::ObserverServerState;
use crate::observer::ObserverRuntime;
use crate::persistence::profile_store::ProfileStore;
use crate::persistence::settings_store::{AppSettings, SettingsStore};
use crate::session::manager::SessionManager;
use crate::session_events::types::SessionStartedEvent;
use crate::types::*;

/// 이벤트 방출 경계(테스트 주입점). 프로덕션=TauriEvents, 테스트=RecordingEvents.
pub trait AppEvents: Send + Sync {
    fn session_started(&self, _ev: &SessionStartedEvent) {}
    fn session_state(&self, ev: &SessionStateEvent);
    fn notification_new(&self, ev: &NotificationEvent);
    fn notification_cleared(&self, agent_id: &str, ids: &[String]);
    fn activity_event(&self, ev: &ActivityEvent);
}

pub struct TauriEvents {
    pub app: AppHandle,
}
impl AppEvents for TauriEvents {
    fn session_state(&self, ev: &SessionStateEvent) {
        let _ = self.app.emit("session-state", ev);
    }
    fn notification_new(&self, ev: &NotificationEvent) {
        let _ = self.app.emit("notification-new", ev);
    }
    fn notification_cleared(&self, agent_id: &str, ids: &[String]) {
        let payload = NotificationClearedEvent {
            agent_id: agent_id.to_string(),
            ids: ids.to_vec(),
        };
        let _ = self.app.emit("notification-cleared", &payload);
    }
    fn activity_event(&self, ev: &ActivityEvent) {
        let _ = self.app.emit("activity-event", ev);
    }
}

/// sid → (agentId, state). SessionManager가 쓰고 NotificationHub가 읽어 순환 의존 제거.
#[derive(Default)]
pub struct SessionRegistry {
    map: RwLock<HashMap<SessionId, (AgentId, SessionState)>>,
}
impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn insert(&self, sid: &str, agent: &str, state: SessionState) {
        self.map
            .write()
            .unwrap()
            .insert(sid.into(), (agent.into(), state));
    }
    pub fn set_state(&self, sid: &str, state: SessionState) {
        if let Some(e) = self.map.write().unwrap().get_mut(sid) {
            e.1 = state;
        }
    }
    pub fn remove(&self, sid: &str) {
        self.map.write().unwrap().remove(sid);
    }
    pub fn resolve_agent(&self, sid: &str) -> Option<AgentId> {
        self.map.read().unwrap().get(sid).map(|(a, _)| a.clone())
    }
}

/// `tauri::Manager::manage()`가 보관하는 앱 전역 상태. 커맨드는 전부
/// `State<'_, AppState>`를 통해 이 구조체의 필드로만 위임한다.
///
pub struct AppState {
    pub manager: Arc<SessionManager>,
    pub hub: Arc<NotificationHub>,
    pub observer: Arc<ObserverRuntime>,
    pub observer_server: Arc<ObserverServerState>,
    pub store: ProfileStore,
    pub portrait_store: crate::persistence::png_store::PngStore,
    pub sprite_store: crate::persistence::png_store::PngStore,
    /// 세션 턴 시계열 로그(session-times.jsonl) — 턴이 종료될 때마다 append.
    pub session_time_store: crate::persistence::session_time_store::SessionTimeStore,
    /// 앱 전역 opt-in 설정 — 디스크 원본은 settings_store, 커맨드가 읽는
    /// 캐시는 settings(RwLock). set_app_settings가 저장+캐시 갱신을 함께 한다.
    /// `Arc`인 이유: lib.rs의 observer URL getter가 SessionManager
    /// 생성 시점에 이 캐시를 미리 clone해 쥐고 있어야, 실행 중 ON→OFF 전환이
    /// (서버는 유지한 채) 새 세션 훅 배선에 즉시 반영된다.
    pub settings_store: SettingsStore,
    pub settings: Arc<RwLock<AppSettings>>,
    /// 부팅 시 settings.json 부재 여부 — 첫 실행 동의 다이얼로그 신호.
    /// `set_app_settings` 성공 시 false로 내려가야 웹뷰 리로드 후에도 첫
    /// 실행 다이얼로그가 다시 뜨지 않는다 -- `AtomicBool`로 이 갱신을 표현.
    pub settings_first_run: AtomicBool,
}

// ── 테스트용 페이크 ────────────────────────────────────────────────────
//
// `RecordingEvents`(`Arc<Mutex<Vec<...>>>` 수집) — Tauri 앱 없이
// AppEvents 소비자(SessionManager/NotificationHub)를 단위 테스트
// 하기 위한 주입점. `crate::session::pty_factory::fake::FakePtyFactory`와
// 동일한 관례로 `#[cfg(test)] pub mod fake`에 둔다 — crate 전체 `cfg(test)`
// 빌드에서 다른 모듈의 테스트 코드가 그대로 가져다 쓸 수 있다.
#[cfg(test)]
pub mod fake {
    use super::AppEvents;
    use crate::session_events::types::SessionStartedEvent;
    use crate::types::{ActivityEvent, NotificationEvent, SessionState, SessionStateEvent};
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct RecordingEvents {
        session_starts: Mutex<Vec<SessionStartedEvent>>,
        timeline: Mutex<Vec<String>>,
        states: Mutex<Vec<SessionStateEvent>>,
        notifications: Mutex<Vec<NotificationEvent>>,
        cleared: Mutex<Vec<(String, Vec<String>)>>,
        activities: Mutex<Vec<ActivityEvent>>,
    }

    impl AppEvents for RecordingEvents {
        fn session_started(&self, ev: &SessionStartedEvent) {
            self.session_starts.lock().unwrap().push(ev.clone());
            self.timeline.lock().unwrap().push("session_started".into());
        }
        fn session_state(&self, ev: &SessionStateEvent) {
            self.states.lock().unwrap().push(ev.clone());
            self.timeline
                .lock()
                .unwrap()
                .push(format!("session_state:{:?}", ev.state));
        }
        fn notification_new(&self, ev: &NotificationEvent) {
            self.notifications.lock().unwrap().push(ev.clone());
        }
        fn notification_cleared(&self, agent_id: &str, ids: &[String]) {
            self.cleared
                .lock()
                .unwrap()
                .push((agent_id.to_string(), ids.to_vec()));
        }
        fn activity_event(&self, ev: &ActivityEvent) {
            self.activities.lock().unwrap().push(ev.clone());
        }
    }

    impl RecordingEvents {
        pub fn session_starts(&self) -> Vec<SessionStartedEvent> {
            self.session_starts.lock().unwrap().clone()
        }
        pub fn timeline(&self) -> Vec<String> {
            self.timeline.lock().unwrap().clone()
        }
        /// 지금까지 방출된 `session-state` 이벤트의 상태값 시퀀스.
        pub fn states(&self) -> Vec<SessionState> {
            self.states
                .lock()
                .unwrap()
                .iter()
                .map(|e| e.state)
                .collect()
        }
        /// 가장 최근 `session-state` 이벤트 전체(예: `.exit` 상세 확인용).
        ///
        /// # Panics
        /// 아직 이벤트가 하나도 기록되지 않았으면 패닉한다(테스트 전용).
        pub fn last_state(&self) -> SessionStateEvent {
            self.states
                .lock()
                .unwrap()
                .last()
                .cloned()
                .expect("RecordingEvents::last_state called with no recorded session-state events")
        }
        /// 지금까지 방출된 `notification-new` 이벤트 전체.
        pub fn notifications(&self) -> Vec<NotificationEvent> {
            self.notifications.lock().unwrap().clone()
        }
        /// 지금까지 방출된 `notification-cleared` 이벤트 전체: (agentId, ids).
        pub fn cleared(&self) -> Vec<(String, Vec<String>)> {
            self.cleared.lock().unwrap().clone()
        }
        /// 지금까지 방출된 activity-event 전체.
        pub fn activities(&self) -> Vec<ActivityEvent> {
            self.activities.lock().unwrap().clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::RecordingEvents;
    use super::{AppEvents, SessionRegistry};
    use crate::types::*;

    fn state_event(session_id: &str, agent_id: &str, state: SessionState) -> SessionStateEvent {
        SessionStateEvent {
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            state,
            exit: None,
            at: 1,
        }
    }

    fn notification(id: &str, session_id: &str, agent_id: &str) -> NotificationEvent {
        NotificationEvent {
            id: id.into(),
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            source: NotificationSource::Hook,
            message: "needs input".into(),
            dedup_key: format!("hook:{session_id}"),
            at: 1,
        }
    }

    // ---- RecordingEvents ----

    #[test]
    fn recording_events_collects_session_state_in_order() {
        let events = RecordingEvents::default();
        events.session_state(&state_event("s1", "a1", SessionState::Starting));
        events.session_state(&state_event("s1", "a1", SessionState::Running));
        events.session_state(&state_event("s1", "a1", SessionState::Exited));

        assert_eq!(
            events.states(),
            vec![
                SessionState::Starting,
                SessionState::Running,
                SessionState::Exited
            ]
        );
    }

    #[test]
    fn recording_events_last_state_returns_most_recent_full_event() {
        let events = RecordingEvents::default();
        events.session_state(&state_event("s1", "a1", SessionState::Starting));
        let exit_ev = SessionStateEvent {
            session_id: "s1".into(),
            agent_id: "a1".into(),
            state: SessionState::Exited,
            exit: Some(SessionExitInfo {
                session_id: "s1".into(),
                exit_code: Some(1),
                signal: None,
                intentional: false,
            }),
            at: 2,
        };
        events.session_state(&exit_ev);

        let last = events.last_state();
        assert_eq!(last.state, SessionState::Exited);
        let exit = last.exit.unwrap();
        assert!(!exit.intentional);
        assert_eq!(exit.exit_code, Some(1));
    }

    #[test]
    fn recording_events_collects_notifications() {
        let events = RecordingEvents::default();
        events.notification_new(&notification("n1", "s1", "a1"));
        events.notification_new(&notification("n2", "s1", "a1"));

        let got = events.notifications();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].id, "n1");
        assert_eq!(got[1].id, "n2");
    }

    #[test]
    fn recording_events_collects_notification_cleared() {
        let events = RecordingEvents::default();
        events.notification_cleared("a1", &["n1".to_string(), "n2".to_string()]);

        let cleared = events.cleared();
        assert_eq!(cleared.len(), 1);
        assert_eq!(cleared[0].0, "a1");
        assert_eq!(cleared[0].1, vec!["n1".to_string(), "n2".to_string()]);
    }

    #[test]
    fn recording_events_collects_activity_events() {
        use crate::types::{ActivityEvent, ActivityKind};
        let events = RecordingEvents::default();
        events.activity_event(&ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: ActivityKind::Prompt,
            at: 100,
            text: None,
        });
        events.activity_event(&ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: ActivityKind::Tool,
            at: 200,
            text: None,
        });

        let got = events.activities();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].kind, ActivityKind::Prompt);
        assert_eq!(got[0].at, 100);
        assert_eq!(got[1].kind, ActivityKind::Tool);
        assert_eq!(got[1].agent_id, "a1");
    }

    #[test]
    fn recording_events_is_usable_behind_the_appevents_trait_object() {
        // AppEvents is the injection seam: production code should only ever
        // depend on `dyn AppEvents`, never the concrete RecordingEvents type.
        // Keep a concrete handle to the same recorder so we can assert the
        // call made through the trait object was actually captured.
        let recorder = std::sync::Arc::new(RecordingEvents::default());
        let events: std::sync::Arc<dyn AppEvents> = recorder.clone();
        events.session_state(&state_event("s1", "a1", SessionState::Running));
        assert_eq!(recorder.states(), vec![SessionState::Running]);
    }

    // ---- SessionRegistry ----

    #[test]
    fn registry_insert_then_resolve_agent() {
        let reg = SessionRegistry::new();
        reg.insert("s1", "a1", SessionState::Starting);
        assert_eq!(reg.resolve_agent("s1"), Some("a1".to_string()));
    }

    #[test]
    fn registry_resolve_unknown_session_returns_none() {
        let reg = SessionRegistry::new();
        assert_eq!(reg.resolve_agent("missing"), None);
    }

    #[test]
    fn registry_set_state_updates_existing_entry() {
        let reg = SessionRegistry::new();
        reg.insert("s1", "a1", SessionState::Starting);
        reg.set_state("s1", SessionState::Running);
        // The frozen public API exposes no state getter, but this
        // test module is a descendant of `state`, so it can read the private
        // `map` field directly and assert the state value really changed.
        assert_eq!(
            reg.map.read().unwrap().get("s1").unwrap().1,
            SessionState::Running
        );
        // Agent mapping must be unaffected by a state-only update.
        assert_eq!(reg.resolve_agent("s1"), Some("a1".to_string()));
    }

    #[test]
    fn registry_set_state_on_missing_session_is_a_noop() {
        let reg = SessionRegistry::new();
        // Must not panic when the session doesn't exist.
        reg.set_state("missing", SessionState::Running);
        assert_eq!(reg.resolve_agent("missing"), None);
    }

    #[test]
    fn registry_remove_then_resolve_returns_none() {
        let reg = SessionRegistry::new();
        reg.insert("s1", "a1", SessionState::Running);
        reg.remove("s1");
        assert_eq!(reg.resolve_agent("s1"), None);
        // The entry must actually be gone from the map, not just unresolvable.
        assert!(!reg.map.read().unwrap().contains_key("s1"));
    }

    #[test]
    fn registry_remove_missing_session_is_a_noop() {
        let reg = SessionRegistry::new();
        // Must not panic when removing a session that was never inserted.
        reg.remove("missing");
    }

    #[test]
    fn registry_insert_overwrites_existing_entry_for_same_session_id() {
        let reg = SessionRegistry::new();
        reg.insert("s1", "a1", SessionState::Starting);
        reg.insert("s1", "a2", SessionState::Starting);
        assert_eq!(reg.resolve_agent("s1"), Some("a2".to_string()));
    }

    #[test]
    fn registry_tracks_multiple_sessions_independently() {
        let reg = SessionRegistry::new();
        reg.insert("s1", "a1", SessionState::Running);
        reg.insert("s2", "a2", SessionState::Starting);
        assert_eq!(reg.resolve_agent("s1"), Some("a1".to_string()));
        assert_eq!(reg.resolve_agent("s2"), Some("a2".to_string()));
        reg.remove("s1");
        assert_eq!(reg.resolve_agent("s1"), None);
        assert_eq!(reg.resolve_agent("s2"), Some("a2".to_string()));
    }
}
