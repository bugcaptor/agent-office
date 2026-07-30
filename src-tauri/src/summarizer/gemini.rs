use super::{ProviderCommand, SummaryPurpose};
use crate::persistence::settings_store::SummaryProvider;

// gemini CLI는 headless(-p) 모드에서 stdin 본문을 지원한다 — 최종 프롬프트는
// stdin 뒤에 -p 지시문이 덧붙는 형태(공식: "Appended to input on stdin").
// run_with_timeout이 cwd를 임시 폴더로 잡는데 gemini는 비신뢰 폴더에서
// headless 실행을 거부하므로 --skip-trust가 필수다. 순수 텍스트 변환이라
// 도구 승인은 발생하지 않는다.
#[cfg(windows)]
const WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
$OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$c = Get-Command gemini -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
$in = [Console]::In.ReadToEnd()
$in | & $c.Source --prompt $env:AO_INSTRUCTION --model $env:AO_MODEL --output-format text --skip-trust
exit $LASTEXITCODE"#;

#[cfg(windows)]
pub(super) fn build(instruction: &str, purpose: SummaryPurpose) -> ProviderCommand {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]);
    command.creation_flags(CREATE_NO_WINDOW);
    command.env("AO_INSTRUCTION", instruction);
    command.env("AO_MODEL", purpose.gemini_model());
    ProviderCommand {
        command,
        provider: SummaryProvider::Gemini,
    }
}

#[cfg(not(windows))]
pub(super) fn build(instruction: &str, purpose: SummaryPurpose) -> ProviderCommand {
    let mut command = std::process::Command::new("gemini");
    command.args([
        "--prompt",
        instruction,
        "--model",
        purpose.gemini_model(),
        "--output-format",
        "text",
        "--skip-trust",
    ]);
    ProviderCommand {
        command,
        provider: SummaryProvider::Gemini,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn non_windows_command_passes_instruction_model_and_trust_flags() {
        let spec = build("요약 지시", SummaryPurpose::Label);
        assert_eq!(spec.provider, SummaryProvider::Gemini);
        let cmd = spec.command;
        assert_eq!(cmd.get_program(), "gemini");
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            vec![
                "--prompt",
                "요약 지시",
                "--model",
                "gemini-2.5-flash",
                "--output-format",
                "text",
                "--skip-trust",
            ]
        );
    }

    /// 학습자료 목적은 같은 파이프라인을 쓰되 더 큰 모델을 고른다.
    #[cfg(not(windows))]
    #[test]
    fn study_purpose_upgrades_the_model() {
        let spec = build("학습자료 지시", SummaryPurpose::Study);
        let args: Vec<_> = spec
            .command
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert!(args.contains(&"gemini-2.5-pro".to_string()), "{args:?}");
        assert!(!args.contains(&"gemini-2.5-flash".to_string()), "{args:?}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_script_pins_bomless_utf8_output_encoding() {
        assert!(
            WINDOWS_SCRIPT.contains("$OutputEncoding=New-Object System.Text.UTF8Encoding($false)")
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
    fn windows_command_uses_powershell_with_no_window_flag_and_env_instruction() {
        let spec = build("요약 지시", SummaryPurpose::Label);
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
        assert!(WINDOWS_SCRIPT.contains("--skip-trust"), "{WINDOWS_SCRIPT}");
    }
}
