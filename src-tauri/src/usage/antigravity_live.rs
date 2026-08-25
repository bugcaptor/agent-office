// src-tauri/src/usage/antigravity_live.rs
//
// Antigravity 사용량 실시간 조회(kbm #2j4, docs/usage-design.md §10).
//
// Claude·Codex와 결정적으로 다른 점: **파일 캐시 미러가 없다.** Antigravity는
// 사용량 스냅샷을 로컬 어디에도 남기지 않아(`~/.gemini/antigravity-cli/` 전체를
// 뒤져 확인, 2026-08-25) 실시간 조회가 성공한 적이 없으면 이 provider는 아예
// 값이 없다. 그래서 codex_live처럼 "실패하면 rollout 값으로 강등"하는 층이
// 없고, 실패는 곧 표시 없음이다 — 그 사정을 UI가 말할 수 있게
// `AntigravityLiveStatus`를 항상 함께 내보낸다.
//
// 조회 수단은 codex_live와 같은 결: **자격증명을 우리가 만지지 않고 CLI에게
// 물어본다.** `agy`(Antigravity CLI)의 print 모드는 슬래시 명령을 그대로
// 실행하고 그 구조화 결과를 JSON으로 돌려준다:
//
//   agy -p /usage --output-format json --print-timeout 30s
//   → {"status":"SUCCESS", "command":{"name":"usage","data":{"groups":[
//        {"name":"Gemini Models","buckets":[{"id":"gemini-weekly",
//           "window":"weekly","remaining_fraction":0.106,
//           "reset_time":"2026-08-29T06:50:27Z"}]}, ...]}}}
//
// 주의할 값 규약 둘:
//   1. `remaining_fraction`은 **잔여**다. 우리 `used_percent`는 그 여집합이다.
//   2. 모델 턴을 돌지 않는다(`usage.total_tokens = 0`) — 사용량을 보려고
//      사용량을 쓰는 일은 없다. 다만 에이전트 백엔드 콜드 스타트 때문에
//      1회 8~10초가 걸린다(실측). codex_live보다도 스로틀이 절실한 이유다.
//
// 왜 gemini CLI는 없는가: 개인 계정의 Gemini Code Assist 무료 티어가
// Antigravity로 이관되면서 gemini CLI의 OAuth 클라이언트가 자격을 잃었다
// (`loadCodeAssist` → UNSUPPORTED_CLIENT, `retrieveUserQuota` → 403
// SUBSCRIPTION_REQUIRED, 2026-08-25 실측). 조회할 한도 자체가 여기(Antigravity)
// 로 옮겨왔으므로 provider도 하나다.

use std::process::Stdio;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;

use super::claude_live::should_fetch;
use super::{parse_iso8601_ms, Provider, ProviderUsage, UsageWindow, UsageWindowKind};

/// 자식 프로세스 전체의 상한. 실측 8~10초(에이전트 백엔드 콜드 스타트)라
/// 넉넉히 잡되 매달리지는 않는다. CLI 자신에게 주는 `--print-timeout`은 이보다
/// 짧게 둬서 우리가 죽이기 전에 CLI가 스스로 정리할 기회를 준다.
const RUN_TIMEOUT: Duration = Duration::from_secs(45);

/// CLI에게 주는 print 모드 상한. 위 `RUN_TIMEOUT`보다 짧아야 한다.
const PRINT_TIMEOUT_ARG: &str = "30s";

/// 진단 detail에 싣는 CLI 오류 문자열 상한(로그·툴팁이 터지지 않게).
const DETAIL_MAX_CHARS: usize = 120;

/// 주간 창의 길이(분). 응답의 `window: "weekly"`를 분으로 정규화한다 —
/// 다른 provider의 `window_minutes`와 같은 단위로 맞춰 둬야 UI가 창 종류를
/// 하드코딩하지 않는다.
const WEEKLY_MINUTES: i64 = 7 * 24 * 60;

/// 5시간 창의 길이(분). 현재 응답에는 주간 버킷만 나오지만 `window` 값이
/// 열려 있는 계약이라 아는 값은 매핑해 둔다.
const SESSION_MINUTES: i64 = 5 * 60;

/// 실시간 조회의 마지막 시도 결과. TS `AntigravityLiveOutcome` 미러(serde
/// snake_case). Codex와 어휘가 겹치지만 **분리해 둔다** — 이쪽은 JSON-RPC가
/// 아니라 print 모드 1회 실행이라 "RPC 오류"가 없고, 대신 CLI가 성공 상태를
/// 붙여 돌려주는 `status` 필드가 실패 갈래를 가른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AntigravityLiveOutcome {
    /// 아직 한 번도 시도하지 않음(부팅 직후 첫 폴링 전).
    #[default]
    NeverAttempted,
    Ok,
    /// `agy` 실행 파일을 찾지 못함(미설치·PATH 밖).
    CliMissing,
    /// 프로세스는 떴는데 실패로 끝났거나 출력이 없었다.
    CliFailed,
    /// 상한 시간 안에 끝나지 않음.
    Timeout,
    /// CLI가 실패 status를 돌려줌(미로그인·계정 문제 등).
    CommandFailed,
    /// 출력은 왔는데 아는 모양이 아님(슬래시 명령 계약 변화).
    UnexpectedResponse,
}

/// 실시간 조회 진단 스냅샷. TS `AntigravityLiveStatus` 미러(camelCase).
/// 스냅샷마다 항상 존재한다 — "모름"은 null이 아니라 `NeverAttempted`.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityLiveStatus {
    pub outcome: AntigravityLiveOutcome,
    /// 사람이 읽을 진단 보조(예: "agy 실행 실패", "not logged in").
    pub detail: Option<String>,
    /// 마지막 시도 시각(epoch ms). 스로틀에 막혀 건너뛴 폴링은 시도가 아니다.
    pub last_attempt_ms: Option<i64>,
    /// 마지막 성공 시각(epoch ms). 한 번도 성공한 적 없으면 null.
    pub last_success_ms: Option<i64>,
}

/// 실시간 조회 실패 하나(사유 + 사람이 읽을 보조 문자열).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AntigravityLiveFailure {
    pub outcome: AntigravityLiveOutcome,
    pub detail: Option<String>,
}

impl AntigravityLiveFailure {
    fn new(outcome: AntigravityLiveOutcome, detail: impl Into<String>) -> Self {
        Self {
            outcome,
            detail: Some(truncate(detail.into())),
        }
    }
}

/// 진단 문자열을 표시 가능한 길이로 자른다(순수).
fn truncate(mut s: String) -> String {
    s = s.trim().replace(['\n', '\r'], " ");
    if s.chars().count() <= DETAIL_MAX_CHARS {
        return s;
    }
    let cut: String = s.chars().take(DETAIL_MAX_CHARS).collect();
    format!("{cut}…")
}

// ── 자식 프로세스 ────────────────────────────────────────────────────────

/// print 모드 인자. 슬래시 명령은 print 모드에서도 확장되므로(`agy --help`의
/// `--disable-slash-commands` 설명이 그 반대를 보증한다) `/usage`를 그대로
/// 프롬프트로 준다.
fn print_args() -> [&'static str; 6] {
    [
        "-p",
        "/usage",
        "--output-format",
        "json",
        "--print-timeout",
        PRINT_TIMEOUT_ARG,
    ]
}

/// CLI를 띄우는 커맨드. Windows는 `agy`가 .cmd 셰임일 수 있어 codex_live와
/// 같은 PowerShell 경유 해석을 쓴다.
#[cfg(windows)]
fn cli_command() -> tokio::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const SCRIPT: &str = r#"$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
$OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$c = Get-Command agy -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
& $c.Source -p /usage --output-format json --print-timeout 30s
exit $LASTEXITCODE"#;
    let mut command = tokio::process::Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT]);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
fn cli_command() -> tokio::process::Command {
    let mut command = tokio::process::Command::new("agy");
    command.args(print_args());
    command
}

/// PATH에서 `agy`를 못 찾았을 때의 마지막 후보. `agy install`이 기본으로
/// 놓는 자리다 — 번들 앱의 최소 PATH가 로그인 셸 PATH로 보강되지 않은
/// 환경(env_capture 실패)에서 이 한 걸음이 유일한 차이가 된다.
#[cfg(not(windows))]
fn fallback_cli_command() -> Option<tokio::process::Command> {
    let home = std::env::var_os("HOME")?;
    let path = std::path::Path::new(&home).join(".local/bin/agy");
    if !path.is_file() {
        return None;
    }
    let mut command = tokio::process::Command::new(path);
    command.args(print_args());
    Some(command)
}

#[cfg(windows)]
fn fallback_cli_command() -> Option<tokio::process::Command> {
    None
}

/// 실시간 조회 1회. 성공하면 `ProviderUsage`(fetched_at_ms = 조회 시각),
/// 실패는 분류된 `AntigravityLiveFailure`다.
pub(super) async fn fetch_live(now_ms: i64) -> Result<ProviderUsage, AntigravityLiveFailure> {
    let output = match run(cli_command()).await {
        // PATH에서 못 찾은 경우에만 설치 기본 경로를 한 번 더 본다. 다른
        // 실패(권한·타임아웃)는 그대로 올린다 — 두 번 띄울 이유가 없다.
        Err(failure) if failure.outcome == AntigravityLiveOutcome::CliMissing => {
            match fallback_cli_command() {
                Some(command) => run(command).await?,
                None => return Err(failure),
            }
        }
        other => other?,
    };
    parse_usage_output(&output, now_ms)
}

/// 자식 하나를 끝까지 돌리고 stdout을 문자열로. 자식은 어떤 갈래로 끝나든
/// `kill_on_drop`으로 정리된다.
async fn run(mut command: tokio::process::Command) -> Result<String, AntigravityLiveFailure> {
    // 워크스페이스를 건드리지 않도록 중립적인 cwd에서 띄운다(codex_live와 같은
    // 규율) — print 모드는 cwd를 프로젝트로 잡으려 든다.
    command.current_dir(std::env::temp_dir());
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.kill_on_drop(true);

    let child = command.spawn().map_err(|e| {
        let outcome = if e.kind() == std::io::ErrorKind::NotFound {
            AntigravityLiveOutcome::CliMissing
        } else {
            AntigravityLiveOutcome::CliFailed
        };
        AntigravityLiveFailure::new(outcome, e.to_string())
    })?;

    let output = match tokio::time::timeout(RUN_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Err(AntigravityLiveFailure::new(
                AntigravityLiveOutcome::CliFailed,
                e.to_string(),
            ))
        }
        Err(_) => {
            return Err(AntigravityLiveFailure {
                outcome: AntigravityLiveOutcome::Timeout,
                detail: Some("시간 초과".into()),
            })
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if stdout.trim().is_empty() {
        // Windows 셰임 스크립트는 `agy`를 못 찾으면 3으로 끝난다.
        let outcome = if output.status.code() == Some(3) {
            AntigravityLiveOutcome::CliMissing
        } else {
            AntigravityLiveOutcome::CliFailed
        };
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let detail = if stderr.trim().is_empty() {
            format!("출력 없음(종료 코드 {:?})", output.status.code())
        } else {
            stderr
        };
        return Err(AntigravityLiveFailure::new(outcome, detail));
    }
    Ok(stdout)
}

// ── 응답 파싱(순수) ──────────────────────────────────────────────────────

/// print 모드 stdout → `ProviderUsage`. stdout에 배너 같은 비-JSON 줄이 섞일
/// 수 있어 **마지막 JSON 객체 줄**을 결과로 삼는다(print 모드의 결과는 항상
/// 마지막에 온다).
pub(super) fn parse_usage_output(
    stdout: &str,
    now_ms: i64,
) -> Result<ProviderUsage, AntigravityLiveFailure> {
    let value = stdout
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .filter(Value::is_object)
        .ok_or_else(|| {
            AntigravityLiveFailure::new(
                AntigravityLiveOutcome::UnexpectedResponse,
                "JSON 결과 줄 없음",
            )
        })?;

    // status는 명령 자체의 성패다. SUCCESS가 아니면 대개 미로그인·계정 문제라
    // 파싱을 더 해 봐야 의미가 없다.
    let status = value.get("status").and_then(Value::as_str).unwrap_or("");
    if !status.is_empty() && !status.eq_ignore_ascii_case("SUCCESS") {
        let detail = value
            .get("error")
            .and_then(Value::as_str)
            .or_else(|| value.get("response").and_then(Value::as_str))
            .unwrap_or(status);
        return Err(AntigravityLiveFailure::new(
            AntigravityLiveOutcome::CommandFailed,
            detail,
        ));
    }

    let groups = value
        .get("command")
        .and_then(|c| c.get("data"))
        .and_then(|d| d.get("groups"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AntigravityLiveFailure::new(
                AntigravityLiveOutcome::UnexpectedResponse,
                "command.data.groups 없음",
            )
        })?;

    let mut windows = Vec::new();
    for group in groups {
        let label = group
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(buckets) = group.get("buckets").and_then(Value::as_array) else {
            continue;
        };
        for bucket in buckets {
            if let Some(window) = parse_bucket(bucket, label) {
                windows.push(window);
            }
        }
    }
    if windows.is_empty() {
        return Err(AntigravityLiveFailure::new(
            AntigravityLiveOutcome::UnexpectedResponse,
            "아는 한도 버킷이 없음",
        ));
    }
    Ok(ProviderUsage {
        provider: Provider::Antigravity,
        fetched_at_ms: now_ms,
        // 응답에 플랜 표시명이 없다. 그룹 설명이 "Your weekly limit is tied
        // directly to your individual tier"라고만 할 뿐 티어 이름은 안 준다.
        plan_label: None,
        windows,
    })
}

/// 버킷 하나 → 윈도. `remaining_fraction`이 **잔여**라는 것이 핵심이다.
/// 모든 버킷이 모델 그룹 소속이라 창 종류는 항상 모델별 갈래로 낸다 —
/// 그래야 라벨(그룹명) 없이는 뜻이 서지 않는 값임이 UI에 드러난다.
fn parse_bucket(bucket: &Value, label: Option<&str>) -> Option<UsageWindow> {
    let remaining = bucket.get("remaining_fraction").and_then(Value::as_f64)?;
    let window = bucket.get("window").and_then(Value::as_str).unwrap_or("");
    let (kind, window_minutes) = match window {
        "weekly" => (UsageWindowKind::WeeklyModel, Some(WEEKLY_MINUTES)),
        "session" | "five_hour" => (UsageWindowKind::SessionModel, Some(SESSION_MINUTES)),
        _ => (UsageWindowKind::Unknown, None),
    };
    let used_percent = ((1.0 - remaining) * 100.0).clamp(0.0, 100.0);
    Some(UsageWindow {
        kind,
        label: label.map(str::to_string),
        used_percent,
        resets_at_ms: bucket
            .get("reset_time")
            .and_then(Value::as_str)
            .and_then(parse_iso8601_ms),
        window_minutes,
        // Antigravity 응답에는 is_active 개념이 없다.
        is_active: None,
    })
}

// ── 스로틀 상태 ──────────────────────────────────────────────────────────

/// 실시간 조회 메모리 상태. Claude·Codex와 같은 규율: 판단·기록만 락 안에서
/// 하고 자식 프로세스 왕복은 락 밖에서 한다.
///
/// 파일 캐시가 없는 provider라 `last_success`가 곧 표시값이다 — 실패해도
/// 지우지 않는 이유가 다른 provider보다 강하다(지우면 화면에서 사라진다).
#[derive(Default)]
pub struct AntigravityLiveState {
    inner: Mutex<AntigravityLiveInner>,
}

#[derive(Default)]
struct AntigravityLiveInner {
    last_success: Option<ProviderUsage>,
    last_attempt_ms: Option<i64>,
    outcome: AntigravityLiveOutcome,
    detail: Option<String>,
}

impl AntigravityLiveState {
    /// 스로틀 통과 시 `last_attempt_ms`를 먼저 갱신하고 true. 판단+갱신이 한
    /// 락 안에 있어 60초 폴링이 겹쳐도 자식 프로세스는 하나만 뜬다.
    pub(crate) fn begin_attempt_if_due(&self, now_ms: i64) -> bool {
        let mut guard = self.inner.lock();
        let due = should_fetch(guard.last_success.as_ref(), guard.last_attempt_ms, now_ms);
        if due {
            guard.last_attempt_ms = Some(now_ms);
        }
        due
    }

    pub(super) fn record_success(&self, usage: ProviderUsage) {
        let mut guard = self.inner.lock();
        guard.last_success = Some(usage);
        guard.outcome = AntigravityLiveOutcome::Ok;
        guard.detail = None;
    }

    /// 실패 사유를 기록한다. `last_success`는 건드리지 않는다 — 이 provider의
    /// 유일한 표시값이라 지우면 화면에서 통째로 사라진다.
    pub(super) fn record_failure(&self, failure: AntigravityLiveFailure) {
        let mut guard = self.inner.lock();
        guard.outcome = failure.outcome;
        guard.detail = failure.detail;
    }

    pub(super) fn status(&self) -> AntigravityLiveStatus {
        let guard = self.inner.lock();
        AntigravityLiveStatus {
            outcome: guard.outcome,
            detail: guard.detail.clone(),
            last_attempt_ms: guard.last_attempt_ms,
            last_success_ms: guard.last_success.as_ref().map(|u| u.fetched_at_ms),
        }
    }

    pub(super) fn last_success(&self) -> Option<ProviderUsage> {
        self.inner.lock().last_success.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `agy -p /usage --output-format json`의 실측 출력(2026-08-25) 축약본.
    const SAMPLE: &str = r#"{"conversation_id":"","status":"SUCCESS","response":"Gemini Models\tWeekly Limit Remaining\t11%\t2026-08-29T06:50:27Z\n","duration_seconds":0,"num_turns":0,"usage":{"input_tokens":0,"output_tokens":0,"total_tokens":0},"command":{"name":"usage","data":{"description":"Within each group, models share a weekly limit.","groups":[{"name":"Gemini Models","description":"Models within this group: Gemini Flash, Gemini Pro","buckets":[{"id":"gemini-weekly","name":"Weekly Limit Remaining","window":"weekly","remaining_fraction":0.10649120062589645,"reset_time":"2026-08-29T06:50:27Z"}]},{"name":"Claude and GPT models","description":"Models within this group: Claude Opus, Claude Sonnet, GPT-OSS","buckets":[{"id":"3p-weekly","name":"Weekly Limit Remaining","window":"weekly","remaining_fraction":1,"reset_time":"2026-09-01T12:39:21Z"}]}]}}}"#;

    #[test]
    fn parses_measured_sample_into_two_weekly_windows() {
        let usage = parse_usage_output(SAMPLE, 5_000).unwrap();
        assert_eq!(usage.provider, Provider::Antigravity);
        assert_eq!(usage.fetched_at_ms, 5_000);
        assert_eq!(usage.windows.len(), 2);

        let gemini = &usage.windows[0];
        assert_eq!(gemini.kind, UsageWindowKind::WeeklyModel);
        assert_eq!(gemini.label.as_deref(), Some("Gemini Models"));
        assert_eq!(gemini.window_minutes, Some(10_080));
        // remaining_fraction은 잔여 — 10.6% 남았으면 89.4% 쓴 것이다.
        assert!((gemini.used_percent - 89.350_879_937_410_35).abs() < 1e-9);
        assert_eq!(gemini.resets_at_ms, parse_iso8601_ms("2026-08-29T06:50:27Z"));

        let third_party = &usage.windows[1];
        assert_eq!(third_party.label.as_deref(), Some("Claude and GPT models"));
        // 잔여 100% = 사용 0%. 부동소수 오차로 -0이 나오지 않아야 한다.
        assert_eq!(third_party.used_percent, 0.0);
    }

    #[test]
    fn ignores_leading_non_json_banner_lines() {
        let stdout = format!("checking for updates...\n{SAMPLE}\n");
        assert!(parse_usage_output(&stdout, 0).is_ok());
    }

    #[test]
    fn non_success_status_becomes_command_failed() {
        let stdout = r#"{"status":"ERROR","error":"not logged in"}"#;
        let failure = parse_usage_output(stdout, 0).unwrap_err();
        assert_eq!(failure.outcome, AntigravityLiveOutcome::CommandFailed);
        assert_eq!(failure.detail.as_deref(), Some("not logged in"));
    }

    #[test]
    fn missing_groups_becomes_unexpected_response() {
        let stdout = r#"{"status":"SUCCESS","response":"hi"}"#;
        let failure = parse_usage_output(stdout, 0).unwrap_err();
        assert_eq!(failure.outcome, AntigravityLiveOutcome::UnexpectedResponse);
    }

    #[test]
    fn empty_buckets_becomes_unexpected_response() {
        let stdout = r#"{"status":"SUCCESS","command":{"name":"usage","data":{"groups":[]}}}"#;
        let failure = parse_usage_output(stdout, 0).unwrap_err();
        assert_eq!(failure.outcome, AntigravityLiveOutcome::UnexpectedResponse);
    }

    #[test]
    fn non_json_output_becomes_unexpected_response() {
        let failure = parse_usage_output("agy: unknown flag\n", 0).unwrap_err();
        assert_eq!(failure.outcome, AntigravityLiveOutcome::UnexpectedResponse);
    }

    #[test]
    fn unknown_window_kind_still_yields_a_window() {
        let stdout = r#"{"status":"SUCCESS","command":{"data":{"groups":[{"name":"G","buckets":[{"window":"monthly","remaining_fraction":0.5}]}]}}}"#;
        let usage = parse_usage_output(stdout, 0).unwrap();
        assert_eq!(usage.windows[0].kind, UsageWindowKind::Unknown);
        assert_eq!(usage.windows[0].window_minutes, None);
        assert_eq!(usage.windows[0].used_percent, 50.0);
    }

    /// 실제 `agy`를 띄워 계약(플래그·출력 모양)이 살아 있는지 확인한다.
    /// 네트워크와 로그인 상태에 의존하고 10초쯤 걸려서 기본으로는 건너뛴다:
    /// `cargo test -p agent-office -- --ignored antigravity_live`.
    #[test]
    #[ignore = "실제 agy CLI와 로그인이 필요하다"]
    fn live_fetch_against_real_cli() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let usage = rt.block_on(fetch_live(1_000)).expect("agy /usage");
        assert_eq!(usage.provider, Provider::Antigravity);
        assert!(!usage.windows.is_empty());
        for w in &usage.windows {
            assert!((0.0..=100.0).contains(&w.used_percent));
        }
    }

    #[test]
    fn state_throttles_second_attempt_within_five_minutes() {
        let state = AntigravityLiveState::default();
        assert!(state.begin_attempt_if_due(1_000_000));
        assert!(!state.begin_attempt_if_due(1_060_000));
    }

    #[test]
    fn failure_keeps_last_success_because_there_is_no_file_cache() {
        let state = AntigravityLiveState::default();
        let usage = parse_usage_output(SAMPLE, 5_000).unwrap();
        state.record_success(usage.clone());
        state.record_failure(AntigravityLiveFailure::new(
            AntigravityLiveOutcome::CliMissing,
            "no agy",
        ));
        assert_eq!(state.last_success(), Some(usage));
        assert_eq!(state.status().outcome, AntigravityLiveOutcome::CliMissing);
        assert_eq!(state.status().last_success_ms, Some(5_000));
    }
}
