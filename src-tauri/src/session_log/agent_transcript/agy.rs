// src-tauri/src/session_log/agent_transcript/agy.rs
//
// Antigravity CLI(agy) 전사 소스. agy는 대화마다
// `~/.gemini/antigravity-cli/brain/<conversationId>/.system_generated/logs/transcript_full.jsonl`
// 을 남긴다. 위치는 훅이 실어 온 `transcriptPath`를 `AgyResumeStore`가 캐릭터별로
// 기억해 두므로, 그 경로를 그대로 따라간다(Claude 소스가 리줌 스토어를 보는 것과 같은 결).
//
// 줄 모양(agy 1.5.18 실측) — 한 줄이 한 step:
//   {"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"<USER_REQUEST>\n…\n</USER_REQUEST>\n<ADDITIONAL_METADATA>…"}
//   {"type":"PLANNER_RESPONSE","content":"답","thinking":"…","tool_calls":[{"name":"run_command","args":{"CommandLine":"ls"}}]}
//   {"type":"GENERIC","content":"Created At: …\nCompleted At: …\n<도구 출력>"}
// thinking은 다른 소스처럼 항목으로 만들지 않는다.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use super::{ItemRole, TranscriptItem, TranscriptSource};
use crate::persistence::agy_resume_store::AgyResumeStore;

pub struct AgySource {
    store: Arc<AgyResumeStore>,
}

impl TranscriptSource for AgySource {
    fn label(&self) -> &'static str {
        "agy"
    }

    fn locate(&mut self, agent_id: &str, _cwd: &str) -> Option<PathBuf> {
        let entry = self.store.load_all().remove(agent_id)?;
        let path = PathBuf::from(entry.transcript_path?);
        path.is_file().then_some(path)
    }

    fn parse(&self, raw: &str) -> Vec<TranscriptItem> {
        serde_json::from_str::<Value>(raw).map(|v| parse_step(&v)).unwrap_or_default()
    }
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn parse_step(v: &Value) -> Vec<TranscriptItem> {
    let content = str_of(v, "content");
    match str_of(v, "type") {
        // 사람이 친 입력만 보인다. 다른 source(시스템이 끼워 넣는 알림 등)는 대화가 아니다.
        "USER_INPUT" if str_of(v, "source") == "USER_EXPLICIT" => user_request(content)
            .map(|text| vec![TranscriptItem::speech(ItemRole::User, text)])
            .unwrap_or_default(),
        "PLANNER_RESPONSE" => {
            let mut items = Vec::new();
            if !content.trim().is_empty() {
                items.push(TranscriptItem::speech(ItemRole::Assistant, content.trim()));
            }
            for call in v.get("tool_calls").and_then(Value::as_array).into_iter().flatten() {
                let name = call.get("name").and_then(Value::as_str).map(str::to_string);
                let args = call.get("args").unwrap_or(&Value::Null);
                items.push(TranscriptItem::tool_use(name, tool_args_text(args)));
            }
            items
        }
        "GENERIC" => {
            let text = strip_step_header(content);
            if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![TranscriptItem::tool_result(text, false)]
            }
        }
        kind if kind.contains("ERROR") => {
            let text = strip_step_header(content);
            let text = if text.trim().is_empty() { kind.to_string() } else { text };
            vec![TranscriptItem::tool_result(text, true)]
        }
        _ => Vec::new(),
    }
}

/// `<USER_REQUEST>` 안쪽. 태그가 없으면 본문 전체(메타데이터 블록 없는 옛 모양 대비).
fn user_request(content: &str) -> Option<String> {
    let inner = match content.split_once("<USER_REQUEST>") {
        Some((_, rest)) => rest.split_once("</USER_REQUEST>").map_or(rest, |(inner, _)| inner),
        None => content,
    };
    let inner = inner.trim();
    (!inner.is_empty()).then(|| inner.to_string())
}

/// 도구 결과 앞의 `Created At:`/`Completed At:` 줄을 걷어낸다(실측: 같은 두 줄이 겹쳐 온다).
fn strip_step_header(content: &str) -> String {
    let mut lines = content.lines().peekable();
    while lines
        .peek()
        .is_some_and(|l| l.starts_with("Created At:") || l.starts_with("Completed At:"))
    {
        lines.next();
    }
    lines.collect::<Vec<_>>().join("\n")
}

/// 도구 인자 → 한 덩어리 글. 셸 명령·경로처럼 뜻이 서는 값이 있으면 그것을,
/// 없으면 내부 키(`toolSummary`·`WaitMsBeforeAsync` 같은)를 뺀 JSON을 보인다.
fn tool_args_text(args: &Value) -> String {
    let Some(obj) = args.as_object() else {
        return String::new();
    };
    const KEYS: [&str; 9] = [
        "commandline", "command", "absolutepath", "targetfile", "path", "file_path",
        "directorypath", "query", "url",
    ];
    for key in KEYS {
        if let Some(value) = obj
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .and_then(|(_, v)| v.as_str())
        {
            let value = value.trim().trim_matches('"').trim();
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }
    let rest: serde_json::Map<String, Value> = obj
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "toolSummary" | "WaitMsBeforeAsync"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if rest.is_empty() {
        str_of(args, "toolSummary").to_string()
    } else {
        Value::Object(rest).to_string()
    }
}

/// agy 소스를 만든다. 전사 경로는 리줌 스토어가 알려 주므로 홈 디렉터리를 뒤지지 않는다.
pub fn source(store: Arc<AgyResumeStore>) -> Option<Box<dyn TranscriptSource>> {
    Some(Box::new(AgySource { store }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_log::agent_transcript::ItemKind;

    fn parse(raw: &str) -> Vec<TranscriptItem> {
        parse_step(&serde_json::from_str(raw).unwrap())
    }

    #[test]
    fn user_input_keeps_only_the_request_body() {
        let items = parse(
            r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"<USER_REQUEST>\nRead note.txt\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nThe current local time is: x.\n</ADDITIONAL_METADATA>"}"#,
        );
        assert_eq!(items, vec![TranscriptItem::speech(ItemRole::User, "Read note.txt")]);
        assert!(parse(r#"{"source":"SYSTEM","type":"USER_INPUT","content":"<USER_REQUEST>x</USER_REQUEST>"}"#).is_empty());
    }

    #[test]
    fn planner_response_yields_text_and_tool_calls_but_not_thinking() {
        let items = parse(
            r#"{"type":"PLANNER_RESPONSE","thinking":"hmm","content":"","tool_calls":[{"name":"run_command","args":{"CommandLine":"ls -la","WaitMsBeforeAsync":5000,"toolSummary":"List"}},{"name":"read_file","args":{"Path":"\"/w/note.txt\""}},{"name":"mystery","args":{"Foo":1,"toolSummary":"x"}}]}"#,
        );
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].kind, ItemKind::ToolUse);
        assert_eq!(items[0].tool_name.as_deref(), Some("run_command"));
        assert_eq!(items[0].text, "ls -la");
        assert_eq!(items[1].text, "/w/note.txt");
        assert_eq!(items[2].text, r#"{"Foo":1}"#);

        let answer = parse(r#"{"type":"PLANNER_RESPONSE","content":"pineapple","thinking":"t"}"#);
        assert_eq!(answer, vec![TranscriptItem::speech(ItemRole::Assistant, "pineapple")]);
    }

    #[test]
    fn generic_step_is_a_tool_result_without_the_timestamp_header() {
        let items = parse(
            r#"{"type":"GENERIC","content":"Created At: a\nCompleted At: b\nCreated At: a\nCompleted At: b\nShowing lines 1-1 of 1\n1: secret"}"#,
        );
        assert_eq!(items, vec![TranscriptItem::tool_result("Showing lines 1-1 of 1\n1: secret", false)]);
        assert!(parse(r#"{"type":"GENERIC","content":"Created At: a\nCompleted At: b"}"#).is_empty());
    }

    #[test]
    fn error_steps_are_error_results_and_unknown_lines_are_ignored() {
        let items = parse(r#"{"type":"ERROR","content":"Created At: a\nboom"}"#);
        assert_eq!(items, vec![TranscriptItem::tool_result("boom", true)]);
        assert!(parse(r#"{"type":"CHECKPOINT"}"#).is_empty());
        assert!(AgySource { store: Arc::new(AgyResumeStore::new(PathBuf::from("/nonexistent/x.json"))) }
            .parse("not json")
            .is_empty());
    }
}
