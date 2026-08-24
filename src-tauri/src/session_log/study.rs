// src-tauri/src/session_log/study.rs
//
// 세션 로그 한 개 -> 회고·학습용 Markdown. docs/session-log-design.md §5 가 정본.
//
// 요약기 파이프라인(summarizer/)을 `SummaryPurpose::Study`로 재사용한다.
// 일기와 같은 구조이고 상한·타임아웃·모델만 다르다. 자동 생성은 없다 --
// 사용자가 메뉴에서 명시적으로 부를 때만 돈다.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::persistence::settings_store::SummaryProvider;
use crate::summarizer::SummaryPurpose;

/// 로그 파일에서 읽어 올릴 최대 바이트. 이보다 크면 앞뒤로 나눠 읽는다
/// (문자 단위 최종 캡은 요약기가 한다).
const MAX_READ_BYTES: u64 = 8 * 1024 * 1024;

pub const STUDY_SYSTEM_PROMPT_KO: &str = "\
너는 개발 세션 기록을 회고용 학습자료로 바꾸는 편집자다. 입력은 터미널 세션 한 개의 전사(轉寫)다 — \
사람이 친 명령, AI 코딩 에이전트와의 대화, 도구 출력이 시간 순으로 섞여 있다. \
이것을 다른 사람도 읽고 배울 수 있는 한국어 Markdown 문서 한 편으로 정리하라.\n\n\
줄머리 표식: `▶ 사용자:`=사람의 지시, `⏺ 에이전트:`=AI 응답, `⚒ <도구>:`=도구 호출, \
`⇤ 결과:`=도구 출력, `⤷`=서브에이전트, `--- HH:MM:SS ---`=시각, \
`[전체 화면 앱 시작]`=vim 같은 전체 화면 앱 구간(내용 없음), `… (이하 생략)`=잘린 자리. \
표식이 없는 줄은 셸 출력이다.\n\n\
반드시 이 순서와 제목을 그대로 쓴다:\n\
## 한 줄 요약\n\
## 무엇을 하려 했나\n\
## 어떻게 진행됐나\n\
## 막힌 지점과 해결\n\
## 배울 점\n\
## 다시 해본다면\n\n\
지침:\n\
- '어떻게 진행됐나'는 시간 순 흐름으로 쓰고 실제 명령·파일 경로·함수명을 그대로 인용한다.\n\
- '막힌 지점과 해결'이 이 문서의 핵심이다. 오류 메시지는 원문 그대로 코드 블록에 넣고, \
무엇이 원인이었고 무엇으로 넘겼는지 쓴다. 막힌 곳이 없었으면 그렇게 쓴다.\n\
- '배울 점'은 다음에 같은 상황에서 바로 꺼내 쓸 수 있는 형태로 쓴다(원칙 + 그 근거가 된 이 세션의 장면).\n\
- 로그에 실제로 있는 내용만 쓴다. 추측·일반론·교과서적 설명으로 채우지 마라. \
로그가 짧거나 알맹이가 없으면 그 사실을 짧게 적고 끝낸다.\n\
- 진행 표시줄·반복 출력·잡음은 무시한다.\n\
- 한국어로 쓰고, 사과·머리말·메타발언 없이 문서 본문만 출력한다. 문서 전체를 코드펜스로 감싸지 마라.";

pub const STUDY_SYSTEM_PROMPT_EN: &str = "\
You are an editor who turns a development session record into study material for review. The input is the \
transcript of one terminal session -- commands a person typed, a conversation with an AI coding agent, and \
tool output, interleaved in time order. Turn it into one Markdown document in English that someone else can \
read and learn from.\n\n\
Line markers (they stay in Korean in the transcript): `▶ 사용자:`=a human instruction, \
`⏺ 에이전트:`=the AI reply, `⚒ <tool>:`=a tool call, \
`⇤ 결과:`=tool output, `⤷`=a subagent, `--- HH:MM:SS ---`=a timestamp, \
`[전체 화면 앱 시작]`=a full-screen app section such as vim (no content), \
`… (이하 생략)`=a truncation point. Unmarked lines are shell output.\n\n\
Use exactly this order and these headings:\n\
## Summary\n\
## What was the goal\n\
## How it went\n\
## Where it got stuck and how it was resolved\n\
## What to take away\n\
## If done again\n\n\
Guidelines:\n\
- Write 'How it went' in time order and quote the actual commands, file paths, and function names verbatim.\n\
- 'Where it got stuck and how it was resolved' is the heart of this document. Put error messages verbatim in \
code blocks, and say what the cause was and what got past it. If nothing got stuck, say so.\n\
- 'What to take away' should be usable straight away next time the same situation comes up (a principle plus \
the scene in this session that grounds it).\n\
- Write only what is actually in the log. Do not pad with guesses, generalities, or textbook explanations. \
If the log is short or has no substance, say so briefly and stop.\n\
- Ignore progress bars, repeated output, and noise.\n\
- Write in English and output the document body only, with no apologies, prefixes, or meta commentary. \
Do not wrap the whole document in a code fence.";

/// UI 언어별 학습자료 프롬프트. ko는 Phase 6 이전 문자열 그대로(이동만),
/// en은 번역이 아니라 같은 지침을 영어로 다시 쓴 것이다.
///
/// 줄머리 표식(`▶ 사용자:` 등)은 **양쪽 프롬프트에서 한국어 그대로**다 —
/// 표식을 만드는 `session_log::agent_transcript`가 언어를 타지 않기 때문이다.
/// 프롬프트는 입력에 실제로 등장하는 리터럴을 가리켜야 하므로 번역하면 안 된다.
pub fn study_system_prompt(lang: crate::i18n::Lang) -> &'static str {
    match lang {
        crate::i18n::Lang::Ko => STUDY_SYSTEM_PROMPT_KO,
        crate::i18n::Lang::En => STUDY_SYSTEM_PROMPT_EN,
    }
}

pub struct StudyResult {
    pub path: PathBuf,
    pub dir: PathBuf,
    pub file_name: String,
}

/// 로그 파일을 읽어 학습자료를 만들고 `study/` 아래에 저장한다.
/// `openrouter_key`는 provider가 OpenRouter일 때만 쓰인다(키 스토어가 Tauri
/// State 안에 있어 호출측이 떠서 넘긴다 — `models`와 같은 이유).
pub async fn generate(
    root: &Path,
    agent_id: &str,
    log_path: &Path,
    provider: SummaryProvider,
    models: &crate::persistence::settings_store::SummaryModels,
    openrouter_key: Option<&str>,
    lang: crate::i18n::Lang,
) -> Result<StudyResult, String> {
    let text = read_capped(log_path, lang)?;
    if text.trim().is_empty() {
        return Err("empty-log".to_string());
    }

    let body = crate::summarizer::summarize(
        provider,
        SummaryPurpose::Study,
        study_system_prompt(lang),
        &text,
        models,
        openrouter_key,
        lang,
    )
    .await?;
    let body = strip_wrapping_fence(&body);

    let dir = super::store::study_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("study-dir-failed: {e}"))?;

    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let file_name = format!("{agent_id}-{stamp}.md");
    let path = dir.join(&file_name);

    let source = log_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let doc = format!(
        "<!-- agent-office 학습자료 -->\n> 원본 세션 로그: `{source}`  ·  생성: {}\n\n{}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M"),
        body.trim()
    );
    std::fs::write(&path, doc).map_err(|e| format!("study-write-failed: {e}"))?;

    Ok(StudyResult {
        path,
        dir,
        file_name,
    })
}

/// 로그를 읽되 너무 크면 앞뒤만 읽는다. 세션의 시작(목표)과 끝(결말)이 회고에
/// 가장 중요하므로 가운데를 버린다 -- 요약기의 문자 캡과 같은 방침이다.
/// 중략 마커도 요약기 것을 그대로 쓴다(`summarizer::truncation_marker`) --
/// 둘 다 같은 프롬프트 안에 실려 모델에게 보이므로 언어가 갈리면 안 된다.
fn read_capped(path: &Path, lang: crate::i18n::Lang) -> Result<String, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("log-read-failed: {e}"))?;
    let len = file
        .metadata()
        .map(|m| m.len())
        .map_err(|e| format!("log-read-failed: {e}"))?;

    if len <= MAX_READ_BYTES {
        let mut buf = Vec::with_capacity(len as usize);
        file.read_to_end(&mut buf)
            .map_err(|e| format!("log-read-failed: {e}"))?;
        return Ok(String::from_utf8_lossy(&buf).into_owned());
    }

    let half = (MAX_READ_BYTES / 2) as usize;
    let mut head = vec![0u8; half];
    file.read_exact(&mut head)
        .map_err(|e| format!("log-read-failed: {e}"))?;
    let mut tail = vec![0u8; half];
    file.seek(SeekFrom::End(-(half as i64)))
        .map_err(|e| format!("log-read-failed: {e}"))?;
    file.read_exact(&mut tail)
        .map_err(|e| format!("log-read-failed: {e}"))?;

    Ok(format!(
        "{}{}{}",
        String::from_utf8_lossy(&head),
        crate::summarizer::truncation_marker(lang),
        String::from_utf8_lossy(&tail)
    ))
}

/// 문서 전체를 감싼 코드펜스를 벗긴다(모델이 지시를 어겼을 때의 안전망).
fn strip_wrapping_fence(raw: &str) -> String {
    let trimmed = raw.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed.to_string();
    };
    // 첫 줄(``` 뒤 언어 표시)을 버리고 마지막 ```를 벗긴다.
    let after_first_line = rest.split_once('\n').map(|(_, r)| r).unwrap_or("");
    after_first_line
        .trim_end()
        .strip_suffix("```")
        .unwrap_or(after_first_line)
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_file_is_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.log");
        std::fs::write(&path, "hello\nworld\n").unwrap();
        assert_eq!(read_capped(&path, crate::i18n::Lang::Ko).unwrap(), "hello\nworld\n");
    }

    #[test]
    fn huge_file_keeps_head_and_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.log");
        let mut body = String::from("HEAD-MARKER\n");
        body.push_str(&"x".repeat((MAX_READ_BYTES + 1000) as usize));
        body.push_str("\nTAIL-MARKER\n");
        std::fs::write(&path, &body).unwrap();

        let read = read_capped(&path, crate::i18n::Lang::Ko).unwrap();
        assert!(read.starts_with("HEAD-MARKER"), "머리가 없다");
        assert!(read.trim_end().ends_with("TAIL-MARKER"), "꼬리가 없다");
        assert!(read.contains("…(중략)…"));
        assert!(read.len() < body.len());
    }

    #[test]
    fn wrapping_code_fence_is_stripped() {
        assert_eq!(
            strip_wrapping_fence("```markdown\n## 제목\n본문\n```"),
            "## 제목\n본문"
        );
        assert_eq!(strip_wrapping_fence("## 제목\n본문"), "## 제목\n본문");
    }

    #[test]
    fn prompt_pins_the_required_sections() {
        for section in [
            "## 한 줄 요약",
            "## 무엇을 하려 했나",
            "## 어떻게 진행됐나",
            "## 막힌 지점과 해결",
            "## 배울 점",
            "## 다시 해본다면",
        ] {
            assert!(STUDY_SYSTEM_PROMPT_KO.contains(section), "{section}");
        }
    }

    // ── UI 언어 분기 ───────────────────────────────────────────────────
    #[test]
    fn study_prompt_splits_by_language() {
        use crate::i18n::Lang;
        let ko = study_system_prompt(Lang::Ko);
        let en = study_system_prompt(Lang::En);
        assert_ne!(ko, en);
        assert!(ko.contains("## 한 줄 요약"), "{ko}");
        assert!(en.contains("## Summary"), "{en}");
        // 절 제목은 언어를 타지만 개수와 순서는 같아야 한다.
        for section in [
            "## Summary",
            "## What was the goal",
            "## How it went",
            "## Where it got stuck and how it was resolved",
            "## What to take away",
            "## If done again",
        ] {
            assert!(en.contains(section), "{section}");
        }
    }

    // 줄머리 표식은 `agent_transcript`가 만드는 **입력의 리터럴**이라 언어를
    // 타지 않는다 — 영어 프롬프트도 같은 한국어 표식을 가리켜야 한다.
    #[test]
    fn english_prompt_still_points_at_the_korean_transcript_markers() {
        let en = study_system_prompt(crate::i18n::Lang::En);
        for marker in ["▶ 사용자:", "⏺ 에이전트:", "⇤ 결과:", "[전체 화면 앱 시작]"] {
            assert!(en.contains(marker), "{marker}");
        }
        // 그래도 출력 언어 지시는 영어여야 한다.
        assert!(en.contains("Write in English"), "{en}");
    }

    // 사용자에게 닿는 실패는 문구가 아니라 안정적인 코드다(프런트가 번역).
    #[tokio::test]
    async fn empty_log_fails_with_a_stable_code() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("empty.log");
        std::fs::write(&log, "   \n\t\n").unwrap();
        let err = generate(
            dir.path(),
            "a1",
            &log,
            SummaryProvider::Codex,
            &Default::default(),
            None,
            crate::i18n::Lang::Ko,
        )
        .await
        .err()
        .expect("빈 로그는 실패해야 한다");
        assert_eq!(err, "empty-log");
    }

    #[test]
    fn read_errors_carry_a_code_and_a_technical_detail() {
        let err = read_capped(
            std::path::Path::new("/definitely/not/a/log/xyzzy.log"),
            crate::i18n::Lang::Ko,
        )
        .unwrap_err();
        let (code, detail) = err.split_once(": ").expect("{code}: {detail} 형식");
        assert_eq!(code, "log-read-failed");
        assert!(!detail.is_empty(), "OS 오류 원문이 상세로 붙어야 한다");
    }

    // 중략 마커도 프롬프트 안에 실려 모델에게 보인다 — 요약기와 같은 언어를 쓴다.
    #[test]
    fn huge_file_truncation_marker_follows_the_language() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.log");
        let mut body = String::from("HEAD\n");
        body.push_str(&"x".repeat((MAX_READ_BYTES + 1000) as usize));
        body.push_str("\nTAIL\n");
        std::fs::write(&path, &body).unwrap();
        let en = read_capped(&path, crate::i18n::Lang::En).unwrap();
        assert!(en.contains("(truncated)"), "영어 마커가 없다");
        assert!(!en.contains("중략"), "영어 프롬프트에 한국어가 섞였다");
    }

}
