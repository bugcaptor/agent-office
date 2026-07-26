// src-tauri/src/session_log/agent_transcript/claude.rs
//
// Claude Code 전사 소스. `~/.claude/projects/<슬러그>/<sessionId>.jsonl`.
//
// 세션 ID는 훅이 채우는 `claude-resume.json`(ClaudeResumeStore)에서 온다 --
// 이미 에이전트별 최신 native 세션을 추적하고 있으므로 새 배선이 필요 없다.
// 경로는 cwd 슬러그를 먼저 추측하고(한 번의 stat), 빗나가면 프로젝트 디렉터리를
// 한 번 훑어 찾는다. 찾은 경로는 세션 ID별로 캐시한다.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use super::{block, clamp_value, compact_json_brief, AgentSessionLookup, TranscriptSource};

/// `~/.claude/projects`. 홈을 못 찾으면 None(소스가 만들어지지 않는다).
pub fn default_projects_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".claude").join("projects"))
}

/// cwd → Claude Code의 프로젝트 디렉터리 이름. 영숫자와 `-`만 남기고 나머지는
/// 전부 `-`로 바꾸는 규칙(`/Users/me/dev/app` → `-Users-me-dev-app`).
fn slug(cwd: &str) -> String {
    cwd.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

pub struct ClaudeSource {
    projects_root: PathBuf,
    lookup: Arc<dyn AgentSessionLookup>,
    /// sessionId → 확인된 전사 경로. 디렉터리 훑기를 세션마다 한 번으로 묶는다.
    resolved: HashMap<String, PathBuf>,
}

impl ClaudeSource {
    pub fn new(projects_root: PathBuf, lookup: Arc<dyn AgentSessionLookup>) -> Self {
        Self {
            projects_root,
            lookup,
            resolved: HashMap::new(),
        }
    }

    fn find(&mut self, session_id: &str, cwd: &str) -> Option<PathBuf> {
        if let Some(hit) = self.resolved.get(session_id) {
            if hit.exists() {
                return Some(hit.clone());
            }
            self.resolved.remove(session_id);
        }
        let file = format!("{session_id}.jsonl");
        // 1) cwd 슬러그로 바로 찾기.
        let guess = self.projects_root.join(slug(cwd)).join(&file);
        if guess.is_file() {
            self.resolved.insert(session_id.to_string(), guess.clone());
            return Some(guess);
        }
        // 2) 빗나갔으면(리줌 후 cwd 변경, 슬러그 규칙 변화) 한 번 훑는다.
        for entry in std::fs::read_dir(&self.projects_root).ok()?.flatten() {
            let candidate = entry.path().join(&file);
            if candidate.is_file() {
                self.resolved
                    .insert(session_id.to_string(), candidate.clone());
                return Some(candidate);
            }
        }
        None
    }
}

impl TranscriptSource for ClaudeSource {
    fn label(&self) -> &'static str {
        "claude"
    }

    fn locate(&mut self, agent_id: &str, cwd: &str) -> Option<PathBuf> {
        let (session_id, hook_cwd) = self.lookup.latest_session(agent_id)?;
        // 훅이 실어 온 cwd가 있으면 그쪽이 정확하다(세션 중 폴더가 바뀌었을 수 있다).
        let cwd = hook_cwd.unwrap_or_else(|| cwd.to_string());
        self.find(&session_id, &cwd)
    }

    fn render(&self, raw: &str) -> Vec<String> {
        let Ok(v) = serde_json::from_str::<Value>(raw) else {
            return Vec::new();
        };
        render_entry(&v)
    }
}

/// JSONL 한 항목 → 로그 줄들. 대화(사용자/에이전트/도구)만 남기고 나머지
/// 메타 항목(mode, ai-title, attachment, file-history-snapshot …)은 버린다.
fn render_entry(v: &Value) -> Vec<String> {
    let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
    if !matches!(kind, "user" | "assistant") {
        return Vec::new();
    }
    // 훅·시스템이 끼워 넣은 항목은 대화가 아니다.
    if v.get("isMeta").and_then(Value::as_bool).unwrap_or(false) {
        return Vec::new();
    }
    // 서브에이전트(sidechain) 대화는 들여쓰기 표식을 붙여 구분한다.
    let side = v
        .get("isSidechain")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mark = |glyph: &str| {
        if side {
            format!("⤷ {glyph}")
        } else {
            glyph.to_string()
        }
    };

    let Some(message) = v.get("message") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match message.get("content") {
        Some(Value::String(text)) => {
            if kind == "user" {
                out.extend(block(&mark("▶ 사용자:"), text));
            } else {
                out.extend(block(&mark("⏺ 에이전트:"), text));
            }
        }
        Some(Value::Array(blocks)) => {
            for b in blocks {
                out.extend(render_block(b, kind, &mark));
            }
        }
        _ => {}
    }
    out
}

fn render_block(b: &Value, kind: &str, mark: &dyn Fn(&str) -> String) -> Vec<String> {
    match b.get("type").and_then(Value::as_str).unwrap_or("") {
        "text" => {
            let text = b.get("text").and_then(Value::as_str).unwrap_or("");
            if text.trim().is_empty() {
                return Vec::new();
            }
            let glyph = if kind == "user" {
                mark("▶ 사용자:")
            } else {
                mark("⏺ 에이전트:")
            };
            block(&glyph, text)
        }
        "tool_use" => {
            let name = b.get("name").and_then(Value::as_str).unwrap_or("도구");
            let brief = tool_brief(b.get("input"));
            block(&mark(&format!("⚒ {name}:")), &brief)
        }
        "tool_result" => {
            let body = flatten_content(b.get("content"));
            if body.trim().is_empty() {
                return Vec::new();
            }
            let failed = b
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let glyph = if failed { "⇤ 결과(오류):" } else { "⇤ 결과:" };
            block(&mark(glyph), &body)
        }
        // thinking/signature/이미지 등은 남기지 않는다 -- 서명 블롭과 base64는
        // 로그를 못 읽게 만들고, 사고 과정은 원본 JSONL에 그대로 있다.
        _ => Vec::new(),
    }
}

/// 도구 입력을 한눈에 보이는 한 덩어리로. 알아보는 키가 있으면 그 값만,
/// 없으면 압축 JSON을 쓴다(둘 다 상한에서 잘린다).
fn tool_brief(input: Option<&Value>) -> String {
    let Some(Value::Object(map)) = input else {
        return input.map(compact_json_brief).unwrap_or_default();
    };
    for key in [
        "command",
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "prompt",
        "skill",
    ] {
        if let Some(Value::String(s)) = map.get(key) {
            if !s.trim().is_empty() {
                return s.clone();
            }
        }
    }
    compact_json_brief(&Value::Object(map.clone()))
}

/// tool_result의 content는 문자열이거나 `[{type:"text", text:…}]`다.
fn flatten_content(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|i| match i.get("type").and_then(Value::as_str) {
                Some("text") => i.get("text").and_then(Value::as_str).map(str::to_string),
                Some("image") => Some("(이미지)".to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => clamp_value(&compact_json_brief(other)).join("\n"),
        None => String::new(),
    }
}

/// `ClaudeResumeStore`를 조회기로 그대로 쓴다 -- 훅이 매번 갱신하므로 세션
/// 도중 리줌으로 ID가 바뀌어도 다음 틱에 새 파일로 따라간다.
impl AgentSessionLookup for crate::persistence::claude_resume_store::ClaudeResumeStore {
    fn latest_session(&self, agent_id: &str) -> Option<(String, Option<String>)> {
        let all = self.load_all();
        let entry = all.get(agent_id)?;
        Some((entry.session_id.clone(), entry.cwd.clone()))
    }
}

/// 홈의 기본 위치에서 Claude 소스를 만든다. 프로젝트 디렉터리가 아예 없으면
/// (Claude Code를 안 쓰는 환경) None.
pub fn source(lookup: Arc<dyn AgentSessionLookup>) -> Option<Box<dyn TranscriptSource>> {
    let root = default_projects_root()?;
    if !root.is_dir() {
        return None;
    }
    Some(Box::new(ClaudeSource::new(root, lookup)))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedLookup(Option<(String, Option<String>)>);
    impl AgentSessionLookup for FixedLookup {
        fn latest_session(&self, _agent_id: &str) -> Option<(String, Option<String>)> {
            self.0.clone()
        }
    }

    fn render(raw: &str) -> Vec<String> {
        render_entry(&serde_json::from_str(raw).unwrap())
    }

    #[test]
    fn slug_matches_claude_code_rule() {
        assert_eq!(slug("/Users/me/dev/app"), "-Users-me-dev-app");
        assert_eq!(
            slug("/Users/me/dev/app/.claude/worktrees/fix"),
            "-Users-me-dev-app--claude-worktrees-fix"
        );
    }

    #[test]
    fn user_prompt_is_rendered() {
        let out = render(
            r#"{"type":"user","isSidechain":false,"message":{"role":"user","content":"로그를 고쳐줘"}}"#,
        );
        assert_eq!(out, vec!["▶ 사용자: 로그를 고쳐줘"]);
    }

    #[test]
    fn assistant_text_and_tool_use_are_rendered_thinking_is_not() {
        let out = render(
            r#"{"type":"assistant","message":{"role":"assistant","content":[
                {"type":"thinking","thinking":"안 남을 것","signature":"AAAA"},
                {"type":"text","text":"확인하겠습니다."},
                {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git status","description":"상태"}}
            ]}}"#,
        );
        assert_eq!(
            out,
            vec!["⏺ 에이전트: 확인하겠습니다.", "⚒ Bash: git status"]
        );
    }

    #[test]
    fn tool_result_and_error_are_marked() {
        let ok = render(
            r#"{"type":"user","message":{"role":"user","content":[
                {"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"clean"}]}]}}"#,
        );
        assert_eq!(ok, vec!["⇤ 결과: clean"]);

        let err = render(
            r#"{"type":"user","message":{"role":"user","content":[
                {"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"boom"}]}}"#,
        );
        assert_eq!(err, vec!["⇤ 결과(오류): boom"]);
    }

    #[test]
    fn sidechain_entries_are_marked() {
        let out = render(
            r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant","content":[
                {"type":"text","text":"서브에이전트 응답"}]}}"#,
        );
        assert_eq!(out, vec!["⤷ ⏺ 에이전트: 서브에이전트 응답"]);
    }

    #[test]
    fn meta_entries_are_dropped() {
        for raw in [
            r#"{"type":"mode","mode":"normal"}"#,
            r#"{"type":"ai-title","aiTitle":"제목"}"#,
            r#"{"type":"attachment","attachment":{"type":"skill_listing","content":"x"}}"#,
            r#"{"type":"file-history-snapshot","snapshot":{}}"#,
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"훅 잡음"}}"#,
        ] {
            assert!(render(raw).is_empty(), "{raw}");
        }
    }

    #[test]
    fn unparsable_line_is_ignored() {
        let src = ClaudeSource::new(PathBuf::from("/nowhere"), Arc::new(FixedLookup(None)));
        assert!(src.render("not json").is_empty());
        assert!(src.render("").is_empty());
    }

    #[test]
    fn locate_finds_the_file_by_slug_then_by_scan() {
        let dir = std::env::temp_dir().join(format!(
            "agent-office-claude-src-test-{}",
            uuid::Uuid::new_v4()
        ));
        let cwd = "/w/proj";
        let by_slug = dir.join(slug(cwd));
        std::fs::create_dir_all(&by_slug).unwrap();
        std::fs::write(by_slug.join("sess-1.jsonl"), b"").unwrap();

        let lookup = Arc::new(FixedLookup(Some((
            "sess-1".to_string(),
            Some(cwd.to_string()),
        ))));
        let mut src = ClaudeSource::new(dir.clone(), lookup);
        assert_eq!(src.locate("a1", cwd), Some(by_slug.join("sess-1.jsonl")));

        // 슬러그가 안 맞는 위치에 있어도(리줌 후 폴더 변경) 훑어서 찾아낸다.
        let odd = dir.join("-somewhere-else");
        std::fs::create_dir_all(&odd).unwrap();
        std::fs::write(odd.join("sess-2.jsonl"), b"").unwrap();
        let lookup2 = Arc::new(FixedLookup(Some(("sess-2".to_string(), None))));
        let mut src2 = ClaudeSource::new(dir.clone(), lookup2);
        assert_eq!(src2.locate("a1", cwd), Some(odd.join("sess-2.jsonl")));

        // 세션 ID를 모르면 아무것도 안 한다.
        let mut blind = ClaudeSource::new(dir.clone(), Arc::new(FixedLookup(None)));
        assert_eq!(blind.locate("a1", cwd), None);

        std::fs::remove_dir_all(&dir).ok();
    }
}
