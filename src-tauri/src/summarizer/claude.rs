use super::ProviderCommand;
use crate::persistence::settings_store::SummaryProvider;

#[cfg(windows)]
const WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
$OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$c = Get-Command $env:AO_PROGRAM -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
$in = [Console]::In.ReadToEnd()
$in | & $c.Source -p $env:AO_INSTRUCTION --model $env:AO_MODEL --output-format text --max-turns 1
exit $LASTEXITCODE"#;

#[cfg(windows)]
pub(super) fn build(program: &str, instruction: &str, model: &str) -> ProviderCommand {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]);
    command.creation_flags(CREATE_NO_WINDOW);
    command.env("AO_PROGRAM", program);
    command.env("AO_INSTRUCTION", instruction);
    command.env("AO_MODEL", model);
    ProviderCommand {
        command,
        provider: SummaryProvider::Claude,
    }
}

#[cfg(not(windows))]
pub(super) fn build(program: &str, instruction: &str, model: &str) -> ProviderCommand {
    let mut command = std::process::Command::new(program);
    command.args([
        "-p",
        instruction,
        "--model",
        model,
        "--output-format",
        "text",
        "--max-turns",
        "1",
    ]);
    ProviderCommand {
        command,
        provider: SummaryProvider::Claude,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_debug(command: &std::process::Command) -> String {
        let mut parts = vec![command.get_program().to_string_lossy().into_owned()];
        parts.extend(
            command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned()),
        );
        parts.extend(command.get_envs().filter_map(|(key, value)| {
            value.map(|value| format!("{}={}", key.to_string_lossy(), value.to_string_lossy()))
        }));
        parts.join(" ")
    }

    #[test]
    fn claude_command_pins_existing_behavior() {
        let spec = build("claude", "요약 지시", "haiku");
        let rendered = command_debug(&spec.command);
        assert!(rendered.contains("haiku"), "{rendered}");
        assert!(rendered.contains("--output-format"), "{rendered}");
        assert!(rendered.contains("text"), "{rendered}");
        assert!(rendered.contains("--max-turns"), "{rendered}");
        assert!(rendered.contains("1"), "{rendered}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_script_pins_bomless_utf8_output_encoding() {
        assert!(
            WINDOWS_SCRIPT.contains("$OutputEncoding=New-Object System.Text.UTF8Encoding($false)")
        );
        assert!(!WINDOWS_SCRIPT.contains("$OutputEncoding=[System.Text.Encoding]::UTF8"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_script_reads_stdin_to_eof_before_invoking_provider() {
        let gate = WINDOWS_SCRIPT.find("[Console]::In.ReadToEnd()").unwrap();
        let invocation = WINDOWS_SCRIPT.find("$in | & $c.Source").unwrap();
        assert!(gate < invocation, "{WINDOWS_SCRIPT}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_command_uses_powershell_with_no_window_flag_and_env_instruction() {
        let spec = build("claude", "요약 지시", "haiku");
        let cmd = spec.command;
        assert_eq!(cmd.get_program(), "powershell.exe");
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            vec!["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]
        );
        let env_val = cmd
            .get_envs()
            .find(|(k, _)| *k == "AO_INSTRUCTION")
            .and_then(|(_, v)| v);
        assert_eq!(env_val, Some(std::ffi::OsStr::new("요약 지시")));
    }

    /// 모델은 이제 호출측(`summarizer::resolve_model`)이 정해 넘긴다 — 여기서는
    /// 그 값이 왜곡 없이 커맨드로 실리는지만 고정한다(목적별 기본값·설정
    /// 오버라이드 규칙은 mod.rs의 `resolve_model` 테스트가 지킨다).
    #[cfg(not(windows))]
    #[test]
    fn explicit_model_is_passed_through() {
        let spec = build("claude", "학습자료 지시", "sonnet");
        let rendered = command_debug(&spec.command);
        assert!(rendered.contains("sonnet"), "{rendered}");
        assert!(!rendered.contains("haiku"), "{rendered}");
    }

    /// 커스텀 실행 명령(별개 계정 래퍼 `claude-t` 같은 것)이 그대로 프로그램이
    /// 돼야 한다 — 인자 규약은 건드리지 않는다.
    #[cfg(not(windows))]
    #[test]
    fn custom_program_replaces_the_binary_but_keeps_the_arguments() {
        let spec = build("claude-t", "요약 지시", "haiku");
        assert_eq!(spec.command.get_program(), "claude-t");
        let args: Vec<_> = spec
            .command
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(args[0], "-p");
        assert!(args.contains(&"--max-turns".to_string()), "{args:?}");
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_command_passes_instruction_and_model_flags() {
        let spec = build("claude", "요약 지시", "haiku");
        let cmd = spec.command;
        assert_eq!(cmd.get_program(), "claude");
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            vec![
                "-p",
                "요약 지시",
                "--model",
                "haiku",
                "--output-format",
                "text",
                "--max-turns",
                "1"
            ]
        );
    }
}
