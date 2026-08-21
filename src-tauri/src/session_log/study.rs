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

pub const STUDY_SYSTEM_PROMPT: &str = "\
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

pub struct StudyResult {
    pub path: PathBuf,
    pub dir: PathBuf,
    pub file_name: String,
}

/// 로그 파일을 읽어 학습자료를 만들고 `study/` 아래에 저장한다.
pub async fn generate(
    root: &Path,
    agent_id: &str,
    log_path: &Path,
    provider: SummaryProvider,
    models: &crate::persistence::settings_store::SummaryModels,
) -> Result<StudyResult, String> {
    let text = read_capped(log_path)?;
    if text.trim().is_empty() {
        return Err("빈 로그입니다".to_string());
    }

    let body = crate::summarizer::summarize(
        provider,
        SummaryPurpose::Study,
        STUDY_SYSTEM_PROMPT,
        &text,
        models,
    )
    .await?;
    let body = strip_wrapping_fence(&body);

    let dir = super::store::study_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("학습자료 폴더를 만들지 못했습니다: {e}"))?;

    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let file_name = format!("{agent_id}-{stamp}.md");
    let path = dir.join(&file_name);

    let source = log_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let doc = format!(
        "<!-- agent-office 학습자료 -->\n> 원본 세션 로그: `{source}`  ·  생성: {}\n\n{}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M"),
        body.trim()
    );
    std::fs::write(&path, doc).map_err(|e| format!("학습자료를 저장하지 못했습니다: {e}"))?;

    Ok(StudyResult {
        path,
        dir,
        file_name,
    })
}

/// 로그를 읽되 너무 크면 앞뒤만 읽는다. 세션의 시작(목표)과 끝(결말)이 회고에
/// 가장 중요하므로 가운데를 버린다 -- 요약기의 문자 캡과 같은 방침이다.
fn read_capped(path: &Path) -> Result<String, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("로그를 열지 못했습니다: {e}"))?;
    let len = file
        .metadata()
        .map(|m| m.len())
        .map_err(|e| format!("로그 정보를 읽지 못했습니다: {e}"))?;

    if len <= MAX_READ_BYTES {
        let mut buf = Vec::with_capacity(len as usize);
        file.read_to_end(&mut buf)
            .map_err(|e| format!("로그를 읽지 못했습니다: {e}"))?;
        return Ok(String::from_utf8_lossy(&buf).into_owned());
    }

    let half = (MAX_READ_BYTES / 2) as usize;
    let mut head = vec![0u8; half];
    file.read_exact(&mut head)
        .map_err(|e| format!("로그를 읽지 못했습니다: {e}"))?;
    let mut tail = vec![0u8; half];
    file.seek(SeekFrom::End(-(half as i64)))
        .map_err(|e| format!("로그를 읽지 못했습니다: {e}"))?;
    file.read_exact(&mut tail)
        .map_err(|e| format!("로그를 읽지 못했습니다: {e}"))?;

    Ok(format!(
        "{}\n…(중략)…\n{}",
        String::from_utf8_lossy(&head),
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
        assert_eq!(read_capped(&path).unwrap(), "hello\nworld\n");
    }

    #[test]
    fn huge_file_keeps_head_and_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.log");
        let mut body = String::from("HEAD-MARKER\n");
        body.push_str(&"x".repeat((MAX_READ_BYTES + 1000) as usize));
        body.push_str("\nTAIL-MARKER\n");
        std::fs::write(&path, &body).unwrap();

        let read = read_capped(&path).unwrap();
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
            assert!(STUDY_SYSTEM_PROMPT.contains(section), "{section}");
        }
    }
}
