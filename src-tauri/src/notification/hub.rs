// src-tauri/src/notification/hub.rs
//
// NotificationHub + Clock. `SessionManager` holds an `Arc<NotificationHub>`
// concretely (not a trait object), and even SessionManager's own unit tests
// need a real `NotificationHub` to construct one — so this module has to
// exist and compile before SessionManager's tests can run, even though this
// file's own dedicated coverage (dedup window, partial/full clear,
// discarding hooks for dead/unknown sessions) isn't exercised by those
// tests. SessionManager's tests only cover its own exit-transition,
// autostart, and reuse-on-duplicate-create behavior; a follow-up should add
// dedup/clear/dead-session tests for this file.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::observer::event::{message as observer_message, prompt_text, ObserverEvent};
use crate::state::{AppEvents, SessionRegistry};
use crate::types::*;

const ATTENTION_FALLBACK: &str = "확인이 필요합니다";
const STOP_FALLBACK: &str = "작업이 완료되었습니다.";

/// 주입 가능한 시계. dedup 윈도우(Instant) + at 타임스탬프(epoch ms).
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn now_ms(&self) -> u64;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn now_ms(&self) -> u64 {
        now_ms()
    }
}

pub struct NotificationHub {
    registry: Arc<SessionRegistry>,
    events: Arc<dyn AppEvents>,
    clock: Arc<dyn Clock>,
    dedup_window: Duration,
    queues: Mutex<HashMap<SessionId, Vec<NotificationEvent>>>,
    last_seen: Mutex<HashMap<String, Instant>>,
}

impl NotificationHub {
    pub fn new(
        registry: Arc<SessionRegistry>,
        events: Arc<dyn AppEvents>,
        clock: Arc<dyn Clock>,
        dedup_window: Duration,
    ) -> Self {
        Self {
            registry,
            events,
            clock,
            dedup_window,
            queues: Mutex::new(HashMap::new()),
            last_seen: Mutex::new(HashMap::new()),
        }
    }

    /// axum 핸들러가 호출: 원본 hook body에서 메시지 추출 후 ingest.
    pub fn ingest_hook(&self, session_id: &str, source: NotificationSource, body: &[u8]) {
        let message = observer_message(body).unwrap_or_else(|| {
            match source {
                NotificationSource::Stop => STOP_FALLBACK,
                _ => ATTENTION_FALLBACK,
            }
            .to_string()
        });
        self.ingest(session_id, source, message);
    }

    /// axum 핸들러가 호출: prompt/tool activity 신호를 dedup/큐 없이 즉시
    /// activity-event로 방출한다. 죽은/미지 세션은 폐기한다.
    pub fn ingest_activity(&self, session_id: &str, kind: ActivityKind) {
        self.ingest_activity_inner(session_id, kind, None);
    }

    /// prompt 훅 전용: body(UserPromptSubmit 이벤트 JSON)에서 원문을 추출해 싣는다.
    /// 파싱 실패는 text=None으로 강등될 뿐, 이벤트 방출 자체는 항상 일어난다.
    pub fn ingest_activity_with_body(&self, session_id: &str, kind: ActivityKind, body: &[u8]) {
        self.ingest_activity_inner(session_id, kind, prompt_text(body));
    }

    pub fn ingest_observer(&self, session_id: &str, event: ObserverEvent) {
        match event {
            ObserverEvent::Prompt { text } => {
                self.ingest_activity_inner(session_id, ActivityKind::Prompt, text)
            }
            ObserverEvent::Tool => self.ingest_activity(session_id, ActivityKind::Tool),
            ObserverEvent::SubStart => self.ingest_activity(session_id, ActivityKind::SubStart),
            ObserverEvent::SubStop => self.ingest_activity(session_id, ActivityKind::SubStop),
            ObserverEvent::Attention { message } => self.ingest(
                session_id,
                NotificationSource::Hook,
                message.filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| ATTENTION_FALLBACK.to_string()),
            ),
            ObserverEvent::Stop { message } => self.ingest(
                session_id,
                NotificationSource::Stop,
                message.filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| STOP_FALLBACK.to_string()),
            ),
        }
    }

    fn ingest_activity_inner(&self, session_id: &str, kind: ActivityKind, text: Option<String>) {
        let Some(agent_id) = self.registry.resolve_agent(session_id) else {
            return;
        };
        let ev = ActivityEvent {
            agent_id,
            session_id: session_id.to_string(),
            kind,
            at: self.clock.now_ms(),
            text,
        };
        self.events.activity_event(&ev);
    }

    /// BEL 폴백: output pump가 0x07 감지 시.
    pub fn on_bell(&self, session_id: &str) {
        self.ingest(session_id, NotificationSource::Bell, "Terminal bell".to_string());
    }

    fn ingest(&self, session_id: &str, source: NotificationSource, message: String) {
        // 죽은/미지 세션의 hook은 폐기.
        let Some(agent_id) = self.registry.resolve_agent(session_id) else {
            return;
        };

        let key = dedup_key(session_id, source, &message);
        let now_i = self.clock.now();
        {
            let mut ls = self.last_seen.lock().unwrap();
            if let Some(prev) = ls.get(&key) {
                if now_i.duration_since(*prev) < self.dedup_window {
                    ls.insert(key, now_i); // 윈도우 슬라이드
                    return; // 억제
                }
            }
            ls.insert(key.clone(), now_i);
        }

        let ev = NotificationEvent {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            agent_id,
            source,
            message,
            dedup_key: key,
            at: self.clock.now_ms(),
        };
        self.queues.lock().unwrap().entry(session_id.to_string()).or_default().push(ev.clone());
        self.events.notification_new(&ev);
    }

    pub fn pending(&self, session_id: &str) -> Vec<NotificationEvent> {
        self.queues.lock().unwrap().get(session_id).cloned().unwrap_or_default()
    }

    /// 터미널 열림 시 클리어. ids 없으면 세션 전체. cleared된 id 방출.
    pub fn clear(&self, session_id: &str, ids: Option<Vec<String>>) -> Vec<String> {
        let cleared: Vec<String> = {
            let mut q = self.queues.lock().unwrap();
            let Some(list) = q.get_mut(session_id) else {
                return Vec::new();
            };
            match ids {
                Some(ids) if !ids.is_empty() => {
                    let set: std::collections::HashSet<_> = ids.into_iter().collect();
                    let hit: Vec<String> =
                        list.iter().filter(|e| set.contains(&e.id)).map(|e| e.id.clone()).collect();
                    list.retain(|e| !set.contains(&e.id));
                    hit
                }
                _ => {
                    let all: Vec<String> = list.iter().map(|e| e.id.clone()).collect();
                    q.remove(session_id);
                    all
                }
            }
        };
        if !cleared.is_empty() {
            if let Some(agent_id) = self.registry.resolve_agent(session_id) {
                self.events.notification_cleared(&agent_id, &cleared);
            }
        }
        cleared
    }

    pub fn purge_session(&self, session_id: &str) {
        self.queues.lock().unwrap().remove(session_id);
    }
}

fn dedup_key(session_id: &str, source: NotificationSource, message: &str) -> String {
    // sha1-or-equivalent. sha1_smol(순수 Rust, 추가 트랜지티브 의존 없음).
    let mut h = sha1_smol::Sha1::new();
    h.update(format!("{}|{}|{}", session_id, source.as_key(), message.trim()).as_bytes());
    h.digest().to_string()
}

// ── 테스트용 페이크 ────────────────────────────────────────────────────
//
// `FakeClock`(atomic ms 오프셋 + `advance()`) — dedup 윈도우를
// 실제 sleep 없이 결정론적으로 제어하기 위한 `Clock` 주입점. `base: Instant`를
// 생성 시 한 번 잡고, 이후 `now()`는 `base + offset_ms`, `now_ms()`는 고정
// synthetic epoch + offset_ms를 반환한다 — 두 시계가 `advance()`에 맞춰
// 정확히 같은 양만큼 함께 흐른다.
#[cfg(test)]
pub mod fake {
    use super::Clock;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    pub struct FakeClock {
        base: Instant,
        offset_ms: AtomicU64,
    }

    impl FakeClock {
        pub fn new() -> Self {
            Self { base: Instant::now(), offset_ms: AtomicU64::new(0) }
        }

        /// 시계를 `ms` 밀리초 전진시킨다. `now()`/`now_ms()` 모두 동일하게 반영된다.
        pub fn advance(&self, ms: u64) {
            self.offset_ms.fetch_add(ms, Ordering::SeqCst);
        }

        fn offset(&self) -> u64 {
            self.offset_ms.load(Ordering::SeqCst)
        }
    }

    impl Default for FakeClock {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.base + Duration::from_millis(self.offset())
        }
        fn now_ms(&self) -> u64 {
            // 임의의 고정 synthetic epoch + offset. 절대값이 아니라 advance()에
            // 맞춰 결정론적으로 흐르는지가 테스트 관심사이므로 실제 epoch일 필요 없음.
            1_700_000_000_000 + self.offset()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeClock;
    use super::{dedup_key, Clock, NotificationHub};
    use crate::observer::event::ObserverEvent;
    use crate::state::fake::RecordingEvents;
    use crate::state::SessionRegistry;
    use crate::types::*;
    use std::sync::Arc;
    use std::time::Duration;

    /// Standard fixture: session "s1" mapped to agent "a1", registered as
    /// Running, wired to a fresh FakeClock + RecordingEvents + 3s dedup
    /// window.
    fn fixture() -> (Arc<NotificationHub>, Arc<RecordingEvents>, Arc<FakeClock>) {
        let registry = Arc::new(SessionRegistry::new());
        registry.insert("s1", "a1", SessionState::Running);
        let events = Arc::new(RecordingEvents::default());
        let clock = Arc::new(FakeClock::new());
        let hub = Arc::new(NotificationHub::new(
            registry,
            events.clone(),
            clock.clone(),
            Duration::from_millis(3000),
        ));
        (hub, events, clock)
    }

    #[test]
    fn observer_events_project_to_existing_public_contracts() {
        let (hub, events, _clock) = fixture();
        hub.ingest_observer("s1", ObserverEvent::Prompt { text: Some("버그 수정".into()) });
        hub.ingest_observer("s1", ObserverEvent::Tool);
        hub.ingest_observer("s1", ObserverEvent::Attention { message: None });
        hub.ingest_observer("s1", ObserverEvent::Stop { message: None });

        let activity = events.activities();
        assert_eq!(activity[0].kind, ActivityKind::Prompt);
        assert_eq!(activity[0].text.as_deref(), Some("버그 수정"));
        assert_eq!(activity[1].kind, ActivityKind::Tool);

        let notifications = events.notifications();
        assert_eq!(notifications[0].source, NotificationSource::Hook);
        assert_eq!(notifications[0].message, "확인이 필요합니다");
        assert_eq!(notifications[1].source, NotificationSource::Stop);
        assert_eq!(notifications[1].message, "작업이 완료되었습니다.");
    }

    fn msg(text: &str) -> Vec<u8> {
        serde_json::json!({ "message": text }).to_string().into_bytes()
    }

    fn is_uuid_v4(s: &str) -> bool {
        // 8-4-4-4-12 hex groups, version nibble '4' at the start of the 3rd group.
        let parts: Vec<&str> = s.split('-').collect();
        parts.len() == 5
            && [8, 4, 4, 4, 12].iter().zip(&parts).all(|(len, p)| p.len() == *len)
            && parts[2].starts_with('4')
            && s.chars().all(|c| c == '-' || c.is_ascii_hexdigit())
    }

    // ---- T-C: dedup 3s window (FakeClock) ----

    #[test]
    fn dedup_suppresses_within_window_then_passes_once_window_elapses() {
        let (hub, events, clock) = fixture();

        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input")); // t=0, passes
        clock.advance(1000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input")); // t=1000, suppressed (slides window to 1000)
        clock.advance(4000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input")); // t=5000, 5000-1000=4000 >= 3000, passes

        assert_eq!(events.notifications().len(), 2);
        let pending = hub.pending("s1");
        assert_eq!(pending.len(), 2);

        // Pin real emitted payload values, not just counts.
        assert_eq!(pending[0].session_id, "s1");
        assert_eq!(pending[0].agent_id, "a1");
        assert_eq!(pending[0].source, NotificationSource::Hook);
        assert_eq!(pending[0].message, "need input");
        assert_eq!(pending[0].dedup_key, dedup_key("s1", NotificationSource::Hook, "need input"));
        assert_eq!(pending[0].at, 1_700_000_000_000); // clock.now_ms() at t=0

        // Second passing notification reflects the *slid* window, i.e. it
        // was emitted at t=5000, not t=0 or t=1000.
        assert_eq!(pending[1].at, 1_700_000_005_000);
        assert_ne!(pending[0].id, pending[1].id, "each passing notification gets a fresh id");
        assert!(is_uuid_v4(&pending[0].id) && is_uuid_v4(&pending[1].id));
    }

    #[test]
    fn dedup_window_slide_means_suppressed_hit_resets_the_clock() {
        // If suppression did NOT slide last_seen, a hit at t=1000 followed by
        // one at t=3500 (3500ms after t=0) would incorrectly pass. Pin that
        // it stays suppressed because the window slid to t=1000 on the
        // t=1000 hit, so t=3500 is only 2500ms after the new baseline.
        let (hub, events, clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input")); // t=0, passes
        clock.advance(1000);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input")); // t=1000, suppressed, slides to 1000
        clock.advance(2500);
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input")); // t=3500, 3500-1000=2500 < 3000, still suppressed

        assert_eq!(events.notifications().len(), 1);
        assert_eq!(hub.pending("s1").len(), 1);
    }

    #[test]
    fn dedup_key_is_per_session_so_other_sessions_are_unaffected() {
        let registry = Arc::new(SessionRegistry::new());
        registry.insert("s1", "a1", SessionState::Running);
        registry.insert("s2", "a2", SessionState::Running);
        let events = Arc::new(RecordingEvents::default());
        let clock = Arc::new(FakeClock::new());
        let hub = NotificationHub::new(registry, events.clone(), clock, Duration::from_millis(3000));

        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input"));
        hub.ingest_hook("s2", NotificationSource::Hook, &msg("need input")); // same message, different session: not suppressed

        assert_eq!(events.notifications().len(), 2);
        assert_eq!(hub.pending("s1").len(), 1);
        assert_eq!(hub.pending("s2").len(), 1);
    }

    #[test]
    fn different_messages_on_same_session_are_not_deduped() {
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input"));
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("different message")); // distinct message, separate entry

        assert_eq!(events.notifications().len(), 2);
        assert_eq!(hub.pending("s1").len(), 2);
    }

    // ---- T-D: clear partial vs full ----

    #[test]
    fn clear_with_specific_ids_removes_only_those_and_emits_cleared() {
        let (hub, events, _clock) = fixture();
        for m in ["m1", "m2", "m3"] {
            hub.ingest_hook("s1", NotificationSource::Hook, &msg(m));
        }
        let ids: Vec<String> = hub.pending("s1").iter().map(|e| e.id.clone()).collect();
        assert_eq!(ids.len(), 3);

        let cleared = hub.clear("s1", Some(vec![ids[1].clone()]));
        assert_eq!(cleared, vec![ids[1].clone()]);

        let remaining: Vec<String> = hub.pending("s1").iter().map(|e| e.id.clone()).collect();
        assert_eq!(remaining, vec![ids[0].clone(), ids[2].clone()]);

        let cleared_events = events.cleared();
        assert_eq!(cleared_events.len(), 1);
        assert_eq!(cleared_events[0], ("a1".to_string(), vec![ids[1].clone()]));
    }

    #[test]
    fn clear_with_none_clears_entire_session_and_emits_all_ids() {
        let (hub, events, _clock) = fixture();
        for m in ["m1", "m2", "m3"] {
            hub.ingest_hook("s1", NotificationSource::Hook, &msg(m));
        }
        let ids: Vec<String> = hub.pending("s1").iter().map(|e| e.id.clone()).collect();

        let cleared = hub.clear("s1", None);
        assert_eq!(cleared.len(), 3);
        assert_eq!(
            cleared.iter().collect::<std::collections::HashSet<_>>(),
            ids.iter().collect::<std::collections::HashSet<_>>()
        );
        assert!(hub.pending("s1").is_empty());

        let cleared_events = events.cleared();
        assert_eq!(cleared_events.len(), 1);
        assert_eq!(cleared_events[0].0, "a1");
        assert_eq!(
            cleared_events[0].1.iter().collect::<std::collections::HashSet<_>>(),
            ids.iter().collect::<std::collections::HashSet<_>>()
        );
    }

    #[test]
    fn clear_with_some_empty_vec_behaves_like_clear_all() {
        // The match arm `Some(ids) if !ids.is_empty()` only takes the
        // partial-clear path for a *non-empty* id list; `Some(vec![])` falls
        // through to the `_` (clear-all) arm. Pin this exact, easy-to-break
        // boundary condition rather than assuming it.
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("m1"));
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("m2"));

        let cleared = hub.clear("s1", Some(vec![]));
        assert_eq!(cleared.len(), 2);
        assert!(hub.pending("s1").is_empty());
        assert_eq!(events.cleared().len(), 1);
    }

    #[test]
    fn clear_on_session_with_no_pending_returns_empty_and_emits_nothing() {
        let (hub, events, _clock) = fixture();
        // No ingest happened for "s1" at all.
        assert_eq!(hub.clear("s1", None), Vec::<String>::new());
        assert!(events.cleared().is_empty());
    }

    #[test]
    fn clear_with_unknown_ids_matches_nothing_and_leaves_queue_intact() {
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("m1"));

        let cleared = hub.clear("s1", Some(vec!["not-a-real-id".to_string()]));
        assert!(cleared.is_empty());
        assert_eq!(hub.pending("s1").len(), 1); // untouched
        assert!(events.cleared().is_empty()); // no emit when nothing actually cleared
    }

    // ---- on_bell ----

    #[test]
    fn on_bell_ingests_bell_source_with_fixed_message() {
        let (hub, events, _clock) = fixture();
        hub.on_bell("s1");

        let pending = hub.pending("s1");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].source, NotificationSource::Bell);
        assert_eq!(pending[0].message, "Terminal bell");
        assert_eq!(events.notifications().len(), 1);
    }

    #[test]
    fn on_bell_shares_the_same_dedup_pipeline_as_hook_ingest() {
        let (hub, events, clock) = fixture();
        hub.on_bell("s1"); // passes
        clock.advance(500);
        hub.on_bell("s1"); // within 3s window, suppressed

        assert_eq!(events.notifications().len(), 1);
        assert_eq!(hub.pending("s1").len(), 1);
    }

    // ---- purge_session ----

    #[test]
    fn purge_session_drops_pending_without_emitting_cleared() {
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("m1"));
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("m2"));
        assert_eq!(hub.pending("s1").len(), 2);

        hub.purge_session("s1");

        assert!(hub.pending("s1").is_empty());
        // Unlike clear(), purge_session (used on exit/dispose for
        // unresolved notifications) must NOT emit
        // notification-cleared — the session/agent may already be torn down.
        assert!(events.cleared().is_empty());
    }

    #[test]
    fn purge_session_on_unknown_session_is_a_noop() {
        let (hub, events, _clock) = fixture();
        hub.purge_session("does-not-exist"); // must not panic
        assert!(events.cleared().is_empty());
    }

    // ---- hook for unknown/dead session is discarded ----

    #[test]
    fn ingest_hook_for_unregistered_session_is_discarded() {
        let registry = Arc::new(SessionRegistry::new()); // "s1" never inserted
        let events = Arc::new(RecordingEvents::default());
        let clock = Arc::new(FakeClock::new());
        let hub = NotificationHub::new(registry, events.clone(), clock, Duration::from_millis(3000));

        hub.ingest_hook("s1", NotificationSource::Hook, &msg("need input"));

        assert!(hub.pending("s1").is_empty());
        assert!(events.notifications().is_empty());
    }

    #[test]
    fn ingest_hook_after_session_removed_from_registry_is_discarded() {
        // Simulates the actual trigger: Disposed sessions are removed from
        // the registry (see session/manager.rs on_exit), so a hook that
        // arrives afterward must resolve to nothing and be dropped.
        let registry = Arc::new(SessionRegistry::new());
        registry.insert("s1", "a1", SessionState::Running);
        let events = Arc::new(RecordingEvents::default());
        let clock = Arc::new(FakeClock::new());
        let hub = NotificationHub::new(registry.clone(), events.clone(), clock, Duration::from_millis(3000));

        hub.ingest_hook("s1", NotificationSource::Hook, &msg("first")); // registered, passes
        registry.remove("s1");
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("second")); // now unresolvable, discarded

        assert_eq!(events.notifications().len(), 1);
        assert_eq!(hub.pending("s1").len(), 1);
        assert_eq!(hub.pending("s1")[0].message, "first");
    }

    #[test]
    fn on_bell_for_unregistered_session_is_discarded() {
        let registry = Arc::new(SessionRegistry::new());
        let events = Arc::new(RecordingEvents::default());
        let clock = Arc::new(FakeClock::new());
        let hub = NotificationHub::new(registry, events.clone(), clock, Duration::from_millis(3000));

        hub.on_bell("unknown-session");

        assert!(events.notifications().is_empty());
    }

    // ---- ingest_activity: dedup/큐 우회, now_ms 타임스탬프 ----

    #[test]
    fn ingest_activity_emits_event_with_backend_timestamp_and_resolved_agent() {
        let (hub, events, clock) = fixture();
        clock.advance(500); // now_ms = 1_700_000_000_500

        hub.ingest_activity("s1", ActivityKind::Prompt);

        let acts = events.activities();
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].agent_id, "a1");
        assert_eq!(acts[0].session_id, "s1");
        assert_eq!(acts[0].kind, ActivityKind::Prompt);
        assert_eq!(acts[0].at, 1_700_000_000_500); // 백엔드 now_ms()

        // 알림 파이프라인은 전혀 오염되지 않는다.
        assert!(events.notifications().is_empty());
        assert!(hub.pending("s1").is_empty());
    }

    #[test]
    fn ingest_activity_bypasses_dedup_even_for_identical_repeats() {
        let (hub, events, _clock) = fixture();
        hub.ingest_activity("s1", ActivityKind::Tool);
        hub.ingest_activity("s1", ActivityKind::Tool); // 같은 종류 연속 — dedup 없이 둘 다 방출

        assert_eq!(events.activities().len(), 2);
        assert!(hub.pending("s1").is_empty());
    }

    #[test]
    fn ingest_activity_for_unregistered_session_is_discarded() {
        let registry = Arc::new(SessionRegistry::new()); // s1 미등록
        let events = Arc::new(RecordingEvents::default());
        let clock = Arc::new(FakeClock::new());
        let hub = NotificationHub::new(registry, events.clone(), clock, Duration::from_millis(3000));

        hub.ingest_activity("s1", ActivityKind::Prompt);

        assert!(events.activities().is_empty());
    }

    // ---- overhead-task-label: prompt 원문 추출 ----

    fn prompt_body(text: &str) -> Vec<u8> {
        serde_json::json!({ "session_id": "s1", "prompt": text }).to_string().into_bytes()
    }

    #[test]
    fn ingest_activity_with_body_extracts_and_truncates_prompt_text() {
        let (hub, events, _clock) = fixture();

        hub.ingest_activity_with_body("s1", ActivityKind::Prompt, &prompt_body("버그 고쳐줘"));
        let acts = events.activities();
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].kind, ActivityKind::Prompt);
        assert_eq!(acts[0].text.as_deref(), Some("버그 고쳐줘"));

        // 2,000자 초과 → chars 기준 절단 (멀티바이트 안전)
        let long: String = "가".repeat(2500);
        hub.ingest_activity_with_body("s1", ActivityKind::Prompt, &prompt_body(&long));
        let acts = events.activities();
        assert_eq!(acts[1].text.as_ref().unwrap().chars().count(), 2000);
    }

    #[test]
    fn ingest_activity_with_body_survives_bad_bodies() {
        let (hub, events, _clock) = fixture();
        // 비JSON / prompt 필드 부재 / 빈 문자열 → text=None, 이벤트는 방출된다.
        hub.ingest_activity_with_body("s1", ActivityKind::Prompt, b"not json");
        hub.ingest_activity_with_body("s1", ActivityKind::Prompt, br#"{"session_id":"s1"}"#);
        hub.ingest_activity_with_body("s1", ActivityKind::Prompt, br#"{"prompt":"   "}"#);
        let acts = events.activities();
        assert_eq!(acts.len(), 3);
        assert!(acts.iter().all(|a| a.text.is_none()));
    }

    #[test]
    fn ingest_activity_with_body_drops_command_prompts() {
        let (hub, events, _clock) = fixture();
        hub.ingest_activity_with_body("s1", ActivityKind::Prompt, &prompt_body("!git status"));
        hub.ingest_activity_with_body("s1", ActivityKind::Prompt, &prompt_body("/clear"));
        hub.ingest_activity_with_body("s1", ActivityKind::Prompt, &prompt_body("#메모"));
        let acts = events.activities();
        assert_eq!(acts.len(), 3);
        assert!(acts.iter().all(|a| a.text.is_none()));
    }

    #[test]
    fn plain_ingest_activity_keeps_text_none() {
        let (hub, events, _clock) = fixture();
        hub.ingest_activity("s1", ActivityKind::Tool);
        assert_eq!(events.activities()[0].text, None);
    }

    // ---- temporary legacy hook entry point fallback behavior ----

    #[test]
    fn observer_legacy_ingest_hook_prefers_body_message_field() {
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("custom text"));
        assert_eq!(events.notifications()[0].message, "custom text");
    }

    #[test]
    fn observer_legacy_ingest_hook_uses_neutral_stop_fallback() {
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Stop, b"{}");
        assert_eq!(events.notifications()[0].message, "작업이 완료되었습니다.");
    }

    #[test]
    fn observer_legacy_ingest_hook_uses_neutral_attention_fallback() {
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, b"{}");
        hub.ingest_hook("s1", NotificationSource::Bell, b"{}");
        assert_eq!(events.notifications()[0].message, "확인이 필요합니다");
        assert_eq!(events.notifications()[1].message, "확인이 필요합니다");
    }

    #[test]
    fn observer_legacy_ingest_hook_falls_back_on_blank_message_field() {
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, &msg("   "));
        assert_eq!(events.notifications()[0].message, "확인이 필요합니다");
    }

    #[test]
    fn observer_legacy_ingest_hook_falls_back_on_invalid_json_body() {
        let (hub, events, _clock) = fixture();
        hub.ingest_hook("s1", NotificationSource::Hook, b"not json");
        assert_eq!(events.notifications()[0].message, "확인이 필요합니다");
    }

    // ---- dedup_key sanity (used implicitly above; pin the algorithm's key inputs directly) ----

    #[test]
    fn dedup_key_differs_by_session_source_and_message() {
        let base = dedup_key("s1", NotificationSource::Hook, "hello");
        assert_ne!(base, dedup_key("s2", NotificationSource::Hook, "hello"));
        assert_ne!(base, dedup_key("s1", NotificationSource::Stop, "hello"));
        assert_ne!(base, dedup_key("s1", NotificationSource::Hook, "other"));
        assert_eq!(base, dedup_key("s1", NotificationSource::Hook, "hello")); // stable/deterministic
    }

    #[test]
    fn dedup_key_trims_message_whitespace() {
        assert_eq!(
            dedup_key("s1", NotificationSource::Hook, "hello"),
            dedup_key("s1", NotificationSource::Hook, "  hello  ")
        );
    }

    // Sanity check that Clock is genuinely an injection seam usable behind a
    // trait object (mirrors state.rs's analogous AppEvents check).
    #[test]
    fn fake_clock_is_usable_behind_the_clock_trait_object() {
        // Advance through the concrete type (only tests get to control
        // time), then erase to `Arc<dyn Clock>` — the same shape
        // `NotificationHub::new` requires in production — and confirm both
        // `now()` and `now_ms()` reflect the advance through the trait object.
        let clock = Arc::new(FakeClock::new());
        let ms_before = clock.now_ms();
        let instant_before = clock.now();
        clock.advance(2000);
        let handle: Arc<dyn Clock> = clock;

        assert_eq!(handle.now_ms() - ms_before, 2000);
        assert_eq!(handle.now().duration_since(instant_before), Duration::from_millis(2000));
    }
}
