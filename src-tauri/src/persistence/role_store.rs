// src-tauri/src/persistence/role_store.rs
//
// 토의 역할 목록 영속화. `<app_data>/roles.json` 하나(`RolesFile`)다.
//
// - 파일 부재는 `Ok(None)`이다. 렌더러는 이때만 프리셋을 시드한다 — 사용자가
//   전부 지운 빈 목록(`Some`)에는 프리셋이 되살아나지 않는다.
// - 손상·미래 버전은 Err이고 파일을 건드리지 않는다(awards_store 정책). save도
//   먼저 현재 파일을 읽어 같은 검사를 지난다.
// - 쓰기는 `durable::write_atomic`(fsync + rename).

use std::fs;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

use super::durable::write_atomic;
use crate::types::RolesFile;

/// 이 빌드가 읽을 수 있는 최대 스키마 버전. TS `ROLES_SCHEMA_VERSION` 미러.
pub const ROLES_SCHEMA_VERSION: u32 = 1;

pub struct RoleStore {
    file: PathBuf,
    lock: Mutex<()>,
}

impl RoleStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { file: dir.join("roles.json"), lock: Mutex::new(()) }
    }

    pub fn load(&self) -> Result<Option<RolesFile>, String> {
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        self.read()
    }

    fn read(&self) -> Result<Option<RolesFile>, String> {
        let bytes = match fs::read(&self.file) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let parsed: RolesFile =
            serde_json::from_slice(&bytes).map_err(|e| format!("roles-corrupt: {e}"))?;
        if parsed.version > ROLES_SCHEMA_VERSION {
            return Err(format!("roles-unsupported-version: {}", parsed.version));
        }
        Ok(Some(parsed))
    }

    /// 목록 전체를 바꾼다. id 비어 있음·중복, 이름 공백은 거부하고 기존 파일을 둔다.
    pub fn save(&self, file: &RolesFile) -> Result<(), String> {
        let mut ids = HashSet::new();
        if file.version != ROLES_SCHEMA_VERSION
            || file.roles.iter().any(|r| {
                r.id.trim().is_empty() || r.name.trim().is_empty() || !ids.insert(r.id.as_str())
            })
        {
            return Err("invalid-roles".into());
        }
        let bytes = serde_json::to_vec_pretty(file).map_err(|e| e.to_string())?;
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        // 손상·미래 버전 파일은 덮어쓰지 않는다.
        self.read()?;
        write_atomic(&self.file, &bytes).map_err(|e| e.to_string())
    }

    /// 읽고 고쳐 쓰기를 잠금 하나로 묶는다(역할 수신함). 파일이 없으면 `f`를 부르지 않고 `Ok(None)` —
    /// 프리셋은 렌더러가 시드한다. `f`가 false면 쓰지 않는다. 쓴 뒤의 목록을 돌려준다.
    pub fn update(&self, f: impl FnOnce(&mut RolesFile) -> bool) -> Result<Option<RolesFile>, String> {
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        let Some(mut file) = self.read()? else { return Ok(None) };
        if !f(&mut file) {
            return Ok(Some(file));
        }
        let bytes = serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?;
        write_atomic(&self.file, &bytes).map_err(|e| e.to_string())?;
        Ok(Some(file))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DiscussionRole, DiscussionStance, WorkAgent, WorkCli};

    fn store() -> (RoleStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("office-roles-{}", uuid::Uuid::new_v4()));
        (RoleStore::new(dir.clone()), dir)
    }
    fn role(id: &str, name: &str) -> DiscussionRole {
        DiscussionRole {
            id: id.into(),
            name: name.into(),
            perspective: "사용자 입장에서 본다".into(),
            stance: DiscussionStance::DevilsAdvocate,
            criteria: "근거 없는 동의 금지".into(),
            agent: Some(WorkAgent { cli: WorkCli::Codex, model: Some("gpt".into()), effort: None }),
        }
    }

    #[test]
    fn missing_file_is_none_and_roundtrip_keeps_every_field() {
        let (store, dir) = store();
        assert_eq!(store.load().unwrap(), None);
        let file = RolesFile { version: 1, roles: vec![role("r1", "비판자"), role("r2", "사회자")] };
        store.save(&file).unwrap();
        assert_eq!(store.load().unwrap(), Some(file));
        // 빈 목록도 "시드됨"으로 남는다(None과 구분).
        let empty = RolesFile { version: 1, roles: vec![] };
        store.save(&empty).unwrap();
        assert_eq!(store.load().unwrap(), Some(empty));
        let names: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("roles.json")]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_lists_are_rejected_without_touching_the_file() {
        let (store, dir) = store();
        let good = RolesFile { version: 1, roles: vec![role("r1", "a")] };
        store.save(&good).unwrap();
        for bad in [
            RolesFile { version: 1, roles: vec![role("r1", "a"), role("r1", "b")] },
            RolesFile { version: 1, roles: vec![role(" ", "a")] },
            RolesFile { version: 1, roles: vec![role("r2", "  ")] },
            RolesFile { version: 2, roles: vec![] },
        ] {
            assert_eq!(store.save(&bad), Err("invalid-roles".into()));
        }
        assert_eq!(store.load().unwrap(), Some(good));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_or_future_file_is_an_error_and_is_preserved() {
        let (store, dir) = store();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("roles.json");
        for raw in [&b"{broken"[..], &br#"{"version":9,"roles":[]}"#[..]] {
            fs::write(&path, raw).unwrap();
            assert!(store.load().is_err());
            assert!(store.save(&RolesFile { version: 1, roles: vec![] }).is_err());
            assert_eq!(fs::read(&path).unwrap(), raw);
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn optional_fields_default_when_absent() {
        let parsed: RolesFile = serde_json::from_str(
            r#"{"version":1,"roles":[{"id":"r","name":"n","stance":"moderator"}]}"#,
        )
        .unwrap();
        let r = &parsed.roles[0];
        assert_eq!((r.perspective.as_str(), r.criteria.as_str(), &r.agent), ("", "", &None));
        assert_eq!(r.stance, DiscussionStance::Moderator);
    }
}
