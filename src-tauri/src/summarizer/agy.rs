use super::ProviderCommand;
use crate::persistence::settings_store::SummaryProvider;

// agy(Google Antigravity CLI)는 print 모드에서 stdin을 본문으로 읽지 않는다
// (stdin만 주면 "입력을 달라"고 답한다 — 실측). 그래서 claude처럼 stdin에 본문을
// 흘리지 못하고, 셸 래퍼가 stdin을 다 읽어 지시문 뒤에 붙인 한 개의 프롬프트
// 인자로 넘긴다. 래퍼 셸 자체는 항상 spawn에 성공하므로 CLI 부재를 spawn
// NotFound로 알릴 수 없다 — Windows 스크립트와 같은 exit 3 규약을 쓴다
// (run_with_timeout이 code 3을 `-not-found`로 매핑).
//
// 모델명과 무관하게 --effort low를 넘겨 요약 비용을 제한한다.
#[cfg(not(windows))]
const UNIX_SCRIPT: &str = r#"command -v "${AO_PROGRAM}" >/dev/null 2>&1 || exit 3
in=$(cat)
exec "${AO_PROGRAM}" --print "${AO_INSTRUCTION}

${in}" --model "${AO_MODEL}" --effort low --output-format text"#;

#[cfg(windows)]
const WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
$OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$c = Get-Command $env:AO_PROGRAM -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
$in = [Console]::In.ReadToEnd()
& $c.Source --print ($env:AO_INSTRUCTION + "`n`n" + $in) --model $env:AO_MODEL --effort low --output-format text
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
        provider: SummaryProvider::Agy,
    }
}

#[cfg(not(windows))]
pub(super) fn build(program: &str, instruction: &str, model: &str) -> ProviderCommand {
    let mut command = std::process::Command::new("/bin/sh");
    command.args(["-c", UNIX_SCRIPT]);
    command.env("AO_PROGRAM", program);
    command.env("AO_INSTRUCTION", instruction);
    command.env("AO_MODEL", model);
    ProviderCommand {
        command,
        provider: SummaryProvider::Agy,
    }
}

// ── 설정 화면의 모델 카탈로그(`list_provider_models`) ───────────────────────
//
// agy에는 `agy models`("List available models")가 있다. 실측 출력은 첫 줄에
// 진행 문구("Fetching available models...")가 오고, 그 뒤로 한 줄에
// `<id>\t<사람이 읽는 이름>` 형식이다. 요약 실행 경로(`build`)와 달리 stdin도
// 세마포어도 쓰지 않는다.
//
// 파싱은 **탭이 있는 줄만** 취해 첫 필드를 쓴다 — 진행 문구·오류 안내처럼
// 탭이 없는 줄을 모델 id로 오인하지 않게 하는 값싼 방어다.
#[cfg(windows)]
const MODELS_WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference='Stop'
$OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$c = Get-Command $env:AO_PROGRAM -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
& $c.Source models
exit $LASTEXITCODE"#;

#[cfg(windows)]
fn models_command(program: &str) -> tokio::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = tokio::process::Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        MODELS_WINDOWS_SCRIPT,
    ]);
    command.creation_flags(CREATE_NO_WINDOW);
    command.env("AO_PROGRAM", program);
    command
}

#[cfg(not(windows))]
fn models_command(program: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command.arg("models");
    command
}

/// `agy models` stdout → 모델 id 목록. 탭으로 갈린 줄의 첫 필드만 취하고,
/// 중복은 첫 등장 순서로 눌러 담는다. 순수.
pub fn parse_models_stdout(stdout: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let Some((id, _label)) = line.split_once('\t') else {
            continue;
        };
        let id = id.trim();
        if id.is_empty() || out.iter().any(|m| m == id) {
            continue;
        }
        out.push(id.to_string());
    }
    out
}

/// 모델 목록 조회. 실패(미설치·타임아웃·비정상 종료)는 전부 빈 목록이다 —
/// opencode::list_models와 같은 계약이다.
pub async fn list_models(timeout: std::time::Duration, program: &str) -> Vec<String> {
    let mut command = models_command(program);
    command.current_dir(std::env::temp_dir());
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    command.kill_on_drop(true);

    let Ok(child) = command.spawn() else {
        return Vec::new();
    };
    let Ok(Ok(output)) = tokio::time::timeout(timeout, child.wait_with_output()).await else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_models_stdout(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `agy models`의 실측 모양: 첫 줄은 진행 문구(탭 없음), 그 뒤로 `id\t이름`.
    #[test]
    fn parse_models_stdout_takes_the_id_field_and_skips_the_progress_line() {
        let stdout = "Fetching available models...\n                      gemini-3.7-flash-low\tGemini 3.7 Flash (Low)\n                      gemini-3.1-pro-high\tGemini 3.1 Pro (High)\n";
        assert_eq!(
            parse_models_stdout(stdout),
            vec![
                "gemini-3.7-flash-low".to_string(),
                "gemini-3.1-pro-high".to_string(),
            ]
        );
    }

    #[test]
    fn parse_models_stdout_drops_blanks_and_duplicates() {
        let stdout = "\ta\n  x  \tX\nx\tX again\n\n";
        assert_eq!(parse_models_stdout(stdout), vec!["x".to_string()]);
    }

    #[test]
    fn parse_models_stdout_on_empty_input_is_empty_list() {
        assert!(parse_models_stdout("").is_empty());
        assert!(parse_models_stdout("no tabs here\nnor here\n").is_empty());
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_models_command_invokes_the_models_subcommand_directly() {
        let cmd = models_command("agy");
        assert_eq!(cmd.as_std().get_program(), "agy");
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(args, vec!["models"]);
    }

    /// agy만 셸 래퍼를 거치므로, 커스텀 명령이 인자가 아니라 **환경변수**로
    /// 들어가고 스크립트가 그것을 따옴표로 감싸 부르는지 함께 못 박는다 —
    /// 공백이 든 경로를 넣어도 단어 분리되지 않아야 한다.
    #[cfg(not(windows))]
    #[test]
    fn custom_program_travels_through_the_env_and_is_quoted_in_the_script() {
        let spec = build("/opt/my tools/agy-t", "요약 지시", "gemini-3.6-flash-low");
        let env = spec
            .command
            .get_envs()
            .find(|(k, _)| *k == "AO_PROGRAM")
            .and_then(|(_, v)| v)
            .map(|v| v.to_string_lossy().to_string());
        assert_eq!(env.as_deref(), Some("/opt/my tools/agy-t"));
        assert!(UNIX_SCRIPT.contains(r#"command -v "${AO_PROGRAM}""#), "{UNIX_SCRIPT}");
        assert!(UNIX_SCRIPT.contains(r#"exec "${AO_PROGRAM}""#), "{UNIX_SCRIPT}");
    }

    const DANGEROUS_INSTRUCTION: &str = "--dangerously-skip-permissions";

    #[cfg(not(windows))]
    #[test]
    fn unix_command_wraps_agy_in_sh_with_env_instruction_and_model() {
        let spec = build("agy", "요약 지시", "gemini-3.6-flash-low");
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

    /// 모델은 이제 호출측(`summarizer::resolve_model`)이 정해 넘긴다 — 여기서는
    /// 그 값이 왜곡 없이 커맨드로 실리는지만 고정한다(목적별 기본값·설정
    /// 오버라이드 규칙은 mod.rs의 `resolve_model` 테스트가 지킨다).
    #[cfg(not(windows))]
    #[test]
    fn explicit_model_is_passed_through() {
        let spec = build("agy", "학습자료 지시", "gemini-3.1-pro-low");
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
        let invocation = UNIX_SCRIPT.find(r#"exec "${AO_PROGRAM}""#).unwrap();
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
        let mut spec = build("agy", DANGEROUS_INSTRUCTION, "gemini-3.6-flash-low");
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
                "--effort",
                "low",
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
        let spec = build("agy", DANGEROUS_INSTRUCTION, "gemini-3.6-flash-low");
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
        assert!(WINDOWS_SCRIPT.contains("--effort low"), "{WINDOWS_SCRIPT}");
    }

    /// 실 CLI 스모크 — `agy models`의 TSV 형식이 바뀌지 않았는지 사람이
    /// 확인할 때 쓴다.
    #[tokio::test]
    #[ignore = "agy CLI 실행 필요(수동 스모크)"]
    async fn live_catalog_smoke() {
        let models = list_models(std::time::Duration::from_secs(30), "agy").await;
        assert!(
            !models.is_empty(),
            "빈 목록 -- 출력 형식이 바뀌었을 수 있다"
        );
        println!("{models:?}");
    }
}
