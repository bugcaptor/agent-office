use super::ProviderCommand;
use crate::persistence::settings_store::SummaryProvider;

#[cfg(windows)]
const WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
$OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$c = Get-Command codex -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
$in = [Console]::In.ReadToEnd()
$config = if ($c.CommandType -eq 'Application') { 'model_reasoning_effort=\"low\"' } else { 'model_reasoning_effort="low"' }
$aoArgs = @('exec', '--ignore-user-config', '--ignore-rules', '--ephemeral', '--sandbox', 'read-only', '--model', 'gpt-5.4-mini', '--config', $config, '--skip-git-repo-check', '--color', 'never', '--', $env:AO_INSTRUCTION)
$in | & $c.Source @aoArgs
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
        provider: SummaryProvider::Codex,
    }
}

#[cfg(not(windows))]
pub(super) fn build(instruction: &str) -> ProviderCommand {
    let mut command = std::process::Command::new("codex");
    command.args([
        "exec",
        "--ignore-user-config",
        "--ignore-rules",
        "--ephemeral",
        "--sandbox",
        "read-only",
        "--model",
        "gpt-5.4-mini",
        "--config",
        "model_reasoning_effort=\"low\"",
        "--skip-git-repo-check",
        "--color",
        "never",
        "--",
        instruction,
    ]);
    ProviderCommand {
        command,
        provider: SummaryProvider::Codex,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DANGEROUS_INSTRUCTION: &str = "--dangerously-bypass-approvals-and-sandbox";

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
    fn codex_command_pins_low_cost_isolated_contract() {
        let spec = build("요약 지시");
        let rendered = command_debug(&spec.command);
        let config = "model_reasoning_effort=\"low\"";
        for expected in [
            "exec",
            "--ignore-user-config",
            "--ignore-rules",
            "--ephemeral",
            "--sandbox",
            "read-only",
            "--model",
            "gpt-5.4-mini",
            "--config",
            config,
            "--skip-git-repo-check",
            "--color",
            "never",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected}: {rendered}"
            );
        }
        assert!(!rendered.contains("luna"), "{rendered}");
        assert!(!rendered.contains("dangerously"), "{rendered}");
    }

    #[cfg(windows)]
    #[test]
    fn codex_command_terminates_options_before_dangerous_instruction() {
        let spec = build(DANGEROUS_INSTRUCTION);
        let args: Vec<_> = spec
            .command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec!["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]
        );
        assert!(
            WINDOWS_SCRIPT.contains("'never', '--', $env:AO_INSTRUCTION"),
            "{WINDOWS_SCRIPT}"
        );
        assert!(
            WINDOWS_SCRIPT.contains("& $c.Source @aoArgs"),
            "{WINDOWS_SCRIPT}"
        );
        let instruction = spec
            .command
            .get_envs()
            .find(|(key, _)| *key == "AO_INSTRUCTION")
            .and_then(|(_, value)| value);
        assert_eq!(
            instruction,
            Some(std::ffi::OsStr::new(DANGEROUS_INSTRUCTION))
        );
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
    fn windows_powershell_native_boundary_preserves_exact_codex_argv() {
        let dir = std::env::temp_dir().join(format!("ao-codex-argv-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let capture = dir.join("argv.json");
        std::fs::write(
            dir.join("capture.js"),
            "const fs = require('fs'); fs.writeFileSync(process.env.AO_CAPTURE_FILE, JSON.stringify(process.argv.slice(2)), 'utf8');",
        )
        .unwrap();
        std::fs::write(
            dir.join("codex.cmd"),
            "@echo off\r\nnode \"%~dp0capture.js\" %*\r\nexit /b %ERRORLEVEL%\r\n",
        )
        .unwrap();

        let original_path = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(dir.clone()).chain(std::env::split_paths(&original_path)),
        )
        .unwrap();
        let mut spec = build(DANGEROUS_INSTRUCTION);
        spec.command.env("PATH", path);
        spec.command.env("AO_CAPTURE_FILE", &capture);
        let output = spec.command.output().unwrap();
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let actual: Vec<String> =
            serde_json::from_slice(&std::fs::read(&capture).unwrap()).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(
            actual,
            vec![
                "exec",
                "--ignore-user-config",
                "--ignore-rules",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--model",
                "gpt-5.4-mini",
                "--config",
                "model_reasoning_effort=\"low\"",
                "--skip-git-repo-check",
                "--color",
                "never",
                "--",
                DANGEROUS_INSTRUCTION,
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_powershell_script_boundary_preserves_exact_codex_argv() {
        let dir = std::env::temp_dir().join(format!("ao-codex-ps1-argv-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let capture = dir.join("argv.json");
        std::fs::write(
            dir.join("codex.ps1"),
            r#"@($input) | Out-Null
[IO.File]::WriteAllText($env:AO_CAPTURE_FILE, (ConvertTo-Json -Compress -InputObject @($args)))
exit 0
"#,
        )
        .unwrap();

        let original_path = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(dir.clone()).chain(std::env::split_paths(&original_path)),
        )
        .unwrap();
        let mut spec = build(DANGEROUS_INSTRUCTION);
        spec.command.env("PATH", path);
        spec.command.env("AO_CAPTURE_FILE", &capture);
        let output = spec.command.output().unwrap();
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let actual: Vec<String> =
            serde_json::from_slice(&std::fs::read(&capture).unwrap()).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(
            actual,
            vec![
                "exec",
                "--ignore-user-config",
                "--ignore-rules",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--model",
                "gpt-5.4-mini",
                "--config",
                "model_reasoning_effort=\"low\"",
                "--skip-git-repo-check",
                "--color",
                "never",
                "--",
                DANGEROUS_INSTRUCTION,
            ]
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn codex_command_terminates_options_before_dangerous_instruction() {
        let spec = build(DANGEROUS_INSTRUCTION);
        assert_eq!(spec.command.get_program(), "codex");
        let args: Vec<_> = spec
            .command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "exec",
                "--ignore-user-config",
                "--ignore-rules",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--model",
                "gpt-5.4-mini",
                "--config",
                "model_reasoning_effort=\"low\"",
                "--skip-git-repo-check",
                "--color",
                "never",
                "--",
                DANGEROUS_INSTRUCTION,
            ]
        );
    }
}
