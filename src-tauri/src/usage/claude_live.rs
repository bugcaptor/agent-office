// src-tauri/src/usage/claude_live.rs
//
// Claude 사용량 실시간 조회(이슈 #33, docs/claude-usage-live-fetch-design.md).
// v1(claude.rs)은 `.claude.json`의 캐시 미러라 CLI가 재fetch할 때까지 리셋
// 경계 후에도 낡은 값이 남는다. 이 모듈은 Claude Code CLI가 내부적으로 쓰는
// 사용량 엔드포인트를 앱이 직접 호출해 리셋 후 ≤1분 내 실제 값을 얻는다.
// 실패(토큰 없음/401/네트워크/파싱 실패)는 전부 조용히 None → 조립 단계에서
// 파일 캐시 미러로 자연 폴백한다.
//
// 토큰은 로그·에러 문자열에 절대 넣지 않는다(설계 §2.2). 파싱·스로틀 판단은
// 전부 순수 함수라 네트워크 목 없이 테스트한다(HTTP 호출부만 얇게 유지).

use std::path::Path;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::Value;

use super::claude::{parse_fallback, parse_limits};
use super::{
    ClaudeLiveStatus, FetchTransport, LiveFetchOutcome, Provider, ProviderUsage, TokenSource,
    UsageWindow,
};

/// Claude Code CLI가 내부적으로 치는 사용량 엔드포인트(비공식). UA·beta
/// 헤더를 CLI와 맞추는 것이 계약의 일부다(설계 §2.1).
/// pub(super): 외부 프로세스 폴백(claude_live_fallback)이 **같은** 엔드포인트·
/// 헤더로 조회해야 응답 모양이 같고 파서를 공유할 수 있다.
pub(super) const USAGE_ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
pub(super) const OAUTH_BETA: &str = "oauth-2025-04-20";
pub(super) const CLIENT_USER_AGENT: &str = "claude-code/2.1.0";
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Keychain 자식 프로세스(`security`) 대기 상한. Keychain이 잠겨 있거나 권한
/// 다이얼로그가 응답 없이 방치되면 `security`가 무한정 매달릴 수 있는데,
/// 그 동안 `load_usage_snapshot` invoke가 pending으로 남아 파일/Codex 폴백조차
/// 못 돌려주게 된다(PR #34 리뷰 P2). 상한 초과 시 자식을 죽이고 None → 다음
/// 폴백으로 강등한다.
#[cfg(target_os = "macos")]
const KEYCHAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// 레거시 Keychain 서비스명(CLAUDE_CONFIG_DIR 미설정 시).
#[cfg(target_os = "macos")]
const KEYCHAIN_LEGACY_SERVICE: &str = "Claude Code-credentials";

/// 시도 간 하한(성공/실패 공통 — 실패 백오프와 60초 폴링 중복 fetch 차단을
/// 겸한다). 설계 §3.1.
const MIN_ATTEMPT_GAP_MS: i64 = 5 * 60 * 1000;
/// 성공 후 정기 리프레시 간격. 설계 §3.1.
const REFRESH_INTERVAL_MS: i64 = 15 * 60 * 1000;
/// 외부 프로세스 폴백(claude_live_fallback)의 시도 간 하한. 1차 경로보다
/// 훨씬 길게 잡는다: 폴백은 자식 프로세스를 띄우는 무거운 경로이고, `claude`
/// 갈래는 CLI 정책이 바뀌면 언젠가 과금될 수도 있어 "자주 돌지 않는다"가
/// 그 자체로 안전장치다. 1차 조회가 실패한 폴링에서만, 그것도 1시간에 한 번만
/// 시도한다.
const FALLBACK_MIN_GAP_MS: i64 = 60 * 60 * 1000;

// ── 토큰 읽기 ────────────────────────────────────────────────────────────

/// `{"claudeAiOauth":{"accessToken":"..."}}` JSON에서 액세스 토큰만 뽑는
/// 순수 함수(파일/Keychain 값 공통 모양). 키 부재·빈 값·깨진 JSON은 None.
pub(super) fn parse_access_token(json: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json).ok()?;
    v.get("claudeAiOauth")
        .and_then(|o| o.get("accessToken"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 설계 §2.2 우선순위로 액세스 토큰을 읽는다: (macOS) 스코프 Keychain →
/// 레거시 Keychain → `<config_dir>/.credentials.json`. 어느 경로든 실패는
/// None(토큰은 반환값 안에서만 존재하며 로그·에러에 남기지 않는다).
///
/// `config_dir`은 CLAUDE_CONFIG_DIR(설정 시) 또는 `~/.claude`(미설정 시)다.
/// 스코프 Keychain 서비스명은 이 경로 문자열의 sha256 앞 8자로 만든다 —
/// 미설정 케이스에선 그 항목이 존재하지 않아 조용히 레거시로 강등된다.
/// 반환값에는 토큰을 실제로 읽어낸 출처가 함께 온다 — 진단 표시가 "Keychain이
/// 막혀 파일로 폴백했다"를 구분해 말할 수 있어야 하기 때문이다(§7).
pub async fn read_access_token(config_dir: &Path) -> Option<(String, TokenSource)> {
    #[cfg(target_os = "macos")]
    {
        // 1) 스코프 항목(CLAUDE_CONFIG_DIR 설정 시 Claude Code가 쓰는 위치).
        let scoped = scoped_keychain_service(&config_dir.to_string_lossy());
        if let Some(tok) = read_keychain(&scoped).await.and_then(|j| parse_access_token(&j)) {
            return Some((tok, TokenSource::KeychainScoped));
        }
        // 2) 레거시 항목.
        if let Some(tok) = read_keychain(KEYCHAIN_LEGACY_SERVICE)
            .await
            .and_then(|j| parse_access_token(&j))
        {
            return Some((tok, TokenSource::KeychainLegacy));
        }
    }
    // 3) 파일 폴백(비-macOS는 이 경로만).
    let content = std::fs::read_to_string(config_dir.join(".credentials.json")).ok()?;
    parse_access_token(&content).map(|tok| (tok, TokenSource::File))
}

/// 스코프 Keychain 서비스명: `Claude Code-credentials-<sha256(dir) hex 앞 8자>`.
/// 설계 §2.2. 순수 함수(테스트 가능).
#[cfg(target_os = "macos")]
fn scoped_keychain_service(config_dir: &str) -> String {
    format!(
        "Claude Code-credentials-{}",
        &sha256_hex(config_dir.as_bytes())[..8]
    )
}

#[cfg(target_os = "macos")]
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `security find-generic-password -s <service> -a $USER -w`로 Keychain 값을
/// 읽는다(설계 §2.2). 항목 부재·권한 거부는 비영 종료 → None. 표준출력의
/// 값(JSON)만 돌려주고 stderr는 무시한다(토큰 노출 방지).
///
/// 비동기 + KEYCHAIN_TIMEOUT 상한: 잠긴 Keychain·방치된 권한 다이얼로그로
/// `security`가 매달려도 폴링 경로가 막히지 않는다. kill_on_drop이라 타임아웃
/// 시 자식도 정리된다.
#[cfg(target_os = "macos")]
async fn read_keychain(service: &str) -> Option<String> {
    let user = std::env::var("USER").ok()?;
    let output = tokio::time::timeout(
        KEYCHAIN_TIMEOUT,
        tokio::process::Command::new("security")
            .args(["find-generic-password", "-s", service, "-a", &user, "-w"])
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

// ── 응답 파싱 ────────────────────────────────────────────────────────────

/// 사용량 응답 본문 → 윈도 배열. 루트가 `cachedUsageUtilization.utilization`과
/// 같은 모양(`limits[]`/`five_hour`/`seven_day`)이라, claude.rs의 기존 파서를
/// 그대로 재사용한다(limits[] 우선, 없으면 five_hour/seven_day 폴백). 계약이
/// 바뀌어 어느 쪽도 안 맞으면 None → 파일 캐시 폴백. 설계 §2.1.
pub(super) fn parse_live_response(root: &Value) -> Option<Vec<UsageWindow>> {
    parse_limits(root).or_else(|| parse_fallback(root))
}

// ── HTTP 호출(얇게) ──────────────────────────────────────────────────────

/// 실시간 조회 실패 하나. 표시용 사유(outcome)와 사람이 읽을 보조 문자열을
/// 함께 나른다. **토큰·자격증명은 어디에도 넣지 않는다**(설계 §2.2) — detail은
/// HTTP 상태코드나 "시간 초과" 같은 고정 어휘만 담는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LiveFailure {
    pub outcome: LiveFetchOutcome,
    pub detail: Option<String>,
}

impl LiveFailure {
    pub(super) fn new(outcome: LiveFetchOutcome, detail: Option<String>) -> Self {
        Self { outcome, detail }
    }

    /// 토큰을 어느 출처에서도 못 읽은 경우(조립 단계에서 사용).
    pub(super) fn no_credentials() -> Self {
        Self::new(LiveFetchOutcome::NoCredentials, None)
    }
}

/// 토큰으로 사용량 엔드포인트를 GET 해 윈도 배열을 얻는다. 타임아웃 10초.
/// 실패는 전부 Err(→ 파일 캐시 폴백)이되, **사유를 분류해서** 돌려준다 —
/// 조용한 폴백이 "값이 왜 안 움직이는지 알 수 없음"이 되지 않도록(§7).
/// 로컬 만료 판정은 여전히 하지 않는다 — 서버 401이 유일한 판정자(설계 §2.2).
pub(super) async fn fetch_live(token: &str) -> Result<Vec<UsageWindow>, LiveFailure> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        // 번들 루트와 OS 스토어 루트를 모두 신뢰한다. 사내 프록시가 TLS를
        // 가로채는 환경에서는 후자만이 유일한 통로다(Cargo.toml의 reqwest
        // features 주석 참조). 그래도 못 뚫으면 claude_live_fallback이
        // 외부 CLI로 우회한다.
        .tls_built_in_webpki_certs(true)
        .tls_built_in_native_certs(true)
        .build()
        .map_err(|_| {
            LiveFailure::new(LiveFetchOutcome::NetworkError, Some("클라이언트 초기화 실패".into()))
        })?;
    let resp = client
        .get(USAGE_ENDPOINT)
        .bearer_auth(token)
        .header("anthropic-beta", OAUTH_BETA)
        .header(reqwest::header::USER_AGENT, CLIENT_USER_AGENT)
        .send()
        .await
        .map_err(|e| {
            // reqwest 오류 문자열에는 URL이 섞여 나오므로 그대로 싣지 않고
            // 고정 어휘로만 분류한다.
            let detail = if e.is_timeout() {
                "시간 초과"
            } else if e.is_connect() {
                "연결 실패"
            } else {
                "요청 실패"
            };
            LiveFailure::new(LiveFetchOutcome::NetworkError, Some(detail.into()))
        })?;
    let status = resp.status();
    if !status.is_success() {
        return Err(http_failure(status.as_u16()));
    }
    let body: Value = resp.json().await.map_err(|_| {
        LiveFailure::new(LiveFetchOutcome::UnexpectedResponse, Some("본문이 JSON이 아님".into()))
    })?;
    parse_live_response(&body).ok_or_else(|| {
        LiveFailure::new(LiveFetchOutcome::UnexpectedResponse, Some("아는 한도 필드가 없음".into()))
    })
}

/// 비2xx 상태코드 → 실패 분류(순수 함수). 401/403만 자격증명 문제로 따로 세워
/// UI가 "재로그인/토큰 만료"를 말할 수 있게 한다.
pub(super) fn http_failure(status: u16) -> LiveFailure {
    let outcome = if status == 401 || status == 403 {
        LiveFetchOutcome::Unauthorized
    } else {
        LiveFetchOutcome::HttpError
    };
    LiveFailure::new(outcome, Some(format!("HTTP {status}")))
}

// ── 스로틀 상태 + 판단 ───────────────────────────────────────────────────

/// 순수 스로틀 판단(설계 §3.1). 락 밖에서도 검증 가능하도록 상태를 인자로
/// 받는다. 순서가 중요하다:
/// 1. 마지막 시도 후 5분 하한 미만이면 무조건 안 함(실패 백오프·중복 차단).
/// 2. 성공 스냅샷이 없으면 fetch(최초, 혹은 이전 시도가 모두 실패한 뒤).
/// 3. 마지막 성공 윈도 중 이미 리셋된(resets_at<now) 것이 있으면 조기 fetch.
/// 4. 마지막 성공 후 15분 경과면 fetch.
pub(super) fn should_fetch(
    last_success: Option<&ProviderUsage>,
    last_attempt_ms: Option<i64>,
    now_ms: i64,
) -> bool {
    if let Some(attempt) = last_attempt_ms {
        if now_ms - attempt < MIN_ATTEMPT_GAP_MS {
            return false;
        }
    }
    let Some(success) = last_success else {
        return true;
    };
    let past_reset = success
        .windows
        .iter()
        .any(|w| w.resets_at_ms.is_some_and(|r| r < now_ms));
    if past_reset {
        return true;
    }
    now_ms - success.fetched_at_ms >= REFRESH_INTERVAL_MS
}

/// 외부 프로세스 폴백을 시도해도 되는지(순수 함수). 호출자는 **1차 조회가
/// 실패한 직후에만** 이걸 묻는다 — 그래서 여기선 시간 간격만 본다.
pub(super) fn should_try_fallback(last_fallback_attempt_ms: Option<i64>, now_ms: i64) -> bool {
    match last_fallback_attempt_ms {
        None => true,
        Some(prev) => now_ms - prev >= FALLBACK_MIN_GAP_MS,
    }
}

/// 실시간 조회 메모리 상태(AppState 보관). Mutex를 잡은 채 await 하지 않는다:
/// 판단·기록은 짧은 임계구역이고 실제 fetch는 락 밖에서 진행한다.
#[derive(Default)]
pub struct LiveUsageState {
    inner: Mutex<LiveUsageInner>,
    /// Codex 실시간 조회 상태. 별도 Arc를 앱 전역에 다시 배선하지 않으려고
    /// 같은 그릇에 담았다 — 이 구조체는 이미 "사용량 실시간 조회의 메모리
    /// 상태"로 네이티브 커맨드와 웹 RPC가 공유하고 있고, Codex도 같은 폴링에
    /// 얹히기 때문이다. 두 provider의 상태는 서로 독립이다.
    pub(super) codex: super::codex_live::CodexLiveState,
}

#[derive(Default)]
struct LiveUsageInner {
    last_success: Option<ProviderUsage>,
    last_attempt_ms: Option<i64>,
    /// 마지막 **시도**의 결과. 성공 스냅샷(last_success)과 별개로 유지된다 —
    /// 성공 이력이 있어도 지금은 실패 중일 수 있고, 그때 표시값이 낡는 이유가
    /// 바로 이 값이다.
    outcome: LiveFetchOutcome,
    detail: Option<String>,
    token_source: Option<TokenSource>,
    /// 마지막으로 **값을 얻어낸** 수단(실패는 이 값을 건드리지 않는다).
    /// 폴백으로 값이 오고 있다는 사실 자체가 진단이다 — "1차 경로가 이
    /// 환경에서 막혀 있고 우회로로 연명 중"임을 UI가 말할 수 있다.
    via: Option<FetchTransport>,
    /// 마지막 **폴백** 시도 시각(epoch ms). 1차 시도(last_attempt_ms)와 별도
    /// 축으로 1시간 간격을 건다.
    last_fallback_attempt_ms: Option<i64>,
    /// `claude` CLI 갈래를 봉인한 사유. CLI가 사용량 조회에 모델을 태우기
    /// 시작하면(=과금) 그 즉시 채워지고, 이후 세션 내내 그 갈래를 건너뛴다.
    cli_disabled: Option<String>,
}

impl LiveUsageState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 스로틀 통과 시 `last_attempt_ms`를 now로 먼저 갱신하고 true를 돌려준다.
    /// 판단+갱신을 한 락 안에서 하므로, 60초 폴링이 겹쳐 들어와도 첫 호출만
    /// true를 받고 나머지는 5분 하한에 걸려 중복 fetch가 자연 차단된다.
    ///
    /// pub(crate): commands.rs의 위임 테스트가 스로틀을 선점해 live 경로
    /// (Keychain 자식 프로세스·실 API 호출)를 결정적으로 차단하는 데도 쓴다 —
    /// 개발 머신엔 실 자격증명이 있어 "토큰이 없으니 자연 강등"을 믿을 수 없다.
    pub(crate) fn begin_attempt_if_due(&self, now_ms: i64) -> bool {
        let mut guard = self.inner.lock();
        let due = should_fetch(guard.last_success.as_ref(), guard.last_attempt_ms, now_ms);
        if due {
            guard.last_attempt_ms = Some(now_ms);
        }
        due
    }

    /// 폴백을 시도해도 되는지 판단하고, 통과하면 `last_fallback_attempt_ms`를
    /// 먼저 갱신한다(begin_attempt_if_due와 같은 규율 — 판단+갱신을 한 락 안에서).
    pub(super) fn begin_fallback_if_due(&self, now_ms: i64) -> bool {
        let mut guard = self.inner.lock();
        let due = should_try_fallback(guard.last_fallback_attempt_ms, now_ms);
        if due {
            guard.last_fallback_attempt_ms = Some(now_ms);
        }
        due
    }

    /// `claude` CLI 갈래가 아직 살아 있는지. 봉인됐으면 사유를 돌려준다.
    pub(super) fn cli_disabled_reason(&self) -> Option<String> {
        self.inner.lock().cli_disabled.clone()
    }

    /// CLI 갈래를 봉인한다(과금 감지). 되돌리는 경로는 없다 — 앱을 다시 켜면
    /// 초기화되므로, CLI 정책이 바뀌었다면 다음 실행에서 한 번 더 확인하고
    /// 다시 봉인될 뿐이다.
    pub(super) fn disable_cli_fallback(&self, reason: String) {
        self.inner.lock().cli_disabled = Some(reason);
    }

    /// fetch 성공 결과를 기록한다(진단 상태도 함께 Ok로 되돌린다).
    ///
    /// `token_source`가 Option인 것은 `claude` CLI 갈래 때문이다 — 그 경로는
    /// **CLI 자신의 자격증명**으로 조회하므로 우리가 토큰을 한 줄도 못 읽은
    /// 상태에서도 성공할 수 있다.
    pub(super) fn record_success(
        &self,
        usage: ProviderUsage,
        token_source: Option<TokenSource>,
        via: FetchTransport,
    ) {
        let mut guard = self.inner.lock();
        guard.last_success = Some(usage);
        guard.outcome = LiveFetchOutcome::Ok;
        guard.detail = None;
        guard.token_source = token_source;
        guard.via = Some(via);
    }

    /// 실패 사유를 기록한다. `last_success`와 `via`는 건드리지 않는다 —
    /// 마지막으로 성공했던 값은 파일 캐시보다 신선할 수 있어 계속 쓰이고,
    /// 그 값을 어느 수단으로 얻었는지도 함께 남아야 "지금은 실패 중이지만
    /// 아까는 curl로 받아온 값"이라는 설명이 성립한다. 왜 값이 안 갱신되는지는
    /// outcome/detail이 말하며, 폴백 갈래별 실패는 detail에 접두로 구분돼
    /// 함께 실린다("연결 실패 · curl: HTTP 401" 등).
    pub(super) fn record_failure(&self, token_source: Option<TokenSource>, failure: LiveFailure) {
        let mut guard = self.inner.lock();
        guard.outcome = failure.outcome;
        guard.detail = failure.detail;
        guard.token_source = token_source;
    }

    /// 렌더러로 나갈 진단 스냅샷. `last_success_ms`는 성공 스냅샷에서 유도해
    /// 중복 필드를 두지 않는다.
    pub(super) fn status(&self) -> ClaudeLiveStatus {
        let guard = self.inner.lock();
        ClaudeLiveStatus {
            outcome: guard.outcome,
            token_source: guard.token_source,
            detail: guard.detail.clone(),
            last_attempt_ms: guard.last_attempt_ms,
            last_success_ms: guard.last_success.as_ref().map(|u| u.fetched_at_ms),
            via: guard.via,
        }
    }

    /// 마지막 성공 스냅샷 복제본(조립에서 파일 캐시와 신선도 비교).
    pub(super) fn last_success(&self) -> Option<ProviderUsage> {
        self.inner.lock().last_success.clone()
    }
}

/// fetch 성공 윈도로 Claude ProviderUsage를 만든다. `fetched_at_ms`는 조회
/// 시각(now)이고 `plan_label`은 응답에 없어 여기선 None — 조립 단계에서 파일
/// 캐시의 값을 접목한다(설계 §3.2).
pub(super) fn live_provider_usage(windows: Vec<UsageWindow>, now_ms: i64) -> ProviderUsage {
    ProviderUsage {
        provider: Provider::Claude,
        fetched_at_ms: now_ms,
        plan_label: None,
        windows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{Provider, UsageWindow, UsageWindowKind};

    fn window(resets_at_ms: Option<i64>) -> UsageWindow {
        UsageWindow {
            kind: UsageWindowKind::Session,
            label: None,
            used_percent: 50.0,
            resets_at_ms,
            window_minutes: None,
            is_active: None,
        }
    }

    fn usage(fetched_at_ms: i64, windows: Vec<UsageWindow>) -> ProviderUsage {
        ProviderUsage {
            provider: Provider::Claude,
            fetched_at_ms,
            plan_label: None,
            windows,
        }
    }

    // ── parse_access_token ──

    #[test]
    fn parse_access_token_reads_nested_oauth_field() {
        let json = r#"{"claudeAiOauth":{"accessToken":"tok-123","refreshToken":"r"}}"#;
        assert_eq!(parse_access_token(json).as_deref(), Some("tok-123"));
    }

    #[test]
    fn parse_access_token_missing_key_yields_none() {
        assert_eq!(parse_access_token(r#"{"claudeAiOauth":{}}"#), None);
        assert_eq!(parse_access_token(r#"{"other":1}"#), None);
    }

    #[test]
    fn parse_access_token_empty_string_yields_none() {
        assert_eq!(parse_access_token(r#"{"claudeAiOauth":{"accessToken":""}}"#), None);
    }

    #[test]
    fn parse_access_token_corrupt_json_yields_none() {
        assert_eq!(parse_access_token("{ not json "), None);
    }

    // ── parse_live_response (기존 파서 재사용 확인) ──

    #[test]
    fn parse_live_response_prefers_limits_array() {
        let body = serde_json::json!({
            "five_hour": { "utilization": 99, "resets_at": "2026-07-17T09:50:00+00:00" },
            "limits": [
                { "kind": "session", "percent": 61, "resets_at": "2026-07-17T09:50:00+00:00", "is_active": true },
                { "kind": "weekly_all", "percent": 18, "resets_at": "2026-07-21T04:00:00+00:00" }
            ]
        });
        let windows = parse_live_response(&body).unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].kind, UsageWindowKind::Session);
        assert_eq!(windows[0].used_percent, 61.0);
        assert_eq!(windows[1].kind, UsageWindowKind::Weekly);
    }

    #[test]
    fn parse_live_response_falls_back_to_five_hour_seven_day() {
        let body = serde_json::json!({
            "five_hour": { "utilization": 61, "resets_at": "2026-07-17T09:50:00+00:00" },
            "seven_day": { "utilization": 18, "resets_at": "2026-07-21T04:00:00+00:00" }
        });
        let windows = parse_live_response(&body).unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].kind, UsageWindowKind::Session);
        assert_eq!(windows[1].kind, UsageWindowKind::Weekly);
    }

    #[test]
    fn parse_live_response_empty_body_yields_none() {
        assert_eq!(parse_live_response(&serde_json::json!({})), None);
    }

    // ── should_fetch ──

    #[test]
    fn should_fetch_true_on_first_call_no_state() {
        assert!(should_fetch(None, None, 1_000_000));
    }

    #[test]
    fn should_fetch_false_within_five_minute_lower_bound() {
        // 마지막 시도 1분 전, 성공 스냅샷 없음이어도 5분 하한이 우선한다.
        let now = 1_000_000;
        assert!(!should_fetch(None, Some(now - 60_000), now));
    }

    #[test]
    fn should_fetch_true_after_five_minutes_with_no_success() {
        let now = 10_000_000;
        assert!(should_fetch(None, Some(now - MIN_ATTEMPT_GAP_MS), now));
    }

    #[test]
    fn should_fetch_false_when_success_fresh_and_no_reset_passed() {
        let now = 10_000_000;
        let last = usage(now - 60_000, vec![window(Some(now + 3_600_000))]);
        assert!(!should_fetch(Some(&last), Some(now - MIN_ATTEMPT_GAP_MS), now));
    }

    #[test]
    fn should_fetch_true_after_refresh_interval() {
        let now = 10_000_000;
        let last = usage(now - REFRESH_INTERVAL_MS, vec![window(Some(now + 3_600_000))]);
        assert!(should_fetch(Some(&last), Some(now - MIN_ATTEMPT_GAP_MS), now));
    }

    #[test]
    fn should_fetch_true_when_a_window_reset_boundary_passed() {
        // 15분은 안 됐지만 윈도 하나가 이미 리셋됨 → 조기 fetch.
        let now = 10_000_000;
        let last = usage(now - 60_000, vec![window(Some(now - 1))]);
        assert!(should_fetch(Some(&last), Some(now - MIN_ATTEMPT_GAP_MS), now));
    }

    #[test]
    fn should_fetch_reset_boundary_still_respects_five_minute_lower_bound() {
        // 리셋 경계라도 5분 하한은 유지 → 방금 시도했으면 안 함.
        let now = 10_000_000;
        let last = usage(now - 60_000, vec![window(Some(now - 1))]);
        assert!(!should_fetch(Some(&last), Some(now - 60_000), now));
    }

    // ── LiveUsageState 동시성(중복 fetch 차단) ──

    #[test]
    fn begin_attempt_if_due_dedups_concurrent_polls() {
        let state = LiveUsageState::new();
        let now = 5_000_000;
        // 첫 호출은 통과하며 last_attempt를 갱신한다.
        assert!(state.begin_attempt_if_due(now));
        // 같은/직후 시각의 재진입은 5분 하한에 걸려 false(중복 fetch 차단).
        assert!(!state.begin_attempt_if_due(now));
        assert!(!state.begin_attempt_if_due(now + 60_000));
    }

    #[test]
    fn record_success_then_last_success_round_trips() {
        let state = LiveUsageState::new();
        assert!(state.last_success().is_none());
        let u = usage(123, vec![window(None)]);
        state.record_success(
            u.clone(),
            Some(TokenSource::KeychainLegacy),
            FetchTransport::Direct,
        );
        assert_eq!(state.last_success(), Some(u));
    }

    // ── 폴백 스로틀(1시간) ──

    #[test]
    fn should_try_fallback_true_when_never_tried() {
        assert!(should_try_fallback(None, 0));
    }

    #[test]
    fn should_try_fallback_false_within_an_hour() {
        let now = 10_000_000;
        assert!(!should_try_fallback(Some(now), now));
        assert!(!should_try_fallback(Some(now), now + 59 * 60 * 1000));
    }

    #[test]
    fn should_try_fallback_true_after_an_hour() {
        let now = 10_000_000;
        assert!(should_try_fallback(Some(now), now + 60 * 60 * 1000));
    }

    #[test]
    fn begin_fallback_if_due_dedups_within_the_hour() {
        let state = LiveUsageState::new();
        let now = 20_000_000;
        assert!(state.begin_fallback_if_due(now), "첫 시도는 통과");
        assert!(!state.begin_fallback_if_due(now + 5 * 60 * 1000), "1시간 안엔 막힌다");
        assert!(state.begin_fallback_if_due(now + 60 * 60 * 1000), "1시간 뒤엔 다시 통과");
    }

    /// 1차 시도 스로틀과 폴백 스로틀은 별개 축이다 — 폴백을 한 번 썼다고
    /// 5분마다 도는 1차 조회가 막히면 안 된다.
    #[test]
    fn fallback_throttle_is_independent_from_primary_throttle() {
        let state = LiveUsageState::new();
        let now = 30_000_000;
        assert!(state.begin_fallback_if_due(now));
        assert!(state.begin_attempt_if_due(now), "1차 시도는 폴백과 무관하게 통과");
    }

    // ── CLI 갈래 봉인(과금 안전장치) ──

    #[test]
    fn cli_fallback_starts_enabled_and_stays_disabled_once_sealed() {
        let state = LiveUsageState::new();
        assert_eq!(state.cli_disabled_reason(), None);
        state.disable_cli_fallback("토큰 12 소모".into());
        assert_eq!(state.cli_disabled_reason().as_deref(), Some("토큰 12 소모"));
    }

    // ── 진단 상태(§7) ──

    #[test]
    fn status_starts_as_never_attempted() {
        let s = LiveUsageState::new().status();
        assert_eq!(s.outcome, LiveFetchOutcome::NeverAttempted);
        assert_eq!(s.token_source, None);
        assert_eq!(s.detail, None);
        assert_eq!(s.last_attempt_ms, None);
        assert_eq!(s.last_success_ms, None);
    }

    #[test]
    fn status_reports_attempt_and_success_times() {
        let state = LiveUsageState::new();
        let now = 5_000_000;
        assert!(state.begin_attempt_if_due(now));
        state.record_success(
            usage(now, vec![window(None)]),
            Some(TokenSource::KeychainScoped),
            FetchTransport::Direct,
        );
        let s = state.status();
        assert_eq!(s.outcome, LiveFetchOutcome::Ok);
        assert_eq!(s.token_source, Some(TokenSource::KeychainScoped));
        assert_eq!(s.last_attempt_ms, Some(now));
        assert_eq!(s.last_success_ms, Some(now));
        assert_eq!(s.via, Some(FetchTransport::Direct));
    }

    /// 실패는 마지막 성공 값을 지우지 않는다 — 그 값이 계속 표시되고,
    /// 사유는 "왜 그 값이 안 움직이는지"를 설명하는 역할만 한다.
    #[test]
    fn record_failure_keeps_last_success_but_changes_outcome() {
        let state = LiveUsageState::new();
        let u = usage(1_000, vec![window(None)]);
        state.record_success(u.clone(), Some(TokenSource::KeychainLegacy), FetchTransport::Curl);
        state.record_failure(
            Some(TokenSource::File),
            LiveFailure::new(LiveFetchOutcome::Unauthorized, Some("HTTP 401".into())),
        );
        assert_eq!(state.last_success(), Some(u), "성공 스냅샷은 유지");
        let s = state.status();
        assert_eq!(s.outcome, LiveFetchOutcome::Unauthorized);
        assert_eq!(s.detail.as_deref(), Some("HTTP 401"));
        assert_eq!(s.token_source, Some(TokenSource::File));
        assert_eq!(s.last_success_ms, Some(1_000), "성공 시각도 남아 있어야");
        assert_eq!(
            s.via,
            Some(FetchTransport::Curl),
            "실패는 '무엇으로 얻어낸 값인지'를 지우지 않는다"
        );
    }

    #[test]
    fn no_credentials_failure_has_no_token_source() {
        let state = LiveUsageState::new();
        state.record_failure(None, LiveFailure::no_credentials());
        let s = state.status();
        assert_eq!(s.outcome, LiveFetchOutcome::NoCredentials);
        assert_eq!(s.token_source, None);
    }

    #[test]
    fn http_failure_classifies_auth_errors_separately() {
        assert_eq!(http_failure(401).outcome, LiveFetchOutcome::Unauthorized);
        assert_eq!(http_failure(403).outcome, LiveFetchOutcome::Unauthorized);
        let other = http_failure(500);
        assert_eq!(other.outcome, LiveFetchOutcome::HttpError);
        assert_eq!(other.detail.as_deref(), Some("HTTP 500"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn scoped_keychain_service_is_deterministic_prefix_plus_8_hex() {
        let s = scoped_keychain_service("/Users/x/.claude");
        assert!(s.starts_with("Claude Code-credentials-"));
        let suffix = s.strip_prefix("Claude Code-credentials-").unwrap();
        assert_eq!(suffix.len(), 8);
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
        // 결정적: 같은 입력은 같은 서비스명.
        assert_eq!(s, scoped_keychain_service("/Users/x/.claude"));
    }

    // 실 API smoke: 토큰이 있으면 실호출한다. 기본은 무시(#[ignore]) — 자격
    // 증명·네트워크가 필요하므로 사용자가 수동으로만 실행한다.
    //   cargo test -p agent-office --lib -- --ignored live_usage_smoke
    #[tokio::test]
    #[ignore = "실 자격증명·네트워크 필요. 수동 실행 전용."]
    async fn live_usage_smoke() {
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
        let config_dir = std::env::var("CLAUDE_CONFIG_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| home.join(".claude"));
        let (token, source) = read_access_token(&config_dir).await.expect("토큰을 찾지 못함");
        let windows = fetch_live(&token).await.expect("실시간 사용량 조회 실패");
        assert!(!windows.is_empty(), "윈도가 하나 이상 있어야 함");
        // 어느 출처가 실제로 쓰였는지는 진단 표시의 핵심 입력이라 함께 찍는다.
        println!("token source: {source:?}");
    }
}
