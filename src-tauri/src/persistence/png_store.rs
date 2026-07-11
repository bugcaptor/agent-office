// src-tauri/src/persistence/png_store.rs
//
// 초상/스프라이트 공용 PNG 파일 저장소. `<dir>/<agentId>.png` 하나당 한 파일.
// 크기 상한은 인스턴스별 주입(portraits=2MiB, sprites=256KiB).
// `profile_store.rs`와 동일한 임시파일+rename 원자적 쓰기를 재사용한다.
// 이미지 바이트는 profiles.json에 넣지 않는다. 프런트와는 base64로
// 왕복한다: save는 base64를 받아 디코드 후 검증·저장, load는 파일을 읽어 base64로
// 인코드해 돌려준다.

use std::fs;
use std::path::PathBuf;

use base64::Engine;

/// 초상 PNG 상한(2 MiB). 정상 경로 프런트는 240×320 PNG(수십 KB).
pub const MAX_PORTRAIT_BYTES: usize = 2 * 1024 * 1024;
/// 커스텀 스프라이트 시트 PNG 상한(설계 C: 1 MiB, 1024×256 RGBA 대응). 정상 경로는
/// 4N×N PNG(수백 B~수십 KB).
pub const MAX_SPRITE_BYTES: usize = 1024 * 1024;

/// PNG 8바이트 시그니처.
const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

#[derive(Debug)]
pub enum PngStoreError {
    /// agentId가 저장된 프로필에 없음.
    UnknownAgent,
    /// agentId가 경로 요소로 안전하지 않음(구분자/`..`/빈 문자열).
    InvalidId,
    /// 디코딩 결과가 PNG 시그니처로 시작하지 않음.
    NotPng,
    /// 디코딩된 바이트가 상한 초과(인스턴스별 `max_bytes`).
    TooLarge(usize),
    /// base64 디코드 실패.
    BadBase64,
    /// 파일 시스템 오류.
    Io(String),
}

impl std::fmt::Display for PngStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PngStoreError::UnknownAgent => write!(f, "unknown agentId"),
            PngStoreError::InvalidId => write!(f, "invalid agentId (unsafe path element)"),
            PngStoreError::NotPng => write!(f, "payload is not a PNG"),
            PngStoreError::TooLarge(max_bytes) => write!(f, "png exceeds {max_bytes} bytes"),
            PngStoreError::BadBase64 => write!(f, "invalid base64"),
            PngStoreError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for PngStoreError {}

/// `<dir>/<agentId>.png` 파일들을 관리한다. `dir`은 주입(테스트는 tempdir).
/// `max_bytes`는 인스턴스별 상한 주입(portraits=2MiB, sprites=256KiB).
pub struct PngStore {
    dir: PathBuf,
    max_bytes: usize,
}

impl PngStore {
    pub fn new(dir: PathBuf, max_bytes: usize) -> Self {
        Self { dir, max_bytes }
    }

    /// `agent_id`를 경로 요소로 쓰기 전 안전성 검증(경로 조작 방지). 구분자/`..`/
    /// 빈 문자열을 거부한다.
    fn validate_id(agent_id: &str) -> Result<(), PngStoreError> {
        if agent_id.is_empty()
            || agent_id.contains('/')
            || agent_id.contains('\\')
            || agent_id.contains("..")
        {
            return Err(PngStoreError::InvalidId);
        }
        Ok(())
    }

    fn path_for(&self, agent_id: &str) -> PathBuf {
        self.dir.join(format!("{agent_id}.png"))
    }

    /// base64 PNG를 검증 후 원자적으로 저장한다. `known_ids`는 현재 저장된 프로필
    /// id 목록(존재 검증용).
    pub fn save(
        &self,
        agent_id: &str,
        png_base64: &str,
        known_ids: &[String],
    ) -> Result<(), PngStoreError> {
        Self::validate_id(agent_id)?;
        if !known_ids.iter().any(|id| id == agent_id) {
            return Err(PngStoreError::UnknownAgent);
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(png_base64.as_bytes())
            .map_err(|_| PngStoreError::BadBase64)?;
        if bytes.len() > self.max_bytes {
            return Err(PngStoreError::TooLarge(self.max_bytes));
        }
        if bytes.len() < PNG_MAGIC.len() || bytes[..PNG_MAGIC.len()] != PNG_MAGIC {
            return Err(PngStoreError::NotPng);
        }

        fs::create_dir_all(&self.dir).map_err(|e| PngStoreError::Io(e.to_string()))?;
        let final_path = self.path_for(agent_id);
        // profile_store와 동일: 같은 디렉터리에 임시파일 -> rename(원자적).
        let tmp = self
            .dir
            .join(format!("{agent_id}.png.tmp-{}", uuid::Uuid::new_v4()));
        fs::write(&tmp, &bytes).map_err(|e| PngStoreError::Io(e.to_string()))?;
        if let Err(e) = fs::rename(&tmp, &final_path) {
            let _ = fs::remove_file(&tmp);
            return Err(PngStoreError::Io(e.to_string()));
        }
        Ok(())
    }

    /// 파일을 읽어 base64로 돌려준다. 없으면 `Ok(None)`.
    pub fn load(&self, agent_id: &str) -> Result<Option<String>, PngStoreError> {
        Self::validate_id(agent_id)?;
        match fs::read(self.path_for(agent_id)) {
            Ok(bytes) => Ok(Some(
                base64::engine::general_purpose::STANDARD.encode(bytes),
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(PngStoreError::Io(e.to_string())),
        }
    }

    /// 파일을 삭제한다. 없어도 성공.
    pub fn delete(&self, agent_id: &str) -> Result<(), PngStoreError> {
        Self::validate_id(agent_id)?;
        match fs::remove_file(self.path_for(agent_id)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PngStoreError::Io(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-png-store-test-{}",
            uuid::Uuid::new_v4()
        ))
    }

    /// 최소 유효 PNG: 8바이트 시그니처 + 짧은 IHDR 흉내. 매직 바이트 검증만
    /// 통과시키면 되므로 완전한 PNG 디코드는 필요 없다.
    fn tiny_png_bytes() -> Vec<u8> {
        let mut v = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend_from_slice(b"rest-of-fake-png-body");
        v
    }

    fn b64(bytes: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn save_then_load_roundtrips_the_same_base64() {
        let dir = scratch_dir();
        let store = PngStore::new(dir.clone(), MAX_PORTRAIT_BYTES);
        let png = tiny_png_bytes();
        let encoded = b64(&png);

        store
            .save("p1", &encoded, &["p1".to_string()])
            .expect("save succeeds for a known agent with valid png");
        let loaded = store.load("p1").expect("load ok");

        assert_eq!(loaded, Some(encoded));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_returns_none_when_no_file() {
        let dir = scratch_dir();
        let store = PngStore::new(dir.clone(), MAX_PORTRAIT_BYTES);
        assert_eq!(store.load("ghost").expect("load ok"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_rejects_unknown_agent_id() {
        let dir = scratch_dir();
        let store = PngStore::new(dir.clone(), MAX_PORTRAIT_BYTES);
        let encoded = b64(&tiny_png_bytes());
        let err = store
            .save("nope", &encoded, &["p1".to_string()])
            .unwrap_err();
        assert!(matches!(err, PngStoreError::UnknownAgent));
        // 파일이 만들어지지 않아야 한다.
        assert!(!dir.join("nope.png").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_rejects_non_png_bytes() {
        let dir = scratch_dir();
        let store = PngStore::new(dir.clone(), MAX_PORTRAIT_BYTES);
        let encoded = b64(b"this is not a png at all");
        let err = store.save("p1", &encoded, &["p1".to_string()]).unwrap_err();
        assert!(matches!(err, PngStoreError::NotPng));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_rejects_oversize_payload() {
        let dir = scratch_dir();
        let store = PngStore::new(dir.clone(), MAX_PORTRAIT_BYTES);
        // 8바이트 PNG 시그니처 + 상한 초과 본문.
        let mut big = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        big.resize(MAX_PORTRAIT_BYTES + 1, 0u8);
        let encoded = b64(&big);
        let err = store.save("p1", &encoded, &["p1".to_string()]).unwrap_err();
        assert!(matches!(err, PngStoreError::TooLarge(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_rejects_bad_base64() {
        let dir = scratch_dir();
        let store = PngStore::new(dir.clone(), MAX_PORTRAIT_BYTES);
        let err = store
            .save("p1", "!!!!not base64!!!!", &["p1".to_string()])
            .unwrap_err();
        assert!(matches!(err, PngStoreError::BadBase64));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ids_with_path_separators_or_dotdot_are_rejected() {
        let dir = scratch_dir();
        let store = PngStore::new(dir.clone(), MAX_PORTRAIT_BYTES);
        let encoded = b64(&tiny_png_bytes());
        for bad in ["../evil", "a/b", "a\\b", "..", ""] {
            let err = store.save(bad, &encoded, &[bad.to_string()]).unwrap_err();
            assert!(
                matches!(err, PngStoreError::InvalidId),
                "id {bad:?} must be rejected"
            );
            assert!(store.load(bad).is_err(), "load must also guard {bad:?}");
            assert!(store.delete(bad).is_err(), "delete must also guard {bad:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_is_ok_even_when_file_absent() {
        let dir = scratch_dir();
        let store = PngStore::new(dir.clone(), MAX_PORTRAIT_BYTES);
        store.delete("p1").expect("delete of missing file is ok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_removes_an_existing_file() {
        let dir = scratch_dir();
        let store = PngStore::new(dir.clone(), MAX_PORTRAIT_BYTES);
        let encoded = b64(&tiny_png_bytes());
        store.save("p1", &encoded, &["p1".to_string()]).unwrap();
        assert!(dir.join("p1.png").exists());

        store.delete("p1").unwrap();
        assert!(!dir.join("p1.png").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_is_atomic_and_leaves_no_temp_file() {
        let dir = scratch_dir();
        let store = PngStore::new(dir.clone(), MAX_PORTRAIT_BYTES);
        let encoded = b64(&tiny_png_bytes());
        store.save("p1", &encoded, &["p1".to_string()]).unwrap();

        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(
            names.iter().any(|n| n == "p1.png"),
            "final file present: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains(".tmp")),
            "no temp file should remain after save: {names:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sprite_limit_is_one_mib() {
        assert_eq!(MAX_SPRITE_BYTES, 1024 * 1024);
    }

    #[test]
    fn max_bytes_is_per_instance() {
        let dir = scratch_dir();
        // sprites 인스턴스는 작은 상한을 강제한다.
        let store = PngStore::new(dir.clone(), MAX_SPRITE_BYTES);
        let mut big = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        big.resize(MAX_SPRITE_BYTES + 1, 0u8);
        let err = store
            .save("p1", &b64(&big), &["p1".to_string()])
            .unwrap_err();
        assert!(matches!(err, PngStoreError::TooLarge(_)));
        // 같은 페이로드도 portraits 상한으로는 통과한다.
        let store2 = PngStore::new(dir.clone(), MAX_PORTRAIT_BYTES);
        store2
            .save("p1", &b64(&big), &["p1".to_string()])
            .expect("2MiB 상한으로는 저장됨");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
