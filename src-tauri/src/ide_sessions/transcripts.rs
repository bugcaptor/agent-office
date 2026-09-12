//! Read-only adapters for the small, version-sensitive part of IDE transcript files.
//!
//! This module deliberately returns metadata and activity kinds only.  It never exposes
//! transcript text, prompts, tool arguments, or file contents to the rest of the app.

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use crate::session::external::ObservedEventKind;
use crate::types::{SessionEventTokens, SessionModelTokens};

const MAX_LINE: usize = 2 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;
const MAX_TICK: u64 = 4 * 1024 * 1024;
const DISCOVERY_BUDGET: usize = 2_000;
const INSPECT_LIMIT: usize = 200;
const RESULT_LIMIT: usize = 20;
// Attachment scans only enough metadata to establish a conservative watermark.  It
// never returns transcript content and a missing watermark means "skip until known".
const USAGE_BASELINE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub provider: String,
    pub source_session_id: String,
    pub cwd: String,
    pub file: String,
    pub updated_at: u64,
    pub source: String,
    #[serde(skip)]
    baseline: Baseline,
}

impl Candidate {
    pub(crate) fn kilo(
        source_session_id: String,
        cwd: String,
        file: String,
        updated_at: u64,
    ) -> Self {
        Self {
            provider: "kilo".into(),
            source_session_id,
            cwd,
            file,
            updated_at,
            source: "kilo-shared".into(),
            baseline: Baseline {
                identity: FileIdentity("kilo-db".into()),
                size: 0,
            },
        }
    }
}

#[derive(Debug, Clone)]
struct Baseline {
    identity: FileIdentity,
    size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileIdentity(String);

fn identity(meta: &fs::Metadata) -> FileIdentity {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return FileIdentity(format!("{}:{}", meta.dev(), meta.ino()));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // `std` does not expose the Windows file index.  Creation time is stable while
        // an active transcript grows, unlike last-write time and file size.
        return FileIdentity(meta.creation_time().to_string());
    }
    #[cfg(not(any(unix, windows)))]
    FileIdentity(format!("{:?}:{}", meta.modified().ok(), meta.len()))
}

#[cfg(windows)]
fn identity_for_file(file: &File, meta: &fs::Metadata) -> FileIdentity {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `file` owns a live Windows file handle for this call and `info` is a
    // correctly sized writable output buffer.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } != 0 {
        return FileIdentity(format!(
            "{}:{}",
            info.dwVolumeSerialNumber,
            ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64
        ));
    }
    identity(meta)
}

#[cfg(not(windows))]
fn identity_for_file(_: &File, meta: &fs::Metadata) -> FileIdentity {
    identity(meta)
}

fn modified_ms(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis().min(u64::MAX as u128) as u64)
                .unwrap_or(0)
        })
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)?.as_str()
}

fn is_truthy(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Bool(true)))
}

fn metadata(provider: &str, records: &[Value]) -> Option<(String, String, String)> {
    match provider {
        "codex" => {
            let payload = records.iter().find_map(|r| {
                (text(r, "type") == Some("session_meta"))
                    .then(|| r.get("payload"))
                    .flatten()
            })?;
            let session_id = text(payload, "id")
                .or_else(|| text(payload, "session_id"))
                .filter(|id| !id.is_empty())?;
            let cwd = text(payload, "cwd")?;
            let subagent = payload
                .get("parent_thread_id")
                .is_some_and(|v| !v.is_null())
                || is_truthy(payload.get("source").and_then(|v| v.get("subagent")))
                || text(payload, "thread_source") == Some("subagent")
                || is_truthy(payload.get("thread_source").and_then(|v| v.get("subagent")));
            let vscode = text(payload, "source") == Some("vscode")
                && text(payload, "originator") == Some("codex_vscode")
                && text(payload, "thread_source") == Some("user");
            Some((
                session_id.to_string(),
                cwd.to_string(),
                if subagent {
                    "subagent"
                } else if vscode {
                    "vscode"
                } else {
                    "unknown"
                }
                .to_string(),
            ))
        }
        "claude" => {
            let record = records
                .iter()
                .find(|r| text(r, "sessionId").is_some() && text(r, "cwd").is_some())?;
            let session_id = text(record, "sessionId").filter(|id| !id.is_empty())?;
            let cwd = text(record, "cwd")?;
            let source = if is_truthy(record.get("isSidechain")) {
                "subagent"
            } else if text(record, "entrypoint") == Some("claude-vscode") {
                "vscode"
            } else {
                "unknown"
            };
            Some((session_id.to_string(), cwd.to_string(), source.to_string()))
        }
        _ => None,
    }
}

/// Inspect the first complete JSONL records only.  Unsupported or incomplete transcripts
/// are rejected instead of guessed from their filename.
pub fn inspect(file: &Path, provider: &str) -> Result<Candidate, String> {
    if !matches!(provider, "codex" | "claude") {
        return Err("invalid-provider".into());
    }
    let canonical = fs::canonicalize(file).map_err(|e| e.to_string())?;
    let mut handle = File::open(&canonical).map_err(|e| e.to_string())?;
    let stat = handle.metadata().map_err(|e| e.to_string())?;
    if !stat.is_file() {
        return Err("not-regular-file".into());
    }
    let mut bytes = vec![0; (stat.len() as usize).min(MAX_LINE)];
    let read = handle.read(&mut bytes).map_err(|e| e.to_string())?;
    bytes.truncate(read);
    let complete = bytes
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|i| &bytes[..=i])
        .unwrap_or(&[]);
    let records: Vec<Value> = complete
        .split(|b| *b == b'\n')
        .filter_map(|line| {
            if line.is_empty() || line.len() > MAX_LINE {
                return None;
            }
            std::str::from_utf8(line)
                .ok()
                .and_then(|s| serde_json::from_str(s).ok())
        })
        .collect();
    let (source_session_id, cwd, source) =
        metadata(provider, &records).ok_or("unsupported-transcript")?;
    if !Path::new(&cwd).is_absolute() {
        return Err("unsupported-transcript".into());
    }
    Ok(Candidate {
        provider: provider.to_string(),
        source_session_id,
        cwd,
        file: canonical.to_string_lossy().into_owned(),
        updated_at: modified_ms(&stat),
        source,
        baseline: Baseline {
            identity: identity_for_file(&handle, &stat),
            size: stat.len(),
        },
    })
}

fn same_path(left: &str, right: &str) -> bool {
    super::same_directory(left, right)
}

/// Find the newest strict VS Code user transcripts.  Roots are the `sessions` and
/// `projects` directories themselves, allowing callers to choose their config directories.
pub fn discover(
    provider: Option<&str>,
    cwd: Option<&str>,
    codex_root: &Path,
    claude_root: &Path,
) -> Vec<Candidate> {
    let mut found = Vec::new();
    for (kind, root, depth) in [
        ("codex", codex_root, 3usize),
        ("claude", claude_root, 1usize),
    ] {
        if provider.is_some_and(|p| p != kind) {
            continue;
        }
        let mut budget = DISCOVERY_BUDGET;
        let mut files = Vec::new();
        visit(root, depth, &mut budget, &mut files);
        files.sort_by_key(|p| fs::metadata(p).ok().map(|m| modified_ms(&m)).unwrap_or(0));
        files.reverse();
        for path in files.into_iter().take(INSPECT_LIMIT) {
            let Ok(candidate) = inspect(&path, kind) else {
                continue;
            };
            if candidate.source == "vscode"
                && cwd
                    .map(|wanted| same_path(&candidate.cwd, wanted))
                    .unwrap_or(true)
            {
                found.push(candidate);
            }
        }
    }
    found.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
    found.truncate(RESULT_LIMIT);
    found
}

fn visit(dir: &Path, remaining: usize, budget: &mut usize, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    for entry in entries {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() && remaining > 0 {
            visit(&entry.path(), remaining - 1, budget, files);
        } else if kind.is_file() && entry.path().extension().is_some_and(|e| e == "jsonl") {
            files.push(entry.path());
        }
    }
}

pub struct TranscriptTail {
    file: PathBuf,
    handle: File,
    identity: FileIdentity,
    offset: u64,
    pending: Vec<u8>,
    discard: bool,
}

impl TranscriptTail {
    pub fn from_now(file: &Path) -> Result<Self, String> {
        Self::open(file, None)
    }

    /// This variant closes the inspect → tail race: a replacement or truncation between
    /// discovery and attachment is rejected before observations begin.
    pub fn from_candidate(candidate: &Candidate) -> Result<Self, String> {
        Self::open(Path::new(&candidate.file), Some(&candidate.baseline))
    }

    /// Bounded, complete metadata records that existed exactly at attachment.  This
    /// reads the same open file identity only up to `offset`, so records appended
    /// between tail creation and watcher registration are never swallowed by a
    /// baseline scan.
    pub fn usage_baseline_records(&self) -> Result<Vec<Value>, String> {
        let start = self.offset.saturating_sub(USAGE_BASELINE_BYTES);
        let mut handle = self.handle.try_clone().map_err(|e| e.to_string())?;
        handle
            .seek(SeekFrom::Start(start))
            .map_err(|e| e.to_string())?;
        let mut bytes = vec![0; (self.offset - start) as usize];
        handle.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        // A bounded read can begin in a giant JSONL record.  Never parse its tail.
        let body = if start > 0 {
            bytes
                .iter()
                .position(|b| *b == b'\n')
                .map(|i| &bytes[i + 1..])
                .unwrap_or(&[])
        } else {
            &bytes[..]
        };
        // The attachment EOF can likewise end in a record still being written.
        let complete = body
            .iter()
            .rposition(|b| *b == b'\n')
            .map(|i| &body[..=i])
            .unwrap_or(&[]);
        Ok(complete
            .split(|b| *b == b'\n')
            .filter_map(|line| {
                (line.len() <= MAX_LINE)
                    .then(|| serde_json::from_slice::<Value>(line).ok())
                    .flatten()
            })
            .collect())
    }

    pub fn usage_baseline_is_incomplete(&self) -> bool {
        self.offset > USAGE_BASELINE_BYTES || self.discard
    }

    fn open(file: &Path, baseline: Option<&Baseline>) -> Result<Self, String> {
        let canonical = fs::canonicalize(file).map_err(|e| e.to_string())?;
        let mut handle = File::open(&canonical).map_err(|e| e.to_string())?;
        let stat = handle.metadata().map_err(|e| e.to_string())?;
        if !stat.is_file() {
            return Err("not-regular-file".into());
        }
        let current_identity = identity_for_file(&handle, &stat);
        if baseline.is_some_and(|b| b.identity != current_identity || stat.len() < b.size) {
            return Err("transcript-replaced".into());
        }
        let mut last = [0u8; 1];
        if stat.len() > 0 {
            handle
                .seek(SeekFrom::Start(stat.len() - 1))
                .map_err(|e| e.to_string())?;
            handle.read_exact(&mut last).map_err(|e| e.to_string())?;
        }
        Ok(Self {
            file: canonical,
            handle,
            identity: current_identity,
            offset: stat.len(),
            pending: Vec::new(),
            discard: stat.len() > 0 && last[0] != b'\n',
        })
    }

    /// Return only complete, valid JSONL values added since the preceding call.  Rotation,
    /// truncation, invalid UTF-8, and oversized lines never produce an event.
    pub fn read(&mut self) -> Result<Vec<Value>, String> {
        let current = File::open(&self.file).map_err(|e| e.to_string())?;
        let stat = current.metadata().map_err(|e| e.to_string())?;
        if identity_for_file(&current, &stat) != self.identity || stat.len() < self.offset {
            return Err("transcript-replaced".into());
        }
        let end = stat.len().min(self.offset.saturating_add(MAX_TICK));
        let mut records = Vec::new();
        while self.offset < end {
            let size = ((end - self.offset) as usize).min(CHUNK);
            let mut chunk = vec![0; size];
            self.handle
                .seek(SeekFrom::Start(self.offset))
                .map_err(|e| e.to_string())?;
            let count = self.handle.read(&mut chunk).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            self.offset += count as u64;
            chunk.truncate(count);
            self.pending.extend_from_slice(&chunk);
            while let Some(newline) = self.pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=newline).collect();
                if !self.discard && line.len() - 1 <= MAX_LINE {
                    if let Ok(text) = std::str::from_utf8(&line[..line.len() - 1]) {
                        if let Ok(value) = serde_json::from_str(text) {
                            records.push(value);
                        }
                    }
                }
                self.discard = false;
            }
            if self.pending.len() > MAX_LINE {
                self.pending.clear();
                self.discard = true;
            }
        }
        Ok(records)
    }
}

pub struct EventFilter {
    provider: String,
    session_id: String,
    seen: VecDeque<String>,
    usage: UsageTracker,
}

impl EventFilter {
    pub fn new(provider: &str, session_id: &str) -> Self {
        Self {
            provider: provider.into(),
            session_id: session_id.into(),
            seen: VecDeque::new(),
            usage: UsageTracker::new(provider),
        }
    }

    pub fn with_baseline(
        provider: &str,
        session_id: &str,
        records: Vec<Value>,
        incomplete: bool,
    ) -> Self {
        let mut this = Self::new(provider, session_id);
        this.usage.seed(records, incomplete);
        this
    }

    /// Update the usage watermark for every new record.  A token snapshot is emitted
    /// only when the matching stop is observed, so tool activity never creates cost.
    pub fn take_with_tokens(
        &mut self,
        record: &Value,
    ) -> Option<(ObservedEventKind, Option<SessionEventTokens>)> {
        if let Some(delta) = self.usage.observe(record, &self.session_id) {
            self.usage.pending = Some(match self.usage.pending.take() {
                Some(total) => total.merged(delta),
                None => delta,
            });
        }
        self.take(record).map(|kind| {
            let tokens = (kind == ObservedEventKind::Stop)
                .then(|| self.usage.pending.take())
                .flatten();
            (kind, tokens)
        })
    }

    pub fn take(&mut self, record: &Value) -> Option<ObservedEventKind> {
        let (kind, key) = event_from_record(&self.provider, record, &self.session_id)?;
        if let Some(key) = key {
            // Transcript identifiers are provider-controlled input. Hashing keeps the
            // fixed 4,096-entry dedup window bounded even for malformed giant IDs.
            let mut hash = sha1_smol::Sha1::new();
            hash.update(key.as_bytes());
            let dedup = format!("{kind:?}:{}", hash.digest());
            if self.seen.contains(&dedup) {
                return None;
            }
            self.seen.push_back(dedup);
            if self.seen.len() > 4096 {
                self.seen.pop_front();
            }
        }
        Some(kind)
    }
}

#[derive(Default)]
struct UsageTracker {
    provider: String,
    codex: Option<CodexSnapshot>,
    codex_model: Option<String>,
    claude: HashMap<String, ClaudeSnapshot>,
    claude_unknown_is_baseline: bool,
    claude_new_turn_confirmed: bool,
    pending: Option<SessionEventTokens>,
}

#[derive(Clone, Default)]
struct CodexSnapshot {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    model: Option<String>,
}
#[derive(Clone, Default)]
struct ClaudeSnapshot {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    model: Option<String>,
}

impl UsageTracker {
    fn new(provider: &str) -> Self {
        Self {
            provider: provider.into(),
            ..Default::default()
        }
    }

    fn seed(&mut self, records: Vec<Value>, incomplete: bool) {
        if self.provider == "claude" && incomplete {
            // An old streaming ID may be outside this bounded scan.  Skip its first
            // post-attach snapshot instead of charging its entire historical reply.
            self.claude_unknown_is_baseline = true;
        }
        for record in records {
            let _ = self.observe(&record, "");
        }
        // Historic user rows are not an attachment-era boundary.
        self.claude_new_turn_confirmed = false;
    }

    fn observe(&mut self, record: &Value, session_id: &str) -> Option<SessionEventTokens> {
        match self.provider.as_str() {
            "codex" => self.observe_codex(record),
            "claude" => self.observe_claude(record, session_id),
            _ => None,
        }
    }

    fn observe_codex(&mut self, record: &Value) -> Option<SessionEventTokens> {
        let payload = record.get("payload")?;
        // Models are announced separately by Codex; retain the last safe identifier.
        let announced = payload
            .get("model")
            .and_then(Value::as_str)
            .filter(|m| !m.trim().is_empty())
            .map(str::to_owned);
        if record.get("type").and_then(Value::as_str) == Some("turn_context") {
            if let Some(model) = announced {
                self.codex_model = Some(model);
            }
            return None;
        }
        if payload.get("type").and_then(Value::as_str) != Some("token_count") {
            return None;
        }
        let total = payload.get("info")?.get("total_token_usage")?;
        let next = CodexSnapshot {
            input: total
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            output: total
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            cache_read: total
                .get("cached_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            cache_write: total
                .get("cache_write_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            model: announced
                .or_else(|| self.codex_model.clone())
                .or_else(|| self.codex.as_ref().and_then(|s| s.model.clone())),
        };
        let previous = self.codex.replace(next.clone())?;
        // Totals falling means rotation/reset.  Seed again and never turn history into a delta.
        if next.input < previous.input
            || next.output < previous.output
            || next.cache_read < previous.cache_read
            || next.cache_write < previous.cache_write
        {
            return None;
        }
        token_delta(&next, &previous)
    }

    fn observe_claude(&mut self, record: &Value, session_id: &str) -> Option<SessionEventTokens> {
        if record.get("type").and_then(Value::as_str) == Some("user")
            && record
                .get("sessionId")
                .and_then(Value::as_str)
                .is_some_and(|id| session_id.is_empty() || id == session_id)
            && !is_truthy(record.get("isSidechain"))
            && !is_truthy(record.get("isMeta"))
        {
            // A new user boundary proves that later assistant IDs began after attach.
            self.claude_new_turn_confirmed = true;
            return None;
        }
        if record.get("type")?.as_str()? != "assistant"
            || record
                .get("sessionId")
                .and_then(Value::as_str)
                .is_some_and(|id| !session_id.is_empty() && id != session_id)
            || is_truthy(record.get("isSidechain"))
            || is_truthy(record.get("isMeta"))
        {
            return None;
        }
        let message = record.get("message")?;
        let id = message.get("id")?.as_str()?.to_owned();
        let usage = message.get("usage")?;
        let next = ClaudeSnapshot {
            input: usage
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            output: usage
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            cache_read: usage
                .get("cache_read_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            cache_write: usage
                .get("cache_creation_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            model: message
                .get("model")
                .and_then(Value::as_str)
                .filter(|m| !m.trim().is_empty())
                .map(str::to_owned),
        };
        let previous = self.claude.get(&id).cloned();
        match previous {
            // Repeated streaming records are snapshots: only their increase counts.
            Some(previous)
                if next.input >= previous.input
                    && next.output >= previous.output
                    && next.cache_read >= previous.cache_read
                    && next.cache_write >= previous.cache_write =>
            {
                self.claude.insert(id, next.clone());
                token_delta(&next, &previous)
            }
            // A reduced/replayed snapshot must never move an ID watermark backwards.
            Some(_) => None,
            // A new post-attach message is a complete snapshot for that message.
            None if self.claude_unknown_is_baseline && !self.claude_new_turn_confirmed => {
                self.claude.insert(id, next);
                None
            }
            None => {
                self.claude.insert(id, next.clone());
                token_delta(&next, &ClaudeSnapshot::default())
            }
        }
    }
}

fn token_delta(
    next: &impl UsageSnapshot,
    previous: &impl UsageSnapshot,
) -> Option<SessionEventTokens> {
    let raw_input = next.input().saturating_sub(previous.input());
    let output = next.output().saturating_sub(previous.output());
    let cache_read = next.cache_read().saturating_sub(previous.cache_read());
    let cache_write = next.cache_write().saturating_sub(previous.cache_write());
    let input = if next.input_includes_cache() {
        raw_input
            .saturating_sub(cache_read)
            .saturating_sub(cache_write)
    } else {
        raw_input
    };
    let model = next.model();
    SessionEventTokens {
        input: Some(input),
        output: Some(output),
        cache_read: Some(cache_read),
        cache_write: Some(cache_write),
        model: model.clone(),
        by_model: model.map(|model| {
            vec![SessionModelTokens {
                input: Some(input),
                output: Some(output),
                cache_read: Some(cache_read),
                cache_write: Some(cache_write),
                model: Some(model),
            }]
        }),
    }
    .non_empty()
}
trait UsageSnapshot {
    fn input(&self) -> u64;
    fn output(&self) -> u64;
    fn cache_read(&self) -> u64;
    fn cache_write(&self) -> u64;
    fn model(&self) -> Option<String>;
    fn input_includes_cache(&self) -> bool;
}
macro_rules! usage_snapshot {
    ($t:ty) => {
        impl UsageSnapshot for $t {
            fn input(&self) -> u64 {
                self.input
            }
            fn output(&self) -> u64 {
                self.output
            }
            fn cache_read(&self) -> u64 {
                self.cache_read
            }
            fn cache_write(&self) -> u64 {
                self.cache_write
            }
            fn model(&self) -> Option<String> {
                self.model.clone()
            }
            fn input_includes_cache(&self) -> bool {
                false
            }
        }
    };
}
usage_snapshot!(ClaudeSnapshot);
impl UsageSnapshot for CodexSnapshot {
    fn input(&self) -> u64 {
        self.input
    }
    fn output(&self) -> u64 {
        self.output
    }
    fn cache_read(&self) -> u64 {
        self.cache_read
    }
    fn cache_write(&self) -> u64 {
        self.cache_write
    }
    fn model(&self) -> Option<String> {
        self.model.clone()
    }
    fn input_includes_cache(&self) -> bool {
        true
    }
}

fn event_from_record(
    provider: &str,
    r: &Value,
    session_id: &str,
) -> Option<(ObservedEventKind, Option<String>)> {
    if provider == "codex" {
        let payload = r.get("payload")?;
        if text(r, "type") == Some("event_msg") && text(payload, "type") == Some("task_started") {
            return Some((
                ObservedEventKind::Prompt,
                text(payload, "turn_id").map(str::to_string),
            ));
        }
        if text(r, "type") == Some("event_msg") && text(payload, "type") == Some("task_complete") {
            return Some((
                ObservedEventKind::Stop,
                text(payload, "turn_id").map(str::to_string),
            ));
        }
        if text(r, "type") == Some("response_item")
            && matches!(
                text(payload, "type"),
                Some("function_call") | Some("custom_tool_call")
            )
        {
            return Some((
                ObservedEventKind::Tool,
                text(payload, "call_id").map(str::to_string),
            ));
        }
        return None;
    }
    if provider != "claude"
        || text(r, "sessionId") != Some(session_id)
        || is_truthy(r.get("isSidechain"))
        || is_truthy(r.get("isMeta"))
    {
        return None;
    }
    let message = r.get("message")?;
    if text(r, "type") == Some("user") {
        let content = message.get("content")?;
        let has_text = content.is_string()
            || content.as_array().is_some_and(|a| {
                a.iter().any(|v| text(v, "type") == Some("text"))
                    && !a.iter().any(|v| text(v, "type") == Some("tool_result"))
            });
        if text(r, "promptId").is_some() && text(message, "role") == Some("user") && has_text {
            return Some((
                ObservedEventKind::Prompt,
                text(r, "promptId").map(str::to_string),
            ));
        }
    }
    if text(r, "type") == Some("assistant") {
        if text(message, "stop_reason") == Some("end_turn") {
            return Some((
                ObservedEventKind::Stop,
                text(message, "id")
                    .or_else(|| text(r, "uuid"))
                    .map(str::to_string),
            ));
        }
        if message
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|a| a.iter().any(|v| text(v, "type") == Some("tool_use")))
        {
            return Some((
                ObservedEventKind::Tool,
                text(message, "id")
                    .or_else(|| text(r, "uuid"))
                    .map(str::to_string),
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(path: &Path, text: &str) {
        fs::write(path, text).unwrap();
    }

    fn write_jsonl(path: &Path, records: &[Value]) {
        let body = records
            .iter()
            .map(|v| format!("{}\n", serde_json::to_string(v).unwrap()))
            .collect::<String>();
        write(path, &body);
    }

    #[test]
    fn discovers_only_vscode_user_transcripts() {
        let temp = tempfile::tempdir().unwrap();
        let codex = temp.path().join("sessions");
        let claude = temp.path().join("projects");
        fs::create_dir_all(codex.join("2026/09/12")).unwrap();
        fs::create_dir_all(&claude).unwrap();
        let cwd = temp.path().join("work");
        fs::create_dir(&cwd).unwrap();
        write_jsonl(
            &codex.join("2026/09/12/ok.jsonl"),
            &[serde_json::json!({
                "type": "session_meta", "payload": {"id": "c1", "cwd": cwd, "source": "vscode", "originator": "codex_vscode", "thread_source": "user"}
            })],
        );
        write_jsonl(
            &claude.join("sub.jsonl"),
            &[serde_json::json!({
                "sessionId": "x", "cwd": cwd, "entrypoint": "claude-vscode", "isSidechain": true
            })],
        );
        let got = discover(None, Some(cwd.to_str().unwrap()), &codex, &claude);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source_session_id, "c1");
        assert_eq!(got[0].source, "vscode");
    }

    #[test]
    fn tail_ignores_partial_and_deduplicates_events() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("t.jsonl");
        write(&file, "partial");
        let mut tail = TranscriptTail::from_now(&file).unwrap();
        let mut out = fs::OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(
            out,
            r#"{{"type":"event_msg","payload":{{"type":"task_started","turn_id":"1"}}}}"#
        )
        .unwrap();
        out.flush().unwrap();
        let records = tail.read().unwrap();
        assert_eq!(records.len(), 0);
        writeln!(
            out,
            r#"{{"type":"event_msg","payload":{{"type":"task_complete","turn_id":"2"}}}}"#
        )
        .unwrap();
        out.flush().unwrap();
        let mut filter = EventFilter::new("codex", "unused");
        let events: Vec<_> = tail
            .read()
            .unwrap()
            .iter()
            .filter_map(|r| filter.take(r))
            .collect();
        assert_eq!(events, vec![ObservedEventKind::Stop]);
    }

    #[test]
    fn usage_baseline_is_pinned_to_tail_offset_and_ignores_partial_eof() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("usage.jsonl");
        write(&file, "{\"n\":1}\n{\"partial\"");
        let tail = TranscriptTail::from_now(&file).unwrap();
        let mut out = fs::OpenOptions::new().append(true).open(&file).unwrap();
        out.write_all(b":2}\n{\"n\":3}\n").unwrap();
        out.flush().unwrap();
        let records = tail.usage_baseline_records().unwrap();
        assert_eq!(records, vec![serde_json::json!({"n": 1})]);
    }

    #[test]
    fn partial_claude_eof_marks_baseline_incomplete_and_skips_same_id_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("claude-partial.jsonl");
        write(
            &file,
            r#"{"type":"assistant","sessionId":"s1","message":{"id":"m1""#,
        );
        let mut tail = TranscriptTail::from_now(&file).unwrap();
        assert!(tail.usage_baseline_is_incomplete());
        let mut filter = EventFilter::with_baseline(
            "claude",
            "s1",
            tail.usage_baseline_records().unwrap(),
            tail.usage_baseline_is_incomplete(),
        );
        let mut out = fs::OpenOptions::new().append(true).open(&file).unwrap();
        // Finish the pre-attach partial line, then write a complete repeated snapshot.
        writeln!(out, r#"}}"#).unwrap();
        writeln!(out, r#"{{"type":"assistant","sessionId":"s1","message":{{"id":"m1","model":"claude-sonnet-4","usage":{{"input_tokens":50,"output_tokens":5,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}},"stop_reason":"end_turn"}}}}"#).unwrap();
        out.flush().unwrap();
        let records = tail.read().unwrap();
        assert_eq!(records.len(), 1, "partial pre-attach line is discarded");
        let (_, usage) = filter.take_with_tokens(&records[0]).unwrap();
        assert!(usage.is_none(), "same ID may contain pre-attach usage");
    }

    #[test]
    fn claude_filter_accepts_activity_and_rejects_non_user_records() {
        let mut filter = EventFilter::new("claude", "s1");
        let cases = [
            (
                serde_json::json!({"type":"user", "sessionId":"s1", "promptId":"p1", "message":{"role":"user", "content":"go"}}),
                Some(ObservedEventKind::Prompt),
            ),
            (
                serde_json::json!({"type":"assistant", "sessionId":"s1", "message":{"id":"a1", "content":[{"type":"tool_use"}]}}),
                Some(ObservedEventKind::Tool),
            ),
            (
                serde_json::json!({"type":"assistant", "sessionId":"s1", "message":{"id":"a2", "stop_reason":"end_turn"}}),
                Some(ObservedEventKind::Stop),
            ),
            (
                serde_json::json!({"type":"user", "sessionId":"other", "promptId":"p2", "message":{"role":"user", "content":"ignored"}}),
                None,
            ),
            (
                serde_json::json!({"type":"user", "sessionId":"s1", "promptId":"p3", "message":{"role":"user", "content":[{"type":"text"}, {"type":"tool_result"}]}}),
                None,
            ),
            (
                serde_json::json!({"type":"assistant", "sessionId":"s1", "isSidechain":true, "message":{"id":"a3", "stop_reason":"end_turn"}}),
                None,
            ),
            (
                serde_json::json!({"type":"assistant", "sessionId":"s1", "isMeta":true, "message":{"id":"a4", "stop_reason":"end_turn"}}),
                None,
            ),
        ];
        for (record, expected) in cases {
            assert_eq!(filter.take(&record), expected);
        }
    }

    #[test]
    fn codex_filter_deduplicates_and_does_not_turn_abort_into_stop() {
        let mut filter = EventFilter::new("codex", "unused");
        let prompt = serde_json::json!({"type":"event_msg", "payload":{"type":"task_started", "turn_id":"same"}});
        let abort = serde_json::json!({"type":"event_msg", "payload":{"type":"task_aborted", "turn_id":"same"}});
        assert_eq!(filter.take(&prompt), Some(ObservedEventKind::Prompt));
        assert_eq!(filter.take(&prompt), None);
        assert_eq!(filter.take(&abort), None);
    }

    #[test]
    fn dedup_does_not_retain_a_giant_provider_identifier() {
        let mut filter = EventFilter::new("codex", "unused");
        let record = serde_json::json!({"type":"response_item", "payload":{"type":"function_call", "call_id":"x".repeat(1_000_000)}});
        assert_eq!(filter.take(&record), Some(ObservedEventKind::Tool));
        assert!(filter.seen.front().unwrap().len() < 64);
    }

    #[test]
    fn codex_usage_uses_attach_baseline_and_handles_repeat_reset_and_model_change() {
        let mut filter = EventFilter::new("codex", "s");
        let total = |input, output, model: &str| serde_json::json!({"type":"event_msg","payload":{"type":"token_count","model":model,"info":{"total_token_usage":{"input_tokens":input,"output_tokens":output,"cached_input_tokens":0,"cache_write_input_tokens":0}}}});
        // First observed cumulative value is a boundary, not historical cost.
        assert_eq!(filter.take_with_tokens(&total(100, 10, "gpt-5")), None);
        assert_eq!(filter.take_with_tokens(&total(100, 10, "gpt-5")), None);
        assert_eq!(filter.take_with_tokens(&total(130, 15, "gpt-5.1")), None);
        let stop = serde_json::json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"t1"}});
        let (_, usage) = filter.take_with_tokens(&stop).unwrap();
        let usage = usage.unwrap();
        assert_eq!(usage.input, Some(30));
        assert_eq!(usage.model.as_deref(), Some("gpt-5.1"));
        assert_eq!(usage.by_model.unwrap()[0].model.as_deref(), Some("gpt-5.1"));
        // A counter reset establishes another boundary and never emits a giant delta.
        assert_eq!(filter.take_with_tokens(&total(1, 1, "gpt-5.1")), None);
        let stop2 = serde_json::json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"t2"}});
        assert!(filter.take_with_tokens(&stop2).unwrap().1.is_none());
    }

    #[test]
    fn codex_input_delta_excludes_cache_tokens_like_the_native_observer() {
        let mut filter = EventFilter::new("codex", "s");
        let total = |input, cached| serde_json::json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":input,"output_tokens":0,"cached_input_tokens":cached,"cache_write_input_tokens":0}}}});
        let _ = filter.take_with_tokens(&total(100, 80));
        let _ = filter.take_with_tokens(&total(200, 160));
        let stop = serde_json::json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"cached"}});
        let usage = filter.take_with_tokens(&stop).unwrap().1.unwrap();
        assert_eq!(usage.input, Some(20));
        assert_eq!(usage.cache_read, Some(80));
    }

    #[test]
    fn claude_usage_replaces_streaming_message_snapshots_and_counts_only_post_attach_delta() {
        let mut filter = EventFilter::new("claude", "s1");
        let assistant = |id: &str, input, output, stop| serde_json::json!({"type":"assistant","sessionId":"s1","message":{"id":id,"model":"claude-sonnet-4","usage":{"input_tokens":input,"output_tokens":output,"cache_read_input_tokens":0,"cache_creation_input_tokens":0},"stop_reason": if stop { "end_turn" } else { "" }}});
        // Simulate connecting mid-response: an existing snapshot is the baseline.
        let old = assistant("m1", 10, 2, false);
        let _ = filter.usage.observe(&old, "s1");
        let newer = assistant("m1", 13, 4, true);
        let (_, usage) = filter.take_with_tokens(&newer).unwrap();
        let usage = usage.unwrap();
        assert_eq!(usage.input, Some(3));
        assert_eq!(usage.output, Some(2));
        // Same completed ID is deduplicated, so it cannot create a second turn.
        assert!(filter.take_with_tokens(&newer).is_none());
    }

    #[test]
    fn incomplete_claude_baseline_starts_counting_new_ids_after_a_prompt_boundary() {
        let mut filter = EventFilter::new("claude", "s1");
        filter.usage.claude_unknown_is_baseline = true;
        let prompt = serde_json::json!({"type":"user","sessionId":"s1","message":{"role":"user","content":"new"}});
        assert!(filter.take_with_tokens(&prompt).is_none());
        let assistant = serde_json::json!({"type":"assistant","sessionId":"s1","message":{"id":"new","model":"claude-sonnet-4","usage":{"input_tokens":7,"output_tokens":3,"cache_read_input_tokens":0,"cache_creation_input_tokens":0},"stop_reason":"end_turn"}});
        let usage = filter.take_with_tokens(&assistant).unwrap().1.unwrap();
        assert_eq!(usage.input, Some(7));
    }

    #[test]
    fn claude_replayed_smaller_snapshot_never_reopens_an_old_message_watermark() {
        let mut filter = EventFilter::new("claude", "s1");
        let assistant = |id: &str, output| serde_json::json!({"type":"assistant","sessionId":"s1","message":{"id":id,"model":"claude-sonnet-4","usage":{"input_tokens":0,"output_tokens":output,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}});
        // m1 is already known at its largest snapshot. A replayed smaller row then
        // the same maximum must not leak an extra delta into the next Stop.
        let _ = filter.usage.observe(&assistant("m1", 10), "s1");
        let _ = filter.take_with_tokens(&assistant("m1", 5));
        let _ = filter.take_with_tokens(&assistant("m1", 10));
        let completed = serde_json::json!({"type":"assistant","sessionId":"s1","message":{"id":"m2","model":"claude-sonnet-4","usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0},"stop_reason":"end_turn"}});
        let usage = filter.take_with_tokens(&completed).unwrap().1.unwrap();
        assert_eq!(usage.output, Some(1));
    }

    #[test]
    fn tail_keeps_split_utf8_and_partial_line_until_complete() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("split.jsonl");
        write(&file, "");
        let mut tail = TranscriptTail::from_now(&file).unwrap();
        let mut out = fs::OpenOptions::new().append(true).open(&file).unwrap();
        out.write_all(b"{\"value\":\"").unwrap();
        out.write_all("한".as_bytes()).unwrap();
        out.flush().unwrap();
        assert!(tail.read().unwrap().is_empty());
        out.write_all("글\"}\n".as_bytes()).unwrap();
        out.flush().unwrap();
        assert_eq!(
            tail.read().unwrap(),
            vec![serde_json::json!({"value":"한글"})]
        );
    }

    #[test]
    fn oversized_line_is_discarded_and_next_complete_record_recovers() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("large.jsonl");
        write(&file, "");
        let mut tail = TranscriptTail::from_now(&file).unwrap();
        let mut out = fs::OpenOptions::new().append(true).open(&file).unwrap();
        out.write_all(&vec![b'x'; MAX_LINE + 1]).unwrap();
        out.write_all(b"\n{\"ok\":true}\n").unwrap();
        out.flush().unwrap();
        assert_eq!(tail.read().unwrap(), vec![serde_json::json!({"ok":true})]);
    }

    #[test]
    fn tail_reads_at_most_four_mebibytes_per_tick() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("bounded.jsonl");
        write(&file, "");
        let mut tail = TranscriptTail::from_now(&file).unwrap();
        let mut out = fs::OpenOptions::new().append(true).open(&file).unwrap();
        let line = format!("{{\"v\":\"{}\"}}\n", "x".repeat(8_000));
        for _ in 0..700 {
            out.write_all(line.as_bytes()).unwrap();
        }
        out.flush().unwrap();
        let first = tail.read().unwrap();
        assert!(!first.is_empty());
        assert!(tail.offset <= MAX_TICK);
        let second = tail.read().unwrap();
        assert!(!second.is_empty());
    }

    #[test]
    fn replacement_and_truncation_end_observation() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("session.jsonl");
        let cwd = temp.path().join("work");
        fs::create_dir(&cwd).unwrap();
        let record = serde_json::json!({"type":"session_meta", "payload":{"id":"one", "cwd":cwd, "source":"vscode", "originator":"codex_vscode", "thread_source":"user"}});
        write_jsonl(&file, &[record]);
        let candidate = inspect(&file, "codex").unwrap();
        let replacement = temp.path().join("replacement.jsonl");
        write_jsonl(
            &replacement,
            &[
                serde_json::json!({"type":"session_meta", "payload":{"id":"two", "cwd":cwd, "source":"vscode", "originator":"codex_vscode", "thread_source":"user"}}),
            ],
        );
        fs::remove_file(&file).unwrap();
        fs::rename(&replacement, &file).unwrap();
        assert!(
            matches!(TranscriptTail::from_candidate(&candidate), Err(ref e) if e == "transcript-replaced")
        );
        let mut tail = TranscriptTail::from_now(&file).unwrap();
        write(&file, "");
        assert_eq!(tail.read().unwrap_err(), "transcript-replaced");
    }

    #[test]
    fn inspect_rejects_empty_source_session_id() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("empty.jsonl");
        let cwd = temp.path().join("work");
        fs::create_dir(&cwd).unwrap();
        write_jsonl(
            &file,
            &[
                serde_json::json!({"type":"session_meta", "payload":{"id":"", "cwd":cwd, "source":"vscode", "originator":"codex_vscode", "thread_source":"user"}}),
            ],
        );
        assert_eq!(
            inspect(&file, "codex").unwrap_err(),
            "unsupported-transcript"
        );
    }
}
