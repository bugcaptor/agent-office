// src-tauri/src/api_keys.rs
//
// 외부 API 키는 전부 환경변수로 공급된다(설정 store/UI 없음).
// 읽기는 이 모듈로 일원화한다. ② 머리 위 표시의 ANTHROPIC_API_KEY도 같은
// 패턴으로 여기에 상수를 추가한다. 키 값은 로그/에러 메시지에 절대 싣지
// 않는다.

/// PixelLab API 키 환경변수 이름.
pub const PIXELLAB_API_KEY: &str = "PIXELLAB_API_KEY";

/// 봇 모드(#58) Gitea 접근 토큰 환경변수 이름. 봇 시작 시 로그인 셸에서 캡처해
/// 프로세스 env에 심는다(session::env_capture).
pub const GITEA_TOKEN: &str = "GITEA_TOKEN";

/// 봇 모드(#58) Gitea 웹 베이스 URL 환경변수 이름(예: `http://host:5088`).
/// 미설정 시 http(s) origin에서 파싱을 시도하고, ssh origin이면 이 값을 요구한다.
pub const GITEA_BASE_URL: &str = "GITEA_BASE_URL";

/// 환경변수에서 API 키를 읽는다. 미설정이거나 공백뿐이면 None.
pub fn env_api_key(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(v) => {
            let t = v.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 주의: cargo test는 병렬 실행 — 전역 env 오염을 피하려고 테스트마다
    // 고유한 변수 이름을 쓴다 (PIXELLAB_API_KEY 실물은 건드리지 않는다).
    #[test]
    fn returns_trimmed_value_when_set() {
        std::env::set_var("AO_TEST_KEY_SET", "  sk-abc  ");
        assert_eq!(env_api_key("AO_TEST_KEY_SET"), Some("sk-abc".to_string()));
        std::env::remove_var("AO_TEST_KEY_SET");
    }

    #[test]
    fn returns_none_when_unset() {
        assert_eq!(env_api_key("AO_TEST_KEY_UNSET_NEVER_DEFINED"), None);
    }

    #[test]
    fn returns_none_when_blank() {
        std::env::set_var("AO_TEST_KEY_BLANK", "   ");
        assert_eq!(env_api_key("AO_TEST_KEY_BLANK"), None);
        std::env::remove_var("AO_TEST_KEY_BLANK");
    }
}
