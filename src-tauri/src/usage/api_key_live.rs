// src-tauri/src/usage/api_key_live.rs
//
// 앱 키 저장소의 API 키로 사용량을 조회하는 provider(OpenRouter, OpenCode
// Go)의 공용 뼈대: 조회 결과 분류, 스로틀, 키 교체 처리, HTTP GET.
// provider 모듈은 엔드포인트와 응답 파싱만 가진다.
//
// 키는 키 저장소(key_store.rs, env 폴백 포함)에서 폴링마다 다시 읽는다.
// 키가 바뀌면(지문 비교) 이전 키의 값·시도 시각을 버리고 바로 다시 조회한다
// — 남의 키 사용량이 섞여 보이면 안 된다. 키가 없으면 상태를 비우고 `NoKey`.
//
// 스로틀: 정기 조회는 5분 간격. 상세 창을 열 때는 `force`로 즉시 조회하되,
// 창을 연달아 여닫아도 요청이 쏟아지지 않게 15초 하한은 남긴다. 실패는
// 마지막 성공 값을 지우지 않는다.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;

const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// 정기 조회 간격(실패 백오프 겸 60초 폴링 중복 차단).
const MIN_GAP_MS: i64 = 5 * 60 * 1000;
/// `force` 조회의 하한.
const FORCE_MIN_GAP_MS: i64 = 15 * 1000;

/// 마지막 조회 시도의 결과. TS `ApiKeyLiveOutcome` 미러(serde snake_case).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyLiveOutcome {
    #[default]
    NeverAttempted,
    Ok,
    /// 키가 저장돼 있지 않고 env 폴백도 없음.
    NoKey,
    /// 서버가 키를 거부(401/403).
    Unauthorized,
    HttpError,
    NetworkError,
    UnexpectedResponse,
}

/// 조회 진단. TS `ApiKeyLiveStatus` 미러(camelCase). 항상 존재한다.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyLiveStatus {
    pub outcome: ApiKeyLiveOutcome,
    /// 진단 보조(예: "HTTP 500", "시간 초과"). 키 문자열은 절대 넣지 않는다.
    pub detail: Option<String>,
    pub last_attempt_ms: Option<i64>,
    pub last_success_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ApiKeyFailure {
    pub outcome: ApiKeyLiveOutcome,
    pub detail: Option<String>,
}

impl ApiKeyFailure {
    pub(super) fn new(outcome: ApiKeyLiveOutcome, detail: impl Into<String>) -> Self {
        Self {
            outcome,
            detail: Some(detail.into()),
        }
    }

    pub(super) fn unexpected(detail: impl Into<String>) -> Self {
        Self::new(ApiKeyLiveOutcome::UnexpectedResponse, detail)
    }
}

/// Bearer 키로 GET 해 JSON 본문을 얻는다. 실패는 사유를 분류해 돌려준다.
pub(super) async fn get_json(url: &str, key: &str) -> Result<Value, ApiKeyFailure> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        // claude_live와 같은 이유로 번들 루트와 OS 스토어 루트를 모두 신뢰한다.
        .tls_built_in_webpki_certs(true)
        .tls_built_in_native_certs(true)
        .build()
        .map_err(|_| {
            ApiKeyFailure::new(ApiKeyLiveOutcome::NetworkError, "클라이언트 초기화 실패")
        })?;
    let resp = client.get(url).bearer_auth(key).send().await.map_err(|e| {
        // reqwest 오류 문자열은 싣지 않고 고정 어휘로만 분류한다.
        let detail = if e.is_timeout() {
            "시간 초과"
        } else if e.is_connect() {
            "연결 실패"
        } else {
            "요청 실패"
        };
        ApiKeyFailure::new(ApiKeyLiveOutcome::NetworkError, detail)
    })?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(http_failure(status));
    }
    resp.json()
        .await
        .map_err(|_| ApiKeyFailure::unexpected("본문이 JSON이 아님"))
}

/// 비2xx 상태코드 → 실패 분류(순수).
pub(super) fn http_failure(status: u16) -> ApiKeyFailure {
    let outcome = if status == 401 || status == 403 {
        ApiKeyLiveOutcome::Unauthorized
    } else {
        ApiKeyLiveOutcome::HttpError
    };
    ApiKeyFailure::new(outcome, format!("HTTP {status}"))
}

/// 키 지문. 키 자체는 상태에 남기지 않는다.
fn fingerprint(key: &str) -> u64 {
    let mut h = DefaultHasher::new();
    key.hash(&mut h);
    h.finish()
}

/// 조회 메모리 상태(LiveUsageState 안에 산다). 판단·기록만 락 안에서 하고
/// 네트워크 왕복은 락 밖에서 한다. `T`는 provider의 스냅샷 모양.
pub struct ApiKeyLiveState<T> {
    inner: Mutex<Inner<T>>,
}

impl<T> Default for ApiKeyLiveState<T> {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }
}

struct Inner<T> {
    key_fingerprint: Option<u64>,
    last_success: Option<T>,
    last_success_ms: Option<i64>,
    last_attempt_ms: Option<i64>,
    outcome: ApiKeyLiveOutcome,
    detail: Option<String>,
}

impl<T> Default for Inner<T> {
    fn default() -> Self {
        Self {
            key_fingerprint: None,
            last_success: None,
            last_success_ms: None,
            last_attempt_ms: None,
            outcome: ApiKeyLiveOutcome::default(),
            detail: None,
        }
    }
}

impl<T: Clone> ApiKeyLiveState<T> {
    /// 이번 폴링에서 조회할지 판단하고, 통과하면 시도 시각을 먼저 기록한다.
    /// 키가 없으면 상태를 비우고 false. 키가 바뀌었으면 이전 키의 상태를
    /// 버리고 true.
    pub(super) fn begin_attempt_if_due(&self, key: Option<&str>, force: bool, now_ms: i64) -> bool {
        let mut guard = self.inner.lock();
        let Some(key) = key else {
            *guard = Inner {
                outcome: ApiKeyLiveOutcome::NoKey,
                ..Inner::default()
            };
            return false;
        };
        let fp = fingerprint(key);
        if guard.key_fingerprint != Some(fp) {
            *guard = Inner {
                key_fingerprint: Some(fp),
                ..Inner::default()
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

    /// 조회 결과를 기록한다. 조회가 끝나기 전에 키가 바뀌었으면 버린다(다른
    /// 키의 값이 새 키 자리에 기록되지 않게).
    pub(super) fn record(&self, key: &str, result: Result<T, ApiKeyFailure>, now_ms: i64) {
        let mut guard = self.inner.lock();
        if guard.key_fingerprint != Some(fingerprint(key)) {
            return;
        }
        match result {
            Ok(value) => {
                guard.last_success = Some(value);
                guard.last_success_ms = Some(now_ms);
                guard.outcome = ApiKeyLiveOutcome::Ok;
                guard.detail = None;
            }
            Err(failure) => {
                guard.outcome = failure.outcome;
                guard.detail = failure.detail;
            }
        }
    }

    pub(super) fn status(&self) -> ApiKeyLiveStatus {
        let guard = self.inner.lock();
        ApiKeyLiveStatus {
            outcome: guard.outcome,
            detail: guard.detail.clone(),
            last_attempt_ms: guard.last_attempt_ms,
            last_success_ms: guard.last_success_ms,
        }
    }

    pub(super) fn last_success(&self) -> Option<T> {
        self.inner.lock().last_success.clone()
    }
}

/// 조회 1회분: 스로틀·키 판단 → 조회 → 기록.
pub(super) async fn refresh<T, F, Fut>(
    state: &ApiKeyLiveState<T>,
    key: Option<&str>,
    force: bool,
    now_ms: i64,
    fetch: F,
) where
    T: Clone,
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<T, ApiKeyFailure>>,
{
    if !state.begin_attempt_if_due(key, force, now_ms) {
        return;
    }
    let Some(key) = key else { return };
    let result = fetch(key.to_string()).await;
    state.record(key, result, now_ms);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_failure_classifies_auth_errors() {
        assert_eq!(http_failure(401).outcome, ApiKeyLiveOutcome::Unauthorized);
        assert_eq!(http_failure(403).outcome, ApiKeyLiveOutcome::Unauthorized);
        let f = http_failure(500);
        assert_eq!(f.outcome, ApiKeyLiveOutcome::HttpError);
        assert_eq!(f.detail.as_deref(), Some("HTTP 500"));
    }

    #[test]
    fn throttles_regular_polls_and_lets_force_through_after_short_gap() {
        let s = ApiKeyLiveState::<u32>::default();
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
        let s = ApiKeyLiveState::<u32>::default();
        assert!(s.begin_attempt_if_due(Some("k1"), false, 0));
        s.record("k1", Ok(7), 0);
        assert_eq!(s.last_success(), Some(7));
        assert!(
            s.begin_attempt_if_due(Some("k2"), false, 1_000),
            "새 키는 바로 조회"
        );
        assert_eq!(s.last_success(), None, "이전 키의 값은 버린다");
        // 키가 바뀌기 전에 떠난 k1 조회의 결과는 k2 자리에 기록되지 않는다.
        s.record("k1", Ok(9), 1_500);
        assert_eq!(s.last_success(), None);
        s.record("k2", Ok(3), 2_000);
        assert_eq!(s.last_success(), Some(3));
        assert_eq!(s.status().last_success_ms, Some(2_000));
    }

    #[test]
    fn missing_key_clears_state_and_reports_no_key() {
        let s = ApiKeyLiveState::<u32>::default();
        assert!(s.begin_attempt_if_due(Some("k1"), false, 0));
        s.record("k1", Ok(7), 0);
        assert!(!s.begin_attempt_if_due(None, true, 1_000));
        assert_eq!(s.last_success(), None);
        let st = s.status();
        assert_eq!(st.outcome, ApiKeyLiveOutcome::NoKey);
        assert_eq!(st.last_attempt_ms, None);
    }

    #[test]
    fn failure_keeps_last_success() {
        let s = ApiKeyLiveState::<u32>::default();
        assert!(s.begin_attempt_if_due(Some("k1"), false, 0));
        s.record("k1", Ok(7), 0);
        s.record("k1", Err(http_failure(500)), 100);
        let st = s.status();
        assert_eq!(st.outcome, ApiKeyLiveOutcome::HttpError);
        assert_eq!(st.last_success_ms, Some(0));
        assert_eq!(s.last_success(), Some(7));
    }
}
