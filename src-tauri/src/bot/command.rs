// src-tauri/src/bot/command.rs
//
// 봇 트리거 판정의 순수 로직: 캐릭터 이름 → 슬래시 slug 파생, 이슈/댓글 본문에서
// 슬래시 명령 매칭, 화이트리스트 검증, 봇 자기 댓글(숨은 마커) 스킵. 부수효과가
// 없어 단위 테스트로 전부 덮는다. 설계 정본은 docs/bot-mode-design.md.

/// 봇 댓글에 심는 숨은 기계 마커. 에이전트가 다는 모든 댓글 머리에 이 마커를 넣게
/// 프롬프트로 지시하고, 앱은 마커가 있는 댓글을 트리거·릴레이에서 모두 스킵한다
/// (봇 댓글이 로컬 계정 명의라 작성자 필터만으로는 자기 목소리를 못 거른다).
pub const MARKER_PREFIX: &str = "<!-- agent-office-bot:";

/// `agent_id`를 담은 봇 댓글 마커 한 줄. 프롬프트 템플릿에 그대로 넣는다.
pub fn bot_marker(agent_id: &str) -> String {
    format!("{MARKER_PREFIX}{agent_id} -->")
}

/// 본문에 봇 마커가 있으면 true(자기/다른 봇 댓글 → 무시 대상).
pub fn has_bot_marker(body: &str) -> bool {
    body.contains(MARKER_PREFIX)
}

/// 캐릭터 이름에서 슬래시 slug를 파생한다. 소문자화하고 공백·`/`를 제거하며,
/// 그 밖의 문자(한글 등 비ASCII 포함)는 유지한다. `"Nova Kim"` → `"novakim"`,
/// `"빌더"` → `"빌더"`. 결과가 비면(예: 이름이 공백뿐) 호출부가 별칭을 요구한다.
pub fn derive_slug(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace() && *c != '/')
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// `config_slug`(별칭)가 비어있지 않으면 그것을, 아니면 이름 파생 slug를 쓴다.
pub fn effective_slug(name: &str, config_slug: Option<&str>) -> String {
    match config_slug {
        Some(s) if !s.trim().is_empty() => derive_slug(s),
        _ => derive_slug(name),
    }
}

/// 본문에 `/slug` 슬래시 명령이 (줄 시작/공백 경계의) 단독 토큰으로 있으면 true.
/// 대소문자 무시. 뒤따르는 동사(`/slug go` 등)는 별도 토큰이라 무시된다. 토큰
/// 끝의 구두점(`.,;:!?)` 등)은 벗겨 `/slug,` 같은 경우도 매치한다.
pub fn matches_command(text: &str, slug: &str) -> bool {
    if slug.is_empty() {
        return false;
    }
    let needle: String = format!("/{}", slug.to_lowercase());
    text.split_whitespace().any(|tok| {
        let trimmed = tok.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '/');
        trimmed.to_lowercase() == needle
    })
}

/// 화이트리스트 작성자가 `/slug`와 함께 남긴 **신뢰 가능한 지시문**을 뽑는다.
/// 본문에서 첫 번째 `/slug` 토큰(트리거)만 제거하고 남은 텍스트를 공백 정규화해
/// 돌려준다. 트리거만 있고 별도 문장이 없으면 None(순수 트리거).
///
/// 주입 프롬프트에서 이 지시문은 요청자 본인의 말이므로 신뢰하되, 에이전트가
/// `tea`로 읽어들이는 이슈 본문 등은 여전히 신뢰불가 참고자료로 다룬다(리뷰:
/// 트리거 발신자=화이트리스트라 그의 지시는 신뢰, 그 밖 콘텐츠는 불신).
pub fn extract_directive(text: &str, slug: &str) -> Option<String> {
    if slug.is_empty() {
        return None;
    }
    let needle = format!("/{}", slug.to_lowercase());
    let mut removed = false;
    let mut kept: Vec<&str> = Vec::new();
    for tok in text.split_whitespace() {
        let trimmed = tok.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '/');
        if !removed && trimmed.to_lowercase() == needle {
            removed = true; // 트리거 토큰 1개만 제거
            continue;
        }
        kept.push(tok);
    }
    let joined = kept.join(" ");
    let joined = joined.trim();
    if joined.is_empty() {
        None
    } else {
        Some(joined.to_string())
    }
}

/// 작성자가 명령을 발동할 권한이 있는지. tea 로그인 계정(owner)은 항상 허용이고,
/// 추가 화이트리스트는 대소문자 무시로 비교한다.
pub fn is_authorized(author: &str, owner: &str, whitelist: &[String]) -> bool {
    author.eq_ignore_ascii_case(owner)
        || whitelist.iter().any(|w| w.eq_ignore_ascii_case(author))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_slug_lowercases_and_strips_space() {
        assert_eq!(derive_slug("Nova Kim"), "novakim");
        assert_eq!(derive_slug("Ada"), "ada");
        assert_eq!(derive_slug("빌더"), "빌더");
        assert_eq!(derive_slug("  "), "");
    }

    #[test]
    fn effective_slug_prefers_alias() {
        assert_eq!(effective_slug("Nova Kim", Some("nova")), "nova");
        assert_eq!(effective_slug("Nova Kim", Some("  ")), "novakim");
        assert_eq!(effective_slug("Nova Kim", None), "novakim");
    }

    #[test]
    fn matches_command_token_boundary() {
        assert!(matches_command("/ada go 작업 시작해줘", "ada"));
        assert!(matches_command("작업 부탁 /ada", "ada"));
        assert!(matches_command("/ADA", "ada")); // 대소문자 무시
        assert!(matches_command("/ada, 부탁해", "ada")); // 후행 구두점
        assert!(matches_command("/빌더 이슈 봐줘", "빌더"));
    }

    #[test]
    fn matches_command_rejects_substring_and_empty() {
        assert!(!matches_command("/adabra 마법", "ada")); // 부분 문자열 아님
        assert!(!matches_command("email/ada@x", "ada")); // 토큰 경계 아님
        assert!(!matches_command("ada 그냥 언급", "ada")); // 슬래시 없음
        assert!(!matches_command("/ada", "")); // 빈 slug
    }

    #[test]
    fn extract_directive_strips_trigger_keeps_sentence() {
        assert_eq!(
            extract_directive("/ada 로그인 버그 고쳐줘", "ada").as_deref(),
            Some("로그인 버그 고쳐줘")
        );
        assert_eq!(
            extract_directive("먼저 확인 부탁 /ada, 그리고 PR", "ada").as_deref(),
            Some("먼저 확인 부탁 그리고 PR")
        );
        assert_eq!(
            extract_directive("/빌더 이슈 봐줘", "빌더").as_deref(),
            Some("이슈 봐줘")
        );
    }

    #[test]
    fn extract_directive_none_when_only_trigger() {
        assert_eq!(extract_directive("/ada", "ada"), None);
        assert_eq!(extract_directive("  /ada  ", "ada"), None);
        assert_eq!(extract_directive("아무 문장", ""), None); // 빈 slug
    }

    #[test]
    fn marker_roundtrip() {
        let m = bot_marker("agent-123");
        assert_eq!(m, "<!-- agent-office-bot:agent-123 -->");
        assert!(has_bot_marker(&format!("{m}\n**[Nova]** 시작합니다")));
        assert!(!has_bot_marker("사람이 쓴 평범한 댓글"));
    }

    #[test]
    fn authorization_owner_and_whitelist() {
        let wl = vec!["alice".to_string(), "Bob".to_string()];
        assert!(is_authorized("bugcaptor", "bugcaptor", &wl)); // owner
        assert!(is_authorized("BugCaptor", "bugcaptor", &wl)); // 대소문자 무시
        assert!(is_authorized("bob", "bugcaptor", &wl)); // 화이트리스트
        assert!(!is_authorized("mallory", "bugcaptor", &wl)); // 외부인
    }
}
