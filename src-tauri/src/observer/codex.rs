use std::path::PathBuf;

use super::event::{agent_id, codex_stop_message, prompt_text, tool_activity_text, tool_description};
use super::hook_command::forwarder_shell_command;
use super::{
    AdapterSessionPlan, CommandWrapperSpec, ObserverAdapter, ObserverAdapterError, ObserverEvent,
    ObserverProvider, ObserverSessionContext, RawObserverHook, WrapperArg,
};

const CODEX_PROMPT_CONFIG: &str = "AGENT_OFFICE_CODEX_HOOK_USER_PROMPT";
const CODEX_TOOL_CONFIG: &str = "AGENT_OFFICE_CODEX_HOOK_POST_TOOL";
const CODEX_ATTENTION_CONFIG: &str = "AGENT_OFFICE_CODEX_HOOK_PERMISSION";
const CODEX_STOP_CONFIG: &str = "AGENT_OFFICE_CODEX_HOOK_STOP";
const CODEX_SUBAGENT_START_CONFIG: &str = "AGENT_OFFICE_CODEX_HOOK_SUBAGENT_START";
const CODEX_SUBAGENT_STOP_CONFIG: &str = "AGENT_OFFICE_CODEX_HOOK_SUBAGENT_STOP";

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
        // codex는 이벤트명을 body에서 얻으므로 forwarder에 이벤트 인자를 넘기지 않는다.
        forwarder_shell_command(&self.forwarder_executable, &["codex"])
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
            (
                CODEX_SUBAGENT_START_CONFIG.into(),
                Self::hook_config("SubagentStart", false, &command),
            ),
            (
                CODEX_SUBAGENT_STOP_CONFIG.into(),
                Self::hook_config("SubagentStop", false, &command),
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
                    WrapperArg::Literal("-c".into()),
                    WrapperArg::Env(CODEX_SUBAGENT_START_CONFIG.into()),
                    WrapperArg::Literal("-c".into()),
                    WrapperArg::Env(CODEX_SUBAGENT_STOP_CONFIG.into()),
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
            // 이슈 #43: claude와 동일하게 도구 요약을 싣되, 서브에이전트 내부 도구
            // (agent_id 있음)는 하트비트만 유지한다. codex는 transcript 꼬리가 없어
            // assistant 내레이션은 항상 None.
            "PostToolUse" => Some(if agent_id(raw.body).is_some() {
                ObserverEvent::Tool {
                    text: None,
                    assistant: None,
                }
            } else {
                ObserverEvent::Tool {
                    text: tool_activity_text(raw.body),
                    assistant: None,
                }
            }),
            "PermissionRequest" => Some(ObserverEvent::Attention {
                message: tool_description(raw.body),
            }),
            // 이슈 #39: 예전엔 last_assistant_message 를 의도적으로 버렸으나,
            // 이제 완료 알림 본문으로 실어 보낸다(절단은 codex_stop_message).
            "Stop" => Some(ObserverEvent::Stop {
                message: codex_stop_message(raw.body),
                running: None,
            }),
            "SubagentStart" => Some(ObserverEvent::SubStart),
            "SubagentStop" => Some(ObserverEvent::SubStop),
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
    use std::path::{Path, PathBuf};

    #[cfg(windows)]
    const HOOK_BODY: &str = r#"{"hook_event_name":"UserPromptSubmit","prompt":"marker"}"#;

    #[cfg(windows)]
    struct HookCommandFixture {
        dir: PathBuf,
        forwarder: PathBuf,
        args_file: PathBuf,
        stdin_file: PathBuf,
    }

    #[cfg(windows)]
    impl HookCommandFixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "Agent Office Codex Hook Test {}",
                uuid::Uuid::new_v4(),
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let forwarder = dir.join("capture forwarder.ps1");
            let args_file = dir.join("forwarded args.txt");
            let stdin_file = dir.join("forwarded stdin.txt");
            std::fs::write(
                &forwarder,
                r#"[IO.File]::WriteAllLines($env:AO_CAPTURE_ARGS, [string[]]$args)
[IO.File]::WriteAllText($env:AO_CAPTURE_STDIN, [Console]::In.ReadToEnd())
"#,
            )
            .unwrap();
            Self {
                dir,
                forwarder,
                args_file,
                stdin_file,
            }
        }

        fn invoke(&self, shell: &Path, shell_args: &[&str], command: &str) -> std::process::Output {
            use std::io::Write as _;
            use std::process::{Command, Stdio};

            let mut child = Command::new(shell)
                .args(shell_args)
                .arg(command)
                .env("AO_CAPTURE_ARGS", &self.args_file)
                .env("AO_CAPTURE_STDIN", &self.stdin_file)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(HOOK_BODY.as_bytes())
                .unwrap();
            child.wait_with_output().unwrap()
        }

        fn assert_forwarded(&self, command: &str, output: &std::process::Output) {
            assert!(
                output.status.success(),
                "command={command:?} stdout={} stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            assert_eq!(
                std::fs::read_to_string(&self.args_file).unwrap(),
                "--observer-forward\r\ncodex\r\n",
            );
            assert_eq!(
                std::fs::read_to_string(&self.stdin_file).unwrap(),
                HOOK_BODY,
            );
        }
    }

    #[cfg(windows)]
    impl Drop for HookCommandFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[cfg(windows)]
    fn installed_git_bash() -> Option<PathBuf> {
        ["ProgramFiles", "ProgramFiles(x86)"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from)
            .map(|root| root.join("Git").join("bin").join("bash.exe"))
            .find(|path| path.is_file())
    }

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
        assert_eq!(first.env.len(), 6);
        assert_eq!(
            first
                .env
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "AGENT_OFFICE_CODEX_HOOK_USER_PROMPT",
                "AGENT_OFFICE_CODEX_HOOK_POST_TOOL",
                "AGENT_OFFICE_CODEX_HOOK_PERMISSION",
                "AGENT_OFFICE_CODEX_HOOK_STOP",
                "AGENT_OFFICE_CODEX_HOOK_SUBAGENT_START",
                "AGENT_OFFICE_CODEX_HOOK_SUBAGENT_STOP",
            ],
        );

        let command = adapter.forwarder_command().unwrap();
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
                (
                    "AGENT_OFFICE_CODEX_HOOK_SUBAGENT_START".into(),
                    format!(
                        "hooks.SubagentStart=[{{hooks=[{{type=\"command\",command={command},timeout=2}}]}}]"
                    ),
                ),
                (
                    "AGENT_OFFICE_CODEX_HOOK_SUBAGENT_STOP".into(),
                    format!(
                        "hooks.SubagentStop=[{{hooks=[{{type=\"command\",command={command},timeout=2}}]}}]"
                    ),
                ),
            ],
        );
        assert!(first.env.iter().all(|(_, config)| {
            !config.contains("ao-s1") && !config.contains("ao-s2") && !config.contains("127.0.0.1")
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
                WrapperArg::Literal("-c".into()),
                WrapperArg::Env("AGENT_OFFICE_CODEX_HOOK_SUBAGENT_START".into()),
                WrapperArg::Literal("-c".into()),
                WrapperArg::Env("AGENT_OFFICE_CODEX_HOOK_SUBAGENT_STOP".into()),
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
            "observer forwarder path must be absolute",
        );

        let dir = scratch_dir();
        let claude = ClaudeAdapter::new(dir.clone(), std::env::current_exe().unwrap());
        assert!(claude.prepare_session(&context).is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn codex_post_tool_use_carries_summary_and_suppresses_subagent() {
        use crate::observer::{ObserverEvent, RawObserverHook};
        let codex = CodexAdapter::new(forwarder_path_with_spaces());
        // 이슈 #43: 메인 세션 도구 요약을 싣는다(assistant는 codex라 항상 None).
        assert_eq!(
            codex.map_hook(&RawObserverHook {
                event_name: "PostToolUse",
                body: br#"{"tool_name":"Grep","tool_input":{"pattern":"TODO"}}"#,
            }),
            Some(ObserverEvent::Tool {
                text: Some("Grep: TODO".into()),
                assistant: None,
            }),
        );
        // 서브에이전트 내부 도구(agent_id)는 하트비트만.
        assert_eq!(
            codex.map_hook(&RawObserverHook {
                event_name: "PostToolUse",
                body: br#"{"agent_id":"sub-1","tool_name":"Grep","tool_input":{"pattern":"TODO"}}"#,
            }),
            Some(ObserverEvent::Tool {
                text: None,
                assistant: None,
            }),
        );
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
            "observer forwarder path must be absolute",
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_hook_command_executes_spaced_forwarder_via_cmd() {
        let fixture = HookCommandFixture::new();
        let command = CodexAdapter::new(fixture.forwarder.clone())
            .forwarder_command()
            .unwrap();
        let output = fixture.invoke(Path::new("cmd.exe"), &["/D", "/S", "/C"], &command);
        fixture.assert_forwarded(&command, &output);
    }

    #[cfg(windows)]
    #[test]
    fn windows_hook_command_executes_spaced_forwarder_via_pwsh() {
        let fixture = HookCommandFixture::new();
        let command = CodexAdapter::new(fixture.forwarder.clone())
            .forwarder_command()
            .unwrap();
        let output = fixture.invoke(
            Path::new("pwsh.exe"),
            &["-NoProfile", "-NonInteractive", "-Command"],
            &command,
        );
        fixture.assert_forwarded(&command, &output);
    }

    #[cfg(windows)]
    #[test]
    fn windows_hook_command_executes_spaced_forwarder_via_windows_powershell() {
        let fixture = HookCommandFixture::new();
        let command = CodexAdapter::new(fixture.forwarder.clone())
            .forwarder_command()
            .unwrap();
        let output = fixture.invoke(
            Path::new("powershell.exe"),
            &["-NoProfile", "-NonInteractive", "-Command"],
            &command,
        );
        fixture.assert_forwarded(&command, &output);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires an installed Git Bash; run explicitly on Windows"]
    fn windows_hook_command_executes_spaced_forwarder_via_git_bash() {
        let bash = installed_git_bash().expect("Git Bash is not installed in a standard path");
        let fixture = HookCommandFixture::new();
        let command = CodexAdapter::new(fixture.forwarder.clone())
            .forwarder_command()
            .unwrap();
        let output = fixture.invoke(&bash, &["--noprofile", "--norc", "-c"], &command);
        fixture.assert_forwarded(&command, &output);
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
            "observer forwarder path contains a quote",
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
