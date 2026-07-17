// src-tauri/src/usage/codex.rs
//
// Codex CLI 사용량: `<codex_root>/sessions/YYYY/MM/DD/rollout-*.jsonl`의
// `token_count` 이벤트에 append된 `rate_limits` 스냅샷을 읽는다
// (docs/usage-limits-design.md §2).
//
// 스캔 전략: 가장 최근 날짜 디렉터리 7개에서 모든 rollout 파일을 모아 파일
// mtime 내림차순으로(날짜 디렉터리 경계 없이 전역) 정렬한 뒤, 각 파일에서
// 마지막 non-null `rate_limits` 라인(스냅샷)을 파싱해 그 스냅샷의
// `fetched_at_ms`(이벤트 timestamp)가 가장 큰 후보를 취한다. mtime이 아니라
// 스냅샷 자체의 timestamp로 승자를 가리는 이유:
//   (a) 동시 세션 — mtime이 가장 최신인 파일이라도 파일 끝부분이 null
//       rate_limits 이벤트로 끝나면, 그 파일의 마지막 유효 스냅샷은 다른
//       파일의 스냅샷보다 오래될 수 있다.
//   (b) 장기 세션 — rollout 파일은 세션 "시작" 날짜 디렉터리에 계속
//       append되므로, 오래된 날짜 디렉터리의 파일이 실제로는 가장 신선한
//       스냅샷을 담고 있을 수 있다.
// 조기 종료: 파일 내 스냅샷 timestamp는 그 파일의 mtime보다 늦을 수 없으므로,
// 전역 mtime 내림차순 순회 중 현재 최선 후보의 fetched_at_ms가 다음 파일의
// mtime(epoch ms) 이상이 되는 순간 나머지(mtime이 더 낮은) 파일들은 이를
// 넘어설 수 없어 스캔을 중단한다. `window_minutes`로 윈도 종류를
// 판별하고(300=5시간, 10080=주간), `resets_at`은 유닉스 초라 ms로 변환한다.
//
// 참고: 설계의 "최근 7일"은 스캔 비용 상한을 뜻한다. 벽시계 대신 실재하는
// 날짜 디렉터리를 날짜 내림차순으로 정렬해 상위 7개를 취한다 — 결정적이라
// tempdir 주입 테스트가 쉽고, CLI 미사용 구간에는 (신선도 표시와 함께) 조금
// 오래된 실값이라도 보여줄 수 있다.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::{parse_iso8601_ms, Provider, ProviderUsage, UsageWindow, UsageWindowKind};

/// 최근 스캔에서 훑을 날짜 디렉터리 개수 상한.
const RECENT_DAYS: usize = 7;

/// `<codex_root>/sessions`에서 가장 최근(스냅샷 timestamp 기준) non-null
/// rate_limits 스냅샷을 찾는다. 없으면 None.
pub fn load(codex_root: &Path) -> Option<ProviderUsage> {
    let sessions = codex_root.join("sessions");
    let files = all_rollout_files_by_mtime_desc(&sessions, RECENT_DAYS);
    let mut best: Option<ProviderUsage> = None;
    for (mtime, file) in files {
        // 조기 종료: 남은 파일들은 mtime이 이 파일 이하이므로, 현재 최선
        // 후보의 스냅샷이 이 파일의 mtime보다도 이미 최신이면 더 볼 필요 없다.
        if let Some(best_usage) = &best {
            if best_usage.fetched_at_ms >= epoch_ms(mtime) {
                break;
            }
        }
        if let Some(usage) = parse_file(&file) {
            let is_better = best
                .as_ref()
                .is_none_or(|b| usage.fetched_at_ms > b.fetched_at_ms);
            if is_better {
                best = Some(usage);
            }
        }
    }
    best
}

/// `SystemTime`을 유닉스 epoch 밀리초로. 변환 실패(UNIX_EPOCH 이전)는 0.
fn epoch_ms(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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

/// 최근 `limit_dirs`개 날짜 디렉터리를 통틀어 모든 `rollout-*.jsonl`을 모아
/// 파일 mtime 내림차순으로(날짜 디렉터리 경계 없이 전역) 정렬한다.
fn all_rollout_files_by_mtime_desc(
    sessions: &Path,
    limit_dirs: usize,
) -> Vec<(SystemTime, PathBuf)> {
    let mut files: Vec<(SystemTime, PathBuf)> = Vec::new();
    for dir in recent_date_dirs(sessions, limit_dirs) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
                continue;
            }
            let mtime = e.metadata().and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
            files.push((mtime, e.path()));
        }
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files
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
    fn freshest_snapshot_timestamp_wins_over_older_snapshot() {
        let root = scratch();
        // 오래된 날짜 디렉터리 + 오래된 스냅샷 timestamp.
        write_rollout(
            &root,
            ("2026", "07", "10"),
            "rollout-old.jsonl",
            &event(
                "2026-07-10T10:00:00.000Z",
                r#"{"primary":{"used_percent":99.0,"window_minutes":10080,"resets_at":1784000000},"secondary":null}"#,
            ),
        );
        // 최신 날짜 디렉터리 + 최신 스냅샷 timestamp: 이걸 취해야 한다.
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

    /// 시나리오 (a) 동시 세션: mtime이 가장 최신인 파일이라도, 그 파일의
    /// 마지막 유효(non-null) 스냅샷이 다른 파일의 스냅샷보다 오래됐다면
    /// 더 신선한 스냅샷을 가진 쪽이 이겨야 한다.
    #[test]
    fn latest_mtime_file_loses_to_older_file_with_fresher_snapshot() {
        let root = scratch();
        // 먼저 쓰는 파일(= mtime 더 오래됨)이지만 스냅샷 timestamp는 더 최신.
        write_rollout(
            &root,
            ("2026", "07", "17"),
            "rollout-a-fresher-snapshot.jsonl",
            &event(
                "2026-07-17T12:00:00.000Z",
                r#"{"primary":{"used_percent":42.0,"window_minutes":10080,"resets_at":1784786662},"secondary":null}"#,
            ),
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        // 나중에 쓰는 파일(= mtime 더 최신)이지만, 파일 끝에는 동시 세션에서
        // append된 null rate_limits 이벤트만 있어 마지막 유효 스냅샷은 더 오래됨.
        let mut body = String::new();
        body.push_str(&event(
            "2026-07-17T09:00:00.000Z",
            r#"{"primary":{"used_percent":5.0,"window_minutes":10080,"resets_at":1784000000},"secondary":null}"#,
        ));
        body.push('\n');
        body.push_str(&event("2026-07-17T12:30:00.000Z", "null"));
        body.push('\n');
        write_rollout(
            &root,
            ("2026", "07", "17"),
            "rollout-b-newer-mtime-stale-snapshot.jsonl",
            &body,
        );

        let usage = load(&root).unwrap();
        // mtime은 b가 더 최신이지만, 유효 스냅샷 timestamp는 a가 더 최신이므로 a가 이겨야 한다.
        assert_eq!(usage.windows[0].used_percent, 42.0);
    }

    /// 시나리오 (b) 장기 세션: rollout 파일은 세션 "시작" 날짜 디렉터리에
    /// 계속 append되므로, 오래된 날짜 디렉터리의 파일이 실제로는 가장 신선한
    /// 스냅샷을 담고 있을 수 있다. 날짜 디렉터리 우선순위로 최신 날짜를
    /// 먼저 골랐다면 이 파일을 보지 못했을 것이다.
    #[test]
    fn older_date_dir_with_freshest_snapshot_wins() {
        let root = scratch();
        // 최신 날짜 디렉터리: 스냅샷 timestamp는 더 오래됨.
        write_rollout(
            &root,
            ("2026", "07", "17"),
            "rollout-newdir.jsonl",
            &event(
                "2026-07-17T08:00:00.000Z",
                r#"{"primary":{"used_percent":5.0,"window_minutes":10080,"resets_at":1784000000},"secondary":null}"#,
            ),
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        // 오래된 날짜 디렉터리(장기 세션 시작일): 파일은 나중에 쓰였고(mtime
        // 더 최신) 스냅샷 timestamp도 더 최신 — 이게 이겨야 한다.
        write_rollout(
            &root,
            ("2026", "07", "05"),
            "rollout-olddir-longrunning.jsonl",
            &event(
                "2026-07-17T13:00:00.000Z",
                r#"{"primary":{"used_percent":77.0,"window_minutes":10080,"resets_at":1784900000},"secondary":null}"#,
            ),
        );
        let usage = load(&root).unwrap();
        assert_eq!(usage.windows[0].used_percent, 77.0);
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
