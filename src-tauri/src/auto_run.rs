//! 자동 진행의 네이티브 조각: 작업자 세션의 마지막 응답을
//! 에이전트 전사에서 읽고, 판단자(`claude -p` 이어지는 세션)를 한 번 부른다.
//! 루프와 상태는 렌더러가 쥔다.

use crate::session_log::agent_transcript::{ItemKind, ItemRole, TranscriptItem, TranscriptSource};
use crate::types::{AutoRunDecision, AutoRunJudgeRequest, AutoRunJudgeResult};
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

/// 판단자에게 넘기는 보고 상한(문자). 넘으면 뒤쪽을 자른다 — 형식상 상태가 맨 앞이다.
pub const MAX_REPORT_CHARS: usize = 16_000;
/// 이보다 오래된 보고는 읽을 때 지운다. 끝난 실행의 보고가 쌓이지 않게 한다.
const REPORT_KEEP: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// 작업자가 `office-task --report`로 남기는 보고가 사는 폴더. 수신함 아래에 두어
/// Codex 쓰기 허용 경로와 Claude 사전 승인 명령을 그대로 쓴다. 수신함 scan은 파일만 본다.
pub fn reports_dir(inbox: &Path) -> PathBuf {
    inbox.join("reports")
}

/// 그 태스크의 보고. `since_ms`(이번 턴을 보낸 시각, epoch ms)보다 먼저 쓰인 것은 지난
/// 턴의 보고이므로 읽지 않는다.
pub fn read_report(inbox: &Path, task_id: &str, since_ms: u64, now: std::time::SystemTime) -> Option<String> {
    let dir = reports_dir(inbox);
    prune_reports(&dir, now);
    if !crate::persistence::work_inbox::valid_task_id(task_id) {
        return None;
    }
    let path = dir.join(format!("{task_id}.md"));
    let modified = std::fs::symlink_metadata(&path).ok().filter(|m| m.is_file())?.modified().ok()?;
    let written_ms = modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    if written_ms < u128::from(since_ms) {
        return None;
    }
    let text = std::fs::read_to_string(&path).ok()?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(if text.chars().count() > MAX_REPORT_CHARS {
        format!("{}…", text.chars().take(MAX_REPORT_CHARS).collect::<String>())
    } else {
        text.to_string()
    })
}

fn prune_reports(dir: &Path, now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .ok()
            .filter(|m| m.is_file())
            .and_then(|m| m.modified().ok())
            .and_then(|at| now.duration_since(at).ok())
            .is_some_and(|age| age > REPORT_KEEP);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

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

/// `--output-format json` 결과 한 덩어리 → 결정. 구조화 출력이 없거나 틀렸으면 `result`
/// 본문에서 결정 JSON을 관대하게 찾아 본다(`decision_from_text`).
pub fn parse_judge_output(stdout: &str) -> Result<AutoRunJudgeResult, String> {
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).map_err(|_| "auto-run-judge-output".to_string())?;
    if value.get("is_error").and_then(|v| v.as_bool()) == Some(true) {
        let detail = value.get("result").and_then(|v| v.as_str()).unwrap_or_default();
        return Err(format!("auto-run-judge-error:{}", bounded(detail)));
    }
    let mut result = value
        .get("structured_output")
        .and_then(decision_from_value)
        .or_else(|| decision_from_text(value.get("result")?.as_str()?))
        .ok_or_else(|| "auto-run-judge-output".to_string())?;
    result.cost_usd = output_cost(&value);
    Ok(result)
}

fn output_cost(value: &serde_json::Value) -> Option<f64> {
    value.get("total_cost_usd").and_then(|v| v.as_f64())
}

/// 결정 객체 하나를 읽는다. 대소문자·공백이 섞인 결정 이름은 받아 주고, 지시가 빈
/// `followup`은 쓸 수 없으니 읽지 못한 것으로 친다.
fn decision_from_value(value: &serde_json::Value) -> Option<AutoRunJudgeResult> {
    let mut value = value.clone();
    let decision = value.get_mut("decision")?;
    *decision = serde_json::Value::String(decision.as_str()?.trim().to_lowercase());
    let result: AutoRunJudgeResult = serde_json::from_value(value).ok()?;
    if result.decision == AutoRunDecision::Followup && result.message.trim().is_empty() {
        return None;
    }
    Some(result)
}

/// 본문 전체, 코드펜스 안, 첫 `{`부터 마지막 `}`까지 순서로 결정 JSON을 찾는다.
pub fn decision_from_text(text: &str) -> Option<AutoRunJudgeResult> {
    let text = text.trim();
    let fenced = text
        .split_once("```")
        .map(|(_, rest)| rest.trim_start_matches("json"))
        .and_then(|rest| rest.split_once("```"))
        .map(|(inner, _)| inner);
    let braced = text.find('{').zip(text.rfind('}')).filter(|(a, b)| a < b).map(|(a, b)| &text[a..=b]);
    [Some(text), fenced, braced]
        .into_iter()
        .flatten()
        .filter_map(|candidate| serde_json::from_str::<serde_json::Value>(candidate.trim()).ok())
        .find_map(|value| decision_from_value(&value))
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
/// 답을 결정으로 읽지 못하면 같은 세션에 결정만 다시 달라고 한 번 더 묻는다.
pub async fn judge(
    program: &str,
    dir: &Path,
    request: AutoRunJudgeRequest,
) -> Result<AutoRunJudgeResult, String> {
    let args = judge_args(&request)?;
    if request.prompt.trim().is_empty() {
        return Err("auto-run-judge-prompt-empty".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("auto-run-judge-dir:{}", bounded(&e.to_string())))?;
    // 번들 앱의 최소 PATH로는 claude를 못 찾는다(요약기와 같은 이유).
    let _ = tokio::task::spawn_blocking(crate::session::env_capture::ensure_captured).await;
    let stdout = run_judge(program, dir, &args, request.prompt.clone()).await??;
    let first_error = match parse_judge_output(&stdout) {
        Ok(result) => return Ok(result),
        Err(error) => error,
    };
    if first_error != "auto-run-judge-output" {
        return Err(first_error);
    }
    // 첫 호출이었어도 세션은 이미 생겼다. 이어서 묻는다.
    let first_cost = serde_json::from_str::<serde_json::Value>(stdout.trim()).ok().as_ref().and_then(output_cost);
    let retry = judge_args(&AutoRunJudgeRequest { resume: true, prompt: String::new(), ..request })?;
    let stdout = run_judge(program, dir, &retry, JUDGE_REPAIR_PROMPT.to_string()).await??;
    let mut result = parse_judge_output(&stdout)?;
    if let Some(cost) = first_cost {
        result.cost_usd = Some(result.cost_usd.unwrap_or(0.0) + cost);
    }
    Ok(result)
}

/// 결정을 읽지 못했을 때 같은 판단자 세션에 보내는 재요청.
pub const JUDGE_REPAIR_PROMPT: &str = "Your last answer could not be read as a decision. Answer the same turn again with only the JSON object {\"decision\": \"next\" | \"followup\" | \"stop\", \"message\": string, \"note\": string}. A \"followup\" needs a non-empty \"message\".";

/// claude를 한 번 돌린다. 바깥 `Err`는 실행 자체의 실패, 안쪽은 `is_error`·종료 코드 실패다.
/// 정상 종료면 stdout을 돌려준다.
async fn run_judge(
    program: &str,
    dir: &Path,
    args: &[String],
    prompt: String,
) -> Result<Result<String, String>, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
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
    let prompt = prompt.into_bytes();
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
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    if status.success() {
        return Ok(Ok(stdout));
    }
    // 실패해도 JSON 결과(is_error)가 있으면 그쪽 사유가 더 정확하다.
    Ok(Err(match parse_judge_output(&stdout) {
        Err(error) if error.starts_with("auto-run-judge-error:") => error,
        _ => format!(
            "auto-run-judge-exit:{} {}",
            status.code().map_or("?".into(), |c| c.to_string()),
            bounded(&String::from_utf8_lossy(&stderr))
        ),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(role: ItemRole, body: &str) -> TranscriptItem {
        TranscriptItem::speech(role, body)
    }

    #[test]
    fn report_is_read_only_when_written_this_turn_and_old_ones_are_pruned() {
        use std::time::{Duration, SystemTime, UNIX_EPOCH};
        let inbox = tempfile::tempdir().unwrap();
        let dir = reports_dir(inbox.path());
        std::fs::create_dir_all(&dir).unwrap();
        let at = |path: &Path, time: SystemTime| std::fs::File::options().write(true).open(path).unwrap().set_modified(time).unwrap();
        let now = SystemTime::now();
        let since = (now - Duration::from_secs(60)).duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        std::fs::write(dir.join("t-1.md"), "  status: done\n").unwrap();
        at(&dir.join("t-1.md"), now);
        assert_eq!(read_report(inbox.path(), "t-1", since, now).as_deref(), Some("status: done"));
        // 이번 턴을 보내기 전에 쓰인 보고는 지난 턴의 것이다.
        at(&dir.join("t-1.md"), now - Duration::from_secs(120));
        assert_eq!(read_report(inbox.path(), "t-1", since, now), None);
        assert_eq!(read_report(inbox.path(), "../t-1", 0, now), None);
        assert_eq!(read_report(inbox.path(), "t-missing", 0, now), None);
        std::fs::write(dir.join("t-long.md"), "가".repeat(MAX_REPORT_CHARS + 5)).unwrap();
        assert_eq!(read_report(inbox.path(), "t-long", 0, now).unwrap().chars().count(), MAX_REPORT_CHARS + 1);
        std::fs::write(dir.join("t-old.md"), "old").unwrap();
        at(&dir.join("t-old.md"), now - REPORT_KEEP - Duration::from_secs(1));
        read_report(inbox.path(), "t-1", 0, now);
        assert!(!dir.join("t-old.md").exists());
        assert!(dir.join("t-1.md").exists());
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

    #[test]
    fn decision_text_is_found_in_fences_and_prose() {
        let fenced = "Here you go:\n```json\n{\"decision\":\"Next\",\"message\":\"\",\"note\":\"n\"}\n```";
        assert_eq!(decision_from_text(fenced).unwrap().decision, AutoRunDecision::Next);
        let prose = "I think {\"decision\": \" stop \", \"message\": \"ask the human\", \"note\": \"\"} is right.";
        assert_eq!(decision_from_text(prose).unwrap().message, "ask the human");
        // 지시가 빈 followup은 쓸 수 없다.
        assert!(decision_from_text(r#"{"decision":"followup","message":" ","note":""}"#).is_none());
        assert!(decision_from_text("next").is_none());
        // 구조화 출력이 틀렸어도 본문이 맞으면 읽는다.
        let both = r#"{"is_error":false,"structured_output":{"decision":"maybe"},"result":"{\"decision\":\"next\",\"message\":\"\",\"note\":\"\"}"}"#;
        assert_eq!(parse_judge_output(both).unwrap().decision, AutoRunDecision::Next);
    }

    /// 가짜 claude: 첫 호출은 결정 없이 답하고, 이어지는 재요청에 결정을 준다.
    #[cfg(unix)]
    #[tokio::test]
    async fn unreadable_answer_is_asked_again_in_the_same_session() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("claude");
        std::fs::write(
            &fake,
            r#"#!/bin/sh
cat > "prompt-$#-$(ls | wc -l | tr -d ' ')"
echo "$@" >> args.log
case "$*" in
  *--resume*) echo '{"is_error":false,"result":"","structured_output":{"decision":"followup","message":"Run the tests","note":""},"total_cost_usd":0.25}' ;;
  *) echo '{"is_error":false,"result":"Looks fine to me.","total_cost_usd":0.5}' ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let work = dir.path().join("judge");
        let result = judge(fake.to_str().unwrap(), &work, request(false, None)).await.unwrap();
        assert_eq!(result.decision, AutoRunDecision::Followup);
        assert_eq!(result.cost_usd, Some(0.75));
        let log = std::fs::read_to_string(work.join("args.log")).unwrap();
        let calls: Vec<&str> = log.lines().collect();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].contains("--session-id 3a21bfb6"));
        assert!(calls[1].contains("--resume 3a21bfb6"));
        let prompts: Vec<String> = std::fs::read_dir(&work)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("prompt-"))
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .collect();
        assert!(prompts.iter().any(|p| p == JUDGE_REPAIR_PROMPT));

        // 재요청도 읽지 못하면 그대로 실패한다.
        let stubborn = dir.path().join("stubborn");
        std::fs::write(&stubborn, "#!/bin/sh\ncat >/dev/null\necho '{\"is_error\":false,\"result\":\"hmm\"}'\n").unwrap();
        std::fs::set_permissions(&stubborn, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = judge(stubborn.to_str().unwrap(), &work, request(false, None)).await.unwrap_err();
        assert_eq!(error, "auto-run-judge-output");
    }
}
