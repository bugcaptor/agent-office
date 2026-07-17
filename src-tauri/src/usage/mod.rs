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
mod claude_live;
mod codex;

use std::path::Path;

use serde::Serialize;

pub use claude_live::LiveUsageState;

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
    /// "지금 구속 중인 윈도"인지(Claude `limits[]`에만 있음). **유효성이
    /// 아니다** — 실측(`~/.claude.json`)상 weekly_all/weekly_scoped도 살아
    /// 있는 한도인데 is_active:false로 온다. 걸러내는 용도로 쓰지 말 것,
    /// 표시용 보조 정보로만 쓴다. Codex와 Claude five_hour/seven_day 폴백
    /// 경로는 항상 null.
    pub is_active: Option<bool>,
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

/// 실시간 조회를 얹은 조립(이슈 #33, docs/claude-usage-live-fetch-design.md
/// §3.2). 커맨드가 이것에 위임한다. 동기 `load_usage_snapshot`(파일 캐시
/// 미러)은 그대로 두고 그 결과의 claude 필드만 실시간 값으로 보강한다.
///
/// 흐름:
/// 1. 파일 캐시 스냅샷을 먼저 읽는다(항상 성공, 실패 소스는 None).
/// 2. 스로틀(§3.1)을 통과하면 토큰을 읽어 실시간 fetch를 시도한다. Mutex는
///    판단·기록의 짧은 임계구역에서만 잡고 fetch await는 락 밖에서 한다.
/// 3. claude 필드를 파일 캐시와 메모리 live 중 더 신선한 쪽으로 확정하되,
///    plan_label은 응답에 없으므로 파일 캐시 값을 접목한다.
///
/// `claude_config_dir`은 자격증명(.credentials.json)·스코프 Keychain의 기준
/// 디렉터리로, `.claude.json`을 읽는 `claude_root`와 다를 수 있다
/// (CLAUDE_CONFIG_DIR 미설정 시 claude_root=홈, config_dir=~/.claude).
pub async fn load_usage_snapshot_with_live(
    live: &LiveUsageState,
    claude_root: &Path,
    claude_config_dir: &Path,
    codex_root: &Path,
    now_ms: i64,
) -> UsageSnapshot {
    let mut snapshot = load_usage_snapshot(claude_root, codex_root);

    // 락 안에서 스로틀 판단 + last_attempt 갱신(중복 fetch 차단) → 락 해제 후 fetch.
    if live.begin_attempt_if_due(now_ms) {
        if let Some(token) = claude_live::read_access_token(claude_config_dir) {
            if let Some(windows) = claude_live::fetch_live(&token).await {
                live.record_success(claude_live::live_provider_usage(windows, now_ms));
            }
        }
    }

    snapshot.claude = merge_claude(snapshot.claude.take(), live.last_success());
    snapshot
}

/// 파일 캐시와 실시간 결과 중 `fetched_at_ms`가 큰 쪽을 고른다(렌더러
/// fresherProvider와 같은 규칙 — Claude Code가 방금 캐시를 갱신했다면 그쪽이
/// 이길 수 있다). 동률·live 우선(이 기능의 취지). live가 이기면 plan_label을
/// 파일 캐시에서 접목한다(live 응답엔 plan_label이 없음). 설계 §3.2.
fn merge_claude(
    file: Option<ProviderUsage>,
    live: Option<ProviderUsage>,
) -> Option<ProviderUsage> {
    match (file, live) {
        (None, None) => None,
        (Some(f), None) => Some(f),
        (None, Some(l)) => Some(l),
        (Some(f), Some(mut l)) => {
            if l.fetched_at_ms >= f.fetched_at_ms {
                if l.plan_label.is_none() {
                    l.plan_label = f.plan_label;
                }
                Some(l)
            } else {
                Some(f)
            }
        }
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

    // ── merge_claude (실시간/파일 캐시 조립) ──

    fn provider(fetched_at_ms: i64, plan_label: Option<&str>) -> ProviderUsage {
        ProviderUsage {
            provider: Provider::Claude,
            fetched_at_ms,
            plan_label: plan_label.map(str::to_string),
            windows: vec![UsageWindow {
                kind: UsageWindowKind::Session,
                label: None,
                used_percent: 42.0,
                resets_at_ms: None,
                window_minutes: None,
                is_active: None,
            }],
        }
    }

    #[test]
    fn merge_claude_live_wins_when_fresher_and_grafts_plan_label() {
        // live는 plan_label이 없다(응답에 없음). 파일 캐시에서 접목해야 한다.
        let file = provider(1_000, Some("max_20x"));
        let live = provider(2_000, None);
        let merged = merge_claude(Some(file), Some(live)).unwrap();
        assert_eq!(merged.fetched_at_ms, 2_000, "더 신선한 live가 이겨야");
        assert_eq!(merged.plan_label.as_deref(), Some("max_20x"), "plan_label 접목");
    }

    #[test]
    fn merge_claude_file_wins_when_it_is_fresher() {
        // Claude Code가 방금 캐시를 갱신한 경우 파일이 이길 수 있어야 한다.
        let file = provider(5_000, Some("max_20x"));
        let live = provider(2_000, None);
        let merged = merge_claude(Some(file), Some(live)).unwrap();
        assert_eq!(merged.fetched_at_ms, 5_000);
        assert_eq!(merged.plan_label.as_deref(), Some("max_20x"));
    }

    #[test]
    fn merge_claude_falls_back_to_file_when_no_live() {
        let file = provider(1_000, Some("max_20x"));
        let merged = merge_claude(Some(file), None).unwrap();
        assert_eq!(merged.fetched_at_ms, 1_000);
    }

    #[test]
    fn merge_claude_uses_live_when_no_file() {
        let live = provider(2_000, None);
        let merged = merge_claude(None, Some(live)).unwrap();
        assert_eq!(merged.fetched_at_ms, 2_000);
        assert_eq!(merged.plan_label, None);
    }

    #[test]
    fn merge_claude_none_when_both_absent() {
        assert_eq!(merge_claude(None, None), None);
    }
}
