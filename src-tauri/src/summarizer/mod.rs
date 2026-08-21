mod agy;
mod claude;
mod codex;
mod gemini;
mod openrouter;

use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;

use crate::persistence::settings_store::{SummaryModels, SummaryProvider};

const TEXT_MAX_CHARS: usize = 2_000;
/// 학습자료(세션 로그 전사)는 세션 한 편을 통째로 넣어야 의미가 있다 —
/// 라벨·일기와 달리 상한이 훨씬 크다(docs/session-log-design.md §5.1).
const TEXT_MAX_CHARS_STUDY: usize = 120_000;
const ERROR_MAX_CHARS: usize = 512;
const MAX_CONCURRENT: usize = 2;
/// 라벨 요약(인터랙티브 — 머리 위 라벨). 짧게 잡아 UX 지연을 막는다.
const TIMEOUT_LABEL: Duration = Duration::from_secs(20);
/// 일기 생성(#66). 백그라운드 유휴 스윕에서만 도는 배치라 종료 데드라인이
/// 없다 — 긴 세션도 완주하도록 넉넉히 기다린다.
const TIMEOUT_DIARY: Duration = Duration::from_secs(120);
/// 학습자료 생성. 사용자가 명시적으로 누르고 기다리는 배치이고, 입력이
/// 60배 크며 출력도 문서 한 편이다.
const TIMEOUT_STUDY: Duration = Duration::from_secs(300);

/// 요약 호출의 목적. 목적별로 타임아웃·입력 상한·모델이 달라지고 나머지
/// 파이프라인은 공유한다(#66).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SummaryPurpose {
    #[default]
    Label,
    Diary,
    Study,
}

impl SummaryPurpose {
    fn timeout(self) -> Duration {
        match self {
            Self::Label => TIMEOUT_LABEL,
            Self::Diary => TIMEOUT_DIARY,
            Self::Study => TIMEOUT_STUDY,
        }
    }

    fn max_chars(self) -> usize {
        match self {
            Self::Label | Self::Diary => TEXT_MAX_CHARS,
            Self::Study => TEXT_MAX_CHARS_STUDY,
        }
    }

    /// 이 목적이 무거운 쪽(학습자료)인지. 설정의 provider별 오버라이드가
    /// light/heavy 두 칸이라 그 선택에 쓰인다.
    fn is_heavy(self) -> bool {
        matches!(self, Self::Study)
    }

    /// 이 목적이 요구하는 모델 등급. 라벨·일기는 한 문단짜리 변환이라 빠른
    /// 모델로 충분하지만, 학습자료는 긴 전사를 읽고 구조화하는 일이다.
    pub(super) fn claude_model(self) -> &'static str {
        match self {
            Self::Label | Self::Diary => "haiku",
            Self::Study => "sonnet",
        }
    }

    pub(super) fn codex_model(self) -> &'static str {
        match self {
            Self::Label | Self::Diary => "gpt-5.4-mini",
            Self::Study => "gpt-5.4",
        }
    }

    pub(super) fn codex_effort(self) -> &'static str {
        match self {
            Self::Label | Self::Diary => "low",
            Self::Study => "medium",
        }
    }

    /// agy(Google Antigravity CLI) 모델명은 reasoning effort를 접미로
    /// 포함한다(`agy models` 출력 기준) — 별도 effort 플래그가 없다.
    pub(super) fn agy_model(self) -> &'static str {
        match self {
            Self::Label | Self::Diary => "gemini-3.6-flash-low",
            Self::Study => "gemini-3.1-pro-low",
        }
    }

    /// gemini CLI의 안정 기본 모델 상수(DEFAULT_GEMINI_FLASH_MODEL /
    /// DEFAULT_GEMINI_MODEL)와 같은 이름을 쓴다.
    pub(super) fn gemini_model(self) -> &'static str {
        match self {
            Self::Label | Self::Diary => "gemini-2.5-flash",
            Self::Study => "gemini-2.5-pro",
        }
    }

    /// OpenRouter 모델 id는 `<벤더>/<모델>` 표기다. TTS 리라이트의
    /// OpenRouter 기본과 같은 계열을 쓴다.
    pub(super) fn openrouter_model(self) -> &'static str {
        match self {
            Self::Label | Self::Diary => "openai/gpt-5.4-mini",
            Self::Study => "openai/gpt-5.4",
        }
    }
}

/// 목적별 하드코딩 기본 모델. 오버라이드 해석(`resolve_model`)의 폴백이자,
/// 어느 provider가 무엇을 기본으로 쓰는지의 단일 출처다.
fn default_model(provider: SummaryProvider, purpose: SummaryPurpose) -> &'static str {
    match provider {
        SummaryProvider::Claude => purpose.claude_model(),
        SummaryProvider::Codex => purpose.codex_model(),
        SummaryProvider::Agy => purpose.agy_model(),
        SummaryProvider::Gemini => purpose.gemini_model(),
        SummaryProvider::Openrouter => purpose.openrouter_model(),
    }
}

/// 설정 오버라이드 우선, 비어 있으면 하드코딩 기본값. 순수.
///
/// 오버라이드는 자유 입력이라 앱이 유효성을 판단하지 않는다 — 오타는 해당
/// CLI가 오류로 알려주고 요약은 원문 폴백으로 강등된다(기존 실패 경로와 동일).
pub(super) fn resolve_model(
    provider: SummaryProvider,
    purpose: SummaryPurpose,
    models: &SummaryModels,
) -> String {
    let o = models.for_provider(provider);
    let picked = if purpose.is_heavy() { &o.heavy } else { &o.light };
    let picked = picked.trim();
    if picked.is_empty() {
        default_model(provider, purpose).to_string()
    } else {
        picked.to_string()
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
fn cap_text(text: &str, max_chars: usize) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("validation: text is empty".to_string());
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= max_chars {
        return Ok(trimmed.to_string());
    }
    const MARKER: &str = "\n…(중략)…\n";
    let marker_len = MARKER.chars().count();
    let budget = max_chars.saturating_sub(marker_len);
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

/// `models`는 설정의 provider별 모델 오버라이드다(빈 문자열 = 기본 모델).
/// 호출측이 설정 캐시에서 값으로 떠서 넘긴다 — 가드를 `.await` 너머로 들고
/// 가지 않기 위해서다.
///
/// `openrouter_key`도 같은 이유로 호출측이 떠서 넘긴다(키 스토어는 Tauri
/// State 안에 있고 이 함수는 State를 모른다). provider가 `Openrouter`가
/// **아니면 무시된다** — 호출측은 그 경우 굳이 키를 읽지 않아도 된다.
pub async fn summarize(
    provider: SummaryProvider,
    purpose: SummaryPurpose,
    instruction: &str,
    text: &str,
    models: &SummaryModels,
    openrouter_key: Option<&str>,
) -> Result<String, String> {
    let capped = cap_text(text, purpose.max_chars())?;
    let model = resolve_model(provider, purpose, models);

    // OpenRouter는 서브프로세스가 아니라 HTTP다 — 로그인 셸 PATH 병합(CLI 전용)도,
    // stdin 파이프도 필요 없다. 세마포어와 목적별 타임아웃만 CLI 경로와 공유한다.
    if provider == SummaryProvider::Openrouter {
        let key = openrouter_key
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .ok_or_else(|| openrouter::KEY_MISSING.to_string())?;
        return run_openrouter(key, purpose, &model, instruction, &capped).await;
    }

    // GUI(Finder/launchd)로 띄운 번들 앱은 프로세스 PATH가 최소값(`/usr/bin:/bin:…`)
    // 이라 `claude`/`codex`를 못 찾아 `-not-found`로 조용히 실패한다(#58과 동일 원인,
    // 요약기·일기 경로에서 재발). spawn 직전에 로그인 셸 PATH를 1회 병합해 보장한다.
    // 멱등이라 첫 호출만 로그인 셸을 돌리고, 블로킹 호출이라 blocking 풀에서 실행한다.
    let _ = tokio::task::spawn_blocking(crate::session::env_capture::ensure_captured).await;
    let command = match provider {
        SummaryProvider::Claude => claude::build(instruction, &model),
        SummaryProvider::Codex => codex::build(instruction, purpose, &model),
        SummaryProvider::Agy => agy::build(instruction, &model),
        SummaryProvider::Gemini => gemini::build(instruction, &model),
        // 위에서 이미 갈라져 나갔다.
        SummaryProvider::Openrouter => unreachable!("openrouter는 HTTP 경로로 처리된다"),
    };
    run_with_timeout(command, &capped, purpose.timeout()).await
}

/// HTTP 경로의 실행 껍데기. CLI 경로(`run_with_timeout`)와 같은 전역 세마포어와
/// 같은 목적별 타임아웃을 쓴다 — 동시 요약 개수와 대기 예산이 provider에 따라
/// 달라지면 안 된다.
async fn run_openrouter(
    api_key: &str,
    purpose: SummaryPurpose,
    model: &str,
    instruction: &str,
    text: &str,
) -> Result<String, String> {
    let _permit = permits()
        .acquire()
        .await
        .expect("semaphore is never closed");
    let timeout = purpose.timeout();
    // reqwest 자체 타임아웃도 같은 값으로 걸지만, 에러 문자열을 CLI 경로와
    // 똑같은 "timeout"으로 맞추기 위해 바깥에서 한 번 더 감싼다.
    tokio::time::timeout(
        timeout,
        openrouter::summarize(api_key, purpose, model, instruction, text, timeout),
    )
    .await
    .map_err(|_| "timeout".to_string())?
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
        let error = summarize(
            SummaryProvider::Codex,
            SummaryPurpose::Label,
            "summarize",
            "   ",
            &SummaryModels::default(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(error, "validation: text is empty");
    }

    // 키가 없으면 네트워크를 만지기 전에 안정 문자열로 실패해야 한다 —
    // 렌더러는 이것을 다른 실패와 똑같이 원문 폴백으로 강등한다.
    #[tokio::test]
    async fn openrouter_without_a_key_fails_before_any_request() {
        for key in [None, Some("   ")] {
            let error = summarize(
                SummaryProvider::Openrouter,
                SummaryPurpose::Label,
                "요약하라",
                "작업 로그",
                &SummaryModels::default(),
                key,
            )
            .await
            .unwrap_err();
            assert_eq!(error, "openrouter-key-missing");
        }
    }

    // ── 모델 오버라이드 ───────────────────────────────────────────────
    #[test]
    fn empty_override_keeps_the_hardcoded_default_per_provider_and_purpose() {
        let none = SummaryModels::default();
        for (provider, label, study) in [
            (SummaryProvider::Claude, "haiku", "sonnet"),
            (SummaryProvider::Codex, "gpt-5.4-mini", "gpt-5.4"),
            (SummaryProvider::Agy, "gemini-3.6-flash-low", "gemini-3.1-pro-low"),
            (SummaryProvider::Gemini, "gemini-2.5-flash", "gemini-2.5-pro"),
            (
                SummaryProvider::Openrouter,
                "openai/gpt-5.4-mini",
                "openai/gpt-5.4",
            ),
        ] {
            assert_eq!(resolve_model(provider, SummaryPurpose::Label, &none), label);
            assert_eq!(resolve_model(provider, SummaryPurpose::Diary, &none), label);
            assert_eq!(resolve_model(provider, SummaryPurpose::Study, &none), study);
        }
    }

    #[test]
    fn override_wins_and_light_heavy_split_follows_the_purpose() {
        let mut models = SummaryModels::default();
        models.codex.light = "gpt-5.4-nano".into();
        models.codex.heavy = "gpt-5.4-pro".into();
        assert_eq!(
            resolve_model(SummaryProvider::Codex, SummaryPurpose::Label, &models),
            "gpt-5.4-nano"
        );
        assert_eq!(
            resolve_model(SummaryProvider::Codex, SummaryPurpose::Diary, &models),
            "gpt-5.4-nano",
            "일기는 라벨과 같은 경량 등급이다"
        );
        assert_eq!(
            resolve_model(SummaryProvider::Codex, SummaryPurpose::Study, &models),
            "gpt-5.4-pro"
        );
        // 다른 provider는 자기 오버라이드만 본다.
        assert_eq!(
            resolve_model(SummaryProvider::Claude, SummaryPurpose::Label, &models),
            "haiku"
        );
    }

    // 한쪽 칸만 채운 흔한 상태에서 빈 칸이 빈 --model 인자로 새어 나가면
    // CLI가 통째로 실패한다 — 빈/공백 오버라이드는 기본값으로 돌아가야 한다.
    #[test]
    fn blank_override_falls_back_instead_of_passing_an_empty_model() {
        let mut models = SummaryModels::default();
        models.gemini.light = "   ".into();
        assert_eq!(
            resolve_model(SummaryProvider::Gemini, SummaryPurpose::Label, &models),
            "gemini-2.5-flash"
        );
        models.gemini.light = "gemini-3-flash".into();
        assert_eq!(
            resolve_model(SummaryProvider::Gemini, SummaryPurpose::Study, &models),
            "gemini-2.5-pro",
            "light만 채웠으면 heavy는 기본값 그대로"
        );
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
        assert_eq!(cap_text(&input, TEXT_MAX_CHARS).unwrap().chars().count(), TEXT_MAX_CHARS);
    }

    #[test]
    fn cap_text_passes_through_when_within_budget() {
        let input = "가".repeat(TEXT_MAX_CHARS);
        assert_eq!(cap_text(&input, TEXT_MAX_CHARS).unwrap(), input);
    }

    #[test]
    fn cap_text_preserves_both_head_and_tail() {
        // 앞뒤를 구분할 수 있게 머리엔 'H', 꼬리엔 'T'를 채운다.
        let input = format!("{}{}", "H".repeat(TEXT_MAX_CHARS), "T".repeat(TEXT_MAX_CHARS));
        let capped = cap_text(&input, TEXT_MAX_CHARS).unwrap();
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
