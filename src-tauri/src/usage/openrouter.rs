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
// 조회·스로틀·키 교체 처리는 api_key_live가 맡고, 여기는 엔드포인트와
// 응답 파싱만 가진다.

use serde::Serialize;
use serde_json::Value;

use super::api_key_live::{get_json, ApiKeyFailure};

const KEY_ENDPOINT: &str = "https://openrouter.ai/api/v1/key";

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

/// 키로 `/key`를 GET 해 사용액을 얻는다.
pub(super) async fn fetch(key: String, now_ms: i64) -> Result<OpenRouterUsage, ApiKeyFailure> {
    let body = get_json(KEY_ENDPOINT, &key).await?;
    parse_key_response(&body, now_ms).ok_or_else(|| ApiKeyFailure::unexpected("사용액 필드가 없음"))
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
}
