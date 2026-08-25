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
use crate::persistence::claude_resume_store::ClaudeResumeStore;
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
    /// 동료 대화 한 마디(기본 no-op — 미러/테스트 구현은 무시해도 된다).
    fn talk_message(&self, _ev: &crate::types::TalkEvent) {}
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
    fn talk_message(&self, ev: &crate::types::TalkEvent) {
        let _ = self.app.emit("talk-message", ev);
    }
}

/// 두 방출 경계를 하나로 세운다(웹 원격 — 앱 이벤트 미러).
/// `AppEvents`가 이미 **모든 앱 이벤트의 단일 관문**이라, 여기 한 겹만 끼우면
/// 공유 중인 캐릭터의 상태·알림·활동이 그대로 뷰어로 미러된다 — 방출 지점마다
/// 훅을 추가할 필요가 없다. `primary`(=TauriEvents)가 먼저다.
pub struct CompositeEvents {
    pub primary: Arc<dyn AppEvents>,
    pub secondary: Arc<dyn AppEvents>,
}

impl CompositeEvents {
    pub fn new(primary: Arc<dyn AppEvents>, secondary: Arc<dyn AppEvents>) -> Self {
        Self { primary, secondary }
    }
}

impl AppEvents for CompositeEvents {
    fn session_started(&self, ev: &SessionStartedEvent) {
        self.primary.session_started(ev);
        self.secondary.session_started(ev);
    }
    fn session_state(&self, ev: &SessionStateEvent) {
        self.primary.session_state(ev);
        self.secondary.session_state(ev);
    }
    fn notification_new(&self, ev: &NotificationEvent) {
        self.primary.notification_new(ev);
        self.secondary.notification_new(ev);
    }
    fn talk_message(&self, ev: &crate::types::TalkEvent) {
        self.primary.talk_message(ev);
        self.secondary.talk_message(ev);
    }
    fn notification_cleared(&self, agent_id: &str, ids: &[String]) {
        self.primary.notification_cleared(agent_id, ids);
        self.secondary.notification_cleared(agent_id, ids);
    }
    fn activity_event(&self, ev: &ActivityEvent) {
        self.primary.activity_event(ev);
        self.secondary.activity_event(ev);
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
    /// (sessionId, agentId, state) 전체 스냅샷 — CLI 제어(#55) `list`가
    /// 프로필과 병합해 실행 중 세션의 상태를 보여주는 데 쓴다.
    pub fn snapshot(&self) -> Vec<(SessionId, AgentId, SessionState)> {
        self.map
            .read()
            .unwrap()
            .iter()
            .map(|(sid, (agent, state))| (sid.clone(), agent.clone(), *state))
            .collect()
    }
}

/// 봇 주입 프롬프트에 붙는 표식이 만료되는 시간(ms). 주입 직후 세션이 죽어
/// prompt 이벤트가 끝내 안 오면, 그 표식이 남아 **다음 사람 프롬프트**를
/// 봇으로 오염시킨다. 2분이면 CLI가 입력을 큐잉했다가 처리하는 지연을 다
/// 덮으면서, 사람이 다시 앉기 전에 확실히 만료된다.
pub const BOT_PROMPT_ARM_TTL_MS: u64 = 120_000;

/// 봇 주입 표식(agentId → armed_at epoch ms). `bot/runner.rs::inject`가
/// `write_input` 직전에 arm하고, `RecordingAppEvents`가 그 agent의 다음 prompt
/// 이벤트 하나로 **소비**한다.
///
/// 왜 세션이 아니라 이런 표식인가: 봇은 별도 세션을 띄우지 않고 이미 떠 있는
/// 터미널에 프롬프트를 밀어넣는다. 세션도 agentId도 사람이 쓸 때와 같아서
/// 세션 단위로는 구분할 수 없고, 유일한 구분선이 턴 단위다(kbm #2j8).
#[derive(Default)]
pub struct BotPromptArms {
    armed: std::sync::Mutex<HashMap<AgentId, u64>>,
}

impl BotPromptArms {
    pub fn new() -> Self {
        Self::default()
    }

    /// 다음 프롬프트 하나를 봇 것으로 표식한다(`now` = epoch ms). 이미 arm돼
    /// 있으면 시각만 갱신한다 — 주입이 연달아 두 번 들어가도 표식은 하나다.
    pub fn arm(&self, agent_id: &str, now: u64) {
        self.armed.lock().unwrap().insert(agent_id.to_string(), now);
    }

    /// 표식을 소비한다. 한 번 쓰면 지워지므로 두 번째 프롬프트는 사람 몫이다.
    /// `at`(프롬프트 이벤트 시각)이 arm 시각에서 TTL을 넘겼으면 만료로 보고
    /// 버린다(소비는 하되 봇으로 세지 않는다).
    pub fn consume(&self, agent_id: &str, at: u64) -> bool {
        let Some(armed_at) = self.armed.lock().unwrap().remove(agent_id) else {
            return false;
        };
        at.saturating_sub(armed_at) <= BOT_PROMPT_ARM_TTL_MS
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
    /// 초상 PNG 저장소. 웹 원격(`media.portrait` RPC)도 같은 인스턴스를 읽으므로
    /// Arc다 — 디렉터리 규약이 두 곳에 복제되지 않는다.
    pub portrait_store: Arc<crate::persistence::png_store::PngStore>,
    pub sprite_store: crate::persistence::png_store::PngStore,
    /// 서브에이전트 미니미 전용 PNG 저장소(`minimis/<agentId>.png`). 단일 N×N
    /// 프레임 하나만 담는다 — 없으면 부모 스프라이트 idle0 축소판을 쓴다.
    pub minimi_store: crate::persistence::png_store::PngStore,
    /// 세션 턴 시계열 로그(session-times.jsonl) — 턴이 종료될 때마다 append.
    pub session_time_store: crate::persistence::session_time_store::SessionTimeStore,
    /// 캐릭터 일기(#56) per-agent 로그(`diaries/<agentId>.jsonl`) — 일기 생성 시 append.
    pub diary_store: crate::persistence::diary_store::DiaryStore,
    /// 캐릭터 일기(#60) 작업 로그 스냅샷(`worklogs/<agentId>.json`) — 일기화 전까지
    /// 렌더러 버퍼를 디스크에 보존해 앱 재시작 후 복원한다. 렌더러가 쓰기를 주도한다.
    pub work_log_store: crate::persistence::work_log_store::WorkLogStore,
    /// 이 달의 우수사원 시상 저장소(`awards/awards.json` + `awards/portraits/<YYYY-MM>.png`).
    /// 확정은 write-once라 같은 달을 다시 확정해도 첫 레코드가 남는다.
    pub awards_store: crate::persistence::awards_store::AwardsStore,
    /// 에이전트별 포스트잇 메모(#79) 장 저장소(`memos/<agentId>/<sheetId>.txt`).
    /// 사람이 직접 쓰는 메모라 frontmatter + plain text로 보관한다.
    pub memo_store: crate::persistence::memo_store::MemoStore,
    /// Claude native 세션 ID(리줌) 스냅샷 스토어(`claude-resume.json`). observer
    /// ingest가 ClaudeResumeRecorder를 통해 쓰고, list_claude_resume_sessions가 읽는다.
    pub claude_resume_store: Arc<ClaudeResumeStore>,
    /// 앱 전역 opt-in 설정 — 디스크 원본은 settings_store, 커맨드가 읽는
    /// 캐시는 settings(RwLock). set_app_settings가 저장+캐시 갱신을 함께 한다.
    /// `Arc`인 이유: lib.rs의 observer URL getter가 SessionManager
    /// 생성 시점에 이 캐시를 미리 clone해 쥐고 있어야, 실행 중 ON→OFF 전환이
    /// (서버는 유지한 채) 새 세션 훅 배선에 즉시 반영된다.
    pub settings_store: SettingsStore,
    pub settings: Arc<RwLock<AppSettings>>,
    /// 세션 이벤트 시계열 루트(`<app-data>/session-events/v1`). 수집 측
    /// `SessionEventStore`가 쓰는 것과 같은 경로 — 분석 커맨드
    /// (`load_session_events`)가 reader로 읽기만 하려고 경로를 따로 보관한다
    /// (스토어는 쓰기 전용 원칙 유지, docs/session-analytics-design.md §4.1).
    pub session_event_root: std::path::PathBuf,
    /// 세션 로그 저장소 루트(`<app-data>/session-logs/v1`). 목록·열기·학습자료
    /// 커맨드가 여기서만 파일을 찾는다 — 경로 탈출 검증의 기준점이기도 하다
    /// (docs/session-log-design.md §6).
    pub session_log_root: std::path::PathBuf,
    /// 설정 `session_log_enabled`의 런타임 미러. SessionManager와 공유하는
    /// 같은 Arc라 `set_app_settings`가 여기만 갱신해도 기록 중인 세션에 즉시
    /// 반영된다.
    pub session_log_enabled: Arc<AtomicBool>,
    /// 부팅 시 settings.json 부재 여부 — 첫 실행 동의 다이얼로그 신호.
    /// `set_app_settings` 성공 시 false로 내려가야 웹뷰 리로드 후에도 첫
    /// 실행 다이얼로그가 다시 뜨지 않는다 -- `AtomicBool`로 이 갱신을 표현.
    pub settings_first_run: AtomicBool,
    /// Claude 사용량 실시간 조회(이슈 #33)의 메모리 상태. `load_usage_snapshot`
    /// 커맨드가 스로틀 판단·직전 성공 스냅샷을 여기 보관해, 렌더러 60초 폴링에
    /// 얹혀 리셋 경계 후 빠르게 실제 값을 갱신한다(docs/claude-usage-live-fetch-design.md).
    /// `Arc`인 이유: 웹 RPC(`usage.snapshot`)가 같은 스로틀 상태를 공유해야
    /// 폰 폴링이 중복 fetch를 일으키지 않는다(웹 호스팅 #7m).
    pub live_usage: Arc<crate::usage::LiveUsageState>,
    /// CLI 제어(#55, docs/cli-control-design.md)의 로컬 control 서버 상태.
    /// cli_enabled ON일 때만 기동되고 포트/토큰 파일을 관리한다.
    pub control_server: Arc<crate::control::ControlServerState>,
    /// control 핸들러가 쥐는 앱 상태 클론들. `set_app_settings`가 cli_enabled
    /// ON 전환 시 `control_server.ensure(control_ctx)`에 넘긴다.
    pub control_ctx: Arc<crate::control::ControlContext>,
    /// 웹 원격의 수신 서버 상태. `web_remote_enabled`가 켜져 있을 때만 뜬다.
    pub web_remote_server: Arc<crate::webremote::WebRemoteServerState>,
    /// 웹 원격 핸들러가 쥐는 앱 상태 클론 + 허브. 설정 토글 시 `ensure`에 넘긴다.
    pub web_remote_ctx: Arc<crate::webremote::WebRemoteContext>,
    /// 캐릭터 봇 모드(#57, docs/bot-mode-design.md)의 탭별 폴링 태스크 소유자.
    pub bot_runtime: Arc<crate::bot::BotRuntime>,
    /// 봇 폴링 태스크가 쥐는 앱 상태 클론(세션 주입·프로필/상태 접근).
    pub bot_ctx: Arc<crate::bot::runner::BotContext>,
    /// 작업 중 시스템 잠자기 방지(이슈 #68) 웨이크락 소유자. `set_keep_awake`
    /// 커맨드가 lease를 갱신/해제하고, lib.rs의 주기 감시 태스크가 lease 만료
    /// 시 강제 해제한다. 설정 `keep_awake_enabled`가 꺼져 있으면 무시된다.
    pub wake_lock: Arc<crate::power::WakeLock>,
    /// 확인 요청 대사 TTS의 런타임 상태 — API 키 스토어(0600 별도 파일),
    /// mp3 디스크 캐시, 보이스 목록 1회 캐시. 설정(`tts_enabled`/
    /// `tts_rewrite_model`)은 `settings`에 있고 **키는 여기에만** 있다
    /// (설정은 렌더러로 통째로 왕복하므로).
    pub tts: Arc<crate::tts::TtsState>,
    /// 동료 대화(docs/agent-talk-design.md)의 메시지 큐·대화 상태. control
    /// 핸들러와 배달 워커가 같은 Arc를 쥐고, `set_app_settings`가 토글·상한을
    /// 즉시 반영한다(끄면 대기 중 메시지까지 버려지는 킬 스위치).
    pub talk: Arc<crate::talk::TalkHub>,
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
            external: None,
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
            tokens: None,
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
            external: None,
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
            assistant_text: None,
            cwd: None,
            count: None,
        });
        events.activity_event(&ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: ActivityKind::Tool,
            at: 200,
            text: None,
            assistant_text: None,
            cwd: None,
            count: None,
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
