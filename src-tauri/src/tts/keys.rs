// src-tauri/src/tts/keys.rs
//
// TTS가 쓰는 외부 API 키 두 개(ElevenLabs, Anthropic)의 보관소.
//
// 왜 `AppSettings`가 아니라 별도 파일인가: `get_app_settings`는 설정 구조체를
// **통째로** 렌더러(웹뷰)에 돌려준다. 키를 설정에 넣으면 그 순간부터 웹뷰
// 컨텍스트에 평문 키가 상주하고, 렌더러 크래시 리포트·devtools·XSS 표면 전부에
// 노출된다. 그래서 키는 이 파일에만 두고, 렌더러에는 **존재 여부 bool만**
// (`TtsKeyStatus`) 내려준다. 합성은 전부 백엔드에서 하고 렌더러는 오디오
// 바이트만 받는다.
//
// 파일은 `<app-data>/tts-keys.json`, unix에서는 0600. 쓰기는 다른 스토어와 같은
// temp+rename 원자 교체이며, temp도 처음부터 0600으로 만든다(rename 전 잠깐
// 0644로 노출되는 창을 없앤다).
//
// 키는 로그·에러 메시지에 절대 싣지 않는다(api_keys.rs와 같은 규칙).

use std::fs;
use std::path::PathBuf;

/// 저장된 키가 없을 때 폴백할 환경변수 이름. Claude CLI를 쓰는 사용자는 이미
/// 셸에 이걸 갖고 있는 경우가 많아 설정 입력을 건너뛸 수 있다.
pub const ANTHROPIC_API_KEY_ENV: &str = "ANTHROPIC_API_KEY";
/// ElevenLabs 키도 같은 편의를 준다(설정 입력이 정공법이지만 env가 있으면 존중).
pub const ELEVENLABS_API_KEY_ENV: &str = "ELEVENLABS_API_KEY";

/// 디스크 표현. 필드가 비어 있으면 "미설정"이다(Option 대신 빈 문자열을 쓰는
/// 이유: 렌더러에서 "지우기"를 빈 문자열 전송으로 자연스럽게 표현할 수 있다).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredKeys {
    #[serde(default)]
    elevenlabs: String,
    #[serde(default)]
    anthropic: String,
}

/// 렌더러에 내려가는 마스킹된 상태 — 키 값은 절대 포함하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsKeyStatus {
    /// ElevenLabs 키를 (저장값이든 env든) 쓸 수 있는지.
    pub elevenlabs_set: bool,
    /// Anthropic 키를 (저장값이든 env든) 쓸 수 있는지. 없으면 리라이트를
    /// 건너뛰고 원문을 그대로 읽는다(우아한 강등).
    pub anthropic_set: bool,
    /// ElevenLabs 키가 저장값이 아니라 env 폴백인지(UI 안내용).
    pub elevenlabs_from_env: bool,
    /// Anthropic 키가 저장값이 아니라 env 폴백인지(UI 안내용).
    pub anthropic_from_env: bool,
}

/// 공백뿐이면 None으로 정규화한다.
fn nonblank(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

pub struct TtsKeyStore {
    file: PathBuf,
}

impl TtsKeyStore {
    pub fn new(file: PathBuf) -> Self {
        Self { file }
    }

    /// 파일 부재/파손은 "키 없음"으로 폴백한다 — TTS는 장식 기능이라 여기서
    /// 앱을 막지 않는다.
    fn load(&self) -> StoredKeys {
        fs::read(&self.file)
            .ok()
            .and_then(|b| serde_json::from_slice::<StoredKeys>(&b).ok())
            .unwrap_or_default()
    }

    /// 저장값 우선, 없으면 env 폴백. (값, env유래여부).
    fn resolve(stored: &str, env_name: &str) -> (Option<String>, bool) {
        match nonblank(stored) {
            Some(v) => (Some(v), false),
            None => match crate::api_keys::env_api_key(env_name) {
                Some(v) => (Some(v), true),
                None => (None, false),
            },
        }
    }

    pub fn elevenlabs_key(&self) -> Option<String> {
        Self::resolve(&self.load().elevenlabs, ELEVENLABS_API_KEY_ENV).0
    }

    pub fn anthropic_key(&self) -> Option<String> {
        Self::resolve(&self.load().anthropic, ANTHROPIC_API_KEY_ENV).0
    }

    pub fn status(&self) -> TtsKeyStatus {
        let stored = self.load();
        let (el, el_env) = Self::resolve(&stored.elevenlabs, ELEVENLABS_API_KEY_ENV);
        let (an, an_env) = Self::resolve(&stored.anthropic, ANTHROPIC_API_KEY_ENV);
        TtsKeyStatus {
            elevenlabs_set: el.is_some(),
            anthropic_set: an.is_some(),
            elevenlabs_from_env: el_env,
            anthropic_from_env: an_env,
        }
    }

    /// `None`인 필드는 기존 값을 유지한다(부분 갱신). `Some("")`는 삭제다 —
    /// 지운 뒤에도 env 폴백은 살아 있으므로 status가 계속 true일 수 있다.
    pub fn set(
        &self,
        elevenlabs: Option<String>,
        anthropic: Option<String>,
    ) -> std::io::Result<TtsKeyStatus> {
        let mut stored = self.load();
        if let Some(v) = elevenlabs {
            stored.elevenlabs = v.trim().to_string();
        }
        if let Some(v) = anthropic {
            stored.anthropic = v.trim().to_string();
        }
        self.save(&stored)?;
        Ok(self.status())
    }

    fn save(&self, keys: &StoredKeys) -> std::io::Result<()> {
        if let Some(parent) = self.file.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(keys)?;
        let tmp = self
            .file
            .with_file_name(format!("tts-keys.json.tmp-{}", uuid::Uuid::new_v4()));
        write_private(&tmp, &bytes)?;
        if let Err(e) = fs::rename(&tmp, &self.file) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        Ok(())
    }
}

/// 0600(unix)으로 새 파일을 만들어 쓴다. 다른 OS는 기본 권한 — control-token과
/// 같은 관례다(Windows에는 대응 개념이 없다).
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(bytes)?;
    f.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        std::env::temp_dir()
            .join(format!("agent-office-tts-keys-{}", uuid::Uuid::new_v4()))
            .join("tts-keys.json")
    }

    #[test]
    fn missing_file_means_no_stored_keys() {
        let store = TtsKeyStore::new(scratch());
        let s = store.load();
        assert_eq!(s, StoredKeys::default());
    }

    #[test]
    fn set_then_status_reports_stored_not_env() {
        let file = scratch();
        let store = TtsKeyStore::new(file.clone());
        let st = store
            .set(Some("  xi-abc  ".into()), Some("sk-ant-xyz".into()))
            .unwrap();
        assert!(st.elevenlabs_set && st.anthropic_set);
        assert!(!st.elevenlabs_from_env && !st.anthropic_from_env);
        // 트림돼서 저장된다.
        assert_eq!(store.elevenlabs_key().as_deref(), Some("xi-abc"));
        assert_eq!(store.anthropic_key().as_deref(), Some("sk-ant-xyz"));
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn none_field_keeps_previous_value() {
        let file = scratch();
        let store = TtsKeyStore::new(file.clone());
        store.set(Some("xi-1".into()), Some("an-1".into())).unwrap();
        store.set(None, Some("an-2".into())).unwrap();
        assert_eq!(store.elevenlabs_key().as_deref(), Some("xi-1"));
        assert_eq!(store.anthropic_key().as_deref(), Some("an-2"));
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn empty_string_clears_stored_key() {
        let file = scratch();
        let store = TtsKeyStore::new(file.clone());
        store.set(Some("xi-1".into()), None).unwrap();
        store.set(Some("".into()), None).unwrap();
        // env가 없다면 미설정으로 떨어진다(테스트 환경에 ELEVENLABS_API_KEY가
        // 있으면 env 폴백이 잡히므로 저장값 자체만 검사한다).
        assert_eq!(store.load().elevenlabs, "");
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn corrupt_file_falls_back_to_empty() {
        let file = scratch();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"not json").unwrap();
        let store = TtsKeyStore::new(file.clone());
        assert_eq!(store.load(), StoredKeys::default());
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn save_leaves_no_temp_file() {
        let file = scratch();
        let store = TtsKeyStore::new(file.clone());
        store.set(Some("xi".into()), None).unwrap();
        let names: Vec<String> = fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["tts-keys.json".to_string()]);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let file = scratch();
        let store = TtsKeyStore::new(file.clone());
        store.set(Some("xi".into()), None).unwrap();
        let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "키 파일은 소유자만 읽을 수 있어야 한다");
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }
}
