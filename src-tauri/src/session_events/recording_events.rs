use std::sync::Arc;

use crate::state::{AppEvents, BotPromptArms};
use crate::types::{
    ActivityEvent, ActivityKind, NotificationEvent, NotificationSource, SessionStateEvent,
    TurnUsageEvent,
};

use super::store::SessionEventStore;
use super::types::{PromptOrigin, SessionEventDraft, SessionEventKind, SessionStartedEvent};

pub struct RecordingAppEvents {
    inner: Arc<dyn AppEvents>,
    store: Arc<SessionEventStore>,
    /// 봇 주입 표식(`bot/runner.rs::inject`가 arm). prompt 이벤트가 소비한다.
    bot_arms: Arc<BotPromptArms>,
}

impl RecordingAppEvents {
    pub fn new(
        inner: Arc<dyn AppEvents>,
        store: Arc<SessionEventStore>,
        bot_arms: Arc<BotPromptArms>,
    ) -> Self {
        Self {
            inner,
            store,
            bot_arms,
        }
    }

    fn record(&self, draft: SessionEventDraft) {
        if let Err(error) = self.store.append(draft) {
            eprintln!(
                "agent-office: session event append failed under {}: {error}",
                self.store.root().display()
            );
        }
    }
}

impl AppEvents for RecordingAppEvents {
    fn session_started(&self, event: &SessionStartedEvent) {
        self.record(SessionEventDraft {
            agent_id: event.agent_id.clone(),
            session_id: event.session_id.clone(),
            kind: SessionEventKind::SessionStarted,
            at: event.at,
            agent_name: Some(event.agent_name.clone()),
            agent_role: event.agent_role.clone(),
            cwd: Some(event.cwd.clone()),
            shell: Some(event.shell.clone()),
            state: None,
            tokens: None,
            origin: None,
            partial: None,
        });
        self.inner.session_started(event);
    }

    fn session_state(&self, event: &SessionStateEvent) {
        let mut draft = SessionEventDraft::simple(
            event.agent_id.clone(),
            event.session_id.clone(),
            SessionEventKind::SessionState,
            event.at,
        );
        draft.state = Some(event.state);
        self.record(draft);
        self.inner.session_state(event);
    }

    /// 동료 대화는 세션 이벤트로 기록하지 않는다(자체 감사 로그가 있다) —
    /// 그대로 흘려보내기만 한다.
    fn talk_message(&self, event: &crate::types::TalkEvent) {
        self.inner.talk_message(event);
    }

    fn notification_new(&self, event: &NotificationEvent) {
        let kind = match event.source {
            NotificationSource::Hook => SessionEventKind::Notification,
            NotificationSource::Stop => SessionEventKind::Stop,
            NotificationSource::Bell => SessionEventKind::Bell,
        };
        let draft = SessionEventDraft::simple(
            event.agent_id.clone(),
            event.session_id.clone(),
            kind,
            event.at,
        );
        self.record(draft);
        self.inner.notification_new(event);
    }

    fn notification_cleared(&self, agent_id: &str, ids: &[String]) {
        self.inner.notification_cleared(agent_id, ids);
    }

    /// 한 턴의 토큰 사용량을 kind=Usage 레코드로 남긴다. 과거 파일은 이 값이
    /// kind=Stop 레코드에 실렸지만(알림에 업힌 계측), 지금은 `notification_new`가
    /// 더 이상 tokens를 만지지 않으므로 stop 레코드엔 안 실린다 — 소비자는
    /// kind가 아니라 tokens 유무로 합산해야 신구 파일을 모두 커버한다.
    ///
    /// `partial`도 그대로 싣는다(§11.9) — hub가 PostToolUse 중간 갱신에는
    /// `true`를, Stop에는 `false`를 실어 보낸다. 이걸 안 남기면 재부팅 후
    /// `aggregateSeed`가 과거 기록만 보고 "이 usage 레코드가 턴을 하나 닫은
    /// 것인지, 아직 진행 중인 턴의 중간 스냅샷인지"를 구분 못 해 턴 수가
    /// 부풀거나 유실된다.
    fn turn_usage(&self, event: &TurnUsageEvent) {
        let mut draft = SessionEventDraft::simple(
            event.agent_id.clone(),
            event.session_id.clone(),
            SessionEventKind::Usage,
            event.at,
        );
        draft.tokens = Some(event.tokens.clone());
        draft.partial = Some(event.partial);
        self.record(draft);
        self.inner.turn_usage(event);
    }

    fn activity_event(&self, event: &ActivityEvent) {
        // 서브에이전트 카운트 신호(SubStart/SubStop/SubCount)는 시각 효과 전용 —
        // 턴 시계열엔 기록하지 않고 렌더러 릴레이만 한다.
        let kind = match event.kind {
            ActivityKind::Prompt => Some(SessionEventKind::Prompt),
            ActivityKind::Tool => Some(SessionEventKind::Tool),
            // 서브에이전트 카운트 신호와 resume(이슈 #39, 출력 휴리스틱 복귀 신호),
            // idle(kbm #2f9, 셸 명령 종료 = 턴 정산 신호)은 렌더러 릴레이 전용 —
            // 시계열엔 기록하지 않는다(정산 결과는 세션 시간 로그가 이미 남긴다).
            ActivityKind::SubStart
            | ActivityKind::SubStop
            | ActivityKind::SubCount
            | ActivityKind::Resume
            | ActivityKind::Idle => None,
        };
        if let Some(kind) = kind {
            let mut draft = SessionEventDraft::simple(
                event.agent_id.clone(),
                event.session_id.clone(),
                kind,
                event.at,
            );
            // 턴을 여는 prompt만 출처를 갖는다. 표식은 한 번 쓰면 사라지므로
            // 뒤이은 사람 프롬프트는 사람 몫으로 남는다(kbm #2j8).
            if kind == SessionEventKind::Prompt
                && self.bot_arms.consume(&event.agent_id, event.at)
            {
                draft.origin = Some(PromptOrigin::Bot);
            }
            self.record(draft);
        }
        self.inner.activity_event(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_events::store::SessionEventStore;
    use crate::session_events::types::{SessionEventKind, SessionEventRecord, SessionStartedEvent};
    use crate::state::fake::RecordingEvents;
    use crate::state::{AppEvents, BOT_PROMPT_ARM_TTL_MS};
    use crate::types::{
        ActivityEvent, ActivityKind, NotificationEvent, NotificationSource, SessionState,
        SessionStateEvent,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    fn scratch_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-recording-events-{}",
            uuid::Uuid::new_v4()
        ))
    }

    fn read(root: &Path) -> Vec<SessionEventRecord> {
        fs::read_dir(root)
            .unwrap()
            .flat_map(|entry| {
                fs::read_to_string(entry.unwrap().path())
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn maps_events_without_sensitive_payloads_and_forwards_once() {
        let root = scratch_root();
        let inner = Arc::new(RecordingEvents::default());
        let store = Arc::new(SessionEventStore::new(root.clone()));
        let events = RecordingAppEvents::new(inner.clone(), store, Arc::new(BotPromptArms::new()));
        events.activity_event(&ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: ActivityKind::Prompt,
            at: 1_783_728_000_000,
            text: Some("do not persist this prompt".into()),
            assistant_text: None,
            cwd: None,
            count: None,
        });
        events.notification_new(&NotificationEvent {
            id: "n1".into(),
            session_id: "s1".into(),
            agent_id: "a1".into(),
            source: NotificationSource::Hook,
            message: "do not persist this message".into(),
            dedup_key: "do not persist this key".into(),
            at: 1_783_728_000_001,
        });
        let records = read(&root);
        assert_eq!(
            records.iter().map(|r| r.kind).collect::<Vec<_>>(),
            vec![SessionEventKind::Prompt, SessionEventKind::Notification,]
        );
        let raw = fs::read_to_string(root.join("2026-07-11.jsonl")).unwrap();
        assert!(!raw.contains("persist this"));
        assert!(!raw.contains("dedup"));
        assert_eq!(inner.activities().len(), 1);
        assert_eq!(inner.notifications().len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn maps_session_started_state_bell_stop_and_tool() {
        let root = scratch_root();
        let inner = Arc::new(RecordingEvents::default());
        let store = Arc::new(SessionEventStore::new(root.clone()));
        let events = RecordingAppEvents::new(inner, store, Arc::new(BotPromptArms::new()));
        events.session_started(&SessionStartedEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            agent_name: "Compiler".into(),
            agent_role: Some("Platform".into()),
            cwd: "/work".into(),
            shell: "/bin/zsh".into(),
            at: 1_783_728_000_000,
        });
        events.session_state(&SessionStateEvent {
            session_id: "s1".into(),
            agent_id: "a1".into(),
            state: SessionState::Running,
            exit: None,
            at: 1_783_728_000_001,
            external: None,
        });
        for (offset, source) in [NotificationSource::Bell, NotificationSource::Stop]
            .into_iter()
            .enumerate()
        {
            events.notification_new(&NotificationEvent {
                id: format!("n{offset}"),
                session_id: "s1".into(),
                agent_id: "a1".into(),
                source,
                message: String::new(),
                dedup_key: format!("k{offset}"),
                at: 1_783_728_000_002 + offset as u64,
            });
        }
        events.activity_event(&ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: ActivityKind::Tool,
            at: 1_783_728_000_004,
            text: None,
            assistant_text: None,
            cwd: None,
            count: None,
        });
        let records = read(&root);
        assert_eq!(
            records.iter().map(|r| r.kind).collect::<Vec<_>>(),
            vec![
                SessionEventKind::SessionStarted,
                SessionEventKind::SessionState,
                SessionEventKind::Bell,
                SessionEventKind::Stop,
                SessionEventKind::Tool,
            ]
        );
        assert_eq!(records[0].agent_name.as_deref(), Some("Compiler"));
        assert_eq!(records[0].agent_role.as_deref(), Some("Platform"));
        assert_eq!(records[0].cwd.as_deref(), Some("/work"));
        assert_eq!(records[0].shell.as_deref(), Some("/bin/zsh"));
        let _ = fs::remove_dir_all(root);
    }

    /// 턴 사용량은 `turn_usage`가 kind=Usage 레코드로 따로 남기고, `notification_new`가
    /// 만드는 stop 레코드에는 이제 tokens가 전혀 실리지 않는다 — 알림과 사용량이
    /// 서로 다른 채널이라는 계약을 시계열 기록 계층에서도 확인한다. `inner`를
    /// move하지 않고 `clone()`으로 넘겨야(≈189행 선례) `self.inner.turn_usage(event)`
    /// 전달까지 이 테스트가 실제로 검증한다 — move하면 `inner.usages()`를 볼 수
    /// 없어, 그 forward 호출을 지워도(=하위 홉이 조용히 사라져도) 이 테스트는
    /// 그린인 채로 남는다(필수 2).
    #[test]
    fn turn_usage_writes_a_usage_record_and_stop_stays_token_free() {
        use crate::types::SessionEventTokens;
        let root = scratch_root();
        let inner = Arc::new(RecordingEvents::default());
        let store = Arc::new(SessionEventStore::new(root.clone()));
        let events = RecordingAppEvents::new(inner.clone(), store, Arc::new(BotPromptArms::new()));
        let tokens = SessionEventTokens {
            input: Some(120),
            output: Some(340),
            cache_read: Some(9_000),
            cache_write: Some(50),
            model: Some("claude-opus-5".into()),
        };
        let usage_event = TurnUsageEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            at: 1_783_728_000_000,
            tokens: tokens.clone(),
            partial: false,
        };
        events.turn_usage(&usage_event);
        assert_eq!(read(&root)[0].partial, Some(false));
        events.notification_new(&NotificationEvent {
            id: "n0".into(),
            session_id: "s1".into(),
            agent_id: "a1".into(),
            source: NotificationSource::Stop,
            message: String::new(),
            dedup_key: "k0".into(),
            at: 1_783_728_000_001,
        });
        let records = read(&root);
        assert_eq!(records[0].kind, SessionEventKind::Usage);
        assert_eq!(records[0].tokens.as_ref(), Some(&tokens));
        assert_eq!(records[1].kind, SessionEventKind::Stop);
        assert_eq!(records[1].tokens, None);
        // stop 레코드엔 "tokens" 키 자체가 나가지 않는다(과거 파일과 동형이 아니라
        // 이제 신규 stop은 원천적으로 tokens를 안 만든다는 확인).
        let raw = fs::read_to_string(root.join("2026-07-11.jsonl")).unwrap();
        assert_eq!(raw.lines().count(), 2);
        assert!(!raw.lines().last().unwrap().contains("tokens"));
        // `self.inner.turn_usage(event)` 전달이 실제로 일어났는지 — 하위 홉 확인.
        let forwarded = inner.usages();
        assert_eq!(forwarded.len(), 1);
        assert_eq!(forwarded[0].agent_id, usage_event.agent_id);
        assert_eq!(forwarded[0].at, usage_event.at);
        assert_eq!(forwarded[0].tokens, usage_event.tokens);
        let _ = fs::remove_dir_all(root);
    }

    /// 턴 중간 갱신(§11.9): PostToolUse가 낸 `partial: true` usage 이벤트도
    /// kind=Usage 레코드에 그 값 그대로 남는다 — 재부팅 시드가 이 레코드를
    /// "이미 끝난 턴"으로 잘못 세면 안 되기 때문에 partial 자체가 파일에
    /// 남아야 한다.
    #[test]
    fn turn_usage_persists_the_partial_flag_for_mid_turn_updates() {
        use crate::types::SessionEventTokens;
        let root = scratch_root();
        let inner = Arc::new(RecordingEvents::default());
        let store = Arc::new(SessionEventStore::new(root.clone()));
        let events = RecordingAppEvents::new(inner, store, Arc::new(BotPromptArms::new()));
        events.turn_usage(&TurnUsageEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            at: 1_783_728_000_000,
            tokens: SessionEventTokens {
                input: Some(5),
                ..Default::default()
            },
            partial: true,
        });
        assert_eq!(read(&root)[0].partial, Some(true));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preserves_every_session_state_value() {
        let root = scratch_root();
        let inner = Arc::new(RecordingEvents::default());
        let store = Arc::new(SessionEventStore::new(root.clone()));
        let events = RecordingAppEvents::new(inner, store, Arc::new(BotPromptArms::new()));
        let states = [
            SessionState::Starting,
            SessionState::Running,
            SessionState::Exited,
            SessionState::Disposed,
        ];
        for (offset, state) in states.into_iter().enumerate() {
            events.session_state(&SessionStateEvent {
                session_id: "s1".into(),
                agent_id: "a1".into(),
                state,
                exit: None,
                at: 1_783_728_000_000 + offset as u64,
                external: None,
            });
        }
        assert_eq!(
            read(&root)
                .iter()
                .map(|record| record.state.unwrap())
                .collect::<Vec<_>>(),
            states,
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn subagent_activity_is_relayed_but_not_recorded_as_session_event() {
        let root = scratch_root();
        fs::create_dir_all(&root).unwrap();
        let inner = Arc::new(RecordingEvents::default());
        let store = Arc::new(SessionEventStore::new(root.clone()));
        let events = RecordingAppEvents::new(inner.clone(), store, Arc::new(BotPromptArms::new()));
        events.activity_event(&ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: ActivityKind::SubStart,
            at: 1_783_728_000_000,
            text: None,
            assistant_text: None,
            cwd: None,
            count: None,
        });
        // inner(렌더러 릴레이)로는 전달된다.
        assert_eq!(inner.activities().len(), 1);
        assert_eq!(inner.activities()[0].kind, ActivityKind::SubStart);
        // 시계열 스토어에는 기록되지 않는다(Prompt/Tool만 기록).
        assert!(
            read(&root).is_empty(),
            "서브 신호는 시계열 기록 대상이 아니다"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn append_failure_does_not_block_forwarding() {
        let root = scratch_root();
        fs::write(&root, b"not a directory").unwrap();
        let inner = Arc::new(RecordingEvents::default());
        let store = Arc::new(SessionEventStore::new(root.clone()));
        let events = RecordingAppEvents::new(inner.clone(), store, Arc::new(BotPromptArms::new()));
        events.activity_event(&ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: ActivityKind::Tool,
            at: 1_783_728_000_000,
            text: None,
            assistant_text: None,
            cwd: None,
            count: None,
        });
        assert_eq!(inner.activities().len(), 1);
        fs::remove_file(root).unwrap();
    }

    // ── 봇 주입 표식(kbm #2j8) ────────────────────────────────────────────

    fn prompt(agent_id: &str, at: u64) -> ActivityEvent {
        ActivityEvent {
            agent_id: agent_id.into(),
            session_id: "s1".into(),
            kind: ActivityKind::Prompt,
            at,
            text: None,
            assistant_text: None,
            cwd: None,
            count: None,
        }
    }

    /// arm된 뒤 **처음 하나**만 봇이다 — 봇 주입은 1회 프롬프트이고, 그 뒤
    /// 사람이 이어서 치는 프롬프트까지 봇으로 새면 안 된다.
    #[test]
    fn armed_agent_marks_only_the_next_prompt_as_bot() {
        let root = scratch_root();
        let inner = Arc::new(RecordingEvents::default());
        let store = Arc::new(SessionEventStore::new(root.clone()));
        let arms = Arc::new(BotPromptArms::new());
        let events = RecordingAppEvents::new(inner, store, arms.clone());

        arms.arm("a1", 1_783_728_000_000);
        events.activity_event(&prompt("a1", 1_783_728_000_010));
        events.activity_event(&prompt("a1", 1_783_728_000_020));

        let records = read(&root);
        assert_eq!(
            records.iter().map(|r| r.origin).collect::<Vec<_>>(),
            vec![Some(PromptOrigin::Bot), None]
        );
        let _ = fs::remove_dir_all(root);
    }

    /// 주입 직후 세션이 죽어 프롬프트가 끝내 안 오면, 남은 표식이 한참 뒤의
    /// 사람 프롬프트를 오염시키면 안 된다.
    #[test]
    fn expired_arm_is_dropped_instead_of_marking_a_later_human_prompt() {
        let root = scratch_root();
        let inner = Arc::new(RecordingEvents::default());
        let store = Arc::new(SessionEventStore::new(root.clone()));
        let arms = Arc::new(BotPromptArms::new());
        let events = RecordingAppEvents::new(inner, store, arms.clone());

        let armed_at = 1_783_728_000_000;
        arms.arm("a1", armed_at);
        events.activity_event(&prompt("a1", armed_at + BOT_PROMPT_ARM_TTL_MS + 1));

        let records = read(&root);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].origin, None);
        let _ = fs::remove_dir_all(root);
    }

    /// TTL 경계 정확히 위(=만료 아님)까지는 봇으로 인정한다.
    #[test]
    fn arm_at_exactly_the_ttl_boundary_still_counts_as_bot() {
        let root = scratch_root();
        let inner = Arc::new(RecordingEvents::default());
        let store = Arc::new(SessionEventStore::new(root.clone()));
        let arms = Arc::new(BotPromptArms::new());
        let events = RecordingAppEvents::new(inner, store, arms.clone());

        let armed_at = 1_783_728_000_000;
        arms.arm("a1", armed_at);
        events.activity_event(&prompt("a1", armed_at + BOT_PROMPT_ARM_TTL_MS));

        let records = read(&root);
        assert_eq!(records[0].origin, Some(PromptOrigin::Bot));
        let _ = fs::remove_dir_all(root);
    }

    /// 표식은 agent별이다 — 봇 탭의 arm이 옆자리 사람 프롬프트로 새면 안 된다.
    #[test]
    fn arm_does_not_leak_to_another_agent() {
        let root = scratch_root();
        let inner = Arc::new(RecordingEvents::default());
        let store = Arc::new(SessionEventStore::new(root.clone()));
        let arms = Arc::new(BotPromptArms::new());
        let events = RecordingAppEvents::new(inner, store, arms.clone());

        arms.arm("a1", 1_783_728_000_000);
        events.activity_event(&prompt("a2", 1_783_728_000_010));
        events.activity_event(&prompt("a1", 1_783_728_000_020));

        let records = read(&root);
        let by_agent: Vec<(String, Option<PromptOrigin>)> = records
            .iter()
            .map(|r| (r.agent_id.clone(), r.origin))
            .collect();
        assert_eq!(
            by_agent,
            vec![
                ("a2".to_string(), None),
                ("a1".to_string(), Some(PromptOrigin::Bot))
            ]
        );
        let _ = fs::remove_dir_all(root);
    }

    /// 사람 프롬프트에는 `origin` **키 자체가** 나가지 않는다(옵션 추가라
    /// schemaVersion 1을 유지하는 근거 — 과거 파일과 바이트 모양이 같다).
    #[test]
    fn human_prompt_omits_the_origin_key_entirely() {
        let root = scratch_root();
        let inner = Arc::new(RecordingEvents::default());
        let store = Arc::new(SessionEventStore::new(root.clone()));
        let events = RecordingAppEvents::new(inner, store, Arc::new(BotPromptArms::new()));

        events.activity_event(&prompt("a1", 1_783_728_000_000));

        let raw = fs::read_to_string(root.join("2026-07-11.jsonl")).unwrap();
        assert!(!raw.contains("origin"));
        let _ = fs::remove_dir_all(root);
    }

    /// 봇 프롬프트는 `"origin":"bot"`으로 나간다(TS 미러 `origin?: "bot"`).
    #[test]
    fn bot_prompt_serializes_origin_as_snake_case_bot() {
        let root = scratch_root();
        let inner = Arc::new(RecordingEvents::default());
        let store = Arc::new(SessionEventStore::new(root.clone()));
        let arms = Arc::new(BotPromptArms::new());
        let events = RecordingAppEvents::new(inner, store, arms.clone());

        arms.arm("a1", 1_783_728_000_000);
        events.activity_event(&prompt("a1", 1_783_728_000_000));

        let raw = fs::read_to_string(root.join("2026-07-11.jsonl")).unwrap();
        assert!(raw.contains("\"origin\":\"bot\""));
        let _ = fs::remove_dir_all(root);
    }

    /// prompt가 아닌 종류는 표식을 소비하지 않는다 — 봇 주입과 프롬프트 사이에
    /// 도구 이벤트가 끼어도 표식이 엉뚱한 곳에서 사라지면 안 된다.
    #[test]
    fn non_prompt_events_do_not_consume_the_arm() {
        let root = scratch_root();
        let inner = Arc::new(RecordingEvents::default());
        let store = Arc::new(SessionEventStore::new(root.clone()));
        let arms = Arc::new(BotPromptArms::new());
        let events = RecordingAppEvents::new(inner, store, arms.clone());

        arms.arm("a1", 1_783_728_000_000);
        let mut tool = prompt("a1", 1_783_728_000_005);
        tool.kind = ActivityKind::Tool;
        events.activity_event(&tool);
        events.activity_event(&prompt("a1", 1_783_728_000_010));

        let records = read(&root);
        assert_eq!(
            records.iter().map(|r| r.origin).collect::<Vec<_>>(),
            vec![None, Some(PromptOrigin::Bot)]
        );
        let _ = fs::remove_dir_all(root);
    }
}
