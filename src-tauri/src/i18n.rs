// src-tauri/src/i18n.rs
//
// 백엔드의 UI 언어 해석. **번역 카탈로그가 아니다** — Rust에는 카탈로그를 두지
// 않는다(프런트 `src/shared/i18n/locales/*`가 유일한 카탈로그다).
//
// 언어를 타는 Rust 문자열은 AI에게 보내는 프롬프트 몇 개뿐이라(TTS 리라이트,
// 학습자료, 동료 대화 주입 템플릿, 요약기 중략 마커) 카탈로그 인프라를 세우는
// 것은 과설계다. 각 모듈이 언어별 `&'static str` 상수를 자기 옆에 두고 이
// 모듈의 `Lang`으로만 갈라 쓴다.
//
// 사용자 화면에 나가는 **에러**도 여기서 번역하지 않는다 — 백엔드는 `"{code}:
// {detail}"` 형태의 안정적인 코드를 내려보내고 문구는 프런트가 카탈로그에서
// 고른다(`src/renderer/shared/backendError.ts`).
//
// ## 폴백 규칙 — 프런트 `catalog.matchLanguage`와 같다
//
// 정확 일치 → 프리픽스 일치(`ko-KR` → `ko`) → 폴백(en). `AppSettings.language`는
// 자유 문자열이므로(언어를 늘려도 타입 계약이 안 바뀌게 한 결정) 모르는 값은
// 판단하지 않고 그냥 En으로 떨어뜨린다.
//
// 프런트와 다른 점 하나: 프런트는 카탈로그 폴더가 곧 지원 언어 목록이지만,
// 여기서는 프롬프트 상수를 실제로 갖고 있는 언어만 `Lang`이다. 카탈로그에
// 언어를 추가해도 여기 variant를 늘리기 전까지는 프롬프트가 En으로 도는데,
// 이건 버그가 아니라 정상 동작이다(프런트 `promptProfiles.ts`의 폴백 규칙과
// 같은 판단 — 기계 번역한 프롬프트로 출력 품질을 망치느니 영어가 낫다).

use crate::persistence::settings_store::AppSettings;

/// `AppSettings.language`가 이 값이면 OS 로케일을 따른다(프런트
/// `LANGUAGE_SYSTEM`과 같은 약속).
pub const LANGUAGE_SYSTEM: &str = "system";

/// 백엔드가 프롬프트를 갈라 쓰는 언어.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Lang {
    Ko,
    /// 폴백. 카탈로그·프롬프트가 없는 언어는 전부 여기로 떨어진다.
    #[default]
    En,
}

impl Lang {
    /// BCP47 기본 코드. 로그·테스트용이며 와이어로 나가지 않는다.
    pub fn code(self) -> &'static str {
        match self {
            Self::Ko => "ko",
            Self::En => "en",
        }
    }
}

/// 설정의 UI 언어. 프롬프트를 만드는 모든 지점의 단일 진입점이다.
pub fn ui_lang(settings: &AppSettings) -> Lang {
    resolve_lang(&settings.language)
}

/// 설정값(`"system"` 또는 언어 코드) → 실제 언어. `"system"`·빈 값은 OS 로케일.
pub fn resolve_lang(setting: &str) -> Lang {
    resolve_with_locale(setting, sys_locale::get_locale().as_deref())
}

/// `resolve_lang`의 순수 본체 — OS 로케일을 주입받아 테스트 가능하게 한다.
/// (실제 OS 로케일은 테스트가 통제할 수 없으므로 여기까지만 검증한다.)
fn resolve_with_locale(setting: &str, os_locale: Option<&str>) -> Lang {
    let trimmed = setting.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(LANGUAGE_SYSTEM) {
        return match_lang(os_locale.unwrap_or(""));
    }
    match_lang(trimmed)
}

/// 임의의 로케일 문자열을 `Lang` 하나로 좁힌다. 정확 일치 → 프리픽스 일치
/// (`ko-KR` → `ko`) → En. 프런트 `catalog.matchLanguage`와 같은 규칙이다.
fn match_lang(locale: &str) -> Lang {
    let lower = locale.trim().to_ascii_lowercase();
    // 로케일 구분자는 `-`가 정본이지만 POSIX(`ko_KR.UTF-8`)로 오는 환경도 있다.
    let prefix = lower
        .split(['-', '_', '.'])
        .next()
        .unwrap_or_default();
    match prefix {
        "ko" => Lang::Ko,
        _ => Lang::En,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_codes_resolve_to_their_language() {
        assert_eq!(resolve_with_locale("ko", None), Lang::Ko);
        assert_eq!(resolve_with_locale("en", None), Lang::En);
        // 지역 변종은 프리픽스로 좁힌다.
        assert_eq!(resolve_with_locale("ko-KR", None), Lang::Ko);
        assert_eq!(resolve_with_locale("en-GB", None), Lang::En);
        // 대소문자·공백에 흔들리지 않는다.
        assert_eq!(resolve_with_locale("  KO-kr ", None), Lang::Ko);
    }

    // 카탈로그에 없는 언어는 **판단하지 않고** En으로 떨어진다. 설정값이 자유
    // 문자열인 계약(언어 추가 시 Rust를 고치지 않는다)의 뒷면이다.
    #[test]
    fn unknown_languages_fall_back_to_english() {
        for setting in ["ja", "de-DE", "zh-Hans", "gibberish", "!!"] {
            assert_eq!(resolve_with_locale(setting, None), Lang::En, "{setting}");
        }
    }

    #[test]
    fn system_follows_the_os_locale() {
        assert_eq!(resolve_with_locale("system", Some("ko-KR")), Lang::Ko);
        assert_eq!(resolve_with_locale("system", Some("en-US")), Lang::En);
        // POSIX 표기(`ko_KR.UTF-8`)도 같은 규칙으로 좁힌다.
        assert_eq!(resolve_with_locale("system", Some("ko_KR.UTF-8")), Lang::Ko);
        // 로케일을 못 읽는 환경 / 모르는 로케일은 폴백.
        assert_eq!(resolve_with_locale("system", None), Lang::En);
        assert_eq!(resolve_with_locale("system", Some("ja-JP")), Lang::En);
        // 빈 설정값도 "system"과 같게 취급한다(설정 파일이 손상된 경우 등).
        assert_eq!(resolve_with_locale("", Some("ko")), Lang::Ko);
    }

    #[test]
    fn ui_lang_reads_the_settings_field() {
        let mut settings = AppSettings::default();
        settings.language = "ko".to_string();
        assert_eq!(ui_lang(&settings), Lang::Ko);
        settings.language = "ja".to_string();
        assert_eq!(ui_lang(&settings), Lang::En);
    }

    #[test]
    fn default_lang_is_the_fallback() {
        assert_eq!(Lang::default(), Lang::En);
        assert_eq!(Lang::Ko.code(), "ko");
        assert_eq!(Lang::En.code(), "en");
    }

    /// 실제 OS 로케일 경로가 패닉 없이 도는지만 확인한다(값은 기계마다 다르다).
    #[test]
    fn real_os_locale_path_does_not_panic() {
        let _ = resolve_lang(LANGUAGE_SYSTEM);
    }
}
