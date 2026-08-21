// src-tauri/src/summarizer/openrouter.rs
//
// 요약기의 다섯 번째 경로이자 유일한 비(非)CLI 경로: OpenRouter.
// OpenAI 호환 `POST /api/v1/chat/completions`를 `Authorization: Bearer <키>`로
// 부른다 — CLI를 깔지 않은 사용자도 요약을 쓸 수 있고, 한 키로 여러 벤더 모델을
// 고를 수 있는 것이 이 경로의 존재 이유다.
//
// tts/openrouter.rs와 같은 규칙을 따른다:
//  - 순수 로직(build_request_body / parse_response)과 HTTP(summarize)를 분리한다.
//  - `temperature`/`top_p` 같은 샘플링 파라미터를 보내지 않는다 — 모델이 자유
//    입력이라 앱은 무엇이 올지 모른다. 어느 모델을 골라도 400이 나지 않는
//    최소 형태로 고정한다.
//  - 에러 문자열에 응답 body를 싣지 않는다(키가 반사될 여지를 없앤다).
//  - 실패는 전부 호출측에서 원문 폴백으로 강등된다(기존 CLI 실패 경로와 동일).
//
// 지시문(instruction)은 CLI 경로에서 system prompt 자리에 들어가던 것과 **같은
// 문자열**을 쓴다 — 경로에 따라 요약 어조가 달라지면 사용자에게는 버그로 보인다.

use super::SummaryPurpose;

pub const BASE_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

/// 라벨·일기는 한 문단짜리 출력이라 넉넉히 잡아도 이 정도면 충분하다.
const MAX_TOKENS_LIGHT: u32 = 1_024;
/// 학습자료는 문서 한 편(여러 절 + 코드 블록)이라 자릿수가 다르다.
const MAX_TOKENS_HEAVY: u32 = 16_384;

/// 목적별 출력 상한. 순수.
pub fn max_tokens(purpose: SummaryPurpose) -> u32 {
    match purpose {
        SummaryPurpose::Label | SummaryPurpose::Diary => MAX_TOKENS_LIGHT,
        SummaryPurpose::Study => MAX_TOKENS_HEAVY,
    }
}

/// system(지시문) + user(본문) 두 메시지의 OpenAI 호환 본문. 순수.
pub fn build_request_body(
    purpose: SummaryPurpose,
    model: &str,
    instruction: &str,
    text: &str,
) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "max_tokens": max_tokens(purpose),
        "messages": [
            { "role": "system", "content": instruction },
            { "role": "user", "content": text },
        ],
    })
}

/// HTTP 상태 + body → 요약문/에러. 순수.
///
/// OpenRouter는 상류 벤더 오류를 **200 + `error` 필드**로 돌려주기도 한다 —
/// 상태 코드만 보면 그것을 "빈 응답"으로 오인하므로 둘 다 본다.
pub fn parse_response(status: u16, body: &str) -> Result<String, String> {
    if status != 200 {
        return Err(format!("openrouter http {status}"));
    }
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("openrouter invalid JSON: {}", super::bounded_detail(&e.to_string())))?;
    if let Some(err) = v.get("error") {
        // 코드만 남긴다 — message에는 통제 불가한 상류 텍스트가 실린다.
        let code = err
            .get("code")
            .map(|c| c.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        return Err(format!("openrouter provider error {code}"));
    }
    let choice = v
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first());
    let text = choice
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|t| t.as_str())
        .unwrap_or_default()
        .trim();
    if text.is_empty() {
        // 추론 모델이 max_tokens를 전부 reasoning에 쓴 경우도 여기로 온다.
        return Err("empty output".to_string());
    }
    Ok(text.to_string())
}

/// 얇은 HTTP 래퍼 — 이 함수만 네트워크를 만진다. 키 값은 에러에 싣지 않는다.
///
/// `timeout`은 호출측(`summarizer::summarize`)이 목적별 값을 그대로 넘긴다 —
/// CLI 경로의 `run_with_timeout`과 같은 예산을 쓴다.
pub async fn summarize(
    api_key: &str,
    purpose: SummaryPurpose,
    model: &str,
    instruction: &str,
    text: &str,
    timeout: std::time::Duration,
) -> Result<String, String> {
    if api_key.trim().is_empty() {
        return Err(KEY_MISSING.to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("openrouter client: {}", super::bounded_detail(&e.to_string())))?;
    let body = build_request_body(purpose, model, instruction, text);
    let resp = client
        .post(BASE_URL)
        .header("authorization", format!("Bearer {api_key}"))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("openrouter network: {}", super::bounded_detail(&e.to_string())))?;
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("openrouter network: {}", super::bounded_detail(&e.to_string())))?;
    parse_response(status, &text)
}

/// 키가 없을 때의 안정 에러 문자열. CLI 경로의 `<provider>-not-found`와 같은
/// 자리를 차지한다 — 렌더러는 이것을 실패로 보고 원문 폴백으로 강등한다.
pub const KEY_MISSING: &str = "openrouter-key-missing";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_is_openai_shaped_with_instruction_as_system_message() {
        let b = build_request_body(
            SummaryPurpose::Label,
            "openai/gpt-5.4-mini",
            "한 줄로 요약하라",
            "작업 로그 본문",
        );
        let obj = b.as_object().unwrap();
        assert_eq!(obj["model"], "openai/gpt-5.4-mini");
        assert_eq!(obj["messages"][0]["role"], "system");
        assert_eq!(obj["messages"][0]["content"], "한 줄로 요약하라");
        assert_eq!(obj["messages"][1]["role"], "user");
        assert_eq!(obj["messages"][1]["content"], "작업 로그 본문");
        // 어떤 모델을 골라도 400이 나지 않도록 샘플링 파라미터는 보내지 않는다.
        for forbidden in ["temperature", "top_p", "top_k", "thinking", "reasoning"] {
            assert!(!obj.contains_key(forbidden), "{forbidden} 를 보내면 안 된다");
        }
    }

    // 학습자료는 문서 한 편이라 라벨·일기와 같은 예산으로는 중간에 잘린다.
    #[test]
    fn study_gets_a_much_larger_output_budget_than_label_and_diary() {
        assert_eq!(max_tokens(SummaryPurpose::Label), MAX_TOKENS_LIGHT);
        assert_eq!(max_tokens(SummaryPurpose::Diary), MAX_TOKENS_LIGHT);
        assert!(max_tokens(SummaryPurpose::Study) > MAX_TOKENS_LIGHT * 4);
        let heavy = build_request_body(SummaryPurpose::Study, "m", "i", "t");
        assert_eq!(heavy["max_tokens"], MAX_TOKENS_HEAVY);
    }

    #[test]
    fn parse_reads_the_first_choice_message_content_trimmed() {
        let body = r#"{"choices":[{"finish_reason":"stop",
            "message":{"role":"assistant","content":"  테스트 고치는 중\n"}}]}"#;
        assert_eq!(parse_response(200, body).unwrap(), "테스트 고치는 중");
    }

    #[test]
    fn parse_maps_empty_and_http_errors() {
        assert_eq!(
            parse_response(200, r#"{"choices":[{"message":{"content":"  "}}]}"#),
            Err("empty output".to_string())
        );
        assert_eq!(
            parse_response(200, r#"{"choices":[]}"#),
            Err("empty output".to_string())
        );
        assert_eq!(parse_response(401, "{}"), Err("openrouter http 401".to_string()));
    }

    // 상류 오류는 200으로도 온다 — 그걸 "빈 응답"으로 뭉개면 원인을 못 남긴다.
    #[test]
    fn parse_detects_provider_error_carried_in_a_200_response() {
        assert_eq!(
            parse_response(200, r#"{"error":{"code":429,"message":"rate limited"}}"#),
            Err("openrouter provider error 429".to_string())
        );
    }

    #[test]
    fn errors_never_leak_the_response_body() {
        // body에 키가 반사돼 있어도 에러 문자열에 실리지 않아야 한다.
        for (status, body) in [
            (400u16, r#"{"error":{"code":"bad","message":"key sk-or-SECRET"}}"#),
            (200u16, r#"{"error":{"code":"bad","message":"key sk-or-SECRET"}}"#),
            (200u16, r#"{"choices":[{"message":{"content":"sk-or-SECRET"#),
        ] {
            let e = parse_response(status, body).unwrap_err();
            assert!(!e.contains("SECRET"), "{e}");
        }
    }

    #[tokio::test]
    async fn blank_key_fails_before_touching_the_network() {
        let e = summarize(
            "   ",
            SummaryPurpose::Label,
            "m",
            "i",
            "t",
            std::time::Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert_eq!(e, KEY_MISSING);
    }
}
