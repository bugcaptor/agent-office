use super::ProviderCommand;
use crate::persistence::settings_store::SummaryProvider;
use std::sync::atomic::{AtomicU64, Ordering};

static MODEL_REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn initialize_request_id() -> String {
    format!(
        "agent-office-models-{}",
        MODEL_REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

/// Agent SDK의 Query.supportedModels()가 받는 initialize 응답에서 실제 계정의
/// CLI 선택값(`value`)만 순서대로 꺼낸다.
fn parse_initialize_response(line: &str, request_id: &str) -> Option<Vec<String>> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "control_response"
        || value.pointer("/response/request_id")?.as_str()? != request_id
    {
        return None;
    }
    let response = value.get("response")?;
    if response.get("subtype")?.as_str()? != "success" {
        return Some(Vec::new());
    }
    let payload = response.get("response")?;
    let Some(rows) = payload.get("models").and_then(|value| value.as_array()) else {
        return Some(Vec::new());
    };
    let mut models = Vec::new();
    for model in rows {
        let Some(id) = model.get("value").and_then(|value| value.as_str()) else {
            continue;
        };
        let id = id.trim();
        if !id.is_empty() && !models.iter().any(|known| known == id) {
            models.push(id.to_string());
        }
    }
    Some(models)
}

fn models_command(program: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command.args([
        // safe-mode keeps normal account authentication and model selection while
        // disabling hooks, plugins, skills, custom commands, and MCP servers.
        "--safe-mode",
        // Keep an explicit empty MCP configuration as a second boundary for CLI
        // versions where safe-mode behavior changes.
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
        "--output-format",
        "stream-json",
        "--verbose",
        "--input-format",
        "stream-json",
    ]);
    command.env_remove("CLAUDECODE");
    command.env("CLAUDE_CODE_ENTRYPOINT", "sdk-ts");
    command
}

/// Claude에는 사람용 목록 서브커맨드가 없으므로 SDK control initialize만 보낸다.
/// 프롬프트는 보내지 않고, 응답을 받는 즉시 프로세스를 종료한다.
pub async fn list_models(timeout: std::time::Duration, program: &str) -> Vec<String> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    let mut command = models_command(program);
    command.current_dir(std::env::temp_dir());
    command.stdin(std::process::Stdio::piped());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::null());
    command.kill_on_drop(true);
    let Ok(mut child) = command.spawn() else {
        return Vec::new();
    };
    let request_id = initialize_request_id();
    let request = serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": { "subtype": "initialize", "hooks": null },
    });
    let result = tokio::time::timeout(timeout, async {
        let mut stdin = child.stdin.take()?;
        stdin.write_all(request.to_string().as_bytes()).await.ok()?;
        stdin.write_all(b"\n").await.ok()?;
        stdin.flush().await.ok()?;
        // stdin은 initialize 응답 전 EOF가 되지 않게 이 scope 안에서 유지한다.
        let stdout = child.stdout.take()?;
        // JSONL도 상한을 둔다. 비정상 출력은 EOF처럼 끝내고 Child drop이 kill한다.
        let stdout = stdout.take(super::MODEL_CATALOG_STDOUT_MAX_BYTES + 1);
        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = lines.next_line().await.ok()? {
            if let Some(models) = parse_initialize_response(&line, &request_id) {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Some(models);
            }
        }
        None
    })
    .await
    .ok()
    .flatten();
    result.unwrap_or_default()
}

#[cfg(windows)]
const WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
$OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$c = Get-Command $env:AO_PROGRAM -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
$in = [Console]::In.ReadToEnd()
$in | & $c.Source -p $env:AO_INSTRUCTION --model $env:AO_MODEL --effort low --output-format text --max-turns 1
exit $LASTEXITCODE"#;

#[cfg(windows)]
pub(super) fn build(program: &str, instruction: &str, model: &str) -> ProviderCommand {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]);
    command.creation_flags(CREATE_NO_WINDOW);
    command.env("AO_PROGRAM", program);
    command.env("AO_INSTRUCTION", instruction);
    command.env("AO_MODEL", model);
    ProviderCommand {
        command,
        provider: SummaryProvider::Claude,
    }
}

#[cfg(not(windows))]
pub(super) fn build(program: &str, instruction: &str, model: &str) -> ProviderCommand {
    let mut command = std::process::Command::new(program);
    command.args([
        "-p",
        instruction,
        "--model",
        model,
        "--effort",
        "low",
        "--output-format",
        "text",
        "--max-turns",
        "1",
    ]);
    ProviderCommand {
        command,
        provider: SummaryProvider::Claude,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_response_uses_only_account_model_values() {
        let request_id = "request-7";
        let line = r#"{"type":"control_response","response":{"request_id":"request-7","subtype":"success","response":{"models":[{"value":"default","displayName":"Default"},{"value":"sonnet"},{"value":"sonnet"},{"value":"  "}]}}}"#;
        assert_eq!(
            parse_initialize_response(line, request_id),
            Some(vec!["default".into(), "sonnet".into()])
        );
    }

    #[test]
    fn initialize_response_ignores_other_messages_and_fails_closed() {
        assert_eq!(
            parse_initialize_response("{\"type\":\"system\"}", "r"),
            None
        );
        let error = r#"{"type":"control_response","response":{"request_id":"r","subtype":"error","response":{}}}"#;
        assert_eq!(parse_initialize_response(error, "r"), Some(Vec::new()));
    }

    #[test]
    fn models_command_uses_sdk_control_mode_without_claudecode() {
        let command = models_command("claude");
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            [
                "--safe-mode",
                "--strict-mcp-config",
                "--mcp-config",
                r#"{"mcpServers":{}}"#,
                "--output-format",
                "stream-json",
                "--verbose",
                "--input-format",
                "stream-json"
            ]
        );
        assert!(command
            .as_std()
            .get_envs()
            .any(|(key, value)| key == "CLAUDECODE" && value.is_none()));
        assert!(command
            .as_std()
            .get_envs()
            .all(|(key, _)| key != "CLAUDE_AGENT_SDK_VERSION"));
    }

    fn command_debug(command: &std::process::Command) -> String {
        let mut parts = vec![command.get_program().to_string_lossy().into_owned()];
        parts.extend(
            command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned()),
        );
        parts.extend(command.get_envs().filter_map(|(key, value)| {
            value.map(|value| format!("{}={}", key.to_string_lossy(), value.to_string_lossy()))
        }));
        parts.join(" ")
    }

    #[test]
    fn claude_command_pins_existing_behavior() {
        let spec = build("claude", "요약 지시", "haiku");
        let rendered = command_debug(&spec.command);
        assert!(rendered.contains("haiku"), "{rendered}");
        assert!(rendered.contains("--output-format"), "{rendered}");
        assert!(rendered.contains("--effort"), "{rendered}");
        assert!(rendered.contains("low"), "{rendered}");
        assert!(rendered.contains("text"), "{rendered}");
        assert!(rendered.contains("--max-turns"), "{rendered}");
        assert!(rendered.contains("1"), "{rendered}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_script_pins_bomless_utf8_output_encoding() {
        assert!(
            WINDOWS_SCRIPT.contains("$OutputEncoding=New-Object System.Text.UTF8Encoding($false)")
        );
        assert!(!WINDOWS_SCRIPT.contains("$OutputEncoding=[System.Text.Encoding]::UTF8"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_script_reads_stdin_to_eof_before_invoking_provider() {
        let gate = WINDOWS_SCRIPT.find("[Console]::In.ReadToEnd()").unwrap();
        let invocation = WINDOWS_SCRIPT.find("$in | & $c.Source").unwrap();
        assert!(gate < invocation, "{WINDOWS_SCRIPT}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_command_uses_powershell_with_no_window_flag_and_env_instruction() {
        let spec = build("claude", "요약 지시", "haiku");
        let cmd = spec.command;
        assert_eq!(cmd.get_program(), "powershell.exe");
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            vec!["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]
        );
        let env_val = cmd
            .get_envs()
            .find(|(k, _)| *k == "AO_INSTRUCTION")
            .and_then(|(_, v)| v);
        assert_eq!(env_val, Some(std::ffi::OsStr::new("요약 지시")));
        assert!(WINDOWS_SCRIPT.contains("--effort low"), "{WINDOWS_SCRIPT}");
    }

    /// 모델은 이제 호출측(`summarizer::resolve_model`)이 정해 넘긴다 — 여기서는
    /// 그 값이 왜곡 없이 커맨드로 실리는지만 고정한다(목적별 기본값·설정
    /// 오버라이드 규칙은 mod.rs의 `resolve_model` 테스트가 지킨다).
    #[cfg(not(windows))]
    #[test]
    fn explicit_model_is_passed_through() {
        let spec = build("claude", "학습자료 지시", "sonnet");
        let rendered = command_debug(&spec.command);
        assert!(rendered.contains("sonnet"), "{rendered}");
        assert!(!rendered.contains("haiku"), "{rendered}");
    }

    /// 커스텀 실행 명령(별개 계정 래퍼 `claude-t` 같은 것)이 그대로 프로그램이
    /// 돼야 한다 — 인자 규약은 건드리지 않는다.
    #[cfg(not(windows))]
    #[test]
    fn custom_program_replaces_the_binary_but_keeps_the_arguments() {
        let spec = build("claude-t", "요약 지시", "haiku");
        assert_eq!(spec.command.get_program(), "claude-t");
        let args: Vec<_> = spec
            .command
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(args[0], "-p");
        assert!(args.contains(&"--max-turns".to_string()), "{args:?}");
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_command_passes_instruction_and_model_flags() {
        let spec = build("claude", "요약 지시", "haiku");
        let cmd = spec.command;
        assert_eq!(cmd.get_program(), "claude");
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            vec![
                "-p",
                "요약 지시",
                "--model",
                "haiku",
                "--effort",
                "low",
                "--output-format",
                "text",
                "--max-turns",
                "1"
            ]
        );
    }
}
