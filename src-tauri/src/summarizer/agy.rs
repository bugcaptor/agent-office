use super::{ProviderCommand, SummaryPurpose};
use crate::persistence::settings_store::SummaryProvider;

// agy(Google Antigravity CLI)는 print 모드에서 stdin을 본문으로 읽지 않는다
// (stdin만 주면 "입력을 달라"고 답한다 — 실측). 그래서 claude처럼 stdin에 본문을
// 흘리지 못하고, 셸 래퍼가 stdin을 다 읽어 지시문 뒤에 붙인 한 개의 프롬프트
// 인자로 넘긴다. 래퍼 셸 자체는 항상 spawn에 성공하므로 CLI 부재를 spawn
// NotFound로 알릴 수 없다 — Windows 스크립트와 같은 exit 3 규약을 쓴다
// (run_with_timeout이 code 3을 `-not-found`로 매핑).
//
// 모델명은 reasoning effort를 접미로 포함한다(`agy models` 출력 기준,
// 예: gemini-3.6-flash-low) — 별도 --effort 플래그는 쓰지 않는다.
#[cfg(not(windows))]
const UNIX_SCRIPT: &str = r#"command -v agy >/dev/null 2>&1 || exit 3
in=$(cat)
exec agy --print "${AO_INSTRUCTION}

${in}" --model "${AO_MODEL}" --output-format text"#;

#[cfg(windows)]
const WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
$OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$c = Get-Command agy -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
$in = [Console]::In.ReadToEnd()
& $c.Source --print ($env:AO_INSTRUCTION + "`n`n" + $in) --model $env:AO_MODEL --output-format text
exit $LASTEXITCODE"#;

#[cfg(windows)]
pub(super) fn build(instruction: &str, purpose: SummaryPurpose) -> ProviderCommand {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]);
    command.creation_flags(CREATE_NO_WINDOW);
    command.env("AO_INSTRUCTION", instruction);
    command.env("AO_MODEL", purpose.agy_model());
    ProviderCommand {
        command,
        provider: SummaryProvider::Agy,
    }
}

#[cfg(not(windows))]
pub(super) fn build(instruction: &str, purpose: SummaryPurpose) -> ProviderCommand {
    let mut command = std::process::Command::new("/bin/sh");
    command.args(["-c", UNIX_SCRIPT]);
    command.env("AO_INSTRUCTION", instruction);
    command.env("AO_MODEL", purpose.agy_model());
    ProviderCommand {
        command,
        provider: SummaryProvider::Agy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DANGEROUS_INSTRUCTION: &str = "--dangerously-skip-permissions";

    #[cfg(not(windows))]
    #[test]
    fn unix_command_wraps_agy_in_sh_with_env_instruction_and_model() {
        let spec = build("요약 지시", SummaryPurpose::Label);
        assert_eq!(spec.provider, SummaryProvider::Agy);
        let cmd = spec.command;
        assert_eq!(cmd.get_program(), "/bin/sh");
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(args, vec!["-c", UNIX_SCRIPT]);
        let env = |key: &str| {
            cmd.get_envs()
                .find(|(k, _)| *k == key)
                .and_then(|(_, v)| v)
                .map(|v| v.to_string_lossy().to_string())
        };
        assert_eq!(env("AO_INSTRUCTION").as_deref(), Some("요약 지시"));
        assert_eq!(env("AO_MODEL").as_deref(), Some("gemini-3.6-flash-low"));
    }

    /// 학습자료 목적은 같은 파이프라인을 쓰되 더 큰 모델을 고른다.
    #[cfg(not(windows))]
    #[test]
    fn study_purpose_upgrades_the_model() {
        let spec = build("학습자료 지시", SummaryPurpose::Study);
        let model = spec
            .command
            .get_envs()
            .find(|(k, _)| *k == "AO_MODEL")
            .and_then(|(_, v)| v)
            .map(|v| v.to_string_lossy().to_string());
        assert_eq!(model.as_deref(), Some("gemini-3.1-pro-low"));
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_script_gates_missing_cli_and_reads_stdin_before_invoking() {
        let gate = UNIX_SCRIPT.find("|| exit 3").unwrap();
        let stdin_read = UNIX_SCRIPT.find("in=$(cat)").unwrap();
        let invocation = UNIX_SCRIPT.find("exec agy").unwrap();
        assert!(gate < stdin_read && stdin_read < invocation, "{UNIX_SCRIPT}");
    }

    /// sh 래퍼 경계가 지시문·본문·모델을 왜곡 없이 agy argv로 전달하는지,
    /// 악의적 지시문이 플래그로 해석되지 않는지 가짜 agy로 실측 고정한다.
    #[cfg(not(windows))]
    #[test]
    fn unix_sh_boundary_preserves_exact_agy_argv() {
        use std::io::Write as _;
        use std::process::Stdio;

        let dir = std::env::temp_dir().join(format!("ao-agy-argv-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let capture = dir.join("argv.txt");
        {
            use std::os::unix::fs::PermissionsExt;
            let fake = dir.join("agy");
            std::fs::write(
                &fake,
                "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\x00' \"$a\"; done > \"$AO_CAPTURE_FILE\"\nexit 0\n",
            )
            .unwrap();
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let original_path = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(dir.clone()).chain(std::env::split_paths(&original_path)),
        )
        .unwrap();
        let mut spec = build(DANGEROUS_INSTRUCTION, SummaryPurpose::Label);
        spec.command.env("PATH", path);
        spec.command.env("AO_CAPTURE_FILE", &capture);
        spec.command.stdin(Stdio::piped());
        let mut child = spec.command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all("한글 원문".as_bytes())
            .unwrap();
        let status = child.wait().unwrap();
        assert!(status.success());

        let raw = std::fs::read_to_string(&capture).unwrap();
        let actual: Vec<&str> = raw.split('\0').filter(|s| !s.is_empty()).collect();
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(
            actual,
            vec![
                "--print",
                &format!("{DANGEROUS_INSTRUCTION}\n\n한글 원문") as &str,
                "--model",
                "gemini-3.6-flash-low",
                "--output-format",
                "text",
            ]
        );
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
    fn windows_script_reads_stdin_and_merges_it_into_the_prompt_argument() {
        let gate = WINDOWS_SCRIPT.find("[Console]::In.ReadToEnd()").unwrap();
        let invocation = WINDOWS_SCRIPT
            .find("($env:AO_INSTRUCTION + \"`n`n\" + $in)")
            .unwrap();
        assert!(gate < invocation, "{WINDOWS_SCRIPT}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_command_uses_powershell_with_no_window_flag_and_env_instruction() {
        let spec = build(DANGEROUS_INSTRUCTION, SummaryPurpose::Label);
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
    }
}
