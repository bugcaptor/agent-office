// src-tauri/src/usage/claude_live_fallback.rs
//
// 사용량 실시간 조회의 **외부 프로세스 폴백**(이슈 #33 후속).
//
// 왜 필요한가: 앱의 1차 조회(claude_live::fetch_live)는 프로세스 안의 reqwest로
// 직접 HTTPS를 친다. 그런데 사내 MITM 프록시나 self-signed 루트를 OS에 설치한
// 환경에서는 앱만 TLS 핸드셰이크에 실패하고 `claude` CLI·`curl`은 멀쩡히 통하는
// 비대칭이 생긴다. 1차 경로의 루트 CA 신뢰를 OS 스토어까지 넓혀도(Cargo.toml의
// reqwest features) 클라이언트 인증서를 요구하는 프록시 같은 잔여 케이스가 남는다.
// 그때 "이미 그 환경에서 성공하고 있는 남의 프로세스"에 조회를 위임한다.
//
// 두 갈래를 순서대로 시도한다:
//
//   1. `curl` — OS 인증서 스토어와 프록시 설정을 그대로 쓰는 표준 도구.
//      1차 경로와 **똑같은 엔드포인트·헤더·토큰**이라 응답 본문도 동일하고,
//      따라서 파싱은 claude_live의 파서를 그대로 재사용한다. 모델을 호출하지
//      않으므로 구조적으로 토큰을 소모할 여지가 없다.
//   2. `claude -p /usage --output-format json` — CLI 자신에게 묻는다.
//      **현재(claude 2.1.x) 이 경로는 아무 사용량도 돌려주지 않는다**: `-p`
//      모드에서 `/usage`는 인식되지 않고 세션 종료 요약(`/cost`와 동일한
//      문자열)만 나온다. 그래도 넣어둔 이유는 CLI가 나중에 이 조회를 지원하면
//      코드 변경 없이 살아나게 하기 위해서다. 지금은 사실상 "항상 조용히
//      실패하는 마지막 칸"이며, 그래서 순서도 맨 뒤다.
//
// 안전장치(§ 토큰 소모): 2번은 모델을 호출하는 순간 과금이 된다. 지금은 0턴
// 이지만 CLI 정책이 바뀔 수 있으므로, 응답의 `usage`/`total_cost_usd`가 0이
// 아니면 **그 즉시 이 경로를 세션 내내 비활성화**한다(detect_token_spend).
// 여기에 더해 조립 단계가 최소 1시간 간격 스로틀을 건다(claude_live::
// should_try_fallback) — 폴백은 어디까지나 1차 실패의 보험이지 상시 경로가 아니다.
//
// 유령 세션 방지: 앱이 `claude`를 자식으로 띄우면 사용자의 Claude Code 훅이
// 발화해 오피스 씬에 존재하지 않는 세션이 그려질 수 있다. 훅 스크립트는
// `ORCA_*` 환경변수가 없으면 즉시 종료하도록 되어 있으므로, 자식 env에서 그
// 키들을 명시적으로 지워 확실히 차단한다.
//
// 토큰 취급은 1차 경로와 같은 규율을 따른다: 로그·에러 문자열에 절대 넣지
// 않는다. curl에는 인자가 아니라 **stdin(`--config -`)** 으로 넘긴다 — 명령줄
// 인자는 같은 머신의 다른 사용자에게 `ps`로 보인다.

use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncWriteExt;

use super::claude_live::{
    http_failure, parse_live_response, LiveFailure, CLIENT_USER_AGENT, OAUTH_BETA, USAGE_ENDPOINT,
};
use super::{LiveFetchOutcome, UsageWindow};

/// curl 자체의 전송 상한(`max-time`). 1차 경로의 10초와 맞춘다.
const CURL_MAX_TIME_SECS: u64 = 10;
/// 자식 프로세스 회수 상한. curl이 `max-time`을 무시하고 매달리는 병리적
/// 경우까지 덮는 바깥 울타리다.
const CURL_WAIT: Duration = Duration::from_secs(15);
/// `claude -p` 회수 상한. CLI 부팅(플러그인·MCP 로드)이 느릴 수 있어 넉넉히 준다.
const CLI_WAIT: Duration = Duration::from_secs(60);

/// curl 표준출력 끝에 붙이는 상태코드 마커. 본문과 섞이지 않도록 충분히
/// 특이한 문자열을 쓴다(응답 JSON에 우연히 등장할 수 없는 모양).
const STATUS_MARKER: &str = "<<<agent-office-http:";

/// 자식에게 물려주면 안 되는 훅 연동 환경변수. 이게 살아 있으면 우리가 띄운
/// `claude`가 훅을 통해 앱에 세션 시작을 보고해 유령 캐릭터가 생긴다.
const HOOK_ENV_KEYS: &[&str] = &[
    "ORCA_AGENT_HOOK_PORT",
    "ORCA_AGENT_HOOK_TOKEN",
    "ORCA_AGENT_HOOK_ENDPOINT",
    "ORCA_AGENT_HOOK_ENV",
    "ORCA_AGENT_HOOK_VERSION",
    "ORCA_AGENT_LAUNCH_TOKEN",
    "ORCA_PANE_KEY",
    "ORCA_TAB_ID",
    "ORCA_WORKTREE_ID",
];

// ── curl 폴백 ────────────────────────────────────────────────────────────

/// curl config 파일 본문(순수 함수). `--config -`로 stdin에 흘려 넣어 토큰이
/// 명령줄에 노출되지 않게 한다. config 문법상 큰따옴표 값 안에서는 역슬래시가
/// 이스케이프 문자이므로 값은 반드시 escape_curl_value를 통과시킨다.
pub(super) fn build_curl_config(token: &str) -> String {
    format!(
        "url = \"{USAGE_ENDPOINT}\"\n\
         header = \"Authorization: Bearer {token}\"\n\
         header = \"anthropic-beta: {OAUTH_BETA}\"\n\
         user-agent = \"{CLIENT_USER_AGENT}\"\n\
         max-time = {CURL_MAX_TIME_SECS}\n\
         silent\n\
         write-out = \"\\n{STATUS_MARKER}%{{http_code}}>>>\"\n",
        token = escape_curl_value(token),
    )
}

/// curl config 값 이스케이프(순수 함수). 역슬래시·큰따옴표만 처리하고 제어
/// 문자는 통째로 버린다 — 토큰에 개행이 섞여 들어오면 config 한 줄이 두 줄로
/// 쪼개져 엉뚱한 옵션으로 해석될 수 있다.
fn escape_curl_value(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .flat_map(|c| match c {
            '\\' => vec!['\\', '\\'],
            '"' => vec!['\\', '"'],
            other => vec![other],
        })
        .collect()
}

/// curl 표준출력을 (상태코드, 본문)으로 가른다(순수 함수). 마커를 못 찾으면
/// None — curl이 요청 자체를 못 보낸 경우(DNS·TLS 실패)가 여기 해당한다.
pub(super) fn split_curl_output(raw: &str) -> Option<(u16, &str)> {
    let idx = raw.rfind(STATUS_MARKER)?;
    let body = raw[..idx].trim_end_matches('\n');
    let code = raw[idx + STATUS_MARKER.len()..]
        .trim()
        .trim_end_matches(">>>")
        .parse::<u16>()
        .ok()?;
    Some((code, body))
}

/// curl로 사용량 엔드포인트를 조회한다. 실패 분류는 1차 경로와 같은 어휘를
/// 쓰되(detail만 "curl" 접두로 구분), 401/403은 동일하게 Unauthorized로 세운다.
pub(super) async fn fetch_via_curl(token: &str) -> Result<Vec<UsageWindow>, LiveFailure> {
    let mut command = tokio::process::Command::new("curl");
    command
        .arg("--config")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    no_console_window(&mut command);

    let mut child = command.spawn().map_err(|_| {
        LiveFailure::new(
            LiveFetchOutcome::NetworkError,
            Some("curl 실행 불가".into()),
        )
    })?;

    // stdin을 다 쓰고 닫아야 curl이 config 읽기를 끝낸다(take → drop으로 EOF).
    if let Some(mut stdin) = child.stdin.take() {
        let config = build_curl_config(token);
        if stdin.write_all(config.as_bytes()).await.is_err() {
            return Err(LiveFailure::new(
                LiveFetchOutcome::NetworkError,
                Some("curl 입력 실패".into()),
            ));
        }
    }

    let output = tokio::time::timeout(CURL_WAIT, child.wait_with_output())
        .await
        .map_err(|_| {
            LiveFailure::new(
                LiveFetchOutcome::NetworkError,
                Some("curl 시간 초과".into()),
            )
        })?
        .map_err(|_| {
            LiveFailure::new(LiveFetchOutcome::NetworkError, Some("curl 실패".into()))
        })?;

    let raw = String::from_utf8_lossy(&output.stdout);
    let Some((status, body)) = split_curl_output(&raw) else {
        // 마커가 없다 = 요청이 아예 안 나갔다. curl 종료코드로 사유를 좁힌다
        // (6 DNS, 7 연결, 28 타임아웃, 35/60 TLS·인증서).
        let code = output.status.code().unwrap_or(-1);
        return Err(LiveFailure::new(
            LiveFetchOutcome::NetworkError,
            Some(format!("curl 종료코드 {code}")),
        ));
    };
    if !(200..300).contains(&status) {
        let mut failure = http_failure(status);
        failure.detail = Some(format!("curl HTTP {status}"));
        return Err(failure);
    }
    let root: Value = serde_json::from_str(body).map_err(|_| {
        LiveFailure::new(
            LiveFetchOutcome::UnexpectedResponse,
            Some("curl 응답이 JSON이 아님".into()),
        )
    })?;
    parse_live_response(&root).ok_or_else(|| {
        LiveFailure::new(
            LiveFetchOutcome::UnexpectedResponse,
            Some("curl 응답에 아는 한도 필드가 없음".into()),
        )
    })
}

// ── claude CLI 폴백 ──────────────────────────────────────────────────────

/// `claude -p /usage` 시도 결과. 조회 성패와 **과금 감지**는 별개 축이라 함께
/// 나른다 — 조회에 실패했더라도 토큰을 썼다면 이 경로는 즉시 봉인해야 한다.
pub(super) struct CliProbe {
    pub result: Result<Vec<UsageWindow>, LiveFailure>,
    /// CLI가 스스로 보고한 과금의 설명. Some이면 호출자가 이 경로를 영구
    /// 비활성화한다.
    pub token_spend: Option<String>,
}

/// 응답에서 모델 호출로 인한 과금 흔적을 찾는다(순수 함수). 지금은 항상 0이
/// 나오지만, CLI가 `/usage`를 모델에 태우도록 바뀌는 순간 여기서 걸린다.
pub(super) fn detect_token_spend(root: &Value) -> Option<String> {
    let usage = root.get("usage");
    let field = |key: &str| {
        usage
            .and_then(|u| u.get(key))
            .and_then(Value::as_i64)
            .unwrap_or(0)
    };
    let tokens = field("input_tokens")
        + field("output_tokens")
        + field("cache_creation_input_tokens")
        + field("cache_read_input_tokens");
    if tokens > 0 {
        return Some(format!("토큰 {tokens} 소모"));
    }
    if root
        .get("total_cost_usd")
        .and_then(Value::as_f64)
        .is_some_and(|c| c > 0.0)
    {
        return Some("과금 발생".into());
    }
    if root.get("num_turns").and_then(Value::as_i64).is_some_and(|t| t > 0) {
        return Some("모델 턴 발생".into());
    }
    None
}

/// `claude -p ... --output-format json` 응답에서 사용량 JSON을 찾는다(순수
/// 함수). 세 자리를 본다: 루트 자체 → `result`가 객체인 경우 → `result`가
/// JSON 문자열인 경우. 어디에도 없으면 None(현재 CLI의 정상 동작이다).
pub(super) fn extract_cli_usage(root: &Value) -> Option<Vec<UsageWindow>> {
    if let Some(windows) = parse_live_response(root) {
        return Some(windows);
    }
    let result = root.get("result")?;
    if result.is_object() {
        return parse_live_response(result);
    }
    let inner: Value = serde_json::from_str(result.as_str()?.trim()).ok()?;
    parse_live_response(&inner)
}

/// CLI에게 사용량을 묻는다. 실패는 전부 Err이되 과금 감지는 별도로 보고한다.
pub(super) async fn fetch_via_claude_cli() -> CliProbe {
    let mut command = claude_command();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for key in HOOK_ENV_KEYS {
        command.env_remove(key);
    }
    no_console_window(&mut command);

    let spawned = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            return CliProbe {
                result: Err(LiveFailure::new(
                    LiveFetchOutcome::NetworkError,
                    Some("claude 실행 불가".into()),
                )),
                token_spend: None,
            }
        }
    };

    let output = match tokio::time::timeout(CLI_WAIT, spawned.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(_)) => {
            return CliProbe {
                result: Err(LiveFailure::new(
                    LiveFetchOutcome::NetworkError,
                    Some("claude 실패".into()),
                )),
                token_spend: None,
            }
        }
        Err(_) => {
            return CliProbe {
                result: Err(LiveFailure::new(
                    LiveFetchOutcome::NetworkError,
                    Some("claude 시간 초과".into()),
                )),
                token_spend: None,
            }
        }
    };

    let raw = String::from_utf8_lossy(&output.stdout);
    let Ok(root) = serde_json::from_str::<Value>(raw.trim()) else {
        return CliProbe {
            result: Err(LiveFailure::new(
                LiveFetchOutcome::UnexpectedResponse,
                Some("claude 출력이 JSON이 아님".into()),
            )),
            token_spend: None,
        };
    };
    let token_spend = detect_token_spend(&root);
    let result = extract_cli_usage(&root).ok_or_else(|| {
        LiveFailure::new(
            LiveFetchOutcome::UnexpectedResponse,
            Some("claude가 사용량을 돌려주지 않음".into()),
        )
    });
    CliProbe {
        result,
        token_spend,
    }
}

/// `claude -p /usage --output-format json` 커맨드. Windows에서는 PATH 해석이
/// 번들 프로세스에서 자주 실패해 요약기(summarizer::claude)와 같은 방식으로
/// PowerShell에게 실행 파일을 찾게 한다.
#[cfg(not(windows))]
fn claude_command() -> tokio::process::Command {
    let mut command = tokio::process::Command::new("claude");
    command.args(["-p", "/usage", "--output-format", "json"]);
    command
}

#[cfg(windows)]
const WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
$c = Get-Command claude -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
& $c.Source -p /usage --output-format json
exit $LASTEXITCODE"#;

#[cfg(windows)]
fn claude_command() -> tokio::process::Command {
    let mut command = tokio::process::Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]);
    command
}

/// Windows에서 자식 콘솔 창이 깜빡이지 않게 한다(요약기와 같은 처방).
#[cfg(windows)]
fn no_console_window(command: &mut tokio::process::Command) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_console_window(_command: &mut tokio::process::Command) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::UsageWindowKind;

    // ── curl config 조립 ──

    #[test]
    fn curl_config_carries_endpoint_headers_and_token() {
        let config = build_curl_config("tok-abc");
        assert!(config.contains(&format!("url = \"{USAGE_ENDPOINT}\"")));
        assert!(config.contains("header = \"Authorization: Bearer tok-abc\""));
        assert!(config.contains(&format!("header = \"anthropic-beta: {OAUTH_BETA}\"")));
        assert!(config.contains("silent"));
    }

    #[test]
    fn curl_config_escapes_quotes_and_drops_control_chars() {
        // 개행이 그대로 들어가면 config 한 줄이 쪼개져 엉뚱한 옵션이 된다.
        let config = build_curl_config("a\"b\\c\nd");
        assert!(config.contains(r#"Bearer a\"b\\cd"#), "{config}");
        let header_lines = config
            .lines()
            .filter(|l| l.starts_with("header = \"Authorization"))
            .count();
        assert_eq!(header_lines, 1, "토큰의 개행이 줄을 쪼개면 안 된다");
    }

    // ── curl 출력 분리 ──

    #[test]
    fn split_curl_output_separates_body_and_status() {
        let raw = "{\"five_hour\":null}\n<<<agent-office-http:200>>>";
        let (status, body) = split_curl_output(raw).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, "{\"five_hour\":null}");
    }

    #[test]
    fn split_curl_output_reads_non_2xx_status() {
        let (status, _) = split_curl_output("\n<<<agent-office-http:401>>>").unwrap();
        assert_eq!(status, 401);
    }

    #[test]
    fn split_curl_output_none_when_marker_missing() {
        // curl이 TLS 단계에서 죽으면 표준출력이 통째로 비어 있다.
        assert!(split_curl_output("").is_none());
    }

    // ── 과금 감지(이 경로의 안전장치) ──

    #[test]
    fn detect_token_spend_none_for_current_zero_turn_response() {
        // 실측한 claude 2.1.x의 `-p /usage` 응답 모양.
        let root: Value = serde_json::from_str(
            r#"{"num_turns":0,"total_cost_usd":0,
                "usage":{"input_tokens":0,"output_tokens":0,
                         "cache_creation_input_tokens":0,"cache_read_input_tokens":0}}"#,
        )
        .unwrap();
        assert_eq!(detect_token_spend(&root), None);
    }

    #[test]
    fn detect_token_spend_flags_any_token_use() {
        let root: Value =
            serde_json::from_str(r#"{"usage":{"input_tokens":12,"output_tokens":0}}"#).unwrap();
        assert!(detect_token_spend(&root).is_some());
    }

    #[test]
    fn detect_token_spend_flags_cost_without_tokens() {
        let root: Value = serde_json::from_str(r#"{"total_cost_usd":0.004}"#).unwrap();
        assert_eq!(detect_token_spend(&root).as_deref(), Some("과금 발생"));
    }

    #[test]
    fn detect_token_spend_flags_model_turn() {
        let root: Value = serde_json::from_str(r#"{"num_turns":1}"#).unwrap();
        assert_eq!(detect_token_spend(&root).as_deref(), Some("모델 턴 발생"));
    }

    // ── CLI 응답에서 사용량 찾기 ──

    #[test]
    fn extract_cli_usage_none_for_current_summary_only_response() {
        // 지금의 CLI는 사용량을 주지 않는다. 이 경로가 조용히 실패해야 정상.
        let root: Value = serde_json::from_str(
            r#"{"result":"Total cost: $0.0000\nTotal duration (API): 0s","num_turns":0}"#,
        )
        .unwrap();
        assert!(extract_cli_usage(&root).is_none());
    }

    #[test]
    fn extract_cli_usage_reads_root_level_limits() {
        // 미래에 CLI가 사용량을 루트에 실어주면 그대로 살아나야 한다.
        let root: Value = serde_json::from_str(
            r#"{"limits":[{"kind":"session","percent":42.0,"resets_at":null}]}"#,
        )
        .unwrap();
        let windows = extract_cli_usage(&root).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].kind, UsageWindowKind::Session);
        assert_eq!(windows[0].used_percent, 42.0);
    }

    #[test]
    fn extract_cli_usage_reads_json_string_in_result() {
        let root: Value = serde_json::from_str(
            r#"{"result":"{\"limits\":[{\"kind\":\"weekly_all\",\"percent\":7.5}]}"}"#,
        )
        .unwrap();
        let windows = extract_cli_usage(&root).unwrap();
        assert_eq!(windows[0].kind, UsageWindowKind::Weekly);
    }

    #[test]
    fn extract_cli_usage_reads_object_in_result() {
        let root: Value =
            serde_json::from_str(r#"{"result":{"five_hour":{"utilization":9.0}}}"#).unwrap();
        let windows = extract_cli_usage(&root).unwrap();
        assert_eq!(windows[0].used_percent, 9.0);
    }

    // ── 실제 프로세스 스모크(수동) ──
    //
    // 실 자격증명·네트워크·외부 바이너리가 있어야 하므로 기본 실행에서 제외한다.
    // 폴백 경로를 손댔을 때 다음으로 눈검증한다:
    //   cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture live_fallback

    #[tokio::test]
    #[ignore = "실 자격증명·네트워크 필요(수동 스모크)"]
    async fn live_fallback_curl_smoke() {
        let dir = crate::agent_paths::claude_config_dir_from_env().expect("config dir");
        let (token, source) = super::super::claude_live::read_access_token(&dir)
            .await
            .expect("액세스 토큰을 읽지 못했습니다");
        let windows = fetch_via_curl(&token).await.expect("curl 폴백 실패");
        println!("[curl] 토큰 출처={source:?} 윈도 {}개", windows.len());
        for w in &windows {
            println!("  {:?} {:.1}% resets={:?}", w.kind, w.used_percent, w.resets_at_ms);
        }
        assert!(!windows.is_empty(), "윈도가 하나는 나와야 한다");
    }

    #[tokio::test]
    #[ignore = "claude CLI 실행 필요(수동 스모크)"]
    async fn live_fallback_claude_cli_smoke() {
        // 현재 CLI는 사용량을 돌려주지 않으므로 **실패가 정상**이다. 이 테스트는
        // "실패하더라도 과금은 0이어야 한다"를 눈으로 확인하는 용도다.
        let probe = fetch_via_claude_cli().await;
        println!("[claude] 결과={:?}", probe.result.as_ref().map(|w| w.len()));
        println!("[claude] 과금 감지={:?}", probe.token_spend);
        assert_eq!(
            probe.token_spend, None,
            "이 경로가 토큰을 쓰기 시작했다면 즉시 봉인 대상이다"
        );
    }
}
