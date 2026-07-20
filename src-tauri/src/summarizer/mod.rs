mod claude;
mod codex;

use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;

use crate::persistence::settings_store::SummaryProvider;

const TEXT_MAX_CHARS: usize = 2_000;
const ERROR_MAX_CHARS: usize = 512;
const MAX_CONCURRENT: usize = 2;
/// 라벨 요약(인터랙티브 — 머리 위 라벨). 짧게 잡아 UX 지연을 막는다.
const TIMEOUT_LABEL: Duration = Duration::from_secs(20);
/// 일기 생성(#66). 백그라운드 유휴 스윕에서만 도는 배치라 종료 데드라인이
/// 없다 — 긴 세션도 완주하도록 넉넉히 기다린다.
const TIMEOUT_DIARY: Duration = Duration::from_secs(120);

/// 요약 호출의 목적. 목적별로 타임아웃만 달라지고 나머지 파이프라인은 공유한다(#66).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SummaryPurpose {
    #[default]
    Label,
    Diary,
}

impl SummaryPurpose {
    fn timeout(self) -> Duration {
        match self {
            Self::Label => TIMEOUT_LABEL,
            Self::Diary => TIMEOUT_DIARY,
        }
    }
}

pub(super) struct ProviderCommand {
    pub command: std::process::Command,
    pub provider: SummaryProvider,
}

fn permits() -> &'static Semaphore {
    static PERMITS: OnceLock<Semaphore> = OnceLock::new();
    PERMITS.get_or_init(|| Semaphore::new(MAX_CONCURRENT))
}

/// 초과 입력을 캡한다. 예전에는 앞 `TEXT_MAX_CHARS`자만 남기는 꼬리 절단이라
/// 시간순 append된 작업 로그의 **최신 부분이 통째로 유실**됐다(#66). 이제
/// head 60% + 중략 표시 + tail 40%로 머리(첫 지시)와 꼬리(최근 작업)를 함께
/// 보존한다. 프런트의 우선순위 축소(`formatWorkLog`)가 실패하거나 다른 경로가
/// 긴 입력을 줄 때의 안전망 — 출력은 항상 `TEXT_MAX_CHARS` 이하다.
fn cap_text(text: &str) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("validation: text is empty".to_string());
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= TEXT_MAX_CHARS {
        return Ok(trimmed.to_string());
    }
    const MARKER: &str = "\n…(중략)…\n";
    let marker_len = MARKER.chars().count();
    let budget = TEXT_MAX_CHARS.saturating_sub(marker_len);
    let head_len = budget * 60 / 100;
    let tail_len = budget - head_len;
    let head: String = chars[..head_len].iter().collect();
    let tail: String = chars[chars.len() - tail_len..].iter().collect();
    Ok(format!("{head}{MARKER}{tail}"))
}

fn bounded_detail(detail: &str) -> String {
    detail.trim().chars().take(ERROR_MAX_CHARS).collect()
}

fn missing_error(provider: SummaryProvider) -> String {
    format!("{}-not-found", provider.as_str())
}

pub async fn summarize(
    provider: SummaryProvider,
    purpose: SummaryPurpose,
    instruction: &str,
    text: &str,
) -> Result<String, String> {
    let capped = cap_text(text)?;
    // GUI(Finder/launchd)로 띄운 번들 앱은 프로세스 PATH가 최소값(`/usr/bin:/bin:…`)
    // 이라 `claude`/`codex`를 못 찾아 `-not-found`로 조용히 실패한다(#58과 동일 원인,
    // 요약기·일기 경로에서 재발). spawn 직전에 로그인 셸 PATH를 1회 병합해 보장한다.
    // 멱등이라 첫 호출만 로그인 셸을 돌리고, 블로킹 호출이라 blocking 풀에서 실행한다.
    let _ = tokio::task::spawn_blocking(crate::session::env_capture::ensure_captured).await;
    let command = match provider {
        SummaryProvider::Claude => claude::build(instruction),
        SummaryProvider::Codex => codex::build(instruction),
    };
    run_with_timeout(command, &capped, purpose.timeout()).await
}

async fn run_with_timeout(
    mut spec: ProviderCommand,
    text: &str,
    timeout: Duration,
) -> Result<String, String> {
    let _permit = permits()
        .acquire()
        .await
        .expect("semaphore is never closed");
    let provider = spec.provider;

    spec.command.current_dir(std::env::temp_dir());
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
        let error = summarize(SummaryProvider::Codex, SummaryPurpose::Label, "summarize", "   ")
            .await
            .unwrap_err();
        assert_eq!(error, "validation: text is empty");
    }

    #[test]
    fn purpose_maps_to_distinct_timeouts() {
        assert_eq!(SummaryPurpose::Label.timeout(), TIMEOUT_LABEL);
        assert_eq!(SummaryPurpose::Diary.timeout(), TIMEOUT_DIARY);
        assert!(TIMEOUT_DIARY > TIMEOUT_LABEL);
    }

    #[test]
    fn cap_text_counts_unicode_scalars_not_bytes() {
        let input = "가".repeat(TEXT_MAX_CHARS + 5);
        // head+tail 보존이라 총 길이는 정확히 캡(중략 마커 포함)에 맞춘다.
        assert_eq!(cap_text(&input).unwrap().chars().count(), TEXT_MAX_CHARS);
    }

    #[test]
    fn cap_text_passes_through_when_within_budget() {
        let input = "가".repeat(TEXT_MAX_CHARS);
        assert_eq!(cap_text(&input).unwrap(), input);
    }

    #[test]
    fn cap_text_preserves_both_head_and_tail() {
        // 앞뒤를 구분할 수 있게 머리엔 'H', 꼬리엔 'T'를 채운다.
        let input = format!("{}{}", "H".repeat(TEXT_MAX_CHARS), "T".repeat(TEXT_MAX_CHARS));
        let capped = cap_text(&input).unwrap();
        assert!(capped.starts_with('H'), "머리(첫 지시)가 유실됨");
        assert!(capped.ends_with('T'), "꼬리(최근 작업)가 유실됨");
        assert!(capped.contains("(중략)"), "중략 표시가 없음");
        assert!(capped.chars().count() <= TEXT_MAX_CHARS);
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

        let result = run_with_timeout(spec, "한글 원문", TIMEOUT_LABEL).await.unwrap();

        assert_eq!(result, "Codex fake summary");
        assert_eq!(std::fs::read_to_string(&fake.stdin).unwrap(), "한글 원문");
    }

    #[tokio::test]
    async fn nonzero_provider_returns_provider_error() {
        let _process_lock = PROCESS_TEST_LOCK.lock().await;
        let fake = FakeCliDir::new();
        let spec = fake.provider_command("7", "");

        let error = run_with_timeout(spec, "source text", TIMEOUT_LABEL)
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
        let error = run_with_timeout(spec, "source text", Duration::from_secs(1))
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
