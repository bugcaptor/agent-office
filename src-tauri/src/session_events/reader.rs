// src-tauri/src/session_events/reader.rs
//
// 세션 이벤트 시계열(`<app-data>/session-events/v1/YYYY-MM-DD.jsonl`)의 읽기
// 경로. `SessionEventStore`는 쓰기 전용 원칙을 유지하므로(선행 설계 §5.1)
// 읽기는 이 독립 함수로 분리한다. 분석 패널(docs/session-analytics-design.md
// §4.1)이 기간을 넘겨 원시 레코드를 받아가고, 집계는 렌더러가 한다.

use std::fs;
use std::path::Path;

use chrono::{DateTime, NaiveDate, Utc};

use super::types::SessionEventRecord;

/// `from_at..=to_at`(epoch ms) 범위의 세션 이벤트를 읽어 정렬해 돌려준다.
///
/// - 파일 파티션 키가 `at`의 UTC 날짜이므로, `from_at`의 UTC 날짜부터
///   `to_at`의 UTC 날짜까지 `YYYY-MM-DD.jsonl`을 순서대로 연다. 이 스캔
///   범위가 `at` 필터 범위를 완전히 덮는다.
/// - 없는 파일·열기 실패(I/O 오류)는 해당 파일만 건너뛴다.
/// - 빈 줄·파싱 불가 줄은 조용히 건너뛴다(부분 기록 내구성, 선행 설계 §7).
/// - `from_at <= at <= to_at`(양끝 포함)로 거른 뒤 `(at, runId, seq)`로 정렬.
/// - 반환은 항상 성공한다(오류는 결과 축소로만 나타난다, 설계 §6).
pub fn load_session_events(root: &Path, from_at: u64, to_at: u64) -> Vec<SessionEventRecord> {
    if from_at > to_at {
        return Vec::new();
    }
    let mut out = Vec::new();
    for date in utc_date_range(from_at, to_at) {
        let path = root.join(format!("{date}.jsonl"));
        // 없는 파일·읽기 실패는 스킵 — 반환은 절대 실패하지 않는다.
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // 파싱 불가(부분 write로 잘린 마지막 줄 등)는 조용히 스킵.
            if let Ok(record) = serde_json::from_str::<SessionEventRecord>(line) {
                if record.at >= from_at && record.at <= to_at {
                    out.push(record);
                }
            }
        }
    }
    out.sort_by(|a, b| {
        a.at.cmp(&b.at)
            .then_with(|| a.run_id.cmp(&b.run_id))
            .then_with(|| a.seq.cmp(&b.seq))
    });
    out
}

/// `from_at`의 UTC 날짜부터 `to_at`의 UTC 날짜까지 `YYYY-MM-DD` 문자열을
/// 오름차순으로 나열한다. 타임스탬프가 유효 범위를 벗어나면 빈 목록.
fn utc_date_range(from_at: u64, to_at: u64) -> Vec<String> {
    let (Some(mut cursor), Some(end)) = (utc_date(from_at), utc_date(to_at)) else {
        return Vec::new();
    };
    let mut dates = Vec::new();
    while cursor <= end {
        dates.push(cursor.format("%Y-%m-%d").to_string());
        match cursor.succ_opt() {
            Some(next) => cursor = next,
            None => break,
        }
    }
    dates
}

fn utc_date(at: u64) -> Option<NaiveDate> {
    let millis = i64::try_from(at).ok()?;
    Some(DateTime::<Utc>::from_timestamp_millis(millis)?.date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_events::types::{SessionEventKind, SessionEventRecord};
    use std::path::PathBuf;

    // 1_783_728_000_000 = 2026-07-11 00:00:00 UTC (store 테스트의 기준값).
    const D10_EVENING: u64 = 1_783_710_000_000; // 2026-07-10 19:00 UTC
    const D11_MIDNIGHT: u64 = 1_783_728_000_000; // 2026-07-11 00:00 UTC
    const D11_LATER: u64 = 1_783_760_000_000; // 2026-07-11 08:53 UTC
    const D12_MIDNIGHT: u64 = 1_783_814_400_000; // 2026-07-12 00:00 UTC

    fn scratch_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-session-events-reader-{}",
            uuid::Uuid::new_v4()
        ))
    }

    fn rec(at: u64, run: &str, seq: u64) -> SessionEventRecord {
        SessionEventRecord {
            schema_version: 1,
            run_id: run.into(),
            seq,
            at,
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: SessionEventKind::Tool,
            agent_name: None,
            agent_role: None,
            cwd: None,
            shell: None,
            state: None,
        }
    }

    /// `records`를 그대로(정렬/필터 없이) `date`.jsonl에 직렬화해 쓴다.
    fn write_day(root: &Path, date: &str, records: &[SessionEventRecord]) {
        fs::create_dir_all(root).unwrap();
        let mut body = String::new();
        for record in records {
            body.push_str(&serde_json::to_string(record).unwrap());
            body.push('\n');
        }
        fs::write(root.join(format!("{date}.jsonl")), body).unwrap();
    }

    #[test]
    fn missing_root_yields_empty() {
        let root = scratch_root();
        assert!(load_session_events(&root, D10_EVENING, D12_MIDNIGHT).is_empty());
    }

    #[test]
    fn scans_multiple_files_in_range_and_merges() {
        let root = scratch_root();
        write_day(&root, "2026-07-10", &[rec(D10_EVENING, "r", 1)]);
        write_day(&root, "2026-07-11", &[rec(D11_MIDNIGHT, "r", 2)]);
        write_day(&root, "2026-07-12", &[rec(D12_MIDNIGHT, "r", 3)]);

        let got = load_session_events(&root, D10_EVENING, D12_MIDNIGHT);

        assert_eq!(got.len(), 3);
        assert_eq!(got.iter().map(|r| r.at).collect::<Vec<_>>(), vec![
            D10_EVENING,
            D11_MIDNIGHT,
            D12_MIDNIGHT
        ]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn skips_a_missing_file_in_the_middle_of_the_range() {
        let root = scratch_root();
        write_day(&root, "2026-07-10", &[rec(D10_EVENING, "r", 1)]);
        // 2026-07-11 파일 없음.
        write_day(&root, "2026-07-12", &[rec(D12_MIDNIGHT, "r", 3)]);

        let got = load_session_events(&root, D10_EVENING, D12_MIDNIGHT);

        assert_eq!(got.len(), 2);
        assert_eq!(got[0].at, D10_EVENING);
        assert_eq!(got[1].at, D12_MIDNIGHT);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn skips_blank_and_corrupt_lines() {
        let root = scratch_root();
        fs::create_dir_all(&root).unwrap();
        let good = serde_json::to_string(&rec(D11_MIDNIGHT, "r", 1)).unwrap();
        let good2 = serde_json::to_string(&rec(D11_LATER, "r", 2)).unwrap();
        let body = format!("{good}\n\n   \nnot json at all\n{{\"partial\":\n{good2}\n");
        fs::write(root.join("2026-07-11.jsonl"), body).unwrap();

        let got = load_session_events(&root, D11_MIDNIGHT, D11_LATER);

        assert_eq!(got.len(), 2);
        assert_eq!(got[0].at, D11_MIDNIGHT);
        assert_eq!(got[1].at, D11_LATER);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn filters_at_range_inclusive_on_both_ends() {
        let root = scratch_root();
        // 같은 파일(07-11)에 경계 밖/경계/경계 안 레코드를 섞는다.
        write_day(&root, "2026-07-11", &[
            rec(D11_MIDNIGHT, "r", 1),     // == from (포함)
            rec(D11_MIDNIGHT + 10, "r", 2), // 안쪽
            rec(D11_LATER, "r", 3),        // == to (포함)
        ]);
        // from 이전, to 이후 레코드는 각각 07-10, 07-12에 두어 범위 밖임을 확인.
        write_day(&root, "2026-07-10", &[rec(D10_EVENING, "r", 9)]);
        write_day(&root, "2026-07-12", &[rec(D12_MIDNIGHT, "r", 9)]);

        // 스캔 범위는 07-10..07-12지만 at 필터가 D11_MIDNIGHT..=D11_LATER.
        let got = load_session_events(&root, D11_MIDNIGHT, D11_LATER);

        assert_eq!(got.len(), 3);
        assert_eq!(got.first().unwrap().at, D11_MIDNIGHT);
        assert_eq!(got.last().unwrap().at, D11_LATER);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sorts_by_at_then_run_id_then_seq() {
        let root = scratch_root();
        // 파일 안에서는 일부러 뒤섞어 둔다.
        write_day(&root, "2026-07-11", &[
            rec(D11_LATER, "r-a", 5),
            rec(D11_MIDNIGHT, "r-b", 1),
            rec(D11_MIDNIGHT, "r-a", 2),
            rec(D11_MIDNIGHT, "r-a", 1),
        ]);

        let got = load_session_events(&root, D11_MIDNIGHT, D11_LATER);

        let keys: Vec<_> = got
            .iter()
            .map(|r| (r.at, r.run_id.clone(), r.seq))
            .collect();
        assert_eq!(keys, vec![
            (D11_MIDNIGHT, "r-a".to_string(), 1),
            (D11_MIDNIGHT, "r-a".to_string(), 2),
            (D11_MIDNIGHT, "r-b".to_string(), 1),
            (D11_LATER, "r-a".to_string(), 5),
        ]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reversed_range_yields_empty() {
        let root = scratch_root();
        write_day(&root, "2026-07-11", &[rec(D11_MIDNIGHT, "r", 1)]);
        assert!(load_session_events(&root, D11_LATER, D11_MIDNIGHT).is_empty());
        let _ = fs::remove_dir_all(root);
    }
}
