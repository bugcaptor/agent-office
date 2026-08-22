use super::ProviderCommand;
use crate::persistence::settings_store::SummaryProvider;

// opencode CLI(sst/opencode)의 비대화형 실행은 `opencode run [message..]`다
// (`opencode run --help`: "run opencode with a message"). claude/gemini와 마찬가지로
// stdin으로 흘린 본문을 메시지 뒤에 붙여 읽으므로 지시문만 인자로 넘긴다 —
// 실측: `echo 원문 | opencode run -- "요약하라"` 가 원문을 요약해 돌려준다.
//
// 인자 규약(모두 `opencode run --help` 출력 기준):
//   --pure          외부 플러그인 없이 실행 — 사용자의 플러그인이 요약 파이프라인에
//                   끼어들지 않게 한다(codex의 `--ignore-user-config`와 같은 취지).
//   --agent plan    opencode 기본 에이전트(`build`)는 쓰기·실행 도구를 다 쥔다.
//                   순수 텍스트 변환에 그럴 이유가 없어 읽기 지향 내장 에이전트인
//                   `plan`으로 고정한다(codex의 `--sandbox read-only` 대응).
//   --model         `<provider>/<model>` 표기(`opencode models` 출력 형식).
//   --              뒤의 지시문이 플래그로 해석되지 않게 끊는다.
//
// 출력: 진행 배너("> plan · <model>")는 stderr로 나가고 stdout에는 답변 본문만
// ANSI 없이 실린다 — run_with_timeout이 stdout만 읽으므로 추가 정리가 필요 없다.
#[cfg(windows)]
const WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
$OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$c = Get-Command opencode -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
$in = [Console]::In.ReadToEnd()
$aoArgs = @('run', '--pure', '--agent', 'plan', '--model', $env:AO_MODEL, '--', $env:AO_INSTRUCTION)
$in | & $c.Source @aoArgs
exit $LASTEXITCODE"#;

#[cfg(windows)]
pub(super) fn build(instruction: &str, model: &str) -> ProviderCommand {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]);
    command.creation_flags(CREATE_NO_WINDOW);
    command.env("AO_INSTRUCTION", instruction);
    command.env("AO_MODEL", model);
    ProviderCommand {
        command,
        provider: SummaryProvider::Opencode,
    }
}

#[cfg(not(windows))]
pub(super) fn build(instruction: &str, model: &str) -> ProviderCommand {
    let mut command = std::process::Command::new("opencode");
    command.args([
        "run",
        "--pure",
        "--agent",
        "plan",
        "--model",
        model,
        "--",
        instruction,
    ]);
    ProviderCommand {
        command,
        provider: SummaryProvider::Opencode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 지시문이 플래그처럼 생겼어도 opencode 플래그로 해석되면 안 된다 —
    /// `--`로 끊는 이유를 고정한다.
    const DANGEROUS_INSTRUCTION: &str = "--auto";

    #[cfg(not(windows))]
    #[test]
    fn non_windows_command_pins_the_one_shot_read_only_contract() {
        let spec = build("요약 지시", "opencode-go/deepseek-v4-flash");
        assert_eq!(spec.provider, SummaryProvider::Opencode);
        let cmd = spec.command;
        assert_eq!(cmd.get_program(), "opencode");
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            vec![
                "run",
                "--pure",
                "--agent",
                "plan",
                "--model",
                "opencode-go/deepseek-v4-flash",
                "--",
                "요약 지시",
            ]
        );
    }

    /// 모델은 호출측(`summarizer::resolve_model`)이 정해 넘긴다 — 여기서는 그
    /// 값이 왜곡 없이 커맨드로 실리는지만 고정한다(목적별 기본값·설정
    /// 오버라이드 규칙은 mod.rs의 `resolve_model` 테스트가 지킨다).
    #[cfg(not(windows))]
    #[test]
    fn explicit_model_is_passed_through() {
        let spec = build("학습자료 지시", "opencode-go/deepseek-v4-pro");
        let args: Vec<_> = spec
            .command
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert!(
            args.contains(&"opencode-go/deepseek-v4-pro".to_string()),
            "{args:?}"
        );
        assert!(
            !args.contains(&"opencode-go/deepseek-v4-flash".to_string()),
            "{args:?}"
        );
    }

    /// 플래그로 생긴 지시문이 argv 끝(`--` 뒤)에만 실리는지 — 순서가 뒤집히면
    /// opencode가 그것을 자기 옵션으로 먹는다.
    #[cfg(not(windows))]
    #[test]
    fn flag_like_instruction_stays_behind_the_separator() {
        let spec = build(DANGEROUS_INSTRUCTION, "opencode-go/deepseek-v4-flash");
        let args: Vec<_> = spec
            .command
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        let separator = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(args.last().map(String::as_str), Some(DANGEROUS_INSTRUCTION));
        assert_eq!(separator, args.len() - 2, "{args:?}");
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

    /// 지시문은 배열 splat의 마지막 원소(`--` 뒤)로만 실린다 — 문자열 보간으로
    /// 붙이면 PowerShell 파서가 플래그로 볼 수 있다.
    #[cfg(windows)]
    #[test]
    fn windows_script_puts_the_instruction_after_the_separator() {
        assert!(
            WINDOWS_SCRIPT.contains("'--', $env:AO_INSTRUCTION)"),
            "{WINDOWS_SCRIPT}"
        );
        assert!(WINDOWS_SCRIPT.contains("'--agent', 'plan'"), "{WINDOWS_SCRIPT}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_command_uses_powershell_with_no_window_flag_and_env_instruction() {
        let spec = build(DANGEROUS_INSTRUCTION, "opencode-go/deepseek-v4-flash");
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
        let instruction = cmd
            .get_envs()
            .find(|(k, _)| *k == "AO_INSTRUCTION")
            .and_then(|(_, v)| v);
        assert_eq!(
            instruction,
            Some(std::ffi::OsStr::new(DANGEROUS_INSTRUCTION))
        );
        let model = cmd
            .get_envs()
            .find(|(k, _)| *k == "AO_MODEL")
            .and_then(|(_, v)| v);
        assert_eq!(
            model,
            Some(std::ffi::OsStr::new("opencode-go/deepseek-v4-flash"))
        );
    }
}
