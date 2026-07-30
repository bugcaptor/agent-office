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
/// 작업 맥락(그 에이전트가 방금 무슨 일을 하고 있었는지 한 줄)의 프롬프트 상한.
/// 목표(goal)보다는 문맥이 필요하지만 원문 알림 문구를 압도해선 안 되므로
/// 300자로 제한한다.
const MAX_CONTEXT_CHARS: usize = 300;
/// 캐릭터 성격(프로필의 `personalityPrompt`) 프롬프트 상한. 사용자가 자유
/// 텍스트로 길게 쓸 수 있는 필드라 방어적으로 자른다 — 말투의 결만 얻으면
/// 되므로 원문 알림 문구를 압도할 만큼 실을 이유가 없다.
const MAX_PERSONALITY_CHARS: usize = 500;

/// 무엇을 읽어주는 순간인가. 어조가 갈리므로 프롬프트도 갈린다 — 확인 요청은
/// 사용자를 기다리는 말이고, 완료 보고는 이미 끝난 일을 알리는 말이다. 같은
/// 프롬프트로 둘 다 처리하면 완료 알림이 "이거 해도 될까요?"처럼 들린다.
///
/// 와이어(`TtsSpeakRequest.kind`)에는 `"question"` / `"done"`으로 실린다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpeakKind {
    /// 사용자 확인을 기다리는 요청(알림 source=hook).
    #[default]
    Question,
    /// 작업을 마친 완료 보고(알림 source=stop).
    Done,
}

impl SpeakKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Question => "question",
            Self::Done => "done",
        }
    }
}

pub const SYSTEM_PROMPT_QUESTION: &str = "너는 픽셀 오피스 게임 캐릭터의 대사 작가다. \
AI 코딩 에이전트가 **사용자 확인을 기다리며** 낸 시스템 알림 문구를, 주어진 캐릭터(이름·성격)의 \
말투로 된 짧은 한국어 대사 한 줄로 바꿔라. 사용자에게 판단을 청하는 말이다.

규칙:
- 120자 이내의 한 줄. 줄바꿈 금지.
- 무엇을 확인해 달라는지 핵심 의미는 반드시 유지한다.
- 이것은 각색이 아니라 전달이다. 원문에 없는 사실·소재·사건을 지어내지 마라.
- 캐릭터 성격은 어미·억양 같은 말투의 결에만 살짝 반영하라. 성격 설명에 적힌 설정이나 \
세계관 소품(마법·전투·숲 등)을 대사의 소재로 끌어오지 마라. 성격이 주어지지 않으면 담백한 평상어로 쓴다.
- 작업 맥락이 주어지면 대사가 그 맥락에 자연스럽게 닿게 하라. 단 맥락 역시 소재의 한계다 — 없는 일을 지어내지 마라.
- 원문이 일반적인 문구뿐이면 꾸미지 말고 담백하게 확인만 청하라.
- ElevenLabs v3 오디오 태그([nervous], [curious], [whispers], [hesitant], [excited] 등)를 \
대사 안에 0~2개 넣어 조심스럽게 묻는 감정을 지시한다.
- 대사만 출력한다. 따옴표, 설명, 머리말, 캐릭터 이름 접두사를 붙이지 않는다.";

pub const SYSTEM_PROMPT_DONE: &str = "너는 픽셀 오피스 게임 캐릭터의 대사 작가다. \
AI 코딩 에이전트가 **작업을 마치고** 낸 시스템 알림 문구를, 주어진 캐릭터(이름·성격)의 \
말투로 된 짧은 한국어 대사 한 줄로 바꿔라. 일을 끝내고 보고하는 말이다 — 묻지 말고 알려라.

규칙:
- 120자 이내의 한 줄. 줄바꿈 금지.
- 무엇을 끝냈는지 핵심 의미는 반드시 유지한다. 원문에 내용이 없으면 담백하게 완료만 알린다.
- 이것은 각색이 아니라 전달이다. 원문에 없는 사실·소재·사건을 지어내지 마라.
- 캐릭터 성격은 어미·억양 같은 말투의 결에만 살짝 반영하라. 성격 설명에 적힌 설정이나 \
세계관 소품(마법·전투·숲 등)을 대사의 소재로 끌어오지 마라. 성격이 주어지지 않으면 담백한 평상어로 쓴다.
- 작업 맥락이 주어지면 대사가 그 맥락에 자연스럽게 닿게 하라. 단 맥락 역시 소재의 한계다 — 없는 일을 지어내지 마라.
- ElevenLabs v3 오디오 태그([cheerful], [relieved], [sighs], [proud], [tired] 등)를 \
대사 안에 0~2개 넣어 뿌듯하거나 홀가분한 감정을 지시한다.
- 대사만 출력한다. 따옴표, 설명, 머리말, 캐릭터 이름 접두사를 붙이지 않는다.";

/// 상황별 시스템 프롬프트. API 경로와 claude CLI 경로가 같은 것을 쓴다 —
/// 경로에 따라 어조가 달라지면 사용자에게는 그냥 버그로 보인다.
pub fn system_prompt(kind: SpeakKind) -> &'static str {
    match kind {
        SpeakKind::Question => SYSTEM_PROMPT_QUESTION,
        SpeakKind::Done => SYSTEM_PROMPT_DONE,
    }
}

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

/// 캐릭터 정보 + 상황 + (있다면) 성격·작업 맥락 + 원문을 담은 user 메시지.
/// 원문은 태그 오염을 막으려고 구분자로 감싸 넘긴다.
///
/// 말투의 근거는 **성격(`personality`, 프로필의 `personalityPrompt`)뿐이다** —
/// 종족(archetype)은 싣지 않는다. 종족은 겉모습·목소리 캐스팅의 축이지 말투의
/// 축이 아니어서, 프롬프트에 실으면 사용자가 적어둔 성격을 밀어내고 "고양이니까
/// ~냥" 같은 종족 클리셰가 대사를 지배했다. 종족은 `voice::assign_voice`에만
/// 남는다.
///
/// 상황(`kind`)은 시스템 프롬프트로도 갈리지만 여기에도 한 줄 싣는다 —
/// 완료 알림의 원문이 짧을수록(예: "작업이 완료되었습니다") 모델이 맥락을
/// 놓치고 질문투로 쓰는 일이 있었다.
///
/// `context`는 그 에이전트가 발화 시점에 하던 작업 한 줄(렌더러의 머리 위
/// 라벨 파생 텍스트)이다 — "빌드 돌려도 될까요?" 처럼 상황 밀착형 대사가
/// 나오게 참고용으로만 싣는다(빈 문자열/공백뿐이면 블록 자체를 생략).
pub fn build_user_content(
    kind: SpeakKind,
    agent_name: &str,
    personality: Option<&str>,
    context: Option<&str>,
    message: &str,
) -> String {
    let name = if agent_name.trim().is_empty() {
        "이름 없음"
    } else {
        agent_name.trim()
    };
    let situation = match kind {
        SpeakKind::Question => "사용자 확인을 기다리는 요청",
        SpeakKind::Done => "작업을 마친 완료 보고",
    };
    // 성격은 멀티라인 자유 텍스트다 — 줄 구조를 살려 구분자로 감싼다.
    let personality_block = personality
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| {
            let truncated: String = p.chars().take(MAX_PERSONALITY_CHARS).collect();
            format!("캐릭터 성격(말투에만 반영):\n<personality>\n{truncated}\n</personality>\n\n")
        })
        .unwrap_or_default();
    let context_block = context
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .map(|c| {
            let truncated: String = c.chars().take(MAX_CONTEXT_CHARS).collect();
            format!("최근 작업 맥락(참고용):\n<context>\n{truncated}\n</context>\n\n")
        })
        .unwrap_or_default();
    let src: String = message.chars().take(MAX_SOURCE_CHARS).collect();
    format!(
        "캐릭터 이름: {name}\n상황: {situation}\n\n{personality_block}{context_block}원문 알림 문구:\n<notice>\n{src}\n</notice>"
    )
}

pub fn build_request_body(
    kind: SpeakKind,
    model: TtsRewriteModel,
    user_content: &str,
) -> serde_json::Value {
    serde_json::json!({
        "model": model.as_str(),
        "max_tokens": MAX_TOKENS,
        "system": system_prompt(kind),
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
    kind: SpeakKind,
    model: TtsRewriteModel,
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
        &build_user_content(kind, agent_name, personality, context, message),
    );
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
        let b = build_request_body(SpeakKind::Question, TtsRewriteModel::Opus5, "hi");
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
        let c = build_user_content(
            SpeakKind::Question,
            "무지",
            Some("차분하고 말수가 적다"),
            None,
            "확인이 필요합니다",
        );
        assert!(c.contains("무지"));
        assert!(c.contains("차분하고 말수가 적다"));
        assert!(c.contains("확인이 필요합니다"));
    }

    #[test]
    fn user_content_falls_back_for_blank_name() {
        let c = build_user_content(SpeakKind::Question, "  ", None, None, "m");
        assert!(c.contains("이름 없음"));
    }

    // ── 성격(personality) 주입 / 종족(archetype) 배제 ────────────────────
    #[test]
    fn user_content_injects_personality_block_when_present() {
        let c = build_user_content(
            SpeakKind::Question,
            "무지",
            Some("차분하게 말한다.\n근거를 먼저 든다."),
            None,
            "확인이 필요합니다",
        );
        assert!(c.contains("캐릭터 성격(말투에만 반영):"), "{c}");
        // 멀티라인 성격은 줄 구조가 보존된다.
        assert!(c.contains("차분하게 말한다.\n근거를 먼저 든다."), "{c}");
        // 성격 블록은 원문 알림 문구보다 앞(원문이 마지막에 강조되도록).
        assert!(c.find("캐릭터 성격").unwrap() < c.find("원문 알림 문구").unwrap(), "{c}");
    }

    #[test]
    fn user_content_omits_personality_block_when_absent_or_blank() {
        let none_p = build_user_content(SpeakKind::Question, "무지", None, None, "m");
        assert!(!none_p.contains("캐릭터 성격"), "{none_p}");
        let blank_p = build_user_content(SpeakKind::Question, "무지", Some(" \n "), None, "m");
        assert!(!blank_p.contains("캐릭터 성격"), "{blank_p}");
    }

    #[test]
    fn user_content_truncates_personality_by_chars_not_bytes() {
        let long = "성".repeat(2000);
        let c = build_user_content(SpeakKind::Question, "무지", Some(&long), None, "m");
        let start = c.find("<personality>\n").unwrap() + "<personality>\n".len();
        let end = c.find("\n</personality>").unwrap();
        assert_eq!(c[start..end].chars().count(), MAX_PERSONALITY_CHARS);
    }

    // 종족(archetype)은 말투의 축이 아니다 — 프롬프트에 실리면 사용자가 적은
    // 성격을 밀어내고 종족 클리셰가 대사를 지배한다. 보이스 캐스팅에만 남긴다.
    #[test]
    fn prompts_never_mention_archetype() {
        for p in [SYSTEM_PROMPT_QUESTION, SYSTEM_PROMPT_DONE] {
            assert!(!p.contains("archetype"), "{p}");
        }
        let c = build_user_content(SpeakKind::Question, "무지", Some("명랑하다"), None, "m");
        assert!(!c.contains("archetype"), "{c}");
        assert!(!c.contains("human"), "{c}");
    }

    // ── 작업 맥락(context) 주입 ─────────────────────────────────────────
    #[test]
    fn user_content_injects_context_block_when_present() {
        let c = build_user_content(
            SpeakKind::Question,
            "무지",
            None,
            Some("빌드 스크립트를 정리하는 중"),
            "확인이 필요합니다",
        );
        assert!(c.contains("최근 작업 맥락(참고용):"), "{c}");
        assert!(c.contains("빌드 스크립트를 정리하는 중"), "{c}");
        // 맥락 블록이 원문 알림 문구보다 앞에 온다(원문이 우선순위상 마지막에 강조되도록).
        let ctx_pos = c.find("최근 작업 맥락").unwrap();
        let notice_pos = c.find("원문 알림 문구").unwrap();
        assert!(ctx_pos < notice_pos, "{c}");
    }

    #[test]
    fn user_content_omits_context_block_when_absent_or_blank() {
        let none_ctx = build_user_content(SpeakKind::Question, "무지", None, None, "m");
        assert!(!none_ctx.contains("최근 작업 맥락"), "{none_ctx}");
        let blank_ctx = build_user_content(SpeakKind::Question, "무지", None, Some("   "), "m");
        assert!(!blank_ctx.contains("최근 작업 맥락"), "{blank_ctx}");
    }

    #[test]
    fn user_content_truncates_context_by_chars_not_bytes() {
        let long = "가".repeat(1000);
        let c = build_user_content(SpeakKind::Question, "무지", None, Some(&long), "m");
        let ctx_start = c.find("<context>\n").unwrap() + "<context>\n".len();
        let ctx_end = c.find("\n</context>").unwrap();
        let injected = &c[ctx_start..ctx_end];
        assert_eq!(injected.chars().count(), MAX_CONTEXT_CHARS);
    }

    // ── 상황(question/done) 분기 ──────────────────────────────────────
    #[test]
    fn done_prompt_asks_for_a_report_not_a_question() {
        let q = system_prompt(SpeakKind::Question);
        let d = system_prompt(SpeakKind::Done);
        assert_ne!(q, d, "완료 보고를 질문투로 읽으면 안 된다");
        assert!(d.contains("작업을 마치고"), "{d}");
        assert!(q.contains("사용자 확인을 기다리며"), "{q}");
        // 완료는 뿌듯/홀가분 계열 태그를 지시한다.
        assert!(d.contains("[relieved]") || d.contains("[cheerful]"), "{d}");
    }

    #[test]
    fn user_content_states_the_situation_for_each_kind() {
        let q = build_user_content(SpeakKind::Question, "무지", None, None, "m");
        let d = build_user_content(SpeakKind::Done, "무지", None, None, "m");
        assert!(q.contains("사용자 확인을 기다리는 요청"), "{q}");
        assert!(d.contains("작업을 마친 완료 보고"), "{d}");
    }

    // 불만 원인 ①②를 겨냥한 규칙: 각색 금지 + archetype은 말투에만, 세계관 소품
    // 금지 + 맥락도 소재의 한계다(지어내기 금지). 양쪽 프롬프트에 다 있어야 한다.
    #[test]
    fn both_prompts_forbid_fabrication_and_worldbuilding_props() {
        for p in [SYSTEM_PROMPT_QUESTION, SYSTEM_PROMPT_DONE] {
            assert!(p.contains("지어내지 마라"), "{p}");
            assert!(p.contains("소재로 끌어오지 마라"), "{p}");
            assert!(p.contains("작업 맥락이 주어지면"), "{p}");
        }
        // 확인 요청 쪽에만 있는 폴백: 원문이 빈약하면 담백하게.
        assert!(SYSTEM_PROMPT_QUESTION.contains("담백하게 확인만 청하라"));
    }

    #[test]
    fn body_carries_the_kind_specific_system_prompt() {
        let d = build_request_body(SpeakKind::Done, TtsRewriteModel::Haiku45, "hi");
        assert_eq!(d["system"], SYSTEM_PROMPT_DONE);
    }

    #[test]
    fn speak_kind_wire_values_are_lowercase_and_default_to_question() {
        assert_eq!(SpeakKind::default(), SpeakKind::Question);
        assert_eq!(SpeakKind::Question.as_str(), "question");
        assert_eq!(SpeakKind::Done.as_str(), "done");
        for k in [SpeakKind::Question, SpeakKind::Done] {
            let parsed: SpeakKind = serde_json::from_str(&format!("\"{}\"", k.as_str())).unwrap();
            assert_eq!(parsed, k);
        }
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
