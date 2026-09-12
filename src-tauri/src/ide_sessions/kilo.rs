//! Read-only Kilo Code SQLite event observer. It selects metadata only, never content.

use super::kilo_usage::{KiloUsageTracker, UsageRow};
use super::transcripts::Candidate;
use crate::session::external::ObservedEventKind;
use crate::types::SessionEventTokens;
use rusqlite::{Connection, OpenFlags};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

const LIMIT: i64 = 20;
const EVENT_LIMIT: i64 = 512;

fn open(path: &Path) -> Result<Connection, String> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| "kilo-db-unavailable")?;
    db.busy_timeout(Duration::from_millis(100))
        .map_err(|_| "kilo-db-unavailable")?;
    db.pragma_update(None, "query_only", "ON")
        .map_err(|_| "kilo-db-unavailable")?;
    Ok(db)
}

fn default_db() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    Some(data.join("kilo/kilo.db"))
}

pub fn discover(cwd: Option<&str>) -> Vec<Candidate> {
    default_db().map_or_else(Vec::new, |path| discover_at(&path, cwd))
}

fn discover_at(path: &Path, cwd: Option<&str>) -> Vec<Candidate> {
    let Ok(path) = path.canonicalize() else {
        return vec![];
    };
    let Ok(db) = open(&path) else { return vec![] };
    let Ok(mut q) = db.prepare("SELECT id,directory,time_updated FROM session WHERE parent_id IS NULL AND time_archived IS NULL AND directory IS NOT NULL AND directory != '' ORDER BY time_updated DESC LIMIT ?1") else { return vec![] };
    let Ok(rows) = q.query_map([LIMIT], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    }) else {
        return vec![];
    };
    rows.filter_map(Result::ok)
        .filter(|(_, dir, _)| Path::new(dir).is_absolute())
        .filter(|(_, dir, _)| cwd.map(|v| super::same_directory(v, dir)).unwrap_or(true))
        .map(|(id, dir, at)| {
            Candidate::kilo(
                id,
                dir,
                path.to_string_lossy().into_owned(),
                at.max(0) as u64,
            )
        })
        .collect()
}

pub fn inspect(path: &Path, source_session_id: &str) -> Result<Candidate, String> {
    let path = path.canonicalize().map_err(|_| "kilo-db-unavailable")?;
    let db = open(&path)?;
    let (id,dir,at):(String,String,i64) = db.query_row("SELECT id,directory,time_updated FROM session WHERE id=?1 AND parent_id IS NULL AND time_archived IS NULL AND directory IS NOT NULL AND directory != ''", [source_session_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_| "kilo-session-missing")?;
    if !Path::new(&dir).is_absolute() {
        return Err("kilo-session-missing".into());
    }
    Ok(Candidate::kilo(
        id,
        dir,
        path.to_string_lossy().into_owned(),
        at.max(0) as u64,
    ))
}

// Only selected metadata crosses SQLite's JSON boundary. Message bodies and tool
// input/output are deliberately absent from this query.
const EVENT_FIELDS: &str = "seq, id, type,
    json_extract(data,'$.info.id'), json_extract(data,'$.info.role'),
    json_extract(data,'$.info.finish'), json_extract(data,'$.info.time.completed'),
    json_extract(data,'$.info.modelID'), json_extract(data,'$.info.tokens.input'),
    json_extract(data,'$.info.tokens.output'), json_extract(data,'$.info.tokens.reasoning'),
    json_extract(data,'$.info.tokens.cache.read'), json_extract(data,'$.info.tokens.cache.write'),
    json_extract(data,'$.part.id'), json_extract(data,'$.part.type'),
    json_extract(data,'$.part.state.status')";

#[derive(Debug)]
struct Event {
    seq: i64,
    id: String,
    kind: String,
    usage: UsageRow,
    finish: Option<String>,
    completed: Option<i64>,
    part_id: Option<String>,
    part_kind: Option<String>,
    status: Option<String>,
}

impl Event {
    fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            seq: r.get(0)?,
            id: r.get(1)?,
            kind: r.get(2)?,
            usage: UsageRow {
                id: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                role: r.get(4)?,
                model: r.get(7)?,
                input: r.get(8)?,
                output: r.get(9)?,
                reasoning: r.get(10)?,
                cache_read: r.get(11)?,
                cache_write: r.get(12)?,
            },
            finish: r.get(5)?,
            completed: r.get(6)?,
            part_id: r.get(13)?,
            part_kind: r.get(14)?,
            status: r.get(15)?,
        })
    }

    fn activity(&self) -> Option<(ObservedEventKind, &str)> {
        if self.kind == "message.updated.1" && !self.usage.id.is_empty() {
            if self.usage.role.as_deref() == Some("user") {
                return Some((ObservedEventKind::Prompt, &self.usage.id));
            }
            if self.usage.role.as_deref() == Some("assistant")
                && self.finish.as_deref() == Some("stop")
                && self.completed.is_some()
            {
                return Some((ObservedEventKind::Stop, &self.usage.id));
            }
        }
        if self.kind == "message.part.updated.1"
            && self.part_kind.as_deref() == Some("tool")
            && matches!(self.status.as_deref(), Some("pending" | "running"))
        {
            return self
                .part_id
                .as_deref()
                .filter(|id| !id.is_empty())
                .map(|id| (ObservedEventKind::Tool, id));
        }
        None
    }
}

fn stamp(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|_| "kilo-db-unavailable")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!("{}:{}", meta.dev(), meta.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        Ok(meta.creation_time().to_string())
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(format!("{:?}", meta.created().ok()))
    }
}

fn session_state(db: &Connection, session: &str) -> Result<(String, i64, Option<String>), String> {
    db.query_row(
        "SELECT directory,
        COALESCE((SELECT MAX(seq) FROM event WHERE aggregate_id=session.id),0),
        (SELECT id FROM event WHERE aggregate_id=session.id ORDER BY seq DESC LIMIT 1)
        FROM session WHERE id=?1 AND parent_id IS NULL AND time_archived IS NULL",
        [session],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .map_err(|_| "kilo-session-missing".into())
}

#[derive(Debug)]
pub struct Tail {
    path: PathBuf,
    file_stamp: String,
    session_id: String,
    cwd: String,
    attached_seq: i64,
    watermark: i64,
    watermark_id: Option<String>,
    initialized_usage: HashSet<String>,
    usage: KiloUsageTracker,
    pending_usage: Option<SessionEventTokens>,
}
impl Tail {
    pub fn from_candidate(c: &Candidate) -> Result<Self, String> {
        let path = PathBuf::from(&c.file)
            .canonicalize()
            .map_err(|_| "kilo-db-unavailable")?;
        let file_stamp = stamp(&path)?;
        let mut db = open(&path)?;
        let tx = db.transaction().map_err(|_| "kilo-db-unavailable")?;
        let (cwd, watermark, watermark_id) = session_state(&tx, &c.source_session_id)?;
        if cwd != c.cwd || stamp(&path)? != file_stamp {
            return Err("ide-candidate-changed".into());
        }
        tx.commit().map_err(|_| "kilo-db-unavailable")?;
        Ok(Self {
            path,
            file_stamp,
            session_id: c.source_session_id.clone(),
            cwd,
            attached_seq: watermark,
            watermark,
            watermark_id,
            initialized_usage: HashSet::new(),
            usage: KiloUsageTracker::with_baseline([]),
            pending_usage: None,
        })
    }

    // Resolve the immutable pre-attachment history lazily, once per assistant ID.
    // MAX preserves high watermarks even if Kilo later replays a smaller snapshot.
    fn seed_usage(&mut self, db: &Connection, row: &UsageRow) -> Result<(), String> {
        if row.role.as_deref() != Some("assistant")
            || row.id.is_empty()
            || self.initialized_usage.contains(&row.id)
        {
            return Ok(());
        }
        let baseline = db
            .query_row(
                "SELECT
            MAX(json_extract(data,'$.info.tokens.input')),
            MAX(json_extract(data,'$.info.tokens.output')),
            MAX(json_extract(data,'$.info.tokens.reasoning')),
            MAX(json_extract(data,'$.info.tokens.cache.read')),
            MAX(json_extract(data,'$.info.tokens.cache.write'))
            FROM event WHERE aggregate_id=?1 AND seq<=?2 AND type='message.updated.1'
            AND json_extract(data,'$.info.id')=?3 AND json_extract(data,'$.info.role')='assistant'",
                rusqlite::params![self.session_id, self.attached_seq, row.id],
                |r| {
                    Ok(UsageRow {
                        id: row.id.clone(),
                        role: row.role.clone(),
                        model: row.model.clone(),
                        input: r.get(0)?,
                        output: r.get(1)?,
                        reasoning: r.get(2)?,
                        cache_read: r.get(3)?,
                        cache_write: r.get(4)?,
                    })
                },
            )
            .map_err(|_| "kilo-db-schema-changed")?;
        self.usage.seed(baseline);
        self.initialized_usage.insert(row.id.clone());
        Ok(())
    }

    // Query the durable history instead of a finite dedup window: a user summary
    // rewrite may arrive after thousands of part events, even after task completion.
    fn is_first_activity(
        &self,
        db: &Connection,
        event: &Event,
        kind: ObservedEventKind,
        id: &str,
    ) -> Result<bool, String> {
        let condition = match kind {
            ObservedEventKind::Prompt => "type='message.updated.1' AND json_extract(data,'$.info.role')='user' AND json_extract(data,'$.info.id')=?3",
            ObservedEventKind::Tool => "type='message.part.updated.1' AND json_extract(data,'$.part.type')='tool' AND json_extract(data,'$.part.id')=?3 AND json_extract(data,'$.part.state.status') IN ('pending','running')",
            ObservedEventKind::Stop => "type='message.updated.1' AND json_extract(data,'$.info.role')='assistant' AND json_extract(data,'$.info.id')=?3 AND json_extract(data,'$.info.finish')='stop' AND json_extract(data,'$.info.time.completed') IS NOT NULL",
            _ => return Ok(false),
        };
        db.query_row(&format!("SELECT NOT EXISTS(SELECT 1 FROM event WHERE aggregate_id=?1 AND seq<?2 AND {condition})"),
            rusqlite::params![self.session_id,event.seq,id], |r| r.get(0)).map_err(|_| "kilo-db-schema-changed".into())
    }

    pub fn read(&mut self) -> Result<Vec<(ObservedEventKind, Option<SessionEventTokens>)>, String> {
        if stamp(&self.path)? != self.file_stamp {
            return Err("kilo-db-replaced".into());
        }
        let mut db = open(&self.path)?;
        let tx = db.transaction().map_err(|_| "kilo-db-unavailable")?;
        let (cwd, max_seq, _) = session_state(&tx, &self.session_id)?;
        if cwd != self.cwd || max_seq < self.watermark || stamp(&self.path)? != self.file_stamp {
            return Err("kilo-db-replaced".into());
        }
        if let Some(expected) = &self.watermark_id {
            let matches = tx
                .query_row(
                    "SELECT id=?3 FROM event WHERE aggregate_id=?1 AND seq=?2",
                    rusqlite::params![self.session_id, self.watermark, expected],
                    |r| r.get::<_, bool>(0),
                )
                .unwrap_or(false);
            if !matches {
                return Err("kilo-db-replaced".into());
            }
        }
        let events = {
            let mut query = tx.prepare(&format!("SELECT {EVENT_FIELDS} FROM event WHERE aggregate_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3"))
                .map_err(|_| "kilo-db-schema-changed")?;
            let rows = query
                .query_map(
                    rusqlite::params![self.session_id, self.watermark, EVENT_LIMIT],
                    Event::from_row,
                )
                .map_err(|_| "kilo-db-unavailable")?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|_| "kilo-db-schema-changed")?
        };
        let mut out = Vec::new();
        for event in events {
            if event.kind == "message.updated.1" {
                self.seed_usage(&tx, &event.usage)?;
                if let Some(delta) = self.usage.observe(&event.usage) {
                    self.pending_usage = Some(match self.pending_usage.take() {
                        Some(total) => total.merged(delta),
                        None => delta,
                    });
                }
            }
            if let Some((kind, id)) = event.activity() {
                if self.is_first_activity(&tx, &event, kind, id)? {
                    let tokens = if kind == ObservedEventKind::Stop {
                        self.pending_usage.take()
                    } else {
                        None
                    };
                    out.push((kind, tokens));
                }
            }
            self.watermark = event.seq;
            self.watermark_id = Some(event.id);
        }
        tx.commit().map_err(|_| "kilo-db-unavailable")?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    fn db() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("kilo.db");
        let c = Connection::open(&p).unwrap();
        c.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY,directory TEXT,time_updated INTEGER,time_archived INTEGER,parent_id TEXT);CREATE TABLE event(aggregate_id TEXT,seq INTEGER,type TEXT,data TEXT,id TEXT);CREATE INDEX event_seq ON event(aggregate_id,seq);").unwrap();
        c.execute(
            "INSERT INTO session VALUES('s1',?1,1,NULL,NULL)",
            [d.path().to_str().unwrap()],
        )
        .unwrap();
        (d, p)
    }
    fn e(c: &Connection, n: i64, t: &str, d: serde_json::Value) {
        c.execute(
            "INSERT INTO event VALUES('s1',?1,?2,?3,?4)",
            params![n, t, d.to_string(), format!("e{n}")],
        )
        .unwrap();
    }
    #[test]
    fn tails_only_new_deduplicated_safe_activity() {
        let (d, p) = db();
        let c = Connection::open(&p).unwrap();
        e(
            &c,
            1,
            "message.updated.1",
            serde_json::json!({"info":{"id":"old","role":"user","text":"secret"}}),
        );
        let candidate = discover_at(&p, Some(d.path().to_str().unwrap()))
            .pop()
            .unwrap();
        let mut t = Tail::from_candidate(&candidate).unwrap();
        e(
            &c,
            2,
            "message.updated.1",
            serde_json::json!({"info":{"id":"u","role":"user","text":"secret"}}),
        );
        e(
            &c,
            3,
            "message.updated.1",
            serde_json::json!({"info":{"id":"u","role":"user"}}),
        );
        e(
            &c,
            4,
            "message.part.updated.1",
            serde_json::json!({"part":{"id":"tool","type":"tool","state":{"status":"running","input":"secret"}}}),
        );
        e(
            &c,
            5,
            "message.updated.1",
            serde_json::json!({"info":{"id":"stop","role":"assistant","finish":"stop","time":{"completed":10}}}),
        );
        assert_eq!(
            t.read()
                .unwrap()
                .into_iter()
                .map(|x| x.0)
                .collect::<Vec<_>>(),
            vec![
                ObservedEventKind::Prompt,
                ObservedEventKind::Tool,
                ObservedEventKind::Stop
            ]
        );
        assert!(t.read().unwrap().is_empty());
    }
    #[test]
    fn missing_session_cannot_attach() {
        let (_d, p) = db();
        let c = Connection::open(&p).unwrap();
        let x = discover_at(&p, None).pop().unwrap();
        c.execute("DELETE FROM session", []).unwrap();
        assert_eq!(
            Tail::from_candidate(&x).unwrap_err(),
            "kilo-session-missing"
        );
    }
    fn assistant(
        id: &str,
        input: u64,
        output: u64,
        finish: Option<&str>,
        completed: bool,
    ) -> serde_json::Value {
        let mut value = serde_json::json!({"info":{"id":id,"role":"assistant","modelID":"model-a",
            "tokens":{"input":input,"output":output,"reasoning":2,"cache":{"read":3,"write":1}},"finish":finish}});
        if completed {
            value["info"]["time"] = serde_json::json!({"completed":10});
        }
        value
    }

    #[test]
    fn completion_waits_for_final_assistant_and_counts_only_new_snapshot_usage() {
        let (_d, p) = db();
        let c = Connection::open(&p).unwrap();
        e(
            &c,
            1,
            "message.updated.1",
            assistant("a", 100, 10, None, false),
        );
        let mut tail = Tail::from_candidate(&inspect(&p, "s1").unwrap()).unwrap();
        e(
            &c,
            2,
            "message.part.updated.1",
            serde_json::json!({"part":{"id":"finish","type":"step-finish","reason":"stop"}}),
        );
        e(
            &c,
            3,
            "message.updated.1",
            assistant("a", 105, 12, Some("stop"), false),
        );
        assert!(tail.read().unwrap().is_empty());
        e(
            &c,
            4,
            "message.updated.1",
            assistant("a", 105, 12, Some("stop"), true),
        );
        let events = tail.read().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, ObservedEventKind::Stop);
        let usage = events[0].1.as_ref().unwrap();
        assert_eq!(
            (usage.input, usage.output, usage.cache_read),
            (Some(5), Some(2), Some(0))
        );
        e(
            &c,
            5,
            "message.updated.1",
            assistant("a", 105, 12, Some("stop"), true),
        );
        assert!(tail.read().unwrap().is_empty());
        e(
            &c,
            6,
            "message.updated.1",
            assistant("b", 20, 4, Some("tool-calls"), true),
        );
        assert!(tail.read().unwrap().is_empty());
        e(
            &c,
            7,
            "message.updated.1",
            assistant("c", 30, 5, Some("stop"), true),
        );
        let events = tail.read().unwrap();
        let usage = events[0].1.as_ref().unwrap();
        assert_eq!((usage.input, usage.output), (Some(50), Some(13)));
    }

    #[test]
    fn old_summary_rewrite_is_not_a_prompt_after_more_than_a_dedup_window() {
        let (_d, p) = db();
        let mut c = Connection::open(&p).unwrap();
        e(
            &c,
            1,
            "message.updated.1",
            serde_json::json!({"info":{"id":"u","role":"user"}}),
        );
        {
            let tx = c.transaction().unwrap();
            for seq in 2..4200 {
                e(
                    &tx,
                    seq,
                    "message.part.updated.1",
                    serde_json::json!({"part":{"id":"text","type":"text","text":"not exposed"}}),
                );
            }
            tx.commit().unwrap();
        }
        let mut tail = Tail::from_candidate(&inspect(&p, "s1").unwrap()).unwrap();
        e(
            &c,
            4200,
            "message.updated.1",
            serde_json::json!({"info":{"id":"u","role":"user","summary":"not exposed"}}),
        );
        assert!(tail.read().unwrap().is_empty());
    }

    #[test]
    fn deletion_archival_cwd_change_and_sequence_reset_end_observation() {
        for mutation in [
            "DELETE FROM session",
            "UPDATE session SET time_archived=1",
            "UPDATE session SET parent_id='parent'",
            "UPDATE session SET directory='/other'",
            "DELETE FROM event",
            "UPDATE event SET id='replacement'",
        ] {
            let (_d, p) = db();
            let c = Connection::open(&p).unwrap();
            e(&c, 1, "session.updated.1", serde_json::json!({}));
            let mut tail = Tail::from_candidate(&inspect(&p, "s1").unwrap()).unwrap();
            c.execute(mutation, []).unwrap();
            assert!(tail.read().is_err(), "{mutation}");
        }
    }

    #[test]
    fn replaced_database_is_not_replayed_and_observer_connection_is_read_only() {
        let (_d, p) = db();
        let mut tail = Tail::from_candidate(&inspect(&p, "s1").unwrap()).unwrap();
        assert!(open(&p)
            .unwrap()
            .execute("DELETE FROM session", [])
            .is_err());
        let replacement = p.with_extension("replacement");
        std::fs::copy(&p, &replacement).unwrap();
        std::fs::rename(&replacement, &p).unwrap();
        assert!(tail.read().is_err());
    }

    #[test]
    fn archived_and_child_sessions_are_not_candidates() {
        let (d, p) = db();
        let c = Connection::open(&p).unwrap();
        for (id, parent, archived) in [("child", Some("s1"), None), ("archived", None, Some(10))] {
            c.execute(
                "INSERT INTO session VALUES(?1,?2,20,?3,?4)",
                rusqlite::params![id, d.path().to_str().unwrap(), archived, parent],
            )
            .unwrap();
            assert!(inspect(&p, id).is_err());
        }
        let candidates = discover_at(&p, None);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].source, "kilo-shared");
    }

    #[test]
    #[ignore = "read-only smoke against installed Kilo's local database"]
    fn installed_kilo_database_opens_at_current_watermark_without_history() {
        let candidates = discover(None);
        assert!(!candidates.is_empty(), "no local Kilo candidates");
        let mut tail = Tail::from_candidate(&candidates[0]).unwrap();
        assert!(tail.read().unwrap().is_empty());
    }
}
