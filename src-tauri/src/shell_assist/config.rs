// src-tauri/src/shell_assist/config.rs
//
// 셸 캐릭터의 LLM 설정(`shell-assist.json`, 앱 데이터 폴더). API 키가 들어갈 수 있어
// 0600으로 쓴다. 앱 설정(settings.json)과 따로 두는 것은 웹 상태판 설정과 같은 이유다 —
// 이 기능만의 값이고, 키를 settings.json에 섞지 않는다.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::persistence::settings_store::SummaryProvider;

pub const CONFIG_FILE: &str = "shell-assist.json";

/// OpenAI 호환 엔드포인트 하나(Ollama·LM Studio·llama.cpp·vLLM·mlx_lm 등).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalEndpoint {
    /// 화면에 보일 이름(예: "4090 PC", "이 Mac"). 비면 주소를 쓴다.
    #[serde(default)]
    pub label: String,
    /// `http://host:11434`, `http://host:1234/v1`, `…/v1/chat/completions` 어느 꼴이든 받는다.
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub model: String,
    /// 대부분의 로컬 서버는 키가 없다. 있으면 Bearer로 보낸다.
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellAssistConfig {
    /// 위에서부터 차례로 시도한다.
    #[serde(default)]
    pub endpoints: Vec<LocalEndpoint>,
    /// 로컬이 모두 실패하면 클라우드 provider로 넘어갈지. 명령 출력이 클라우드로 가므로 기본 끔.
    #[serde(default)]
    pub fallback_enabled: bool,
    #[serde(default)]
    pub fallback_provider: SummaryProvider,
    /// 비면 그 provider의 라벨 요약 기본 모델.
    #[serde(default)]
    pub fallback_model: String,
    /// 명령이 끝날 때마다 결과 설명을 붙일지.
    #[serde(default = "yes")]
    pub explain: bool,
}

fn yes() -> bool {
    true
}

impl Default for ShellAssistConfig {
    fn default() -> Self {
        Self {
            endpoints: Vec::new(),
            fallback_enabled: false,
            fallback_provider: SummaryProvider::default(),
            fallback_model: String::new(),
            explain: true,
        }
    }
}

impl ShellAssistConfig {
    pub fn usable_endpoints(&self) -> impl Iterator<Item = &LocalEndpoint> {
        self.endpoints
            .iter()
            .filter(|e| e.enabled && !e.base_url.trim().is_empty() && !e.model.trim().is_empty())
    }

    /// 쓸 수 있는 LLM이 하나라도 있는가.
    pub fn any_llm(&self) -> bool {
        self.usable_endpoints().next().is_some() || self.fallback_enabled
    }

    /// 저장 전 다듬기: 앞뒤 공백, 완전히 빈 줄 제거, 개수 상한.
    pub fn normalized(mut self) -> Self {
        for e in &mut self.endpoints {
            e.label = e.label.trim().to_string();
            e.base_url = e.base_url.trim().to_string();
            e.model = e.model.trim().to_string();
            e.api_key = e.api_key.trim().to_string();
        }
        self.endpoints
            .retain(|e| !(e.base_url.is_empty() && e.model.is_empty() && e.label.is_empty() && e.api_key.is_empty()));
        self.endpoints.truncate(8);
        self.fallback_model = self.fallback_model.trim().to_string();
        self
    }
}

/// `http://h:11434` → `http://h:11434/v1/chat/completions`. `/v1`로 끝나면 경로만 붙이고,
/// 이미 `/chat/completions`로 끝나면 그대로 둔다.
pub fn chat_url(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if b.ends_with("/chat/completions") {
        b.to_string()
    } else if b.ends_with("/v1") {
        format!("{b}/chat/completions")
    } else {
        format!("{b}/v1/chat/completions")
    }
}

/// 같은 규칙으로 모델 목록 주소.
pub fn models_url(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    let b = b.strip_suffix("/chat/completions").unwrap_or(b);
    if b.ends_with("/v1") {
        format!("{b}/models")
    } else {
        format!("{b}/v1/models")
    }
}

pub struct ConfigStore {
    file: PathBuf,
}

impl ConfigStore {
    pub fn in_app_data(dir: &Path) -> Self {
        Self { file: dir.join(CONFIG_FILE) }
    }

    pub fn load(&self) -> ShellAssistConfig {
        std::fs::read(&self.file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, config: &ShellAssistConfig) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(config)?;
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_private(&self.file, &bytes)
    }
}

/// 0600(unix)로 고유 임시 파일에 쓰고 이름을 바꾼다.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_accept_common_base_forms() {
        assert_eq!(chat_url("http://pc:11434"), "http://pc:11434/v1/chat/completions");
        assert_eq!(chat_url("http://pc:1234/v1/"), "http://pc:1234/v1/chat/completions");
        assert_eq!(chat_url(" http://pc/v1/chat/completions "), "http://pc/v1/chat/completions");
        assert_eq!(models_url("http://pc:11434/"), "http://pc:11434/v1/models");
        assert_eq!(models_url("http://pc/v1/chat/completions"), "http://pc/v1/models");
    }

    #[test]
    fn defaults_and_partial_files_load() {
        let c: ShellAssistConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c, ShellAssistConfig::default());
        assert!(!c.fallback_enabled);
        assert!(c.explain);
        let c: ShellAssistConfig =
            serde_json::from_str(r#"{"endpoints":[{"baseUrl":"http://x","model":"m"}],"fallbackProvider":"openrouter"}"#).unwrap();
        assert!(c.endpoints[0].enabled);
        assert_eq!(c.fallback_provider, SummaryProvider::Openrouter);
        assert!(c.any_llm());
    }

    #[test]
    fn usable_endpoints_skip_disabled_and_incomplete() {
        let c = ShellAssistConfig {
            endpoints: vec![
                LocalEndpoint { base_url: "http://a".into(), model: "m".into(), enabled: false, ..Default::default() },
                LocalEndpoint { base_url: "http://b".into(), model: "".into(), enabled: true, ..Default::default() },
                LocalEndpoint { base_url: "http://c".into(), model: "m".into(), enabled: true, ..Default::default() },
            ],
            ..Default::default()
        };
        let urls: Vec<&str> = c.usable_endpoints().map(|e| e.base_url.as_str()).collect();
        assert_eq!(urls, vec!["http://c"]);
    }

    #[test]
    fn save_load_roundtrip_is_private() {
        let dir = std::env::temp_dir().join(format!("agent-office-shell-assist-{}", uuid::Uuid::new_v4()));
        let store = ConfigStore::in_app_data(&dir);
        let config = ShellAssistConfig {
            endpoints: vec![LocalEndpoint { label: "PC".into(), base_url: "http://pc:11434".into(), model: "qwen".into(), api_key: "k".into(), enabled: true }],
            fallback_enabled: true,
            ..Default::default()
        };
        store.save(&config).unwrap();
        assert_eq!(store.load(), config);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(CONFIG_FILE)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn normalized_trims_and_drops_empty_rows() {
        let c = ShellAssistConfig {
            endpoints: vec![
                LocalEndpoint { base_url: " http://a ".into(), model: " m ".into(), enabled: true, ..Default::default() },
                LocalEndpoint { enabled: true, ..Default::default() },
            ],
            ..Default::default()
        }
        .normalized();
        assert_eq!(c.endpoints.len(), 1);
        assert_eq!(c.endpoints[0].base_url, "http://a");
        assert_eq!(c.endpoints[0].model, "m");
    }
}
