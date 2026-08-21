// src-tauri/src/session_log/agent_transcript/claude.rs
//
// Claude Code 전사 소스. `<설정 디렉터리>/projects/<슬러그>/<sessionId>.jsonl`.
//
// 세션 정보는 훅이 채우는 `claude-resume.json`(ClaudeResumeStore)에서 온다 --
// 이미 에이전트별 최신 native 세션을 추적하고 있으므로 새 배선이 필요 없다.
// 전사 파일을 찾는 순서:
//
//   1) 훅이 실어 온 `transcript_path` -- CLI가 직접 알려 준 절대 경로라
//      `CLAUDE_CONFIG_DIR`을 어디로 옮겼든 맞는다. 정상 경로는 여기서 끝난다.
//   2) cwd 슬러그 추측(한 번의 stat)
//   3) 프로젝트 디렉터리 한 번 훑기
//
// 2·3의 기준 루트도 `CLAUDE_CONFIG_DIR`을 존중한다(agent_paths). 찾은 경로는
// 세션 ID별로 캐시한다.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use super::{
    clamp_value, compact_json_brief, AgentSessionLookup, ItemRole, TranscriptItem, TranscriptSource,
};

/// `<CLAUDE_CONFIG_DIR 또는 ~/.claude>/projects`. 홈도 오버라이드도 없으면 None
/// (그 경우 훅이 알려 준 절대 경로에만 의존한다).
pub fn default_projects_root() -> Option<PathBuf> {
    Some(crate::agent_paths::claude_config_dir_from_env()?.join("projects"))
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
    /// 추측·훑기의 기준 루트. 설정 디렉터리를 못 정했으면 None이고, 그때는
    /// 훅이 알려 준 절대 경로만 쓴다.
    projects_root: Option<PathBuf>,
    lookup: Arc<dyn AgentSessionLookup>,
    /// sessionId → 확인된 전사 경로. 디렉터리 훑기를 세션마다 한 번으로 묶는다.
    resolved: HashMap<String, PathBuf>,
}

impl ClaudeSource {
    pub fn new(projects_root: Option<PathBuf>, lookup: Arc<dyn AgentSessionLookup>) -> Self {
        Self {
            projects_root,
            lookup,
            resolved: HashMap::new(),
        }
    }

    fn find(&mut self, session_id: &str, cwd: &str, hook_path: Option<&str>) -> Option<PathBuf> {
        if let Some(hit) = self.resolved.get(session_id) {
            if hit.exists() {
                return Some(hit.clone());
            }
            self.resolved.remove(session_id);
        }
        // 1) 훅이 알려 준 경로. 설정 디렉터리 위치와 무관하게 항상 맞는다.
        if let Some(path) = hook_path.map(PathBuf::from).filter(|p| p.is_file()) {
            self.resolved.insert(session_id.to_string(), path.clone());
            return Some(path);
        }
        // 옛 `claude-resume.json`(transcriptPath 없이 기록된 항목)이나 훅이 경로를
        // 안 실어 준 경우를 위한 폴백 -- 루트를 알 때만 가능하다.
        let root = self.projects_root.clone()?;
        let file = format!("{session_id}.jsonl");
        // 2) cwd 슬러그로 바로 찾기.
        let guess = root.join(slug(cwd)).join(&file);
        if guess.is_file() {
            self.resolved.insert(session_id.to_string(), guess.clone());
            return Some(guess);
        }
        // 3) 빗나갔으면(리줌 후 cwd 변경, 슬러그 규칙 변화) 한 번 훑는다.
        for entry in std::fs::read_dir(&root).ok()?.flatten() {
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
        let session = self.lookup.latest_session(agent_id)?;
        // 훅이 실어 온 cwd가 있으면 그쪽이 정확하다(세션 중 폴더가 바뀌었을 수 있다).
        let cwd = session.cwd.unwrap_or_else(|| cwd.to_string());
        self.find(&session.session_id, &cwd, session.transcript_path.as_deref())
    }

    fn parse(&self, raw: &str) -> Vec<TranscriptItem> {
        let Ok(v) = serde_json::from_str::<Value>(raw) else {
            return Vec::new();
        };
        parse_entry(&v)
    }
}

/// JSONL 한 항목 → 채팅 항목들. 대화(사용자/에이전트/도구)만 남기고 나머지
/// 메타 항목(mode, ai-title, attachment, file-history-snapshot …)은 버린다.
fn parse_entry(v: &Value) -> Vec<TranscriptItem> {
    let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
    if !matches!(kind, "user" | "assistant") {
        return Vec::new();
    }
    // 훅·시스템이 끼워 넣은 항목은 대화가 아니다.
    if v.get("isMeta").and_then(Value::as_bool).unwrap_or(false) {
        return Vec::new();
    }
    // 서브에이전트(sidechain) 대화는 표식을 붙여 구분한다.
    let side = v
        .get("isSidechain")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let role = if kind == "user" {
        ItemRole::User
    } else {
        ItemRole::Assistant
    };

    let Some(message) = v.get("message") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match message.get("content") {
        // 빈 문자열은 로그에서도 줄이 되지 않았다(clamp가 빈 벡터) — 항목도
        // 만들지 않아 빈 버블이 생기지 않게 한다.
        Some(Value::String(text)) if !text.is_empty() => {
            out.push(TranscriptItem::speech(role, text.clone()).with_sidechain(side));
        }
        Some(Value::Array(blocks)) => {
            for b in blocks {
                out.extend(parse_block(b, role, side));
            }
        }
        _ => {}
    }
    out
}

fn parse_block(b: &Value, role: ItemRole, side: bool) -> Vec<TranscriptItem> {
    match b.get("type").and_then(Value::as_str).unwrap_or("") {
        "text" => {
            let text = b.get("text").and_then(Value::as_str).unwrap_or("");
            if text.trim().is_empty() {
                return Vec::new();
            }
            vec![TranscriptItem::speech(role, text).with_sidechain(side)]
        }
        "tool_use" => {
            let name = b.get("name").and_then(Value::as_str).unwrap_or("도구");
            let brief = tool_brief(b.get("input"));
            vec![TranscriptItem::tool_use(Some(name.to_string()), brief).with_sidechain(side)]
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
            vec![TranscriptItem::tool_result(body, failed).with_sidechain(side)]
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
    fn latest_session(&self, agent_id: &str) -> Option<super::AgentSessionSnapshot> {
        let all = self.load_all();
        let entry = all.get(agent_id)?;
        Some(super::AgentSessionSnapshot {
            session_id: entry.session_id.clone(),
            cwd: entry.cwd.clone(),
            transcript_path: entry.transcript_path.clone(),
        })
    }
}

/// Claude 소스를 만든다. 기준 루트(`CLAUDE_CONFIG_DIR` 또는 `~/.claude`) 아래
/// `projects/`가 없어도 소스는 만든다 -- 훅이 실어 오는 절대 경로만으로도
/// 전사를 따라갈 수 있고, 오히려 그 상황(설정 디렉터리를 옮겨 쓰는데 앱이
/// 아직 그 env를 못 본 경우)이 이 소스가 가장 필요한 순간이다. 조회기가 이
/// 캐릭터의 세션을 모르면 `locate`가 즉시 None이라 비용도 없다.
pub fn source(lookup: Arc<dyn AgentSessionLookup>) -> Option<Box<dyn TranscriptSource>> {
    let root = default_projects_root().filter(|root| root.is_dir());
    Some(Box::new(ClaudeSource::new(root, lookup)))
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::AgentSessionSnapshot;

    struct FixedLookup(Option<AgentSessionSnapshot>);
    impl AgentSessionLookup for FixedLookup {
        fn latest_session(&self, _agent_id: &str) -> Option<AgentSessionSnapshot> {
            self.0.clone()
        }
    }

    fn snapshot(session_id: &str, cwd: Option<&str>) -> AgentSessionSnapshot {
        AgentSessionSnapshot {
            session_id: session_id.to_string(),
            cwd: cwd.map(str::to_string),
            transcript_path: None,
        }
    }

    /// 세션 로그가 실제로 받는 줄. **파서 분리 후에도 이 문자열이 그대로여야
    /// 한다** — 아래 케이스들이 그 계약의 핀이다.
    fn render(raw: &str) -> Vec<String> {
        super::super::format_items(&parse_entry(&serde_json::from_str(raw).unwrap()))
    }

    fn parse(raw: &str) -> Vec<TranscriptItem> {
        parse_entry(&serde_json::from_str(raw).unwrap())
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

    /// 채팅 뷰가 받는 구조화 항목(같은 픽스처의 다른 표현).
    #[test]
    fn structured_items_carry_role_kind_and_tool_name() {
        let items = parse(
            r#"{"type":"assistant","message":{"role":"assistant","content":[
                {"type":"thinking","thinking":"안 남을 것","signature":"AAAA"},
                {"type":"text","text":"확인하겠습니다."},
                {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git status","description":"상태"}}
            ]}}"#,
        );
        assert_eq!(items.len(), 2, "thinking은 항목이 아니다: {items:?}");
        assert_eq!(items[0].role, ItemRole::Assistant);
        assert_eq!(items[0].kind, super::super::ItemKind::Text);
        assert_eq!(items[0].text, "확인하겠습니다.");
        assert_eq!(items[1].kind, super::super::ItemKind::ToolUse);
        assert_eq!(items[1].tool_name.as_deref(), Some("Bash"));
        assert_eq!(items[1].text, "git status");
        assert!(!items[1].is_error);

        let err = parse(
            r#"{"type":"user","message":{"role":"user","content":[
                {"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"boom"}]}}"#,
        );
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].kind, super::super::ItemKind::ToolResult);
        assert!(err[0].is_error);
        assert_eq!(err[0].text, "boom");

        let side = parse(
            r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant","content":[
                {"type":"text","text":"서브에이전트 응답"}]}}"#,
        );
        assert!(side[0].sidechain);

        // 도구 인자의 블롭은 항목 단계에서 이미 생략된다.
        let blob = parse(&format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","content":[
                {{"type":"tool_use","name":"Task","input":{{"task_name":"x","message":"{}"}}}}]}}}}"#,
            "g".repeat(500)
        ));
        assert!(blob[0].text.contains("(생략 500자)"), "{:?}", blob[0].text);
        assert!(!blob[0].text.contains("gggg"));
    }

    /// 와이어 모양(웹 클라이언트의 `protocol.ts` 미러).
    #[test]
    fn item_serializes_as_camel_case() {
        let json = serde_json::to_string(&TranscriptItem::tool_use(
            Some("Bash".into()),
            "ls",
        ))
        .unwrap();
        assert!(json.contains("\"role\":\"assistant\""), "{json}");
        assert!(json.contains("\"kind\":\"tool_use\""), "{json}");
        assert!(json.contains("\"toolName\":\"Bash\""), "{json}");
        assert!(json.contains("\"isError\":false"), "{json}");
    }

    #[test]
    fn unparsable_line_is_ignored() {
        let src = ClaudeSource::new(
            Some(PathBuf::from("/nowhere")),
            Arc::new(FixedLookup(None)),
        );
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

        let lookup = Arc::new(FixedLookup(Some(snapshot("sess-1", Some(cwd)))));
        let mut src = ClaudeSource::new(Some(dir.clone()), lookup);
        assert_eq!(src.locate("a1", cwd), Some(by_slug.join("sess-1.jsonl")));

        // 슬러그가 안 맞는 위치에 있어도(리줌 후 폴더 변경) 훑어서 찾아낸다.
        let odd = dir.join("-somewhere-else");
        std::fs::create_dir_all(&odd).unwrap();
        std::fs::write(odd.join("sess-2.jsonl"), b"").unwrap();
        let lookup2 = Arc::new(FixedLookup(Some(snapshot("sess-2", None))));
        let mut src2 = ClaudeSource::new(Some(dir.clone()), lookup2);
        assert_eq!(src2.locate("a1", cwd), Some(odd.join("sess-2.jsonl")));

        // 세션 ID를 모르면 아무것도 안 한다.
        let mut blind = ClaudeSource::new(Some(dir.clone()), Arc::new(FixedLookup(None)));
        assert_eq!(blind.locate("a1", cwd), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 훅이 알려 준 transcript_path는 기준 루트 밖(=CLAUDE_CONFIG_DIR을 옮긴
    /// 경우)에 있어도 그대로 쓰여야 한다. 루트를 아예 모를 때도 마찬가지다.
    #[test]
    fn locate_prefers_the_hook_reported_path_outside_the_root() {
        let dir = std::env::temp_dir().join(format!(
            "agent-office-claude-src-hookpath-{}",
            uuid::Uuid::new_v4()
        ));
        let elsewhere = dir.join("custom-config/projects/-w-proj");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let transcript = elsewhere.join("sess-9.jsonl");
        std::fs::write(&transcript, b"").unwrap();

        let with_hook = |root: Option<PathBuf>| {
            let lookup = Arc::new(FixedLookup(Some(AgentSessionSnapshot {
                session_id: "sess-9".to_string(),
                cwd: Some("/w/proj".to_string()),
                transcript_path: Some(transcript.to_string_lossy().into_owned()),
            })));
            ClaudeSource::new(root, lookup)
        };

        // 기본 루트(~/.claude/projects 상당)에는 이 세션 파일이 없다.
        let home_root = dir.join("home-claude/projects");
        std::fs::create_dir_all(&home_root).unwrap();
        let mut src = with_hook(Some(home_root));
        assert_eq!(src.locate("a1", "/w/proj"), Some(transcript.clone()));

        // 루트를 못 정한 환경(HOME 부재 등)에서도 훅 경로만으로 따라간다.
        let mut rootless = with_hook(None);
        assert_eq!(rootless.locate("a1", "/w/proj"), Some(transcript.clone()));

        // 훅 경로가 사라졌으면(파일 삭제) 폴백으로 내려간다 — 루트가 없으면 None.
        std::fs::remove_file(&transcript).unwrap();
        let mut gone = with_hook(None);
        assert_eq!(gone.locate("a1", "/w/proj"), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 기준 루트는 CLAUDE_CONFIG_DIR을 존중해야 한다(빈 값은 미설정 취급).
    #[test]
    fn projects_root_follows_the_config_dir_override() {
        assert_eq!(
            crate::agent_paths::claude_config_dir(
                std::path::Path::new("/home/u"),
                Some("/data/claude")
            )
            .join("projects"),
            PathBuf::from("/data/claude/projects")
        );
        assert_eq!(
            crate::agent_paths::claude_config_dir(std::path::Path::new("/home/u"), None)
                .join("projects"),
            PathBuf::from("/home/u/.claude/projects")
        );
    }
}
