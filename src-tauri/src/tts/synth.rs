// src-tauri/src/tts/synth.rs
//
// ElevenLabs 음성 합성. `POST /v1/text-to-speech/{voice_id}` (헤더 xi-api-key),
// 응답은 mp3 바이트.
//
// 모델 두 단계:
//  - 1차 `eleven_v3` — 오디오 태그([nervous] 등)를 감정 지시로 해석하는 유일한
//    모델. 리라이트 프롬프트가 태그를 넣는 이유가 이것이다.
//  - 폴백 `eleven_multilingual_v2` — v3를 못 쓰는 계정. 이때는 **반드시**
//    대괄호 태그를 텍스트에서 제거해야 한다. v2는 태그를 감정이 아니라
//    글자로 읽어버려서 "대괄호 너버스 이거 진행해도 될까요"가 된다.
//
// 순수 로직(strip_audio_tags / build_body / classify_status)과 HTTP(synthesize)를
// 분리한다.

/// 오디오 태그를 지원하는 표현형 모델(공개 API 사용 가능).
pub const MODEL_V3: &str = "eleven_v3";
/// v3 불가 계정용 폴백. 태그 미지원 → 태그를 지운 텍스트를 보낸다.
pub const MODEL_V2: &str = "eleven_multilingual_v2";
/// mp3로 받는다 — 웹뷰의 `decodeAudioData`가 바로 디코드할 수 있고 캐시도 작다.
pub const OUTPUT_FORMAT: &str = "mp3_44100_128";
pub const MIME_TYPE: &str = "audio/mpeg";
/// 한 줄 대사 합성 타임아웃.
pub const TIMEOUT_SECS: u64 = 20;

fn base_url(voice_id: &str) -> String {
    format!("https://api.elevenlabs.io/v1/text-to-speech/{voice_id}")
}

#[derive(Debug, Clone, PartialEq)]
pub enum SynthError {
    MissingApiKey,
    InvalidApiKey,
    /// 요청한 model_id를 이 계정/보이스에서 쓸 수 없다 → v2로 폴백 신호.
    ModelUnavailable(String),
    QuotaExceeded,
    RateLimited,
    Validation(String),
    Network(String),
    /// 200인데 본문이 비었거나 mp3가 아님.
    EmptyAudio,
}

impl SynthError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingApiKey => "missing_api_key",
            Self::InvalidApiKey => "invalid_api_key",
            Self::ModelUnavailable(_) => "model_unavailable",
            Self::QuotaExceeded => "quota_exceeded",
            Self::RateLimited => "rate_limited",
            Self::Validation(_) => "validation",
            Self::Network(_) => "network",
            Self::EmptyAudio => "empty_audio",
        }
    }

    /// v2 폴백을 시도할 값어치가 있는 실패인가. 모델 거절뿐 아니라 422
    /// (v3를 모르는 배포에서 model_id 검증 실패로 오는 경로)도 포함한다.
    pub fn should_retry_without_v3(&self) -> bool {
        matches!(self, Self::ModelUnavailable(_) | Self::Validation(_))
    }
}

impl std::fmt::Display for SynthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModelUnavailable(d) | Self::Validation(d) | Self::Network(d) => {
                write!(f, "{}: {d}", self.code())
            }
            _ => write!(f, "{}", self.code()),
        }
    }
}

/// `[nervous]` 류 대괄호 태그를 제거하고 공백을 정리한다. 중첩 대괄호는
/// 상정하지 않지만, 닫히지 않은 `[`는 남은 텍스트를 통째로 삼키지 않도록
/// 문자 그대로 보존한다. 순수.
pub fn strip_audio_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            // 이 '['에 대응하는 ']'가 있으면 구간 전체를 버린다.
            if let Some(close) = (i + 1..chars.len()).find(|&j| chars[j] == ']') {
                i = close + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// voice_settings는 캐릭터 대사에 맞춘 고정값: stability를 낮춰 감정 표현을
/// 살리고, similarity_boost는 중간, style은 약간 올린다.
pub fn build_body(text: &str, model_id: &str) -> serde_json::Value {
    serde_json::json!({
        "text": text,
        "model_id": model_id,
        "voice_settings": {
            "stability": 0.4,
            "similarity_boost": 0.75,
            "style": 0.3,
            "use_speaker_boost": true,
        },
    })
}

/// 상태 코드(+ 짧은 body 힌트) → 에러. 순수.
pub fn classify_status(status: u16, body_hint: &str) -> SynthError {
    match status {
        401 => SynthError::InvalidApiKey,
        // ElevenLabs는 모델 권한 부족을 403/422 어느 쪽으로도 낸다.
        403 => SynthError::ModelUnavailable(detail(body_hint)),
        422 => SynthError::Validation(detail(body_hint)),
        400 => {
            if body_hint.contains("model") {
                SynthError::ModelUnavailable(detail(body_hint))
            } else {
                SynthError::Validation(detail(body_hint))
            }
        }
        429 => SynthError::RateLimited,
        402 => SynthError::QuotaExceeded,
        s => SynthError::Network(format!("HTTP {s}")),
    }
}

/// 통제 불가한 외부 body는 200자로 자른다(로그 폭탄 방지).
fn detail(body: &str) -> String {
    body.chars().take(200).collect()
}

/// 얇은 HTTP 래퍼 — 이 함수만 네트워크를 만진다.
pub async fn synthesize(
    api_key: &str,
    voice_id: &str,
    text: &str,
    model_id: &str,
) -> Result<Vec<u8>, SynthError> {
    if api_key.trim().is_empty() {
        return Err(SynthError::MissingApiKey);
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
        .build()
        .map_err(|e| SynthError::Network(e.to_string()))?;
    let resp = client
        .post(base_url(voice_id))
        .header("xi-api-key", api_key)
        .header("content-type", "application/json")
        .header("accept", MIME_TYPE)
        .query(&[("output_format", OUTPUT_FORMAT)])
        .json(&build_body(text, model_id))
        .send()
        .await
        .map_err(|e| SynthError::Network(e.to_string()))?;
    let status = resp.status().as_u16();
    if status != 200 {
        let hint = resp.text().await.unwrap_or_default();
        return Err(classify_status(status, &hint));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| SynthError::Network(e.to_string()))?;
    if bytes.is_empty() {
        return Err(SynthError::EmptyAudio);
    }
    Ok(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_removes_tags_and_normalizes_spaces() {
        assert_eq!(
            strip_audio_tags("[nervous] 이거 [whispers] 진행해도 될까요?"),
            "이거 진행해도 될까요?"
        );
        assert_eq!(strip_audio_tags("태그 없음"), "태그 없음");
        assert_eq!(strip_audio_tags("[laughs]"), "");
    }

    #[test]
    fn strip_keeps_unclosed_bracket_instead_of_eating_the_line() {
        // 닫히지 않은 '['가 문장을 통째로 삼키면 v2 폴백에서 무음이 된다.
        assert_eq!(strip_audio_tags("[nervous 이거 될까요?"), "[nervous 이거 될까요?");
        assert_eq!(strip_audio_tags("a [x] b [y c"), "a b [y c");
    }

    #[test]
    fn body_carries_text_and_model() {
        let b = build_body("안녕", MODEL_V3);
        assert_eq!(b["text"], "안녕");
        assert_eq!(b["model_id"], "eleven_v3");
        assert!(b["voice_settings"].is_object());
    }

    #[test]
    fn status_classification_drives_the_v2_fallback() {
        assert!(classify_status(403, "model not allowed").should_retry_without_v3());
        assert!(classify_status(422, "model_id invalid").should_retry_without_v3());
        assert!(classify_status(400, "unknown model_id").should_retry_without_v3());
        // 이건 폴백해도 소용없다.
        assert!(!classify_status(401, "").should_retry_without_v3());
        assert!(!classify_status(429, "").should_retry_without_v3());
        assert!(!classify_status(500, "").should_retry_without_v3());
        assert_eq!(classify_status(401, ""), SynthError::InvalidApiKey);
        assert_eq!(classify_status(402, ""), SynthError::QuotaExceeded);
    }

    #[test]
    fn detail_is_capped() {
        let long = "x".repeat(5000);
        let e = classify_status(422, &long);
        assert!(format!("{e}").len() < 300);
    }
}
