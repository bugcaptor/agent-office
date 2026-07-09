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

/// Loads/saves `PersistedState` to a single JSON file. The file path is
/// injected (see `new`) so tests can point it at a tempdir instead of the
/// real Tauri app data dir; that wiring happens in task 2I.
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
                Ok(s) if s.version == 1 => s,
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
    // used in notification/hook_settings.rs and session/manager.rs (no
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
                note: "note".into(),
                seed: "abc123".into(),
                created_at: 1_720_000_000_003,
                desk_index: 2,
                assigned_desk_index: None,
                cwd: None,
                appearance: None,
                portrait_updated_at: None,
                sprite_request: None,
                sprite_updated_at: None,
                archetype: None,
                shell: None,
            }],
            version: 1,
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
}
