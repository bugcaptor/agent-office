//! Append-only terminal history for a running headless host. One writer performs
//! disk IO; replay uses a fixed file boundary and bounded pages, never the full log.
use super::protocol::HostMsg;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    sync::{mpsc, Arc},
    time::Duration,
};
use tokio::sync::{broadcast, oneshot};

#[derive(Serialize, Deserialize)]
pub struct Record {
    pub at: u64,
    pub frame: HostMsg,
}
struct Entry {
    file: File,
    path: PathBuf,
    end: u64,
    total: u64,
    /// Session identity belongs to the same writer-queue position as `end`.
    /// A replay must never pair an old file boundary with a newer session.
    session_id: Option<String>,
}
enum Request {
    Append {
        agent: String,
        record: Record,
        reset: bool,
    },
    Snapshot {
        agent: String,
        reply: oneshot::Sender<Result<Option<Snapshot>, String>>,
    },
    Flush(oneshot::Sender<Result<(), String>>),
}
#[derive(Debug)]
pub struct Snapshot {
    path: PathBuf,
    end: u64,
    pub total: u64,
    pub session_id: Option<String>,
}
pub struct Reader {
    reader: BufReader<std::io::Take<File>>,
    start: u64,
}
impl Snapshot {
    pub fn reader(self, start: u64) -> Result<Reader, String> {
        let file = File::open(self.path).map_err(|e| e.to_string())?;
        Ok(Reader {
            reader: BufReader::new(file.take(self.end)),
            start,
        })
    }
}
impl Reader {
    pub fn page(&mut self) -> Result<Vec<Record>, String> {
        let mut page = Vec::new();
        let mut bytes = 0;
        loop {
            let mut line = String::new();
            let n = self
                .reader
                .read_line(&mut line)
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Ok(page);
            }
            let record: Record = serde_json::from_str(&line).map_err(|e| e.to_string())?;
            if record.at >= self.start {
                page.push(record);
                bytes += n;
            }
            if bytes >= 256 * 1024 || page.len() >= 128 {
                return Ok(page);
            }
        }
    }
}

pub struct Journal {
    tx: mpsc::SyncSender<Request>,
    error: Arc<Mutex<Option<String>>>,
}
impl Journal {
    pub fn new(dir: PathBuf, broadcast: broadcast::Sender<Arc<HostMsg>>) -> Self {
        let (tx, rx) = mpsc::sync_channel(512);
        let error = Arc::new(Mutex::new(None));
        let failure = error.clone();
        std::thread::Builder::new()
            .name("terminal-journal".into())
            .spawn(move || {
                let mut files: HashMap<String, Entry> = HashMap::new();
                for request in rx {
                    match request {
                        Request::Append {
                            agent,
                            record,
                            reset,
                        } => {
                            let result = (|| -> std::io::Result<()> {
                                if reset || !files.contains_key(&agent) {
                                    std::fs::create_dir_all(&dir)?;
                                    // Random names avoid profile IDs being interpreted as paths,
                                    // and preserve earlier sessions when a profile is restarted.
                                    let path = dir.join(format!("{}.jsonl", uuid::Uuid::new_v4()));
                                    let mut options = OpenOptions::new();
                                    options.create_new(true).write(true);
                                    #[cfg(unix)]
                                    {
                                        use std::os::unix::fs::OpenOptionsExt;
                                        options.mode(0o600);
                                    }
                                    let file = options.open(&path)?;
                                    files.insert(
                                        agent.clone(),
                                        Entry {
                                            file,
                                            path,
                                            end: 0,
                                            total: 0,
                                            session_id: None,
                                        },
                                    );
                                }
                                let entry = files.get_mut(&agent).unwrap();
                                let mut data = serde_json::to_vec(&record)?;
                                data.push(b'\n');
                                entry.file.write_all(&data)?;
                                entry.end += data.len() as u64;
                                if let HostMsg::Output(ref out) = record.frame {
                                    entry.total = out.offset + out.bytes;
                                    entry.session_id = Some(out.session_id.clone());
                                } else if let HostMsg::Restore { ref session_id, .. } = record.frame
                                {
                                    entry.session_id = session_id.clone();
                                }
                                Ok(())
                            })();
                            match result {
                                Ok(()) if failure.lock().is_none() => {
                                    let _ = broadcast.send(Arc::new(record.frame));
                                }
                                Err(_) => {
                                    *failure.lock() = Some("terminal-journal-write-failed".into());
                                    let _ = broadcast.send(Arc::new(HostMsg::Error {
                                        message: "terminal-journal-write-failed".into(),
                                    }));
                                }
                                _ => {}
                            }
                        }
                        Request::Snapshot { agent, reply } => {
                            let result = if let Some(error) = failure.lock().clone() {
                                Err(error)
                            } else {
                                Ok(files.get(&agent).map(|e| Snapshot {
                                    path: e.path.clone(),
                                    end: e.end,
                                    total: e.total,
                                    session_id: e.session_id.clone(),
                                }))
                            };
                            let _ = reply.send(result);
                        }
                        Request::Flush(reply) => {
                            let _ = reply.send(failure.lock().clone().map_or(Ok(()), Err));
                        }
                    }
                }
            })
            .expect("terminal journal writer");
        Self { tx, error }
    }
    pub fn append(&self, agent: &str, at: u64, frame: HostMsg, reset: bool) {
        // Bounded producer backpressure prevents unbounded memory on a slow disk.
        // No filesystem operation is executed by the PTY output task itself.
        if self
            .tx
            .send(Request::Append {
                agent: agent.into(),
                record: Record { at, frame },
                reset,
            })
            .is_err()
        {
            *self.error.lock() = Some("terminal-journal-unavailable".into());
        }
    }
    pub async fn snapshot(&self, agent: &str) -> Result<Option<Snapshot>, String> {
        let (reply, rx) = oneshot::channel();
        let tx = self.tx.clone();
        let agent = agent.to_owned();
        tokio::task::spawn_blocking(move || tx.send(Request::Snapshot { agent, reply }))
            .await
            .map_err(|_| "terminal-journal-unavailable")?
            .map_err(|_| "terminal-journal-unavailable")?;
        tokio::time::timeout(Duration::from_secs(10), rx)
            .await
            .map_err(|_| "terminal-journal-timeout")?
            .map_err(|_| "terminal-journal-unavailable".to_string())?
    }
    pub async fn flush(&self) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || tx.send(Request::Flush(reply)))
            .await
            .map_err(|_| "terminal-journal-unavailable")?
            .map_err(|_| "terminal-journal-unavailable")?;
        rx.await
            .map_err(|_| "terminal-journal-unavailable".to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::super::protocol::RemoteOutput;
    use super::*;
    #[tokio::test]
    async fn history_exceeds_ring_and_replays_resize_in_bounded_pages() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = broadcast::channel(4096);
        let journal = Journal::new(dir.path().to_owned(), tx);
        let data = "가나다".repeat(4096);
        let bytes = data.len() as u64;
        for seq in 0..40 {
            if seq == 20 {
                journal.append(
                    "../unsafe-id",
                    seq * bytes,
                    HostMsg::Resized {
                        agent_id: "../unsafe-id".into(),
                        cols: 120,
                        rows: 40,
                    },
                    false,
                );
            }
            journal.append(
                "../unsafe-id",
                seq * bytes,
                HostMsg::Output(RemoteOutput {
                    replay: false,
                    agent_id: "../unsafe-id".into(),
                    session_id: "s".into(),
                    seq,
                    offset: seq * bytes,
                    bytes,
                    data: data.clone(),
                }),
                false,
            );
        }
        let snapshot = journal.snapshot("../unsafe-id").await.unwrap().unwrap();
        assert!(snapshot.total > super::super::host::RING_CAP_BYTES as u64);
        let boundary = snapshot.total;
        // New output cannot change an already captured replay boundary.
        journal.append(
            "../unsafe-id",
            boundary,
            HostMsg::Output(RemoteOutput {
                replay: false,
                agent_id: "../unsafe-id".into(),
                session_id: "s".into(),
                seq: 40,
                offset: boundary,
                bytes: 1,
                data: "x".into(),
            }),
            false,
        );
        let mut reader = snapshot.reader(0).unwrap();
        let mut count = 0;
        let mut resized_after = None;
        let mut total = 0;
        loop {
            let page = reader.page().unwrap();
            if page.is_empty() {
                break;
            }
            assert!(page.len() < 40);
            for record in page {
                match record.frame {
                    HostMsg::Output(out) => {
                        assert_eq!(out.offset, total);
                        total += out.bytes;
                        count += 1;
                    }
                    HostMsg::Resized {
                        cols: 120,
                        rows: 40,
                        ..
                    } => resized_after = Some(count),
                    _ => panic!("unexpected record"),
                }
            }
        }
        assert_eq!(count, 40);
        assert_eq!(resized_after, Some(20));
        assert_eq!(total, boundary);
        let mut resumed = journal
            .snapshot("../unsafe-id")
            .await
            .unwrap()
            .unwrap()
            .reader(30 * bytes)
            .unwrap();
        let first = resumed.page().unwrap();
        assert_eq!(first[0].at, 30 * bytes);
        assert!(!dir
            .path()
            .parent()
            .unwrap()
            .join("unsafe-id.jsonl")
            .exists());
    }

    #[tokio::test]
    async fn snapshot_keeps_its_session_when_a_new_session_starts_after_capture() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = broadcast::channel(4096);
        let journal = Journal::new(dir.path().to_owned(), tx);
        let data = "x".repeat(8192);
        let bytes = data.len() as u64;

        // More than one bounded replay page makes this represent a restore
        // which is still sending S1 when another client restarts the agent.
        for seq in 0..160 {
            journal.append(
                "ada",
                seq * bytes,
                HostMsg::Output(RemoteOutput {
                    replay: false,
                    agent_id: "ada".into(),
                    session_id: "s1".into(),
                    seq,
                    offset: seq * bytes,
                    bytes,
                    data: data.clone(),
                }),
                false,
            );
        }
        let captured = journal.snapshot("ada").await.unwrap().unwrap();
        assert_eq!(captured.session_id.as_deref(), Some("s1"));

        journal.append(
            "ada",
            0,
            HostMsg::Restore {
                agent_id: "ada".into(),
                snapshot: Some(String::new()),
                base_offset: 0,
                cols: 80,
                rows: 24,
                session_id: Some("s2".into()),
            },
            true,
        );
        journal.append(
            "ada",
            0,
            HostMsg::Output(RemoteOutput {
                replay: false,
                agent_id: "ada".into(),
                session_id: "s2".into(),
                seq: 0,
                offset: 0,
                bytes: 1,
                data: "new".into(),
            }),
            false,
        );

        let mut reader = captured.reader(0).unwrap();
        let mut restored = 0;
        loop {
            let page = reader.page().unwrap();
            if page.is_empty() {
                break;
            }
            for record in page {
                if let HostMsg::Output(output) = record.frame {
                    assert_eq!(output.session_id, "s1");
                    restored += 1;
                }
            }
        }
        assert_eq!(restored, 160);

        let current = journal.snapshot("ada").await.unwrap().unwrap();
        assert_eq!(current.session_id.as_deref(), Some("s2"));
        assert_eq!(current.total, 1);
    }
}
