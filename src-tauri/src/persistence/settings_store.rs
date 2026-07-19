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
fn default_attention_hold_ms() -> u64 {
    5000
}

/// 라벨 요약에 사용할 CLI 제공자. 기존 설정과의 호환을 위해 기본은 Claude.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SummaryProvider {
    #[default]
    Claude,
    Codex,
}

impl SummaryProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

/// "OS 터미널로 열기"가 사용할 외부 터미널 앱. macOS에는 시스템 차원의
/// "기본 터미널" 개념이 없어 앱 설정으로 고른다 — 기본은 OS 제공
/// Terminal.app. macOS에서만 의미가 있다(다른 OS는 무시).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExternalTerminal {
    #[default]
    Terminal,
    Iterm,
}

/// 셸 출력 내보내기(.txt)를 열 외부 에디터. 기본은 OS 기본 연결(open/xdg-open).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExternalEditor {
    #[default]
    System,
    Vscode,
}

/// 앱 전역 설정. 요약과 관찰자 연동은 기본 OFF이고, 사운드는 기본 ON이다.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub version: u32,
    #[serde(default, alias = "claudeCliEnabled")]
    pub summarizer_enabled: bool,
    #[serde(default)]
    pub summary_provider: SummaryProvider,
    /// 캐릭터 일기(#56) 자동 생성 허용. 요약기와 같은 provider·CLI를 쓰므로
    /// 크레딧을 소모한다 → opt-in. 기본 꺼짐.
    #[serde(default)]
    pub diary_enabled: bool,
    #[serde(default, alias = "claudeHooksEnabled")]
    pub observer_enabled: bool,
    /// 사무실 앰비언스 사운드(타이핑·효과음·공조음) 재생 여부.
    #[serde(default = "default_true")]
    pub sound_enabled: bool,
    /// 마스터 볼륨 0.0~1.0.
    #[serde(default = "default_sound_volume")]
    pub sound_volume: f32,
    /// "OS 터미널로 열기"가 사용할 외부 터미널 앱(macOS 전용).
    #[serde(default)]
    pub external_terminal: ExternalTerminal,
    /// 셸 출력 내보내기(.txt)를 열 외부 에디터. 기본은 OS 기본 연결.
    #[serde(default)]
    pub external_editor: ExternalEditor,
    /// 질문(Hook) 알림을 방출 전 보류하는 시간(ms). 그 사이 세션이 계속
    /// 일하면(오토모드 자동 승인 등) 알림을 조용히 폐기한다. 0이면 즉시 알림.
    #[serde(default = "default_attention_hold_ms")]
    pub attention_hold_ms: u64,
    /// "작업 폴더 보기"(이슈 #11)에서 파일별 git 상태 뱃지를 조회할지. 거대
    /// 저장소에서 git status가 무거울 수 있어 끌 수 있게 한다. 기본 켜짐.
    #[serde(default = "default_true")]
    pub git_status_enabled: bool,
    /// 로컬 CLI 제어 서버(이슈 #55, docs/cli-control-design.md)를 띄울지.
    /// 켜면 `127.0.0.1`에 임의 포트로 control 서버가 뜨고 `control-port`가
    /// 기록된다. 하지만 실제 명령 수행은 앱에서 **명시적 승인**(control-token
    /// 발급)이 있어야 한다 — 2단계 옵트인. 보안 표면이므로 기본 꺼짐.
    #[serde(default)]
    pub cli_enabled: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: 1,
            summarizer_enabled: false,
            summary_provider: SummaryProvider::Claude,
            diary_enabled: false,
            observer_enabled: false,
            sound_enabled: true,
            sound_volume: 0.5,
            external_terminal: ExternalTerminal::Terminal,
            external_editor: ExternalEditor::System,
            attention_hold_ms: 5000,
            git_status_enabled: true,
            cli_enabled: false,
        }
    }
}

#[derive(Clone)]
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
        let tmp = self
            .file
            .with_file_name(format!("settings.json.tmp-{}", uuid::Uuid::new_v4()));
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
            .join(format!(
                "agent-office-settings-store-test-{}",
                uuid::Uuid::new_v4()
            ))
            .join("settings.json")
    }

    #[test]
    fn load_missing_file_returns_defaults_and_first_run_true() {
        let store = SettingsStore::new(scratch_file());
        let (s, first_run) = store.load();
        assert_eq!(s, AppSettings::default());
        assert!(!s.summarizer_enabled);
        assert_eq!(s.summary_provider, SummaryProvider::Claude);
        assert!(!s.observer_enabled);
        assert!(first_run);
    }

    #[test]
    fn save_then_load_roundtrips_and_first_run_false() {
        let file = scratch_file();
        let store = SettingsStore::new(file.clone());
        let s = AppSettings {
            version: 1,
            summarizer_enabled: true,
            summary_provider: SummaryProvider::Claude,
            diary_enabled: false,
            observer_enabled: true,
            sound_enabled: true,
            sound_volume: 0.5,
            external_terminal: ExternalTerminal::Terminal,
            external_editor: ExternalEditor::System,
            attention_hold_ms: 5000,
            git_status_enabled: true,
            cli_enabled: false,
        };
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
        fs::write(
            &file,
            br#"{"version":2,"claudeCliEnabled":true,"claudeHooksEnabled":true}"#,
        )
        .unwrap();
        let store = SettingsStore::new(file.clone());
        let (s, _) = store.load();
        assert_eq!(s, AppSettings::default());
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn legacy_true_fields_map_to_enabled_claude_without_version_migration() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            br#"{"version":1,"claudeCliEnabled":true,"claudeHooksEnabled":true}"#,
        )
        .unwrap();

        let (settings, first_run) = SettingsStore::new(file.clone()).load();
        assert!(!first_run);
        assert!(settings.summarizer_enabled);
        assert_eq!(settings.summary_provider, SummaryProvider::Claude);
        assert!(settings.observer_enabled);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn summary_provider_as_str_matches_serialized_values() {
        assert_eq!(SummaryProvider::Claude.as_str(), "claude");
        assert_eq!(SummaryProvider::Codex.as_str(), "codex");
    }

    #[test]
    fn legacy_false_fields_map_to_disabled_claude() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            br#"{"version":1,"claudeCliEnabled":false,"claudeHooksEnabled":false}"#,
        )
        .unwrap();

        let (settings, _) = SettingsStore::new(file.clone()).load();
        assert!(!settings.summarizer_enabled);
        assert_eq!(settings.summary_provider, SummaryProvider::Claude);
        assert!(!settings.observer_enabled);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn new_version_one_settings_round_trip_codex_and_neutral_keys() {
        let file = scratch_file();
        let store = SettingsStore::new(file.clone());
        let settings = AppSettings {
            version: 1,
            summarizer_enabled: true,
            summary_provider: SummaryProvider::Codex,
            diary_enabled: false,
            observer_enabled: true,
            sound_enabled: true,
            sound_volume: 0.5,
            external_terminal: ExternalTerminal::Iterm,
            external_editor: ExternalEditor::Vscode,
            attention_hold_ms: 5000,
            git_status_enabled: true,
            cli_enabled: false,
        };
        store.save(&settings).unwrap();
        let json = fs::read_to_string(&file).unwrap();
        assert!(json.contains("\"summarizerEnabled\""), "{json}");
        assert!(json.contains("\"summaryProvider\": \"codex\""), "{json}");
        assert!(json.contains("\"externalTerminal\": \"iterm\""), "{json}");
        assert!(json.contains("\"externalEditor\": \"vscode\""), "{json}");
        assert!(json.contains("\"attentionHoldMs\": 5000"), "{json}");
        assert!(json.contains("\"observerEnabled\""), "{json}");
        assert!(!json.contains("claudeCliEnabled"), "{json}");
        assert!(!json.contains("claudeHooksEnabled"), "{json}");
        assert_eq!(store.load(), (settings, false));
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn missing_provider_defaults_to_claude_and_unknown_provider_fails_safe() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, br#"{"version":1,"summarizerEnabled":true}"#).unwrap();
        assert_eq!(
            SettingsStore::new(file.clone()).load().0.summary_provider,
            SummaryProvider::Claude
        );

        fs::write(
            &file,
            br#"{"version":1,"summarizerEnabled":true,"summaryProvider":"unknown","observerEnabled":true}"#,
        )
        .unwrap();
        assert_eq!(
            SettingsStore::new(file.clone()).load().0,
            AppSettings::default()
        );
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
        assert!(
            !names.iter().any(|n| n.contains(".tmp")),
            "no temp left: {names:?}"
        );
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn serializes_camel_case() {
        let json = serde_json::to_string(&AppSettings::default()).unwrap();
        assert!(json.contains("summarizerEnabled"), "{json}");
        assert!(json.contains("summaryProvider"), "{json}");
        assert!(json.contains("observerEnabled"), "{json}");
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
        assert!(s.summarizer_enabled);
        assert_eq!(s.summary_provider, SummaryProvider::Claude);
        assert!(!s.observer_enabled);
        assert!(s.sound_enabled, "부재 시 기본 켜짐");
        assert_eq!(s.sound_volume, 0.5, "부재 시 기본 볼륨 0.5");
        assert_eq!(
            s.external_terminal,
            ExternalTerminal::Terminal,
            "부재 시 기본 Terminal.app"
        );
        assert_eq!(
            s.external_editor,
            ExternalEditor::System,
            "부재 시 기본 시스템 에디터"
        );
        assert_eq!(s.attention_hold_ms, 5000, "부재 시 기본 홀드 5초");
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }
}
