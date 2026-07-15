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
    Prompt { text: Option<String> },
    Tool,
    SubStart,
    SubStop,
    Attention { message: Option<String> },
    Stop { message: Option<String> },
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

pub fn tool_description(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let description = value.get("tool_input")?.get("description")?.as_str()?;
    (!description.trim().is_empty()).then(|| description.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        agent_id, is_command_prompt, message, prompt_text, tool_description, AdapterSessionPlan,
        CommandWrapperSpec, ObserverProvider, ObserverSessionContext,
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
