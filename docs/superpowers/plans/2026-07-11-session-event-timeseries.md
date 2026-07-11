# Session Event Time Series Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist privacy-safe, versioned, daily JSONL session events that preserve per-character and per-session ordering for later analysis.

**Architecture:** A new Rust `session_events` module owns the v1 record schema, the UTC-partitioned append store, and an `AppEvents` decorator that records before forwarding. `SessionManager` emits an internal `session_started` event with the resolved cwd/shell and renderer-supplied profile snapshot; existing renderer time summaries and `session-times.jsonl` remain unchanged.

**Tech Stack:** Rust 2021, serde/serde_json, chrono, parking_lot, UUID v4, Tauri 2, TypeScript, Vitest.

## Global Constraints

- Store files under `<app-data>/session-events/v1/YYYY-MM-DD.jsonl`, partitioned by the event timestamp's UTC date.
- Every record contains `schemaVersion: 1`, one app-run UUID, a run-local sequence starting at 1, epoch-millisecond `at`, `agentId`, `sessionId`, and `kind`.
- Store `session_started`, `session_state`, `prompt`, `tool`, `notification`, `bell`, and `stop` events.
- Never serialize prompt text, prompt summaries, notification messages, dedup keys, terminal I/O, tool inputs/results, environment variables, or API keys.
- A persistence failure must print a metadata-free `eprintln!` warning and must not block PTY, hook, notification, or renderer event delivery.
- Do not add analysis UI, query/export APIs, retention, compression, retries, or an in-memory retry queue.
- Keep `session-times.jsonl`, `turnReducer`, Zustand `timeTracking`, and `SessionTimePanel` behavior unchanged.
- Baseline exception: on macOS, `session::bash_wrapper::tests::is_bash_matches_bare_and_full_paths` is the only accepted pre-existing Rust failure; no new failures are allowed.
- Run every shell command through `rtk`; in command chains, prefix every segment.

---

## File Structure

- Create `src-tauri/src/session_events/mod.rs`: module exports.
- Create `src-tauri/src/session_events/types.rs`: v1 persisted record, draft, event kind, session-start snapshot.
- Create `src-tauri/src/session_events/store.rs`: run ID/sequence allocation, UTC partition selection, serialized append.
- Create `src-tauri/src/session_events/recording_events.rs`: privacy-preserving `AppEvents` decorator and non-fatal error reporting.
- Modify `src-tauri/src/state.rs`: add the internal `session_started` event boundary and test-fake capture.
- Modify `src-tauri/src/session/manager.rs`: emit `session_started` after spawn succeeds and before `starting` state emission.
- Modify `src-tauri/src/ipc/commands.rs`: accept the profile snapshot in `SessionOpts` and call `create_with_profile`.
- Modify `src/shared/types.ts`: add optional profile snapshot fields to `CreateSessionOptions` only.
- Modify `src/renderer/ipc/sessionOpts.ts`: populate profile name/role in every real session creation.
- Modify `src/renderer/ipc/__tests__/sessionOpts.test.ts`: verify profile metadata and legacy option behavior.
- Modify `src-tauri/src/lib.rs`: construct the v1 store and wrap production `TauriEvents` before hub/manager creation.
- Modify `src-tauri/Cargo.toml` and `src-tauri/Cargo.lock`: make the already-locked `chrono` package a direct dependency.

---

### Task 1: V1 Schema and UTC-Partitioned Append Store

**Files:**
- Create: `src-tauri/src/session_events/mod.rs`
- Create: `src-tauri/src/session_events/types.rs`
- Create: `src-tauri/src/session_events/store.rs`
- Modify: `src-tauri/src/lib.rs:1-15`
- Modify: `src-tauri/Cargo.toml:18-39`
- Modify: `src-tauri/Cargo.lock`

**Interfaces:**
- Consumes: `crate::types::{AgentId, SessionId, SessionState}` and epoch milliseconds.
- Produces: `SessionEventStore::new(root: PathBuf)`, `SessionEventStore::append(draft: SessionEventDraft) -> io::Result<SessionEventRecord>`, `SessionEventDraft`, `SessionEventKind`, `SessionStartedEvent`, and `AgentEventProfile`.

- [ ] **Step 1: Write failing schema and store tests**

Create `session_events/mod.rs` with module declarations, then create `store.rs` tests that specify round-trip serialization, daily rotation, concurrency, and consumed sequence numbers:

```rust
// src-tauri/src/session_events/mod.rs
pub mod store;
pub mod types;
```

```rust
// append inside src-tauri/src/session_events/store.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_events::types::{SessionEventDraft, SessionEventKind};
    use std::collections::HashSet;
    use std::fs;
    use std::sync::Arc;

    const BEFORE_UTC_MIDNIGHT: u64 = 1_783_727_999_999;
    const AT_UTC_MIDNIGHT: u64 = 1_783_728_000_000;

    fn scratch_root() -> PathBuf {
        std::env::temp_dir().join(format!("agent-office-session-events-{}", uuid::Uuid::new_v4()))
    }

    fn draft(at: u64) -> SessionEventDraft {
        SessionEventDraft::simple("a1", "s1", SessionEventKind::Tool, at)
    }

    fn read_records(path: &std::path::Path) -> Vec<SessionEventRecord> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn append_creates_a_v1_record_with_run_id_and_sequence() {
        let root = scratch_root();
        let store = SessionEventStore::with_run_id(root.clone(), "run-1".into());
        let record = store.append(draft(AT_UTC_MIDNIGHT)).unwrap();
        assert_eq!(record.schema_version, 1);
        assert_eq!(record.run_id, "run-1");
        assert_eq!(record.seq, 1);
        assert_eq!(read_records(&root.join("2026-07-11.jsonl")), vec![record]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn append_partitions_on_the_event_utc_date() {
        let root = scratch_root();
        let store = SessionEventStore::with_run_id(root.clone(), "run-1".into());
        store.append(draft(BEFORE_UTC_MIDNIGHT)).unwrap();
        store.append(draft(AT_UTC_MIDNIGHT)).unwrap();
        assert_eq!(read_records(&root.join("2026-07-10.jsonl")).len(), 1);
        assert_eq!(read_records(&root.join("2026-07-11.jsonl")).len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_appends_produce_complete_unique_lines() {
        let root = scratch_root();
        let store = Arc::new(SessionEventStore::with_run_id(root.clone(), "run-1".into()));
        let threads: Vec<_> = (0..32)
            .map(|_| {
                let store = store.clone();
                std::thread::spawn(move || store.append(draft(AT_UTC_MIDNIGHT)).unwrap())
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let records = read_records(&root.join("2026-07-11.jsonl"));
        let seqs: HashSet<_> = records.iter().map(|record| record.seq).collect();
        assert_eq!(records.len(), 32);
        assert_eq!(seqs.len(), 32);
        assert_eq!(seqs.iter().copied().min(), Some(1));
        assert_eq!(seqs.iter().copied().max(), Some(32));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_append_consumes_its_sequence_number() {
        let root = scratch_root();
        fs::write(&root, b"not a directory").unwrap();
        let store = SessionEventStore::with_run_id(root.clone(), "run-1".into());
        assert!(store.append(draft(AT_UTC_MIDNIGHT)).is_err());
        fs::remove_file(&root).unwrap();
        let record = store.append(draft(AT_UTC_MIDNIGHT)).unwrap();
        assert_eq!(record.seq, 2);
        let _ = fs::remove_dir_all(root);
    }
}
```

- [ ] **Step 2: Run the store tests and verify they fail**

Run:

```bash
rtk cargo test session_events::store::tests
```

Expected: compilation fails because `SessionEventStore`, `SessionEventDraft`, `SessionEventKind`, and `SessionEventRecord` are not defined.

- [ ] **Step 3: Implement the v1 types**

Create `src-tauri/src/session_events/types.rs`:

```rust
use serde::{Deserialize, Serialize};

use crate::types::{AgentId, SessionId, SessionState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    SessionStarted,
    SessionState,
    Prompt,
    Tool,
    Notification,
    Bell,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEventProfile {
    pub name: String,
    pub role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStartedEvent {
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub agent_name: String,
    pub agent_role: Option<String>,
    pub cwd: String,
    pub shell: String,
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEventDraft {
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub kind: SessionEventKind,
    pub at: u64,
    pub agent_name: Option<String>,
    pub agent_role: Option<String>,
    pub cwd: Option<String>,
    pub shell: Option<String>,
    pub state: Option<SessionState>,
}

impl SessionEventDraft {
    pub fn simple(
        agent_id: impl Into<String>,
        session_id: impl Into<String>,
        kind: SessionEventKind,
        at: u64,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            session_id: session_id.into(),
            kind,
            at,
            agent_name: None,
            agent_role: None,
            cwd: None,
            shell: None,
            state: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEventRecord {
    pub schema_version: u8,
    pub run_id: String,
    pub seq: u64,
    pub at: u64,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub kind: SessionEventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<SessionState>,
}
```

- [ ] **Step 4: Implement serialized UTC-partitioned append**

Add `chrono = "0.4"` under `[dependencies]`, add `mod session_events;` to `lib.rs`, and implement `store.rs`:

```rust
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use uuid::Uuid;

use super::types::{SessionEventDraft, SessionEventRecord};

pub struct SessionEventStore {
    root: PathBuf,
    run_id: String,
    next_seq: Mutex<u64>,
}

impl SessionEventStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root, run_id: Uuid::new_v4().to_string(), next_seq: Mutex::new(1) }
    }

    #[cfg(test)]
    pub(crate) fn with_run_id(root: PathBuf, run_id: String) -> Self {
        Self { root, run_id, next_seq: Mutex::new(1) }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn append(&self, draft: SessionEventDraft) -> io::Result<SessionEventRecord> {
        let mut next_seq = self.next_seq.lock();
        let seq = *next_seq;
        *next_seq = (*next_seq).saturating_add(1);
        let record = SessionEventRecord {
            schema_version: 1,
            run_id: self.run_id.clone(),
            seq,
            at: draft.at,
            agent_id: draft.agent_id,
            session_id: draft.session_id,
            kind: draft.kind,
            agent_name: draft.agent_name,
            agent_role: draft.agent_role,
            cwd: draft.cwd,
            shell: draft.shell,
            state: draft.state,
        };
        let path = self.path_for(record.at)?;
        fs::create_dir_all(&self.root)?;
        let mut line = serde_json::to_vec(&record)?;
        line.push(b'\n');
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(&line)?;
        Ok(record)
    }

    fn path_for(&self, at: u64) -> io::Result<PathBuf> {
        let millis = i64::try_from(at)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "event timestamp exceeds i64"))?;
        let date = DateTime::<Utc>::from_timestamp_millis(millis)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid event timestamp"))?
            .format("%Y-%m-%d")
            .to_string();
        Ok(self.root.join(format!("{date}.jsonl")))
    }
}
```

- [ ] **Step 5: Run store tests and Rust formatting**

Run:

```bash
rtk cargo fmt -- --check
rtk cargo test session_events::store::tests
```

Expected: formatting exits 0 and all four store tests pass.

- [ ] **Step 6: Commit the store**

```bash
rtk git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/lib.rs src-tauri/src/session_events
rtk git commit -m "feat(rust): add versioned session event store"
```

---

### Task 2: Privacy-Preserving AppEvents Recorder

**Files:**
- Create: `src-tauri/src/session_events/recording_events.rs`
- Modify: `src-tauri/src/state.rs:20-48`
- Modify: `src-tauri/src/session_events/mod.rs`

**Interfaces:**
- Consumes: `Arc<dyn AppEvents>`, `Arc<SessionEventStore>`, and existing backend event structs.
- Produces: `RecordingAppEvents::new(inner, store)` implementing `AppEvents`; adds `AppEvents::session_started(&SessionStartedEvent)` with a default no-op.

Add `pub mod recording_events;` to `session_events/mod.rs` as part of this task, after creating `recording_events.rs`.

- [ ] **Step 1: Write failing normalization, privacy, forwarding, and failure tests**

Add tests to `recording_events.rs` using a temp store and `crate::state::fake::RecordingEvents`. Cover these exact assertions:

```rust
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

    fn scratch_root() -> PathBuf {
        std::env::temp_dir().join(format!("agent-office-recording-events-{}", uuid::Uuid::new_v4()))
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
        let store = Arc::new(SessionEventStore::with_run_id(root.clone(), "run-1".into()));
        let events = RecordingAppEvents::new(inner.clone(), store);
        events.activity_event(&ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: ActivityKind::Prompt,
            at: 1_783_728_000_000,
            text: Some("do not persist this prompt".into()),
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
        assert_eq!(records.iter().map(|r| r.kind).collect::<Vec<_>>(), vec![
            SessionEventKind::Prompt,
            SessionEventKind::Notification,
        ]);
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
        let store = Arc::new(SessionEventStore::with_run_id(root.clone(), "run-1".into()));
        let events = RecordingAppEvents::new(inner, store);
        events.session_started(&SessionStartedEvent {
            agent_id: "a1".into(), session_id: "s1".into(), agent_name: "Compiler".into(),
            agent_role: Some("Platform".into()), cwd: "/work".into(), shell: "/bin/zsh".into(),
            at: 1_783_728_000_000,
        });
        events.session_state(&SessionStateEvent {
            session_id: "s1".into(), agent_id: "a1".into(), state: SessionState::Running,
            exit: None, at: 1_783_728_000_001,
        });
        for (offset, source) in [NotificationSource::Bell, NotificationSource::Stop].into_iter().enumerate() {
            events.notification_new(&NotificationEvent {
                id: format!("n{offset}"), session_id: "s1".into(), agent_id: "a1".into(), source,
                message: String::new(), dedup_key: format!("k{offset}"), at: 1_783_728_000_002 + offset as u64,
            });
        }
        events.activity_event(&ActivityEvent {
            agent_id: "a1".into(), session_id: "s1".into(), kind: ActivityKind::Tool,
            at: 1_783_728_000_004, text: None,
        });
        let records = read(&root);
        assert_eq!(records.iter().map(|r| r.kind).collect::<Vec<_>>(), vec![
            SessionEventKind::SessionStarted,
            SessionEventKind::SessionState,
            SessionEventKind::Bell,
            SessionEventKind::Stop,
            SessionEventKind::Tool,
        ]);
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
        let store = Arc::new(SessionEventStore::with_run_id(root.clone(), "run-1".into()));
        let events = RecordingAppEvents::new(inner, store);
        let states = [
            SessionState::Starting,
            SessionState::Running,
            SessionState::Exited,
            SessionState::Disposed,
        ];
        for (offset, state) in states.into_iter().enumerate() {
            events.session_state(&SessionStateEvent {
                session_id: "s1".into(), agent_id: "a1".into(), state,
                exit: None, at: 1_783_728_000_000 + offset as u64,
            });
        }
        assert_eq!(
            read(&root).iter().map(|record| record.state.unwrap()).collect::<Vec<_>>(),
            states,
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn append_failure_does_not_block_forwarding() {
        let root = scratch_root();
        fs::write(&root, b"not a directory").unwrap();
        let inner = Arc::new(RecordingEvents::default());
        let store = Arc::new(SessionEventStore::with_run_id(root.clone(), "run-1".into()));
        let events = RecordingAppEvents::new(inner.clone(), store);
        events.activity_event(&ActivityEvent {
            agent_id: "a1".into(), session_id: "s1".into(), kind: ActivityKind::Tool,
            at: 1_783_728_000_000, text: None,
        });
        assert_eq!(inner.activities().len(), 1);
        fs::remove_file(root).unwrap();
    }
}
```

- [ ] **Step 2: Run recorder tests and verify they fail**

```bash
rtk cargo test session_events::recording_events::tests
```

Expected: compilation fails because `RecordingAppEvents` and `AppEvents::session_started` do not exist.

- [ ] **Step 3: Add the internal session-start event boundary**

Modify `AppEvents` in `state.rs`:

```rust
use crate::session_events::types::SessionStartedEvent;

pub trait AppEvents: Send + Sync {
    fn session_started(&self, _ev: &SessionStartedEvent) {}
    fn session_state(&self, ev: &SessionStateEvent);
    fn notification_new(&self, ev: &NotificationEvent);
    fn notification_cleared(&self, agent_id: &str, ids: &[String]);
    fn activity_event(&self, ev: &ActivityEvent);
}
```

Inside `state::fake`, add `use crate::session_events::types::SessionStartedEvent;` so the new capture fields and accessors use the same internal type.

Add these two fields to the existing `RecordingEvents` struct:

```rust
session_starts: Mutex<Vec<SessionStartedEvent>>,
timeline: Mutex<Vec<String>>,
```

Add `session_started` to the existing `AppEvents for RecordingEvents` implementation and add the timeline push to its existing `session_state` method:

```rust
fn session_started(&self, ev: &SessionStartedEvent) {
    self.session_starts.lock().unwrap().push(ev.clone());
    self.timeline.lock().unwrap().push("session_started".into());
}

fn session_state(&self, ev: &SessionStateEvent) {
    self.states.lock().unwrap().push(ev.clone());
    self.timeline.lock().unwrap().push(format!("session_state:{:?}", ev.state));
}
```

Add these accessors to the existing inherent implementation:

```rust
pub fn session_starts(&self) -> Vec<SessionStartedEvent> {
    self.session_starts.lock().unwrap().clone()
}

pub fn timeline(&self) -> Vec<String> {
    self.timeline.lock().unwrap().clone()
}
```

- [ ] **Step 4: Implement the recording decorator**

Create `recording_events.rs`:

```rust
use std::sync::Arc;

use crate::state::AppEvents;
use crate::types::{ActivityEvent, ActivityKind, NotificationEvent, NotificationSource, SessionStateEvent};

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
            agent_id: event.agent_id.clone(), session_id: event.session_id.clone(),
            kind: SessionEventKind::SessionStarted, at: event.at,
            agent_name: Some(event.agent_name.clone()), agent_role: event.agent_role.clone(),
            cwd: Some(event.cwd.clone()), shell: Some(event.shell.clone()), state: None,
        });
        self.inner.session_started(event);
    }

    fn session_state(&self, event: &SessionStateEvent) {
        let mut draft = SessionEventDraft::simple(
            event.agent_id.clone(), event.session_id.clone(), SessionEventKind::SessionState, event.at,
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
            event.agent_id.clone(), event.session_id.clone(), kind, event.at,
        ));
        self.inner.notification_new(event);
    }

    fn notification_cleared(&self, agent_id: &str, ids: &[String]) {
        self.inner.notification_cleared(agent_id, ids);
    }

    fn activity_event(&self, event: &ActivityEvent) {
        let kind = match event.kind {
            ActivityKind::Prompt => SessionEventKind::Prompt,
            ActivityKind::Tool => SessionEventKind::Tool,
        };
        self.record(SessionEventDraft::simple(
            event.agent_id.clone(), event.session_id.clone(), kind, event.at,
        ));
        self.inner.activity_event(event);
    }
}
```

- [ ] **Step 5: Run recorder and existing state tests**

```bash
rtk cargo fmt -- --check
rtk cargo test session_events::recording_events::tests
rtk cargo test state::tests
```

Expected: all targeted tests pass, including non-fatal append failure forwarding.

- [ ] **Step 6: Commit the recorder**

```bash
rtk git add src-tauri/src/state.rs src-tauri/src/session_events
rtk git commit -m "feat(rust): record backend session events safely"
```

---

### Task 3: Session Profile Snapshot and Resolved Runtime Context

**Files:**
- Modify: `src/shared/types.ts:139-155`
- Modify: `src/renderer/ipc/sessionOpts.ts`
- Modify: `src/renderer/ipc/__tests__/sessionOpts.test.ts`
- Modify: `src-tauri/src/ipc/commands.rs:20-65,620-675`
- Modify: `src-tauri/src/session/manager.rs:130-270,560-680`

**Interfaces:**
- Consumes: `AgentProfile.name`, `AgentProfile.role`, requested cwd/shell, and resolved `shells::ResolvedShell`.
- Produces: `CreateSessionOptions.agentName?`, `CreateSessionOptions.agentRole?`, Rust `SessionOpts.agent_name/agent_role`, and `SessionManager::create_with_profile(req, profile)`.

- [ ] **Step 1: Write failing renderer option tests**

Add these tests to `sessionOpts.test.ts`:

```ts
it("includes the profile snapshot used by session event analysis", () => {
  expect(sessionOptsFor({ name: "Compiler", role: "Platform" })).toEqual({
    agentName: "Compiler",
    agentRole: "Platform",
  });
});

it("keeps cwd shell and startup command with the profile snapshot", () => {
  expect(sessionOptsFor({
    name: "Compiler",
    role: "Platform",
    cwd: "/work",
    shell: "zsh",
    startupCommand: "source ./init.sh",
  })).toEqual({
    agentName: "Compiler",
    agentRole: "Platform",
    cwd: "/work",
    shell: "zsh",
    startupCommand: "source ./init.sh",
  });
});
```

- [ ] **Step 2: Run renderer option tests and verify they fail**

```bash
rtk npm test -- --run src/renderer/ipc/__tests__/sessionOpts.test.ts
```

Expected: both new tests fail because name and role are omitted.

- [ ] **Step 3: Add profile snapshot fields to the create-session adapter**

Extend `CreateSessionOptions`:

```ts
export interface CreateSessionOptions {
  cols?: number;
  rows?: number;
  cwd?: string;
  shell?: string;
  startupCommand?: string;
  /** Historical profile label copied into session_started analytics only. */
  agentName?: string;
  /** Historical profile role copied into session_started analytics only. */
  agentRole?: string;
}
```

Update `sessionOptsFor`:

```ts
export function sessionOptsFor(
  a?: { name?: string; role?: string; cwd?: string; shell?: string; startupCommand?: string },
): CreateSessionOptions | undefined {
  if (!a) return undefined;
  const o: CreateSessionOptions = {};
  if (a.name) o.agentName = a.name;
  if (a.role) o.agentRole = a.role;
  if (a.cwd) o.cwd = a.cwd;
  if (a.shell) o.shell = a.shell;
  if (a.startupCommand) o.startupCommand = a.startupCommand;
  return Object.keys(o).length ? o : undefined;
}
```

Update the `CreateSessionRequest` comment to state that `agentName` and `agentRole` are consumed by the Tauri command and are not part of Rust's PTY `CreateSessionRequest`.

- [ ] **Step 4: Write failing Rust command and manager tests**

Add a pure command helper test:

```rust
#[test]
fn create_session_opts_profile_snapshot_flows_to_manager() {
    let opts = SessionOpts {
        cols: None,
        rows: None,
        cwd: None,
        shell: None,
        startup_command: None,
        agent_name: Some("Compiler".into()),
        agent_role: Some("Platform".into()),
    };
    assert_eq!(
        event_profile("a1", &opts),
        AgentEventProfile { name: "Compiler".into(), role: Some("Platform".into()) },
    );
    assert_eq!(
        event_profile("a1", &SessionOpts::default()),
        AgentEventProfile { name: "a1".into(), role: None },
    );
}
```

Add this manager test using `RecordingEvents`:

```rust
#[test]
fn successful_spawn_emits_session_started_with_profile_and_resolved_context() {
    let events = Arc::new(RecordingEvents::default());
    let reg = registry();
    let hub = hub_for(reg.clone(), events.clone());
    let (writer, dir) = scratch_hook_writer();
    let (factory, control) = FakePtyFactory::new();
    let manager = Arc::new(
        SessionManager::new(
            Arc::new(factory), writer, reg, events.clone(), hub, Arc::new(|| None),
        )
        .with_shell_resolver(Arc::new(|_| shells::ResolvedShell {
            program: "/bin/test-shell".into(), args: Vec::new(), extra_env: Vec::new(),
        })),
    );
    manager.create_with_profile(
        req_with_cwd("a1", Some("/work".into())),
        AgentEventProfile { name: "Compiler".into(), role: Some("Platform".into()) },
    ).unwrap();
    let starts = events.session_starts();
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0].agent_name, "Compiler");
    assert_eq!(starts[0].agent_role.as_deref(), Some("Platform"));
    assert_eq!(starts[0].cwd, "/work");
    assert_eq!(starts[0].shell, "/bin/test-shell");
    assert_eq!(
        &events.timeline()[..2],
        &["session_started".to_string(), "session_state:Starting".to_string()],
    );
    manager.create_with_profile(
        req_with_cwd("a1", Some("/different-work".into())),
        AgentEventProfile { name: "Renamed".into(), role: None },
    ).unwrap();
    assert_eq!(events.session_starts().len(), 1, "reusing a live session must not log a second start");
    control.close_output();
    let _ = std::fs::remove_dir_all(dir);
}
```

- [ ] **Step 5: Run the new Rust tests and verify they fail**

```bash
rtk cargo test successful_spawn_emits_session_started_with_profile_and_resolved_context
rtk cargo test create_session_opts_profile_snapshot_flows_to_manager
```

Expected: compilation fails because the new option fields and `create_with_profile` do not exist.

- [ ] **Step 6: Implement Rust metadata plumbing and ordered session-start emission**

Extend `SessionOpts` with camelCase-backed fields:

```rust
pub agent_name: Option<String>,
pub agent_role: Option<String>,
```

Import `crate::session_events::types::AgentEventProfile` in `ipc/commands.rs` and import both `AgentEventProfile` and `SessionStartedEvent` in `session/manager.rs`.

Add a pure mapper next to `SessionOpts`:

```rust
fn event_profile(agent_id: &str, opts: &SessionOpts) -> AgentEventProfile {
    AgentEventProfile {
        name: opts.agent_name.clone().unwrap_or_else(|| agent_id.to_string()),
        role: opts.agent_role.clone(),
    }
}
```

In `create_session`, separate analytics metadata from the PTY request and preserve the existing panic guard:

```rust
let profile = event_profile(&agent_id, &o);
let manager = app_state.manager.clone();
let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
    manager.create_with_profile(
        CreateSessionRequest {
            agent_id,
            cols: o.cols,
            rows: o.rows,
            cwd: o.cwd,
            shell: o.shell,
            startup_command: o.startup_command,
            autostart_claude: None,
        },
        profile,
    )
}));
```

Rename the current implementation with this exact signature diff, leaving its body in place:

```diff
-pub fn create(self: &Arc<Self>, req: CreateSessionRequest) -> Result<CreateSessionResult, String> {
+pub fn create_with_profile(
+    self: &Arc<Self>,
+    req: CreateSessionRequest,
+    profile: AgentEventProfile,
+) -> Result<CreateSessionResult, String> {
```

Insert this compatibility entry point immediately before the renamed method so all existing direct manager callers retain the same signature:

```rust
pub fn create(self: &Arc<Self>, req: CreateSessionRequest) -> Result<CreateSessionResult, String> {
    let fallback = AgentEventProfile { name: req.agent_id.clone(), role: None };
    self.create_with_profile(req, fallback)
}
```

Immediately after PTY spawn succeeds, retain the resolved values and emit before `SessionState::Starting`:

```rust
let actual_shell = resolved.program.clone();
let actual_cwd = cwd.clone();
let spawned = self.factory.spawn(PtySpawnOptions {
    shell: resolved.program,
    args: resolved.args,
    cols: req.cols.unwrap_or(80),
    rows: req.rows.unwrap_or(24),
    cwd,
    env,
}).map_err(|error| error.to_string())?;

self.events.session_started(&SessionStartedEvent {
    agent_id: req.agent_id.clone(),
    session_id: session_id.clone(),
    agent_name: profile.name,
    agent_role: profile.role,
    cwd: actual_cwd,
    shell: actual_shell,
    at: now_ms(),
});
```

Place this call after all fallible spawn work and before `self.emit_state(...Starting...)`. Preserve the existing hook-file RAII error path rather than replacing the current `match` unless the guard remains armed on spawn failure.

- [ ] **Step 7: Run targeted frontend and Rust tests**

```bash
rtk npm test -- --run src/renderer/ipc/__tests__/sessionOpts.test.ts
rtk cargo fmt -- --check
rtk cargo test successful_spawn_emits_session_started_with_profile_and_resolved_context
rtk cargo test create_session_opts_profile_snapshot_flows_to_manager
rtk cargo test session::manager::tests
rtk cargo test ipc::commands::tests
```

Expected: all targeted tests pass and existing session creation behavior remains unchanged.

- [ ] **Step 8: Commit profile and runtime context collection**

```bash
rtk git add src/shared/types.ts src/renderer/ipc/sessionOpts.ts src/renderer/ipc/__tests__/sessionOpts.test.ts src-tauri/src/ipc/commands.rs src-tauri/src/session/manager.rs
rtk git commit -m "feat: attach session context to event timeline"
```

---

### Task 4: Production Wiring and Full Regression Verification

**Files:**
- Modify: `src-tauri/src/lib.rs:75-150,220-270`
- Test: `src-tauri/src/lib.rs` internal tests

**Interfaces:**
- Consumes: `RecordingAppEvents`, `SessionEventStore`, Tauri app-data directory, existing `TauriEvents`.
- Produces: production event pipeline writing to `<app-data>/session-events/v1` before forwarding to the existing hub, manager, and renderer.

- [ ] **Step 1: Write a failing versioned-root test**

Add a pure helper test to `lib.rs`:

```rust
#[test]
fn session_event_root_is_versioned_under_app_data() {
    let root = session_event_root(std::path::Path::new("/app-data"));
    assert_eq!(root, std::path::Path::new("/app-data/session-events/v1"));
}
```

- [ ] **Step 2: Run the helper test and verify it fails**

```bash
rtk cargo test session_event_root_is_versioned_under_app_data
```

Expected: compilation fails because `session_event_root` is undefined.

- [ ] **Step 3: Implement the root helper and production decorator wiring**

Add the helper:

```rust
fn session_event_root(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("session-events").join("v1")
}
```

In `setup`, resolve `data_dir` before constructing the event sink, then wrap the real Tauri emitter:

```rust
let data_dir = app.path().app_data_dir()?;
install_panic_logger(data_dir.clone());

let event_store = Arc::new(
    crate::session_events::store::SessionEventStore::new(session_event_root(&data_dir)),
);
let tauri_events: Arc<dyn AppEvents> = Arc::new(TauriEvents { app: handle.clone() });
let events: Arc<dyn AppEvents> = Arc::new(
    crate::session_events::recording_events::RecordingAppEvents::new(
        tauri_events,
        event_store,
    ),
);

let registry = Arc::new(SessionRegistry::new());
let hub = Arc::new(NotificationHub::new(
    registry.clone(),
    events.clone(),
    Arc::new(SystemClock),
    Duration::from_millis(3000),
));
```

Remove the later duplicate `data_dir` lookup. Keep `events.clone()` as the dependency passed to `SessionManager`, ensuring both hub-originated events and manager-originated `session_started`/state events pass through the same recorder.

- [ ] **Step 4: Run focused wiring and privacy tests**

```bash
rtk cargo fmt -- --check
rtk cargo test session_event_root_is_versioned_under_app_data
rtk cargo test session_events::
rtk cargo test successful_spawn_emits_session_started_with_profile_and_resolved_context
```

Expected: every focused test passes.

- [ ] **Step 5: Run full frontend verification**

```bash
rtk npm test -- --run
rtk npm run typecheck
rtk npm run build
```

Expected: all Vitest files pass, typecheck exits 0, and the production build exits 0.

- [ ] **Step 6: Run full Rust verification with the accepted baseline exception isolated**

Run the green suite first:

```bash
rtk cargo test -- --skip session::bash_wrapper::tests::is_bash_matches_bare_and_full_paths
rtk cargo build
```

Expected: zero failures and build exit 0.

Then confirm the unfiltered baseline shape outside a network-restricted sandbox:

```bash
rtk cargo test
```

Expected on macOS: only `session::bash_wrapper::tests::is_bash_matches_bare_and_full_paths` fails; every new session-event test passes. If any other test fails, stop and diagnose before committing.

- [ ] **Step 7: Inspect the diff against the approved privacy and scope constraints**

```bash
rtk git diff --check
rtk git diff --stat main...HEAD
rtk proxy rg -n "text|message|dedup_key|terminal|env" src-tauri/src/session_events
```

Expected: `git diff --check` exits 0; any grep hits are limited to test assertions proving forbidden fields are absent or comments documenting the privacy boundary.

- [ ] **Step 8: Commit production wiring**

```bash
rtk git add src-tauri/src/lib.rs
rtk git commit -m "feat(rust): wire session event recording"
```

- [ ] **Step 9: Record final branch evidence**

```bash
rtk git status --short --branch
rtk git log --oneline main..HEAD
```

Expected: clean worktree and the design/plan plus four focused implementation commits on `feature/session-event-timeseries`.
