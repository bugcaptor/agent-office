// src-tauri/src/persistence/awards_store.rs
//
// 이 달의 우수사원 시상 기록 영속화. 루트 `<app_data>/awards/` 아래에
// 문서 파일 `awards.json` 하나와, 수상자 초상 스냅샷 `portraits/<YYYY-MM>.png`를 둔다.
//
// 쓰기는 전부 `profile_store.rs`/`png_store.rs`와 같은 임시파일+rename 원자적
// 쓰기다 — 크래시가 반쪽짜리 awards.json을 남길 수 없다.
//
// 정책 두 가지가 이 파일의 성격을 결정한다.
// 1. **write-once**: 한 번 확정된 달은 다시 계산하지 않는다. `finalize`는
//    upsert가 아니라 upsert-if-absent다(같은 month가 있으면 무시하고 현재 파일 반환).
// 2. **버전 보존**: `version`이 지원 범위보다 크면 로드를 거부한다. 미래 버전 앱이
//    쓴 파일을 구버전 앱이 덮어써 날리는 사고를 막는다(save 경로는 반드시 load를 지난다).

use std::fs;
use std::path::PathBuf;

use base64::Engine;

use crate::types::{AwardRecord, AwardSpeech, AwardsFile};

/// 이 빌드가 읽을 수 있는 최대 스키마 버전. TS `AWARDS_SCHEMA_VERSION` 미러.
pub const AWARDS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug)]
pub enum AwardsStoreError {
    /// month가 "YYYY-MM"(01~12) 형식이 아님 — 파일명으로 쓰이므로 경로 조작 방지 겸용.
    InvalidMonth,
    /// 파일의 `version`이 이 빌드가 아는 범위를 넘음. 파일은 건드리지 않는다.
    UnsupportedVersion(u32),
    /// awards.json 파싱 실패(손상). 조용히 빈 상태로 덮어쓰지 않는다.
    Corrupt(String),
    /// 소감을 붙일 달의 레코드가 없음.
    UnknownMonth,
    /// 파일 시스템 오류.
    Io(String),
}

impl std::fmt::Display for AwardsStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AwardsStoreError::InvalidMonth => write!(f, "invalid month (expected YYYY-MM)"),
            AwardsStoreError::UnsupportedVersion(v) => {
                write!(f, "unsupported awards schema version: {v}")
            }
            AwardsStoreError::Corrupt(e) => write!(f, "corrupt awards file: {e}"),
            AwardsStoreError::UnknownMonth => write!(f, "no award record for that month"),
            AwardsStoreError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for AwardsStoreError {}

/// `<root>/awards.json` + `<root>/portraits/<month>.png`를 관리한다.
/// `root`는 주입(테스트는 tempdir).
pub struct AwardsStore {
    root: PathBuf,
}

impl AwardsStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// `month`를 파일명 요소로 쓰기 전 형식 검증. `^\d{4}-(0[1-9]|1[0-2])$`만
    /// 허용하므로 구분자·`..`가 낄 자리가 없다(png_store의 `validate_id`와 같은 역할).
    fn validate_month(month: &str) -> Result<(), AwardsStoreError> {
        let b = month.as_bytes();
        if b.len() != 7 || b[4] != b'-' {
            return Err(AwardsStoreError::InvalidMonth);
        }
        if !b[0..4].iter().all(|c| c.is_ascii_digit()) {
            return Err(AwardsStoreError::InvalidMonth);
        }
        let valid_mm = matches!(
            (b[5], b[6]),
            (b'0', b'1'..=b'9') | (b'1', b'0'..=b'2')
        );
        if !valid_mm {
            return Err(AwardsStoreError::InvalidMonth);
        }
        Ok(())
    }

    fn file(&self) -> PathBuf {
        self.root.join("awards.json")
    }

    fn portrait_path(&self, month: &str) -> PathBuf {
        self.root.join("portraits").join(format!("{month}.png"))
    }

    /// 시상 파일 전체. 파일이 없으면 빈 문서(version 1)를 돌려준다.
    /// 파싱 실패는 Err — 손상된 파일을 빈 상태로 덮어쓰지 않기 위해서다.
    /// 미래 버전도 Err이며 이때도 파일은 그대로 남는다.
    pub fn load(&self) -> Result<AwardsFile, AwardsStoreError> {
        let bytes = match fs::read(self.file()) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(AwardsFile {
                    version: AWARDS_SCHEMA_VERSION,
                    awards: Vec::new(),
                })
            }
            Err(e) => return Err(AwardsStoreError::Io(e.to_string())),
        };
        let parsed: AwardsFile = serde_json::from_slice(&bytes)
            .map_err(|e| AwardsStoreError::Corrupt(e.to_string()))?;
        if parsed.version > AWARDS_SCHEMA_VERSION {
            return Err(AwardsStoreError::UnsupportedVersion(parsed.version));
        }
        Ok(parsed)
    }

    /// 시상 파일을 원자적으로 저장한다(profile_store와 동일: 같은 디렉터리에
    /// 임시파일 -> rename).
    fn save(&self, file: &AwardsFile) -> Result<(), AwardsStoreError> {
        fs::create_dir_all(&self.root).map_err(|e| AwardsStoreError::Io(e.to_string()))?;
        let bytes =
            serde_json::to_vec_pretty(file).map_err(|e| AwardsStoreError::Io(e.to_string()))?;
        let final_path = self.file();
        let tmp = self
            .root
            .join(format!("awards.json.tmp-{}", uuid::Uuid::new_v4()));
        fs::write(&tmp, &bytes).map_err(|e| AwardsStoreError::Io(e.to_string()))?;
        if let Err(e) = fs::rename(&tmp, &final_path) {
            let _ = fs::remove_file(&tmp); // 실패해도 임시파일을 흘리지 않는다
            return Err(AwardsStoreError::Io(e.to_string()));
        }
        Ok(())
    }

    /// 초상 원본을 `portraits/<month>.png`로 복사한다. 성공 시 true.
    /// 원본 부재·복사 실패는 치명 오류가 아니라 false다(시상 자체는 성립한다).
    fn copy_portrait(&self, month: &str, source: &PathBuf) -> bool {
        if !source.is_file() {
            return false;
        }
        let dir = self.root.join("portraits");
        if fs::create_dir_all(&dir).is_err() {
            return false;
        }
        let tmp = dir.join(format!("{month}.png.tmp-{}", uuid::Uuid::new_v4()));
        if fs::copy(source, &tmp).is_err() {
            let _ = fs::remove_file(&tmp);
            return false;
        }
        if fs::rename(&tmp, self.portrait_path(month)).is_err() {
            let _ = fs::remove_file(&tmp);
            return false;
        }
        true
    }

    /// 한 달치 시상을 확정한다(**upsert-if-absent**). 같은 `month` 레코드가 이미
    /// 있으면 아무것도 하지 않고 현재 파일을 그대로 돌려준다 — write-once 정책이라
    /// 재계산 결과가 과거 시상을 덮지 못한다.
    ///
    /// `portrait_source`가 있고 실제 파일이 존재하면 확정 시점 스냅샷으로
    /// 복사한다. 복사에 실패하면(원본 부재 포함) 저장되는 레코드의
    /// `winner.has_portrait`를 false로 낮춰 파일과 화면이 어긋나지 않게 한다.
    pub fn finalize(
        &self,
        record: AwardRecord,
        portrait_source: Option<PathBuf>,
    ) -> Result<AwardsFile, AwardsStoreError> {
        Self::validate_month(&record.month)?;
        let mut file = self.load()?;
        if file.awards.iter().any(|a| a.month == record.month) {
            return Ok(file);
        }

        let mut record = record;
        let copied = match &portrait_source {
            Some(src) => self.copy_portrait(&record.month, src),
            None => false,
        };
        if let Some(winner) = record.winner.as_mut() {
            // 스냅샷이 실제로 놓인 경우에만 has_portrait를 유지한다.
            winner.has_portrait = winner.has_portrait && copied;
        }

        file.awards.push(record);
        file.awards.sort_by(|a, b| a.month.cmp(&b.month));
        self.save(&file)?;
        Ok(file)
    }

    /// 해당 월 레코드의 `speeches`에 소감 한 편을 붙인다. 이전 소감은 보존한다
    /// (마지막 원소가 대표 소감). 레코드가 없으면 Err.
    pub fn append_speech(
        &self,
        month: &str,
        speech: AwardSpeech,
    ) -> Result<AwardsFile, AwardsStoreError> {
        Self::validate_month(month)?;
        let mut file = self.load()?;
        let record = file
            .awards
            .iter_mut()
            .find(|a| a.month == month)
            .ok_or(AwardsStoreError::UnknownMonth)?;
        record.speeches.push(speech);
        self.save(&file)?;
        Ok(file)
    }

    /// 확정 시점 초상 스냅샷을 base64로 읽는다. 없으면 `Ok(None)`.
    /// 인코딩 방식은 `png_store::PngStore::load`와 같다(표준 base64, 데이터 URL
    /// 헤더 없음) — 렌더러가 두 경로를 같은 코드로 소비한다.
    pub fn load_portrait(&self, month: &str) -> Result<Option<String>, AwardsStoreError> {
        Self::validate_month(month)?;
        match fs::read(self.portrait_path(month)) {
            Ok(bytes) => Ok(Some(
                base64::engine::general_purpose::STANDARD.encode(bytes),
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(AwardsStoreError::Io(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AwardStanding, AwardStats, AwardWinner};

    fn scratch_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-awards-store-test-{}",
            uuid::Uuid::new_v4()
        ))
    }

    fn stats() -> AwardStats {
        AwardStats {
            worked_ms: 7_200_000,
            turns: 42,
            tool_events: 130,
            active_days: 11,
            tokens_in: 1_000,
            tokens_out: 2_000,
            cost_usd: 1.25,
        }
    }

    fn record(month: &str, has_portrait: bool) -> AwardRecord {
        AwardRecord {
            month: month.into(),
            decided_at: 1_700_000_000_000,
            rules_version: 1,
            winner: Some(AwardWinner {
                agent_id: "a1".into(),
                name: "김코드".into(),
                role: "백엔드".into(),
                archetype: Some("engineer".into()),
                has_portrait,
                stats: stats(),
            }),
            leaderboard: vec![AwardStanding {
                agent_id: "a1".into(),
                name: "김코드".into(),
                worked_ms: 7_200_000,
                turns: 42,
                active_days: 11,
                bot_worked_ms: None,
            }],
            speeches: Vec::new(),
        }
    }

    fn speech(at: u64, text: &str) -> AwardSpeech {
        AwardSpeech {
            at,
            provider: "claude".into(),
            text: text.into(),
        }
    }

    /// 최소 유효 PNG(매직 바이트만 맞으면 복사·인코딩 경로 검증엔 충분하다).
    fn tiny_png_bytes() -> Vec<u8> {
        let mut v = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend_from_slice(b"rest-of-fake-png-body");
        v
    }

    #[test]
    fn load_on_missing_file_returns_empty_v1() {
        let store = AwardsStore::new(scratch_dir());
        let file = store.load().expect("missing file is not an error");
        assert_eq!(file.version, 1);
        assert!(file.awards.is_empty());
    }

    #[test]
    fn finalize_then_load_roundtrips() {
        let dir = scratch_dir();
        let store = AwardsStore::new(dir.clone());
        let rec = record("2026-03", false);

        let saved = store.finalize(rec.clone(), None).expect("finalize ok");
        assert_eq!(saved.awards, vec![rec.clone()]);
        assert_eq!(store.load().unwrap().awards, vec![rec]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn awards_are_sorted_by_month_ascending() {
        let dir = scratch_dir();
        let store = AwardsStore::new(dir.clone());
        store.finalize(record("2026-03", false), None).unwrap();
        store.finalize(record("2025-12", false), None).unwrap();
        let file = store.finalize(record("2026-01", false), None).unwrap();

        let months: Vec<&str> = file.awards.iter().map(|a| a.month.as_str()).collect();
        assert_eq!(months, vec!["2025-12", "2026-01", "2026-03"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_malformed_month_keys() {
        let dir = scratch_dir();
        let store = AwardsStore::new(dir.clone());
        for bad in ["2026-13", "2026-00", "../x", "2026-1", "", "2026/03", "26-03", "2026-3a"] {
            assert!(
                matches!(
                    store.finalize(record(bad, false), None),
                    Err(AwardsStoreError::InvalidMonth)
                ),
                "finalize must reject month {bad:?}"
            );
            assert!(
                matches!(
                    store.append_speech(bad, speech(1, "x")),
                    Err(AwardsStoreError::InvalidMonth)
                ),
                "append_speech must reject month {bad:?}"
            );
            assert!(
                matches!(store.load_portrait(bad), Err(AwardsStoreError::InvalidMonth)),
                "load_portrait must reject month {bad:?}"
            );
        }
        // 어떤 파일도 만들어지지 않아야 한다.
        assert!(!dir.join("awards.json").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn future_version_is_refused_and_the_file_is_left_untouched() {
        let dir = scratch_dir();
        fs::create_dir_all(&dir).unwrap();
        let store = AwardsStore::new(dir.clone());
        let original = r#"{"version":2,"awards":[],"futureKey":"keep me"}"#;
        fs::write(dir.join("awards.json"), original).unwrap();

        assert!(matches!(
            store.load(),
            Err(AwardsStoreError::UnsupportedVersion(2))
        ));
        // finalize/append_speech도 load를 지나므로 덮어쓰기가 일어나선 안 된다.
        assert!(store.finalize(record("2026-03", false), None).is_err());
        assert!(store.append_speech("2026-03", speech(1, "x")).is_err());
        assert_eq!(
            fs::read_to_string(dir.join("awards.json")).unwrap(),
            original
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_errors_instead_of_being_overwritten() {
        let dir = scratch_dir();
        fs::create_dir_all(&dir).unwrap();
        let store = AwardsStore::new(dir.clone());
        fs::write(dir.join("awards.json"), "{ not json").unwrap();

        assert!(matches!(store.load(), Err(AwardsStoreError::Corrupt(_))));
        assert!(store.finalize(record("2026-03", false), None).is_err());
        assert_eq!(fs::read_to_string(dir.join("awards.json")).unwrap(), "{ not json");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn finalize_is_upsert_if_absent_and_keeps_the_first_record() {
        let dir = scratch_dir();
        let store = AwardsStore::new(dir.clone());
        let first = record("2026-03", false);
        store.finalize(first.clone(), None).unwrap();

        let mut second = record("2026-03", false);
        second.decided_at = 9_999_999_999_999;
        second.winner.as_mut().unwrap().name = "다른사람".into();
        let file = store.finalize(second, None).expect("두 번째 확정도 성공은 한다");

        assert_eq!(file.awards, vec![first.clone()], "첫 확정이 그대로 남는다");
        assert_eq!(store.load().unwrap().awards, vec![first]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_speech_pushes_and_preserves_previous_ones() {
        let dir = scratch_dir();
        let store = AwardsStore::new(dir.clone());
        store.finalize(record("2026-03", false), None).unwrap();

        store.append_speech("2026-03", speech(1, "첫 소감")).unwrap();
        let file = store.append_speech("2026-03", speech(2, "다시 쓴 소감")).unwrap();

        assert_eq!(
            file.awards[0].speeches,
            vec![speech(1, "첫 소감"), speech(2, "다시 쓴 소감")]
        );
        assert_eq!(store.load().unwrap().awards[0].speeches.len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_speech_on_unknown_month_errors() {
        let dir = scratch_dir();
        let store = AwardsStore::new(dir.clone());
        store.finalize(record("2026-03", false), None).unwrap();

        assert!(matches!(
            store.append_speech("2026-04", speech(1, "x")),
            Err(AwardsStoreError::UnknownMonth)
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_portrait_source_downgrades_has_portrait_to_false() {
        let dir = scratch_dir();
        let store = AwardsStore::new(dir.clone());
        let source = dir.join("nowhere").join("ghost.png");

        let file = store
            .finalize(record("2026-03", true), Some(source))
            .expect("초상 복사 실패는 치명 오류가 아니다");

        assert!(!file.awards[0].winner.as_ref().unwrap().has_portrait);
        assert_eq!(store.load_portrait("2026-03").unwrap(), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copied_portrait_is_readable_as_base64() {
        let dir = scratch_dir();
        let src_dir = scratch_dir();
        fs::create_dir_all(&src_dir).unwrap();
        let source = src_dir.join("a1.png");
        let png = tiny_png_bytes();
        fs::write(&source, &png).unwrap();
        let store = AwardsStore::new(dir.clone());

        let file = store
            .finalize(record("2026-03", true), Some(source))
            .expect("finalize ok");

        assert!(file.awards[0].winner.as_ref().unwrap().has_portrait);
        let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
        assert_eq!(store.load_portrait("2026-03").unwrap(), Some(encoded));
        // png_store와 같은 "헤더 없는 표준 base64"여야 한다.
        assert!(!store.load_portrait("2026-03").unwrap().unwrap().starts_with("data:"));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&src_dir);
    }

    #[test]
    fn portrait_is_not_copied_for_a_month_that_was_already_finalized() {
        let dir = scratch_dir();
        let src_dir = scratch_dir();
        fs::create_dir_all(&src_dir).unwrap();
        let source = src_dir.join("a1.png");
        fs::write(&source, tiny_png_bytes()).unwrap();
        let store = AwardsStore::new(dir.clone());

        // 초상 없이 먼저 확정된 달은 두 번째 호출로 스냅샷이 생기지 않는다.
        store.finalize(record("2026-03", false), None).unwrap();
        store.finalize(record("2026-03", true), Some(source)).unwrap();

        assert_eq!(store.load_portrait("2026-03").unwrap(), None);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&src_dir);
    }

    #[test]
    fn save_leaves_no_temp_file_behind() {
        let dir = scratch_dir();
        let store = AwardsStore::new(dir.clone());
        store.finalize(record("2026-03", false), None).unwrap();

        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(names.iter().any(|n| n == "awards.json"), "{names:?}");
        assert!(!names.iter().any(|n| n.contains(".tmp")), "{names:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn winner_may_be_null_for_a_month_without_a_qualified_candidate() {
        let dir = scratch_dir();
        let store = AwardsStore::new(dir.clone());
        let mut rec = record("2026-03", false);
        rec.winner = None;
        rec.leaderboard = Vec::new();

        store.finalize(rec.clone(), None).unwrap();

        assert_eq!(store.load().unwrap().awards, vec![rec]);
        // winner 키는 null로라도 항상 나가야 한다(TS가 옵셔널이 아닌 `| null`로 받는다).
        let raw = fs::read_to_string(dir.join("awards.json")).unwrap();
        assert!(raw.contains("\"winner\": null"), "{raw}");
        let _ = fs::remove_dir_all(&dir);
    }
}
