// src-tauri/src/tts/rewrite.rs
//
// 시스템 알림 문구 → 캐릭터 말투 대사 리라이트. Anthropic Messages API
// (`POST /v1/messages`) 원시 HTTP다 — Rust에는 공식 Anthropic SDK가 없다.
//
// 순수 로직(build_request_body / parse_response / sanitize_line)과 HTTP(rewrite)를
// 분리한다 — pixellab.rs와 같은 관례이며, 네트워크 없이 단위 테스트할 수 있는
// 부분을 최대화한다.
//
// 실패는 전부 "원문 그대로 읽기"로 강등된다(호출측 tts::speak). 키가 없어도,
// 타임아웃이어도, 모델이 거절해도 캐릭터는 무언가를 말한다.
//
// 요청 형태에서 주의할 점 세 가지:
//  1) `temperature`/`top_p`/`top_k`를 **보내지 않는다**. claude-sonnet-5·
//     claude-opus-5는 이 파라미터를 받으면 400이다(설정에서 모델을 고를 수
//     있으므로 어떤 선택에서도 안전한 형태로 고정한다).
//  2) `thinking`을 **보내지 않는다**. 대신 max_tokens를 넉넉히 준다 —
//     claude-opus-5는 thinking이 기본 ON이고 max_tokens가 thinking+본문을 함께
//     캡하므로, 300 같은 값을 주면 사고에 다 먹혀 본문이 빈 채로 잘린다.
//  3) 응답의 `text` 블록만 이어붙인다(thinking 블록은 건너뛴다).

use crate::persistence::settings_store::TtsRewriteModel;

pub const BASE_URL: &str = "https://api.anthropic.com/v1/messages";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// 리라이트는 인터랙티브 경로다 — 이 시간을 넘기면 원문으로 강등한다.
pub const TIMEOUT_SECS: u64 = 6;
/// thinking이 기본 ON인 모델(claude-opus-5)도 한 줄 대사를 온전히 낼 수 있는 여유.
pub const MAX_TOKENS: u32 = 1024;
/// 대사 길이 상한(문자 수). 오디오 태그를 포함한 최종 텍스트 기준.
pub const MAX_LINE_CHARS: usize = 120;
/// 원문이 아무리 길어도 프롬프트에 이만큼만 싣는다(훅 문구는 짧지만 방어적).
const MAX_SOURCE_CHARS: usize = 1000;

pub const SYSTEM_PROMPT: &str = "너는 픽셀 오피스 게임 캐릭터의 대사 작가다. \
AI 코딩 에이전트가 사용자 확인을 기다리며 낸 시스템 알림 문구를, 주어진 캐릭터(이름·archetype)의 \
말투로 된 짧은 한국어 대사 한 줄로 바꿔라.

규칙:
- 120자 이내의 한 줄. 줄바꿈 금지.
- 무엇을 확인해 달라는지 핵심 의미는 반드시 유지한다.
- ElevenLabs v3 오디오 태그([nervous], [excited], [whispers], [sighs], [curious] 등)를 \
대사 안에 0~2개 넣어 감정을 지시한다.
- 대사만 출력한다. 따옴표, 설명, 머리말, 캐릭터 이름 접두사를 붙이지 않는다.";

/// 리라이트 실패 사유. 호출측은 코드만 로그에 남기고 원문으로 강등한다.
#[derive(Debug, Clone, PartialEq)]
pub enum RewriteError {
    /// 키 미설정(저장값·env 모두 없음).
    MissingApiKey,
    /// 안전 분류기 거절(`stop_reason: "refusal"`) — 재시도해도 같다.
    Refused,
    /// 응답에 텍스트가 없음(전부 thinking에 먹혔거나 빈 응답).
    EmptyOutput,
    Http(String),
    Network(String),
}

impl RewriteError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingApiKey => "missing_api_key",
            Self::Refused => "refusal",
            Self::EmptyOutput => "empty_output",
            Self::Http(_) => "http",
            Self::Network(_) => "network",
        }
    }
}

impl std::fmt::Display for RewriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(d) | Self::Network(d) => write!(f, "{}: {d}", self.code()),
            _ => write!(f, "{}", self.code()),
        }
    }
}

/// 캐릭터 정보 + 원문을 담은 user 메시지. 원문은 태그 오염을 막으려고
/// 구분자로 감싸 넘긴다.
pub fn build_user_content(agent_name: &str, archetype: Option<&str>, message: &str) -> String {
    let name = if agent_name.trim().is_empty() {
        "이름 없음"
    } else {
        agent_name.trim()
    };
    let arch = archetype
        .map(|a| a.trim())
        .filter(|a| !a.is_empty() && *a != "auto")
        .unwrap_or("human");
    let src: String = message.chars().take(MAX_SOURCE_CHARS).collect();
    format!(
        "캐릭터 이름: {name}\n캐릭터 archetype: {arch}\n\n원문 알림 문구:\n<notice>\n{src}\n</notice>"
    )
}

pub fn build_request_body(model: TtsRewriteModel, user_content: &str) -> serde_json::Value {
    serde_json::json!({
        "model": model.as_str(),
        "max_tokens": MAX_TOKENS,
        "system": SYSTEM_PROMPT,
        "messages": [{ "role": "user", "content": user_content }],
    })
}

/// HTTP 상태 + body → 대사/에러. 순수.
pub fn parse_response(status: u16, body: &str) -> Result<String, RewriteError> {
    if status != 200 {
        // 키 값이 에러 문자열에 실릴 여지가 없도록 body는 싣지 않고 상태만 남긴다.
        return Err(RewriteError::Http(format!("HTTP {status}")));
    }
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| RewriteError::Http(format!("invalid JSON: {e}")))?;
    if v.get("stop_reason").and_then(|s| s.as_str()) == Some("refusal") {
        return Err(RewriteError::Refused);
    }
    // thinking 블록은 건너뛰고 text 블록만 이어붙인다.
    let text: String = v
        .get("content")
        .and_then(|c| c.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    let line = sanitize_line(&text);
    if line.is_empty() {
        return Err(RewriteError::EmptyOutput);
    }
    Ok(line)
}

/// 모델 출력을 한 줄 대사로 정규화한다: 개행 → 공백, 겉따옴표 제거,
/// `MAX_LINE_CHARS` 절단. 프롬프트로 지시했더라도 모델은 가끔 어기므로
/// 코드가 최종 게이트다.
pub fn sanitize_line(raw: &str) -> String {
    // 개행/탭을 공백으로 접고 연속 공백을 하나로.
    let flat: String = raw
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    let mut s = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    // 모델이 대사 전체를 따옴표로 감싼 경우만 벗긴다(내부 인용은 보존).
    for (open, close) in [('"', '"'), ('\'', '\''), ('“', '”'), ('「', '」')] {
        if s.chars().count() >= 2 && s.starts_with(open) && s.ends_with(close) {
            s = s
                .chars()
                .skip(1)
                .take(s.chars().count() - 2)
                .collect::<String>()
                .trim()
                .to_string();
        }
    }
    if s.chars().count() > MAX_LINE_CHARS {
        s = s.chars().take(MAX_LINE_CHARS).collect::<String>();
        s = s.trim_end().to_string();
    }
    s
}

/// 얇은 HTTP 래퍼 — 이 함수만 네트워크를 만진다. 키 값은 에러에 싣지 않는다.
pub async fn rewrite(
    api_key: &str,
    model: TtsRewriteModel,
    agent_name: &str,
    archetype: Option<&str>,
    message: &str,
) -> Result<String, RewriteError> {
    if api_key.trim().is_empty() {
        return Err(RewriteError::MissingApiKey);
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
        .build()
        .map_err(|e| RewriteError::Network(e.to_string()))?;
    let body = build_request_body(model, &build_user_content(agent_name, archetype, message));
    let resp = client
        .post(BASE_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| RewriteError::Network(e.to_string()))?;
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| RewriteError::Network(e.to_string()))?;
    parse_response(status, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_omits_sampling_and_thinking_params() {
        // sonnet-5/opus-5는 temperature류를 받으면 400이고, thinking은 과제
        // 지시대로 아예 보내지 않는다.
        let b = build_request_body(TtsRewriteModel::Opus5, "hi");
        let obj = b.as_object().unwrap();
        for forbidden in ["temperature", "top_p", "top_k", "thinking"] {
            assert!(!obj.contains_key(forbidden), "{forbidden} 를 보내면 안 된다");
        }
        assert_eq!(obj["model"], "claude-opus-5");
        assert_eq!(obj["max_tokens"], MAX_TOKENS);
        assert_eq!(obj["messages"][0]["role"], "user");
    }

    #[test]
    fn user_content_carries_character_and_source() {
        let c = build_user_content("무지", Some("cat"), "확인이 필요합니다");
        assert!(c.contains("무지"));
        assert!(c.contains("cat"));
        assert!(c.contains("확인이 필요합니다"));
    }

    #[test]
    fn user_content_falls_back_for_blank_name_and_auto_archetype() {
        let c = build_user_content("  ", Some("auto"), "m");
        assert!(c.contains("이름 없음"));
        assert!(c.contains("human"), "auto는 확정 전 값이라 human으로 취급");
        let c2 = build_user_content("A", None, "m");
        assert!(c2.contains("human"));
    }

    #[test]
    fn parse_joins_text_blocks_and_skips_thinking() {
        let body = r#"{"stop_reason":"end_turn","content":[
            {"type":"thinking","thinking":"고민..."},
            {"type":"text","text":"[nervous] 이거 "},
            {"type":"text","text":"진행해도 될까요?"}]}"#;
        assert_eq!(
            parse_response(200, body).unwrap(),
            "[nervous] 이거 진행해도 될까요?"
        );
    }

    #[test]
    fn parse_refusal_and_empty_and_http_errors() {
        assert_eq!(
            parse_response(200, r#"{"stop_reason":"refusal","content":[]}"#),
            Err(RewriteError::Refused)
        );
        // thinking에 max_tokens를 다 먹힌 형태 = 텍스트 없음.
        assert_eq!(
            parse_response(
                200,
                r#"{"stop_reason":"max_tokens","content":[{"type":"thinking","thinking":"..."}]}"#
            ),
            Err(RewriteError::EmptyOutput)
        );
        assert!(matches!(
            parse_response(401, "{}"),
            Err(RewriteError::Http(_))
        ));
    }

    #[test]
    fn http_error_does_not_leak_body() {
        // body에 키가 반사돼 있어도 에러 문자열에 실리지 않아야 한다.
        let err = parse_response(400, r#"{"error":{"message":"bad key sk-ant-SECRET"}}"#);
        let s = format!("{}", err.unwrap_err());
        assert!(!s.contains("SECRET"), "{s}");
    }

    #[test]
    fn sanitize_flattens_newlines_and_strips_wrapping_quotes() {
        assert_eq!(sanitize_line("  \"[excited] 좋아요!\"  "), "[excited] 좋아요!");
        assert_eq!(sanitize_line("첫 줄\n둘째 줄"), "첫 줄 둘째 줄");
        assert_eq!(sanitize_line("“따옴표”"), "따옴표");
        // 내부 인용은 보존.
        assert_eq!(sanitize_line("이 \"파일\" 지울까요?"), "이 \"파일\" 지울까요?");
    }

    #[test]
    fn sanitize_truncates_by_chars_not_bytes() {
        let long = "가".repeat(200);
        let out = sanitize_line(&long);
        assert_eq!(out.chars().count(), MAX_LINE_CHARS);
    }

    #[test]
    fn model_ids_are_the_documented_strings() {
        assert_eq!(TtsRewriteModel::Haiku45.as_str(), "claude-haiku-4-5");
        assert_eq!(TtsRewriteModel::Sonnet5.as_str(), "claude-sonnet-5");
        assert_eq!(TtsRewriteModel::Opus5.as_str(), "claude-opus-5");
    }
}
