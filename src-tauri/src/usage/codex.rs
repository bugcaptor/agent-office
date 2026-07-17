// src-tauri/src/usage/codex.rs
//
// Codex CLI 사용량: `<codex_root>/sessions/YYYY/MM/DD/rollout-*.jsonl`의
// `token_count` 이벤트에 append된 `rate_limits` 스냅샷을 읽는다
// (docs/usage-limits-design.md §2).
//
// 스캔 전략: 가장 최근 날짜 디렉터리 7개만, 각 디렉터리 안에서는 파일 mtime
// 내림차순으로 순회하며 각 파일에서 마지막 non-null `rate_limits` 라인을 찾고,
// 처음 발견되는 즉시 전체 스캔을 중단한다. `window_minutes`로 윈도 종류를
// 판별하고(300=5시간, 10080=주간), `resets_at`은 유닉스 초라 ms로 변환한다.
//
// 참고: 설계의 "최근 7일"은 스캔 비용 상한을 뜻한다. 벽시계 대신 실재하는
// 날짜 디렉터리를 날짜 내림차순으로 정렬해 상위 7개를 취한다 — 결정적이라
// tempdir 주입 테스트가 쉽고, CLI 미사용 구간에는 (신선도 표시와 함께) 조금
// 오래된 실값이라도 보여줄 수 있다.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{parse_iso8601_ms, Provider, ProviderUsage, UsageWindow, UsageWindowKind};

/// 최근 스캔에서 훑을 날짜 디렉터리 개수 상한.
const RECENT_DAYS: usize = 7;

/// `<codex_root>/sessions`에서 가장 최근 non-null rate_limits 스냅샷을 찾는다.
/// 없으면 None.
pub fn load(codex_root: &Path) -> Option<ProviderUsage> {
    let sessions = codex_root.join("sessions");
    for dir in recent_date_dirs(&sessions, RECENT_DAYS) {
        for file in rollout_files_by_mtime_desc(&dir) {
            if let Some(usage) = parse_file(&file) {
                return Some(usage);
            }
        }
    }
    None
}

/// `sessions/YYYY/MM/DD` 디렉터리들을 날짜 내림차순으로 정렬해 상위 `limit`개.
fn recent_date_dirs(sessions: &Path, limit: usize) -> Vec<PathBuf> {
    let mut dated: Vec<(chrono::NaiveDate, PathBuf)> = Vec::new();
    let Ok(years) = std::fs::read_dir(sessions) else {
        return Vec::new();
    };
    for y in years.flatten() {
        let Some(year) = parse_component::<i32>(&y) else {
            continue;
        };
        let Ok(months) = std::fs::read_dir(y.path()) else {
            continue;
        };
        for m in months.flatten() {
            let Some(month) = parse_component::<u32>(&m) else {
                continue;
            };
            let Ok(days) = std::fs::read_dir(m.path()) else {
                continue;
            };
            for d in days.flatten() {
                let Some(day) = parse_component::<u32>(&d) else {
                    continue;
                };
                if let Some(date) = chrono::NaiveDate::from_ymd_opt(year, month, day) {
                    dated.push((date, d.path()));
                }
            }
        }
    }
    dated.sort_by(|a, b| b.0.cmp(&a.0));
    dated.into_iter().take(limit).map(|(_, p)| p).collect()
}

/// 디렉터리 엔트리 이름을 정수로 파싱(YYYY/MM/DD 컴포넌트용).
fn parse_component<T: std::str::FromStr>(entry: &std::fs::DirEntry) -> Option<T> {
    entry.file_name().to_str()?.parse::<T>().ok()
}

/// 디렉터리 안의 `rollout-*.jsonl`을 mtime 내림차순으로.
fn rollout_files_by_mtime_desc(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
            continue;
        }
        let mtime = e
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        files.push((mtime, e.path()));
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.into_iter().map(|(_, p)| p).collect()
}

/// 파일 끝에서부터 마지막 non-null rate_limits 라인을 찾아 ProviderUsage로.
/// non-null = rate_limits 객체가 있고 primary/secondary 중 하나 이상이 non-null.
fn parse_file(path: &Path) -> Option<ProviderUsage> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(rl) = v.get("payload").and_then(|p| p.get("rate_limits")) else {
            continue;
        };
        if rl.is_null() {
            continue;
        }
        let primary = rl.get("primary").filter(|x| !x.is_null());
        let secondary = rl.get("secondary").filter(|x| !x.is_null());
        if primary.is_none() && secondary.is_none() {
            continue;
        }
        let Some(fetched_at_ms) = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso8601_ms)
        else {
            continue;
        };
        let plan_label = rl
            .get("plan_type")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut windows = Vec::new();
        for w in [primary, secondary].into_iter().flatten() {
            if let Some(win) = parse_window(w) {
                windows.push(win);
            }
        }
        if windows.is_empty() {
            continue;
        }
        return Some(ProviderUsage {
            provider: Provider::Codex,
            fetched_at_ms,
            plan_label,
            windows,
        });
    }
    None
}

/// `{ "used_percent": n, "window_minutes": m, "resets_at": secs }` → 윈도.
/// used_percent 부재는 None. resets_at은 유닉스 초라 ×1000.
fn parse_window(w: &Value) -> Option<UsageWindow> {
    let used_percent = w.get("used_percent").and_then(Value::as_f64)?;
    let window_minutes = w.get("window_minutes").and_then(Value::as_i64);
    let kind = match window_minutes {
        Some(300) => UsageWindowKind::Session,
        Some(10080) => UsageWindowKind::Weekly,
        _ => UsageWindowKind::Unknown,
    };
    let resets_at_ms = w
        .get("resets_at")
        .and_then(Value::as_i64)
        .map(|secs| secs * 1000);
    Some(UsageWindow {
        kind,
        label: None,
        used_percent,
        resets_at_ms,
        window_minutes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch() -> PathBuf {
        std::env::temp_dir().join(format!("usage-codex-{}", uuid::Uuid::new_v4()))
    }

    /// `sessions/YYYY/MM/DD/<name>`에 내용을 쓴다.
    fn write_rollout(root: &Path, date: (&str, &str, &str), name: &str, body: &str) {
        let dir = root
            .join("sessions")
            .join(date.0)
            .join(date.1)
            .join(date.2);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), body).unwrap();
    }

    /// token_count 이벤트 한 줄.
    fn event(ts: &str, rate_limits: &str) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","rate_limits":{rate_limits}}}}}"#
        )
    }

    #[test]
    fn parses_weekly_only_prolite_snapshot() {
        let root = scratch();
        write_rollout(
            &root,
            ("2026", "07", "17"),
            "rollout-2026-07-17T20-20-13-abc.jsonl",
            &event(
                "2026-07-17T11:20:17.595Z",
                r#"{"primary":{"used_percent":11.0,"window_minutes":10080,"resets_at":1784786662},"secondary":null,"plan_type":"prolite"}"#,
            ),
        );
        let usage = load(&root).unwrap();
        assert_eq!(usage.provider, Provider::Codex);
        assert_eq!(usage.plan_label.as_deref(), Some("prolite"));
        // 신선도 = 이벤트 timestamp.
        assert_eq!(usage.fetched_at_ms, 1_784_287_217_595);
        assert_eq!(usage.windows.len(), 1);
        let w = &usage.windows[0];
        assert_eq!(w.kind, UsageWindowKind::Weekly);
        assert_eq!(w.used_percent, 11.0);
        assert_eq!(w.window_minutes, Some(10080));
        // 유닉스 초 → ms.
        assert_eq!(w.resets_at_ms, Some(1_784_786_662_000));
    }

    #[test]
    fn maps_window_minutes_to_kind_for_both_primary_and_secondary() {
        let root = scratch();
        write_rollout(
            &root,
            ("2026", "07", "17"),
            "rollout-a.jsonl",
            &event(
                "2026-07-17T11:20:17.595Z",
                r#"{"primary":{"used_percent":40.0,"window_minutes":300,"resets_at":1784786662},"secondary":{"used_percent":12.0,"window_minutes":10080,"resets_at":1784800000}}"#,
            ),
        );
        let usage = load(&root).unwrap();
        assert_eq!(usage.windows.len(), 2);
        assert_eq!(usage.windows[0].kind, UsageWindowKind::Session);
        assert_eq!(usage.windows[0].window_minutes, Some(300));
        assert_eq!(usage.windows[1].kind, UsageWindowKind::Weekly);
    }

    #[test]
    fn unknown_window_minutes_maps_to_unknown() {
        let root = scratch();
        write_rollout(
            &root,
            ("2026", "07", "17"),
            "rollout-a.jsonl",
            &event(
                "2026-07-17T11:20:17.595Z",
                r#"{"primary":{"used_percent":5.0,"window_minutes":1440,"resets_at":1784786662},"secondary":null}"#,
            ),
        );
        let usage = load(&root).unwrap();
        assert_eq!(usage.windows[0].kind, UsageWindowKind::Unknown);
    }

    #[test]
    fn skips_null_rate_limits_and_takes_last_non_null_in_file() {
        let root = scratch();
        let mut body = String::new();
        // 첫 줄: 유효한 옛 스냅샷.
        body.push_str(&event(
            "2026-07-17T10:00:00.000Z",
            r#"{"primary":{"used_percent":1.0,"window_minutes":10080,"resets_at":1784786662},"secondary":null}"#,
        ));
        body.push('\n');
        // 중간: rate_limits null.
        body.push_str(&event("2026-07-17T10:30:00.000Z", "null"));
        body.push('\n');
        // 그 다음: primary/secondary 모두 null.
        body.push_str(&event(
            "2026-07-17T11:00:00.000Z",
            r#"{"primary":null,"secondary":null}"#,
        ));
        body.push('\n');
        // 마지막 유효 = 이걸 취해야 한다.
        body.push_str(&event(
            "2026-07-17T11:20:17.595Z",
            r#"{"primary":{"used_percent":11.0,"window_minutes":10080,"resets_at":1784786662},"secondary":null,"plan_type":"prolite"}"#,
        ));
        body.push('\n');
        write_rollout(&root, ("2026", "07", "17"), "rollout-a.jsonl", &body);

        let usage = load(&root).unwrap();
        assert_eq!(usage.fetched_at_ms, 1_784_287_217_595);
        assert_eq!(usage.windows[0].used_percent, 11.0);
    }

    #[test]
    fn newest_date_dir_wins_over_older() {
        let root = scratch();
        // 오래된 날: 다른 값.
        write_rollout(
            &root,
            ("2026", "07", "10"),
            "rollout-old.jsonl",
            &event(
                "2026-07-10T10:00:00.000Z",
                r#"{"primary":{"used_percent":99.0,"window_minutes":10080,"resets_at":1784000000},"secondary":null}"#,
            ),
        );
        // 최신 날: 이걸 취해야 한다.
        write_rollout(
            &root,
            ("2026", "07", "17"),
            "rollout-new.jsonl",
            &event(
                "2026-07-17T11:20:17.595Z",
                r#"{"primary":{"used_percent":11.0,"window_minutes":10080,"resets_at":1784786662},"secondary":null}"#,
            ),
        );
        let usage = load(&root).unwrap();
        assert_eq!(usage.windows[0].used_percent, 11.0);
    }

    #[test]
    fn corrupt_lines_are_skipped() {
        let root = scratch();
        let body = format!(
            "not json\n\n{{\"partial\":\n{}\n",
            event(
                "2026-07-17T11:20:17.595Z",
                r#"{"primary":{"used_percent":11.0,"window_minutes":10080,"resets_at":1784786662},"secondary":null}"#,
            )
        );
        write_rollout(&root, ("2026", "07", "17"), "rollout-a.jsonl", &body);
        let usage = load(&root).unwrap();
        assert_eq!(usage.windows[0].used_percent, 11.0);
    }

    #[test]
    fn file_with_no_usable_snapshot_yields_none() {
        let root = scratch();
        write_rollout(
            &root,
            ("2026", "07", "17"),
            "rollout-a.jsonl",
            &event("2026-07-17T11:20:17.595Z", r#"{"primary":null,"secondary":null}"#),
        );
        assert!(load(&root).is_none());
    }

    #[test]
    fn missing_sessions_dir_yields_none() {
        assert!(load(&scratch()).is_none());
    }
}
