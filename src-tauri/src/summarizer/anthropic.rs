// src-tauri/src/summarizer/anthropic.rs
//
// 설정 화면의 Anthropic 모델 카탈로그(`GET /v1/models`) 조회. tts/rewrite.rs가
// 쓰는 `POST /v1/messages`와 인증 방식(x-api-key + anthropic-version)은
// 같지만 용도가 다르므로(요약기 provider 카탈로그) 별도 파일로 둔다.
// openrouter.rs의 `parse_models_response`/`list_models`와 같은 모양(순수
// parse + 얇은 HTTP 래퍼)을 따른다.
//
// OpenRouter 카탈로그와 달리 이 엔드포인트는 키가 필수다(공개 GET이
// 아니다) — 키가 없을 때 아예 부르지 않는 판단은 호출측(model_catalog)이
// 진다. 여기서는 "호출됐다면 유효한 키가 왔다"고 가정한다.
//
// 키 값은 로그·에러 문자열에 절대 싣지 않는다(tts/keys.rs와 같은 규칙).

pub const MODELS_URL: &str = "https://api.anthropic.com/v1/models?limit=1000";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// 모델 카탈로그 응답 → 모델 id 목록(정렬·중복 제거). 순수.
///
/// OpenRouter와 달리 빈 `data`를 에러로 취급하지 않는다 — 호출측이 이미
/// `Result`를 `unwrap_or_default()`로 빈 목록에 합류시키므로 여기서 굳이
/// 구분할 이유가 없고, "키는 있는데 모델이 0개"도 있을 수 있는 정상 응답이다.
pub fn parse_models_response(status: u16, body: &str) -> Result<Vec<String>, String> {
    if status != 200 {
        return Err(format!("anthropic http {status}"));
    }
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| {
        format!(
            "anthropic invalid JSON: {}",
            super::bounded_detail(&e.to_string())
        )
    })?;
    let mut ids: Vec<String> = v
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|i| i.as_str()))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// 모델 카탈로그 조회. 이 함수만 네트워크를 만진다.
pub async fn list_models(
    api_key: &str,
    timeout: std::time::Duration,
) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| {
            format!(
                "anthropic client: {}",
                super::bounded_detail(&e.to_string())
            )
        })?;
    let resp = client
        .get(MODELS_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .send()
        .await
        .map_err(|e| {
            format!(
                "anthropic network: {}",
                super::bounded_detail(&e.to_string())
            )
        })?;
    let status = resp.status().as_u16();
    let text = resp.text().await.map_err(|e| {
        format!(
            "anthropic network: {}",
            super::bounded_detail(&e.to_string())
        )
    })?;
    parse_models_response(status, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_sorted_deduped_ids() {
        let body = r#"{"data":[
            {"id":"claude-opus-5","display_name":"Opus"},
            {"id":"claude-haiku-4.5"},
            {"id":"claude-haiku-4.5"},
            {"id":"  "},
            {"display_name":"id 없는 항목"}
        ]}"#;
        assert_eq!(
            parse_models_response(200, body).unwrap(),
            vec!["claude-haiku-4.5", "claude-opus-5"]
        );
    }

    #[test]
    fn parse_treats_empty_data_as_success_with_empty_list() {
        assert_eq!(parse_models_response(200, r#"{"data":[]}"#), Ok(Vec::new()));
        assert_eq!(parse_models_response(200, "{}"), Ok(Vec::new()));
    }

    #[test]
    fn parse_maps_http_and_json_errors() {
        assert_eq!(
            parse_models_response(401, "{}"),
            Err("anthropic http 401".to_string())
        );
        assert!(parse_models_response(200, "<html>nope</html>")
            .unwrap_err()
            .starts_with("anthropic invalid JSON"));
    }

    #[test]
    fn errors_never_leak_the_response_body() {
        for (status, body) in [
            (
                401u16,
                r#"{"error":{"message":"key sk-ant-SECRET is invalid"}}"#,
            ),
            (200u16, "sk-ant-SECRET 아닌 JSON"),
        ] {
            let e = parse_models_response(status, body).unwrap_err();
            assert!(!e.contains("SECRET"), "{e}");
        }
    }

    #[tokio::test]
    async fn blank_key_still_reaches_the_network_layer_as_is() {
        // 빈 키를 걸러내는 판단은 호출측(model_catalog)의 몫이다 — 이 함수는
        // 받은 키를 그대로 헤더에 실을 뿐이라는 계약을 고정한다. 실제 네트워크는
        // 타지 않도록 즉시 타임아웃되는 값을 준다.
        let e = list_models("", std::time::Duration::from_nanos(1))
            .await
            .unwrap_err();
        assert!(e.starts_with("anthropic"), "{e}");
    }
}
