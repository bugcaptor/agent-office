// src-tauri/src/usage/claude.rs
//
// Claude Code 사용량: `<claude_root>/.claude.json`의 `cachedUsageUtilization`
// 파싱(docs/usage-limits-design.md §2). 파일이 크고(100KB+) CLI가 세션 중
// 자주 rewrite하므로, 파싱 실패/키 부재는 모두 조용히 None을 돌려 프런트
// 폴링이 다음 기회에 재시도하게 한다(이전 값은 프런트가 유지).
//
// `utilization.limits[]`가 있으면 우선 사용(더 구조화·모델별 주간 포함),
// 없으면 `five_hour`/`seven_day`로 폴백한다.

use std::path::Path;

use serde_json::Value;

use super::{parse_iso8601_ms, Provider, ProviderUsage, UsageWindow, UsageWindowKind};

/// `<claude_root>/.claude.json`을 읽어 ProviderUsage를 만든다. 실패는 None.
pub fn load(claude_root: &Path) -> Option<ProviderUsage> {
    let path = claude_root.join(".claude.json");
    let content = std::fs::read_to_string(&path).ok()?;
    let root: Value = serde_json::from_str(&content).ok()?;
    parse_usage(&root)
}

/// 파싱 본체(파일 I/O와 분리 — 픽스처 Value로 단위 테스트 가능).
fn parse_usage(root: &Value) -> Option<ProviderUsage> {
    let cached = root.get("cachedUsageUtilization")?;
    let fetched_at_ms = cached.get("fetchedAtMs")?.as_i64()?;
    let util = cached.get("utilization")?;

    // 플랜 라벨은 best-effort — 없으면 null. 실측 위치는
    // `oauthAccount.organizationRateLimitTier`(예: "default_claude_max_20x"),
    // 루트 조회는 혹시 모를 구버전/변형 대비 폴백.
    let plan_label = root
        .get("oauthAccount")
        .and_then(|a| a.get("organizationRateLimitTier"))
        .or_else(|| root.get("organizationRateLimitTier"))
        .and_then(Value::as_str)
        .map(str::to_string);

    // limits[] 우선, 없으면 five_hour/seven_day 폴백.
    let windows = parse_limits(util).or_else(|| parse_fallback(util))?;
    Some(ProviderUsage {
        provider: Provider::Claude,
        fetched_at_ms,
        plan_label,
        windows,
    })
}

/// `utilization.limits[]` → 윈도 배열. 배열이 없거나 비어 있으면(또는 유효
/// 항목이 하나도 없으면) None을 돌려 폴백을 유도한다.
fn parse_limits(util: &Value) -> Option<Vec<UsageWindow>> {
    let arr = util.get("limits")?.as_array()?;
    let mut out = Vec::new();
    for item in arr {
        // percent가 없으면 이 항목만 스킵(부분 파손 내구성).
        let Some(used_percent) = item.get("percent").and_then(Value::as_f64) else {
            continue;
        };
        let kind = match item.get("kind").and_then(Value::as_str).unwrap_or("") {
            "session" => UsageWindowKind::Session,
            "weekly_all" => UsageWindowKind::Weekly,
            "weekly_scoped" => UsageWindowKind::WeeklyModel,
            _ => UsageWindowKind::Unknown,
        };
        // weekly_scoped의 모델 표시명(scope.model.display_name).
        let label = item
            .get("scope")
            .and_then(|s| s.get("model"))
            .and_then(|m| m.get("display_name"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let resets_at_ms = item
            .get("resets_at")
            .and_then(Value::as_str)
            .and_then(parse_iso8601_ms);
        // 있는 그대로 전달 — "지금 구속 중인 윈도" 표시일 뿐 유효성이
        // 아니므로 여기서 필터링하지 않는다(모듈 상단 doc 참고).
        let is_active = item.get("is_active").and_then(Value::as_bool);
        out.push(UsageWindow {
            kind,
            label,
            used_percent,
            resets_at_ms,
            window_minutes: None,
            is_active,
        });
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// limits[] 부재 시 폴백: five_hour → session, seven_day → weekly.
fn parse_fallback(util: &Value) -> Option<Vec<UsageWindow>> {
    let mut out = Vec::new();
    if let Some(w) = simple_window(util.get("five_hour"), UsageWindowKind::Session) {
        out.push(w);
    }
    if let Some(w) = simple_window(util.get("seven_day"), UsageWindowKind::Weekly) {
        out.push(w);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// `{ "utilization": n, "resets_at": "iso" }` 모양을 윈도로. null/부재/utilization
/// 부재는 None.
fn simple_window(v: Option<&Value>, kind: UsageWindowKind) -> Option<UsageWindow> {
    let v = v?;
    if v.is_null() {
        return None;
    }
    let used_percent = v.get("utilization").and_then(Value::as_f64)?;
    let resets_at_ms = v
        .get("resets_at")
        .and_then(Value::as_str)
        .and_then(parse_iso8601_ms);
    Some(UsageWindow {
        kind,
        label: None,
        used_percent,
        resets_at_ms,
        window_minutes: None,
        // five_hour/seven_day 폴백 경로에는 is_active가 없다.
        is_active: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch() -> PathBuf {
        std::env::temp_dir().join(format!("usage-claude-{}", uuid::Uuid::new_v4()))
    }

    fn write_claude_json(root: &Path, body: &str) {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(root.join(".claude.json"), body).unwrap();
    }

    #[test]
    fn prefers_limits_array_with_model_scope_and_kind_mapping() {
        let root = scratch();
        write_claude_json(
            &root,
            r#"{
              "oauthAccount": { "organizationRateLimitTier": "max_20x" },
              "cachedUsageUtilization": {
                "fetchedAtMs": 1784281391475,
                "utilization": {
                  "five_hour": { "utilization": 99, "resets_at": "2026-07-17T09:50:00+00:00" },
                  "limits": [
                    { "kind": "session", "percent": 61, "resets_at": "2026-07-17T09:50:00.243466+00:00", "is_active": true },
                    { "kind": "weekly_all", "percent": 18, "resets_at": "2026-07-21T04:00:00+00:00", "is_active": false },
                    { "kind": "weekly_scoped", "percent": 24, "resets_at": "2026-07-21T04:00:00+00:00",
                      "scope": { "model": { "id": null, "display_name": "Fable" } }, "is_active": false }
                  ]
                }
              }
            }"#,
        );
        let usage = load(&root).unwrap();
        assert_eq!(usage.provider, Provider::Claude);
        assert_eq!(usage.fetched_at_ms, 1784281391475);
        assert_eq!(usage.plan_label.as_deref(), Some("max_20x"));
        // five_hour가 아니라 limits[]를 썼는지: 세 항목.
        assert_eq!(usage.windows.len(), 3);
        assert_eq!(usage.windows[0].kind, UsageWindowKind::Session);
        assert_eq!(usage.windows[0].used_percent, 61.0);
        assert_eq!(usage.windows[1].kind, UsageWindowKind::Weekly);
        assert_eq!(usage.windows[2].kind, UsageWindowKind::WeeklyModel);
        assert_eq!(usage.windows[2].label.as_deref(), Some("Fable"));
        // ISO → epoch ms 정규화(2026-07-17T09:50:00.243466Z → .243 ms 절단).
        assert_eq!(usage.windows[0].resets_at_ms, Some(1_784_281_800_243));
        // is_active는 있는 그대로 전달된다 — "지금 구속 중인 윈도" 표시일
        // 뿐이라 weekly_all/weekly_scoped가 살아있는 한도인데도 false로
        // 올 수 있다(실측). 걸러내지 않고 그대로 넘긴다.
        assert_eq!(usage.windows[0].is_active, Some(true));
        assert_eq!(usage.windows[1].is_active, Some(false));
        assert_eq!(usage.windows[2].is_active, Some(false));
    }

    #[test]
    fn plan_label_falls_back_to_root_when_no_oauth_account() {
        let root = scratch();
        write_claude_json(
            &root,
            r#"{
              "organizationRateLimitTier": "legacy_tier",
              "cachedUsageUtilization": {
                "fetchedAtMs": 1,
                "utilization": {
                  "limits": [ { "kind": "session", "percent": 5, "resets_at": "2026-07-17T09:50:00+00:00" } ]
                }
              }
            }"#,
        );
        let usage = load(&root).unwrap();
        assert_eq!(usage.plan_label.as_deref(), Some("legacy_tier"));
    }

    #[test]
    fn missing_is_active_in_limit_item_yields_none() {
        let root = scratch();
        write_claude_json(
            &root,
            r#"{
              "cachedUsageUtilization": {
                "fetchedAtMs": 1,
                "utilization": {
                  "limits": [ { "kind": "session", "percent": 5, "resets_at": "2026-07-17T09:50:00+00:00" } ]
                }
              }
            }"#,
        );
        let usage = load(&root).unwrap();
        assert_eq!(usage.windows[0].is_active, None);
    }

    #[test]
    fn fallback_windows_have_null_is_active() {
        let root = scratch();
        write_claude_json(
            &root,
            r#"{
              "cachedUsageUtilization": {
                "fetchedAtMs": 1,
                "utilization": {
                  "five_hour": { "utilization": 5, "resets_at": "2026-07-17T09:50:00+00:00" },
                  "seven_day": { "utilization": 3, "resets_at": "2026-07-21T04:00:00+00:00" }
                }
              }
            }"#,
        );
        let usage = load(&root).unwrap();
        assert_eq!(usage.windows[0].is_active, None);
        assert_eq!(usage.windows[1].is_active, None);
    }

    #[test]
    fn falls_back_to_five_hour_and_seven_day_when_no_limits() {
        let root = scratch();
        write_claude_json(
            &root,
            r#"{
              "cachedUsageUtilization": {
                "fetchedAtMs": 1784281391475,
                "utilization": {
                  "five_hour": { "utilization": 61, "resets_at": "2026-07-17T09:50:00+00:00" },
                  "seven_day": { "utilization": 18, "resets_at": "2026-07-21T04:00:00+00:00" }
                }
              }
            }"#,
        );
        let usage = load(&root).unwrap();
        assert_eq!(usage.plan_label, None);
        assert_eq!(usage.windows.len(), 2);
        assert_eq!(usage.windows[0].kind, UsageWindowKind::Session);
        assert_eq!(usage.windows[0].used_percent, 61.0);
        assert_eq!(usage.windows[1].kind, UsageWindowKind::Weekly);
        assert_eq!(usage.windows[1].used_percent, 18.0);
    }

    #[test]
    fn empty_limits_array_falls_back() {
        let root = scratch();
        write_claude_json(
            &root,
            r#"{
              "cachedUsageUtilization": {
                "fetchedAtMs": 1,
                "utilization": {
                  "limits": [],
                  "five_hour": { "utilization": 5, "resets_at": "2026-07-17T09:50:00+00:00" }
                }
              }
            }"#,
        );
        let usage = load(&root).unwrap();
        assert_eq!(usage.windows.len(), 1);
        assert_eq!(usage.windows[0].kind, UsageWindowKind::Session);
    }

    #[test]
    fn null_seven_day_is_skipped_in_fallback() {
        let root = scratch();
        write_claude_json(
            &root,
            r#"{
              "cachedUsageUtilization": {
                "fetchedAtMs": 1,
                "utilization": {
                  "five_hour": { "utilization": 5, "resets_at": "2026-07-17T09:50:00+00:00" },
                  "seven_day": null
                }
              }
            }"#,
        );
        let usage = load(&root).unwrap();
        assert_eq!(usage.windows.len(), 1);
        assert_eq!(usage.windows[0].kind, UsageWindowKind::Session);
    }

    #[test]
    fn corrupt_json_yields_none() {
        let root = scratch();
        write_claude_json(&root, "{ this is not valid json ");
        assert!(load(&root).is_none());
    }

    #[test]
    fn missing_file_yields_none() {
        assert!(load(&scratch()).is_none());
    }

    #[test]
    fn missing_cached_key_yields_none() {
        let root = scratch();
        write_claude_json(&root, r#"{"somethingElse": 1}"#);
        assert!(load(&root).is_none());
    }

    #[test]
    fn unknown_limit_kind_maps_to_unknown() {
        let root = scratch();
        write_claude_json(
            &root,
            r#"{
              "cachedUsageUtilization": {
                "fetchedAtMs": 1,
                "utilization": {
                  "limits": [ { "kind": "mystery_future_kind", "percent": 3 } ]
                }
              }
            }"#,
        );
        let usage = load(&root).unwrap();
        assert_eq!(usage.windows[0].kind, UsageWindowKind::Unknown);
        assert_eq!(usage.windows[0].resets_at_ms, None);
    }
}
