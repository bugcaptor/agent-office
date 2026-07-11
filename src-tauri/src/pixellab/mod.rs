// src-tauri/src/pixellab/mod.rs
//
// PixelLab API 클라이언트 (POST /create-image-pixen, 64×64 PNG 단일 이미지 동기 생성).
// 순수 로직(build_request_body/parse_response)과 HTTP(generate_image)를
// 분리 — 단위 테스트는 순수 부분만 다룬다(네트워크 테스트 금지).

use serde::Serialize;

pub const BASE_URL: &str = "https://api.pixellab.ai/v2";
/// 생성 크기 64×64 고정 (pixen 제약: 최소 면적 32×32, 변 4의 배수).
pub const SPRITE_SIZE: u32 = 64;
/// pixen description maxLength.
pub const DESCRIPTION_MAX_CHARS: usize = 2000;

/// 생성 결과 — TS `GeneratedSpriteImage` 미러 (camelCase 직렬화).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedImage {
    pub png_base64: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// IPC로는 `to_ipc_string()`의 `"{code}: {상세}"`로 나간다.
#[derive(Debug, Clone, PartialEq)]
pub enum PixelLabError {
    MissingApiKey,
    InvalidApiKey,
    InsufficientCredits,
    RateLimited,
    Validation(String),
    Network(String),
    MalformedResponse(String),
}

impl PixelLabError {
    pub fn code(&self) -> &'static str {
        match self {
            PixelLabError::MissingApiKey => "missing_api_key",
            PixelLabError::InvalidApiKey => "invalid_api_key",
            PixelLabError::InsufficientCredits => "insufficient_credits",
            PixelLabError::RateLimited => "rate_limited",
            PixelLabError::Validation(_) => "validation",
            PixelLabError::Network(_) => "network",
            PixelLabError::MalformedResponse(_) => "malformed_response",
        }
    }

    /// 렌더러가 첫 ':' 앞 코드로 분기한다 (Global Constraints).
    pub fn to_ipc_string(&self) -> String {
        let detail: String = match self {
            PixelLabError::MissingApiKey => "PIXELLAB_API_KEY is not set".to_string(),
            PixelLabError::InvalidApiKey => "invalid API token".to_string(),
            PixelLabError::InsufficientCredits => "insufficient PixelLab credits".to_string(),
            PixelLabError::RateLimited => "rate limit exceeded".to_string(),
            PixelLabError::Validation(d)
            | PixelLabError::Network(d)
            | PixelLabError::MalformedResponse(d) => d.clone(),
        };
        format!("{}: {}", self.code(), detail)
    }
}

/// 요청 body. 밝고 귀여운 JRPG 룩을 위해 스타일 파라미터를 고정 전송:
/// outline=검정 단색 외곽선, detail=medium(64px에서 highly detailed는 노이즈),
/// view=low top-down, direction=south(정면 — 클래식 JRPG). seed는 기본값 유지.
pub fn build_request_body(description: &str) -> serde_json::Value {
    serde_json::json!({
        "description": description,
        "image_size": { "width": SPRITE_SIZE, "height": SPRITE_SIZE },
        "no_background": true,
        "outline": "single color black outline",
        "detail": "medium detail",
        "view": "low top-down",
        "direction": "south",
    })
}

/// HTTP 상태 + body → 결과/에러 매핑. 순수.
pub fn parse_response(status: u16, body: &str) -> Result<GeneratedImage, PixelLabError> {
    match status {
        200 => {
            let v: serde_json::Value = serde_json::from_str(body)
                .map_err(|e| PixelLabError::MalformedResponse(format!("invalid JSON: {e}")))?;
            let png = v
                .get("image")
                .and_then(|i| i.get("base64"))
                .and_then(|b| b.as_str())
                .ok_or_else(|| {
                    PixelLabError::MalformedResponse("missing image.base64".to_string())
                })?;
            // usage.type == "usd"일 때만 비용 노출 (generations 구독 차감은 None).
            let cost_usd = v
                .get("usage")
                .filter(|u| u.get("type").and_then(|t| t.as_str()) == Some("usd"))
                .and_then(|u| u.get("usd"))
                .and_then(|c| c.as_f64());
            Ok(GeneratedImage {
                png_base64: png.to_string(),
                cost_usd,
            })
        }
        401 => Err(PixelLabError::InvalidApiKey),
        402 => Err(PixelLabError::InsufficientCredits),
        422 => Err(PixelLabError::Validation(extract_detail(body))),
        429 | 529 => Err(PixelLabError::RateLimited),
        s => Err(PixelLabError::Network(format!("HTTP {s}"))),
    }
}

/// 422 body에서 detail을 추출해 200자로 자른다 (키/본문 통제 불가 — 방어적).
fn extract_detail(body: &str) -> String {
    let raw = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("detail").map(|d| d.to_string()))
        .unwrap_or_else(|| body.to_string());
    raw.chars().take(200).collect()
}

/// 얇은 HTTP 래퍼 — 이 함수만 네트워크를 만진다. 응답 지연 수 초~수십 초
/// (동기 HTTP), 타임아웃 120초. 키 값은 에러 문자열에 싣지 않는다.
pub async fn generate_image(description: &str) -> Result<GeneratedImage, PixelLabError> {
    let key = crate::api_keys::env_api_key(crate::api_keys::PIXELLAB_API_KEY)
        .ok_or(PixelLabError::MissingApiKey)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| PixelLabError::Network(e.to_string()))?;
    let resp = client
        .post(format!("{BASE_URL}/create-image-pixen"))
        .bearer_auth(key)
        .json(&build_request_body(description))
        .send()
        .await
        .map_err(|e| PixelLabError::Network(e.to_string()))?;
    let status = resp.status().as_u16();
    let body = resp
        .text()
        .await
        .map_err(|e| PixelLabError::Network(e.to_string()))?;
    parse_response(status, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_body_matches_spec() {
        // JRPG 스타일 파라미터 고정 전송 (outline/detail/view/direction) — seed는 미전송.
        let body = build_request_body("a knight");
        assert_eq!(
            body,
            serde_json::json!({
                "description": "a knight",
                "image_size": { "width": 64, "height": 64 },
                "no_background": true,
                "outline": "single color black outline",
                "detail": "medium detail",
                "view": "low top-down",
                "direction": "south",
            })
        );
    }

    #[test]
    fn parse_200_with_usd_usage() {
        let body =
            r#"{"image":{"type":"base64","base64":"AAAA"},"usage":{"type":"usd","usd":0.02}}"#;
        let img = parse_response(200, body).unwrap();
        assert_eq!(img.png_base64, "AAAA");
        assert_eq!(img.cost_usd, Some(0.02));
    }

    #[test]
    fn parse_200_with_null_usage() {
        let body = r#"{"image":{"type":"base64","base64":"BBBB"},"usage":null}"#;
        let img = parse_response(200, body).unwrap();
        assert_eq!(img.png_base64, "BBBB");
        assert_eq!(img.cost_usd, None);
    }

    #[test]
    fn parse_200_with_generations_usage_has_no_cost() {
        let body = r#"{"image":{"base64":"CCCC"},"usage":{"type":"generations","generations":1}}"#;
        let img = parse_response(200, body).unwrap();
        assert_eq!(img.cost_usd, None);
    }

    #[test]
    fn parse_200_missing_image_is_malformed() {
        let err = parse_response(200, r#"{"usage":null}"#).unwrap_err();
        assert_eq!(err.code(), "malformed_response");
    }

    #[test]
    fn parse_200_non_json_is_malformed() {
        let err = parse_response(200, "<html>oops</html>").unwrap_err();
        assert_eq!(err.code(), "malformed_response");
    }

    #[test]
    fn parse_error_statuses() {
        assert_eq!(
            parse_response(401, "").unwrap_err().code(),
            "invalid_api_key"
        );
        assert_eq!(
            parse_response(402, "").unwrap_err().code(),
            "insufficient_credits"
        );
        assert_eq!(parse_response(429, "").unwrap_err().code(), "rate_limited");
        assert_eq!(parse_response(529, "").unwrap_err().code(), "rate_limited");
        assert_eq!(parse_response(500, "").unwrap_err().code(), "network");
        assert_eq!(parse_response(503, "").unwrap_err().code(), "network");
    }

    #[test]
    fn parse_422_extracts_detail() {
        let err = parse_response(422, r#"{"detail":[{"msg":"too small"}]}"#).unwrap_err();
        assert_eq!(err.code(), "validation");
        assert!(err.to_ipc_string().contains("too small"));
    }

    #[test]
    fn ipc_string_starts_with_code_colon() {
        assert_eq!(
            PixelLabError::MissingApiKey.to_ipc_string(),
            "missing_api_key: PIXELLAB_API_KEY is not set"
        );
        assert!(parse_response(401, "")
            .unwrap_err()
            .to_ipc_string()
            .starts_with("invalid_api_key: "));
    }

    #[test]
    fn generated_image_serializes_camel_case() {
        let img = GeneratedImage {
            png_base64: "AAAA".into(),
            cost_usd: Some(0.02),
        };
        let v = serde_json::to_value(&img).unwrap();
        assert_eq!(v, serde_json::json!({"pngBase64": "AAAA", "costUsd": 0.02}));
        let none = GeneratedImage {
            png_base64: "B".into(),
            cost_usd: None,
        };
        assert_eq!(
            serde_json::to_value(&none).unwrap(),
            serde_json::json!({"pngBase64": "B"})
        );
    }
}
