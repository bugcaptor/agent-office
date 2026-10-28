// src-tauri/src/usage/opencode.rs
//
// OpenCode Go 구독 사용량. Go 구독은 5시간·주간·월간 한도(금액 기준)를 갖고,
// 각 창의 사용률(%)을 준다 — Claude/Codex와 같은 `ProviderUsage` 모양에 맞는다.
//
// 출처: `GET https://opencode.ai/zen/go/v1/usage`(Bearer = 설정에 넣은
// OpenCode API 키). 공식 문서에 없는 엔드포인트라 형식이 바뀌면 조용히
// `UnexpectedResponse`로 떨어진다. 2026-10 실측 응답:
//   {"usage": {"rolling": {"status": "ok", "percent": 0, "resetsAt": "…Z"},
//              "weekly":  {…}, "monthly": {…}}}
// `rolling`이 5시간 창이다. `status`는 "ok" 말고 다른 값을 아직 보지 못해
// 읽지 않는다.
//
// 조회·스로틀·키 교체 처리는 api_key_live가 맡는다.

use serde_json::Value;

use super::api_key_live::{get_json, ApiKeyFailure};
use super::{Provider, ProviderUsage, UsageWindow, UsageWindowKind};

const USAGE_ENDPOINT: &str = "https://opencode.ai/zen/go/v1/usage";

pub(super) async fn fetch(key: String, now_ms: i64) -> Result<ProviderUsage, ApiKeyFailure> {
    let body = get_json(USAGE_ENDPOINT, &key).await?;
    parse_usage_response(&body, now_ms)
        .ok_or_else(|| ApiKeyFailure::unexpected("아는 한도 필드가 없음"))
}

/// 응답 → 스냅샷(순수). 아는 창이 하나도 없으면 None.
pub(super) fn parse_usage_response(body: &Value, now_ms: i64) -> Option<ProviderUsage> {
    let usage = body.get("usage")?;
    let windows: Vec<UsageWindow> = [
        ("rolling", UsageWindowKind::Session, Some(300)),
        ("weekly", UsageWindowKind::Weekly, Some(7 * 24 * 60)),
        ("monthly", UsageWindowKind::Monthly, None),
    ]
    .into_iter()
    .filter_map(|(field, kind, window_minutes)| {
        let w = usage.get(field)?;
        Some(UsageWindow {
            kind,
            label: None,
            used_percent: w.get("percent")?.as_f64()?,
            resets_at_ms: w
                .get("resetsAt")
                .and_then(Value::as_str)
                .and_then(super::parse_iso8601_ms),
            window_minutes,
            is_active: None,
        })
    })
    .collect();
    if windows.is_empty() {
        return None;
    }
    Some(ProviderUsage {
        provider: Provider::Opencode,
        fetched_at_ms: now_ms,
        plan_label: Some("Go".into()),
        windows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_rolling_weekly_monthly() {
        let body = json!({"usage": {
            "rolling": {"status": "ok", "percent": 12, "resetsAt": "2026-10-29T04:18:18.000Z"},
            "weekly": {"status": "ok", "percent": 40, "resetsAt": "2026-11-02T00:00:00.000Z"},
            "monthly": {"status": "ok", "percent": 3, "resetsAt": "2026-11-13T07:08:13.000Z"}
        }});
        let u = parse_usage_response(&body, 5).unwrap();
        assert_eq!(u.provider, Provider::Opencode);
        assert_eq!(u.fetched_at_ms, 5);
        let kinds: Vec<_> = u.windows.iter().map(|w| (w.kind, w.used_percent)).collect();
        assert_eq!(
            kinds,
            vec![
                (UsageWindowKind::Session, 12.0),
                (UsageWindowKind::Weekly, 40.0),
                (UsageWindowKind::Monthly, 3.0),
            ]
        );
        assert_eq!(u.windows[0].window_minutes, Some(300));
        assert_eq!(
            u.windows[1].resets_at_ms,
            super::super::parse_iso8601_ms("2026-11-02T00:00:00.000Z")
        );
    }

    #[test]
    fn skips_malformed_windows_and_rejects_unknown_shape() {
        let partial = json!({"usage": {"rolling": {"percent": "x"}, "weekly": {"percent": 1}}});
        let u = parse_usage_response(&partial, 0).unwrap();
        assert_eq!(u.windows.len(), 1);
        assert_eq!(u.windows[0].kind, UsageWindowKind::Weekly);
        assert_eq!(u.windows[0].resets_at_ms, None);
        assert_eq!(parse_usage_response(&json!({"usage": {}}), 0), None);
        assert_eq!(parse_usage_response(&json!({"error": "x"}), 0), None);
    }
}
