//! 자동 진행의 네이티브 조각(docs/auto-run-design.md): 작업자 세션의 마지막 응답을
//! 에이전트 전사에서 읽고, 판단자(`claude -p` 이어지는 세션)를 한 번 부른다.
//! 루프와 상태는 렌더러가 쥔다.

use crate::session_log::agent_transcript::{ItemKind, ItemRole, TranscriptItem, TranscriptSource};
use crate::types::{AutoRunJudgeRequest, AutoRunJudgeResult};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

/// 전사 끝에서 읽는 양. 마지막 턴의 응답이 들어갈 만큼이면 된다.
const REPLY_TAIL_BYTES: u64 = 1024 * 1024;
/// 판단자에게 넘기는 응답 상한(문자). 넘으면 앞쪽을 자른다 — 결론은 끝에 있다.
pub const MAX_REPLY_CHARS: usize = 32_000;
const JUDGE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const JUDGE_STDOUT_MAX: usize = 4 * 1024 * 1024;

/// 판단자의 출력 스키마. 결정 셋 중 하나와 지시·인계 메모.
pub const JUDGE_SCHEMA: &str = r#"{"type":"object","properties":{"decision":{"type":"string","enum":["next","followup","stop"]},"message":{"type":"string"},"note":{"type":"string"}},"required":["decision","message","note"],"additionalProperties":false}"#;

/// 판단자 세션이 사는 폴더. Claude는 세션을 cwd별로 저장하므로 `--resume`도 같은 곳에서 돈다.
pub fn judge_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("auto-run").join("judge")
}

/// 마지막 사용자 텍스트 뒤의 assistant 텍스트를 합친다. 서브에이전트 대화는 뺀다.
pub fn extract_last_reply(items: &[TranscriptItem]) -> Option<String> {
    let main: Vec<&TranscriptItem> = items.iter().filter(|i| !i.sidechain).collect();
    let start = main
        .iter()
        .rposition(|i| i.role == ItemRole::User && i.kind == ItemKind::Text)
        .map_or(0, |n| n + 1);
    let texts: Vec<&str> = main[start..]
        .iter()
        .filter(|i| i.role == ItemRole::Assistant && i.kind == ItemKind::Text)
        .map(|i| i.text.trim())
        .filter(|t| !t.is_empty())
        .collect();
    if texts.is_empty() {
        return None;
    }
    let joined = texts.join("\n\n");
    let count = joined.chars().count();
    Some(if count > MAX_REPLY_CHARS {
        let tail: String = joined.chars().skip(count - MAX_REPLY_CHARS).collect();
        format!("…{tail}")
    } else {
        joined
    })
}

/// 파일 끝 `max`바이트. 중간에서 시작했으면 잘린 첫 줄을 버린다.
fn read_tail(path: &Path, max: u64) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(max);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Some(if start > 0 {
        text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default()
    } else {
        text
    })
}

/// 전사 소스 하나에서 그 캐릭터의 마지막 응답을 읽는다.
pub fn last_reply(source: &mut dyn TranscriptSource, agent_id: &str, cwd: &str) -> Option<String> {
    let path = source.locate(agent_id, cwd)?;
    let raw = read_tail(&path, REPLY_TAIL_BYTES)?;
    let items: Vec<TranscriptItem> = raw.lines().flat_map(|line| source.parse(line)).collect();
    extract_last_reply(&items)
}

fn invalid_model(model: &str) -> bool {
    model.trim().is_empty() || model.starts_with('-') || model.chars().any(char::is_control)
}

/// 판단자 명령 인자. 도구·MCP·사용자 설정을 모두 끄고 JSON 스키마로 결정을 강제한다.
pub fn judge_args(request: &AutoRunJudgeRequest) -> Result<Vec<String>, String> {
    uuid::Uuid::parse_str(&request.session_id).map_err(|_| "auto-run-judge-session-invalid".to_string())?;
    let mut args: Vec<String> = [
        "-p",
        "--safe-mode",
        "--tools",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
        "--output-format",
        "json",
        "--json-schema",
        JUDGE_SCHEMA,
        if request.resume { "--resume" } else { "--session-id" },
        &request.session_id,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
        if invalid_model(model) {
            return Err("work-send-model-invalid".into());
        }
        args.extend(["--model".to_string(), model.trim().to_string()]);
    }
    Ok(args)
}

/// `--output-format json` 결과 한 덩어리 → 결정. 구조화 출력이 없으면 `result`
/// 본문을 JSON으로 다시 읽어 본다.
pub fn parse_judge_output(stdout: &str) -> Result<AutoRunJudgeResult, String> {
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).map_err(|_| "auto-run-judge-output".to_string())?;
    if value.get("is_error").and_then(|v| v.as_bool()) == Some(true) {
        let detail = value.get("result").and_then(|v| v.as_str()).unwrap_or_default();
        return Err(format!("auto-run-judge-error:{}", bounded(detail)));
    }
    let decision = value
        .get("structured_output")
        .filter(|v| v.is_object())
        .cloned()
        .or_else(|| {
            let text = value.get("result")?.as_str()?;
            serde_json::from_str::<serde_json::Value>(text.trim()).ok()
        })
        .ok_or_else(|| "auto-run-judge-output".to_string())?;
    let mut result: AutoRunJudgeResult =
        serde_json::from_value(decision).map_err(|_| "auto-run-judge-output".to_string())?;
    result.cost_usd = value.get("total_cost_usd").and_then(|v| v.as_f64());
    Ok(result)
}

fn bounded(text: &str) -> String {
    let line = text.trim().replace(['\r', '\n'], " ");
    if line.chars().count() > 300 {
        format!("{}…", line.chars().take(300).collect::<String>())
    } else {
        line
    }
}

/// 판단자 호출 1회. `program`은 claude 실행 파일(설정의 요약 명령 오버라이드를 따른다).
pub async fn judge(
    program: &str,
    dir: &Path,
    request: AutoRunJudgeRequest,
) -> Result<AutoRunJudgeResult, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let args = judge_args(&request)?;
    if request.prompt.trim().is_empty() {
        return Err("auto-run-judge-prompt-empty".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("auto-run-judge-dir:{}", bounded(&e.to_string())))?;
    // 번들 앱의 최소 PATH로는 claude를 못 찾는다(요약기와 같은 이유).
    let _ = tokio::task::spawn_blocking(crate::session::env_capture::ensure_captured).await;
    let mut command = tokio::process::Command::new(program);
    command
        .args(&args)
        .current_dir(dir)
        .env_remove("CLAUDECODE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "claude-not-found".to_string()
        } else {
            format!("auto-run-judge-spawn:{}", bounded(&error.to_string()))
        }
    })?;
    let prompt = request.prompt.into_bytes();
    let run = async move {
        let mut stdin = child.stdin.take().expect("stdin was piped");
        stdin.write_all(&prompt).await.map_err(|e| format!("auto-run-judge-io:{}", bounded(&e.to_string())))?;
        drop(stdin);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut out = child.stdout.take().expect("stdout was piped").take(JUDGE_STDOUT_MAX as u64);
        let mut err = child.stderr.take().expect("stderr was piped").take(64 * 1024);
        let (a, b) = tokio::join!(out.read_to_end(&mut stdout), err.read_to_end(&mut stderr));
        a.and(b).map_err(|e| format!("auto-run-judge-io:{}", bounded(&e.to_string())))?;
        let status = child.wait().await.map_err(|e| format!("auto-run-judge-io:{}", bounded(&e.to_string())))?;
        Ok::<_, String>((status, stdout, stderr))
    };
    let (status, stdout, stderr) = tokio::time::timeout(JUDGE_TIMEOUT, run)
        .await
        .map_err(|_| "auto-run-judge-timeout".to_string())??;
    let stdout = String::from_utf8_lossy(&stdout);
    // 실패해도 JSON 결과(is_error)가 있으면 그쪽 사유가 더 정확하다.
    match parse_judge_output(&stdout) {
        Ok(result) if status.success() => Ok(result),
        Err(error) if error.starts_with("auto-run-judge-error:") => Err(error),
        _ if !status.success() => Err(format!(
            "auto-run-judge-exit:{} {}",
            status.code().map_or("?".into(), |c| c.to_string()),
            bounded(&String::from_utf8_lossy(&stderr))
        )),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AutoRunDecision;

    fn text(role: ItemRole, body: &str) -> TranscriptItem {
        TranscriptItem::speech(role, body)
    }

    #[test]
    fn last_reply_joins_assistant_text_after_the_last_prompt() {
        let items = vec![
            text(ItemRole::User, "first task"),
            text(ItemRole::Assistant, "old answer"),
            text(ItemRole::User, "second task"),
            text(ItemRole::Assistant, "Looking at the code."),
            TranscriptItem::tool_use(Some("Bash".into()), "npm test"),
            TranscriptItem::tool_result("ok", false),
            text(ItemRole::Assistant, "sub agent chatter").with_sidechain(true),
            text(ItemRole::Assistant, "Done. Tests pass."),
        ];
        assert_eq!(
            extract_last_reply(&items).as_deref(),
            Some("Looking at the code.\n\nDone. Tests pass.")
        );
        // 도구 결과만 있고 응답이 없는 턴은 비어 있다.
        assert_eq!(extract_last_reply(&items[..3]), None);
    }

    #[test]
    fn long_reply_keeps_its_end() {
        let long = format!("{}END", "a".repeat(MAX_REPLY_CHARS + 10));
        let reply = extract_last_reply(&[text(ItemRole::Assistant, &long)]).unwrap();
        assert!(reply.ends_with("END"));
        assert_eq!(reply.chars().count(), MAX_REPLY_CHARS + 1);
    }

    #[test]
    fn tail_read_drops_the_cut_first_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(&path, "first line\nsecond\nthird\n").unwrap();
        assert_eq!(read_tail(&path, 13).as_deref(), Some("third\n"));
        assert_eq!(read_tail(&path, 1000).as_deref(), Some("first line\nsecond\nthird\n"));
    }

    fn request(resume: bool, model: Option<&str>) -> AutoRunJudgeRequest {
        AutoRunJudgeRequest {
            session_id: "3a21bfb6-9806-487f-9d37-c2bf8076e849".into(),
            resume,
            prompt: "judge".into(),
            model: model.map(str::to_string),
        }
    }

    #[test]
    fn judge_args_switch_between_new_and_resumed_session_without_tools() {
        let first = judge_args(&request(false, None)).unwrap();
        assert!(first.windows(2).any(|w| w == ["--session-id", "3a21bfb6-9806-487f-9d37-c2bf8076e849"]));
        assert!(first.windows(2).any(|w| w == ["--tools", ""]));
        let next = judge_args(&request(true, Some("sonnet"))).unwrap();
        assert!(next.windows(2).any(|w| w[0] == "--resume"));
        assert!(next.ends_with(&["--model".to_string(), "sonnet".to_string()]));
        // 인자로 해석될 모델 이름과 uuid가 아닌 세션은 거절한다.
        assert!(judge_args(&request(true, Some("--dangerously-skip-permissions"))).is_err());
        let mut bad = request(false, None);
        bad.session_id = "../x".into();
        assert!(judge_args(&bad).is_err());
    }

    /// 실제 Claude 전사 파일 하나로 마지막 응답을 뽑는다.
    /// `AO_TRANSCRIPT=<jsonl> cargo test ... real_transcript_last_reply -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_transcript_last_reply() {
        use crate::session_log::agent_transcript::{AgentSessionLookup, AgentSessionSnapshot};
        struct Fixed(String);
        impl AgentSessionLookup for Fixed {
            fn latest_session(&self, _: &str) -> Option<AgentSessionSnapshot> {
                Some(AgentSessionSnapshot { session_id: "x".into(), cwd: None, transcript_path: Some(self.0.clone()) })
            }
        }
        let path = std::env::var("AO_TRANSCRIPT").expect("AO_TRANSCRIPT");
        let mut source = crate::session_log::agent_transcript::claude::source(std::sync::Arc::new(Fixed(path))).unwrap();
        let reply = last_reply(source.as_mut(), "a", "/").expect("reply");
        println!("--- last reply ({} chars) ---\n{}", reply.chars().count(), reply.chars().take(600).collect::<String>());
    }

    /// 설치된 claude와 계정을 쓴다(작은 비용). `cargo test ... real_claude_judge -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn real_claude_judge_resumes_the_same_session() {
        let dir = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let ask = |resume: bool, prompt: &str| AutoRunJudgeRequest {
            session_id: id.clone(),
            resume,
            prompt: prompt.into(),
            model: Some("haiku".into()),
        };
        let first = judge("claude", dir.path(), ask(false, "You judge a worker's results. decision \"next\" = the answer is correct, \"followup\" = the worker must fix it (message = instruction), \"stop\" = only when a human must decide. Task: add 2+2. Worker reply: '2+2=4, checked with a calculator.'"))
            .await
            .unwrap();
        assert_eq!(first.decision, AutoRunDecision::Next);
        let second = judge("claude", dir.path(), ask(true, "Next task: add 3+3. Worker reply: '3+3=7'."))
            .await
            .unwrap();
        assert_ne!(second.decision, AutoRunDecision::Next);
        assert!(second.cost_usd.is_some());
    }

    #[test]
    fn judge_output_prefers_structured_output_and_reports_errors() {
        let ok = r#"{"type":"result","is_error":false,"result":"","structured_output":{"decision":"followup","message":"Run the tests","note":""},"total_cost_usd":0.0012}"#;
        let result = parse_judge_output(ok).unwrap();
        assert_eq!(result.decision, AutoRunDecision::Followup);
        assert_eq!(result.message, "Run the tests");
        assert_eq!(result.cost_usd, Some(0.0012));
        let fallback = r#"{"is_error":false,"result":"{\"decision\":\"next\",\"message\":\"\",\"note\":\"uses foo.rs\"}"}"#;
        assert_eq!(parse_judge_output(fallback).unwrap().note, "uses foo.rs");
        let error = r#"{"is_error":true,"result":"No conversation found"}"#;
        assert_eq!(parse_judge_output(error).unwrap_err(), "auto-run-judge-error:No conversation found");
        assert_eq!(parse_judge_output("not json").unwrap_err(), "auto-run-judge-output");
        let wrong = r#"{"is_error":false,"structured_output":{"decision":"maybe","message":"","note":""}}"#;
        assert_eq!(parse_judge_output(wrong).unwrap_err(), "auto-run-judge-output");
    }
}
