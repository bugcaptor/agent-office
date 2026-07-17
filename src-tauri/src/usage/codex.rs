// src-tauri/src/usage/codex.rs
//
// Codex CLI 사용량: `<codex_root>/sessions/YYYY/MM/DD/rollout-*.jsonl`의
// `token_count` 이벤트에 append된 `rate_limits` 스냅샷을 읽는다
// (docs/usage-limits-design.md §2).
//
// 스캔 전략: `sessions/YYYY/MM/DD` 아래 **모든** 날짜 디렉터리에서 rollout
// 파일을 모아 파일 mtime 내림차순으로(날짜 디렉터리 경계 없이 전역) 정렬한
// 뒤, 각 파일에서 마지막 non-null `rate_limits` 라인(스냅샷)을 파싱해 그
// 스냅샷의 `fetched_at_ms`(이벤트 timestamp)가 가장 큰 후보를 취한다.
// mtime이 아니라 스냅샷 자체의 timestamp로 승자를 가리는 이유:
//   (a) 동시 세션 — mtime이 가장 최신인 파일이라도 파일 끝부분이 null
//       rate_limits 이벤트로 끝나면, 그 파일의 마지막 유효 스냅샷은 다른
//       파일의 스냅샷보다 오래될 수 있다.
//   (b) 장기 세션 — rollout 파일은 세션 "시작" 날짜 디렉터리에 계속
//       append되므로, 오래된 날짜 디렉터리의 파일이 실제로는 가장 신선한
//       스냅샷을 담고 있을 수 있다. 예전에는 여기서 최근 날짜 디렉터리
//       7개로 컷오프했는데, 그러면 장기 세션 도중 새 날짜 디렉터리가 7개
//       이상 생기는 순간 mtime이 가장 최신인 파일(=가장 신선한 스냅샷을
//       담은 파일)이 스캔에서 통째로 배제되는 버그가 있었다 — 그래서
//       날짜 디렉터리 컷오프는 없앴다.
// 조기 종료: 파일 내 스냅샷 timestamp는 그 파일의 mtime보다 늦을 수 없으므로,
// 전역 mtime 내림차순 순회 중 현재 최선 후보의 fetched_at_ms가 다음 파일의
// mtime(epoch ms) 이상이 되는 순간 나머지(mtime이 더 낮은) 파일들은 이를
// 넘어설 수 없어 스캔을 중단한다. `window_minutes`로 윈도 종류를
// 판별하고(300=5시간, 10080=주간), `resets_at`은 유닉스 초라 ms로 변환한다.
//
// 스캔 비용 상한: 날짜 디렉터리 컷오프 대신 `MAX_PARSED_FILES`로 parse_file
// 호출 횟수 자체를 제한한다. 대상 파일 목록 수집·mtime 정렬은 전체 날짜
// 디렉터리를 대상으로 하고(결정적 정렬이라 비용은 디렉터리 순회 정도),
// 실제 파일 내용을 읽어 파싱하는 비용이 큰 작업만 상한을 둔다.
//
// 재폴링 캐시: load는 60초 폴링마다 다시 불리는데, rollout 파일은
// append-only라 (mtime, len)이 그대로면 내용도 그대로다. 그래서 파일별
// parse_file 결과를 (mtime, len) 키로 캐시해, 변경 없는 파일은 tail 스캔
// 없이 이전 결과(None 포함)를 재사용한다. 이게 없으면 유효 스냅샷이 하나도
// 없는 아카이브(API 키 사용자, rate_limits 도입 전 rollout)에서 매 폴링마다
// 최대 MAX_PARSED_FILES × MAX_TAIL_SCAN_BYTES(=512MB)를 다시 읽게 된다.
//
// 파일 내부 스캔(parse_file): 장기 세션 rollout은 수백 MB가 될 수 있고
// 60초 폴링마다 최대 MAX_PARSED_FILES개를 매번 `read_to_string`으로 통째로
// 읽으면 I/O·메모리 낭비가 크다. 그래서 파일 끝에서부터 TAIL_CHUNK_BYTES
// 단위로 역방향 청크를 읽어, 완성된 라인을 뒤에서부터(EOF 쪽부터) 검사하고
// 첫 non-null 스냅샷을 찾으면 즉시 반환한다 — 상주 메모리는 청크 1개 +
// 청크 경계에 걸친 미완결 라인(carry) 수준으로 유지된다. 청크 경계에 걸린
// 라인은 다음(더 앞쪽) 청크와 이어붙여 처리하므로 온전한 라인만 파싱 대상이
// 된다. 총 스캔은 파일당 MAX_TAIL_SCAN_BYTES로 상한을 둔다 — rate_limits
// 스냅샷은 token_count 이벤트마다 기록되므로, 유효한 파일이라면 스냅샷은
// 항상 꼬리 근처에 있다는 것이 전제(그 이상 뒤져도 못 찾으면 이 파일은
// 포기하고 다음 파일로 넘어간다).

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::{parse_iso8601_ms, Provider, ProviderUsage, UsageWindow, UsageWindowKind};

/// 폴링 1회당 parse_file 호출 횟수 상한(스캔 비용 상한). 캐시 적중은 여기에
/// 세지 않는다 — 실제 tail 스캔이 일어나는 파일만 예산을 쓴다. 이 개수만큼
/// 파싱하고도 더 나은 후보를 못 찾으면(또는 아예 못 찾으면) 탐색을 중단한다.
const MAX_PARSED_FILES: usize = 64;

/// 파일 끝에서부터 역방향으로 읽는 청크 크기.
const TAIL_CHUNK_BYTES: u64 = 64 * 1024;

/// 파일당 꼬리에서부터 스캔하는 총 상한(바이트). rate_limits 스냅샷은
/// token_count 이벤트마다 기록되므로 유효한 파일이라면 꼬리 근처에 있다는
/// 전제 — 이 상한을 넘도록 non-null 스냅샷을 못 찾으면 그 파일은 포기하고
/// (다음 파일로) 넘어간다.
const MAX_TAIL_SCAN_BYTES: u64 = 8 * 1024 * 1024;

/// 파일 1개의 캐시된 파싱 결과. (mtime, len)이 스캔 당시와 같으면
/// append-only rollout 특성상 내용도 같으므로 result를 재사용한다.
/// result는 "유효 스냅샷 없음"(None)도 캐시한다 — 그게 이 캐시의 핵심
/// 목적(스냅샷 없는 아카이브 재스캔 방지)이다.
struct CacheEntry {
    mtime: SystemTime,
    len: u64,
    result: Option<ProviderUsage>,
}

/// 프로세스 전역 parse_file 결과 캐시(모듈 상단 §재폴링 캐시). 삭제된
/// 파일의 엔트리는 남지만 항목당 수백 바이트 수준이라 정리하지 않는다.
fn parse_cache() -> &'static Mutex<HashMap<PathBuf, CacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `<codex_root>/sessions`에서 가장 최근(스냅샷 timestamp 기준) non-null
/// rate_limits 스냅샷을 찾는다. 없으면 None.
pub fn load(codex_root: &Path) -> Option<ProviderUsage> {
    let sessions = codex_root.join("sessions");
    let files = all_rollout_files_by_mtime_desc(&sessions);
    let mut cache = parse_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut best: Option<ProviderUsage> = None;
    let mut parsed_count = 0usize;
    for (mtime, len, file) in files {
        // 조기 종료: 남은 파일들은 mtime이 이 파일 이하이므로, 현재 최선
        // 후보의 스냅샷이 이 파일의 mtime보다도 이미 최신이면 더 볼 필요 없다.
        if let Some(best_usage) = &best {
            if best_usage.fetched_at_ms >= epoch_ms(mtime) {
                break;
            }
        }
        let cached = cache
            .get(&file)
            .filter(|e| e.mtime == mtime && e.len == len)
            .map(|e| e.result.clone());
        let usage = match cached {
            Some(result) => result,
            None => {
                if parsed_count >= MAX_PARSED_FILES {
                    break;
                }
                parsed_count += 1;
                let result = parse_file(&file);
                cache.insert(
                    file,
                    CacheEntry {
                        mtime,
                        len,
                        result: result.clone(),
                    },
                );
                result
            }
        };
        if let Some(usage) = usage {
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

/// `sessions/YYYY/MM/DD` 구조의 모든 날짜 디렉터리. 순서는 무관하다 —
/// 파일 목록은 이후 mtime 기준 전역 정렬되므로, 여기서는 날짜 파싱을
/// 디렉터리 구조 검증(유효한 캘린더 날짜인지)에만 쓴다.
fn all_date_dirs(sessions: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
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
                if chrono::NaiveDate::from_ymd_opt(year, month, day).is_some() {
                    dirs.push(d.path());
                }
            }
        }
    }
    dirs
}

/// 디렉터리 엔트리 이름을 정수로 파싱(YYYY/MM/DD 컴포넌트용).
fn parse_component<T: std::str::FromStr>(entry: &std::fs::DirEntry) -> Option<T> {
    entry.file_name().to_str()?.parse::<T>().ok()
}

/// 모든 날짜 디렉터리를 통틀어 모든 `rollout-*.jsonl`을 (mtime, len, 경로)로
/// 모아 파일 mtime 내림차순으로(날짜 디렉터리 경계 없이 전역) 정렬한다.
/// len은 캐시 키용 — mtime 해상도가 거친 파일시스템에서도 append는 len을
/// 바꾸므로 (mtime, len) 쌍이면 변경 감지에 충분하다.
fn all_rollout_files_by_mtime_desc(sessions: &Path) -> Vec<(SystemTime, u64, PathBuf)> {
    let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for dir in all_date_dirs(sessions) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
                continue;
            }
            let meta = e.metadata().ok();
            let mtime = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .unwrap_or(UNIX_EPOCH);
            let len = meta.map(|m| m.len()).unwrap_or(0);
            files.push((mtime, len, e.path()));
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files
}

/// 파일 끝에서부터 TAIL_CHUNK_BYTES 단위로 역방향 청크를 읽어, 완성된
/// 라인을 뒤에서부터 검사해 마지막 non-null rate_limits 라인을 찾는다.
/// 파일 전체를 메모리에 올리지 않는다(모듈 상단 스캔 전략 주석 참고).
/// 청크 경계에 걸린 라인은 `carry`에 담아 다음(더 앞쪽) 청크와 이어붙인
/// 뒤에 처리한다. `MAX_TAIL_SCAN_BYTES`를 넘도록 못 찾으면 None.
fn parse_file(path: &Path) -> Option<ProviderUsage> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();

    let mut pos = len;
    let mut scanned: u64 = 0;
    // 왼쪽 경계(더 앞쪽 청크로 이어질 수 있는지)가 아직 확정되지 않은 라인
    // 조각. 다음 청크를 읽으면 그 뒤에 붙여(파일상 순서: [새 청크][carry])
    // 완결시킨다.
    let mut carry: Vec<u8> = Vec::new();

    while pos > 0 && scanned < MAX_TAIL_SCAN_BYTES {
        let chunk_len = TAIL_CHUNK_BYTES.min(pos);
        let start = pos - chunk_len;
        file.seek(SeekFrom::Start(start)).ok()?;
        let mut combined = vec![0u8; chunk_len as usize];
        file.read_exact(&mut combined).ok()?;
        scanned += chunk_len;
        pos = start;
        combined.extend_from_slice(&carry);
        carry.clear();

        let mut parts: Vec<&[u8]> = combined.split(|&b| b == b'\n').collect();
        // pos(=start, 이 청크의 파일상 시작 오프셋) > 0이면 parts[0]은 더
        // 앞쪽(왼쪽)으로 이어질 수 있는 미완결 조각이므로 이번 라운드에서는
        // 건너뛰고 carry로 넘긴다. pos == 0(파일 시작에 도달)이면 parts[0]도
        // 이제 완결된 라인이다.
        let leftover = if pos > 0 { Some(parts.remove(0)) } else { None };

        for line in parts.iter().rev() {
            if let Some(usage) = parse_line(line) {
                return Some(usage);
            }
        }

        match leftover {
            Some(l) => carry = l.to_vec(),
            None => return None, // 파일 시작까지 다 훑었는데 못 찾음.
        }
    }
    None
}

/// `parse_file`이 뒤에서부터 검사하는 라인 1개를 ProviderUsage로. non-null =
/// rate_limits 객체가 있고 primary/secondary 중 하나 이상이 non-null. UTF-8이
/// 아닌 라인(청크 경계가 멀티바이트 문자 중간을 자른 경우는 없다 — carry
/// 이어붙이기로 항상 `\n` 단위 완결 라인만 넘어오지만, 혹시 모를 손상 라인
/// 대비)은 조용히 스킵.
fn parse_line(line: &[u8]) -> Option<ProviderUsage> {
    let line = std::str::from_utf8(line).ok()?.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let rl = v.get("payload").and_then(|p| p.get("rate_limits"))?;
    if rl.is_null() {
        return None;
    }
    let primary = rl.get("primary").filter(|x| !x.is_null());
    let secondary = rl.get("secondary").filter(|x| !x.is_null());
    if primary.is_none() && secondary.is_none() {
        return None;
    }
    let fetched_at_ms = v
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_iso8601_ms)?;
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
        return None;
    }
    Some(ProviderUsage {
        provider: Provider::Codex,
        fetched_at_ms,
        plan_label,
        windows,
    })
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
        // Codex rate_limits 스냅샷에는 is_active 개념이 없다.
        is_active: None,
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
        // Codex 스냅샷에는 is_active 개념이 없다 — 항상 null.
        assert_eq!(w.is_active, None);
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

    /// 회귀: 예전에는 최근 7개 날짜 디렉터리로 컷오프해서, 더 새로운 날짜
    /// 디렉터리가 8개 이상 생기면 훨씬 오래된 날짜 디렉터리(장기 세션 시작일)의
    /// 파일이 스캔 대상에서 아예 빠졌다. 컷오프를 없앤 뒤에는 날짜 디렉터리
    /// 개수와 무관하게 mtime이 가장 최신인 파일(=가장 신선한 스냅샷)이 이겨야
    /// 한다.
    #[test]
    fn older_date_dir_wins_even_with_eight_or_more_newer_date_dirs() {
        let root = scratch();
        // 최신 쪽 날짜 디렉터리 9개(예전 RECENT_DAYS=7 컷오프라면 이 중
        // 상위 7개만 스캔 대상이었을 것) — 전부 오래된 스냅샷.
        for day in 9..=17 {
            write_rollout(
                &root,
                ("2026", "07", &format!("{day:02}")),
                "rollout-newdir.jsonl",
                &event(
                    "2026-07-17T08:00:00.000Z",
                    r#"{"primary":{"used_percent":5.0,"window_minutes":10080,"resets_at":1784000000},"secondary":null}"#,
                ),
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        // 훨씬 오래된 날짜 디렉터리(장기 세션 시작일): 컷오프가 있었다면
        // 날짜 상위 7개에 들지 못해 아예 스캔되지 않았을 것. 파일은 나중에
        // 쓰였고(mtime 최신) 스냅샷 timestamp도 최신 — 이게 이겨야 한다.
        write_rollout(
            &root,
            ("2026", "06", "01"),
            "rollout-olddir-longrunning.jsonl",
            &event(
                "2026-07-17T13:00:00.000Z",
                r#"{"primary":{"used_percent":88.0,"window_minutes":10080,"resets_at":1784900000},"secondary":null}"#,
            ),
        );
        let usage = load(&root).unwrap();
        assert_eq!(usage.windows[0].used_percent, 88.0);
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

    /// 재폴링 캐시(§모듈 상단): (mtime, len)이 그대로면 tail 스캔을 건너뛰고
    /// 이전 결과를 재사용한다. 내용을 같은 길이로 바꿔치기하고 mtime을
    /// 원래대로 되돌리면(= append-only 전제에서는 일어나지 않는 변조) 캐시가
    /// 적중해 예전 값이 그대로 나온다 — 재파싱이 생략됐다는 관찰 가능한 증거.
    #[test]
    fn unchanged_mtime_and_len_reuses_cached_result() {
        let root = scratch();
        let line_a = event(
            "2026-07-17T11:20:17.595Z",
            r#"{"primary":{"used_percent":11.0,"window_minutes":10080,"resets_at":1784786662},"secondary":null}"#,
        );
        write_rollout(&root, ("2026", "07", "17"), "rollout-a.jsonl", &line_a);
        assert_eq!(load(&root).unwrap().windows[0].used_percent, 11.0);

        let path = root
            .join("sessions/2026/07/17")
            .join("rollout-a.jsonl");
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        // used_percent만 11.0 → 22.0으로: 바이트 길이는 동일하다.
        let line_b = line_a.replace("11.0", "22.0");
        assert_eq!(line_a.len(), line_b.len());
        std::fs::write(&path, &line_b).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(mtime)
            .unwrap();

        // (mtime, len) 불변 → 캐시 적중 → 예전 값 11.0 유지.
        assert_eq!(load(&root).unwrap().windows[0].used_percent, 11.0);
    }

    /// 재폴링 캐시 무효화: append로 len이 바뀌면(mtime 해상도와 무관하게)
    /// 재파싱해 새 스냅샷을 집는다. "스냅샷 없음"(None) 결과도 캐시되지만
    /// 파일이 자라면 다시 스캔한다는 것을 함께 검증한다.
    #[test]
    fn append_invalidates_cache_even_for_cached_none() {
        let root = scratch();
        // 처음엔 유효 스냅샷 없음 → None(이 결과가 캐시된다).
        write_rollout(
            &root,
            ("2026", "07", "17"),
            "rollout-a.jsonl",
            &event("2026-07-17T10:00:00.000Z", r#"{"primary":null,"secondary":null}"#),
        );
        assert!(load(&root).is_none());

        // 유효 스냅샷 append → len 변경 → 캐시 미스 → 새로 파싱.
        let path = root
            .join("sessions/2026/07/17")
            .join("rollout-a.jsonl");
        let mut body = std::fs::read_to_string(&path).unwrap();
        body.push('\n');
        body.push_str(&event(
            "2026-07-17T11:20:17.595Z",
            r#"{"primary":{"used_percent":33.0,"window_minutes":10080,"resets_at":1784786662},"secondary":null}"#,
        ));
        std::fs::write(&path, &body).unwrap();

        assert_eq!(load(&root).unwrap().windows[0].used_percent, 33.0);
    }

    /// 청크 경계 이어붙이기(§모듈 상단 스캔 전략): 유효 스냅샷 라인보다
    /// 파일 끝(EOF) 쪽에 TAIL_CHUNK_BYTES(64KB)보다 긴 단일 패딩 라인을
    /// 둔다. 뒤에서부터 역방향으로 읽으면 이 패딩 라인 하나를 완결시키는
    /// 데만 최소 2번의 청크 읽기가 필요하다(carry로 이어붙임) — 그 과정이
    /// 깨지면 패딩 라인 파싱 실패 자체는 문제 없지만(non-JSON이라 스킵),
    /// 그 앞(더 왼쪽)의 유효 스냅샷 라인 경계가 잘못 잘려 못 찾거나 잘못된
    /// 값을 주게 된다. 패딩 뒤(EOF 쪽)에 이 라인을 두고, 유효 스냅샷은 그
    /// 앞(파일 시작 쪽)에 둔다 — 즉 스캔이 패딩을 다 넘어서야 도달한다.
    #[test]
    fn long_line_spanning_multiple_chunks_is_stitched_before_earlier_valid_snapshot() {
        let root = scratch();
        let valid_line = event(
            "2026-07-17T11:20:17.595Z",
            r#"{"primary":{"used_percent":33.0,"window_minutes":10080,"resets_at":1784786662},"secondary":null}"#,
        );
        // TAIL_CHUNK_BYTES(64KB)보다 확실히 긴 단일 라인(내부에 '\n' 없음) --
        // 유효하지 않은 JSON이라 그 자체는 스냅샷으로 채택되지 않는다.
        let padding_line = "x".repeat(70_000);
        let body = format!("{valid_line}\n{padding_line}\n");
        write_rollout(&root, ("2026", "07", "17"), "rollout-a.jsonl", &body);

        let usage = load(&root).unwrap();
        assert_eq!(usage.windows[0].used_percent, 33.0);
    }

    /// 스캔 상한(MAX_TAIL_SCAN_BYTES=8MB): 파일 끝에서부터 8MB를 넘는
    /// 위치(파일 시작부)에만 유효 스냅샷이 있고, 마지막 8MB 구간은 '\n'이
    /// 전혀 없는(따라서 한 줄도 완결되지 않는) 필러이면 스캔이 상한에서
    /// 멈추고 앞쪽의 유효 스냅샷에 끝내 도달하지 못해 None을 돌려줘야
    /// 한다.
    #[test]
    fn snapshot_beyond_max_tail_scan_bytes_yields_none() {
        let root = scratch();
        let valid_line = event(
            "2026-07-17T11:20:17.595Z",
            r#"{"primary":{"used_percent":77.0,"window_minutes":10080,"resets_at":1784786662},"secondary":null}"#,
        );
        // 8MB(MAX_TAIL_SCAN_BYTES)보다 넉넉히 큰, 개행이 전혀 없는 필러 --
        // EOF 쪽에서부터 스캔해도 8MB 상한 안에서는 단 한 줄도 완결되지
        // 않는다.
        let filler = "z".repeat(9 * 1024 * 1024);
        let body = format!("{valid_line}\n{filler}");
        write_rollout(&root, ("2026", "07", "17"), "rollout-a.jsonl", &body);

        assert!(load(&root).is_none());
    }
}
