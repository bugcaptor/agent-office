// src-tauri/src/usage/openrouter.rs
//
// OpenRouter 키 사용액 조회. Claude/Codex는 구독 한도의 사용률(%)이지만
// OpenRouter는 선불 크레딧을 쓰는 API 키라 보여줄 값이 **금액**이다 — 그래서
// `ProviderUsage`(윈도 배열)에 억지로 끼우지 않고 따로 모양을 둔다.
//
// 출처: `GET https://openrouter.ai/api/v1/key`(Bearer = 요약기에 넣은 그 키).
// 응답 `data`에서 이 키의 누적·일·주·월 사용액(USD)과, 키에 한도를 걸어 둔
// 경우 한도·남은 한도·리셋 주기를 읽는다(2026-10 실측). 리셋 시각은
// `budgets[]` 중 `scope: "key_limit"` 항목의 `resets_at`에만 있다. 계정 잔액
// (`/credits`)은 일반 키로는 404라 쓰지 않는다.
//
// 키는 앱의 키 저장소(key_store.rs, env 폴백 포함)에서 폴링마다 다시 읽는다.
// 키가 바뀌면(지문 비교) 이전 키의 값·시도 시각을 버리고 바로 다시 조회한다
// — 남의 키 사용액이 섞여 보이면 안 된다. 키가 없으면 상태를 비우고 `NoKey`.
//
// 스로틀: 사용액은 요약기가 호출할 때만 움직이므로 5분 간격이면 충분하다.
// 상세 창을 열 때는 `force`로 즉시 조회하되, 창을 연달아 여닫아도 요청이
// 쏟아지지 않게 15초 하한은 남긴다. 실패는 마지막 성공 값을 지우지 않는다.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;

const KEY_ENDPOINT: &str = "https://openrouter.ai/api/v1/key";
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// 정기 조회 간격(실패 백오프 겸 60초 폴링 중복 차단).
const MIN_GAP_MS: i64 = 5 * 60 * 1000;
/// `force` 조회의 하한.
const FORCE_MIN_GAP_MS: i64 = 15 * 1000;

/// 이 키의 사용액 스냅샷. TS `OpenRouterUsage` 미러(camelCase). 금액은 전부
/// USD. 응답에 없던 값은 null이다(0과 구분).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRouterUsage {
    pub fetched_at_ms: i64,
    /// 키 이름(OpenRouter가 붙인 마스킹 표기 또는 사용자가 정한 이름).
    pub label: Option<String>,
    pub usage_daily: Option<f64>,
    pub usage_weekly: Option<f64>,
    pub usage_monthly: Option<f64>,
    /// 키 생성 이후 누적.
    pub usage_total: f64,
    /// 키 한도. 한도를 걸지 않은 키는 null.
    pub limit: Option<f64>,
    pub limit_remaining: Option<f64>,
    /// 한도 리셋 주기(`daily`/`weekly`/`monthly` 원문). 리셋 없는 한도는 null.
    pub limit_reset: Option<String>,
    pub limit_resets_at_ms: Option<i64>,
    pub is_free_tier: bool,
}

/// 마지막 조회 시도의 결과. TS `OpenRouterLiveOutcome` 미러(serde snake_case).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OpenRouterLiveOutcome {
    #[default]
    NeverAttempted,
    Ok,
    /// 키가 저장돼 있지 않고 `OPENROUTER_API_KEY`도 없음.
    NoKey,
    /// 서버가 키를 거부(401/403).
    Unauthorized,
    HttpError,
    NetworkError,
    UnexpectedResponse,
}

/// 조회 진단. TS `OpenRouterLiveStatus` 미러(camelCase). 항상 존재한다.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OpenRouterLiveStatus {
    pub outcome: OpenRouterLiveOutcome,
    /// 진단 보조(예: "HTTP 500", "시간 초과"). 키 문자열은 절대 넣지 않는다.
    pub detail: Option<String>,
    pub last_attempt_ms: Option<i64>,
    pub last_success_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OpenRouterFailure {
    pub outcome: OpenRouterLiveOutcome,
    pub detail: Option<String>,
}

impl OpenRouterFailure {
    fn new(outcome: OpenRouterLiveOutcome, detail: impl Into<String>) -> Self {
        Self {
            outcome,
            detail: Some(detail.into()),
        }
    }
}

/// 키로 `/key`를 GET 해 사용액을 얻는다. 실패는 사유를 분류해 돌려준다.
pub(super) async fn fetch(key: &str, now_ms: i64) -> Result<OpenRouterUsage, OpenRouterFailure> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        // claude_live와 같은 이유로 번들 루트와 OS 스토어 루트를 모두 신뢰한다.
        .tls_built_in_webpki_certs(true)
        .tls_built_in_native_certs(true)
        .build()
        .map_err(|_| {
            OpenRouterFailure::new(
                OpenRouterLiveOutcome::NetworkError,
                "클라이언트 초기화 실패",
            )
        })?;
    let resp = client
        .get(KEY_ENDPOINT)
        .bearer_auth(key)
        .send()
        .await
        .map_err(|e| {
            // reqwest 오류 문자열은 싣지 않고 고정 어휘로만 분류한다.
            let detail = if e.is_timeout() {
                "시간 초과"
            } else if e.is_connect() {
                "연결 실패"
            } else {
                "요청 실패"
            };
            OpenRouterFailure::new(OpenRouterLiveOutcome::NetworkError, detail)
        })?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(http_failure(status));
    }
    let body: Value = resp.json().await.map_err(|_| {
        OpenRouterFailure::new(
            OpenRouterLiveOutcome::UnexpectedResponse,
            "본문이 JSON이 아님",
        )
    })?;
    parse_key_response(&body, now_ms).ok_or_else(|| {
        OpenRouterFailure::new(
            OpenRouterLiveOutcome::UnexpectedResponse,
            "사용액 필드가 없음",
        )
    })
}

/// 비2xx 상태코드 → 실패 분류(순수).
pub(super) fn http_failure(status: u16) -> OpenRouterFailure {
    let outcome = if status == 401 || status == 403 {
        OpenRouterLiveOutcome::Unauthorized
    } else {
        OpenRouterLiveOutcome::HttpError
    };
    OpenRouterFailure::new(outcome, format!("HTTP {status}"))
}

/// `/key` 응답 → 스냅샷(순수). 누적 사용액(`data.usage`)이 없으면 아는
/// 모양이 아니라고 보고 None.
pub(super) fn parse_key_response(body: &Value, now_ms: i64) -> Option<OpenRouterUsage> {
    let data = body.get("data")?;
    let num = |k: &str| data.get(k).and_then(Value::as_f64);
    let limit_resets_at_ms = data
        .get("budgets")
        .and_then(Value::as_array)
        .and_then(|budgets| {
            budgets
                .iter()
                .find(|b| b.get("scope").and_then(Value::as_str) == Some("key_limit"))
        })
        .and_then(|b| b.get("resets_at").and_then(Value::as_str))
        .and_then(super::parse_iso8601_ms);
    Some(OpenRouterUsage {
        fetched_at_ms: now_ms,
        label: data
            .get("label")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        usage_daily: num("usage_daily"),
        usage_weekly: num("usage_weekly"),
        usage_monthly: num("usage_monthly"),
        usage_total: num("usage")?,
        limit: num("limit"),
        limit_remaining: num("limit_remaining"),
        limit_reset: data
            .get("limit_reset")
            .and_then(Value::as_str)
            .map(str::to_string),
        limit_resets_at_ms,
        is_free_tier: data
            .get("is_free_tier")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// 키 지문. 키 자체는 상태에 남기지 않는다.
fn fingerprint(key: &str) -> u64 {
    let mut h = DefaultHasher::new();
    key.hash(&mut h);
    h.finish()
}

/// 조회 메모리 상태(LiveUsageState 안에 산다). 판단·기록만 락 안에서 하고
/// 네트워크 왕복은 락 밖에서 한다.
#[derive(Default)]
pub struct OpenRouterLiveState {
    inner: Mutex<OpenRouterInner>,
}

#[derive(Default)]
struct OpenRouterInner {
    key_fingerprint: Option<u64>,
    last_success: Option<OpenRouterUsage>,
    last_attempt_ms: Option<i64>,
    outcome: OpenRouterLiveOutcome,
    detail: Option<String>,
}

impl OpenRouterLiveState {
    /// 이번 폴링에서 조회할지 판단하고, 통과하면 시도 시각을 먼저 기록한다.
    /// 키가 없으면 상태를 비우고 false. 키가 바뀌었으면 이전 키의 상태를
    /// 버리고 true.
    pub(super) fn begin_attempt_if_due(&self, key: Option<&str>, force: bool, now_ms: i64) -> bool {
        let mut guard = self.inner.lock();
        let Some(key) = key else {
            *guard = OpenRouterInner {
                outcome: OpenRouterLiveOutcome::NoKey,
                ..OpenRouterInner::default()
            };
            return false;
        };
        let fp = fingerprint(key);
        if guard.key_fingerprint != Some(fp) {
            *guard = OpenRouterInner {
                key_fingerprint: Some(fp),
                ..OpenRouterInner::default()
            };
        }
        let gap = if force { FORCE_MIN_GAP_MS } else { MIN_GAP_MS };
        let due = guard
            .last_attempt_ms
            .is_none_or(|prev| now_ms - prev >= gap);
        if due {
            guard.last_attempt_ms = Some(now_ms);
        }
        due
    }

    pub(super) fn record_success(&self, usage: OpenRouterUsage) {
        let mut guard = self.inner.lock();
        guard.last_success = Some(usage);
        guard.outcome = OpenRouterLiveOutcome::Ok;
        guard.detail = None;
    }

    pub(super) fn record_failure(&self, failure: OpenRouterFailure) {
        let mut guard = self.inner.lock();
        guard.outcome = failure.outcome;
        guard.detail = failure.detail;
    }

    /// 조회가 끝나기 전에 키가 바뀌었으면 결과를 버린다(다른 키의 값이
    /// 새 키 자리에 기록되지 않게).
    pub(super) fn same_key(&self, key: &str) -> bool {
        self.inner.lock().key_fingerprint == Some(fingerprint(key))
    }

    pub(super) fn status(&self) -> OpenRouterLiveStatus {
        let guard = self.inner.lock();
        OpenRouterLiveStatus {
            outcome: guard.outcome,
            detail: guard.detail.clone(),
            last_attempt_ms: guard.last_attempt_ms,
            last_success_ms: guard.last_success.as_ref().map(|u| u.fetched_at_ms),
        }
    }

    pub(super) fn last_success(&self) -> Option<OpenRouterUsage> {
        self.inner.lock().last_success.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `/key` 실측 응답(2026-10) 축약본.
    fn sample() -> Value {
        json!({"data": {
            "label": "sk-or-v1-abc...1cc",
            "limit": 20,
            "limit_reset": "weekly",
            "limit_remaining": 19.701240686,
            "budgets": [
                {"scope": "workspace_default_key", "reset_interval": "daily",
                 "resets_at": "2026-10-29T00:00:00.000Z"},
                {"scope": "key_limit", "reset_interval": "weekly",
                 "resets_at": "2026-11-02T00:00:00.000Z"}
            ],
            "usage": 9.543950843,
            "usage_daily": 0.293717602,
            "usage_weekly": 0.298759314,
            "usage_monthly": 0.353904423,
            "is_free_tier": false
        }})
    }

    #[test]
    fn parses_usage_limit_and_key_limit_reset() {
        let u = parse_key_response(&sample(), 1_000).unwrap();
        assert_eq!(u.fetched_at_ms, 1_000);
        assert_eq!(u.label.as_deref(), Some("sk-or-v1-abc...1cc"));
        assert_eq!(u.usage_daily, Some(0.293717602));
        assert_eq!(u.usage_weekly, Some(0.298759314));
        assert_eq!(u.usage_monthly, Some(0.353904423));
        assert_eq!(u.usage_total, 9.543950843);
        assert_eq!(u.limit, Some(20.0));
        assert_eq!(u.limit_remaining, Some(19.701240686));
        assert_eq!(u.limit_reset.as_deref(), Some("weekly"));
        // workspace 예산이 아니라 key_limit 항목의 리셋 시각을 쓴다.
        assert_eq!(
            u.limit_resets_at_ms,
            super::super::parse_iso8601_ms("2026-11-02T00:00:00.000Z")
        );
        assert!(!u.is_free_tier);
    }

    #[test]
    fn key_without_limit_has_null_limit_fields() {
        let body = json!({"data": {"usage": 1.5, "limit": null, "limit_remaining": null,
                                   "limit_reset": null, "is_free_tier": true}});
        let u = parse_key_response(&body, 0).unwrap();
        assert_eq!(u.usage_total, 1.5);
        assert_eq!(u.usage_daily, None);
        assert_eq!(u.limit, None);
        assert_eq!(u.limit_remaining, None);
        assert_eq!(u.limit_reset, None);
        assert_eq!(u.limit_resets_at_ms, None);
        assert!(u.is_free_tier);
    }

    #[test]
    fn unknown_shape_is_none() {
        assert_eq!(
            parse_key_response(&json!({"error": {"code": 401}}), 0),
            None
        );
        assert_eq!(
            parse_key_response(&json!({"data": {"label": "x"}}), 0),
            None
        );
    }

    #[test]
    fn http_failure_classifies_auth_errors() {
        assert_eq!(
            http_failure(401).outcome,
            OpenRouterLiveOutcome::Unauthorized
        );
        assert_eq!(
            http_failure(403).outcome,
            OpenRouterLiveOutcome::Unauthorized
        );
        let f = http_failure(500);
        assert_eq!(f.outcome, OpenRouterLiveOutcome::HttpError);
        assert_eq!(f.detail.as_deref(), Some("HTTP 500"));
    }

    #[test]
    fn throttles_regular_polls_and_lets_force_through_after_short_gap() {
        let s = OpenRouterLiveState::default();
        assert!(s.begin_attempt_if_due(Some("k1"), false, 0));
        assert!(
            !s.begin_attempt_if_due(Some("k1"), false, 60_000),
            "5분 안의 정기 폴링은 막힌다"
        );
        assert!(
            !s.begin_attempt_if_due(Some("k1"), true, 10_000),
            "force도 15초 하한은 지킨다"
        );
        assert!(s.begin_attempt_if_due(Some("k1"), true, 20_000));
        assert!(s.begin_attempt_if_due(Some("k1"), false, 20_000 + MIN_GAP_MS));
    }

    #[test]
    fn key_change_drops_previous_state_and_fetches_now() {
        let s = OpenRouterLiveState::default();
        assert!(s.begin_attempt_if_due(Some("k1"), false, 0));
        s.record_success(parse_key_response(&sample(), 0).unwrap());
        assert!(s.last_success().is_some());
        assert!(
            s.begin_attempt_if_due(Some("k2"), false, 1_000),
            "새 키는 바로 조회"
        );
        assert!(s.last_success().is_none(), "이전 키의 값은 버린다");
        assert!(!s.same_key("k1"));
        assert!(s.same_key("k2"));
    }

    #[test]
    fn missing_key_clears_state_and_reports_no_key() {
        let s = OpenRouterLiveState::default();
        assert!(s.begin_attempt_if_due(Some("k1"), false, 0));
        s.record_success(parse_key_response(&sample(), 0).unwrap());
        assert!(!s.begin_attempt_if_due(None, true, 1_000));
        assert!(s.last_success().is_none());
        let st = s.status();
        assert_eq!(st.outcome, OpenRouterLiveOutcome::NoKey);
        assert_eq!(st.last_attempt_ms, None);
    }

    #[test]
    fn failure_keeps_last_success() {
        let s = OpenRouterLiveState::default();
        assert!(s.begin_attempt_if_due(Some("k1"), false, 0));
        s.record_success(parse_key_response(&sample(), 0).unwrap());
        s.record_failure(http_failure(500));
        let st = s.status();
        assert_eq!(st.outcome, OpenRouterLiveOutcome::HttpError);
        assert_eq!(st.last_success_ms, Some(0));
        assert!(s.last_success().is_some());
    }
}
