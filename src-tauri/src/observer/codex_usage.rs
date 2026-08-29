// src-tauri/src/observer/codex_usage.rs
//
// Codex 턴 토큰 사용량 추출. Codex 훅 body에는 사용량이 실려 오지 않으므로
// rollout JSONL(`<CODEX_HOME>/sessions/YYYY/MM/DD/rollout-*.jsonl`)의
// `token_count` 이벤트를 읽는다. 그 이벤트의 `info.total_token_usage`는
// **세션 누계**라 그대로 쓰면 턴 단위가 아니다 — 그래서 턴 시작(UserPromptSubmit)
// 시점의 누계를 기억해 두고 Stop에서 델타를 낸다.
//
// 왜 델타인가: rollout에는 "이번 턴" 경계 표시가 없고(태스크 경계 이벤트는
// 있지만 turn 단위 사용량 필드는 없다) 누계만 append되므로, 경계 두 지점의
// 누계 차이가 가장 단순하고 견고하다. 앱이 도중에 껐다 켜져 기준이 없으면
// 그 턴은 조용히 생략한다(과대 집계보다 누락이 낫다).
//
// 단, 신선한 세션(첫 턴이 시작될 때까지 `token_count`가 한 번도 안 실린
// rollout)은 예외다 — 파일 전체를 읽었는데도 `token_count`가 없다면 그건
// "기준을 못 구함"이 아니라 "누계가 진짜 0"이 증명된 것이므로 기준 0을 심는다.
// 그래야 그 세션의 첫 턴이 통째로 생략되지 않는다. 이 증명은 파일이 꼬리
// 읽기 상한(`ROLLOUT_TAIL_BYTES`) 안에 통째로 들어올 때만 성립한다 — 상한에
// 걸려 잘린 큰 파일은 꼬리에 `token_count`가 없어도 앞쪽 어딘가에 있을 수
// 있으므로 기존대로 생략한다.
//
// rollout 파일 찾기: 훅 body의 `session_id`가 rollout 파일명 꼬리
// (`...-<session_id>.jsonl`)와 같다는 규약을 먼저 쓰고, 없으면 body의 `cwd`와
// 첫 줄(`session_meta`)의 cwd가 같은 최근 파일로 폴백한다(agent_transcript/
// codex.rs가 쓰는 것과 같은 휴리스틱). 못 찾으면 조용히 생략.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::types::SessionEventTokens;

use super::event::{hook_cwd, native_session_id};

/// rollout 꼬리에서 마지막 `token_count`를 찾을 때 읽는 최대 바이트. 도구
/// 출력이 큰 턴을 감안해 넉넉히 잡되, 이 안에서 못 찾으면 포기한다.
const ROLLOUT_TAIL_BYTES: u64 = 1024 * 1024;

/// 모델 이름(`turn_context`)을 찾을 때 읽는 파일 앞머리 바이트. `session_meta`
/// (긴 base_instructions 포함) 바로 뒤에 첫 `turn_context`가 오므로 앞머리만
/// 읽으면 된다. 세션 도중 `/model`로 바꾸면 첫 모델이 남지만, 비용 환산
/// 대푯값으로는 충분하다(꼬리에서 turn_context를 만나면 그 값을 우선한다).
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
    output: u64,
}

impl Cumulative {
    /// `self - previous`. 각 항목은 음수로 내려가지 않게 saturating.
    fn delta(self, previous: Self) -> Self {
        Self {
            input: self.input.saturating_sub(previous.input),
            cached_input: self.cached_input.saturating_sub(previous.cached_input),
            output: self.output.saturating_sub(previous.output),
        }
    }

    /// `self + other`(메인 스레드 몫 + 서브스레드 몫 합산용).
    fn plus(self, other: Self) -> Self {
        Self {
            input: self.input.saturating_add(other.input),
            cached_input: self.cached_input.saturating_add(other.cached_input),
            output: self.output.saturating_add(other.output),
        }
    }

    /// 시계열 토큰 필드로. Codex `input_tokens`는 캐시 히트를 **포함**하므로
    /// 빼서 순수 입력으로 만든다(SessionEventTokens 계약). 캐시 기록/읽기
    /// 구분이 없어 cache_write는 항상 생략한다.
    fn into_tokens(self, model: Option<String>) -> SessionEventTokens {
        SessionEventTokens {
            input: Some(self.input.saturating_sub(self.cached_input)),
            output: Some(self.output),
            cache_read: Some(self.cached_input),
            cache_write: None,
            model,
        }
    }
}

/// 세션별 턴 경계 누계를 들고 있는 추출기. `CodexAdapter`가 소유한다.
pub struct CodexUsageTracker {
    /// `<CODEX_HOME>/sessions`. 부재(환경 미해석)면 추출을 통째로 포기한다.
    sessions_root: Option<PathBuf>,
    /// 키(세션 ID 또는 cwd) → 마지막 턴 경계의 누계.
    baselines: Mutex<HashMap<String, Cumulative>>,
    /// 키 → 마지막 턴 경계를 심은 시각. 서브스레드 rollout이 "이번 턴에 새로
    /// 생긴 것"인지 판정하는 기준이다(`subthread_delta`).
    turn_started: Mutex<HashMap<String, SystemTime>>,
    /// 키 → (rollout 경로, 앞머리에서 읽은 모델). 매 턴 디렉터리를 다시 훑지
    /// 않기 위한 캐시. 경로가 사라지면 다음 조회에서 자연히 다시 찾는다.
    located: Mutex<HashMap<String, Located>>,
    /// 키 → 이 세션의 서브스레드 rollout별 마지막 턴 경계 누계. 메인과 같은
    /// 델타 규칙을 파일마다 독립적으로 적용한다.
    sub_baselines: Mutex<HashMap<String, HashMap<PathBuf, Cumulative>>>,
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
            baselines: Mutex::new(HashMap::new()),
            turn_started: Mutex::new(HashMap::new()),
            located: Mutex::new(HashMap::new()),
            sub_baselines: Mutex::new(HashMap::new()),
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
        let Some(key) = self.key_for(body) else { return };
        {
            let baselines = self.baselines.lock().unwrap();
            if baselines.contains_key(&key) {
                return;
            }
        }
        let Some(located) = self.locate(body, &key) else {
            return;
        };
        let current = match read_tail_usage(&located.path) {
            Some((cumulative, _)) => cumulative,
            // 파일 전체를 읽었는데 token_count가 없다 = 누계가 진짜 0인 신선한
            // 세션. 꼬리 상한에 걸려 잘린 파일은 이 증명이 안 되므로 기존대로
            // 생략한다(위 헤더 주석 참고).
            None if whole_file_scanned(&located.path) => Cumulative::default(),
            None => return,
        };
        self.mark_turn_boundary(&key, current);
    }

    /// 턴 경계(기준 누계 + 그 시각)를 함께 심는다. 시각은 서브스레드 rollout이
    /// "이번 턴에 새로 생긴 것"인지 판정하는 데 쓴다.
    fn mark_turn_boundary(&self, key: &str, current: Cumulative) {
        self.baselines.lock().unwrap().insert(key.to_string(), current);
        self.turn_started
            .lock()
            .unwrap()
            .insert(key.to_string(), SystemTime::now());
    }

    /// 턴 종료(Stop) — 기준 대비 델타를 돌려주고, 현재 누계를 새 기준으로
    /// 갱신한다. 기준이 없으면(앱 재시작 등) 이번 턴은 생략하되 기준은 심어
    /// 다음 턴부터 정상 집계되게 한다.
    pub fn turn_usage(&self, body: &[u8]) -> Option<SessionEventTokens> {
        let key = self.key_for(body)?;
        let located = self.locate(body, &key)?;
        let (current, tail_model) = read_tail_usage(&located.path)?;
        let model = tail_model.or_else(|| located.model.clone());

        let previous = self.baselines.lock().unwrap().get(&key).copied();
        let turn_started = self.turn_started.lock().unwrap().get(&key).copied();

        // 서브스레드 몫은 메인 기준이 있든 없든 훑는다 — 기준이 없어 이번 턴을
        // 생략하는 경우에도 서브 쪽 기준을 같이 심어 둬야 다음 턴이 정확하다.
        let sub = match (located.thread_id.as_deref(), turn_started) {
            (Some(thread), Some(started)) => self.subthread_delta(&key, thread, started),
            _ => Cumulative::default(),
        };

        self.mark_turn_boundary(&key, current);
        // 기준이 없으면(앱이 세션 도중에 켜짐) 이번 턴은 통째로 생략한다 —
        // 세션 누계 전체를 한 턴에 몰아넣는 과대 집계보다 누락이 낫다.
        let main = current.delta(previous?);
        main.plus(sub).into_tokens(model).non_empty()
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
            let located = self.located.lock().unwrap();
            if let Some(entry) = located.get(key) {
                if entry.path.exists() {
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
    /// 후보는 (a) 이미 기준을 심어 둔 이 세션의 서브 + (b) 턴 시작 이후에 쓰인
    /// rollout이다. 이번 턴 몫이라면 mtime이 턴 시작보다 뒤일 수밖에 없고, 그
    /// 전에 끝난 서브는 이미 그때의 Stop에서 (a)에 들어와 있다.
    fn subthread_delta(&self, key: &str, main_thread: &str, turn_started: SystemTime) -> Cumulative {
        let Some(root) = self.sessions_root.as_ref() else {
            return Cumulative::default();
        };
        let files = rollout_files(root);
        let mut baselines = self.sub_baselines.lock().unwrap();
        let known = baselines.entry(key.to_string()).or_default();

        let mut candidates: Vec<PathBuf> = known.keys().cloned().collect();
        for path in &files {
            if known.contains_key(path) {
                continue;
            }
            let touched = std::fs::metadata(path).and_then(|meta| meta.modified()).ok();
            if touched.is_none_or(|at| at + TIMESTAMP_SLACK < turn_started) {
                continue;
            }
            candidates.push(path.clone());
        }
        if candidates.is_empty() {
            return Cumulative::default();
        }

        // 부모 사슬을 따라 올라가려면 id로도 찾을 수 있어야 한다(서브의 서브).
        let mut by_id: HashMap<String, ThreadMeta> = HashMap::new();
        for path in &files {
            if let Some(meta) = self.head_meta(path) {
                if let Some(id) = meta.id.clone() {
                    by_id.insert(id, meta);
                }
            }
        }

        let mut total = Cumulative::default();
        for path in candidates {
            let Some(meta) = self.head_meta(&path) else {
                continue;
            };
            if !descends_from(&meta, main_thread, &by_id) {
                continue;
            }
            let Some((current, _)) = read_tail_usage(&path) else {
                continue;
            };
            match known.get(&path).copied() {
                Some(previous) => total = total.plus(current.delta(previous)),
                // 처음 보는 서브: 이 턴 안에서 시작된 스레드면 누계 전체가 이 턴
                // 몫이다. 그보다 먼저 시작된 스레드(앱이 세션 도중에 켜진 경우
                // 등)는 어디까지가 이 턴인지 모르므로 기준만 심고 이번 턴은
                // 생략한다 — 메인 쪽 규칙과 같은 결(과대보다 누락).
                None if meta
                    .started_at
                    .is_some_and(|at| at + TIMESTAMP_SLACK >= turn_started) =>
                {
                    total = total.plus(current);
                }
                None => {}
            }
            known.insert(path, current);
        }
        total
    }

    /// rollout 첫 줄의 스레드 신원(캐시). 첫 줄은 변하지 않으므로 한 번만 읽는다.
    fn head_meta(&self, path: &Path) -> Option<ThreadMeta> {
        if let Some(cached) = self.meta_cache.lock().unwrap().get(path) {
            return cached.clone();
        }
        let meta = read_head_meta(path);
        self.meta_cache
            .lock()
            .unwrap()
            .insert(path.to_path_buf(), meta.clone());
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

/// 파일 꼬리에서 마지막 `token_count` 누계와(있으면) 가장 최근 `turn_context`
/// 모델. `token_count`를 못 찾으면 None.
fn read_tail_usage(path: &Path) -> Option<(Cumulative, Option<String>)> {
    let tail = read_file_tail(path, ROLLOUT_TAIL_BYTES)?;
    let mut model: Option<String> = None;
    for line in tail.lines().rev() {
        // 줄당 1회만 파싱한다(꼬리가 최대 1MB라 이중 파싱은 그대로 낭비다).
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if model.is_none() {
            model = turn_context_model(&value);
        }
        if let Some(cumulative) = token_count_total(&value) {
            return Some((cumulative, model));
        }
    }
    None
}

/// 파일 전체가 꼬리 읽기(`ROLLOUT_TAIL_BYTES`) 범위 안에 들어오는가. 이게
/// 참이어야 "꼬리에 token_count가 없다"가 "누계가 진짜 0이다"의 증명이 된다
/// (결정 B) — 그렇지 않으면 앞쪽 어딘가에 있는 token_count를 놓친 것일 수
/// 있다.
fn whole_file_scanned(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.len() <= ROLLOUT_TAIL_BYTES)
        .unwrap_or(false)
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
        output: field("output_tokens"),
    })
}

/// 파일 끝에서 최대 `max` 바이트. 앞머리의 잘린 줄은 파싱 실패로 자연히 스킵된다.
fn read_file_tail(path: &Path, max: u64) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(max))).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
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
        assert_eq!(tokens.cache_write, None);
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
}
