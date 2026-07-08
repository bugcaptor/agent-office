// src-tauri/src/claude_cli.rs
//
// 머리 위 라벨 요약을 OpenRouter 대신 로컬 `claude` CLI 헤드리스
// 호출(-p, haiku, --output-format text, --max-turns 1)로 대체한다. 호출마다
// 사용자의 Claude 구독/크레딧을 소모하므로 그 사실은 호출부
// (src/renderer/labels/summarizer.ts)에서 명시한다. 짧은 시간에 라벨 요청이
// 몰릴 때 프로세스 폭주(OS 자원 + 구독 크레딧 낭비)를 막기 위해 앱 전역
// 세마포어(허용치 2)로 동시 실행 수를 제한한다.
//
// Windows에서는 `claude`가 보통 npm 글로벌 설치의 셸 래퍼(.ps1/.cmd)로
// 깔려 있어 `std::process::Command::new("claude")`가 곧바로 찾지 못하는
// 경우가 흔하다 -- PowerShell을 경유해 `Get-Command`로 실제 실행 파일
// 경로를 찾아 호출한다. 콘솔 창이 튀지 않도록 CREATE_NO_WINDOW를 건다.
// 스크립트는 상수 그대로 실행하고(문자열 보간 없음), 사용자 지시문은
// 환경변수(AO_INSTRUCTION)로만 전달해 셸 인젝션 표면을 없앤다.

use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;

/// 요약 대상 원문 상한(문자 수). pixellab의 DESCRIPTION_MAX_CHARS와 같은
/// 관례 -- 과도한 stdin/프롬프트 크기를 방지.
const TEXT_MAX_CHARS: usize = 2000;
const TIMEOUT: Duration = Duration::from_secs(20);

/// 동시 `claude` 프로세스 수 상한.
fn permits() -> &'static Semaphore {
    static PERMITS: OnceLock<Semaphore> = OnceLock::new();
    PERMITS.get_or_init(|| Semaphore::new(2))
}

#[cfg(windows)]
const WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
$c = Get-Command claude -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
$in = [Console]::In.ReadToEnd()
$in | & $c.Source -p $env:AO_INSTRUCTION --model haiku --output-format text --max-turns 1
exit $LASTEXITCODE"#;

#[cfg(windows)]
fn build_command(instruction: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = std::process::Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.env("AO_INSTRUCTION", instruction);
    cmd
}

#[cfg(not(windows))]
fn build_command(instruction: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new("claude");
    cmd.args([
        "-p",
        instruction,
        "--model",
        "haiku",
        "--output-format",
        "text",
        "--max-turns",
        "1",
    ]);
    cmd
}

/// `claude -p`(haiku, --max-turns 1) 헤드리스 호출로 `text`를 `instruction`
/// 지시에 따라 요약한다. 실패는 코드성 짧은 문자열로 반환한다
/// ("claude-not-found", "timeout" 등) -- 호출부가 메시지 포함 여부로 분기한다.
pub async fn summarize(instruction: &str, text: &str) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("validation: text is empty".to_string());
    }
    let capped: String = trimmed.chars().take(TEXT_MAX_CHARS).collect();

    let _permit = permits().acquire().await.expect("semaphore is never closed");

    let mut cmd = build_command(instruction);
    // 중립 cwd -- claude가 이 프로젝트의 CLAUDE.md를 로드하지 않도록.
    cmd.current_dir(std::env::temp_dir());
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut tokio_cmd: tokio::process::Command = cmd.into();
    tokio_cmd.kill_on_drop(true);

    let mut child = tokio_cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "claude-not-found".to_string()
        } else {
            format!("spawn failed: {e}")
        }
    })?;

    {
        let mut stdin = child.stdin.take().expect("stdin was piped");
        stdin
            .write_all(capped.as_bytes())
            .await
            .map_err(|e| format!("stdin write failed: {e}"))?;
        // 블록 종료 시 stdin drop -> 파이프 닫힘(EOF), claude가 표준입력을
        // 끝까지 읽고 나서 종료하도록 신호를 준다.
    }

    let output = tokio::time::timeout(TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| "timeout".to_string())?
        .map_err(|e| format!("wait failed: {e}"))?;

    if !output.status.success() {
        if output.status.code() == Some(3) {
            return Err("claude-not-found".to_string());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let code = output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        return Err(format!("claude exited {code}: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Err("empty output".to_string());
    }
    Ok(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 네트워크/프로세스 없이 검증 가능한 경로만 테스트한다 (실 CLI 호출 금지).
    #[tokio::test]
    async fn rejects_empty_text() {
        let err = summarize("요약하라", "   ").await.unwrap_err();
        assert_eq!(err, "validation: text is empty");
    }

    #[cfg(windows)]
    #[test]
    fn windows_command_uses_powershell_with_no_window_flag_and_env_instruction() {
        let cmd = build_command("요약 지시");
        assert_eq!(cmd.get_program(), "powershell.exe");
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect();
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
        let cmd = build_command("요약 지시");
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
