// src-tauri/src/persistence/profile_store.rs
//
// JSON persistence for agent profiles (`profiles.json` in the Tauri app
// data dir). Completion criteria: load/save round-trips through a temp
// dir; a missing file is an empty first-run state.
//
// 손상·버전 불일치 파일은 빈 상태로 취급하지 않는다(수정 F2): 빈 목록이
// 다음 저장에 원본을 덮어 캐릭터가 전부 사라지고, 세션 입양이 빈 known 집합을
// 믿고 살아 있는 세션을 Kill하기 때문이다. `try_load`는 Err를 돌려주며 원본을
// `profiles.json.corrupt-<시각>`로 한 번 복사해 두고, `save`는 디스크의 파일이
// 읽히지 않는 동안 쓰기를 거부한다(awards_store와 같은 정책).
//
// `sessionId` is runtime-only (SessionManager/SessionRegistry memory)
// and is never part of `PersistedState`, so there is nothing to strip here
// -- the schema itself excludes it.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::types::PersistedState;

/// 이 빌드가 읽고 쓰는 profiles.json 스키마 버전.
const PROFILE_SCHEMA_VERSION: u32 = 1;

/// profiles.json이 있는데 읽을 수 없음. 빈 상태로 대체하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileLoadError {
    /// 읽기·파싱·버전 실패 사유.
    pub reason: String,
    /// 원본을 복사해 둔 경로. 복사마저 실패하면 None.
    pub backup: Option<PathBuf>,
}

impl std::fmt::Display for ProfileLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "profiles.json을 읽을 수 없습니다: {}", self.reason)?;
        if let Some(backup) = &self.backup {
            write!(f, " (원본 보존: {})", backup.display())?;
        }
        Ok(())
    }
}

impl std::error::Error for ProfileLoadError {}

/// 레거시 메모를 성격 프롬프트에 합친다(순수). TS `mergeLegacyNote`의 거울 —
/// 둘 중 하나만 있으면 그것, 둘 다면 줄바꿈으로 잇는다. 성격 프롬프트가 이미
/// 메모 문구를 품고 있으면(편집창에서 한 번 합쳐진 경우) 그대로 둔다.
fn merge_legacy_note(personality: Option<&str>, note: Option<&str>) -> Option<String> {
    let p = personality.unwrap_or("").trim();
    let n = note.unwrap_or("").trim();
    if n.is_empty() {
        return if p.is_empty() { None } else { Some(p.to_string()) };
    }
    if p.is_empty() {
        return Some(n.to_string());
    }
    if p.contains(n) {
        return Some(p.to_string());
    }
    Some(format!("{p}\n{n}"))
}

/// 파일에서 막 읽은 상태를 앱이 쓰는 최신 모양으로 올린다(순수, 멱등).
///
/// - **메모 → 성격 프롬프트**: 편집창을 한 번도 열지 않은 사용자의 레거시
///   `note`도 여기서 합쳐진다. 합친 뒤 레거시 필드를 비우고, 다음 저장 때
///   `note` 키 자체가 사라진다(`skip_serializing`).
/// - **외모 힌트 → 초상화/스프라이트 추가 프롬프트**: 예전에는 한 칸(`appearance`)이
///   초상에 쓰이고 스프라이트의 폴백이기도 했다. 이제 칸이 갈라졌으므로 값을
///   양쪽에 복사해 그림 결과가 달라지지 않게 한다(이미 값이 있는 칸은 건드리지
///   않는다). 레거시 필드는 즉시 비워지므로 사용자가 나중에 한쪽을 지워도
///   다시 채워지지 않는다 — 마이그레이션은 파일당 1회다.
fn migrate_loaded(mut state: PersistedState) -> PersistedState {
    for a in &mut state.agents {
        if let Some(note) = a.legacy_note.take() {
            a.personality_prompt = merge_legacy_note(a.personality_prompt.as_deref(), Some(&note));
        }
        if let Some(legacy) = a.legacy_appearance.take() {
            let legacy = legacy.trim();
            if !legacy.is_empty() {
                let empty = |v: &Option<String>| v.as_deref().unwrap_or("").trim().is_empty();
                if empty(&a.portrait_request) {
                    a.portrait_request = Some(legacy.to_string());
                }
                if empty(&a.sprite_request) {
                    a.sprite_request = Some(legacy.to_string());
                }
            }
        }
    }
    state
}

/// Loads/saves `PersistedState` to a single JSON file. The file path is
/// injected (see `new`) so tests can point it at a tempdir instead of the
/// real Tauri app data dir; that wiring happens in task 2I.
#[derive(Clone)]
pub struct ProfileStore {
    file: PathBuf,
    /// 이번 프로세스에서 만든 손상 원본 백업. 클론(AppState·control)끼리
    /// 공유해 로드할 때마다 백업이 쌓이지 않게 한다.
    backup: Arc<Mutex<Option<PathBuf>>>,
}

impl ProfileStore {
    pub fn new(file: PathBuf) -> Self {
        Self { file, backup: Arc::new(Mutex::new(None)) }
    }

    /// 디스크 파일을 읽어 해석만 한다(부작용 없음). 파일이 없으면 Ok(None).
    fn read_disk(&self) -> Result<Option<PersistedState>, String> {
        let bytes = match fs::read(&self.file) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let state: PersistedState =
            serde_json::from_slice(&bytes).map_err(|e| format!("JSON 해석 실패: {e}"))?;
        if state.version != PROFILE_SCHEMA_VERSION {
            return Err(format!("지원하지 않는 버전 {}", state.version));
        }
        Ok(Some(state))
    }

    /// 프로필 파일을 읽는다. 파일이 없으면(첫 실행) 빈 상태. 있는데 읽을 수
    /// 없으면 원본을 백업하고 Err — 호출자는 이를 빈 목록으로 믿으면 안 된다.
    pub fn try_load(&self) -> Result<PersistedState, ProfileLoadError> {
        match self.read_disk() {
            Ok(Some(state)) => Ok(migrate_loaded(state)),
            Ok(None) => Ok(PersistedState::empty()),
            Err(reason) => {
                let backup = self.backup_corrupt();
                let err = ProfileLoadError { reason, backup };
                eprintln!("agent-office: {err}");
                Err(err)
            }
        }
    }

    /// 표시·조회 전용 읽기. 실패하면 빈 상태를 돌려준다 — 이 결과로 저장하거나
    /// 세션을 정리하는 경로는 반드시 `try_load`를 쓴다.
    pub fn load(&self) -> PersistedState {
        self.try_load().unwrap_or_else(|_| PersistedState::empty())
    }

    /// 손상 원본을 `<name>.corrupt-<시각>`으로 복사한다(원본은 그대로 둔다).
    /// 프로세스당 한 번 — 이미 만든 백업이 남아 있으면 그 경로를 돌려준다.
    fn backup_corrupt(&self) -> Option<PathBuf> {
        let mut made = self.backup.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(path) = made.as_ref().filter(|p| p.exists()) {
            return Some(path.clone());
        }
        let name = self.file_name();
        let stamp = chrono::Local::now().format("%Y%m%dT%H%M%S");
        let path = self.file.with_file_name(format!("{name}.corrupt-{stamp}"));
        match fs::copy(&self.file, &path) {
            Ok(_) => {
                *made = Some(path.clone());
                Some(path)
            }
            Err(e) => {
                eprintln!("agent-office: profiles.json 백업 실패: {e}");
                None
            }
        }
    }

    fn file_name(&self) -> &str {
        self.file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("profiles.json")
    }

    /// Serializes `state` as pretty JSON, creating the parent directory first
    /// if it does not exist yet. The write is atomic: the bytes go to a
    /// temp file in the *same* directory, then `rename` swaps it into place, so
    /// a crash mid-write can never leave a truncated `profiles.json` — a reader
    /// sees either the old file or the fully-written new one.
    ///
    /// 디스크의 파일이 읽히지 않는 동안에는 쓰지 않는다 — 렌더러가 빈 상태로
    /// 부팅했더라도 손상 원본을 덮어쓰지 못하게 하는 마지막 방어선이다.
    pub fn save(&self, state: &PersistedState) -> std::io::Result<()> {
        if let Err(reason) = self.read_disk() {
            return Err(std::io::Error::other(format!(
                "profiles.json을 읽을 수 없어 저장하지 않았습니다: {reason}"
            )));
        }
        if let Some(parent) = self.file.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(state)?;
        let tmp = self.tmp_path();
        fs::write(&tmp, &bytes)?;
        // rename within the same dir is atomic on the platforms we target.
        if let Err(e) = fs::rename(&tmp, &self.file) {
            let _ = fs::remove_file(&tmp); // don't leak the temp on failure
            return Err(e);
        }
        Ok(())
    }

    /// A unique sibling temp path in the same directory as `self.file` (same
    /// dir is required for `rename` to be atomic rather than a cross-device
    /// copy). uuid keeps concurrent saves from clobbering each other's temp.
    fn tmp_path(&self) -> PathBuf {
        let name = self.file_name();
        self.file
            .with_file_name(format!("{name}.tmp-{}", uuid::Uuid::new_v4()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AgentProfile;

    // Unique scratch dir under the OS temp dir, matching the convention
    // used in session/manager.rs and observer adapter tests (no
    // `tempfile` dependency needed -- `uuid` is already available).
    fn scratch_file() -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "agent-office-profile-store-test-{}",
                uuid::Uuid::new_v4()
            ))
            .join("profiles.json")
    }

    fn sample_state() -> PersistedState {
        PersistedState {
            agents: vec![AgentProfile {
                id: "p1".into(),
                name: "Ada".into(),
                role: "backend".into(),
                legacy_note: None,
                seed: "abc123".into(),
                created_at: 1_720_000_000_003,
                desk_index: 2,
                assigned_desk_index: None,
                cwd: None,
                legacy_appearance: None,
            portrait_request: None,
                portrait_updated_at: None,
                sprite_request: None,
                minimi_request: None,
                sprite_updated_at: None,
                minimi_updated_at: None,
                archetype: None,
            colors: None,
                shell: None,
                startup_command: None,
                personality_prompt: None,
                clocked_out: None,
            keyboard_sound: None,
            default_agent: None,
            }],
            version: 1,
            vacation_mode: None,
        }
    }

    #[test]
    fn load_returns_empty_when_file_does_not_exist() {
        let store = ProfileStore::new(scratch_file());
        let state = store.load();
        assert_eq!(state.version, 1);
        assert!(state.agents.is_empty());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let file = scratch_file();
        let store = ProfileStore::new(file.clone());
        let original = sample_state();

        store.save(&original).expect("save succeeds");
        let loaded = store.load();

        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.agents.len(), 1);
        assert_eq!(loaded.agents[0].id, "p1");
        assert_eq!(loaded.agents[0].name, "Ada");
        assert_eq!(loaded.agents[0].desk_index, 2);

        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn save_creates_missing_parent_directory() {
        let file = scratch_file();
        let parent = file.parent().unwrap().to_path_buf();
        assert!(!parent.exists());

        let store = ProfileStore::new(file.clone());
        store.save(&sample_state()).expect("save succeeds");

        assert!(parent.exists(), "parent dir should be created");
        assert!(file.exists(), "profiles.json should exist");

        let _ = fs::remove_dir_all(&parent);
    }

    /// 손상 파일을 써 두고 그 store를 돌려준다.
    fn store_with_raw(bytes: &[u8]) -> (PathBuf, ProfileStore) {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, bytes).unwrap();
        (file.clone(), ProfileStore::new(file))
    }

    fn corrupt_backups(file: &PathBuf) -> Vec<PathBuf> {
        fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_name().unwrap().to_str().unwrap().starts_with("profiles.json.corrupt-"))
            .collect()
    }

    /// 손상 파일 회귀(수정 F2): Err + 원본 백업 + 저장 거부 + 원본 무손상.
    fn assert_corrupt_is_preserved(bytes: &[u8]) {
        let (file, store) = store_with_raw(bytes);

        let err = store.try_load().expect_err("corrupt file must not load as empty");
        let backup = err.backup.clone().expect("original is backed up");
        assert_eq!(fs::read(&backup).unwrap(), bytes, "backup holds the original bytes");
        // 표시 전용 load()는 여전히 빈 상태로 폴백한다.
        assert!(store.load().agents.is_empty());

        let save = store.save(&PersistedState::empty());
        assert!(save.is_err(), "save must refuse to overwrite an unreadable file");
        assert_eq!(fs::read(&file).unwrap(), bytes, "original left untouched");

        // 여러 번 로드해도(클론 포함) 백업은 하나뿐이다.
        let _ = store.clone().try_load();
        assert_eq!(corrupt_backups(&file), vec![backup]);

        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn unsupported_version_is_an_error_not_an_empty_state() {
        assert_corrupt_is_preserved(br#"{"agents":[{"id":"p1"}],"version":2}"#);
    }

    #[test]
    fn non_json_file_is_an_error_not_an_empty_state() {
        assert_corrupt_is_preserved(b"not json at all");
    }

    #[test]
    fn truncated_json_is_an_error_not_an_empty_state() {
        // Simulates a partial write (e.g. crash mid-write): valid JSON
        // prefix but cut off before it closes, so it fails to parse.
        let full = serde_json::to_vec_pretty(&sample_state()).unwrap();
        assert_corrupt_is_preserved(&full[..full.len() / 2]);
    }

    #[test]
    fn missing_file_loads_empty_and_save_creates_it() {
        let file = scratch_file();
        let store = ProfileStore::new(file.clone());
        assert_eq!(store.try_load().unwrap().agents.len(), 0);
        store.save(&sample_state()).expect("first-run save succeeds");
        assert_eq!(store.try_load().unwrap().agents.len(), 1);
        assert!(corrupt_backups(&file).is_empty());
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn save_resumes_once_the_file_is_readable_again() {
        let (file, store) = store_with_raw(b"{broken");
        assert!(store.try_load().is_err());
        assert!(store.save(&sample_state()).is_err());

        // 사용자가 원본을 고쳤거나 지웠다 → 저장이 다시 열린다.
        fs::remove_file(&file).unwrap();
        store.save(&sample_state()).expect("save works after the file is gone");
        assert_eq!(store.try_load().unwrap().agents[0].id, "p1");
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn save_is_atomic_temp_then_rename_and_leaves_no_temp_file() {
        // save() writes a sibling temp then renames it into place. After
        // a successful save the dir holds exactly profiles.json — no leftover
        // ".tmp-" file — and the content round-trips.
        let file = scratch_file();
        let store = ProfileStore::new(file.clone());

        store.save(&sample_state()).expect("save succeeds");

        let dir = file.parent().unwrap();
        let names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(names.iter().any(|n| n == "profiles.json"), "final file present: {names:?}");
        assert!(
            !names.iter().any(|n| n.contains(".tmp")),
            "no temp file should remain after save: {names:?}"
        );

        let loaded = store.load();
        assert_eq!(loaded.agents.len(), 1);
        assert_eq!(loaded.agents[0].id, "p1");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_overwrites_previous_contents() {
        let file = scratch_file();
        let store = ProfileStore::new(file.clone());

        store.save(&sample_state()).unwrap();
        store.save(&PersistedState::empty()).unwrap();

        let loaded = store.load();
        assert!(loaded.agents.is_empty());

        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    // ── 레거시 마이그레이션(kbm #2fh) ────────────────────────────────

    /// 레거시 키(note/appearance)만 담긴 profiles.json 텍스트.
    fn legacy_file_json(note: &str, appearance: &str, extra: &str) -> String {
        format!(
            r#"{{"agents":[{{"id":"p1","name":"Ada","role":"backend","note":"{note}",
                "appearance":"{appearance}","seed":"abc123","createdAt":1,"deskIndex":0{extra}}}],
                "version":1}}"#
        )
    }

    fn write_and_load(json: &str) -> (PathBuf, PersistedState) {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, json).unwrap();
        let state = ProfileStore::new(file.clone()).load();
        (file, state)
    }

    #[test]
    fn load_merges_legacy_note_into_personality_prompt() {
        let (file, state) = write_and_load(&legacy_file_json("백엔드 담당", "", ""));
        let a = &state.agents[0];
        assert_eq!(a.personality_prompt.as_deref(), Some("백엔드 담당"));
        assert_eq!(a.legacy_note, None);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn load_appends_legacy_note_below_existing_personality_prompt() {
        let (file, state) = write_and_load(&legacy_file_json(
            "백엔드 담당",
            "",
            r#","personalityPrompt":"차분한 성격""#,
        ));
        assert_eq!(
            state.agents[0].personality_prompt.as_deref(),
            Some("차분한 성격\n백엔드 담당")
        );
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn load_does_not_duplicate_note_already_merged_by_the_editor() {
        // 편집창이 먼저 합쳐 저장했지만 파일에 note가 남아 있던 경우.
        let (file, state) = write_and_load(&legacy_file_json(
            "백엔드 담당",
            "",
            r#","personalityPrompt":"차분한 성격\n백엔드 담당""#,
        ));
        assert_eq!(
            state.agents[0].personality_prompt.as_deref(),
            Some("차분한 성격\n백엔드 담당")
        );
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn load_copies_legacy_appearance_into_both_request_fields() {
        // 예전 "외모 힌트"는 초상에 쓰이고 스프라이트의 폴백이기도 했다 —
        // 두 칸으로 갈라진 뒤에도 그림 결과가 같아야 하므로 양쪽에 복사한다.
        let (file, state) = write_and_load(&legacy_file_json("", "짧은 검은 머리", ""));
        let a = &state.agents[0];
        assert_eq!(a.portrait_request.as_deref(), Some("짧은 검은 머리"));
        assert_eq!(a.sprite_request.as_deref(), Some("짧은 검은 머리"));
        assert_eq!(a.legacy_appearance, None);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn load_keeps_existing_sprite_request_over_legacy_appearance() {
        let (file, state) = write_and_load(&legacy_file_json(
            "",
            "짧은 검은 머리",
            r#","spriteRequest":"빨간 망토 마법사""#,
        ));
        let a = &state.agents[0];
        assert_eq!(a.portrait_request.as_deref(), Some("짧은 검은 머리"));
        assert_eq!(a.sprite_request.as_deref(), Some("빨간 망토 마법사"));
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn migrated_state_drops_legacy_keys_on_next_save() {
        // 마이그레이션은 파일당 1회여야 한다: 저장하면 레거시 키가 사라지고,
        // 사용자가 나중에 스프라이트 칸을 비워도 다시 채워지지 않는다.
        let (file, state) = write_and_load(&legacy_file_json("백엔드 담당", "짧은 검은 머리", ""));
        let store = ProfileStore::new(file.clone());
        store.save(&state).unwrap();

        let text = fs::read_to_string(&file).unwrap();
        assert!(!text.contains("\"note\""), "note 키가 남아 있다: {text}");
        assert!(!text.contains("\"appearance\""), "appearance 키가 남아 있다: {text}");

        let mut cleared = store.load();
        cleared.agents[0].sprite_request = None;
        store.save(&cleared).unwrap();
        assert_eq!(store.load().agents[0].sprite_request, None);

        let _ = fs::remove_dir_all(file.parent().unwrap());
    }
    #[test]
    fn default_work_agent_roundtrips_and_legacy_profile_defaults_to_none() {
        let file = scratch_file(); let store = ProfileStore::new(file.clone()); let mut state = sample_state();
        assert!(state.agents[0].default_agent.is_none());
        state.agents[0].default_agent = Some(crate::types::WorkAgent { cli: crate::types::WorkCli::Codex, model: Some("custom".into()), effort: Some("high".into()) });
        store.save(&state).unwrap(); assert_eq!(store.load().agents[0].default_agent, state.agents[0].default_agent);
        let mut value = serde_json::to_value(&state).unwrap(); value["agents"][0].as_object_mut().unwrap().remove("defaultAgent");
        fs::write(&file, serde_json::to_vec(&value).unwrap()).unwrap(); assert!(store.load().agents[0].default_agent.is_none());
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

}
