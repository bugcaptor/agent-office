use super::*;
use crate::notification::hub::{NotificationHub, SystemClock};
use crate::observer::ObserverRuntime;
use crate::session::external::ExternalDetachReason;
use crate::session::pty_factory::fake::{FakeControl, FakePtyFactory};
use crate::state::{fake::RecordingEvents, AppEvents, SessionRegistry};
use crate::types::{ActivityKind, CreateSessionRequest, SessionState};
use std::io::Write;

struct Fixture {
    manager: Arc<SessionManager>,
    settings: Arc<RwLock<AppSettings>>,
    events: Arc<RecordingEvents>,
    dir: tempfile::TempDir,
    file: std::path::PathBuf,
    control: Arc<FakeControl>,
}

impl Fixture {
    fn new() -> Self {
        let events = Arc::new(RecordingEvents::default());
        let registry = Arc::new(SessionRegistry::new());
        let hub = Arc::new(NotificationHub::new(
            registry.clone(),
            events.clone() as Arc<dyn AppEvents>,
            Arc::new(SystemClock),
            Duration::from_millis(3_000),
        ));
        let observer = Arc::new(ObserverRuntime::new(hub.clone(), vec![]));
        let (factory, control) = FakePtyFactory::new();
        let manager = Arc::new(SessionManager::new(
            Arc::new(factory),
            observer,
            registry,
            events.clone() as Arc<dyn AppEvents>,
            hub,
            Arc::new(|| None),
        ));
        let settings = Arc::new(RwLock::new(AppSettings {
            observer_enabled: true,
            ide_connection_enabled: true,
            ..AppSettings::default()
        }));
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session.jsonl");
        let meta = serde_json::json!({"type":"session_meta","payload":{
            "id":"source-1","cwd":dir.path().to_str().unwrap(),"source":"vscode",
            "originator":"codex_vscode","thread_source":"user"
        }});
        std::fs::write(&file, format!("{meta}\n")).unwrap();
        Self {
            manager,
            settings,
            events,
            dir,
            file,
            control,
        }
    }
    fn attach(&self, agent: &str) -> Result<Watcher, String> {
        Watcher::attach(
            &self.manager,
            &self.settings,
            agent,
            self.dir.path().to_str().unwrap(),
            AgentEventProfile {
                name: "Example".into(),
                role: None,
            },
            &self.file,
            "codex",
            "source-1",
        )
    }
    fn append(&self, text: &str) {
        std::fs::OpenOptions::new()
            .append(true)
            .open(&self.file)
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }

    fn start_pty(&self) {
        self.manager
            .create(CreateSessionRequest {
                agent_id: "a1".into(),
                cwd: Some(self.dir.path().to_string_lossy().into_owned()),
                cols: None,
                rows: None,
                shell: None,
                startup_command: None,
                personality_prompt: None,
                autostart_claude: Some(false),
                tmux_host: None,
            })
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.control.close_output();
        self.control.fire_exit(0);
    }
}

const PROMPT: &str =
    "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"new-turn\"}}\n";
const STOP: &str =
    "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\",\"turn_id\":\"new-turn\"}}\n";
const TOKEN_COUNT: &str = "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"model\":\"gpt-5\",\"info\":{\"total_token_usage\":{\"input_tokens\":120,\"output_tokens\":12,\"cached_input_tokens\":0,\"cache_write_input_tokens\":0}}}}\n";

fn kilo_db(fixture: &Fixture) -> std::path::PathBuf {
    let path = fixture.dir.path().join("kilo.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY, directory TEXT, parent_id TEXT, time_updated INTEGER, time_archived INTEGER); CREATE TABLE event(id TEXT PRIMARY KEY, aggregate_id TEXT NOT NULL, seq INTEGER NOT NULL, type TEXT NOT NULL, data TEXT NOT NULL);").unwrap();
    db.execute(
        "INSERT INTO session VALUES ('k1', ?1, NULL, 1, NULL)",
        [fixture.dir.path().to_str().unwrap()],
    )
    .unwrap();
    path
}

fn kilo_event(path: &std::path::Path, id: &str, seq: i64, kind: &str, data: serde_json::Value) {
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute(
        "INSERT INTO event VALUES (?1, 'k1', ?2, ?3, ?4)",
        rusqlite::params![id, seq, kind, data.to_string()],
    )
    .unwrap();
}

fn attach_kilo(fixture: &Fixture, agent: &str, path: &std::path::Path) -> Result<Watcher, String> {
    Watcher::attach(
        &fixture.manager,
        &fixture.settings,
        agent,
        fixture.dir.path().to_str().unwrap(),
        AgentEventProfile {
            name: "Example".into(),
            role: None,
        },
        path,
        "kilo",
        "k1",
    )
}

#[test]
fn selected_kilo_db_forwards_only_new_turn_usage_and_detach_stops_reading() {
    let f = Fixture::new();
    let path = kilo_db(&f);
    kilo_event(
        &path,
        "historic",
        1,
        "message.updated.1",
        serde_json::json!({"info":{"id":"old","role":"user","text":"private"}}),
    );
    let mut watcher = attach_kilo(&f, "a1", &path).unwrap();
    watcher.tick().unwrap();
    assert!(f.events.notifications().is_empty());
    kilo_event(
        &path,
        "user",
        2,
        "message.updated.1",
        serde_json::json!({"info":{"id":"u1","role":"user","text":"private"}}),
    );
    kilo_event(
        &path,
        "stop",
        3,
        "message.updated.1",
        serde_json::json!({"info":{"id":"a1","role":"assistant","finish":"stop","time":{"completed":1},"modelID":"kilo/model","tokens":{"input":12,"output":3,"reasoning":2,"cache":{"read":1,"write":0}},"text":"private"}}),
    );
    watcher.tick().unwrap();
    assert_eq!(
        f.events
            .activities()
            .iter()
            .filter(|event| event.kind == ActivityKind::Prompt)
            .count(),
        1
    );
    assert_eq!(f.events.notifications().len(), 1);
    assert_eq!(f.events.usages()[0].tokens.input, Some(12));
    assert_eq!(f.events.usages()[0].tokens.output, Some(5));
    assert_eq!(
        attach_kilo(&f, "a2", &path).err().unwrap(),
        "observed-source-already-attached"
    );
    f.manager
        .detach_external("a1", ExternalDetachReason::Detach);
    assert!(watcher.tick().is_err());
    drop(watcher);
    assert!(f.manager.session_id_for("a1").is_none());
}

#[test]
fn kilo_observer_respects_opt_out_at_attach_and_during_tick() {
    let f = Fixture::new();
    let path = kilo_db(&f);
    f.settings.write().unwrap().ide_connection_enabled = false;
    assert_eq!(
        attach_kilo(&f, "a1", &path).err().unwrap(),
        "observed-observer-disabled"
    );
    f.settings.write().unwrap().ide_connection_enabled = true;
    let mut watcher = attach_kilo(&f, "a1", &path).unwrap();
    f.settings.write().unwrap().observer_enabled = false;
    assert!(watcher.tick().is_err());
    drop(watcher);
    assert!(f.manager.session_id_for("a1").is_none());
}

#[test]
fn selected_transcript_only_forwards_new_activity_and_disconnect_preserves_source() {
    let f = Fixture::new();
    f.append(STOP);
    let mut watcher = f.attach("a1").unwrap();
    watcher.tick().unwrap();
    assert!(f.events.notifications().is_empty());
    assert!(f.events.activities().is_empty());
    f.append(PROMPT);
    f.append(PROMPT);
    f.append(STOP);
    watcher.tick().unwrap();
    assert_eq!(
        f.events
            .activities()
            .iter()
            .filter(|a| a.kind == ActivityKind::Prompt)
            .count(),
        1
    );
    assert_eq!(f.events.notifications().len(), 1);
    let before = std::fs::read(&f.file).unwrap();
    f.manager
        .detach_external("a1", ExternalDetachReason::Detach);
    assert!(watcher.tick().is_err());
    drop(watcher);
    assert_eq!(std::fs::read(&f.file).unwrap(), before);
    assert!(f.manager.session_id_for("a1").is_none());
}

#[test]
fn completed_ide_turn_emits_one_usage_event_without_pre_attach_history() {
    let f = Fixture::new();
    // This historical counter becomes the attachment watermark.
    f.append(TOKEN_COUNT);
    let mut watcher = f.attach("a1").unwrap();
    f.append("{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"model\":\"gpt-5\",\"info\":{\"total_token_usage\":{\"input_tokens\":145,\"output_tokens\":20,\"cached_input_tokens\":0,\"cache_write_input_tokens\":0}}}}\n");
    f.append(STOP);
    watcher.tick().unwrap();
    let usages = f.events.usages();
    assert_eq!(usages.len(), 1);
    assert!(!usages[0].partial);
    assert_eq!(usages[0].tokens.input, Some(25));
    // Re-reading the same stop is deduplicated before it can make another usage turn.
    watcher.tick().unwrap();
    assert_eq!(f.events.usages().len(), 1);
}

#[test]
fn duplicate_source_and_existing_external_are_preserved() {
    let f = Fixture::new();
    let watcher = f.attach("a1").unwrap();
    assert_eq!(
        f.attach("a2").err().as_deref(),
        Some("observed-source-already-attached")
    );
    assert_eq!(
        f.attach("a1").err().as_deref(),
        Some("observed-agent-already-attached")
    );
    assert_eq!(
        f.manager.session_id_for("a1"),
        Some(watcher.session_id.clone())
    );
    drop(watcher);
    let old = f.manager.attach_external("a2", None, None, None).unwrap();
    assert_eq!(
        f.attach("a2").err().as_deref(),
        Some("observed-external-exists")
    );
    assert_eq!(
        f.manager.session_id_for("a2").as_deref(),
        Some(old.session_id())
    );
}

#[test]
fn old_watcher_cleanup_cannot_detach_a_replacement() {
    let f = Fixture::new();
    let mut old = f.attach("a1").unwrap();
    f.manager
        .detach_external("a1", ExternalDetachReason::Detach);
    let current = f.attach("a1").unwrap();
    assert!(old.tick().is_err());
    drop(old);
    assert_eq!(
        f.manager.session_id_for("a1"),
        Some(current.session_id.clone())
    );
}

#[test]
fn observer_disable_and_transcript_truncation_stop_observation() {
    let f = Fixture::new();
    let mut watcher = f.attach("a1").unwrap();
    f.settings.write().unwrap().observer_enabled = false;
    assert!(watcher.tick().is_err());
    drop(watcher);
    assert!(f.manager.session_id_for("a1").is_none());
    assert_eq!(
        f.attach("a1").err().as_deref(),
        Some("observed-observer-disabled")
    );
    f.settings.write().unwrap().observer_enabled = true;
    let mut watcher = f.attach("a1").unwrap();
    std::fs::write(&f.file, b"").unwrap();
    assert!(watcher.tick().is_err());
    drop(watcher);
    assert!(f.manager.session_id_for("a1").is_none());
}

#[test]
fn changed_candidate_or_wrong_workdir_does_not_attach() {
    let f = Fixture::new();
    let attach = |cwd: &str, source: &str| {
        Watcher::attach(
            &f.manager,
            &f.settings,
            "a1",
            cwd,
            AgentEventProfile {
                name: "Example".into(),
                role: None,
            },
            &f.file,
            "codex",
            source,
        )
    };
    assert_eq!(
        attach(f.dir.path().to_str().unwrap(), "different-source")
            .err()
            .as_deref(),
        Some("ide-candidate-changed")
    );
    assert_eq!(
        attach("", "source-1").err().as_deref(),
        Some("observed-cwd-mismatch")
    );
    assert!(f.events.session_starts().is_empty());
}

#[test]
fn verified_ide_connection_owns_its_focus_target_until_detached() {
    let f = Fixture::new();
    assert!(f.manager.observed_focus("a1").is_none());
    let watcher = f.attach("a1").unwrap();
    let focus = f.manager.observed_focus("a1").unwrap();
    assert_eq!(focus.session_id, watcher.session_id);
    assert_eq!(
        focus.target,
        ObservedFocusTarget::VsCode {
            cwd: f.dir.path().to_string_lossy().into_owned(),
        }
    );
    drop(watcher);
    assert!(f.manager.observed_focus("a1").is_none());

    // Codex/Claude alone are not host application identifiers. A generic
    // connector must not silently gain a VS Code activation target.
    f.manager
        .attach_observed(
            "a1",
            "codex",
            "source-1",
            f.dir.path().to_str().unwrap(),
            "owner",
            None,
            AgentEventProfile {
                name: "Example".into(),
                role: None,
            },
        )
        .unwrap();
    assert!(f.manager.observed_focus("a1").is_none());
}

#[tokio::test]
async fn running_pty_is_preserved_and_exited_pty_does_not_hide_new_notifications() {
    let f = Fixture::new();
    f.start_pty();
    let original = f.manager.session_id_for("a1");
    assert_eq!(f.attach("a1").err().as_deref(), Some("observed-pty-exists"));
    assert_eq!(f.manager.session_id_for("a1"), original);
    assert_eq!(f.control.kill_count(), 0);
    f.control.close_output();
    f.control.fire_exit(0);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while f.events.last_state().state != SessionState::Exited {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let mut watcher = f.attach("a1").unwrap();
    assert_eq!(
        f.manager.session_id_for("a1"),
        Some(watcher.session_id.clone())
    );
    f.append(PROMPT);
    f.append(STOP);
    watcher.tick().unwrap();
    assert_eq!(f.manager.pending_notifications("a1").len(), 1);
    assert_eq!(f.control.kill_count(), 0);
}
