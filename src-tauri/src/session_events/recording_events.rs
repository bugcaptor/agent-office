use std::sync::Arc;

use crate::state::AppEvents;
use crate::types::{
    ActivityEvent, ActivityKind, NotificationEvent, NotificationSource, SessionStateEvent,
};

use super::store::SessionEventStore;
use super::types::{SessionEventDraft, SessionEventKind, SessionStartedEvent};

pub struct RecordingAppEvents {
    inner: Arc<dyn AppEvents>,
    store: Arc<SessionEventStore>,
}

impl RecordingAppEvents {
    pub fn new(inner: Arc<dyn AppEvents>, store: Arc<SessionEventStore>) -> Self {
        Self { inner, store }
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

    fn notification_new(&self, event: &NotificationEvent) {
        let kind = match event.source {
            NotificationSource::Hook => SessionEventKind::Notification,
            NotificationSource::Stop => SessionEventKind::Stop,
            NotificationSource::Bell => SessionEventKind::Bell,
        };
        self.record(SessionEventDraft::simple(
            event.agent_id.clone(),
            event.session_id.clone(),
            kind,
            event.at,
        ));
        self.inner.notification_new(event);
    }

    fn notification_cleared(&self, agent_id: &str, ids: &[String]) {
        self.inner.notification_cleared(agent_id, ids);
    }

    fn activity_event(&self, event: &ActivityEvent) {
        // 서브에이전트 카운트 신호(SubStart/SubStop/SubCount)는 시각 효과 전용 —
        // 턴 시계열엔 기록하지 않고 렌더러 릴레이만 한다.
        let kind = match event.kind {
            ActivityKind::Prompt => Some(SessionEventKind::Prompt),
            ActivityKind::Tool => Some(SessionEventKind::Tool),
            // 서브에이전트 카운트 신호와 resume(이슈 #39, 출력 휴리스틱 복귀 신호)은
            // 렌더러 릴레이 전용 — 시계열엔 기록하지 않는다.
            ActivityKind::SubStart
            | ActivityKind::SubStop
            | ActivityKind::SubCount
            | ActivityKind::Resume => None,
        };
        if let Some(kind) = kind {
            self.record(SessionEventDraft::simple(
                event.agent_id.clone(),
                event.session_id.clone(),
                kind,
                event.at,
            ));
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
    use crate::state::AppEvents;
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
        let events = RecordingAppEvents::new(inner.clone(), store);
        events.activity_event(&ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: ActivityKind::Prompt,
            at: 1_783_728_000_000,
            text: Some("do not persist this prompt".into()),
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
        let events = RecordingAppEvents::new(inner, store);
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

    #[test]
    fn preserves_every_session_state_value() {
        let root = scratch_root();
        let inner = Arc::new(RecordingEvents::default());
        let store = Arc::new(SessionEventStore::new(root.clone()));
        let events = RecordingAppEvents::new(inner, store);
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
        let events = RecordingAppEvents::new(inner.clone(), store);
        events.activity_event(&ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: ActivityKind::SubStart,
            at: 1_783_728_000_000,
            text: None,
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
        let events = RecordingAppEvents::new(inner.clone(), store);
        events.activity_event(&ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: ActivityKind::Tool,
            at: 1_783_728_000_000,
            text: None,
            count: None,
        });
        assert_eq!(inner.activities().len(), 1);
        fs::remove_file(root).unwrap();
    }
}
