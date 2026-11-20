// src-tauri/src/webdash/log.rs
//
// 웹 상태판의 대화 로그. 캐릭터마다 CLI 전사(JSONL)를 tail 하는 수집기를
// 하나씩 두고, 최근 항목을 순번(seq)과 함께 버퍼에 쌓는다. 브라우저는
// `after`(마지막으로 받은 seq)와 `epoch`를 보내 새 항목만 받는다. 붙은 전사
// 파일이 바뀌면(새 세션) 버퍼를 새 epoch로 다시 채운다 — 브라우저는 epoch가
// 바뀌면 들고 있던 항목을 버린다.
//
// 수집기는 누군가 그 캐릭터 로그를 볼 때만 만들고, 한동안 아무도 안 보면 버린다.
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::session_log::agent_transcript::{
    self, AgentSessionLookup, TranscriptItem, TranscriptTailer, BACKFILL_MAX_BYTES,
    BACKFILL_MAX_ITEMS,
};

/// 캐릭터당 들고 있는 최대 항목 수.
const MAX_ITEMS: usize = 300;
/// 같은 캐릭터 전사를 다시 읽기 전 최소 간격.
const MIN_TICK: Duration = Duration::from_millis(1000);
/// 이만큼 아무도 안 본 수집기는 버린다.
const IDLE_DROP: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogItem {
    pub seq: u64,
    #[serde(flatten)]
    pub item: TranscriptItem,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPage {
    pub epoch: u64,
    /// 버퍼의 마지막 seq(없으면 0). 다음 요청의 `after`.
    pub last: u64,
    /// 전사 파일에 붙어 있는가. false면 대화 로그를 보일 수 없다(일반 셸 등).
    pub attached: bool,
    pub items: Vec<LogItem>,
}

struct AgentLog {
    cwd: String,
    tailer: TranscriptTailer,
    targets: Vec<PathBuf>,
    items: VecDeque<LogItem>,
    next_seq: u64,
    epoch: u64,
    last_tick: Option<Instant>,
    last_access: Instant,
}

impl AgentLog {
    fn push(&mut self, item: TranscriptItem) {
        self.next_seq += 1;
        self.items.push_back(LogItem { seq: self.next_seq, item: item.clamped_for_web() });
        while self.items.len() > MAX_ITEMS {
            self.items.pop_front();
        }
    }

    /// 붙은 파일의 최근 항목으로 버퍼를 새로 채운다.
    fn reseed(&mut self) {
        self.items.clear();
        self.epoch = new_epoch();
        for item in self.tailer.backfill(BACKFILL_MAX_BYTES, BACKFILL_MAX_ITEMS) {
            self.push(item);
        }
    }

    fn tick(&mut self) {
        if self.last_tick.is_some_and(|t| t.elapsed() < MIN_TICK) {
            return;
        }
        let first = self.last_tick.is_none();
        self.last_tick = Some(Instant::now());
        let fresh = self.tailer.tick_items();
        let targets = self.tailer.targets();
        if first || targets != self.targets {
            // 처음 붙었거나 다른 파일로 옮겨 갔다. 새로 연 파일은 끝부터 읽으므로
            // 최근 대화는 거슬러 읽어 채운다.
            self.targets = targets;
            self.reseed();
            return;
        }
        for item in fresh {
            self.push(item);
        }
    }

    fn page(&self, after: Option<u64>, epoch: Option<u64>) -> LogPage {
        let from = if epoch == Some(self.epoch) { after.unwrap_or(0) } else { 0 };
        LogPage {
            epoch: self.epoch,
            last: self.next_seq,
            attached: self.tailer.has_target(),
            items: self.items.iter().filter(|i| i.seq > from).cloned().collect(),
        }
    }
}

fn new_epoch() -> u64 {
    let id = uuid::Uuid::new_v4();
    u64::from_le_bytes(id.as_bytes()[..8].try_into().expect("8 bytes")) >> 11
}

/// 전사 소스 묶음을 만든다. 시험이 가짜 소스를 넣을 수 있게 주입한다.
pub type SourceFactory =
    Arc<dyn Fn() -> Vec<Box<dyn agent_transcript::TranscriptSource>> + Send + Sync>;

pub fn production_sources(lookup: Arc<dyn AgentSessionLookup>) -> SourceFactory {
    Arc::new(move || {
        let mut sources: Vec<Box<dyn agent_transcript::TranscriptSource>> = Vec::new();
        sources.extend(agent_transcript::claude::source(lookup.clone()));
        sources.extend(agent_transcript::codex::source());
        sources
    })
}

pub struct LogCache {
    sources: SourceFactory,
    agents: Mutex<HashMap<String, Arc<Mutex<AgentLog>>>>,
}

impl LogCache {
    pub fn new(sources: SourceFactory) -> Self {
        Self { sources, agents: Mutex::new(HashMap::new()) }
    }

    /// 한 캐릭터의 로그 쪽. 파일을 읽으므로 블로킹 스레드에서 부른다.
    pub fn read(&self, agent_id: &str, cwd: &str, after: Option<u64>, epoch: Option<u64>) -> LogPage {
        let entry = {
            let mut agents = self.agents.lock().unwrap();
            agents.retain(|_, log| log.lock().map(|l| l.last_access.elapsed() < IDLE_DROP).unwrap_or(false));
            let stale = agents
                .get(agent_id)
                .is_some_and(|log| log.lock().map(|l| l.cwd != cwd).unwrap_or(true));
            if stale {
                agents.remove(agent_id);
            }
            agents
                .entry(agent_id.to_string())
                .or_insert_with(|| {
                    Arc::new(Mutex::new(AgentLog {
                        cwd: cwd.to_string(),
                        tailer: TranscriptTailer::new(agent_id, cwd, (self.sources)()),
                        targets: Vec::new(),
                        items: VecDeque::new(),
                        next_seq: 0,
                        epoch: new_epoch(),
                        last_tick: None,
                        last_access: Instant::now(),
                    }))
                })
                .clone()
        };
        let mut log = entry.lock().unwrap();
        log.last_access = Instant::now();
        log.tick();
        log.page(after, epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_log::agent_transcript::{ItemRole, TranscriptSource};
    use std::io::Write;

    /// 지정한 파일을 붙고 줄마다 사용자 발화 하나를 내는 가짜 소스.
    struct FileSource(Arc<Mutex<PathBuf>>);
    impl TranscriptSource for FileSource {
        fn label(&self) -> &'static str {
            "fake"
        }
        fn locate(&mut self, _: &str, _: &str) -> Option<PathBuf> {
            Some(self.0.lock().unwrap().clone())
        }
        fn parse(&self, raw: &str) -> Vec<TranscriptItem> {
            vec![TranscriptItem::speech(ItemRole::User, raw)]
        }
    }

    fn append(path: &PathBuf, line: &str) {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        writeln!(f, "{line}").unwrap();
    }

    #[test]
    fn backfills_then_streams_new_items_and_resets_on_file_switch() {
        let dir = std::env::temp_dir().join(format!("webdash-log-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.jsonl");
        let b = dir.join("b.jsonl");
        append(&a, "one");
        append(&a, "two");
        let current = Arc::new(Mutex::new(a.clone()));
        let cache = LogCache::new({
            let current = current.clone();
            Arc::new(move || vec![Box::new(FileSource(current.clone())) as Box<dyn TranscriptSource>])
        });

        let first = cache.read("x", "/w", None, None);
        assert!(first.attached);
        assert_eq!(first.items.iter().map(|i| i.item.text.as_str()).collect::<Vec<_>>(), ["one", "two"]);

        append(&a, "three");
        // 최소 간격 안에서는 다시 읽지 않는다.
        cache.agents.lock().unwrap()["x"].lock().unwrap().last_tick = Some(Instant::now() - MIN_TICK);
        let next = cache.read("x", "/w", Some(first.last), Some(first.epoch));
        assert_eq!(next.epoch, first.epoch);
        assert_eq!(next.items.iter().map(|i| i.item.text.as_str()).collect::<Vec<_>>(), ["three"]);

        append(&b, "fresh");
        *current.lock().unwrap() = b.clone();
        cache.agents.lock().unwrap()["x"].lock().unwrap().last_tick = Some(Instant::now() - MIN_TICK);
        let switched = cache.read("x", "/w", Some(next.last), Some(next.epoch));
        assert_ne!(switched.epoch, first.epoch);
        assert_eq!(switched.items.iter().map(|i| i.item.text.as_str()).collect::<Vec<_>>(), ["fresh"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
