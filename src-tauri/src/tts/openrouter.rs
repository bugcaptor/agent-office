// src-tauri/src/tts/openrouter.rs
//
// 대사 리라이트의 세 번째 경로: OpenRouter. OpenAI 호환
// `POST /api/v1/chat/completions`에 `Authorization: Bearer <키>`로 부른다 —
// 한 키로 여러 벤더 모델을 고를 수 있어서, 모델 선택을 Anthropic 3종에
// 묶어 두지 않으려는 것이 이 경로의 존재 이유다.
//
// rewrite.rs(Anthropic)와 같은 규칙을 따른다:
//  - 순수 로직(build_request_body / parse_response)과 HTTP(rewrite)를 분리한다.
//  - 실패는 전부 호출측(`tts::speak`)에서 "원문 그대로 읽기"로 강등된다.
//  - `temperature`/`top_p` 같은 샘플링 파라미터를 보내지 않는다 — 어느 모델을
//    골라도 400이 나지 않는 최소 형태로 고정한다(모델이 자유 입력이라 앱은
//    무엇이 올지 모른다).
//  - 에러 문자열에 응답 body를 싣지 않는다(키가 반사될 여지를 없앤다).
//
// 시스템 프롬프트는 Anthropic 경로와 **같은 것**을 쓴다(`rewrite::system_prompt`).
// 경로에 따라 어조가 달라지면 사용자에게는 그냥 버그로 보인다.

use super::rewrite::{sanitize_line, system_prompt, RewriteError, SpeakKind, MAX_TOKENS};

pub const BASE_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
/// OpenRouter는 요청을 다시 각 벤더로 중계하므로 Anthropic 직통(6초)보다
/// 느릴 수 있다 — claude CLI 경로와 같은 20초를 준다. 넘기면 원문 발화로 강등.
pub const TIMEOUT_SECS: u64 = 20;

/// system + user 두 메시지의 OpenAI 호환 본문. 순수.
pub fn build_request_body(kind: SpeakKind, model: &str, user_content: &str) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "messages": [
            { "role": "system", "content": system_prompt(kind) },
            { "role": "user", "content": user_content },
        ],
    })
}

/// HTTP 상태 + body → 대사/에러. 순수.
///
/// OpenRouter는 상류 벤더 오류를 **200 + `error` 필드**로 돌려주기도 한다 —
/// 상태 코드만 보면 그것을 "빈 응답"으로 오인하므로 둘 다 본다.
pub fn parse_response(status: u16, body: &str) -> Result<String, RewriteError> {
    if status != 200 {
        return Err(RewriteError::Http(format!("HTTP {status}")));
    }
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| RewriteError::Http(format!("invalid JSON: {e}")))?;
    if let Some(err) = v.get("error") {
        // 코드만 남긴다 — message에는 통제 불가한 상류 텍스트가 실린다.
        let code = err
            .get("code")
            .map(|c| c.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        return Err(RewriteError::Http(format!("provider error {code}")));
    }
    let choice = v.get("choices").and_then(|c| c.as_array()).and_then(|a| a.first());
    // 안전 거절은 finish_reason으로 온다(벤더마다 표기가 갈려 두 가지를 본다).
    if let Some(reason) = choice
        .and_then(|c| c.get("finish_reason"))
        .and_then(|r| r.as_str())
    {
        if reason == "content_filter" || reason == "refusal" {
            return Err(RewriteError::Refused);
        }
    }
    let text = choice
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|t| t.as_str())
        .unwrap_or_default();
    let line = sanitize_line(text);
    if line.is_empty() {
        // 추론 모델이 예산을 전부 reasoning에 쓴 경우도 여기로 온다.
        return Err(RewriteError::EmptyOutput);
    }
    Ok(line)
}

/// 얇은 HTTP 래퍼 — 이 함수만 네트워크를 만진다. 키 값은 에러에 싣지 않는다.
pub async fn rewrite(
    api_key: &str,
    kind: SpeakKind,
    model: &str,
    agent_name: &str,
    personality: Option<&str>,
    context: Option<&str>,
    message: &str,
) -> Result<String, RewriteError> {
    if api_key.trim().is_empty() {
        return Err(RewriteError::MissingApiKey);
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
        .build()
        .map_err(|e| RewriteError::Network(e.to_string()))?;
    let body = build_request_body(
        kind,
        model,
        &super::rewrite::build_user_content(kind, agent_name, personality, context, message),
    );
    let resp = client
        .post(BASE_URL)
        .header("authorization", format!("Bearer {api_key}"))
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
    fn body_is_openai_shaped_with_system_and_user_messages() {
        let b = build_request_body(SpeakKind::Question, "openai/gpt-5.4-mini", "hi");
        let obj = b.as_object().unwrap();
        assert_eq!(obj["model"], "openai/gpt-5.4-mini");
        assert_eq!(obj["max_tokens"], MAX_TOKENS);
        assert_eq!(obj["messages"][0]["role"], "system");
        assert_eq!(obj["messages"][0]["content"], system_prompt(SpeakKind::Question));
        assert_eq!(obj["messages"][1]["role"], "user");
        assert_eq!(obj["messages"][1]["content"], "hi");
        // 어떤 모델을 골라도 400이 나지 않도록 샘플링 파라미터는 보내지 않는다.
        for forbidden in ["temperature", "top_p", "top_k", "thinking", "reasoning"] {
            assert!(!obj.contains_key(forbidden), "{forbidden} 를 보내면 안 된다");
        }
    }

    // 완료 보고는 Anthropic 경로와 같은 프롬프트를 써야 한다.
    #[test]
    fn done_kind_swaps_the_system_prompt_like_the_anthropic_path() {
        let q = build_request_body(SpeakKind::Question, "m", "c");
        let d = build_request_body(SpeakKind::Done, "m", "c");
        assert_ne!(q["messages"][0]["content"], d["messages"][0]["content"]);
        assert_eq!(d["messages"][0]["content"], system_prompt(SpeakKind::Done));
    }

    #[test]
    fn parse_reads_the_first_choice_message_content() {
        let body = r#"{"choices":[{"finish_reason":"stop",
            "message":{"role":"assistant","content":"  [nervous] 진행해도 될까요?\n"}}]}"#;
        assert_eq!(
            parse_response(200, body).unwrap(),
            "[nervous] 진행해도 될까요?"
        );
    }

    #[test]
    fn parse_maps_filtered_and_empty_and_http_errors() {
        assert_eq!(
            parse_response(
                200,
                r#"{"choices":[{"finish_reason":"content_filter","message":{"content":""}}]}"#
            ),
            Err(RewriteError::Refused)
        );
        assert_eq!(
            parse_response(200, r#"{"choices":[{"message":{"content":""}}]}"#),
            Err(RewriteError::EmptyOutput)
        );
        assert_eq!(
            parse_response(200, r#"{"choices":[]}"#),
            Err(RewriteError::EmptyOutput)
        );
        assert!(matches!(
            parse_response(401, "{}"),
            Err(RewriteError::Http(_))
        ));
    }

    // 상류 오류는 200으로도 온다 — 그걸 "빈 응답"으로 뭉개면 원인을 못 남긴다.
    #[test]
    fn parse_detects_provider_error_carried_in_a_200_response() {
        let err = parse_response(200, r#"{"error":{"code":429,"message":"rate limited"}}"#);
        assert_eq!(err, Err(RewriteError::Http("provider error 429".into())));
    }

    #[test]
    fn errors_never_leak_the_response_body() {
        // body에 키가 반사돼 있어도 에러 문자열에 실리지 않아야 한다.
        for (status, body) in [
            (400u16, r#"{"error":{"code":"bad","message":"key sk-or-SECRET"}}"#),
            (200u16, r#"{"error":{"code":"bad","message":"key sk-or-SECRET"}}"#),
        ] {
            let s = format!("{}", parse_response(status, body).unwrap_err());
            assert!(!s.contains("SECRET"), "{s}");
        }
    }

    #[tokio::test]
    async fn blank_key_fails_before_touching_the_network() {
        assert_eq!(
            rewrite("   ", SpeakKind::Question, "m", "무지", None, None, "확인").await,
            Err(RewriteError::MissingApiKey)
        );
    }
}
