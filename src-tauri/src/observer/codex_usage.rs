// Codex rollout의 모델별 증분 사용량. UserPromptSubmit에서 첫 기준을 잡고
// PostToolUse(5초 스로틀)와 Stop에서 새 token_count 행만 순서대로 읽는다.
// 메인·서브 rollout마다 위치/누계/모델을 보존하며 세션별 잠금으로 중복을 막는다.
// 도중 합류는 과거 누계를 생략한다. 파일 전체를 확인한 신선한 세션만 기준 0.
// 비용 계산용 by_model과 표시용 전체 토큰을 함께 내보낸다.
// 정본: docs/session-analytics-design.md §9.3·§11.10.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;

use crate::types::{SessionEventTokens, SessionModelTokens};

use super::event::{hook_cwd, native_session_id};

/// rollout 꼬리에서 마지막 `token_count`를 찾을 때 읽는 최대 바이트. 도구
/// 출력이 큰 턴을 감안해 넉넉히 잡되, 이 안에서 못 찾으면 포기한다.
const ROLLOUT_TAIL_BYTES: u64 = 1024 * 1024;

/// 최초 snapshot의 context가 꼬리 밖에 있을 때만 쓰는 모델 폴백 범위.
/// 이후 변경은 cursor가 turn_context 행을 읽어 반영한다.
const ROLLOUT_HEAD_BYTES: u64 = 256 * 1024;

/// cwd 폴백에서 "살아 있는" 파일로 볼 최대 무수정 시간.
const LIVE_WINDOW: Duration = Duration::from_secs(30 * 60);

/// 턴 시작 시각과 파일 시각(mtime, `session_meta.timestamp`)을 비교할 때 주는
/// 여유. 두 시각은 출처가 달라(우리 시계 vs 파일시스템/Codex 기록) 경계에서
/// 몇백 ms가 엇갈릴 수 있는데, 그 때문에 이번 턴에 생긴 서브스레드를 통째로
/// 놓치는 쪽이 몇 초 이전 것을 포함하는 쪽보다 나쁘다.
const TIMESTAMP_SLACK: Duration = Duration::from_secs(2);

/// rollout `info.total_token_usage` 누계 스냅샷.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Cumulative {
    input: u64,
    cached_input: u64,
    cache_write: u64,
    output: u64,
}

impl Cumulative {
    /// `self - previous`. 각 항목은 음수로 내려가지 않게 saturating.
    fn delta(self, previous: Self) -> Self {
        Self {
            input: self.input.saturating_sub(previous.input),
            cached_input: self.cached_input.saturating_sub(previous.cached_input),
            cache_write: self.cache_write.saturating_sub(previous.cache_write),
            output: self.output.saturating_sub(previous.output),
        }
    }

    /// `self + other`(메인 스레드 몫 + 서브스레드 몫 합산용).
    fn plus(self, other: Self) -> Self {
        Self {
            input: self.input.saturating_add(other.input),
            cached_input: self.cached_input.saturating_add(other.cached_input),
            cache_write: self.cache_write.saturating_add(other.cache_write),
            output: self.output.saturating_add(other.output),
        }
    }
}

/// rollout 한 줄 사이의 누계 증가분과, 그 줄 앞에 선언된 모델.
#[derive(Debug, Clone)]
struct ModelDelta {
    cumulative: Cumulative,
    model: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ThreadCursor {
    offset: u64,
    cumulative: Cumulative,
    model: Option<String>,
}

#[derive(Default)]
struct UsageState {
    main: Option<ThreadCursor>,
    started: Option<SystemTime>,
    sub: HashMap<PathBuf, ThreadCursor>,
    last_progress: Option<Instant>,
    had_partial: bool,
    finalized: bool,
}

/// 세션별 턴 경계 누계를 들고 있는 추출기. `CodexAdapter`가 소유한다.
pub struct CodexUsageTracker {
    /// `<CODEX_HOME>/sessions`. 부재(환경 미해석)면 추출을 통째로 포기한다.
    sessions_root: Option<PathBuf>,
    /// 같은 native 세션의 prompt/tool/stop 훅은 파일 읽기부터 워터마크 갱신까지
    /// 한 락으로 직렬화한다. forwarder 재시도와 Stop의 경합도 여기서 막는다.
    states: Mutex<HashMap<String, Arc<Mutex<UsageState>>>>,
    /// 키 → (rollout 경로, 앞머리에서 읽은 모델). 매 턴 디렉터리를 다시 훑지
    /// 않기 위한 캐시. 경로가 사라지면 다음 조회에서 자연히 다시 찾는다.
    located: Mutex<HashMap<String, Located>>,
    /// rollout 경로 → 첫 줄(`session_meta`)에서 읽은 스레드 신원. 첫 줄은 절대
    /// 안 바뀌므로 영구 캐시다(부모 추적에 매 Stop마다 모든 rollout의 첫 줄이
    /// 필요한데, 그때마다 다시 읽지 않기 위해). `None`은 "첫 줄이 session_meta가
    /// 아님"을 캐시한 것.
    meta_cache: Mutex<HashMap<PathBuf, Option<ThreadMeta>>>,
}

/// 찾아낸 메인 rollout과 거기서 뽑은 부수 정보.
#[derive(Debug, Clone)]
struct Located {
    path: PathBuf,
    /// 파일 앞머리에서 읽은 대표 모델(꼬리에서 찾으면 그쪽이 우선한다).
    model: Option<String>,
    /// 이 스레드의 id. 서브스레드(`parent_thread_id`)를 이 값으로 찾는다.
    thread_id: Option<String>,
}

/// rollout 첫 줄(`session_meta`)에서 뽑은 스레드 신원.
#[derive(Debug, Clone, Default)]
struct ThreadMeta {
    /// 이 스레드의 id(`payload.id`).
    id: Option<String>,
    /// 부모 스레드 id(`payload.parent_thread_id`). 있으면 서브스레드다
    /// (서브에이전트·guardian_review 등 — `thread_source` 값은 종류마다 다르다).
    parent: Option<String>,
    /// 작업 디렉터리(`payload.cwd`).
    cwd: Option<String>,
    /// 스레드 시작 시각(줄의 `timestamp`).
    started_at: Option<SystemTime>,
}

impl CodexUsageTracker {
    pub fn new(sessions_root: Option<PathBuf>) -> Self {
        Self {
            sessions_root,
            states: Mutex::new(HashMap::new()),
            located: Mutex::new(HashMap::new()),
            meta_cache: Mutex::new(HashMap::new()),
        }
    }

    /// 프로덕션 생성자: `CODEX_HOME`(또는 `~/.codex`)/sessions.
    pub fn from_env() -> Self {
        Self::new(crate::agent_paths::codex_home_from_env().map(|home| home.join("sessions")))
    }

    /// 턴 시작(UserPromptSubmit) — 기준 누계가 **없을 때만** 기록한다.
    /// 한 턴에 프롬프트가 여러 번 제출돼도(작업 중 추가 지시) 턴을 여는 첫
    /// 프롬프트의 기준을 유지해야 그 사이에 쓴 토큰이 새지 않는다.
    pub fn mark_turn_start(&self, body: &[u8]) {
        let Some(key) = self.key_for(body) else {
            return;
        };
        let Some(located) = self.locate(body, &key) else {
            return;
        };
        let lock = self.state_lock(&key);
        let mut state = lock.lock().unwrap();
        if state.main.is_some() {
            if state.finalized {
                // 이전 Stop 뒤 늦게 flush된 행도 다음 usage에서 cursor로 읽어야
                // 하므로 baseline/cursor는 그대로 둔다.
                // 서브스레드 발견 기준은 최초 관측 시각을 유지한다. 앞선 턴에
                // 시작했지만 늦게 파일이 완성된 서브도 처음부터 회수해야 한다.
                state.had_partial = false;
                state.finalized = false;
                state.last_progress = None;
            }
            return;
        }
        let current = match read_tail_snapshot(&located.path) {
            Some((cumulative, model, offset)) => ThreadCursor {
                offset,
                cumulative,
                model: model.or_else(|| located.model.clone()),
            },
            None => return,
        };
        state.main = Some(current);
        state.started = Some(SystemTime::now());
        state.had_partial = false;
        state.finalized = false;
        state.last_progress = None;
    }

    /// 턴 경계(기준 누계 + 그 시각)를 함께 심는다. 시각은 서브스레드 rollout이
    /// "이번 턴에 새로 생긴 것"인지 판정하는 데 쓴다.
    /// 턴 종료(Stop) — 기준 대비 델타를 돌려주고, 현재 누계를 새 기준으로
    /// 갱신한다. 기준이 없으면(앱 재시작 등) 이번 턴은 생략하되 기준은 심어
    /// 다음 턴부터 정상 집계되게 한다.
    pub fn turn_usage(&self, body: &[u8]) -> Option<SessionEventTokens> {
        self.usage(body, false)
    }

    /// PostToolUse용 5초 제한 증분. 스로틀을 통과한 경우에도 cursor는 실제
    /// append를 읽을 때만 전진하므로, 늦게 flush된 서브스레드를 잃지 않는다.
    pub fn progress_usage(&self, body: &[u8]) -> Option<SessionEventTokens> {
        self.usage(body, true)
    }

    fn usage(&self, body: &[u8], progress: bool) -> Option<SessionEventTokens> {
        let key = self.key_for(body)?;
        let located = self.locate(body, &key)?;
        let lock = self.state_lock(&key);
        let mut state = lock.lock().unwrap();
        if progress {
            if state
                .last_progress
                .is_some_and(|at| at.elapsed() < Duration::from_secs(5))
            {
                return None;
            }
            state.last_progress = Some(Instant::now());
        }
        // attach 중 Stop: 누계를 기준으로만 심고 이 턴은 안전하게 생략한다.
        if state.main.is_none() {
            self.seed_main(&mut state, &located);
            // attach의 첫 Stop은 과거 턴을 생략한 경계다. 다음 prompt가 새 turn
            // 시작 시각을 반드시 다시 심도록 완료 상태로 둔다.
            state.finalized = !progress;
            return None;
        }
        // Stop 재시도는 이미 완료된 논리 턴을 새 turn으로 만들면 안 된다. 늦게
        // flush된 행은 cursor를 건드리지 않고 남겨 다음 prompt/tool 관측에서만
        // 증분으로 회수한다.
        if !progress && state.finalized {
            return None;
        }
        let mut deltas = read_thread_deltas(&located.path, state.main.as_mut().unwrap());
        if let (Some(thread), Some(started)) = (located.thread_id.as_deref(), state.started) {
            deltas.extend(self.subthread_deltas(&mut state, thread, started));
        }
        if let Some(mut tokens) = tokens_from_deltas(deltas) {
            // 툴팁의 대표 모델은 메인 세션이며 실제 비용 귀속은 by_model을 쓴다.
            tokens.model = state
                .main
                .as_ref()
                .and_then(|cursor| cursor.model.clone())
                .or(tokens.model);
            if progress {
                state.had_partial = true;
            } else {
                state.finalized = true;
            }
            return Some(tokens);
        }
        // PostToolUse에서 이미 보낸 부분 사용량 뒤 Stop이 오면, 0도 한 번 보내
        // 최종 턴을 확정한다. 아무 사용량 없는 Stop은 기존처럼 생략한다.
        if !progress && state.had_partial && !state.finalized {
            state.finalized = true;
            return Some(zero_tokens(
                state.main.as_ref().and_then(|c| c.model.clone()),
            ));
        }
        if !progress {
            state.finalized = true;
        }
        None
    }

    fn state_lock(&self, key: &str) -> Arc<Mutex<UsageState>> {
        self.states
            .lock()
            .unwrap()
            .entry(key.into())
            .or_default()
            .clone()
    }

    fn seed_main(&self, state: &mut UsageState, located: &Located) {
        let Some((cumulative, model, offset)) = read_tail_snapshot(&located.path) else {
            return;
        };
        state.main = Some(ThreadCursor {
            offset,
            cumulative,
            model: model.or_else(|| located.model.clone()),
        });
        state.started = Some(SystemTime::now());
    }

    /// 이 세션을 식별하는 캐시 키. 훅 body의 native session_id 우선, 없으면 cwd.
    fn key_for(&self, body: &[u8]) -> Option<String> {
        self.sessions_root.as_ref()?;
        native_session_id(body).or_else(|| hook_cwd(body))
    }

    /// 이 세션의 rollout 경로(+앞머리 모델)를 찾는다. 캐시된 경로가 아직
    /// 존재하면 그대로 쓴다.
    fn locate(&self, body: &[u8], key: &str) -> Option<Located> {
        {
            let mut located = self.located.lock().unwrap();
            if let Some(entry) = located.get_mut(key) {
                if entry.path.exists() {
                    if entry.thread_id.is_none() {
                        entry.thread_id = self.head_meta(&entry.path).and_then(|meta| meta.id);
                    }
                    if entry.model.is_none() {
                        entry.model = read_head_model(&entry.path);
                    }
                    return Some(entry.clone());
                }
            }
        }
        let root = self.sessions_root.as_ref()?;
        let path = native_session_id(body)
            .and_then(|id| find_by_session_id(root, &id))
            .or_else(|| hook_cwd(body).and_then(|cwd| find_by_cwd(root, &cwd)))?;
        let entry = Located {
            model: read_head_model(&path),
            thread_id: self.head_meta(&path).and_then(|meta| meta.id),
            path,
        };
        self.located
            .lock()
            .unwrap()
            .insert(key.to_string(), entry.clone());
        Some(entry)
    }

    /// 이 세션의 **서브스레드 rollout들**이 이번 턴에 쓴 몫.
    ///
    /// Codex는 서브에이전트·guardian_review를 별도 스레드로 돌리고 각자의
    /// rollout에 기록하는데, `info.total_token_usage`는 **스레드별 누계**라
    /// 메인 rollout만 보면 그 몫이 어디에도 안 잡힌다(실측 누락 비중 약 39%).
    /// `find_by_cwd`가 서브스레드를 배제하는 규칙(메인 rollout 식별용)은 그대로
    /// 두고, 합산 단계인 여기서만 되불러온다.
    ///
    /// 세션 최초 관측 이후 시작된 서브는 처음부터 읽고 오래된 서브는 기준만
    /// 심는다. 발견 뒤에는 파일별 cursor를 유지해 다음 관측에서 이어 읽는다.
    fn subthread_deltas(
        &self,
        state: &mut UsageState,
        main_thread: &str,
        turn_started: SystemTime,
    ) -> Vec<ModelDelta> {
        let Some(root) = self.sessions_root.as_ref() else {
            return vec![];
        };
        let files = rollout_files(root);

        // 부모 사슬을 따라 올라가려면 id로도 찾을 수 있어야 한다(서브의 서브).
        let mut by_id: HashMap<String, ThreadMeta> = HashMap::new();
        for path in &files {
            if let Some(meta) = self.head_meta(path) {
                if let Some(id) = meta.id.clone() {
                    by_id.insert(id, meta);
                }
            }
        }

        let mut total = vec![];
        for path in files {
            let Some(meta) = self.head_meta(&path) else {
                continue;
            };
            if !descends_from(&meta, main_thread, &by_id) {
                continue;
            }
            match state.sub.get_mut(&path) {
                Some(cursor) => total.extend(read_thread_deltas(&path, cursor)),
                // 처음 보는 서브: 이 턴 안에서 시작된 스레드면 누계 전체가 이 턴
                // 몫이다. mtime은 늦은 flush에서 거짓말할 수 있어 시작 시각만 쓴다.
                None if meta
                    .started_at
                    .is_some_and(|at| at + TIMESTAMP_SLACK >= turn_started) =>
                {
                    let mut cursor = ThreadCursor::default();
                    total.extend(read_thread_deltas(&path, &mut cursor));
                    state.sub.insert(path, cursor);
                }
                None => {
                    // attach 이전부터 있던 서브는 기준만 심는다.
                    if let Some((cumulative, model, offset)) = read_tail_snapshot(&path) {
                        state.sub.insert(
                            path.clone(),
                            ThreadCursor {
                                offset,
                                cumulative,
                                model,
                            },
                        );
                    }
                }
            }
        }
        total
    }

    /// rollout 첫 줄의 스레드 신원(캐시). 첫 줄은 변하지 않으므로 한 번만 읽는다.
    fn head_meta(&self, path: &Path) -> Option<ThreadMeta> {
        if let Some(cached) = self.meta_cache.lock().unwrap().get(path) {
            return cached.clone();
        }
        let meta = read_head_meta(path);
        if meta.is_some() {
            self.meta_cache
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), meta.clone());
        }
        meta
    }
}

/// `meta`의 부모 사슬이 `main_thread`에 닿는가(서브의 서브까지). 상한은 순환
/// 방어용이다 — 실측 깊이는 1이다.
fn descends_from(
    meta: &ThreadMeta,
    main_thread: &str,
    by_id: &HashMap<String, ThreadMeta>,
) -> bool {
    let mut parent = meta.parent.clone();
    for _ in 0..8 {
        let Some(id) = parent else { return false };
        if id == main_thread {
            return true;
        }
        parent = by_id.get(&id).and_then(|meta| meta.parent.clone());
    }
    false
}

/// `sessions/YYYY/MM/DD` 아래 모든 rollout 파일 경로(순서 무관).
fn rollout_files(sessions: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(years) = std::fs::read_dir(sessions) else {
        return files;
    };
    for year in years.flatten() {
        let Ok(months) = std::fs::read_dir(year.path()) else {
            continue;
        };
        for month in months.flatten() {
            let Ok(days) = std::fs::read_dir(month.path()) else {
                continue;
            };
            for day in days.flatten() {
                let Ok(entries) = std::fs::read_dir(day.path()) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let Some(name) = name.to_str() else { continue };
                    if name.starts_with("rollout-") && name.ends_with(".jsonl") {
                        files.push(entry.path());
                    }
                }
            }
        }
    }
    files
}

/// 파일명이 `-<session_id>.jsonl`로 끝나는 rollout.
fn find_by_session_id(sessions: &Path, session_id: &str) -> Option<PathBuf> {
    let suffix = format!("-{session_id}.jsonl");
    rollout_files(sessions).into_iter().find(|path| {
        path.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| name.ends_with(&suffix))
    })
}

/// 첫 줄 `session_meta.payload.cwd`가 같고 최근에 쓰인 rollout 중 가장 새 것.
/// 부모 스레드가 있는(=서브 스레드) rollout은 제외한다.
fn find_by_cwd(sessions: &Path, cwd: &str) -> Option<PathBuf> {
    let now = SystemTime::now();
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for path in rollout_files(sessions) {
        let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
            continue;
        };
        if now
            .duration_since(modified)
            .is_ok_and(|age| age > LIVE_WINDOW)
        {
            continue;
        }
        // 부모가 있는 스레드(서브에이전트·guardian_review 등)는 메인 rollout
        // 후보가 아니다 — 사용량 합산에서는 `subthread_delta`가 따로 되불러온다.
        let Some(meta) = read_head_meta(&path) else {
            continue;
        };
        if meta.parent.is_some() || meta.cwd.as_deref() != Some(cwd) {
            continue;
        }
        if best.as_ref().is_none_or(|(at, _)| modified > *at) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, path)| path)
}

/// 첫 줄(`session_meta`)에서 스레드 신원을 읽는다. 첫 줄이 `session_meta`가
/// 아니면(잘린 파일 등) None.
///
/// `parent_thread_id`가 서브스레드 판정의 유일한 기준이다 — 처음엔
/// `thread_source == "subagent"` 단일 비교였지만, 같은 cwd에
/// `thread_source: "guardian_review"`이면서 부모를 든 rollout이 실측에서
/// 확인됐다(서브 스레드인데 값이 "subagent"가 아니었다).
fn read_head_meta(path: &Path) -> Option<ThreadMeta> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    BufReader::new(file).read_line(&mut line).ok()?;
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    let payload = value.get("payload")?;
    let text = |field: &str| {
        payload
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
    };
    Some(ThreadMeta {
        id: text("id"),
        parent: text("parent_thread_id"),
        cwd: text("cwd"),
        started_at: value
            .get("timestamp")
            .or_else(|| payload.get("timestamp"))
            .and_then(Value::as_str)
            .and_then(parse_rfc3339),
    })
}

/// `2026-08-29T00:00:47.547Z` → SystemTime. 파싱 실패는 None(그러면 그 스레드는
/// "이번 턴에 새로 생겼다"는 증명이 없는 것으로 취급된다).
fn parse_rfc3339(text: &str) -> Option<SystemTime> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| SystemTime::from(at.with_timezone(&chrono::Utc)))
}

/// 파일 앞머리에서 첫 `turn_context.payload.model`.
fn read_head_model(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; ROLLOUT_HEAD_BYTES as usize];
    let read = file.read(&mut buf).ok()?;
    buf.truncate(read);
    let head = String::from_utf8_lossy(&buf);
    head.lines().find_map(|line| {
        let value: Value = serde_json::from_str(line.trim()).ok()?;
        turn_context_model(&value)
    })
}

/// tail을 읽기 전에 길이를 고정해 그 이후 append된 바이트를 cursor가 건너뛰지
/// 않게 한다. 파일이 자라더라도 offset은 이 스냅샷 끝까지만 전진한다.
fn read_tail_snapshot(path: &Path) -> Option<(Cumulative, Option<String>, u64)> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(ROLLOUT_TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(len - start).read_to_end(&mut bytes).ok()?;
    let last_start = bytes
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let consumed = if bytes.last() == Some(&b'\n')
        || serde_json::from_slice::<Value>(&bytes[last_start..]).is_ok()
    {
        bytes.len()
    } else {
        last_start
    };
    let text = String::from_utf8_lossy(&bytes[..consumed]);
    let latest_context = text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| turn_context_model(&value))
        .last();
    let (cumulative, prior_model) = match parse_tail_usage(&text) {
        Some(snapshot) => snapshot,
        // 파일 전체를 읽었고 누계가 없으면 0부터 시작한다. incomplete EOF는
        // consumed에 포함하지 않으므로 처음 token_count를 쓰는 중이어도 안전하다.
        None if start == 0 => (Cumulative::default(), None),
        None => return None,
    };
    Some((
        cumulative,
        latest_context.or(prior_model),
        start + consumed as u64,
    ))
}

fn parse_tail_usage(tail: &str) -> Option<(Cumulative, Option<String>)> {
    let mut model: Option<String> = None;
    let mut cumulative = None;
    for line in tail.lines().rev() {
        // 줄당 1회만 파싱한다(꼬리가 최대 1MB라 이중 파싱은 그대로 낭비다).
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if cumulative.is_none() {
            cumulative = token_count_total(&value);
            continue;
        }
        // 최신 token_count보다 **앞**의 context가 그 누계에 대응한다. 이전 구현은
        // token_count를 먼저 만나 곧장 반환해 context를 보지 못했다.
        if model.is_none() {
            model = turn_context_model(&value);
        }
        if model.is_some() {
            break;
        }
    }
    cumulative.map(|cumulative| (cumulative, model))
}

/// cursor 뒤에 새로 완결된 JSONL 행만 전진하며, 각 token_count 증가분을 그
/// 시점의 turn_context 모델에 귀속한다. 모델 전환이 두 관측 사이에 여러 번
/// 있어도 토큰 행별로 쪼개므로 최신 모델에 전부 몰리지 않는다.
fn read_thread_deltas(path: &Path, cursor: &mut ThreadCursor) -> Vec<ModelDelta> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return vec![];
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if len < cursor.offset {
        *cursor = ThreadCursor::default();
    }
    if file.seek(SeekFrom::Start(cursor.offset)).is_err() {
        return vec![];
    }
    let mut reader = BufReader::new(file);
    let mut deltas = vec![];
    let mut line = Vec::new();
    loop {
        line.clear();
        let Ok(read) = reader.read_until(b'\n', &mut line) else {
            break;
        };
        if read == 0 {
            break;
        }
        let newline = line.last() == Some(&b'\n');
        let Ok(value) = serde_json::from_slice::<Value>(if newline {
            &line[..line.len() - 1]
        } else {
            &line
        }) else {
            // EOF의 잘린 행은 다음 호출에서 다시 읽는다. JSONL의 중간 깨짐은
            // cursor를 넘겨 이후 유효 행을 살린다.
            if !newline {
                break;
            }
            cursor.offset = cursor.offset.saturating_add(read as u64);
            continue;
        };
        if let Some(model) = turn_context_model(&value) {
            cursor.model = Some(model);
        }
        if let Some(current) = token_count_total(&value) {
            let delta = current.delta(cursor.cumulative);
            cursor.cumulative = current;
            if delta.input > 0
                || delta.cached_input > 0
                || delta.cache_write > 0
                || delta.output > 0
            {
                deltas.push(ModelDelta {
                    cumulative: delta,
                    model: cursor.model.clone(),
                });
            }
        }
        cursor.offset = cursor.offset.saturating_add(read as u64);
    }
    deltas
}

fn tokens_from_deltas(deltas: Vec<ModelDelta>) -> Option<SessionEventTokens> {
    if deltas.is_empty() {
        return None;
    }
    let mut grouped: Vec<(Option<String>, Cumulative)> = vec![];
    let mut total = Cumulative::default();
    let mut representative = None;
    for delta in deltas {
        total = total.plus(delta.cumulative);
        representative = delta.model.clone().or(representative);
        if let Some((_, acc)) = grouped.iter_mut().find(|(model, _)| *model == delta.model) {
            *acc = acc.plus(delta.cumulative);
        } else {
            grouped.push((delta.model, delta.cumulative));
        }
    }
    let by_model = grouped
        .into_iter()
        .map(|(model, c)| SessionModelTokens {
            input: Some(
                c.input
                    .saturating_sub(c.cached_input)
                    .saturating_sub(c.cache_write),
            ),
            output: Some(c.output),
            cache_read: Some(c.cached_input),
            cache_write: Some(c.cache_write),
            model,
        })
        .collect();
    Some(SessionEventTokens {
        input: Some(
            total
                .input
                .saturating_sub(total.cached_input)
                .saturating_sub(total.cache_write),
        ),
        output: Some(total.output),
        cache_read: Some(total.cached_input),
        cache_write: Some(total.cache_write),
        model: representative,
        by_model: Some(by_model),
    })
}

fn zero_tokens(model: Option<String>) -> SessionEventTokens {
    SessionEventTokens {
        input: Some(0),
        output: Some(0),
        cache_read: Some(0),
        cache_write: Some(0),
        model,
        by_model: Some(vec![]),
    }
}

/// `{"type":"turn_context","payload":{"model":"gpt-5.4",...}}` → 모델 이름.
fn turn_context_model(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) != Some("turn_context") {
        return None;
    }
    value
        .get("payload")?
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.trim().is_empty())
        .map(str::to_string)
}

/// `token_count` 이벤트의 `info.total_token_usage`(세션 누계).
fn token_count_total(value: &Value) -> Option<Cumulative> {
    let payload = value.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    let total = payload.get("info")?.get("total_token_usage")?;
    let field = |name: &str| total.get(name).and_then(Value::as_u64).unwrap_or(0);
    Some(Cumulative {
        input: field("input_tokens"),
        cached_input: field("cached_input_tokens"),
        cache_write: field("cache_write_input_tokens"),
        output: field("output_tokens"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        std::env::temp_dir().join(format!("agent-office-codex-usage-{}", uuid::Uuid::new_v4()))
    }

    fn token_count(input: u64, cached: u64, output: u64) -> String {
        format!(
            r#"{{"timestamp":"2026-08-21T00:00:00.000Z","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{input},"cached_input_tokens":{cached},"output_tokens":{output},"total_tokens":0}}}},"rate_limits":null}}}}"#
        )
    }

    fn meta(cwd: &str) -> String {
        format!(
            r#"{{"type":"session_meta","payload":{{"id":"sess-1","cwd":"{cwd}","originator":"cli"}}}}"#
        )
    }

    const TURN_CONTEXT: &str =
        r#"{"type":"turn_context","payload":{"cwd":"/w","model":"gpt-5.4","effort":"medium"}}"#;

    /// `sessions/2026/08/21/rollout-...-<id>.jsonl` 를 만들고 경로를 돌려준다.
    fn write_rollout(root: &Path, id: &str, lines: &[String]) -> PathBuf {
        let dir = root.join("sessions").join("2026").join("08").join("21");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-2026-08-21T00-00-00-{id}.jsonl"));
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    fn body(session_id: &str, cwd: &str) -> Vec<u8> {
        serde_json::json!({ "session_id": session_id, "cwd": cwd })
            .to_string()
            .into_bytes()
    }

    #[test]
    fn stop_reports_the_delta_since_the_turn_started() {
        let root = scratch();
        let path = write_rollout(
            &root,
            "sess-1",
            &[
                meta("/w"),
                TURN_CONTEXT.into(),
                token_count(1_000, 800, 100),
            ],
        );
        let tracker = CodexUsageTracker::new(Some(root.join("sessions")));
        let hook = body("sess-1", "/w");

        tracker.mark_turn_start(&hook);
        // 턴 진행 중 누계가 늘어난다.
        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push('\n');
        content.push_str(&token_count(3_500, 2_800, 400));
        std::fs::write(&path, content).unwrap();

        let tokens = tracker.turn_usage(&hook).unwrap();
        // input 은 캐시를 제외한 순수 입력: (3500-1000) - (2800-800) = 500.
        assert_eq!(tokens.input, Some(500));
        assert_eq!(tokens.cache_read, Some(2_000));
        assert_eq!(tokens.output, Some(300));
        assert_eq!(tokens.cache_write, Some(0));
        assert_eq!(tokens.model.as_deref(), Some("gpt-5.4"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_baseline_is_skipped_but_seeds_the_next_turn() {
        let root = scratch();
        let path = write_rollout(
            &root,
            "sess-1",
            &[meta("/w"), TURN_CONTEXT.into(), token_count(9_000, 0, 900)],
        );
        let tracker = CodexUsageTracker::new(Some(root.join("sessions")));
        let hook = body("sess-1", "/w");

        // 앱이 도중에 켜진 경우 — 기준이 없으므로 이번 턴은 생략(누계 전체를
        // 한 턴에 몰아 넣지 않는다).
        assert_eq!(tracker.turn_usage(&hook), None);

        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push('\n');
        content.push_str(&token_count(9_500, 0, 950));
        std::fs::write(&path, content).unwrap();

        // 다음 턴부터는 정상 델타.
        tracker.mark_turn_start(&hook);
        let tokens = tracker.turn_usage(&hook).unwrap();
        assert_eq!(tokens.input, Some(500));
        assert_eq!(tokens.output, Some(50));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn repeated_prompts_keep_the_first_baseline_of_the_turn() {
        let root = scratch();
        let path = write_rollout(
            &root,
            "sess-1",
            &[meta("/w"), TURN_CONTEXT.into(), token_count(100, 0, 10)],
        );
        let tracker = CodexUsageTracker::new(Some(root.join("sessions")));
        let hook = body("sess-1", "/w");

        tracker.mark_turn_start(&hook); // 기준 = 100/10
        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push('\n');
        content.push_str(&token_count(600, 0, 60));
        std::fs::write(&path, content).unwrap();
        tracker.mark_turn_start(&hook); // 작업 중 추가 지시 — 기준을 밀지 않는다

        let tokens = tracker.turn_usage(&hook).unwrap();
        assert_eq!(tokens.input, Some(500));
        assert_eq!(tokens.output, Some(50));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn falls_back_to_cwd_when_the_hook_has_no_session_id() {
        let root = scratch();
        let path = write_rollout(
            &root,
            "other-id",
            &[meta("/w"), TURN_CONTEXT.into(), token_count(100, 0, 10)],
        );
        let tracker = CodexUsageTracker::new(Some(root.join("sessions")));
        let hook = serde_json::json!({ "cwd": "/w" }).to_string().into_bytes();

        tracker.mark_turn_start(&hook);
        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push('\n');
        content.push_str(&token_count(300, 0, 30));
        std::fs::write(&path, content).unwrap();

        let tokens = tracker.turn_usage(&hook).unwrap();
        assert_eq!(tokens.input, Some(200));
        assert_eq!(tokens.output, Some(20));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unknown_session_and_empty_delta_yield_none() {
        let root = scratch();
        write_rollout(
            &root,
            "sess-1",
            &[meta("/w"), TURN_CONTEXT.into(), token_count(100, 0, 10)],
        );
        let tracker = CodexUsageTracker::new(Some(root.join("sessions")));

        // rollout 을 못 찾는 세션.
        let unknown = body("sess-missing", "/nope");
        tracker.mark_turn_start(&unknown);
        assert_eq!(tracker.turn_usage(&unknown), None);

        // 누계가 그대로면 델타 0 → 빈 tokens 는 싣지 않는다.
        let hook = body("sess-1", "/w");
        tracker.mark_turn_start(&hook);
        assert_eq!(tracker.turn_usage(&hook), None);

        // sessions_root 자체가 없으면 통째로 비활성.
        let disabled = CodexUsageTracker::new(None);
        disabled.mark_turn_start(&hook);
        assert_eq!(disabled.turn_usage(&hook), None);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn fresh_session_without_token_count_gets_a_zero_baseline() {
        let root = scratch();
        // 신선한 세션 — 아직 token_count가 한 번도 안 실렸다. 파일 전체가
        // 꼬리 상한보다 훨씬 작으니 "누계가 진짜 0"임이 증명돼 기준 0을 심는다.
        let path = write_rollout(&root, "sess-1", &[meta("/w"), TURN_CONTEXT.into()]);
        let tracker = CodexUsageTracker::new(Some(root.join("sessions")));
        let hook = body("sess-1", "/w");

        tracker.mark_turn_start(&hook);

        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push('\n');
        content.push_str(&token_count(500, 0, 50));
        std::fs::write(&path, content).unwrap();

        // 기준이 0이었으므로 이번 턴 전체가 그대로 집계된다(생략되지 않는다).
        let tokens = tracker.turn_usage(&hook).unwrap();
        assert_eq!(tokens.input, Some(500));
        assert_eq!(tokens.output, Some(50));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn truncated_tail_without_token_count_still_skips_the_turn() {
        let root = scratch();
        // 꼬리 상한(1MB)보다 크게 패딩 — token_count가 없어도 "파일 전체를
        // 봤다"는 증명이 안 되므로(꼬리 밖 어딘가에 있을 수 있음) 기준 0을
        // 심지 않는다(결정 B).
        let padding = "x".repeat(ROLLOUT_TAIL_BYTES as usize + 1024);
        write_rollout(&root, "sess-1", &[meta("/w"), TURN_CONTEXT.into(), padding]);
        let tracker = CodexUsageTracker::new(Some(root.join("sessions")));
        let hook = body("sess-1", "/w");

        tracker.mark_turn_start(&hook);
        // 기준이 안 심어졌으므로 첫 Stop은 여전히 생략된다.
        assert_eq!(tracker.turn_usage(&hook), None);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sub_thread_rollouts_are_ignored_by_the_cwd_fallback() {
        let root = scratch();
        // user 스레드 rollout(먼저 생성 — 더 오래됨).
        let user_path = write_rollout(
            &root,
            "sess-1",
            &[meta("/w"), TURN_CONTEXT.into(), token_count(100, 0, 10)],
        );
        // 서브 스레드 rollout. thread_source가 "subagent"가 아니라
        // "guardian_review"라 예전 필터를 통과했지만, parent_thread_id를
        // 들고 있어(실측 사례) 새 필터로 배제돼야 한다. 나중에 써서 더
        // 최근이다.
        let sub_meta = r#"{"type":"session_meta","payload":{"id":"sess-2","cwd":"/w","originator":"cli","thread_source":"guardian_review","parent_thread_id":"sess-1"}}"#;
        write_rollout(
            &root,
            "sess-2",
            &[
                sub_meta.to_string(),
                TURN_CONTEXT.into(),
                token_count(9_999, 0, 999),
            ],
        );

        let tracker = CodexUsageTracker::new(Some(root.join("sessions")));
        // session_id 없이 cwd만 실어 폴백을 강제한다.
        let hook = serde_json::json!({ "cwd": "/w" }).to_string().into_bytes();

        // user rollout(100/10)이 골라져야 기준이 맞는다. 서브 스레드가
        // 골라졌다면 기준이 9999/999가 돼 아래 델타가 어긋난다.
        tracker.mark_turn_start(&hook);
        let mut content = std::fs::read_to_string(&user_path).unwrap();
        content.push('\n');
        content.push_str(&token_count(300, 0, 30));
        std::fs::write(&user_path, content).unwrap();

        let tokens = tracker.turn_usage(&hook).unwrap();
        assert_eq!(tokens.input, Some(200));
        assert_eq!(tokens.output, Some(20));

        let _ = std::fs::remove_dir_all(root);
    }

    /// `session_meta` 한 줄(부모/시각 지정 가능).
    fn meta_line(id: &str, cwd: &str, parent: Option<&str>, at: SystemTime) -> String {
        let timestamp = chrono::DateTime::<chrono::Utc>::from(at).to_rfc3339();
        let parent = match parent {
            Some(parent) => format!(r#""{parent}""#),
            None => "null".to_string(),
        };
        format!(
            r#"{{"timestamp":"{timestamp}","type":"session_meta","payload":{{"id":"{id}","cwd":"{cwd}","parent_thread_id":{parent},"originator":"cli"}}}}"#
        )
    }

    /// Codex는 서브에이전트·guardian_review를 **별도 rollout**으로 돌리고
    /// `total_token_usage`는 스레드별 누계다 — 메인만 보면 그 몫(실측 약 39%)이
    /// 어디에도 안 잡힌다. 이번 턴에 생긴 서브스레드는 누계 전체가 이 턴 몫이다.
    #[test]
    fn stop_adds_subthread_rollouts_spawned_during_this_turn() {
        let root = scratch();
        let sessions = root.join("sessions");
        let main = write_rollout(
            &root,
            "sess-1",
            &[
                meta_line("sess-1", "/w", None, SystemTime::now()),
                TURN_CONTEXT.into(),
                token_count(1_000, 800, 100),
            ],
        );
        let tracker = CodexUsageTracker::new(Some(sessions));
        let hook = body("sess-1", "/w");
        tracker.mark_turn_start(&hook);

        // 턴 도중: 메인 누계가 늘고, 서브스레드 rollout이 새로 생긴다.
        let mut grown = std::fs::read_to_string(&main).unwrap();
        grown.push('\n');
        grown.push_str(&token_count(3_500, 2_800, 400));
        std::fs::write(&main, grown).unwrap();
        write_rollout(
            &root,
            "sub-1",
            &[
                meta_line("sub-1", "/w", Some("sess-1"), SystemTime::now()),
                TURN_CONTEXT.into(),
                token_count(500, 100, 50),
            ],
        );

        let tokens = tracker.turn_usage(&hook).unwrap();
        // 메인 델타(2500/2000/300) + 서브 누계 전체(500/100/50).
        // 순수 입력 = (2500+500) - (2000+100) = 900.
        assert_eq!(tokens.input, Some(900));
        assert_eq!(tokens.cache_read, Some(2_100));
        assert_eq!(tokens.output, Some(350));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 턴 시작보다 먼저 생긴 서브스레드는 어디까지가 이 턴인지 모른다 — 기준만
    /// 심고 이번 턴은 생략하되, 다음 턴부터는 델타가 정상으로 잡힌다.
    #[test]
    fn subthread_older_than_the_turn_only_plants_a_baseline() {
        let root = scratch();
        let sessions = root.join("sessions");
        let main = write_rollout(
            &root,
            "sess-1",
            &[
                meta_line("sess-1", "/w", None, SystemTime::now()),
                TURN_CONTEXT.into(),
                token_count(1_000, 0, 100),
            ],
        );
        let old_start = SystemTime::now() - Duration::from_secs(3_600);
        let sub = write_rollout(
            &root,
            "sub-1",
            &[
                meta_line("sub-1", "/w", Some("sess-1"), old_start),
                TURN_CONTEXT.into(),
                token_count(9_000, 0, 900),
            ],
        );
        let tracker = CodexUsageTracker::new(Some(sessions));
        let hook = body("sess-1", "/w");
        tracker.mark_turn_start(&hook);

        let mut grown = std::fs::read_to_string(&main).unwrap();
        grown.push('\n');
        grown.push_str(&token_count(1_200, 0, 150));
        std::fs::write(&main, grown).unwrap();

        // 첫 턴: 서브의 옛 누계 9000은 안 실린다(메인 델타 200/50만).
        let tokens = tracker.turn_usage(&hook).unwrap();
        assert_eq!(tokens.input, Some(200));
        assert_eq!(tokens.output, Some(50));

        // 다음 턴: 서브가 더 돌면 그 델타는 실린다.
        tracker.mark_turn_start(&hook);
        let mut grown = std::fs::read_to_string(&sub).unwrap();
        grown.push('\n');
        grown.push_str(&token_count(9_400, 0, 950));
        std::fs::write(&sub, grown).unwrap();
        let tokens = tracker.turn_usage(&hook).unwrap();
        assert_eq!(tokens.input, Some(400));
        assert_eq!(tokens.output, Some(50));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn switched_models_and_cache_writes_are_counted_per_snapshot() {
        let root = scratch();
        let path = write_rollout(
            &root,
            "sess-1",
            &[
                meta("/w"),
                r#"{"type":"turn_context","payload":{"model":"model-a"}}"#.into(),
                token_count(100, 20, 10),
            ],
        );
        let tracker = CodexUsageTracker::new(Some(root.join("sessions")));
        let hook = body("sess-1", "/w");
        tracker.mark_turn_start(&hook);
        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push('\n');
        content.push_str(r#"{"type":"turn_context","payload":{"model":"model-a"}}"#);
        content.push('\n');
        content.push_str(r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":160,"cached_input_tokens":30,"cache_write_input_tokens":10,"output_tokens":20}}}}"#);
        content.push('\n');
        content.push_str(r#"{"type":"turn_context","payload":{"model":"model-b"}}"#);
        content.push('\n');
        content.push_str(r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":260,"cached_input_tokens":50,"cache_write_input_tokens":30,"output_tokens":50}}}}"#);
        std::fs::write(&path, content).unwrap();
        let tokens = tracker.turn_usage(&hook).unwrap();
        assert_eq!(tokens.input, Some(100)); // raw 160 - read 30 - write 30
        assert_eq!(tokens.cache_read, Some(30));
        assert_eq!(tokens.cache_write, Some(30));
        assert_eq!(tokens.output, Some(40));
        let by_model = tokens.by_model.unwrap();
        assert_eq!(by_model.len(), 2);
        assert_eq!(by_model[0].model.as_deref(), Some("model-a"));
        assert_eq!(by_model[0].input, Some(40));
        assert_eq!(by_model[1].model.as_deref(), Some("model-b"));
        assert_eq!(by_model[1].input, Some(60));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn partial_then_stop_emits_final_zero_once_without_duplication() {
        let root = scratch();
        let path = write_rollout(
            &root,
            "sess-1",
            &[meta("/w"), TURN_CONTEXT.into(), token_count(10, 0, 1)],
        );
        let tracker = CodexUsageTracker::new(Some(root.join("sessions")));
        let hook = body("sess-1", "/w");
        tracker.mark_turn_start(&hook);
        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push('\n');
        content.push_str(&token_count(20, 0, 2));
        std::fs::write(&path, content).unwrap();
        assert_eq!(tracker.progress_usage(&hook).unwrap().input, Some(10));
        assert_eq!(tracker.turn_usage(&hook).unwrap().input, Some(0));
        assert_eq!(tracker.turn_usage(&hook), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn seed_uses_context_after_latest_token_count() {
        let root = scratch();
        let path = write_rollout(
            &root,
            "sess-1",
            &[
                meta("/w"),
                r#"{"type":"turn_context","payload":{"model":"old"}}"#.into(),
                token_count(10, 0, 1),
                r#"{"type":"turn_context","payload":{"model":"new"}}"#.into(),
            ],
        );
        let tracker = CodexUsageTracker::new(Some(root.join("sessions")));
        let hook = body("sess-1", "/w");
        tracker.mark_turn_start(&hook);
        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push('\n');
        content.push_str(&token_count(20, 0, 2));
        std::fs::write(&path, content).unwrap();
        let tokens = tracker.turn_usage(&hook).unwrap();
        assert_eq!(tokens.by_model.unwrap()[0].model.as_deref(), Some("new"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn incomplete_last_json_line_is_retried_after_completion() {
        let root = scratch();
        let path = write_rollout(
            &root,
            "sess-1",
            &[meta("/w"), TURN_CONTEXT.into(), token_count(10, 0, 1)],
        );
        let tracker = CodexUsageTracker::new(Some(root.join("sessions")));
        let hook = body("sess-1", "/w");
        tracker.mark_turn_start(&hook);
        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push_str("\n{\"type\":\"turn_context\"");
        std::fs::write(&path, &content).unwrap();
        assert_eq!(tracker.progress_usage(&hook), None);
        content.push_str(",\"payload\":{\"model\":\"gpt-5.4\"}}\n");
        content.push_str(&token_count(20, 0, 2));
        std::fs::write(&path, content).unwrap();
        assert_eq!(tracker.turn_usage(&hook).unwrap().input, Some(10));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn snapshot_retries_partial_first_or_later_token_count() {
        for has_history in [false, true] {
            let root = scratch();
            let mut lines = vec![meta("/w"), TURN_CONTEXT.into()];
            if has_history {
                lines.push(token_count(10, 0, 1));
            }
            let row = token_count(20, 0, 2);
            lines.push(row[..row.len() / 2].into());
            let path = write_rollout(&root, "sess-1", &lines);
            let tracker = CodexUsageTracker::new(Some(root.join("sessions")));
            let hook = body("sess-1", "/w");
            tracker.mark_turn_start(&hook);
            let mut content = std::fs::read_to_string(&path).unwrap();
            content.push_str(&row[row.len() / 2..]);
            std::fs::write(&path, content).unwrap();
            assert_eq!(
                tracker.turn_usage(&hook).unwrap().input,
                Some(if has_history { 10 } else { 20 })
            );
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn attach_at_progress_can_finalize_after_a_partial() {
        let root = scratch();
        let path = write_rollout(
            &root,
            "sess-1",
            &[meta("/w"), TURN_CONTEXT.into(), token_count(10, 0, 1)],
        );
        let tracker = CodexUsageTracker::new(Some(root.join("sessions")));
        let hook = body("sess-1", "/w");
        assert_eq!(tracker.progress_usage(&hook), None);
        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push('\n');
        content.push_str(&token_count(20, 0, 2));
        std::fs::write(&path, content).unwrap();
        tracker.state_lock("sess-1").lock().unwrap().last_progress = None;
        assert_eq!(tracker.progress_usage(&hook).unwrap().input, Some(10));
        assert_eq!(tracker.turn_usage(&hook).unwrap().input, Some(0));
        assert_eq!(tracker.turn_usage(&hook), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn partial_main_and_child_metadata_are_retried_with_child_model() {
        let root = scratch();
        let main_meta = meta_line("sess-1", "/w", None, SystemTime::now());
        let main = write_rollout(&root, "sess-1", &[main_meta[..main_meta.len() / 2].into()]);
        let tracker = CodexUsageTracker::new(Some(root.join("sessions")));
        let hook = body("sess-1", "/w");
        tracker.mark_turn_start(&hook);
        std::fs::write(
            &main,
            format!("{main_meta}\n{TURN_CONTEXT}\n{}", token_count(10, 0, 1)),
        )
        .unwrap();
        let child_meta = meta_line("sub-1", "/w", Some("sess-1"), SystemTime::now());
        let child = write_rollout(&root, "sub-1", &[child_meta[..child_meta.len() / 2].into()]);
        assert_eq!(tracker.progress_usage(&hook).unwrap().input, Some(10));
        let child_context = r#"{"type":"turn_context","payload":{"model":"gpt-6-astra"}}"#;
        std::fs::write(
            child,
            format!(
                "{child_meta}\n{child_context}\n{}",
                token_count(100, 20, 10)
            ),
        )
        .unwrap();
        let tokens = tracker.turn_usage(&hook).unwrap();
        assert_eq!(tokens.input, Some(80));
        assert_eq!(
            tokens.by_model.unwrap()[0].model.as_deref(),
            Some("gpt-6-astra")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn overlapping_progress_and_stop_consume_tokens_once() {
        let root = scratch();
        let path = write_rollout(
            &root,
            "sess-1",
            &[meta("/w"), TURN_CONTEXT.into(), token_count(10, 0, 1)],
        );
        let tracker = Arc::new(CodexUsageTracker::new(Some(root.join("sessions"))));
        let hook = body("sess-1", "/w");
        tracker.mark_turn_start(&hook);
        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push('\n');
        content.push_str(&token_count(20, 0, 2));
        std::fs::write(path, content).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = [true, false]
            .into_iter()
            .map(|progress| {
                let tracker = tracker.clone();
                let hook = hook.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    tracker.usage(&hook, progress)
                })
            })
            .collect();
        let observed: Vec<_> = workers
            .into_iter()
            .filter_map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(
            observed
                .iter()
                .map(|tokens| tokens.input.unwrap_or(0))
                .sum::<u64>(),
            10
        );
        assert_eq!(tracker.turn_usage(&hook), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn late_main_append_after_stop_is_not_lost_at_next_prompt() {
        let root = scratch();
        let path = write_rollout(
            &root,
            "sess-1",
            &[meta("/w"), TURN_CONTEXT.into(), token_count(10, 0, 1)],
        );
        let tracker = CodexUsageTracker::new(Some(root.join("sessions")));
        let hook = body("sess-1", "/w");
        tracker.mark_turn_start(&hook);
        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push('\n');
        content.push_str(&token_count(20, 0, 2));
        std::fs::write(&path, &content).unwrap();
        assert_eq!(tracker.turn_usage(&hook).unwrap().input, Some(10));
        content.push('\n');
        content.push_str(&token_count(30, 0, 3));
        std::fs::write(&path, content).unwrap();
        assert_eq!(tracker.turn_usage(&hook), None); // duplicate Stop must leave late bytes unread
        tracker.mark_turn_start(&hook);
        assert_eq!(tracker.turn_usage(&hook).unwrap().input, Some(10));
        let _ = std::fs::remove_dir_all(root);
    }
}
