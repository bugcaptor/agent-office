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
    /// 키 → (rollout 경로, 앞머리에서 읽은 모델). 매 턴 디렉터리를 다시 훑지
    /// 않기 위한 캐시. 경로가 사라지면 다음 조회에서 자연히 다시 찾는다.
    located: Mutex<HashMap<String, (PathBuf, Option<String>)>>,
}

impl CodexUsageTracker {
    pub fn new(sessions_root: Option<PathBuf>) -> Self {
        Self {
            sessions_root,
            baselines: Mutex::new(HashMap::new()),
            located: Mutex::new(HashMap::new()),
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
        let Some((current, _)) = self.read_current(body, &key) else {
            return;
        };
        self.baselines.lock().unwrap().insert(key, current);
    }

    /// 턴 종료(Stop) — 기준 대비 델타를 돌려주고, 현재 누계를 새 기준으로
    /// 갱신한다. 기준이 없으면(앱 재시작 등) 이번 턴은 생략하되 기준은 심어
    /// 다음 턴부터 정상 집계되게 한다.
    pub fn turn_usage(&self, body: &[u8]) -> Option<SessionEventTokens> {
        let key = self.key_for(body)?;
        let (current, model) = self.read_current(body, &key)?;
        let previous = self.baselines.lock().unwrap().insert(key, current);
        current.delta(previous?).into_tokens(model).non_empty()
    }

    /// 이 세션을 식별하는 캐시 키. 훅 body의 native session_id 우선, 없으면 cwd.
    fn key_for(&self, body: &[u8]) -> Option<String> {
        self.sessions_root.as_ref()?;
        native_session_id(body).or_else(|| hook_cwd(body))
    }

    /// 현재 누계와 대표 모델. rollout을 못 찾거나 `token_count`가 없으면 None.
    fn read_current(&self, body: &[u8], key: &str) -> Option<(Cumulative, Option<String>)> {
        let (path, head_model) = self.locate(body, key)?;
        let (cumulative, tail_model) = read_tail_usage(&path)?;
        Some((cumulative, tail_model.or(head_model)))
    }

    /// 이 세션의 rollout 경로(+앞머리 모델)를 찾는다. 캐시된 경로가 아직
    /// 존재하면 그대로 쓴다.
    fn locate(&self, body: &[u8], key: &str) -> Option<(PathBuf, Option<String>)> {
        {
            let located = self.located.lock().unwrap();
            if let Some(entry) = located.get(key) {
                if entry.0.exists() {
                    return Some(entry.clone());
                }
            }
        }
        let root = self.sessions_root.as_ref()?;
        let path = native_session_id(body)
            .and_then(|id| find_by_session_id(root, &id))
            .or_else(|| hook_cwd(body).and_then(|cwd| find_by_cwd(root, &cwd)))?;
        let model = read_head_model(&path);
        let entry = (path, model);
        self.located
            .lock()
            .unwrap()
            .insert(key.to_string(), entry.clone());
        Some(entry)
    }
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
/// 서브에이전트 스레드(`thread_source == "subagent"`)는 제외한다.
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
        if head_meta_cwd(&path).as_deref() != Some(cwd) {
            continue;
        }
        if best.as_ref().is_none_or(|(at, _)| modified > *at) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, path)| path)
}

/// 첫 줄(session_meta)의 cwd. 서브에이전트 스레드면 None.
fn head_meta_cwd(path: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    BufReader::new(file).read_line(&mut line).ok()?;
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    let payload = value.get("payload")?;
    if payload.get("thread_source").and_then(Value::as_str) == Some("subagent") {
        return None;
    }
    payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_string)
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
}
