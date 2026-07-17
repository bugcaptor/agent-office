// src-tauri/src/usage/mod.rs
//
// 구독 사용량(rate limit) 스냅샷 조립. 설계: docs/usage-limits-design.md.
// 기존 session-analytics 관례를 따라 백엔드는 정규화된 원시 스냅샷만 반환하고
// 집계·해석·표시는 프런트가 한다. 두 소스(claude/codex)는 서로 독립적으로
// 파싱되며, 실패한 소스는 해당 provider가 None일 뿐 조립 자체는 항상 성공한다
// (반환은 오류 없이 축소로만 나타남).
//
// 단위 정규화는 전부 여기서: resets_at → epoch ms(Claude ISO8601 파싱,
// Codex 유닉스 초→ms), 신선도(fetchedAtMs)도 epoch ms.

mod claude;
mod codex;

use std::path::Path;

use serde::Serialize;

/// 한도 윈도 종류. TS `UsageWindowKind` 미러(serde snake_case).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageWindowKind {
    Session,
    Weekly,
    WeeklyModel,
    Unknown,
}

/// CLI provider. TS `"claude" | "codex"` 미러(serde lowercase).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    Codex,
}

/// 한도 윈도 1개. TS `UsageWindow` 미러(camelCase). nullable 필드는
/// skip 하지 않고 null로 직렬화한다(TS는 `T | null`이지 optional이 아님).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub kind: UsageWindowKind,
    /// weekly_model일 때 모델 표시명 등. 없으면 null.
    pub label: Option<String>,
    pub used_percent: f64,
    /// epoch ms로 정규화. 파싱 불가/부재 시 null.
    pub resets_at_ms: Option<i64>,
    pub window_minutes: Option<i64>,
}

/// provider별 사용량. TS `ProviderUsage` 미러.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsage {
    pub provider: Provider,
    /// 신선도 기준 시각(epoch ms).
    pub fetched_at_ms: i64,
    /// codex plan_type, claude organizationRateLimitTier 등. 없으면 null.
    pub plan_label: Option<String>,
    /// 가변 배열 — UI가 "5시간+주간 둘 다"를 하드코딩하지 않는다.
    pub windows: Vec<UsageWindow>,
}

/// 전체 스냅샷. TS `UsageSnapshot` 미러. 실패한 소스는 null.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsageSnapshot {
    pub claude: Option<ProviderUsage>,
    pub codex: Option<ProviderUsage>,
}

/// `claude_root`(홈, `.claude.json`이 이 아래)와 `codex_root`(`~/.codex`,
/// `sessions/`가 이 아래)에서 각 provider 스냅샷을 읽어 조립한다. 각 소스
/// 파싱은 독립적이며 실패 시 해당 필드가 None이 된다.
pub fn load_usage_snapshot(claude_root: &Path, codex_root: &Path) -> UsageSnapshot {
    UsageSnapshot {
        claude: claude::load(claude_root),
        codex: codex::load(codex_root),
    }
}

/// timezone 포함 ISO8601/RFC3339 문자열을 epoch ms로. Claude의
/// `2026-07-17T09:50:00.243466+00:00`(소수 초 + 오프셋)와 Codex의
/// `2026-07-17T11:20:17.595Z`(Z 접미)를 모두 처리한다. 실패 시 None.
pub(super) fn parse_iso8601_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_style_offset_timestamp() {
        // 2026-07-17T09:50:00Z == 1_784_281_800_000 ms.
        let ms = parse_iso8601_ms("2026-07-17T09:50:00.000000+00:00").unwrap();
        assert_eq!(ms, 1_784_281_800_000);
    }

    #[test]
    fn parses_codex_style_zulu_timestamp() {
        let ms = parse_iso8601_ms("2026-07-17T09:50:00.000Z").unwrap();
        assert_eq!(ms, 1_784_281_800_000);
    }

    #[test]
    fn rejects_garbage_timestamp() {
        assert_eq!(parse_iso8601_ms("not a date"), None);
    }

    #[test]
    fn missing_sources_yield_null_provider_but_snapshot_succeeds() {
        let root = std::env::temp_dir().join(format!("usage-empty-{}", uuid::Uuid::new_v4()));
        let snap = load_usage_snapshot(&root, &root);
        assert!(snap.claude.is_none());
        assert!(snap.codex.is_none());
    }
}
