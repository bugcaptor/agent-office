// src-tauri/src/tts/mod.rs
//
// 확인 요청 대사 TTS. AI 에이전트가 사용자 확인을 기다릴 때(질문 알림,
// source=hook) 그 시스템 문구를 캐릭터 말투의 짧은 대사로 리라이트하고
// (rewrite.rs) ElevenLabs로 합성해(synth.rs) 렌더러에 오디오 바이트를 준다.
//
// 파이프라인:
//   문구 ─rewrite─▶ 대사 ─pick_voice(seed 해시)─▶ voice_id
//         │                └─cache hit?─▶ 디스크 mp3
//         │                └─miss──────▶ synth(ElevenLabs) ─▶ 캐시 저장
//         └ 공급자 체인(resolve_rewrite_route): Anthropic Messages API(rewrite.rs)
//           → `claude -p` 헤드리스 CLI(cli.rs) → 생략(원문 발화)
//
// 설계 원칙 세 가지:
//  1) **장식 기능이다.** 어느 단계가 실패해도 앱 동작에 영향이 없어야 한다.
//     리라이트 실패는 원문 그대로 읽기로 강등하고, 합성 실패만 렌더러에 에러로
//     올린다(렌더러는 그때 기존 딩으로 대체한다).
//  2) **키는 웹뷰에 안 나간다.** 합성 전체가 백엔드에서 끝나고 렌더러는 base64
//     오디오만 받는다(keys.rs 머리말 참고).
//  3) **결정적 배정 + 캐시.** 같은 캐릭터는 항상 같은 목소리(voice.rs), 같은
//     (voice, model, 텍스트)는 API를 다시 부르지 않는다.

pub mod cli;
pub mod keys;
pub mod rewrite;
pub mod synth;
pub mod voice;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::persistence::settings_store::{TtsRewriteModel, TtsRewriteProvider};

/// 캐시 파일 수 상한. 넘으면 mtime이 오래된 것부터 지운다. 한 줄 대사 mp3는
/// 수십 KB라 이 정도면 수 MB 수준이다.
pub const MAX_CACHE_FILES: usize = 300;

/// `tts_speak` 입력. 캐릭터 정보는 렌더러 스토어에서 온다(백엔드가 프로필을
/// 다시 읽지 않는 이유: 마스코트 창처럼 스토어가 진실의 원천인 값들이다).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakRequest {
    pub agent_id: String,
    #[serde(default)]
    pub agent_name: String,
    #[serde(default)]
    pub archetype: Option<String>,
    #[serde(default)]
    pub seed: String,
    pub message: String,
}

/// `tts_speak` 결과. `line`은 실제로 합성된 텍스트(디버그/표시용) —
/// 리라이트가 강등됐다면 원문과 같다.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakResult {
    /// mp3 바이트의 base64(데이터 URL 접두사 없음).
    pub audio_base64: String,
    pub mime_type: String,
    /// 합성에 쓴 최종 텍스트(v2 폴백이면 태그가 제거된 상태).
    pub line: String,
    /// 배정된 보이스(디버그용).
    pub voice_id: String,
    /// 사용한 ElevenLabs model_id(`eleven_v3` 또는 폴백).
    pub model_id: String,
    /// 디스크 캐시 히트였는지(외부 API 호출 없음).
    pub cached: bool,
    /// 대사가 LLM 리라이트를 거쳤는지. false면 원문 그대로 읽었다는 뜻.
    pub rewritten: bool,
    /// 실제로 리라이트를 수행한 경로. 강등됐으면 `"none"`.
    pub rewrite_via: &'static str,
}

/// `tts_key_status`가 돌려주는 설정 UI용 상태. 키는 **존재 여부만** 담고
/// (keys.rs 머리말), 여기에 claude CLI 가용성을 더해 "자동" 선택이 실제로 어느
/// 경로를 쓸지 UI가 안내할 수 있게 한다.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsStatus {
    #[serde(flatten)]
    pub keys: keys::TtsKeyStatus,
    /// PATH에 `claude`가 있는지(앱 수명 동안 1회 탐색 후 캐시).
    pub claude_cli_available: bool,
    /// 현재 설정으로 실제 선택될 리라이트 경로 라벨(`"api"`/`"claude-cli"`/`"none"`).
    pub effective_rewrite_via: &'static str,
}

/// 리라이트를 실제로 수행할 수단. `TtsRewriteProvider`(사용자 의도) + 지금
/// 쓸 수 있는 자원(키/CLI)을 합쳐 결정한 결과다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewriteRoute {
    /// Anthropic Messages API. 키를 값으로 들고 있다(설정값 또는 env 폴백).
    Api(String),
    /// `claude -p` 헤드리스 서브프로세스(구독 사용량 소모).
    ClaudeCli,
    /// 리라이트 없음 — 원문 발화.
    None,
}

impl RewriteRoute {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Api(_) => "api",
            Self::ClaudeCli => "claude-cli",
            Self::None => "none",
        }
    }
}

/// 설정(공급자 의도) + 가용 자원 → 실제 경로. 순수 — `anthropic_key`는
/// 호출측이 키 스토어에서 미리 해결해 넘긴다(테스트 가능성 확보).
///
/// `Auto` 순서: 저장/env API 키 → claude CLI → 생략. API를 먼저 두는 이유는
/// 키가 있으면 그게 가장 빠르고(6초 타임아웃) 구독 사용량을 건드리지 않기
/// 때문이다. `claude_cli_available`은 PATH 탐색 결과다.
pub fn resolve_rewrite_route(
    provider: TtsRewriteProvider,
    anthropic_key: Option<String>,
    claude_cli_available: bool,
) -> RewriteRoute {
    match provider {
        TtsRewriteProvider::None => RewriteRoute::None,
        TtsRewriteProvider::Api => match anthropic_key {
            Some(k) => RewriteRoute::Api(k),
            None => RewriteRoute::None,
        },
        TtsRewriteProvider::ClaudeCli => {
            if claude_cli_available {
                RewriteRoute::ClaudeCli
            } else {
                RewriteRoute::None
            }
        }
        TtsRewriteProvider::Auto => match anthropic_key {
            Some(k) => RewriteRoute::Api(k),
            None if claude_cli_available => RewriteRoute::ClaudeCli,
            None => RewriteRoute::None,
        },
    }
}

/// PATH에 `claude`가 있는지. 결과를 프로세스 수명 동안 캐시한다 — 매 알림마다
/// PATH를 훑을 이유가 없고, 설치/삭제 중간 상태는 앱 재시작으로 반영된다.
pub fn claude_cli_available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| which_claude().is_some())
}

/// `claude` 실행 파일 탐색. Windows는 shim(.cmd/.ps1/.exe)이라 PATHEXT를 함께 본다.
fn which_claude() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    #[cfg(windows)]
    let exts: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .filter(|e| !e.is_empty())
        .map(|e| e.to_lowercase())
        .collect();
    #[cfg(not(windows))]
    let exts: Vec<String> = vec![String::new()];
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("claude{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// IPC로는 `"{code}: {상세}"`로 나간다(pixellab과 같은 관례 — 렌더러가 첫 ':'
/// 앞 코드로 분기한다).
#[derive(Debug, Clone, PartialEq)]
pub enum TtsError {
    Disabled,
    MissingElevenLabsKey,
    EmptyMessage,
    NoVoiceAvailable,
    Synth(synth::SynthError),
    Cache(String),
}

impl TtsError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Disabled => "tts_disabled",
            Self::MissingElevenLabsKey => "missing_elevenlabs_key",
            Self::EmptyMessage => "empty_message",
            Self::NoVoiceAvailable => "no_voice",
            Self::Synth(e) => e.code(),
            Self::Cache(_) => "cache",
        }
    }

    pub fn to_ipc_string(&self) -> String {
        let detail = match self {
            Self::Disabled => "TTS 설정이 꺼져 있습니다".to_string(),
            Self::MissingElevenLabsKey => "ElevenLabs API 키가 설정되지 않았습니다".to_string(),
            Self::EmptyMessage => "발화할 문구가 없습니다".to_string(),
            Self::NoVoiceAvailable => "사용할 수 있는 보이스가 없습니다".to_string(),
            Self::Synth(e) => format!("{e}"),
            Self::Cache(d) => d.clone(),
        };
        format!("{}: {}", self.code(), detail)
    }
}

impl std::fmt::Display for TtsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_ipc_string())
    }
}

/// TTS 런타임 상태 — 키 스토어, mp3 캐시 디렉터리, 보이스 목록 1회 캐시.
///
/// 보이스 목록은 `Mutex<Option<..>>`이고 **락을 await 넘어 들고 가지 않는다**
/// (crate 전역 규칙). 동시 요청 둘이 각각 목록을 fetch하는 레이스는 허용한다 —
/// 결과가 결정적(정렬)이라 어느 쪽이 이겨도 배정이 같고, 비용은 앱 수명 중
/// 최대 몇 번의 GET이다. 락을 잡은 채 기다리는 것보다 이게 안전하다.
pub struct TtsState {
    pub keys: keys::TtsKeyStore,
    cache_dir: PathBuf,
    voices: Mutex<Option<Vec<voice::VoiceRef>>>,
}

impl TtsState {
    pub fn new(app_data_dir: &Path) -> Self {
        Self {
            keys: keys::TtsKeyStore::new(app_data_dir.join("tts-keys.json")),
            cache_dir: app_data_dir.join("tts-cache"),
            voices: Mutex::new(None),
        }
    }

    fn cached_voices(&self) -> Option<Vec<voice::VoiceRef>> {
        self.voices.lock().unwrap().clone()
    }

    fn store_voices(&self, v: Vec<voice::VoiceRef>) {
        *self.voices.lock().unwrap() = Some(v);
    }
}

/// 캐시 키: (voice_id, model_id, 최종 텍스트)의 sha256 앞 16바이트 hex.
/// 세 값 중 하나만 달라도 다른 파일이다 — 특히 v3/v2 폴백은 텍스트(태그 유무)와
/// model_id가 모두 달라 서로를 오염시키지 않는다. 순수.
pub fn cache_key(voice_id: &str, model_id: &str, text: &str) -> String {
    let mut h = Sha256::new();
    // 길이 구분자 없이 이어붙이면 ("ab","c") 와 ("a","bc") 가 충돌한다 → \x00 구분.
    h.update(voice_id.as_bytes());
    h.update([0u8]);
    h.update(model_id.as_bytes());
    h.update([0u8]);
    h.update(text.as_bytes());
    let d = h.finalize();
    d.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

fn cache_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.mp3"))
}

/// 파일 수 상한을 넘으면 mtime 오래된 것부터 지운다. 최선노력(실패는 무시).
fn prune_cache(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "mp3"))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().unwrap_or(std::time::UNIX_EPOCH), e.path()))
        })
        .collect();
    if files.len() <= MAX_CACHE_FILES {
        return;
    }
    files.sort_by_key(|(t, _)| *t);
    for (_, p) in files.iter().take(files.len() - MAX_CACHE_FILES) {
        let _ = std::fs::remove_file(p);
    }
}

/// 캐시 저장은 temp+rename — 동시에 같은 대사를 합성해도 반쯤 쓰인 mp3를
/// 다른 요청이 읽는 일이 없다.
fn write_cache(dir: &Path, key: &str, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{key}.mp3.tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes)?;
    if let Err(e) = std::fs::rename(&tmp, cache_path(dir, key)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    prune_cache(dir);
    Ok(())
}

/// 확인 요청 문구 하나를 캐릭터 목소리로 합성한다.
///
/// 락을 `.await` 넘어 들고 가지 않는다 — 보이스 목록은 잡았다 놓고, 나머지
/// 상태는 값으로 꺼내 쓴다.
pub async fn speak(
    state: &TtsState,
    model: TtsRewriteModel,
    provider: TtsRewriteProvider,
    req: &SpeakRequest,
) -> Result<SpeakResult, TtsError> {
    let source = req.message.trim();
    if source.is_empty() {
        return Err(TtsError::EmptyMessage);
    }
    let el_key = state
        .keys
        .elevenlabs_key()
        .ok_or(TtsError::MissingElevenLabsKey)?;

    // ── 1. 리라이트(실패는 어느 경로든 원문 강등) ─────────────────────
    let route = resolve_rewrite_route(
        provider,
        state.keys.anthropic_key(),
        claude_cli_available(),
    );
    let attempted = route.label();
    let outcome = match &route {
        RewriteRoute::Api(key) => {
            rewrite::rewrite(
                key,
                model,
                &req.agent_name,
                req.archetype.as_deref(),
                source,
            )
            .await
            .map(Some)
        }
        RewriteRoute::ClaudeCli => {
            cli::rewrite_via_cli(model, &req.agent_name, req.archetype.as_deref(), source)
                .await
                .map(Some)
        }
        RewriteRoute::None => Ok(None),
    };
    let (line, rewritten, rewrite_via) = match outcome {
        Ok(Some(l)) => (l, true, attempted),
        Ok(None) => (rewrite::sanitize_line(source), false, "none"),
        Err(e) => {
            eprintln!("tts: 대사 리라이트({attempted}) 실패 — 원문으로 발화 ({e})");
            (rewrite::sanitize_line(source), false, "none")
        }
    };
    if line.is_empty() {
        return Err(TtsError::EmptyMessage);
    }

    // ── 2. 보이스 결정적 배정 ─────────────────────────────────────────
    let voices = match state.cached_voices() {
        Some(v) => v,
        None => {
            let fetched = voice::fetch_voices(&el_key).await;
            state.store_voices(fetched.clone());
            fetched
        }
    };
    let picked = voice::pick_voice(&voices, &voice::voice_key(&req.agent_id, &req.seed))
        .ok_or(TtsError::NoVoiceAvailable)?;

    // ── 3. v3 시도(캐시 우선) → 실패 시 태그 제거 + v2 ────────────────
    let attempts = [
        (synth::MODEL_V3, line.clone()),
        (synth::MODEL_V2, synth::strip_audio_tags(&line)),
    ];
    let mut last_err: Option<synth::SynthError> = None;
    for (idx, (model_id, text)) in attempts.iter().enumerate() {
        if text.is_empty() {
            continue;
        }
        let key = cache_key(&picked.voice_id, model_id, text);
        if let Ok(bytes) = std::fs::read(cache_path(&state.cache_dir, &key)) {
            if !bytes.is_empty() {
                return Ok(SpeakResult {
                    audio_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
                    mime_type: synth::MIME_TYPE.to_string(),
                    line: text.clone(),
                    voice_id: picked.voice_id.clone(),
                    model_id: (*model_id).to_string(),
                    cached: true,
                    rewritten,
                    rewrite_via,
                });
            }
        }
        match synth::synthesize(&el_key, &picked.voice_id, text, model_id).await {
            Ok(bytes) => {
                if let Err(e) = write_cache(&state.cache_dir, &key, &bytes) {
                    // 캐시는 최적화일 뿐 — 저장 실패로 발화를 막지 않는다.
                    eprintln!("tts: 오디오 캐시 저장 실패(무해) ({e})");
                }
                return Ok(SpeakResult {
                    audio_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
                    mime_type: synth::MIME_TYPE.to_string(),
                    line: text.clone(),
                    voice_id: picked.voice_id.clone(),
                    model_id: (*model_id).to_string(),
                    cached: false,
                    rewritten,
                    rewrite_via,
                });
            }
            Err(e) => {
                // 마지막 시도이거나 폴백해도 소용없는 실패면 즉시 포기.
                let is_last = idx + 1 == attempts.len();
                if is_last || !e.should_retry_without_v3() {
                    return Err(TtsError::Synth(e));
                }
                eprintln!("tts: {model_id} 사용 불가 — 태그 제거 후 폴백 ({e})");
                last_err = Some(e);
            }
        }
    }
    Err(TtsError::Synth(last_err.unwrap_or(synth::SynthError::EmptyAudio)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_stable_and_field_separated() {
        let a = cache_key("v1", "eleven_v3", "안녕");
        assert_eq!(a, cache_key("v1", "eleven_v3", "안녕"), "결정적이어야 한다");
        assert_eq!(a.len(), 32, "16바이트 hex");
        // 어느 필드가 달라도 다른 키.
        assert_ne!(a, cache_key("v2", "eleven_v3", "안녕"));
        assert_ne!(a, cache_key("v1", "eleven_multilingual_v2", "안녕"));
        assert_ne!(a, cache_key("v1", "eleven_v3", "안녕?"));
        // 경계 없이 이어붙이면 충돌하는 조합이 갈려야 한다.
        assert_ne!(cache_key("ab", "c", "d"), cache_key("a", "bc", "d"));
    }

    #[test]
    fn v3_and_v2_attempts_never_share_a_cache_entry() {
        // 폴백은 텍스트(태그 제거)와 model_id가 둘 다 다르므로 서로의 캐시를
        // 히트해서는 안 된다 — 안 그러면 태그를 읽어버린 오디오가 재사용된다.
        let line = "[nervous] 진행할까요?";
        let stripped = synth::strip_audio_tags(line);
        assert_ne!(
            cache_key("v", synth::MODEL_V3, line),
            cache_key("v", synth::MODEL_V2, &stripped)
        );
    }

    #[test]
    fn write_then_read_roundtrips_and_leaves_no_temp() {
        let dir = std::env::temp_dir().join(format!("ao-tts-cache-{}", uuid::Uuid::new_v4()));
        let key = cache_key("v", "m", "t");
        write_cache(&dir, &key, b"ID3fake").unwrap();
        assert_eq!(std::fs::read(cache_path(&dir, &key)).unwrap(), b"ID3fake");
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(!names.iter().any(|n| n.contains(".tmp")), "{names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_keeps_cache_bounded() {
        let dir = std::env::temp_dir().join(format!("ao-tts-prune-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..MAX_CACHE_FILES + 20 {
            std::fs::write(dir.join(format!("k{i:04}.mp3")), b"x").unwrap();
        }
        prune_cache(&dir);
        let count = std::fs::read_dir(&dir).unwrap().count();
        assert!(count <= MAX_CACHE_FILES, "남은 {count}개");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ipc_error_strings_carry_a_parseable_code_prefix() {
        for e in [
            TtsError::Disabled,
            TtsError::MissingElevenLabsKey,
            TtsError::EmptyMessage,
            TtsError::NoVoiceAvailable,
            TtsError::Synth(synth::SynthError::InvalidApiKey),
        ] {
            let s = e.to_ipc_string();
            assert_eq!(s.split(':').next().unwrap(), e.code(), "{s}");
        }
    }

    // ── 리라이트 공급자 체인 ──────────────────────────────────────────
    #[test]
    fn auto_prefers_api_key_then_cli_then_skips() {
        use TtsRewriteProvider::Auto;
        assert_eq!(
            resolve_rewrite_route(Auto, Some("k".into()), true),
            RewriteRoute::Api("k".into()),
            "키가 있으면 구독 사용량을 건드리지 않는 API를 먼저"
        );
        assert_eq!(
            resolve_rewrite_route(Auto, None, true),
            RewriteRoute::ClaudeCli
        );
        assert_eq!(resolve_rewrite_route(Auto, None, false), RewriteRoute::None);
    }

    #[test]
    fn explicit_providers_do_not_silently_cross_over() {
        // api 선택인데 키가 없으면 CLI로 몰래 넘어가 구독을 소모해선 안 된다.
        assert_eq!(
            resolve_rewrite_route(TtsRewriteProvider::Api, None, true),
            RewriteRoute::None
        );
        // claude-cli 선택이면 키가 있어도 CLI를 쓴다(사용자 명시 의도).
        assert_eq!(
            resolve_rewrite_route(TtsRewriteProvider::ClaudeCli, Some("k".into()), true),
            RewriteRoute::ClaudeCli
        );
        // CLI가 없으면 생략으로 강등.
        assert_eq!(
            resolve_rewrite_route(TtsRewriteProvider::ClaudeCli, Some("k".into()), false),
            RewriteRoute::None
        );
        // none은 무조건 원문.
        assert_eq!(
            resolve_rewrite_route(TtsRewriteProvider::None, Some("k".into()), true),
            RewriteRoute::None
        );
    }

    #[test]
    fn route_labels_match_the_settings_wire_values() {
        assert_eq!(RewriteRoute::Api("k".into()).label(), "api");
        assert_eq!(
            RewriteRoute::ClaudeCli.label(),
            TtsRewriteProvider::ClaudeCli.as_str()
        );
        assert_eq!(RewriteRoute::None.label(), TtsRewriteProvider::None.as_str());
    }

    #[test]
    fn route_label_never_leaks_the_key() {
        let r = RewriteRoute::Api("sk-ant-SECRET".into());
        assert!(!r.label().contains("SECRET"));
    }

    #[test]
    fn speak_request_deserializes_camel_case_with_optional_fields() {
        let r: SpeakRequest =
            serde_json::from_str(r#"{"agentId":"a1","message":"확인이 필요합니다"}"#).unwrap();
        assert_eq!(r.agent_id, "a1");
        assert_eq!(r.agent_name, "");
        assert_eq!(r.archetype, None);
        assert_eq!(r.seed, "");
    }
}
