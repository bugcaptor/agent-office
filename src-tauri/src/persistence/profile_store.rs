// src-tauri/src/persistence/profile_store.rs
//
// JSON persistence for agent profiles (`profiles.json` in the Tauri app
// data dir). Completion criteria: load/save round-trips through a temp
// dir, a version-mismatched or corrupt file falls back to an empty state
// rather than erroring.
//
// `sessionId` is runtime-only (SessionManager/SessionRegistry memory)
// and is never part of `PersistedState`, so there is nothing to strip here
// -- the schema itself excludes it.

use std::fs;
use std::path::PathBuf;

use crate::types::PersistedState;

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
}

impl ProfileStore {
    pub fn new(file: PathBuf) -> Self {
        Self { file }
    }

    /// Reads and parses the profiles file. Falls back to
    /// `PersistedState::empty()` (version 1, no agents) whenever the file is
    /// missing, unreadable, not valid JSON, or has a `version` other than 1
    /// -- this is the migration-hook seam.
    pub fn load(&self) -> PersistedState {
        match fs::read(&self.file) {
            Ok(bytes) => match serde_json::from_slice::<PersistedState>(&bytes) {
                Ok(s) if s.version == 1 => migrate_loaded(s),
                _ => PersistedState::empty(),
            },
            Err(_) => PersistedState::empty(),
        }
    }

    /// Serializes `state` as pretty JSON, creating the parent directory first
    /// if it does not exist yet. The write is atomic: the bytes go to a
    /// temp file in the *same* directory, then `rename` swaps it into place, so
    /// a crash mid-write can never leave a truncated `profiles.json` — a reader
    /// sees either the old file or the fully-written new one.
    pub fn save(&self, state: &PersistedState) -> std::io::Result<()> {
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
        let name = self
            .file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("profiles.json");
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
            voice_id: None,
            bot: None,
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

    #[test]
    fn load_returns_empty_when_version_is_not_one() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, r#"{"agents":[],"version":2}"#).unwrap();

        let store = ProfileStore::new(file.clone());
        let state = store.load();

        assert_eq!(state.version, 1);
        assert!(state.agents.is_empty());

        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn load_returns_empty_when_file_is_not_json() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"not json at all").unwrap();

        let store = ProfileStore::new(file.clone());
        let state = store.load();

        assert_eq!(state.version, 1);
        assert!(state.agents.is_empty());

        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn load_returns_empty_when_json_is_truncated() {
        // Simulates a partial write (e.g. crash mid-write): valid JSON
        // prefix but cut off before it closes, so it fails to parse.
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let full = serde_json::to_vec_pretty(&sample_state()).unwrap();
        let truncated = &full[..full.len() / 2];
        fs::write(&file, truncated).unwrap();

        let store = ProfileStore::new(file.clone());
        let state = store.load();

        assert_eq!(state.version, 1);
        assert!(state.agents.is_empty());

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
}
