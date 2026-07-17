use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObserverProvider {
    Claude,
    Codex,
}

impl ObserverProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObserverEvent {
    Prompt {
        text: Option<String>,
    },
    Tool,
    SubStart,
    SubStop,
    SubCount {
        running: u32,
    },
    Attention {
        message: Option<String>,
    },
    Stop {
        message: Option<String>,
        running: Option<u32>,
    },
}

pub struct RawObserverHook<'a> {
    pub event_name: &'a str,
    pub body: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WrapperArg {
    Literal(String),
    Env(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandWrapperSpec {
    pub command: String,
    pub prefix_args: Vec<WrapperArg>,
    pub skip_if_present: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserverSessionContext {
    pub session_id: String,
    pub hook_url: String,
}

impl ObserverSessionContext {
    pub fn new(session_id: impl Into<String>, hook_url: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            hook_url: hook_url.into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdapterSessionPlan {
    pub env: Vec<(String, String)>,
    pub wrappers: Vec<CommandWrapperSpec>,
    pub cleanup_paths: Vec<PathBuf>,
}

impl AdapterSessionPlan {
    pub fn merge(&mut self, mut other: Self) {
        self.env.append(&mut other.env);
        self.wrappers.append(&mut other.wrappers);
        self.cleanup_paths.append(&mut other.cleanup_paths);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserverAdapterError(String);

impl ObserverAdapterError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for ObserverAdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub const MAX_PROMPT_TEXT_CHARS: usize = 2_000;

fn is_command_prompt(value: &str) -> bool {
    value.starts_with('!') || value.starts_with('/') || value.starts_with('#')
}

pub fn prompt_text(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let prompt = value.get("prompt")?.as_str()?.trim();
    if prompt.is_empty() || is_command_prompt(prompt) {
        return None;
    }
    Some(prompt.chars().take(MAX_PROMPT_TEXT_CHARS).collect())
}

pub fn message(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let message = value.get("message")?.as_str()?;
    (!message.trim().is_empty()).then(|| message.to_string())
}

pub fn agent_id(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let agent_id = value.get("agent_id")?.as_str()?;
    (!agent_id.trim().is_empty()).then(|| agent_id.to_string())
}

/// Claude 훅 body의 top-level `session_id`(= native 리줌 ID). 모든 이벤트마다
/// 실려 오므로 종료 전에도 캡처할 수 있다(docs/claude-session-resume-design.md §2).
/// 공백/빈 값은 None.
pub fn native_session_id(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let session_id = value.get("session_id")?.as_str()?;
    (!session_id.trim().is_empty()).then(|| session_id.to_string())
}

/// Claude 훅 body의 top-level `cwd`(리줌은 같은 프로젝트 디렉터리에서만 가능해
/// 함께 저장해 둔다). 공백/빈 값은 None.
pub fn hook_cwd(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let cwd = value.get("cwd")?.as_str()?;
    (!cwd.trim().is_empty()).then(|| cwd.to_string())
}

/// Claude Stop/SubagentStop body의 background_tasks에서 실행 중 서브에이전트 수를 센다.
/// SubagentStop 스냅샷에는 정지 중인 자기 자신이 아직 "running"으로 포함되므로
/// top-level agent_id와 id가 일치하는 엔트리는 제외한다. 배열 부재/파싱 실패 = None.
pub fn running_subagents(body: &[u8]) -> Option<u32> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let tasks = value.get("background_tasks")?.as_array()?;
    let self_id = value.get("agent_id").and_then(|v| v.as_str());
    let count = tasks
        .iter()
        .filter(|t| {
            t.get("type").and_then(|v| v.as_str()) == Some("subagent")
                && t.get("status").and_then(|v| v.as_str()) == Some("running")
                && (self_id.is_none() || t.get("id").and_then(|v| v.as_str()) != self_id)
        })
        .count();
    Some(count as u32)
}

pub fn tool_description(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let description = value.get("tool_input")?.get("description")?.as_str()?;
    (!description.trim().is_empty()).then(|| description.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        agent_id, hook_cwd, is_command_prompt, message, native_session_id, prompt_text,
        running_subagents, tool_description, AdapterSessionPlan, CommandWrapperSpec,
        ObserverProvider, ObserverSessionContext,
    };

    fn wrapper(command: &str) -> CommandWrapperSpec {
        CommandWrapperSpec {
            command: command.into(),
            prefix_args: vec![],
            skip_if_present: vec![],
        }
    }

    #[test]
    fn observer_json_helpers_preserve_prompt_and_message_rules() {
        assert_eq!(prompt_text(b"not-json"), None);
        assert_eq!(prompt_text(br#"{"prompt":"   "}"#), None);
        assert_eq!(prompt_text(br#"{"prompt":"/clear"}"#), None);
        let long = serde_json::json!({ "prompt": "가".repeat(2_500) }).to_string();
        assert_eq!(prompt_text(long.as_bytes()).unwrap().chars().count(), 2_000);
        assert_eq!(
            message(br#"{"message":"attention"}"#).as_deref(),
            Some("attention")
        );
        assert_eq!(message(br#"{"message":" "}"#), None);
        assert_eq!(
            tool_description(br#"{"tool_input":{"description":"approval detail"}}"#).as_deref(),
            Some("approval detail"),
        );
    }

    #[test]
    fn agent_id_reads_only_non_empty_top_level_strings() {
        assert_eq!(
            agent_id(br#"{"agent_id":"uuid-123"}"#).as_deref(),
            Some("uuid-123")
        );
        assert_eq!(agent_id(br#"{}"#), None);
        assert_eq!(agent_id(b"not json"), None);
        assert_eq!(agent_id(br#"{"agent_id":""}"#), None);
        assert_eq!(agent_id(br#"{"agent_id":"   "}"#), None);
        assert_eq!(agent_id(br#"{"agent_id":42}"#), None);
        assert_eq!(agent_id(br#"{"agent_id":null}"#), None);
    }

    #[test]
    fn native_session_id_reads_only_non_empty_top_level_strings() {
        assert_eq!(
            native_session_id(br#"{"session_id":"native-abc"}"#).as_deref(),
            Some("native-abc")
        );
        assert_eq!(native_session_id(br#"{}"#), None);
        assert_eq!(native_session_id(b"not json"), None);
        assert_eq!(native_session_id(br#"{"session_id":""}"#), None);
        assert_eq!(native_session_id(br#"{"session_id":"   "}"#), None);
        assert_eq!(native_session_id(br#"{"session_id":42}"#), None);
        assert_eq!(native_session_id(br#"{"session_id":null}"#), None);
    }

    #[test]
    fn hook_cwd_reads_only_non_empty_top_level_strings() {
        assert_eq!(
            hook_cwd(br#"{"cwd":"/home/x/project"}"#).as_deref(),
            Some("/home/x/project")
        );
        assert_eq!(hook_cwd(br#"{}"#), None);
        assert_eq!(hook_cwd(b"not json"), None);
        assert_eq!(hook_cwd(br#"{"cwd":""}"#), None);
        assert_eq!(hook_cwd(br#"{"cwd":"  "}"#), None);
        assert_eq!(hook_cwd(br#"{"cwd":5}"#), None);
    }

    #[test]
    fn running_subagents_excludes_matching_self_id() {
        let body = br#"{
            "agent_id":"self",
            "background_tasks":[
                {"id":"self","type":"subagent","status":"running"},
                {"id":"other","type":"subagent","status":"running"}
            ]
        }"#;
        assert_eq!(running_subagents(body), Some(1));
    }

    #[test]
    fn running_subagents_stop_shape_without_agent_id_counts_all() {
        let body = br#"{"background_tasks":[
            {"id":"one","type":"subagent","status":"running"},
            {"id":"two","type":"subagent","status":"running"}
        ]}"#;
        assert_eq!(running_subagents(body), Some(2));
    }

    #[test]
    fn running_subagents_does_not_subtract_when_self_is_absent() {
        let body = br#"{"agent_id":"missing","background_tasks":[
            {"id":"other","type":"subagent","status":"running"}
        ]}"#;
        assert_eq!(running_subagents(body), Some(1));
    }

    #[test]
    fn running_subagents_filters_status_and_type() {
        let body = br#"{"background_tasks":[
            {"id":"running-sub","type":"subagent","status":"running"},
            {"id":"stopped-sub","type":"subagent","status":"stopped"},
            {"id":"running-shell","type":"shell","status":"running"}
        ]}"#;
        assert_eq!(running_subagents(body), Some(1));
    }

    #[test]
    fn running_subagents_distinguishes_missing_invalid_and_empty_arrays() {
        assert_eq!(running_subagents(br#"{}"#), None);
        assert_eq!(running_subagents(b"not-json"), None);
        assert_eq!(running_subagents(br#"{"background_tasks":[]}"#), Some(0));
    }

    #[test]
    fn prompt_text_preserves_plain_text_and_filters_command_prefixes() {
        assert_eq!(
            prompt_text(r#"{"prompt":"  버그 고쳐줘  "}"#.as_bytes()).as_deref(),
            Some("버그 고쳐줘"),
        );
        assert_eq!(prompt_text(br#"{"prompt":"!git status"}"#), None);
        assert_eq!(prompt_text(br##"{"prompt":"#remember"}"##), None);
        assert_eq!(prompt_text(br#"{"session_id":"s1"}"#), None);
    }

    #[test]
    fn is_command_prompt_flags_bash_slash_and_memory_prefixes() {
        assert!(is_command_prompt("!git status"));
        assert!(is_command_prompt("/clear"));
        assert!(is_command_prompt("#remember"));
        assert!(!is_command_prompt("버그 고쳐줘"));
        assert!(!is_command_prompt("git status"));
        // 절대경로 텍스트도 '/'로 시작하면 명령으로 취급된다 — 감수하는 트레이드오프.
        assert!(is_command_prompt("/home/x"));
    }

    #[test]
    fn observer_contract_constructors_preserve_exact_values() {
        assert_eq!(ObserverProvider::Claude.as_str(), "claude");
        assert_eq!(ObserverProvider::Codex.as_str(), "codex");
        assert_eq!(
            ObserverProvider::parse("claude"),
            Some(ObserverProvider::Claude)
        );
        assert_eq!(
            ObserverProvider::parse("codex"),
            Some(ObserverProvider::Codex)
        );
        assert_eq!(ObserverProvider::parse("other"), None);

        let codex_wrapper = wrapper("codex");
        assert_eq!(codex_wrapper.command, "codex");
        assert!(codex_wrapper.prefix_args.is_empty());
        assert!(codex_wrapper.skip_if_present.is_empty());

        let context = ObserverSessionContext::new("s1", "http://127.0.0.1/hook");
        assert_eq!(context.session_id, "s1");
        assert_eq!(context.hook_url, "http://127.0.0.1/hook");

        let mut merged = AdapterSessionPlan {
            env: vec![("FIRST".into(), "1".into())],
            wrappers: vec![wrapper("claude")],
            cleanup_paths: vec!["first.json".into()],
        };
        merged.merge(AdapterSessionPlan {
            env: vec![("SECOND".into(), "2".into())],
            wrappers: vec![wrapper("codex")],
            cleanup_paths: vec!["second.json".into()],
        });
        assert_eq!(merged.env[1], ("SECOND".into(), "2".into()));
        assert_eq!(merged.wrappers[1].command, "codex");
        assert_eq!(
            merged.cleanup_paths[1],
            std::path::PathBuf::from("second.json")
        );
    }
}
