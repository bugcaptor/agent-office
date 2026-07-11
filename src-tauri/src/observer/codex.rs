use std::path::PathBuf;

use super::event::{prompt_text, tool_description};
use super::{
    AdapterSessionPlan, CommandWrapperSpec, ObserverAdapter, ObserverAdapterError,
    ObserverCapabilities, ObserverEvent, ObserverProvider, ObserverSessionContext, RawObserverHook,
    ToolCoverage, WrapperArg,
};

const CODEX_PROMPT_CONFIG: &str = "AGENT_OFFICE_CODEX_HOOK_USER_PROMPT";
const CODEX_TOOL_CONFIG: &str = "AGENT_OFFICE_CODEX_HOOK_POST_TOOL";
const CODEX_ATTENTION_CONFIG: &str = "AGENT_OFFICE_CODEX_HOOK_PERMISSION";
const CODEX_STOP_CONFIG: &str = "AGENT_OFFICE_CODEX_HOOK_STOP";

pub struct CodexAdapter {
    forwarder_executable: PathBuf,
}

impl CodexAdapter {
    pub fn new(forwarder_executable: PathBuf) -> Self {
        Self {
            forwarder_executable,
        }
    }

    fn forwarder_command(&self) -> Result<String, ObserverAdapterError> {
        if self.forwarder_executable.as_os_str().is_empty()
            || !self.forwarder_executable.is_absolute()
        {
            return Err(ObserverAdapterError::new(
                "Codex observer forwarder path must be absolute",
            ));
        }
        let path = self.forwarder_executable.to_str().ok_or_else(|| {
            ObserverAdapterError::new("Codex observer forwarder path must be Unicode")
        })?;
        if cfg!(windows) {
            if path.contains('"') {
                return Err(ObserverAdapterError::new(
                    "Codex observer forwarder path contains a quote",
                ));
            }
            Ok(format!("\"{path}\" --observer-forward codex"))
        } else {
            Ok(format!(
                "'{}' --observer-forward codex",
                path.replace('\'', "'\"'\"'"),
            ))
        }
    }

    fn hook_config(event: &str, matcher: bool, command: &str) -> String {
        let command = serde_json::to_string(command).expect("serializing a string cannot fail");
        let matcher = if matcher { "matcher=\"*\"," } else { "" };
        format!(
            "hooks.{event}=[{{{matcher}hooks=[{{type=\"command\",command={command},timeout=2}}]}}]"
        )
    }
}

impl ObserverAdapter for CodexAdapter {
    fn provider(&self) -> ObserverProvider {
        ObserverProvider::Codex
    }

    fn capabilities(&self) -> ObserverCapabilities {
        ObserverCapabilities {
            prompt: true,
            attention: true,
            stop: true,
            tool: ToolCoverage::BestEffort,
        }
    }

    fn prepare_session(
        &self,
        _context: &ObserverSessionContext,
    ) -> Result<AdapterSessionPlan, ObserverAdapterError> {
        let command = self.forwarder_command()?;
        let env = vec![
            (
                CODEX_PROMPT_CONFIG.into(),
                Self::hook_config("UserPromptSubmit", false, &command),
            ),
            (
                CODEX_TOOL_CONFIG.into(),
                Self::hook_config("PostToolUse", true, &command),
            ),
            (
                CODEX_ATTENTION_CONFIG.into(),
                Self::hook_config("PermissionRequest", true, &command),
            ),
            (
                CODEX_STOP_CONFIG.into(),
                Self::hook_config("Stop", false, &command),
            ),
        ];
        Ok(AdapterSessionPlan {
            env,
            wrappers: vec![CommandWrapperSpec {
                command: "codex".into(),
                prefix_args: vec![
                    WrapperArg::Literal("--enable".into()),
                    WrapperArg::Literal("hooks".into()),
                    WrapperArg::Literal("-c".into()),
                    WrapperArg::Env(CODEX_PROMPT_CONFIG.into()),
                    WrapperArg::Literal("-c".into()),
                    WrapperArg::Env(CODEX_TOOL_CONFIG.into()),
                    WrapperArg::Literal("-c".into()),
                    WrapperArg::Env(CODEX_ATTENTION_CONFIG.into()),
                    WrapperArg::Literal("-c".into()),
                    WrapperArg::Env(CODEX_STOP_CONFIG.into()),
                ],
                skip_if_present: vec![],
            }],
            cleanup_paths: vec![],
        })
    }

    fn map_hook(&self, raw: &RawObserverHook<'_>) -> Option<ObserverEvent> {
        match raw.event_name {
            "UserPromptSubmit" => Some(ObserverEvent::Prompt {
                text: prompt_text(raw.body),
            }),
            "PostToolUse" => Some(ObserverEvent::Tool),
            "PermissionRequest" => Some(ObserverEvent::Attention {
                message: tool_description(raw.body),
            }),
            "Stop" => Some(ObserverEvent::Stop { message: None }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CodexAdapter, CODEX_ATTENTION_CONFIG, CODEX_PROMPT_CONFIG, CODEX_STOP_CONFIG,
        CODEX_TOOL_CONFIG,
    };
    use crate::observer::claude::ClaudeAdapter;
    use crate::observer::{ObserverAdapter, ObserverSessionContext, WrapperArg};
    use std::path::PathBuf;

    fn scratch_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-observer-adapter-test-{}",
            uuid::Uuid::new_v4(),
        ))
    }

    fn forwarder_path_with_spaces() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Program Files\Agent Office\agent-office.exe")
        } else {
            PathBuf::from("/tmp/Agent Office/agent-office")
        }
    }

    #[test]
    fn codex_plan_uses_stable_additive_hooks_and_no_policy_override() {
        let forwarder = forwarder_path_with_spaces();
        let adapter = CodexAdapter::new(forwarder.clone());
        let first = adapter
            .prepare_session(&ObserverSessionContext::new(
                "ao-s1",
                "http://127.0.0.1:1/hook",
            ))
            .unwrap();
        let second = adapter
            .prepare_session(&ObserverSessionContext::new(
                "ao-s2",
                "http://127.0.0.1:2/hook",
            ))
            .unwrap();

        assert_eq!(
            first.env, second.env,
            "hook definitions must not contain session or port",
        );
        assert_eq!(first.env.len(), 4);
        assert_eq!(
            first
                .env
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            vec![
                CODEX_PROMPT_CONFIG,
                CODEX_TOOL_CONFIG,
                CODEX_ATTENTION_CONFIG,
                CODEX_STOP_CONFIG,
            ],
        );

        let command = if cfg!(windows) {
            format!(
                "\"{}\" --observer-forward codex",
                forwarder.to_str().unwrap()
            )
        } else {
            format!("'{}' --observer-forward codex", forwarder.to_str().unwrap())
        };
        let command = serde_json::to_string(&command).unwrap();
        assert_eq!(
            first.env,
            vec![
                (
                    CODEX_PROMPT_CONFIG.into(),
                    format!(
                        "hooks.UserPromptSubmit=[{{hooks=[{{type=\"command\",command={command},timeout=2}}]}}]"
                    ),
                ),
                (
                    CODEX_TOOL_CONFIG.into(),
                    format!(
                        "hooks.PostToolUse=[{{matcher=\"*\",hooks=[{{type=\"command\",command={command},timeout=2}}]}}]"
                    ),
                ),
                (
                    CODEX_ATTENTION_CONFIG.into(),
                    format!(
                        "hooks.PermissionRequest=[{{matcher=\"*\",hooks=[{{type=\"command\",command={command},timeout=2}}]}}]"
                    ),
                ),
                (
                    CODEX_STOP_CONFIG.into(),
                    format!(
                        "hooks.Stop=[{{hooks=[{{type=\"command\",command={command},timeout=2}}]}}]"
                    ),
                ),
            ],
        );
        assert!(first.env.iter().all(|(_, config)| {
            config.contains("--observer-forward codex")
                && !config.contains("ao-s1")
                && !config.contains("ao-s2")
                && !config.contains("127.0.0.1")
        }));

        assert_eq!(first.wrappers.len(), 1);
        let wrapper = &first.wrappers[0];
        assert_eq!(wrapper.command, "codex");
        assert_eq!(
            wrapper.prefix_args,
            vec![
                WrapperArg::Literal("--enable".into()),
                WrapperArg::Literal("hooks".into()),
                WrapperArg::Literal("-c".into()),
                WrapperArg::Env(CODEX_PROMPT_CONFIG.into()),
                WrapperArg::Literal("-c".into()),
                WrapperArg::Env(CODEX_TOOL_CONFIG.into()),
                WrapperArg::Literal("-c".into()),
                WrapperArg::Env(CODEX_ATTENTION_CONFIG.into()),
                WrapperArg::Literal("-c".into()),
                WrapperArg::Env(CODEX_STOP_CONFIG.into()),
            ],
        );
        assert!(wrapper.skip_if_present.is_empty());
        assert!(first.cleanup_paths.is_empty());

        let rendered = format!("{first:?}");
        for forbidden in [
            "dangerously-bypass-hook-trust",
            "approval_policy",
            "sandbox_mode",
            "model=",
            "--ignore-user-config",
        ] {
            assert!(
                !rendered.contains(forbidden),
                "forbidden override: {forbidden}"
            );
        }
    }

    #[test]
    fn empty_forwarder_path_disables_only_codex_preparation() {
        let codex = CodexAdapter::new(PathBuf::new());
        let context = ObserverSessionContext::new("ao-s1", "http://127.0.0.1:43123/hook");
        assert_eq!(
            codex.prepare_session(&context).unwrap_err().to_string(),
            "Codex observer forwarder path must be absolute",
        );

        let dir = scratch_dir();
        let claude = ClaudeAdapter::new(dir.clone());
        assert!(claude.prepare_session(&context).is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn relative_forwarder_path_is_rejected_before_plan_creation() {
        let codex = CodexAdapter::new(PathBuf::from("agent-office"));
        let error = codex
            .prepare_session(&ObserverSessionContext::new(
                "ao-s1",
                "http://127.0.0.1:43123/hook",
            ))
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Codex observer forwarder path must be absolute",
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_forwarder_path_with_quote_is_rejected() {
        let codex = CodexAdapter::new(PathBuf::from(
            r#"C:\Program Files\Agent "Office"\agent-office.exe"#,
        ));
        let error = codex
            .prepare_session(&ObserverSessionContext::new(
                "ao-s1",
                "http://127.0.0.1:43123/hook",
            ))
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Codex observer forwarder path contains a quote",
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_forwarder_path_with_quote_is_shell_escaped() {
        let codex = CodexAdapter::new(PathBuf::from("/tmp/Agent 'Office'/agent-office"));
        let command = codex.forwarder_command().unwrap();

        assert_eq!(
            command,
            "'/tmp/Agent '\"'\"'Office'\"'\"'/agent-office' --observer-forward codex",
        );
    }
}
