mod claude;
mod codex;

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;

use crate::persistence::settings_store::SummaryProvider;

const TEXT_MAX_CHARS: usize = 2_000;
const ERROR_MAX_CHARS: usize = 512;
const MAX_CONCURRENT: usize = 2;
const TIMEOUT: Duration = Duration::from_secs(20);
/// 실험 툴 모드는 여러 턴 동안 파일을 훑으므로 여유 있는 상한을 준다.
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) struct ProviderCommand {
    pub command: std::process::Command,
    pub provider: SummaryProvider,
}

fn permits() -> &'static Semaphore {
    static PERMITS: OnceLock<Semaphore> = OnceLock::new();
    PERMITS.get_or_init(|| Semaphore::new(MAX_CONCURRENT))
}

fn cap_text(text: &str) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("validation: text is empty".to_string());
    }
    Ok(trimmed.chars().take(TEXT_MAX_CHARS).collect())
}

fn bounded_detail(detail: &str) -> String {
    detail.trim().chars().take(ERROR_MAX_CHARS).collect()
}

fn missing_error(provider: SummaryProvider) -> String {
    format!("{}-not-found", provider.as_str())
}

/// `tool_cwd` 는 호출부(command)가 이미 "설정 ON + provider=Claude" 를 확인한
/// 경우에만 Some 이다. 여기서 디렉터리 실존을 마지막으로 검증해, 없으면 조용히
/// 플레인 모드로 강등한다(에러 아님). Codex 경로는 tool_cwd 와 무관하게 불변.
pub async fn summarize(
    provider: SummaryProvider,
    instruction: &str,
    text: &str,
    tool_cwd: Option<PathBuf>,
) -> Result<String, String> {
    let capped = cap_text(text)?;
    // 실존하는 디렉터리일 때만 툴 모드. 그 외에는 플레인.
    let workdir = tool_cwd.filter(|path| path.is_dir());
    let (command, run_dir, timeout) = match provider {
        SummaryProvider::Claude => match workdir {
            Some(dir) => (claude::build_with_tools(instruction), Some(dir), PROBE_TIMEOUT),
            None => (claude::build(instruction), None, TIMEOUT),
        },
        SummaryProvider::Codex => (codex::build(instruction), None, TIMEOUT),
    };
    run_with_timeout(command, &capped, run_dir, timeout).await
}

async fn run_with_timeout(
    mut spec: ProviderCommand,
    text: &str,
    workdir: Option<PathBuf>,
    timeout: Duration,
) -> Result<String, String> {
    let _permit = permits()
        .acquire()
        .await
        .expect("semaphore is never closed");
    let provider = spec.provider;

    spec.command
        .current_dir(workdir.unwrap_or_else(std::env::temp_dir));
    spec.command.stdin(Stdio::piped());
    spec.command.stdout(Stdio::piped());
    spec.command.stderr(Stdio::piped());

    let mut command: tokio::process::Command = spec.command.into();
    command.kill_on_drop(true);
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            missing_error(provider)
        } else {
            format!("spawn failed: {}", bounded_detail(&error.to_string()))
        }
    })?;

    let text = text.as_bytes().to_vec();
    let execution = async move {
        let mut stdin = child.stdin.take().expect("stdin was piped");
        stdin.write_all(&text).await.map_err(|error| {
            format!("stdin write failed: {}", bounded_detail(&error.to_string()))
        })?;
        drop(stdin);

        child
            .wait_with_output()
            .await
            .map_err(|error| format!("wait failed: {}", bounded_detail(&error.to_string())))
    };

    let output = tokio::time::timeout(timeout, execution)
        .await
        .map_err(|_| "timeout".to_string())??;

    if !output.status.success() {
        if output.status.code() == Some(3) {
            return Err(missing_error(provider));
        }
        let code = output
            .status
            .code()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let detail = bounded_detail(&String::from_utf8_lossy(&output.stderr));
        return Err(format!("{} exited {code}: {detail}", provider.as_str()));
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

    static PROCESS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct FakeCliDir {
        root: std::path::PathBuf,
        stdin: std::path::PathBuf,
        pid: std::path::PathBuf,
        cwd: std::path::PathBuf,
    }

    impl FakeCliDir {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "agent-office-fake-summarizer-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&root).unwrap();

            #[cfg(windows)]
            {
                std::fs::write(
                    root.join("codex.ps1"),
                    r#"$ErrorActionPreference='Stop'
[Console]::InputEncoding=[System.Text.Encoding]::UTF8
[IO.File]::WriteAllText($env:AO_FAKE_PID, "$PID")
[IO.File]::WriteAllText($env:AO_FAKE_CWD, (Get-Location).Path)
$in = [Console]::In.ReadToEnd()
[IO.File]::WriteAllText($env:AO_FAKE_STDIN, $in)
if ($env:AO_FAKE_SLEEP_SECONDS) { Start-Sleep -Seconds ([int]$env:AO_FAKE_SLEEP_SECONDS) }
Write-Output 'Codex fake summary'
exit ([int]$env:AO_FAKE_EXIT)
"#,
                )
                .unwrap();
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let codex = root.join("codex");
                std::fs::write(
                    &codex,
                    r#"#!/bin/sh
printf '%s' "$$" > "$AO_FAKE_PID"
pwd -P > "$AO_FAKE_CWD"
cat > "$AO_FAKE_STDIN"
[ -n "$AO_FAKE_SLEEP_SECONDS" ] && sleep "$AO_FAKE_SLEEP_SECONDS"
printf '%s\n' 'Codex fake summary'
exit "$AO_FAKE_EXIT"
"#,
                )
                .unwrap();
                std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
            }

            Self {
                stdin: root.join("codex.stdin"),
                pid: root.join("codex.pid"),
                cwd: root.join("codex.cwd"),
                root,
            }
        }

        fn provider_command(&self, exit: &str, sleep_seconds: &str) -> ProviderCommand {
            #[cfg(windows)]
            let mut command = {
                use std::os::windows::process::CommandExt;
                let windows = std::env::var_os("SystemRoot")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
                let mut command = std::process::Command::new(
                    windows.join("System32/WindowsPowerShell/v1.0/powershell.exe"),
                );
                command.args(["-NoProfile", "-NonInteractive", "-File"]);
                command.arg(self.root.join("codex.ps1"));
                command.creation_flags(0x0800_0000);
                command
            };
            #[cfg(unix)]
            let mut command = std::process::Command::new(self.root.join("codex"));

            command
                .env("AO_FAKE_STDIN", &self.stdin)
                .env("AO_FAKE_PID", &self.pid)
                .env("AO_FAKE_CWD", &self.cwd)
                .env("AO_FAKE_EXIT", exit)
                .env("AO_FAKE_SLEEP_SECONDS", sleep_seconds);

            ProviderCommand {
                command,
                provider: SummaryProvider::Codex,
            }
        }
    }

    impl Drop for FakeCliDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[cfg(windows)]
    fn process_is_running(pid: u32) -> bool {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let script = format!("Get-Process -Id {pid} -ErrorAction Stop | Out-Null");
        std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[cfg(unix)]
    fn process_is_running(pid: u32) -> bool {
        let proc_path = std::path::PathBuf::from(format!("/proc/{pid}"));
        if std::path::Path::new("/proc").is_dir() {
            return proc_path.exists();
        }
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    async fn wait_until_stopped(pid: u32) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while process_is_running(pid) {
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        true
    }

    #[tokio::test]
    async fn rejects_empty_text_before_spawning_a_provider() {
        let error = summarize(SummaryProvider::Codex, "summarize", "   ", None)
            .await
            .unwrap_err();
        assert_eq!(error, "validation: text is empty");
    }

    #[test]
    fn cap_text_counts_unicode_scalars_not_bytes() {
        let input = "가".repeat(TEXT_MAX_CHARS + 5);
        assert_eq!(cap_text(&input).unwrap().chars().count(), TEXT_MAX_CHARS);
    }

    #[test]
    fn error_detail_is_bounded() {
        let bounded = bounded_detail(&"x".repeat(ERROR_MAX_CHARS + 50));
        assert_eq!(bounded.chars().count(), ERROR_MAX_CHARS);
    }

    #[tokio::test]
    async fn fake_provider_preserves_utf8_stdin_and_summary() {
        let _process_lock = PROCESS_TEST_LOCK.lock().await;
        let fake = FakeCliDir::new();
        let spec = fake.provider_command("0", "");

        let result = run_with_timeout(spec, "한글 원문", None, TIMEOUT)
            .await
            .unwrap();

        assert_eq!(result, "Codex fake summary");
        assert_eq!(std::fs::read_to_string(&fake.stdin).unwrap(), "한글 원문");
    }

    #[tokio::test]
    async fn some_workdir_runs_child_in_that_directory() {
        let _process_lock = PROCESS_TEST_LOCK.lock().await;
        let fake = FakeCliDir::new();
        let spec = fake.provider_command("0", "");
        let expected = std::fs::canonicalize(&fake.root).unwrap();

        run_with_timeout(spec, "text", Some(fake.root.clone()), TIMEOUT)
            .await
            .unwrap();

        let recorded = std::fs::read_to_string(&fake.cwd).unwrap();
        assert_eq!(
            std::fs::canonicalize(recorded.trim()).unwrap(),
            expected,
            "child ran outside the requested workdir"
        );
    }

    #[tokio::test]
    async fn none_workdir_runs_child_in_temp_dir() {
        let _process_lock = PROCESS_TEST_LOCK.lock().await;
        let fake = FakeCliDir::new();
        let spec = fake.provider_command("0", "");
        let expected = std::fs::canonicalize(std::env::temp_dir()).unwrap();

        run_with_timeout(spec, "text", None, TIMEOUT)
            .await
            .unwrap();

        let recorded = std::fs::read_to_string(&fake.cwd).unwrap();
        assert_eq!(std::fs::canonicalize(recorded.trim()).unwrap(), expected);
    }

    #[tokio::test]
    async fn nonzero_provider_returns_provider_error() {
        let _process_lock = PROCESS_TEST_LOCK.lock().await;
        let fake = FakeCliDir::new();
        let spec = fake.provider_command("7", "");

        let error = run_with_timeout(spec, "source text", None, TIMEOUT)
            .await
            .unwrap_err();

        assert!(error.starts_with("codex exited 7:"), "{error}");
    }

    #[tokio::test]
    async fn timeout_returns_promptly_and_kills_the_root_process() {
        let _process_lock = PROCESS_TEST_LOCK.lock().await;
        let fake = FakeCliDir::new();
        let spec = fake.provider_command("0", "60");

        let started = std::time::Instant::now();
        let error = run_with_timeout(spec, "source text", None, Duration::from_secs(1))
            .await
            .unwrap_err();

        assert_eq!(error, "timeout");
        assert!(started.elapsed() < Duration::from_secs(3));
        let pid = std::fs::read_to_string(&fake.pid)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(
            wait_until_stopped(pid).await,
            "root process survived timeout"
        );
    }

    #[tokio::test]
    async fn global_semaphore_allows_two_and_blocks_a_third() {
        let _process_lock = PROCESS_TEST_LOCK.lock().await;
        let first = permits().acquire().await.unwrap();
        let second = permits().acquire().await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), permits().acquire())
                .await
                .is_err()
        );
        drop(first);
        assert!(
            tokio::time::timeout(Duration::from_millis(200), permits().acquire())
                .await
                .is_ok()
        );
        drop(second);
    }
}
