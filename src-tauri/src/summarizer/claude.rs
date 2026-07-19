use super::ProviderCommand;
use crate::persistence::settings_store::SummaryProvider;

#[cfg(windows)]
const WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
$OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$c = Get-Command claude -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
$in = [Console]::In.ReadToEnd()
$in | & $c.Source -p $env:AO_INSTRUCTION --model haiku --output-format text --max-turns 1
exit $LASTEXITCODE"#;

#[cfg(windows)]
pub(super) fn build(instruction: &str) -> ProviderCommand {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]);
    command.creation_flags(CREATE_NO_WINDOW);
    command.env("AO_INSTRUCTION", instruction);
    ProviderCommand {
        command,
        provider: SummaryProvider::Claude,
    }
}

#[cfg(not(windows))]
pub(super) fn build(instruction: &str) -> ProviderCommand {
    let mut command = std::process::Command::new("claude");
    command.args([
        "-p",
        instruction,
        "--model",
        "haiku",
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
        let spec = build("요약 지시");
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
        let spec = build("요약 지시");
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

    #[cfg(not(windows))]
    #[test]
    fn non_windows_command_passes_instruction_and_model_flags() {
        let spec = build("요약 지시");
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
