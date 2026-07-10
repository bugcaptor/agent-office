// src-tauri/src/persistence/settings_store.rs
//
// 앱 전역 설정(`settings.json`, Tauri app data dir) 영속화. 파일 부재 =
// 첫 실행(first_run=true) — 렌더러가 첫 실행 동의 다이얼로그를 띄우는
// 신호다. 파손/버전 불일치는 기본값(Claude 기능 OFF, 사운드 ON)으로 폴백하되 first_run은
// false(파일이 존재했다는 것 자체가 온보딩 완료의 증거 — 유저를 온보딩으로
// 다시 괴롭히지 않는다). 쓰기는 ProfileStore와 같은 temp+rename 원자 쓰기.

use std::fs;
use std::path::PathBuf;

fn default_true() -> bool {
    true
}
fn default_sound_volume() -> f32 {
    0.5
}

/// 앱 전역 opt-in 설정. Claude 관련은 기본 false(명시적 opt-in),
/// 사운드는 기본 켜짐(장식 기능 — 끄는 쪽이 opt-in).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub version: u32,
    #[serde(default)]
    pub claude_cli_enabled: bool,
    #[serde(default)]
    pub claude_hooks_enabled: bool,
    /// 사무실 앰비언스 사운드(타이핑·효과음·공조음) 재생 여부.
    #[serde(default = "default_true")]
    pub sound_enabled: bool,
    /// 마스터 볼륨 0.0~1.0.
    #[serde(default = "default_sound_volume")]
    pub sound_volume: f32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: 1,
            claude_cli_enabled: false,
            claude_hooks_enabled: false,
            sound_enabled: true,
            sound_volume: 0.5,
        }
    }
}

pub struct SettingsStore {
    file: PathBuf,
}

impl SettingsStore {
    pub fn new(file: PathBuf) -> Self {
        Self { file }
    }

    /// (설정, first_run). first_run은 "파일이 아예 없다"일 때만 true.
    pub fn load(&self) -> (AppSettings, bool) {
        match fs::read(&self.file) {
            Ok(bytes) => match serde_json::from_slice::<AppSettings>(&bytes) {
                Ok(s) if s.version == 1 => (s, false),
                _ => (AppSettings::default(), false),
            },
            Err(_) => (AppSettings::default(), true),
        }
    }

    pub fn save(&self, settings: &AppSettings) -> std::io::Result<()> {
        if let Some(parent) = self.file.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(settings)?;
        let tmp = self.file.with_file_name(format!(
            "settings.json.tmp-{}",
            uuid::Uuid::new_v4()
        ));
        fs::write(&tmp, &bytes)?;
        if let Err(e) = fs::rename(&tmp, &self.file) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn scratch_file() -> PathBuf {
        std::env::temp_dir()
            .join(format!("agent-office-settings-store-test-{}", uuid::Uuid::new_v4()))
            .join("settings.json")
    }

    #[test]
    fn load_missing_file_returns_defaults_and_first_run_true() {
        let store = SettingsStore::new(scratch_file());
        let (s, first_run) = store.load();
        assert_eq!(s, AppSettings::default());
        assert!(!s.claude_cli_enabled);
        assert!(!s.claude_hooks_enabled);
        assert!(first_run);
    }

    #[test]
    fn save_then_load_roundtrips_and_first_run_false() {
        let file = scratch_file();
        let store = SettingsStore::new(file.clone());
        let s = AppSettings { version: 1, claude_cli_enabled: true, claude_hooks_enabled: true, sound_enabled: true, sound_volume: 0.5 };
        store.save(&s).expect("save succeeds");
        let (loaded, first_run) = store.load();
        assert_eq!(loaded, s);
        assert!(!first_run);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn load_corrupt_file_returns_defaults_and_first_run_false() {
        // 파손 파일은 기본값(전부 OFF)으로 안전 폴백하되, 온보딩을 다시
        // 띄우지 않는다(first_run=false) — 파일이 존재했다는 사실 자체가
        // 온보딩 완료의 증거.
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"not json").unwrap();
        let store = SettingsStore::new(file.clone());
        let (s, first_run) = store.load();
        assert_eq!(s, AppSettings::default());
        assert!(!first_run);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn load_unknown_version_returns_defaults_and_first_run_false() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, br#"{"version":2,"claudeCliEnabled":true,"claudeHooksEnabled":true}"#).unwrap();
        let store = SettingsStore::new(file.clone());
        let (s, _) = store.load();
        assert_eq!(s, AppSettings::default());
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn save_is_atomic_and_leaves_no_temp_file() {
        let file = scratch_file();
        let store = SettingsStore::new(file.clone());
        store.save(&AppSettings::default()).expect("save succeeds");
        let names: Vec<String> = fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(names.iter().any(|n| n == "settings.json"));
        assert!(!names.iter().any(|n| n.contains(".tmp")), "no temp left: {names:?}");
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn serializes_camel_case() {
        let json = serde_json::to_string(&AppSettings::default()).unwrap();
        assert!(json.contains("claudeCliEnabled"), "{json}");
        assert!(json.contains("claudeHooksEnabled"), "{json}");
    }

    // 하위 호환: 사운드 필드가 없는 기존 settings.json도 기본값(켜짐/0.5)으로
    // 로드된다 — 버전 마이그레이션 없이 serde default로 처리.
    #[test]
    fn load_settings_without_sound_fields_falls_back_to_defaults() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            br#"{"version":1,"claudeCliEnabled":true,"claudeHooksEnabled":false}"#,
        )
        .unwrap();
        let (s, first_run) = SettingsStore::new(file.clone()).load();
        assert!(!first_run);
        assert!(s.claude_cli_enabled);
        assert!(s.sound_enabled, "부재 시 기본 켜짐");
        assert_eq!(s.sound_volume, 0.5, "부재 시 기본 볼륨 0.5");
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }
}
