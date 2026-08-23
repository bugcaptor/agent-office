// src-tauri/src/usage/codex_live.rs
//
// Codex 사용량 실시간 조회(kbm #2h8, docs/usage-design.md §8).
// v1(codex.rs)은 rollout jsonl에 남은 `rate_limits` 스냅샷이라 **CLI가 실제로
// 돌 때만** 갱신된다 — Claude의 `.claude.json` 캐시 미러와 같은 문제(며칠씩
// 멈춘 숫자)를 가진다. 이 모듈은 codex CLI 자신에게 물어 지금 값을 받아온다.
//
// Claude(claude_live.rs)와 결정적으로 다른 점: **자격증명을 우리가 만지지
// 않는다.** codex CLI가 app-server로 노출하는 JSON-RPC에 물어보면 토큰 읽기·
// 갱신·계정 선택을 전부 CLI가 처리한다. 그래서 Keychain 접근도, 비공식 HTTP
// 엔드포인트·UA 위장도, curl 우회 체인도 여기엔 없다.
//
// 프로토콜(codex-cli 0.149 실측, `codex app-server generate-json-schema`):
//   1. `initialize`(clientInfo 필수) → 응답
//   2. `initialized` 알림
//   3. `account/rateLimits/read` → `{rateLimits, rateLimitsByLimitId, ...}`
// 줄바꿈 구분 JSON-RPC를 자식 프로세스 stdio로 주고받는다. 세 줄을 한꺼번에
// 쏟아붓고 stdin을 닫는 방식은 **쓸 수 없다** — 서버가 EOF를 보는 즉시
// 응답 없이 종료한다(실측). 그래서 응답을 읽어 가며 다음 요청을 쓴다.
//
// `app-server`는 experimental 표시가 붙은 서브커맨드다. 그래서 실패든 형식
// 변화든 전부 Err로 눌러 담고, 조립 단계(mod.rs)가 rollout 파서 값으로 조용히
// 강등한다 — 표시가 비는 일은 없고, 왜 낡았는지는 `CodexLiveStatus`가 말한다.
//
// 스로틀은 Claude와 같은 판단(claude_live::should_fetch)을 그대로 쓴다:
// 5분 하한(실패 백오프 겸 60초 폴링 중복 차단) · 15분 정기 리프레시 ·
// 리셋 경계를 지난 윈도가 있으면 조기 조회. 자식 프로세스를 띄우는 경로라
// 폴링마다 도는 일이 없어야 한다는 요구는 Claude보다 오히려 강하다.

use std::process::Stdio;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::claude_live::should_fetch;
use super::{Provider, ProviderUsage, UsageWindow, UsageWindowKind};

/// 3왕복 전체의 상한. 첫 조회는 CLI 콜드 스타트 + 백엔드 왕복이라
/// 0.7초(실측)보다 느릴 수 있어 넉넉히 잡되, 매달리지는 않게 한다.
const RPC_TIMEOUT: Duration = Duration::from_secs(20);

/// 진단 detail에 싣는 CLI 오류 문자열 상한(로그·툴팁이 터지지 않게).
const DETAIL_MAX_CHARS: usize = 120;

/// 실시간 조회의 마지막 시도 결과. TS `CodexLiveOutcome` 미러(serde
/// snake_case). Claude의 `LiveFetchOutcome`와 **일부러 분리했다** — 이쪽 실패는
/// HTTP 상태코드가 아니라 "CLI가 없다/죽었다/모르는 응답을 줬다"의 어휘다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CodexLiveOutcome {
    /// 아직 한 번도 시도하지 않음(부팅 직후 첫 폴링 전).
    #[default]
    NeverAttempted,
    Ok,
    /// `codex` 실행 파일을 찾지 못함(미설치·PATH 밖).
    CliMissing,
    /// 프로세스는 떴는데 응답 없이 죽었거나 파이프가 끊김.
    CliFailed,
    /// 상한 시간 안에 응답이 오지 않음.
    Timeout,
    /// 서버가 JSON-RPC error를 돌려줌(미로그인·계정 문제 등).
    RpcError,
    /// 응답은 왔는데 아는 모양이 아님(experimental 계약 변화).
    UnexpectedResponse,
}

/// 실시간 조회 진단 스냅샷. TS `CodexLiveStatus` 미러(camelCase).
/// 스냅샷마다 항상 존재한다 — "모름"은 null이 아니라 `NeverAttempted`.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexLiveStatus {
    pub outcome: CodexLiveOutcome,
    /// 사람이 읽을 진단 보조(예: "codex 실행 실패", "not logged in").
    pub detail: Option<String>,
    /// 마지막 시도 시각(epoch ms). 스로틀에 막혀 건너뛴 폴링은 시도가 아니다.
    pub last_attempt_ms: Option<i64>,
    /// 마지막 성공 시각(epoch ms). 한 번도 성공한 적 없으면 null.
    pub last_success_ms: Option<i64>,
}

/// 실시간 조회 실패 하나(사유 + 사람이 읽을 보조 문자열).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CodexLiveFailure {
    pub outcome: CodexLiveOutcome,
    pub detail: Option<String>,
}

impl CodexLiveFailure {
    fn new(outcome: CodexLiveOutcome, detail: impl Into<String>) -> Self {
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

/// app-server를 띄우는 커맨드. Windows는 `codex`가 .cmd 셰임일 수 있어
/// summarizer/codex.rs와 같은 PowerShell 경유 해석을 쓴다(다만 이쪽은 stdin을
/// ReadToEnd 하지 않는다 — 대화형 왕복이라 파이프를 그대로 물려줘야 한다).
#[cfg(windows)]
fn app_server_command() -> tokio::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const SCRIPT: &str = r#"$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
$OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$c = Get-Command codex -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
& $c.Source app-server --listen stdio://
exit $LASTEXITCODE"#;
    let mut command = tokio::process::Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT]);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
fn app_server_command() -> tokio::process::Command {
    let mut command = tokio::process::Command::new("codex");
    command.args(["app-server", "--listen", "stdio://"]);
    command
}

/// 실시간 조회 1회. 성공하면 `ProviderUsage`(fetched_at_ms = 조회 시각),
/// 실패는 분류된 `CodexLiveFailure`다. 자식은 어떤 갈래로 끝나든
/// `kill_on_drop`으로 정리된다.
pub(super) async fn fetch_live(now_ms: i64) -> Result<ProviderUsage, CodexLiveFailure> {
    let mut command = app_server_command();
    // 세션 디렉터리를 건드리지 않도록 중립적인 cwd에서 띄운다
    // (summarizer/codex.rs list_models와 같은 규율).
    command.current_dir(std::env::temp_dir());
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::null());
    command.kill_on_drop(true);

    let mut child = command
        .spawn()
        .map_err(|e| CodexLiveFailure::new(CodexLiveOutcome::CliMissing, e.to_string()))?;

    let result = match tokio::time::timeout(RPC_TIMEOUT, exchange(&mut child)).await {
        Ok(result) => result,
        Err(_) => Err(CodexLiveFailure {
            outcome: CodexLiveOutcome::Timeout,
            detail: Some("시간 초과".into()),
        }),
    };
    // 응답을 얻었든 실패했든 서버는 계속 살아 있다(stdin EOF를 봐야 끝난다).
    // drop만 믿지 않고 명시적으로 죽인다.
    let _ = child.start_kill();
    let value = result?;
    parse_rate_limits(&value, now_ms).ok_or_else(|| {
        CodexLiveFailure::new(
            CodexLiveOutcome::UnexpectedResponse,
            "아는 한도 필드가 없음",
        )
    })
}

/// initialize → initialized → account/rateLimits/read 왕복. 관심 없는 알림
/// (`remoteControl/status/changed` 등)은 흘려보낸다.
async fn exchange(child: &mut tokio::process::Child) -> Result<Value, CodexLiveFailure> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| CodexLiveFailure::new(CodexLiveOutcome::CliFailed, "stdin 없음"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CodexLiveFailure::new(CodexLiveOutcome::CliFailed, "stdout 없음"))?;
    let mut lines = BufReader::new(stdout).lines();

    write_line(&mut stdin, &initialize_request()).await?;
    let mut requested = false;
    loop {
        let line = lines
            .next_line()
            .await
            .map_err(|e| CodexLiveFailure::new(CodexLiveOutcome::CliFailed, e.to_string()))?;
        let Some(line) = line else {
            return Err(CodexLiveFailure::new(
                CodexLiveOutcome::CliFailed,
                "응답 없이 종료",
            ));
        };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue; // JSON이 아닌 줄(배너 등)은 무시.
        };
        match msg.get("id").and_then(Value::as_i64) {
            Some(1) if !requested => {
                if let Some(failure) = rpc_error(&msg) {
                    return Err(failure);
                }
                write_line(&mut stdin, &initialized_notification()).await?;
                write_line(&mut stdin, &rate_limits_request()).await?;
                requested = true;
            }
            Some(2) => {
                if let Some(failure) = rpc_error(&msg) {
                    return Err(failure);
                }
                return msg.get("result").cloned().ok_or_else(|| {
                    CodexLiveFailure::new(CodexLiveOutcome::UnexpectedResponse, "result 없음")
                });
            }
            _ => {}
        }
    }
}

/// 응답에 JSON-RPC error가 실려 있으면 실패로 옮긴다(순수).
fn rpc_error(msg: &Value) -> Option<CodexLiveFailure> {
    let err = msg.get("error")?;
    let text = err
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| err.to_string());
    Some(CodexLiveFailure::new(CodexLiveOutcome::RpcError, text))
}

async fn write_line(
    stdin: &mut tokio::process::ChildStdin,
    value: &Value,
) -> Result<(), CodexLiveFailure> {
    let mut line = value.to_string();
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| CodexLiveFailure::new(CodexLiveOutcome::CliFailed, e.to_string()))?;
    stdin
        .flush()
        .await
        .map_err(|e| CodexLiveFailure::new(CodexLiveOutcome::CliFailed, e.to_string()))
}

fn initialize_request() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": "agent-office",
                "title": "Agent Office",
                "version": env!("CARGO_PKG_VERSION"),
            }
        }
    })
}

fn initialized_notification() -> Value {
    json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} })
}

fn rate_limits_request() -> Value {
    json!({ "jsonrpc": "2.0", "id": 2, "method": "account/rateLimits/read", "params": {} })
}

// ── 응답 파싱(순수) ──────────────────────────────────────────────────────

/// `account/rateLimits/read` 결과 → `ProviderUsage`.
///
/// 두 층을 모두 읽는다:
/// - `rateLimits` — 기본 버킷. rollout 스냅샷과 같은 의미의 값이라 라벨 없이
///   session/weekly 창으로 낸다(표시 의미가 v1과 동일하게 유지된다).
/// - `rateLimitsByLimitId` — 모델별 버킷(예: `codex_bengalfox` =
///   "GPT-5.3-Codex-Spark"). 기본 버킷과 같은 `limitId`는 중복이라 건너뛰고,
///   나머지는 `limitName`을 라벨로 달아 session_model/weekly_model 창으로 낸다
///   (rollout 경로에는 아예 없던 정보다).
///
/// 순회 순서는 serde_json Map(BTreeMap)의 키 정렬이라 결정적이다.
/// 유효한 창이 하나도 없으면 None → 호출자가 rollout 값으로 강등한다.
pub(super) fn parse_rate_limits(result: &Value, now_ms: i64) -> Option<ProviderUsage> {
    let main = result.get("rateLimits")?;
    let main_id = main.get("limitId").and_then(Value::as_str);
    let mut windows = Vec::new();
    push_bucket_windows(&mut windows, main, None);

    if let Some(map) = result.get("rateLimitsByLimitId").and_then(Value::as_object) {
        for (id, snap) in map {
            let id = snap.get("limitId").and_then(Value::as_str).unwrap_or(id);
            if Some(id) == main_id {
                continue;
            }
            let label = snap
                .get("limitName")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(id);
            push_bucket_windows(&mut windows, snap, Some(label));
        }
    }
    if windows.is_empty() {
        return None;
    }
    Some(ProviderUsage {
        provider: Provider::Codex,
        fetched_at_ms: now_ms,
        plan_label: plan_label(result),
        windows,
    })
}

/// 버킷 하나의 primary/secondary 창을 담는다.
fn push_bucket_windows(out: &mut Vec<UsageWindow>, bucket: &Value, label: Option<&str>) {
    for key in ["primary", "secondary"] {
        if let Some(win) = bucket.get(key).and_then(|w| parse_window(w, label)) {
            out.push(win);
        }
    }
}

/// `{usedPercent, windowDurationMins, resetsAt}` → 윈도. usedPercent 부재는
/// None. `resetsAt`은 유닉스 **초**라 ×1000(rollout의 `resets_at`과 같다).
/// 라벨이 붙은(=모델별) 버킷은 같은 창 길이라도 종류를 구분해 낸다 — 뱃지의
/// "5시간 창" 자리를 모델별 창이 차지하지 않게 하는 것이 목적이다.
fn parse_window(w: &Value, label: Option<&str>) -> Option<UsageWindow> {
    let used_percent = w.get("usedPercent").and_then(Value::as_f64)?;
    let window_minutes = w.get("windowDurationMins").and_then(Value::as_i64);
    let kind = match (window_minutes, label.is_some()) {
        (Some(300), false) => UsageWindowKind::Session,
        (Some(300), true) => UsageWindowKind::SessionModel,
        (Some(10080), false) => UsageWindowKind::Weekly,
        (Some(10080), true) => UsageWindowKind::WeeklyModel,
        _ => UsageWindowKind::Unknown,
    };
    let resets_at_ms = w
        .get("resetsAt")
        .and_then(Value::as_i64)
        .map(|secs| secs * 1000);
    Some(UsageWindow {
        kind,
        label: label.map(str::to_string),
        used_percent,
        resets_at_ms,
        window_minutes,
        // Codex rate limit 응답에는 is_active 개념이 없다.
        is_active: None,
    })
}

/// plan 표시명. 기본 버킷의 `planType`을 쓰고, 없으면 다른 버킷에서 아무거나
/// 하나(모든 버킷이 같은 계정의 것이라 값이 갈릴 이유가 없다).
fn plan_label(result: &Value) -> Option<String> {
    let from = |v: &Value| {
        v.get("planType")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && *s != "unknown")
            .map(str::to_string)
    };
    if let Some(label) = result.get("rateLimits").and_then(from) {
        return Some(label);
    }
    result
        .get("rateLimitsByLimitId")
        .and_then(Value::as_object)
        .and_then(|map| map.values().find_map(from))
}

// ── 스로틀 상태 ──────────────────────────────────────────────────────────

/// 실시간 조회 메모리 상태. Claude(`LiveUsageState`)와 같은 규율:
/// 판단·기록만 락 안에서 하고 자식 프로세스 왕복은 락 밖에서 한다.
#[derive(Default)]
pub struct CodexLiveState {
    inner: Mutex<CodexLiveInner>,
}

#[derive(Default)]
struct CodexLiveInner {
    last_success: Option<ProviderUsage>,
    last_attempt_ms: Option<i64>,
    outcome: CodexLiveOutcome,
    detail: Option<String>,
}

impl CodexLiveState {
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
        guard.outcome = CodexLiveOutcome::Ok;
        guard.detail = None;
    }

    /// 실패 사유를 기록한다. `last_success`는 건드리지 않는다 — 마지막으로
    /// 받아 둔 값이 rollout 스냅샷보다 신선할 수 있어 계속 쓰인다.
    pub(super) fn record_failure(&self, failure: CodexLiveFailure) {
        let mut guard = self.inner.lock();
        guard.outcome = failure.outcome;
        guard.detail = failure.detail;
    }

    pub(super) fn status(&self) -> CodexLiveStatus {
        let guard = self.inner.lock();
        CodexLiveStatus {
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

    /// `account/rateLimits/read`의 실측 응답(codex-cli 0.149) 축약본.
    fn sample() -> Value {
        serde_json::from_str(
            r#"{
              "rateLimits": {
                "limitId": "codex",
                "limitName": null,
                "primary": {"usedPercent": 29, "windowDurationMins": 10080, "resetsAt": 1787998886},
                "secondary": null,
                "credits": {"hasCredits": false, "unlimited": false, "balance": "0"},
                "planType": "prolite"
              },
              "rateLimitsByLimitId": {
                "codex_bengalfox": {
                  "limitId": "codex_bengalfox",
                  "limitName": "GPT-5.3-Codex-Spark",
                  "primary": {"usedPercent": 4, "windowDurationMins": 300, "resetsAt": 1787497554},
                  "secondary": {"usedPercent": 7, "windowDurationMins": 10080, "resetsAt": 1788004794},
                  "planType": "prolite"
                },
                "codex": {
                  "limitId": "codex",
                  "primary": {"usedPercent": 29, "windowDurationMins": 10080, "resetsAt": 1787998886},
                  "planType": "prolite"
                }
              },
              "rateLimitResetCredits": {"availableCount": 1, "credits": []}
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn parses_main_bucket_and_named_model_buckets() {
        let usage = parse_rate_limits(&sample(), 5_000).unwrap();
        assert_eq!(usage.provider, Provider::Codex);
        assert_eq!(usage.fetched_at_ms, 5_000, "조회 시각이 신선도가 된다");
        assert_eq!(usage.plan_label.as_deref(), Some("prolite"));
        // 기본 버킷 1창 + 모델 버킷 2창. 기본 버킷과 limitId가 같은 항목은
        // 중복이라 건너뛴다.
        assert_eq!(usage.windows.len(), 3);
        let main = &usage.windows[0];
        assert_eq!(main.kind, UsageWindowKind::Weekly);
        assert_eq!(main.label, None);
        assert_eq!(main.used_percent, 29.0);
        assert_eq!(main.window_minutes, Some(10080));
        assert_eq!(main.resets_at_ms, Some(1_787_998_886_000), "초→ms");
        assert_eq!(main.is_active, None);
        let model_session = &usage.windows[1];
        assert_eq!(model_session.kind, UsageWindowKind::SessionModel);
        assert_eq!(model_session.label.as_deref(), Some("GPT-5.3-Codex-Spark"));
        assert_eq!(usage.windows[2].kind, UsageWindowKind::WeeklyModel);
        assert_eq!(usage.windows[2].used_percent, 7.0);
    }

    #[test]
    fn falls_back_to_limit_id_when_model_bucket_has_no_name() {
        let v: Value = serde_json::from_str(
            r#"{"rateLimits":{"limitId":"codex","primary":{"usedPercent":1,"windowDurationMins":10080}},
                "rateLimitsByLimitId":{"codex_other":{"limitId":"codex_other","limitName":"   ",
                  "primary":{"usedPercent":2,"windowDurationMins":300}}}}"#,
        )
        .unwrap();
        let usage = parse_rate_limits(&v, 0).unwrap();
        assert_eq!(usage.windows[1].label.as_deref(), Some("codex_other"));
    }

    #[test]
    fn unknown_window_length_keeps_the_raw_minutes() {
        let v: Value = serde_json::from_str(
            r#"{"rateLimits":{"primary":{"usedPercent":50,"windowDurationMins":1440}}}"#,
        )
        .unwrap();
        let usage = parse_rate_limits(&v, 0).unwrap();
        assert_eq!(usage.windows[0].kind, UsageWindowKind::Unknown);
        assert_eq!(usage.windows[0].window_minutes, Some(1440));
        assert_eq!(usage.windows[0].resets_at_ms, None, "resetsAt 부재는 null");
    }

    #[test]
    fn plan_label_ignores_unknown_and_falls_back_to_a_named_bucket() {
        let v: Value = serde_json::from_str(
            r#"{"rateLimits":{"primary":{"usedPercent":1,"windowDurationMins":300},"planType":"unknown"},
                "rateLimitsByLimitId":{"codex":{"limitId":"codex","planType":"pro",
                  "primary":{"usedPercent":1,"windowDurationMins":300}}}}"#,
        )
        .unwrap();
        assert_eq!(parse_rate_limits(&v, 0).unwrap().plan_label.as_deref(), Some("pro"));
    }

    /// experimental 서브커맨드라 형식이 바뀔 수 있다 — 그때는 None으로
    /// 강등해 rollout 값이 그대로 쓰이게 한다(패닉·빈 표시 금지).
    #[test]
    fn contract_changes_degrade_to_none() {
        assert!(parse_rate_limits(&json!({}), 0).is_none());
        assert!(parse_rate_limits(&json!({"rateLimits": {}}), 0).is_none());
        assert!(parse_rate_limits(&json!({"rateLimits": {"primary": {}}}), 0).is_none());
        assert!(parse_rate_limits(&json!({"rateLimits": "nope"}), 0).is_none());
    }

    #[test]
    fn rpc_error_response_becomes_a_classified_failure() {
        let msg = json!({"id": 2, "error": {"code": -32603, "message": "not logged in"}});
        let failure = rpc_error(&msg).unwrap();
        assert_eq!(failure.outcome, CodexLiveOutcome::RpcError);
        assert_eq!(failure.detail.as_deref(), Some("not logged in"));
        assert!(rpc_error(&json!({"id": 2, "result": {}})).is_none());
    }

    #[test]
    fn detail_is_trimmed_and_flattened() {
        let long = "x".repeat(DETAIL_MAX_CHARS + 10);
        let failure = CodexLiveFailure::new(CodexLiveOutcome::CliFailed, format!(" a\nb \n{long}"));
        let detail = failure.detail.unwrap();
        assert!(!detail.contains('\n'));
        assert!(detail.starts_with("a b "));
        assert_eq!(detail.chars().count(), DETAIL_MAX_CHARS + 1, "말줄임 1자 포함");
    }

    #[test]
    fn state_throttles_and_reports_status() {
        let state = CodexLiveState::default();
        assert_eq!(state.status().outcome, CodexLiveOutcome::NeverAttempted);
        assert!(state.begin_attempt_if_due(1_000), "첫 시도는 항상 허용");
        assert!(!state.begin_attempt_if_due(2_000), "5분 하한 안에서는 재시도 없음");

        state.record_failure(CodexLiveFailure::new(CodexLiveOutcome::CliMissing, "없음"));
        let status = state.status();
        assert_eq!(status.outcome, CodexLiveOutcome::CliMissing);
        assert_eq!(status.last_attempt_ms, Some(1_000));
        assert_eq!(status.last_success_ms, None);

        state.record_success(ProviderUsage {
            provider: Provider::Codex,
            fetched_at_ms: 9_000,
            plan_label: None,
            windows: vec![],
        });
        let status = state.status();
        assert_eq!(status.outcome, CodexLiveOutcome::Ok);
        assert_eq!(status.detail, None);
        assert_eq!(status.last_success_ms, Some(9_000));
        assert_eq!(state.last_success().unwrap().fetched_at_ms, 9_000);
    }

    /// 실패는 마지막 성공 값을 지우지 않는다 — 그 값이 rollout보다 신선할 수
    /// 있고, 왜 안 움직이는지는 outcome이 말한다.
    #[test]
    fn failure_keeps_the_last_success_snapshot() {
        let state = CodexLiveState::default();
        state.record_success(ProviderUsage {
            provider: Provider::Codex,
            fetched_at_ms: 9_000,
            plan_label: None,
            windows: vec![],
        });
        state.record_failure(CodexLiveFailure::new(CodexLiveOutcome::Timeout, "시간 초과"));
        assert!(state.last_success().is_some());
        assert_eq!(state.status().last_success_ms, Some(9_000));
    }

    /// 실제 codex CLI를 부르는 스모크(수동 실행 전용):
    ///   cargo test -p agent-office --lib -- --ignored codex_live_smoke
    #[tokio::test]
    #[ignore]
    async fn codex_live_smoke() {
        let usage = fetch_live(chrono::Utc::now().timestamp_millis()).await;
        println!("{usage:#?}");
        assert!(usage.is_ok());
    }
}
