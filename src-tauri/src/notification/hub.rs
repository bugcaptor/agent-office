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

// ── 출력 기반 "아직 작업중" 복귀 휴리스틱 상수(이슈 #39) ────────────────
//
// 완료(Stop) 알림 직후에도 PTY 출력이 계속 쏟아지면 사실 아직 작업 중이므로,
// 그 세션의 stop 알림을 걷어내고 렌더러 턴을 working 으로 되돌린다. 임계치를
// 크게 잡는 이유는 사용자 키 입력 에코/프롬프트 리드로우로 인한 오탐 방지.
/// Stop 직후 이 시간 안의 출력은 프롬프트 리드로우로 보고 무시하는 유예기간.
const RESUME_GRACE: Duration = Duration::from_secs(3);
/// Stop 이후 이 시간까지만 복귀 후보로 감시한다(넘으면 감시 종료).
const RESUME_WINDOW: Duration = Duration::from_secs(30);
/// 감시창 내 누적 출력이 이 바이트를 넘으면 1회 복귀 신호를 낸다.
const RESUME_THRESHOLD_BYTES: usize = 8 * 1024;

// ── 오토모드 질문 알림 홀드 상수(이슈 #41) ─────────────────────────────
//
// 오토모드에서 에이전트의 질문(Hook)이 자동 승인되는데도 느낌표 알림이 즉시
// 떠버린다. 그래서 Hook 알림을 hold_duration 만큼 보류했다가, 그 사이 세션이
// 계속 일한다는 신호(프롬프트·도구·서브에이전트·출력 폭주 등)가 오면 조용히
// 폐기하고, 신호가 없으면 그때 방출한다. 홀드 지속시간 자체는 상수가 아니라
// 설정(attention_hold_ms) 주입값이다.
/// 홀드 시작 직후 이 시간 안의 출력은 질문 UI 자체 렌더링으로 보고 무시한다.
const HOLD_OUTPUT_GRACE: Duration = Duration::from_secs(1);
/// grace 이후 누적 출력이 이 바이트를 넘으면 "아직 작업중"으로 보고 홀드를 폐기한다.
const HOLD_OUTPUT_THRESHOLD_BYTES: usize = 8 * 1024;

/// 세션별 보류 중인 질문 알림(이슈 #41). 세션당 최대 1개.
struct HeldNotification {
    /// 보류된 알림 이벤트(방출 시 그대로 큐/이벤트로 나간다).
    ev: NotificationEvent,
    /// 보류 시작 시각(clock.now()). 만료·grace 판정 기준.
    held_at: Instant,
    /// grace 이후 누적 출력 바이트(출력 폭주 폐기 판정용).
    accumulated: usize,
}

/// 세션별 Stop-후 출력 감시 상태(이슈 #39 출력 휴리스틱).
struct ResumeWatch {
    /// 마지막 Stop 알림 시각(clock.now()).
    stop_at: Instant,
    /// grace 이후 감시창 내 누적 출력 바이트.
    accumulated: usize,
    /// 복귀 신호를 이미 1회 냈는지(중복 방지).
    fired: bool,
}

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
    /// Stop-후 출력 감시(이슈 #39). Stop 알림이 실제 방출될 때만 엔트리가 생긴다.
    resume_watch: Mutex<HashMap<SessionId, ResumeWatch>>,
    resume_grace: Duration,
    resume_window: Duration,
    resume_threshold: usize,
    /// 오토모드 질문 알림 홀드(이슈 #41). 세션당 최대 1개.
    held: Mutex<HashMap<SessionId, HeldNotification>>,
    /// Hook 알림 보류 시간. 0이면 즉시 방출(현행 동작 = 기존 테스트 불변).
    hold_duration: Mutex<Duration>,
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
            resume_watch: Mutex::new(HashMap::new()),
            resume_grace: RESUME_GRACE,
            resume_window: RESUME_WINDOW,
            resume_threshold: RESUME_THRESHOLD_BYTES,
            held: Mutex::new(HashMap::new()),
            // 기본 0 = 즉시 방출. 실제 값은 lib.rs가 설정 로드 직후
            // set_hold_duration으로 주입한다(기존 테스트는 0으로 현행 동작 유지).
            hold_duration: Mutex::new(Duration::ZERO),
        }
    }

    /// Hook 알림 보류 시간을 주입한다(이슈 #41). 설정 로드/변경 시 lib.rs·
    /// set_app_settings가 호출. 0이면 즉시 방출(홀드 비활성).
    pub fn set_hold_duration(&self, d: Duration) {
        *self.hold_duration.lock().unwrap() = d;
    }

    /// 테스트 전용: 출력 휴리스틱 임계치를 주입한다(FakeClock 과 함께 결정론적 검증).
    #[cfg(test)]
    pub fn with_resume_params(
        mut self,
        grace: Duration,
        window: Duration,
        threshold: usize,
    ) -> Self {
        self.resume_grace = grace;
        self.resume_window = window;
        self.resume_threshold = threshold;
        self
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
        self.ingest_activity_inner(session_id, kind, None, None, None);
    }

    /// prompt 훅 전용: body(UserPromptSubmit 이벤트 JSON)에서 원문을 추출해 싣는다.
    /// 파싱 실패는 text=None으로 강등될 뿐, 이벤트 방출 자체는 항상 일어난다.
    pub fn ingest_activity_with_body(&self, session_id: &str, kind: ActivityKind, body: &[u8]) {
        self.ingest_activity_inner(session_id, kind, prompt_text(body), None, None);
    }

    pub fn ingest_observer(&self, session_id: &str, event: ObserverEvent) {
        match event {
            ObserverEvent::Prompt { text, cwd } => {
                self.ingest_activity_inner(session_id, ActivityKind::Prompt, text, None, cwd)
            }
            ObserverEvent::Tool { text, assistant } => {
                self.ingest_activity_inner(session_id, ActivityKind::Tool, text, assistant, None)
            }
            ObserverEvent::SubStart => self.ingest_activity(session_id, ActivityKind::SubStart),
            ObserverEvent::SubStop => self.ingest_activity(session_id, ActivityKind::SubStop),
            ObserverEvent::SubCount { running } => self.ingest_subagent_count(session_id, running),
            ObserverEvent::Attention { message } => self.ingest(
                session_id,
                NotificationSource::Hook,
                message
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| ATTENTION_FALLBACK.to_string()),
            ),
            ObserverEvent::Stop { message, running } => {
                let running = running.unwrap_or(0);
                // 이슈 #41: 턴이 종료되면(자동답변 후 케이스 포함) 보류 중인 질문
                // 알림을 폐기한다 — 질문+완료 이중 알림 방지. running 값과 무관.
                self.held.lock().unwrap().remove(session_id);
                self.ingest_subagent_count(session_id, running);
                // 백그라운드 서브에이전트가 아직 도는 중의 Stop은 턴 경계일 뿐
                // 완료가 아니다 — 알림을 내지 않는다(이슈 #27). 렌더러의 턴
                // 정산도 이 알림 이벤트로만 일어나므로, 억제하면 서브에이전트가
                // 일하는 동안 "일하는 중" 표시가 유지된다(이슈 #25). 최종
                // Stop(running=0)에서만 알림·정산한다.
                if running == 0 {
                    self.ingest(
                        session_id,
                        NotificationSource::Stop,
                        message
                            .filter(|value| !value.trim().is_empty())
                            .unwrap_or_else(|| STOP_FALLBACK.to_string()),
                    )
                }
            }
        }
    }

    fn ingest_activity_inner(
        &self,
        session_id: &str,
        kind: ActivityKind,
        text: Option<String>,
        assistant: Option<String>,
        cwd: Option<String>,
    ) {
        // 이슈 #41: 세션이 계속 일한다는 신호(프롬프트 제출·도구 사용·서브에이전트
        // 시작)가 오면 보류 중인 질문 알림을 조용히 폐기한다. SubStop/SubCount/
        // Resume 은 취소 신호가 아니다.
        if matches!(
            kind,
            ActivityKind::Prompt | ActivityKind::Tool | ActivityKind::SubStart
        ) {
            self.held.lock().unwrap().remove(session_id);
        }
        let Some(agent_id) = self.registry.resolve_agent(session_id) else {
            return;
        };
        let ev = ActivityEvent {
            agent_id,
            session_id: session_id.to_string(),
            kind,
            at: self.clock.now_ms(),
            text,
            assistant_text: assistant,
            cwd,
            count: None,
        };
        self.events.activity_event(&ev);
    }

    fn ingest_subagent_count(&self, session_id: &str, running: u32) {
        let Some(agent_id) = self.registry.resolve_agent(session_id) else {
            return;
        };
        let ev = ActivityEvent {
            agent_id,
            session_id: session_id.to_string(),
            kind: ActivityKind::SubCount,
            at: self.clock.now_ms(),
            text: None,
            assistant_text: None,
            cwd: None,
            count: Some(running),
        };
        self.events.activity_event(&ev);
    }

    /// BEL 폴백: output pump가 0x07 감지 시.
    pub fn on_bell(&self, session_id: &str) {
        self.ingest(
            session_id,
            NotificationSource::Bell,
            "Terminal bell".to_string(),
        );
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
            // 이슈 #41: Hook 소스는 ingest가 아니라 실제 방출 시점(emit)에만
            // last_seen 을 남긴다 — 홀드가 폐기된 질문이 dedup 윈도우 안에 다시
            // 와도 알림이 나가야 하므로. Stop/Bell 은 현행대로 ingest 시 기록한다.
            if source != NotificationSource::Hook {
                ls.insert(key.clone(), now_i);
            }
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

        // 이슈 #41: 오토모드 홀드. Hook 알림은 hold_duration 동안 보류했다가
        // flush_expired 가 방출하거나, 취소 신호가 오면 조용히 폐기한다.
        let hold = *self.hold_duration.lock().unwrap();
        if source == NotificationSource::Hook && hold > Duration::ZERO {
            let mut held = self.held.lock().unwrap();
            match held.get(session_id) {
                // 같은 질문의 재수신 — 원래 타이머를 유지한다(무시).
                Some(existing) if existing.ev.dedup_key == ev.dedup_key => {}
                // 세션당 최대 1개: 새 질문이 이전 질문을 대체한다(타이머 리셋).
                _ => {
                    held.insert(
                        session_id.to_string(),
                        HeldNotification {
                            ev,
                            held_at: now_i,
                            accumulated: 0,
                        },
                    );
                }
            }
            return;
        }

        self.emit(ev);
    }

    /// 큐 push + notification_new 방출. Hook 즉시 방출과 flush_expired 가 공유한다.
    /// 실제 방출 시점에 last_seen 을 남겨(Hook 지연 방출 포함) 이후 같은 메시지의
    /// dedup 억제 기준을 방출 시각으로 잡는다(이슈 #41). Stop 이면 출력 감시를 건다(#39).
    fn emit(&self, ev: NotificationEvent) {
        let now_i = self.clock.now();
        self.last_seen
            .lock()
            .unwrap()
            .insert(ev.dedup_key.clone(), now_i);
        let source = ev.source;
        let session_id = ev.session_id.clone();
        self.queues
            .lock()
            .unwrap()
            .entry(session_id.clone())
            .or_default()
            .push(ev.clone());
        self.events.notification_new(&ev);

        // 이슈 #39: 완료 알림이 실제 방출된 시점부터 출력 감시를 시작한다.
        // dedup 으로 억제된 중복 Stop 은 여기 오지 않으므로 감시창이 리셋되지 않는다.
        if source == NotificationSource::Stop {
            self.resume_watch.lock().unwrap().insert(
                session_id,
                ResumeWatch {
                    stop_at: now_i,
                    accumulated: 0,
                    fired: false,
                },
            );
        }
    }

    /// 스위퍼가 500ms 간격으로 호출(이슈 #41). 보류 시간이 지난 held 알림을
    /// 방출한다 — 세션이 registry 에서 사라졌으면(사망) 조용히 폐기한다.
    pub fn flush_expired(&self) {
        let hold = *self.hold_duration.lock().unwrap();
        let now = self.clock.now();
        let ready: Vec<HeldNotification> = {
            let mut held = self.held.lock().unwrap();
            let expired: Vec<SessionId> = held
                .iter()
                .filter(|(_, h)| now.duration_since(h.held_at) >= hold)
                .map(|(sid, _)| sid.clone())
                .collect();
            expired
                .into_iter()
                .filter_map(|sid| held.remove(&sid))
                .collect()
        };
        for h in ready {
            // 세션 사망 → 조용히 폐기(죽은 세션의 알림은 내지 않는다).
            if self.registry.resolve_agent(&h.ev.session_id).is_some() {
                self.emit(h.ev);
            }
        }
    }

    /// output pump 가 세션 PTY 출력 배치를 넘길 때마다 호출(이슈 #39). Stop 이후
    /// grace 를 지나 window 내에 누적 출력이 임계치를 넘으면 1회만: 그 세션의
    /// stop 알림을 걷어내고(notification-cleared 재사용) "아직 작업중" 복귀 신호
    /// (resume activity)를 낸다. Stop 감시 중이 아닌 세션은 즉시 반환(핫 패스).
    pub fn on_output(&self, session_id: &str, byte_len: usize) {
        let now = self.clock.now();

        // 이슈 #41: 보류 중인 질문이 있으면 출력 폭주로 폐기 판정을 먼저 한다.
        // grace 내 출력은 질문 UI 자체 렌더링으로 보고 무시하고, grace 이후
        // 누적이 임계치를 넘으면 세션이 계속 일하는 것으로 보고 홀드를 폐기한다.
        // 아래 resume_watch(#39) 경로는 이 체크와 무관하게 그대로 실행된다.
        {
            let mut held = self.held.lock().unwrap();
            if let Some(h) = held.get_mut(session_id) {
                if now.duration_since(h.held_at) >= HOLD_OUTPUT_GRACE {
                    h.accumulated = h.accumulated.saturating_add(byte_len);
                    if h.accumulated >= HOLD_OUTPUT_THRESHOLD_BYTES {
                        held.remove(session_id);
                    }
                }
            }
        }

        let should_resume = {
            let mut watches = self.resume_watch.lock().unwrap();
            let Some(watch) = watches.get_mut(session_id) else {
                return;
            };
            if watch.fired {
                return;
            }
            let elapsed = now.duration_since(watch.stop_at);
            if elapsed < self.resume_grace {
                // 프롬프트 리드로우 구간 — 무시.
                return;
            }
            if elapsed > self.resume_window {
                // 감시창 종료 — 정리하고 끝.
                watches.remove(session_id);
                return;
            }
            watch.accumulated = watch.accumulated.saturating_add(byte_len);
            if watch.accumulated < self.resume_threshold {
                return;
            }
            watch.fired = true;
            true
        };
        if should_resume {
            self.clear_stop_notifications(session_id);
            self.ingest_activity(session_id, ActivityKind::Resume);
        }
    }

    /// 해당 세션의 stop 소스 알림만 걷어낸다(clear 경로 재사용 → notification-cleared).
    fn clear_stop_notifications(&self, session_id: &str) {
        let ids: Vec<String> = {
            let queues = self.queues.lock().unwrap();
            match queues.get(session_id) {
                Some(list) => list
                    .iter()
                    .filter(|e| e.source == NotificationSource::Stop)
                    .map(|e| e.id.clone())
                    .collect(),
                None => Vec::new(),
            }
        };
        if !ids.is_empty() {
            self.clear(session_id, Some(ids));
        }
    }

    pub fn pending(&self, session_id: &str) -> Vec<NotificationEvent> {
        self.queues
            .lock()
            .unwrap()
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    /// 터미널 열림 시 클리어. ids 없으면 세션 전체. cleared된 id 방출.
    pub fn clear(&self, session_id: &str, ids: Option<Vec<String>>) -> Vec<String> {
        // 이슈 #41: 세션 전체 clear(터미널 열림)면 보류 중인 질문도 폐기한다.
        // 부분 clear(Some(ids))는 held 를 건드리지 않는다.
        if ids.is_none() {
            self.held.lock().unwrap().remove(session_id);
        }
        let cleared: Vec<String> = {
            let mut q = self.queues.lock().unwrap();
            let Some(list) = q.get_mut(session_id) else {
                return Vec::new();
            };
            match ids {
                Some(ids) if !ids.is_empty() => {
                    let set: std::collections::HashSet<_> = ids.into_iter().collect();
                    let hit: Vec<String> = list
                        .iter()
                        .filter(|e| set.contains(&e.id))
                        .map(|e| e.id.clone())
                        .collect();
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
        self.resume_watch.lock().unwrap().remove(session_id);
        // 이슈 #41: 세션 정리 시 보류 중인 질문도 함께 버린다(flush 전 잔류 방지).
        self.held.lock().unwrap().remove(session_id);
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
            Self {
                base: Instant::now(),
                offset_ms: AtomicU64::new(0),
            }
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
mod tests;
