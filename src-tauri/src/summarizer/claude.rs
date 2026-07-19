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

// 실험(옵트인) 툴 모드: 읽기 전용 툴만 허용하고 MCP 서버를 배제한다.
// print 모드(-p)는 비대화형이라 허용 밖 툴 요청은 승인 대기 없이 거절된다.
// --allowedTools 는 variadic 이므로 맨 뒤에 두어 뒤따르는 토큰 흡수를 막는다.
#[cfg(windows)]
const WINDOWS_SCRIPT_TOOLS: &str = r#"$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
$OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$c = Get-Command claude -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
$in = [Console]::In.ReadToEnd()
$in | & $c.Source -p $env:AO_INSTRUCTION --model haiku --output-format text --max-turns 4 --strict-mcp-config --allowedTools Read Glob Grep
exit $LASTEXITCODE"#;

#[cfg(windows)]
fn windows_command(script: &str, instruction: &str) -> ProviderCommand {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    command.creation_flags(CREATE_NO_WINDOW);
    command.env("AO_INSTRUCTION", instruction);
    ProviderCommand {
        command,
        provider: SummaryProvider::Claude,
    }
}

#[cfg(windows)]
pub(super) fn build(instruction: &str) -> ProviderCommand {
    windows_command(WINDOWS_SCRIPT, instruction)
}

/// 실험 툴 모드(세션 작업 폴더에서 읽기 전용 툴 허용). 호출부가 workdir 를
/// 정할 때만 쓰인다. 나머지 플래그는 plain build 와 동일.
#[cfg(windows)]
pub(super) fn build_with_tools(instruction: &str) -> ProviderCommand {
    windows_command(WINDOWS_SCRIPT_TOOLS, instruction)
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

/// 실험 툴 모드(세션 작업 폴더에서 읽기 전용 툴 허용). 호출부가 workdir 를
/// 정할 때만 쓰인다. print 모드라 허용 밖 툴 요청은 대기 없이 거절된다.
/// --allowedTools 는 variadic 이므로 맨 뒤에 둔다.
#[cfg(not(windows))]
pub(super) fn build_with_tools(instruction: &str) -> ProviderCommand {
    let mut command = std::process::Command::new("claude");
    command.args([
        "-p",
        instruction,
        "--model",
        "haiku",
        "--output-format",
        "text",
        "--max-turns",
        "4",
        "--strict-mcp-config",
        "--allowedTools",
        "Read",
        "Glob",
        "Grep",
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

    #[test]
    fn claude_tool_command_pins_read_only_tools_and_never_skips_permissions() {
        let spec = build_with_tools("요약 지시");
        let rendered = command_debug(&spec.command);
        assert!(rendered.contains("--allowedTools"), "{rendered}");
        assert!(rendered.contains("Read"), "{rendered}");
        assert!(rendered.contains("Glob"), "{rendered}");
        assert!(rendered.contains("Grep"), "{rendered}");
        assert!(rendered.contains("--max-turns"), "{rendered}");
        assert!(rendered.contains('4'), "{rendered}");
        assert!(rendered.contains("--strict-mcp-config"), "{rendered}");
        // 쓰기·셸 툴은 절대 허용하지 않는다.
        assert!(!rendered.contains("dangerously"), "{rendered}");
        assert!(!rendered.contains("Bash"), "{rendered}");
        assert!(!rendered.contains("Write"), "{rendered}");
        assert!(!rendered.contains("Edit"), "{rendered}");
    }

    #[cfg(not(windows))]
    #[test]
    fn tool_command_places_variadic_allowed_tools_last() {
        let spec = build_with_tools("요약 지시");
        let args: Vec<_> = spec
            .command
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        // variadic --allowedTools 가 뒤 토큰을 흡수하지 않도록 맨 끝에 온다.
        let idx = args.iter().position(|a| a == "--allowedTools").unwrap();
        assert_eq!(&args[idx..], &["--allowedTools", "Read", "Glob", "Grep"]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_tool_script_pins_read_only_allowed_tools_at_end() {
        assert!(WINDOWS_SCRIPT_TOOLS.contains("--allowedTools Read Glob Grep"));
        assert!(WINDOWS_SCRIPT_TOOLS.contains("--max-turns 4"));
        assert!(WINDOWS_SCRIPT_TOOLS.contains("--strict-mcp-config"));
        assert!(WINDOWS_SCRIPT_TOOLS.trim_end().ends_with("exit $LASTEXITCODE"));
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
