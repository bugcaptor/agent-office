// src-tauri/src/summarizer/model_catalog.rs
//
// 설정 화면의 "provider별 모델 목록" 조회 진입점. 기존 OpenRouter 전용
// `openrouter_list_models` 커맨드를 일곱 개 provider로 일반화한
// `list_provider_models`(ipc::commands::media)가 이 모듈에 위임한다.
//
// 이제 일곱 provider 문자열이 모두 라이브 소스를 갖는다:
//   openrouter  공개 GET 카탈로그
//   claude/anthropic  Anthropic `/v1/models`(키 필요)
//   opencode    `opencode models`
//   codex       `codex debug models`(JSON)
//   agy         `agy models`(TSV)
//   gemini      Generative Language API(환경변수 키가 있을 때만)
// 조회 실패·키 없음·알 수 없는 문자열은 오류가 아니라 **빈 목록**이다 —
// 프런트(src/renderer/settings/modelCatalog.ts와 같은 패턴)는 실패든 빈
// 배열이든 정적 프리셋으로 조용히 강등하므로, 여기서 굳이 실패를 구분해
// 돌려줄 이유가 없다. 그래서 이 모듈의 공개 함수는 절대 `Err`를 내지 않는다 —
// 실패는 전부 호출 지점에서 빈 목록으로 눌러 담는다.

use std::time::Duration;

use super::{agy, anthropic, codex, gemini, opencode};

/// 설정 화면이 보낼 수 있는 provider 문자열. 파싱을 순수 함수로 분리해
/// 테스트한다 — 알 수 없는 문자열은 `None`으로 떨어뜨린다(오류가 아니다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Claude,
    Codex,
    Agy,
    Gemini,
    Opencode,
    Openrouter,
    Anthropic,
}

impl Provider {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "agy" => Some(Self::Agy),
            "gemini" => Some(Self::Gemini),
            "opencode" => Some(Self::Opencode),
            "openrouter" => Some(Self::Openrouter),
            "anthropic" => Some(Self::Anthropic),
            _ => None,
        }
    }
}

/// 설정 화면이 사람을 기다리게 하는 조회라 요약 타임아웃보다 짧게 잡는다 —
/// 기존 `list_openrouter_models`와 같은 예산.
const TIMEOUT_MODELS: Duration = Duration::from_secs(10);

/// `list_provider_models` 커맨드의 실제 구현. `anthropic_key`는 호출측(IPC
/// 커맨드)이 키 스토어에서 값으로 떠서 넘긴다 — 이 모듈은 `AppState`도 락도
/// 모른다. `claude`/`anthropic` 두 provider 문자열이 같은 키를 공유한다.
pub async fn list(provider: &str, anthropic_key: Option<&str>) -> Vec<String> {
    let Some(p) = Provider::parse(provider) else {
        return Vec::new();
    };
    match p {
        // OpenRouter는 기존 경로(공개 GET, 키 불필요) 그대로 재사용한다.
        Provider::Openrouter => super::list_openrouter_models().await.unwrap_or_default(),
        Provider::Claude | Provider::Anthropic => {
            let key = match anthropic_key.map(str::trim) {
                Some(k) if !k.is_empty() => k,
                // 키가 없으면 네트워크를 아예 타지 않는다 — 스펙 요구사항.
                _ => return Vec::new(),
            };
            anthropic::list_models(key, TIMEOUT_MODELS)
                .await
                .unwrap_or_default()
        }
        Provider::Opencode => opencode::list_models(TIMEOUT_MODELS).await,
        Provider::Codex => codex::list_models(TIMEOUT_MODELS).await,
        Provider::Agy => agy::list_models(TIMEOUT_MODELS).await,
        // 환경변수 키가 없으면 네트워크를 타지 않고 빈 목록이다(gemini.rs 주석).
        Provider::Gemini => gemini::list_models(TIMEOUT_MODELS).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_recognizes_all_seven_provider_strings() {
        for s in [
            "claude",
            "codex",
            "agy",
            "gemini",
            "opencode",
            "openrouter",
            "anthropic",
        ] {
            assert!(Provider::parse(s).is_some(), "{s} 는 인식돼야 한다");
        }
    }

    #[test]
    fn parse_rejects_unknown_strings() {
        for s in ["", "Claude", " claude", "claude ", "unknown", "chatgpt"] {
            assert_eq!(Provider::parse(s), None, "{s:?} 는 거부돼야 한다");
        }
    }

    #[tokio::test]
    async fn unsupported_provider_string_yields_empty_list_not_an_error() {
        assert!(list("not-a-provider", Some("sk-ant-whatever"))
            .await
            .is_empty());
    }

    /// gemini는 환경변수 키가 있을 때만 라이브 조회를 한다 — 키 이름이
    /// 조용히 바뀌면 조회가 죽은 채로 통과하므로 여기서 고정한다.
    #[test]
    fn gemini_live_source_is_gated_on_env_api_keys() {
        assert_eq!(gemini::API_KEY_ENVS, ["GEMINI_API_KEY", "GOOGLE_API_KEY"]);
    }

    #[tokio::test]
    async fn claude_and_anthropic_skip_the_network_when_key_is_missing_or_blank() {
        // 키가 없거나 공백뿐이면 네트워크를 타지 않고 즉시 빈 목록이어야
        // 한다 — 느려서가 아니라 스펙(키 없음 = 반드시 빈 목록)을 고정한다.
        for provider in ["claude", "anthropic"] {
            assert!(list(provider, None).await.is_empty(), "{provider}");
            assert!(list(provider, Some("   ")).await.is_empty(), "{provider}");
        }
    }
}
