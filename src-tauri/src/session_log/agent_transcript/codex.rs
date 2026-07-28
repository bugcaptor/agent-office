// src-tauri/src/session_log/agent_transcript/codex.rs
//
// Codex 전사 소스. `<CODEX_HOME|~/.codex>/sessions/<YYYY>/<MM>/<DD>/rollout-*.jsonl`.
//
// Claude와 달리 Codex 세션 ID를 훅으로 받는 경로가 없다. 대신 rollout 파일
// 첫 줄(`session_meta`)에 `cwd`와 `thread_source`가 들어 있어 그것으로 고른다:
//
//   - `cwd`가 이 세션의 작업 폴더와 같고,
//   - `thread_source`가 `subagent`가 아니고(서브에이전트 스레드는 부모에 딸린 별도 파일),
//   - 최근에 쓰인(=살아 있는) 파일 중 가장 새 것.
//
// 한계: 같은 폴더에서 agent-office 밖의 터미널로 codex를 돌리면 그 대화가 이
// 캐릭터 로그에 붙을 수 있다. 캐릭터-폴더가 1:1인 이 앱의 사용 방식에서는
// 실질적으로 드물고, 대신 세션 ID 추적 배선 없이 동작한다.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde_json::Value;

use super::{block, clamp_value, compact_json_brief, TranscriptSource};

/// 이보다 오래 쓰이지 않은 rollout은 "이미 끝난 세션"으로 보고 붙지 않는다.
const LIVE_WINDOW: Duration = Duration::from_secs(30 * 60);
/// 날짜 디렉터리를 다시 훑는 최소 간격. claude와 달리 세션 ID가 없어 매번
/// 디렉터리를 읽어야 하는데, 세션마다 2초 틱으로 그러면 낭비다. 새 codex 세션이
/// 붙기까지 이만큼 늦어질 뿐이다.
const RESCAN_EVERY: Duration = Duration::from_secs(10);

/// `<CODEX_HOME 또는 ~/.codex>/sessions`. Claude의 `CLAUDE_CONFIG_DIR`과 같은
/// 사례 -- 홈만 보고 조립하면 CODEX_HOME을 옮겨 쓰는 환경에서 세션 로그에
/// Codex 대화가 통째로 빠진다(agent_paths).
pub fn default_sessions_root() -> Option<PathBuf> {
    Some(crate::agent_paths::codex_home_from_env()?.join("sessions"))
}

pub struct CodexSource {
    sessions_root: PathBuf,
    /// rollout 경로 → 그 파일이 붙을 수 있는 cwd(첫 줄 파싱 결과).
    /// 서브에이전트 스레드/파싱 실패는 `None`으로 기억해 다시 읽지 않는다.
    probed: HashMap<PathBuf, Option<String>>,
    /// 마지막 디렉터리 훑기(시각, 그때의 cwd, 결과). 훑기 사이에는 같은 cwd에
    /// 대해서만 이 결과를 재사용한다.
    last_scan: Option<(SystemTime, String, Option<PathBuf>)>,
}

impl CodexSource {
    pub fn new(sessions_root: PathBuf) -> Self {
        Self {
            sessions_root,
            probed: HashMap::new(),
            last_scan: None,
        }
    }

    /// 최근 이틀치 날짜 디렉터리(`<root>/YYYY/MM/DD`). 자정을 넘겨도 어제 시작한
    /// 세션을 계속 따라가야 하므로 이틀을 본다.
    fn day_dirs(&self, now: SystemTime) -> Vec<PathBuf> {
        let today = chrono::DateTime::<chrono::Local>::from(now);
        (0..2)
            .filter_map(|back| today.checked_sub_signed(chrono::Duration::days(back)))
            .map(|d| {
                self.sessions_root
                    .join(d.format("%Y").to_string())
                    .join(d.format("%m").to_string())
                    .join(d.format("%d").to_string())
            })
            .collect()
    }

    /// 첫 줄의 `session_meta`에서 cwd를 뽑는다. 사용자 스레드가 아니면 None.
    fn probe(&mut self, path: &PathBuf) -> Option<String> {
        if let Some(cached) = self.probed.get(path) {
            return cached.clone();
        }
        let cwd = read_first_line(path).and_then(|line| {
            let v: Value = serde_json::from_str(&line).ok()?;
            if v.get("type").and_then(Value::as_str) != Some("session_meta") {
                return None;
            }
            let payload = v.get("payload")?;
            if payload.get("thread_source").and_then(Value::as_str) == Some("subagent") {
                return None;
            }
            payload
                .get("cwd")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
        self.probed.insert(path.clone(), cwd.clone());
        cwd
    }

    fn newest_for(&mut self, cwd: &str, now: SystemTime) -> Option<PathBuf> {
        let mut best: Option<(SystemTime, PathBuf)> = None;
        for dir in self.day_dirs(now) {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
                    continue;
                };
                // 오래 조용한 파일은 첫 줄을 읽어 보지도 않는다(디스크 절약).
                if now
                    .duration_since(modified)
                    .map(|age| age > LIVE_WINDOW)
                    .unwrap_or(false)
                {
                    continue;
                }
                if self.probe(&path).as_deref() != Some(cwd) {
                    continue;
                }
                if best.as_ref().is_none_or(|(t, _)| modified > *t) {
                    best = Some((modified, path));
                }
            }
        }
        best.map(|(_, p)| p)
    }
}

fn read_first_line(path: &PathBuf) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    BufReader::new(file).read_line(&mut line).ok()?;
    Some(line)
}

impl TranscriptSource for CodexSource {
    fn label(&self) -> &'static str {
        "codex"
    }

    fn locate(&mut self, _agent_id: &str, cwd: &str) -> Option<PathBuf> {
        if cwd.is_empty() {
            return None;
        }
        let now = SystemTime::now();
        if let Some((at, scanned_cwd, cached)) = &self.last_scan {
            if scanned_cwd == cwd && now.duration_since(*at).unwrap_or_default() < RESCAN_EVERY {
                return cached.clone();
            }
        }
        let found = self.newest_for(cwd, now);
        self.last_scan = Some((now, cwd.to_string(), found.clone()));
        found
    }

    fn render(&self, raw: &str) -> Vec<String> {
        let Ok(v) = serde_json::from_str::<Value>(raw) else {
            return Vec::new();
        };
        render_entry(&v)
    }
}

fn render_entry(v: &Value) -> Vec<String> {
    let Some(payload) = v.get("payload") else {
        return Vec::new();
    };
    let sub = payload.get("type").and_then(Value::as_str).unwrap_or("");
    let text = |key: &str| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };

    match (v.get("type").and_then(Value::as_str).unwrap_or(""), sub) {
        // 사용자·에이전트 발화는 event_msg 쪽이 평문이다(response_item의 같은
        // 내용은 암호화 블롭이라 쓸 수 없다).
        ("event_msg", "user_message") => block("▶ 사용자:", &text("message")),
        ("event_msg", "agent_message") => block("⏺ 에이전트:", &text("message")),
        ("event_msg", "patch_apply_end") => block("⚒ 패치 적용:", &text("stdout")),
        ("event_msg", "sub_agent_activity") => {
            let path = text("agent_path");
            let kind = text("kind");
            vec![format!("⤷ 서브에이전트 {path}: {kind}")]
        }
        ("response_item", "custom_tool_call") | ("response_item", "function_call") => {
            let name = text("name");
            // function_call의 arguments는 JSON 문자열이다. 서브에이전트
            // spawn 인자에는 암호화 블롭이 실려 오므로 걸러서 넣는다.
            let input = if sub == "function_call" {
                let raw = text("arguments");
                serde_json::from_str::<Value>(&raw)
                    .map(|v| compact_json_brief(&v))
                    .unwrap_or(raw)
            } else {
                text("input")
            };
            block(&format!("⚒ {name}:"), &input)
        }
        ("response_item", "custom_tool_call_output")
        | ("response_item", "function_call_output") => {
            let body = flatten_output(payload.get("output"));
            if body.trim().is_empty() {
                return Vec::new();
            }
            block("⇤ 결과:", &body)
        }
        // 나머지(reasoning=암호화, token_count, world_state, turn_context,
        // task_started/complete=중복)는 남기지 않는다.
        _ => Vec::new(),
    }
}

fn flatten_output(output: Option<&Value>) -> String {
    match output {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(Value::as_str).map(str::to_string))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => clamp_value(&compact_json_brief(other)).join("\n"),
        None => String::new(),
    }
}

/// 홈의 기본 위치에서 Codex 소스를 만든다. 세션 디렉터리가 없으면 None.
pub fn source() -> Option<Box<dyn TranscriptSource>> {
    let root = default_sessions_root()?;
    if !root.is_dir() {
        return None;
    }
    Some(Box::new(CodexSource::new(root)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(raw: &str) -> Vec<String> {
        render_entry(&serde_json::from_str(raw).unwrap())
    }

    #[test]
    fn user_and_agent_messages_are_rendered() {
        assert_eq!(
            render(r#"{"type":"event_msg","payload":{"type":"user_message","message":"성운 만들어줘"}}"#),
            vec!["▶ 사용자: 성운 만들어줘"]
        );
        assert_eq!(
            render(r#"{"type":"event_msg","payload":{"type":"agent_message","message":"만들겠습니다."}}"#),
            vec!["⏺ 에이전트: 만들겠습니다."]
        );
    }

    #[test]
    fn tool_calls_and_outputs_are_rendered() {
        assert_eq!(
            render(
                r#"{"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","input":"git status"}}"#
            ),
            vec!["⚒ exec: git status"]
        );
        assert_eq!(
            render(
                r#"{"type":"response_item","payload":{"type":"custom_tool_call_output","output":[{"type":"input_text","text":"clean"}]}}"#
            ),
            vec!["⇤ 결과: clean"]
        );
    }

    #[test]
    fn noise_entries_are_dropped() {
        for raw in [
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{}}}"#,
            r#"{"type":"response_item","payload":{"type":"reasoning","encrypted_content":"gAAA"}}"#,
            r#"{"type":"world_state","payload":{"full":true}}"#,
            r#"{"type":"turn_context","payload":{"cwd":"/w"}}"#,
            r#"{"type":"event_msg","payload":{"type":"task_complete","last_agent_message":"중복"}}"#,
        ] {
            assert!(render(raw).is_empty(), "{raw}");
        }
    }

    #[test]
    fn locate_picks_the_newest_live_user_thread_for_this_cwd() {
        let root = std::env::temp_dir().join(format!(
            "agent-office-codex-src-test-{}",
            uuid::Uuid::new_v4()
        ));
        let now = SystemTime::now();
        let day = chrono::DateTime::<chrono::Local>::from(now);
        let dir = root
            .join(day.format("%Y").to_string())
            .join(day.format("%m").to_string())
            .join(day.format("%d").to_string());
        std::fs::create_dir_all(&dir).unwrap();

        let meta = |cwd: &str, source: &str| {
            format!(
                r#"{{"type":"session_meta","payload":{{"cwd":"{cwd}","thread_source":"{source}"}}}}"#
            )
        };
        let mine = dir.join("rollout-mine.jsonl");
        std::fs::write(&mine, format!("{}\n", meta("/w/proj", "user"))).unwrap();
        std::fs::write(
            dir.join("rollout-other-folder.jsonl"),
            format!("{}\n", meta("/w/elsewhere", "user")),
        )
        .unwrap();
        std::fs::write(
            dir.join("rollout-subagent.jsonl"),
            format!("{}\n", meta("/w/proj", "subagent")),
        )
        .unwrap();

        let mut src = CodexSource::new(root.clone());
        assert_eq!(src.locate("a1", "/w/proj"), Some(mine));
        assert_eq!(src.locate("a1", "/w/nothing-here"), None);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn stale_rollout_is_not_attached() {
        let root = std::env::temp_dir().join(format!(
            "agent-office-codex-stale-test-{}",
            uuid::Uuid::new_v4()
        ));
        let now = SystemTime::now();
        let day = chrono::DateTime::<chrono::Local>::from(now);
        let dir = root
            .join(day.format("%Y").to_string())
            .join(day.format("%m").to_string())
            .join(day.format("%d").to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout-old.jsonl");
        std::fs::write(
            &path,
            "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/w/proj\",\"thread_source\":\"user\"}}\n",
        )
        .unwrap();

        // 파일은 지금 쓰였지만 "현재 시각"을 한참 뒤로 잡으면 창을 벗어난다.
        let mut src = CodexSource::new(root.clone());
        let later = now + LIVE_WINDOW + Duration::from_secs(60);
        assert_eq!(src.newest_for("/w/proj", later), None);
        std::fs::remove_dir_all(&root).ok();
    }
}
