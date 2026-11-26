// src-tauri/src/webdash/log.rs
//
// 웹 상태판의 대화 로그. 캐릭터마다 CLI 전사(JSONL)를 tail 하는 수집기를
// 하나씩 두고, 최근 항목을 순번(seq)과 함께 버퍼에 쌓는다. 브라우저는
// `after`(마지막으로 받은 seq)와 `epoch`를 보내 새 항목만 받는다. 붙은 전사
// 파일이 바뀌면(새 세션) 버퍼를 새 epoch로 다시 채운다 — 브라우저는 epoch가
// 바뀌면 들고 있던 항목을 버린다.
//
// 수집기는 누군가 그 캐릭터 로그를 볼 때만 만들고, 한동안 아무도 안 보면 버린다.
//
// 전사에 실린 이미지(붙여 넣은 그림, 도구가 읽은 그림)는 항목에 base64로 싣지 않는다.
// 디코딩한 바이트를 캐릭터별 상자에 넣고 항목에는 id만 둔다 — 브라우저는
// `/api/image/<캐릭터>/<id>`로 따로 받는다. 상자는 개수·바이트 상한에서 오래된 것부터 버린다.
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::Serialize;

use super::files::{self, FileLink};
use crate::session_log::agent_transcript::{
    self, AgentSessionLookup, ItemKind, ItemRole, TranscriptItem, TranscriptTailer, BACKFILL_MAX_BYTES,
    BACKFILL_MAX_ITEMS,
};

/// 캐릭터당 들고 있는 이미지 수 상한.
const MAX_IMAGES: usize = 60;
/// 캐릭터당 들고 있는 이미지 바이트 상한.
const MAX_IMAGE_STORE_BYTES: usize = 64 * 1024 * 1024;
/// 이보다 큰 이미지 하나는 받지 않는다.
const MAX_IMAGE_BYTES: usize = 24 * 1024 * 1024;

/// 항목에 붙는 이미지 참조.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageMeta {
    pub id: String,
    pub media_type: String,
    pub bytes: usize,
    /// 그림이 온 파일 경로나 URL(전사가 알려 줄 때만). 로그에 그림과 함께 늘 보인다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

struct StoredImage {
    id: String,
    media_type: String,
    bytes: Arc<Vec<u8>>,
}

/// 캐릭터당 들고 있는 최대 항목 수. 백필 상한과 같아, 진입할 때 읽은 과거를 버리지 않는다.
const MAX_ITEMS: usize = BACKFILL_MAX_ITEMS;
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageMeta>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files: Vec<FileLink>,
    /// 에이전트 말을 Markdown으로 그려 거른 HTML(웹 상태판만, `markdown::render`).
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<String>,
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
    file_root: Option<PathBuf>,
    tailer: TranscriptTailer,
    targets: Vec<PathBuf>,
    items: VecDeque<LogItem>,
    next_seq: u64,
    epoch: u64,
    last_tick: Option<Instant>,
    last_access: Instant,
    /// 이미지를 보이는가(웹 상태판 설정). 끄면 이미지 항목을 아예 쌓지 않는다.
    show_images: bool,
    /// 에이전트 말을 HTML로 그려 싣는가(웹 상태판). 앱 대화 보기는 렌더러가 그린다.
    markdown: bool,
    images: VecDeque<StoredImage>,
}

impl AgentLog {
    fn push(&mut self, mut item: TranscriptItem) {
        let image = match (item.kind, item.image.take()) {
            (ItemKind::Image, Some(img)) => {
                if !self.show_images {
                    return;
                }
                let Some(mut meta) = self.store_image(&img.media_type, &img.data) else {
                    return;
                };
                meta.source = img.source;
                Some(meta)
            }
            (ItemKind::Image, None) => return,
            _ => None,
        };
        let item = item.clamped_for_web();
        let files = match (&self.file_root, item.kind, item.role) {
            (Some(root), ItemKind::Text, ItemRole::Assistant) if item.text.contains(']') => {
                files::links(root, &item.text)
            }
            _ => Vec::new(),
        };
        let html = (self.markdown && item.kind == ItemKind::Text && item.role == ItemRole::Assistant && !item.text.is_empty())
            .then(|| super::markdown::render(&item.text));
        self.next_seq += 1;
        self.items.push_back(LogItem { seq: self.next_seq, item, image, files, html });
        while self.items.len() > MAX_ITEMS {
            self.items.pop_front();
        }
    }

    /// base64를 풀어 상자에 넣는다. 같은 그림이면 있던 것을 쓴다.
    fn store_image(&mut self, media_type: &str, data: &str) -> Option<ImageMeta> {
        let engine = base64::engine::general_purpose::STANDARD;
        let bytes = engine
            .decode(data)
            .or_else(|_| engine.decode(data.split_ascii_whitespace().collect::<String>()))
            .ok()?;
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
            return None;
        }
        let id = image_id(&bytes);
        if let Some(pos) = self.images.iter().position(|i| i.id == id) {
            let found = self.images.remove(pos).expect("position is in range");
            let meta = ImageMeta { id, media_type: found.media_type.clone(), bytes: found.bytes.len(), source: None };
            self.images.push_back(found);
            return Some(meta);
        }
        let meta = ImageMeta { id: id.clone(), media_type: media_type.to_string(), bytes: bytes.len(), source: None };
        self.images.push_back(StoredImage { id, media_type: media_type.to_string(), bytes: Arc::new(bytes) });
        let mut total: usize = self.images.iter().map(|i| i.bytes.len()).sum();
        while self.images.len() > MAX_IMAGES || (total > MAX_IMAGE_STORE_BYTES && self.images.len() > 1) {
            if let Some(old) = self.images.pop_front() {
                total -= old.bytes.len();
            }
        }
        Some(meta)
    }

    fn image(&self, id: &str) -> Option<(String, Arc<Vec<u8>>)> {
        self.images.iter().find(|i| i.id == id).map(|i| (i.media_type.clone(), i.bytes.clone()))
    }

    /// 붙은 파일의 최근 항목으로 버퍼를 새로 채운다.
    fn reseed(&mut self) {
        self.items.clear();
        self.images.clear();
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

/// 내용으로 정한 이미지 id(같은 그림은 같은 id).
fn image_id(bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.len().hash(&mut h);
    bytes.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn new_epoch() -> u64 {
    let id = uuid::Uuid::new_v4();
    u64::from_le_bytes(id.as_bytes()[..8].try_into().expect("8 bytes")) >> 11
}

/// 전사 소스 묶음을 만든다. 시험이 가짜 소스를 넣을 수 있게 주입한다.
pub type SourceFactory =
    Arc<dyn Fn() -> Vec<Box<dyn agent_transcript::TranscriptSource>> + Send + Sync>;

pub fn production_sources(
    lookup: Arc<dyn AgentSessionLookup>,
    agy_store: Arc<crate::persistence::agy_resume_store::AgyResumeStore>,
) -> SourceFactory {
    Arc::new(move || {
        let mut sources: Vec<Box<dyn agent_transcript::TranscriptSource>> = Vec::new();
        sources.extend(agent_transcript::claude::source(lookup.clone()));
        sources.extend(agent_transcript::codex::source());
        sources.extend(agent_transcript::agy::source(agy_store.clone()));
        sources
    })
}

pub struct LogCache {
    sources: SourceFactory,
    agents: Mutex<HashMap<String, Arc<Mutex<AgentLog>>>>,
    show_images: AtomicBool,
    markdown: bool,
}

impl LogCache {
    /// `markdown`: 에이전트 말을 HTML로 그려 싣는가(웹 상태판).
    pub fn new(sources: SourceFactory, markdown: bool) -> Self {
        Self { sources, agents: Mutex::new(HashMap::new()), show_images: AtomicBool::new(true), markdown }
    }

    /// 이미지 보이기 설정. 바뀌면 들고 있던 로그를 버린다 — 다음 요청이 새 epoch로 다시 채운다.
    pub fn set_show_images(&self, on: bool) {
        if self.show_images.swap(on, Ordering::SeqCst) != on {
            self.agents.lock().unwrap().clear();
        }
    }

    /// 로그에 실린 이미지 하나(형식, 바이트). 설정이 꺼졌거나 상자에서 밀려났으면 None.
    pub fn image(&self, agent_id: &str, id: &str) -> Option<(String, Arc<Vec<u8>>)> {
        if !self.show_images.load(Ordering::SeqCst) {
            return None;
        }
        let log = self.agents.lock().unwrap().get(agent_id).cloned()?;
        let log = log.lock().unwrap();
        log.image(id)
    }

    pub(crate) fn file(&self, agent_id: &str, epoch: u64, seq: u64, index: usize) -> Option<(String, Vec<u8>)> {
        let log = self.agents.lock().unwrap().get(agent_id).cloned()?;
        let (root, link) = {
            let log = log.lock().unwrap();
            if log.epoch != epoch {
                return None;
            }
            let item = log.items.iter().find(|item| item.seq == seq)?;
            (log.file_root.clone()?, item.files.get(index)?.clone())
        };
        let bytes = files::read(&root, &link)?;
        Some((link.name, bytes))
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
                        file_root: files::working_root(cwd),
                        tailer: TranscriptTailer::new(agent_id, cwd, (self.sources)()),
                        targets: Vec::new(),
                        items: VecDeque::new(),
                        next_seq: 0,
                        epoch: new_epoch(),
                        last_tick: None,
                        last_access: Instant::now(),
                        show_images: self.show_images.load(Ordering::SeqCst),
                        markdown: self.markdown,
                        images: VecDeque::new(),
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

    /// 지정한 파일을 붙고 줄마다 발화 하나를 내는 가짜 소스.
    struct FileSource(Arc<Mutex<PathBuf>>, ItemRole);
    impl TranscriptSource for FileSource {
        fn label(&self) -> &'static str {
            "fake"
        }
        fn locate(&mut self, _: &str, _: &str) -> Option<PathBuf> {
            Some(self.0.lock().unwrap().clone())
        }
        fn parse(&self, raw: &str) -> Vec<TranscriptItem> {
            vec![TranscriptItem::speech(self.1, raw)]
        }
    }

    /// 줄마다 이미지 항목 하나(본문은 줄 그대로 base64로 본다).
    struct ImageSource(PathBuf);
    impl TranscriptSource for ImageSource {
        fn label(&self) -> &'static str {
            "img"
        }
        fn locate(&mut self, _: &str, _: &str) -> Option<PathBuf> {
            Some(self.0.clone())
        }
        fn parse(&self, raw: &str) -> Vec<TranscriptItem> {
            let img = agent_transcript::ItemImage::new("image/png", raw).unwrap();
            vec![TranscriptItem::speech(ItemRole::User, "보세요"), TranscriptItem::image(ItemRole::User, img)]
        }
    }

    #[test]
    fn images_are_served_by_id_not_sent_inline_and_can_be_turned_off() {
        let dir = std::env::temp_dir().join(format!("webdash-img-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("i.jsonl");
        let engine = base64::engine::general_purpose::STANDARD;
        append(&path, &engine.encode(b"PNG-ONE"));
        append(&path, "%%not base64%%");
        let cache = LogCache::new({
            let path = path.clone();
            Arc::new(move || vec![Box::new(ImageSource(path.clone())) as Box<dyn TranscriptSource>])
        }, false);
        let page = cache.read("x", "/w", None, None);
        let images: Vec<&ImageMeta> = page.items.iter().filter_map(|i| i.image.as_ref()).collect();
        // 풀 수 없는 base64는 버리고, 글 항목은 그대로 남는다.
        assert_eq!(images.len(), 1);
        assert_eq!(page.items.len(), 3);
        let wire = serde_json::to_string(&page).unwrap();
        assert!(!wire.contains(&engine.encode(b"PNG-ONE")), "{wire}");
        assert!(wire.contains(r#""kind":"image""#), "{wire}");
        let (media, bytes) = cache.image("x", &images[0].id).unwrap();
        assert_eq!((media.as_str(), bytes.as_slice()), ("image/png", b"PNG-ONE".as_slice()));
        assert!(cache.image("x", "nope").is_none());

        cache.set_show_images(false);
        assert!(cache.image("x", &images[0].id).is_none());
        let off = cache.read("x", "/w", None, None);
        assert_ne!(off.epoch, page.epoch);
        assert!(off.items.iter().all(|i| i.item.kind != ItemKind::Image));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn append(path: &PathBuf, line: &str) {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        writeln!(f, "{line}").unwrap();
    }

    #[test]
    fn downloads_only_linked_workspace_files_from_the_current_log() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("work");
        std::fs::create_dir(&workspace).unwrap();
        let report = workspace.join("실행 결과.pdf");
        let outside = dir.path().join("private.txt");
        std::fs::write(&report, b"report").unwrap();
        std::fs::write(&outside, b"private").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, workspace.join("alias.txt")).unwrap();

        let path = dir.path().join("chat.jsonl");
        append(&path, "[결과](실행%20결과.pdf) [외부](../private.txt) [별칭](alias.txt) [웹](https://example.org)");
        let current = Arc::new(Mutex::new(path));
        let cache = LogCache::new(Arc::new(move || {
            vec![Box::new(FileSource(current.clone(), ItemRole::Assistant))]
        }), false);
        let page = cache.read("a", workspace.to_str().unwrap(), None, None);
        let seq = page.items[0].seq;
        let wire = serde_json::to_value(&page.items[0]).unwrap();
        assert_eq!(wire["files"], serde_json::json!([{ "name": "실행 결과.pdf" }]));
        assert_eq!(
            cache.file("a", page.epoch, seq, 0),
            Some(("실행 결과.pdf".into(), b"report".to_vec()))
        );
        assert!(cache.file("a", page.epoch, seq, 1).is_none());
        assert!(cache.file("a", page.epoch + 1, seq, 0).is_none());

        std::fs::remove_file(&report).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &report).unwrap();
        assert!(cache.file("a", page.epoch, seq, 0).is_none());
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
            Arc::new(move || vec![Box::new(FileSource(current.clone(), ItemRole::User)) as Box<dyn TranscriptSource>])
        }, false);

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
