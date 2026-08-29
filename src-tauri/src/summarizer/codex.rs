use super::{ProviderCommand, SummaryPurpose};
use crate::persistence::settings_store::SummaryProvider;

#[cfg(windows)]
const WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
$OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$c = Get-Command $env:AO_PROGRAM -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
$in = [Console]::In.ReadToEnd()
$effort = $env:AO_EFFORT
$config = if ($c.CommandType -eq 'Application') { 'model_reasoning_effort=\"' + $effort + '\"' } else { 'model_reasoning_effort="' + $effort + '"' }
$aoArgs = @('exec', '--ignore-user-config', '--ignore-rules', '--ephemeral', '--sandbox', 'read-only', '--model', $env:AO_MODEL, '--config', $config, '--skip-git-repo-check', '--color', 'never', '--', $env:AO_INSTRUCTION)
$in | & $c.Source @aoArgs
exit $LASTEXITCODE"#;

#[cfg(windows)]
pub(super) fn build(
    program: &str,
    instruction: &str,
    purpose: SummaryPurpose,
    model: &str,
) -> ProviderCommand {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]);
    command.creation_flags(CREATE_NO_WINDOW);
    command.env("AO_PROGRAM", program);
    command.env("AO_INSTRUCTION", instruction);
    command.env("AO_MODEL", model);
    command.env("AO_EFFORT", purpose.codex_effort());
    ProviderCommand {
        command,
        provider: SummaryProvider::Codex,
    }
}

#[cfg(not(windows))]
pub(super) fn build(
    program: &str,
    instruction: &str,
    purpose: SummaryPurpose,
    model: &str,
) -> ProviderCommand {
    let config = format!("model_reasoning_effort=\"{}\"", purpose.codex_effort());
    let mut command = std::process::Command::new(program);
    command.args([
        "exec",
        "--ignore-user-config",
        "--ignore-rules",
        "--ephemeral",
        "--sandbox",
        "read-only",
        "--model",
        model,
        "--config",
        config.as_str(),
        "--skip-git-repo-check",
        "--color",
        "never",
        "--",
        instruction,
    ]);
    ProviderCommand {
        command,
        provider: SummaryProvider::Codex,
    }
}

// ── 설정 화면의 모델 카탈로그(`list_provider_models`) ───────────────────────
//
// codex CLI에는 `opencode models` 같은 사람용 목록 커맨드가 없지만
// `codex debug models`가 **모델 카탈로그 원본을 JSON으로** 뱉는다(codex-cli
// 0.149 실측). 요약 실행 경로(`build`)와 달리 stdin도 세마포어도 쓰지 않는다.
//
// 출력은 슬러그당 시스템 프롬프트 전문까지 들어 있어 수백 KB다 — 그래서
// 파싱은 slug/visibility/priority 세 필드만 본다. `visibility`가 `"hide"`인
// 항목(내부용 `gpt-reserve`, `codex-auto-review` 등)은 사람이 고를 것이
// 아니므로 버린다. 정렬은 카탈로그가 주는 `priority` 오름차순 — CLI의 모델
// 선택 메뉴와 같은 순서다.
//
// `debug`는 이름 그대로 안정성을 보장하지 않는 서브커맨드다. 그래서 실패든
// 형식 변화든 전부 빈 목록으로 눌러 담고(호출측 model_catalog가 정적
// 프리셋으로 조용히 강등한다), 스키마도 필요한 필드만 느슨하게 읽는다.
#[cfg(windows)]
const MODELS_WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference='Stop'
$OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$c = Get-Command $env:AO_PROGRAM -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
& $c.Source debug models
exit $LASTEXITCODE"#;

#[cfg(windows)]
fn models_command(program: &str) -> tokio::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = tokio::process::Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        MODELS_WINDOWS_SCRIPT,
    ]);
    command.creation_flags(CREATE_NO_WINDOW);
    command.env("AO_PROGRAM", program);
    command
}

#[cfg(not(windows))]
fn models_command(program: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command.args(["debug", "models"]);
    command
}

/// `codex debug models` stdout(JSON) → 모델 slug 목록. 숨김 항목을 버리고
/// `priority` 오름차순으로 정렬한다(같은 값이면 카탈로그 순서 유지).
/// 파싱 실패는 오류가 아니라 빈 목록이다. 순수.
pub fn parse_models_stdout(stdout: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(stdout) else {
        return Vec::new();
    };
    let Some(models) = v.get("models").and_then(|m| m.as_array()) else {
        return Vec::new();
    };
    // priority가 없는 항목은 맨 뒤로 — 순서를 잃느니 목록 끝에 붙이는 편이 낫다.
    let mut rows: Vec<(i64, String)> = models
        .iter()
        .filter(|m| {
            // visibility 자체가 없으면 보여 준다 — 필드가 사라지는 쪽으로
            // 스키마가 바뀌었을 때 목록이 통째로 비는 것이 더 나쁘다.
            m.get("visibility")
                .and_then(|x| x.as_str())
                .map(|x| x != "hide")
                .unwrap_or(true)
        })
        .filter_map(|m| {
            let slug = m.get("slug").and_then(|s| s.as_str())?.trim();
            if slug.is_empty() {
                return None;
            }
            let priority = m
                .get("priority")
                .and_then(|p| p.as_i64())
                .unwrap_or(i64::MAX);
            Some((priority, slug.to_string()))
        })
        .collect();
    rows.sort_by_key(|(priority, _)| *priority);
    let mut out: Vec<String> = Vec::with_capacity(rows.len());
    for (_, slug) in rows {
        if !out.contains(&slug) {
            out.push(slug);
        }
    }
    out
}

/// 모델 목록 조회. 실패(미설치·타임아웃·비정상 종료·형식 변화)는 전부 빈
/// 목록이다 — opencode::list_models와 같은 계약이다.
pub async fn list_models(timeout: std::time::Duration, program: &str) -> Vec<String> {
    let mut command = models_command(program);
    command.current_dir(std::env::temp_dir());
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    command.kill_on_drop(true);

    let Ok(child) = command.spawn() else {
        return Vec::new();
    };
    let Ok(Ok(output)) = tokio::time::timeout(timeout, child.wait_with_output()).await else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_models_stdout(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `codex debug models`의 실측 모양(codex-cli 0.149) 축약본. 숨김 항목이
    /// 걸러지고 priority 오름차순으로 정렬되는지 고정한다.
    #[test]
    fn parse_models_stdout_keeps_listed_slugs_in_priority_order() {
        let stdout = r#"{"models":[
            {"slug":"gpt-5.4","visibility":"list","priority":16},
            {"slug":"gpt-reserve","visibility":"hide","priority":3},
            {"slug":"gpt-5.6-sol","visibility":"list","priority":1},
            {"slug":"gpt-5.4-mini","visibility":"list","priority":23},
            {"slug":"codex-auto-review","visibility":"hide","priority":43}
        ]}"#;
        assert_eq!(
            parse_models_stdout(stdout),
            vec![
                "gpt-5.6-sol".to_string(),
                "gpt-5.4".to_string(),
                "gpt-5.4-mini".to_string(),
            ]
        );
    }

    /// 필드가 빠져도 목록이 통째로 비면 안 된다 — visibility 없음은 보이는
    /// 것으로, priority 없음은 맨 뒤로.
    #[test]
    fn parse_models_stdout_tolerates_missing_optional_fields() {
        let stdout = r#"{"models":[
            {"slug":"no-priority"},
            {"slug":"first","visibility":"list","priority":2},
            {"slug":"  "},
            {"nope":true}
        ]}"#;
        assert_eq!(
            parse_models_stdout(stdout),
            vec!["first".to_string(), "no-priority".to_string()]
        );
    }

    /// `debug`는 안정성을 보장하지 않는 서브커맨드다 — 형식이 바뀌거나
    /// 쓰레기가 나와도 빈 목록으로 강등할 뿐 패닉하지 않는다.
    #[test]
    fn parse_models_stdout_on_garbage_or_empty_is_empty_list() {
        assert!(parse_models_stdout("").is_empty());
        assert!(parse_models_stdout("not json at all").is_empty());
        assert!(parse_models_stdout(r#"{"models":"nope"}"#).is_empty());
        assert!(parse_models_stdout(r#"{"other":[]}"#).is_empty());
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_models_command_uses_the_json_catalog_subcommand() {
        let cmd = models_command("codex");
        assert_eq!(cmd.as_std().get_program(), "codex");
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(args, vec!["debug", "models"]);
    }

    const DANGEROUS_INSTRUCTION: &str = "--dangerously-bypass-approvals-and-sandbox";

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
    fn codex_command_pins_low_cost_isolated_contract() {
        let spec = build("codex", "요약 지시", SummaryPurpose::Label, "gpt-5.4-mini");
        let rendered = command_debug(&spec.command);
        let config = "model_reasoning_effort=\"low\"";
        for expected in [
            "exec",
            "--ignore-user-config",
            "--ignore-rules",
            "--ephemeral",
            "--sandbox",
            "read-only",
            "--model",
            "gpt-5.4-mini",
            "--config",
            config,
            "--skip-git-repo-check",
            "--color",
            "never",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected}: {rendered}"
            );
        }
        assert!(!rendered.contains("luna"), "{rendered}");
        assert!(!rendered.contains("dangerously"), "{rendered}");
    }

    #[cfg(windows)]
    #[test]
    fn codex_command_terminates_options_before_dangerous_instruction() {
        let spec = build("codex", DANGEROUS_INSTRUCTION, SummaryPurpose::Label, "gpt-5.4-mini");
        let args: Vec<_> = spec
            .command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec!["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]
        );
        assert!(
            WINDOWS_SCRIPT.contains("'never', '--', $env:AO_INSTRUCTION"),
            "{WINDOWS_SCRIPT}"
        );
        assert!(
            WINDOWS_SCRIPT.contains("& $c.Source @aoArgs"),
            "{WINDOWS_SCRIPT}"
        );
        let instruction = spec
            .command
            .get_envs()
            .find(|(key, _)| *key == "AO_INSTRUCTION")
            .and_then(|(_, value)| value);
        assert_eq!(
            instruction,
            Some(std::ffi::OsStr::new(DANGEROUS_INSTRUCTION))
        );
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
    fn windows_powershell_native_boundary_preserves_exact_codex_argv() {
        let dir = std::env::temp_dir().join(format!("ao-codex-argv-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let capture = dir.join("argv.json");
        std::fs::write(
            dir.join("capture.js"),
            "const fs = require('fs'); fs.writeFileSync(process.env.AO_CAPTURE_FILE, JSON.stringify(process.argv.slice(2)), 'utf8');",
        )
        .unwrap();
        std::fs::write(
            dir.join("codex.cmd"),
            "@echo off\r\nnode \"%~dp0capture.js\" %*\r\nexit /b %ERRORLEVEL%\r\n",
        )
        .unwrap();

        let original_path = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(dir.clone()).chain(std::env::split_paths(&original_path)),
        )
        .unwrap();
        let mut spec = build("codex", DANGEROUS_INSTRUCTION, SummaryPurpose::Label, "gpt-5.4-mini");
        spec.command.env("PATH", path);
        spec.command.env("AO_CAPTURE_FILE", &capture);
        let output = spec.command.output().unwrap();
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let actual: Vec<String> =
            serde_json::from_slice(&std::fs::read(&capture).unwrap()).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(
            actual,
            vec![
                "exec",
                "--ignore-user-config",
                "--ignore-rules",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--model",
                "gpt-5.4-mini",
                "--config",
                "model_reasoning_effort=\"low\"",
                "--skip-git-repo-check",
                "--color",
                "never",
                "--",
                DANGEROUS_INSTRUCTION,
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_powershell_script_boundary_preserves_exact_codex_argv() {
        let dir = std::env::temp_dir().join(format!("ao-codex-ps1-argv-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let capture = dir.join("argv.json");
        std::fs::write(
            dir.join("codex.ps1"),
            r#"@($input) | Out-Null
[IO.File]::WriteAllText($env:AO_CAPTURE_FILE, (ConvertTo-Json -Compress -InputObject @($args)))
exit 0
"#,
        )
        .unwrap();

        let original_path = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(dir.clone()).chain(std::env::split_paths(&original_path)),
        )
        .unwrap();
        let mut spec = build("codex", DANGEROUS_INSTRUCTION, SummaryPurpose::Label, "gpt-5.4-mini");
        spec.command.env("PATH", path);
        spec.command.env("AO_CAPTURE_FILE", &capture);
        let output = spec.command.output().unwrap();
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let actual: Vec<String> =
            serde_json::from_slice(&std::fs::read(&capture).unwrap()).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(
            actual,
            vec![
                "exec",
                "--ignore-user-config",
                "--ignore-rules",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--model",
                "gpt-5.4-mini",
                "--config",
                "model_reasoning_effort=\"low\"",
                "--skip-git-repo-check",
                "--color",
                "never",
                "--",
                DANGEROUS_INSTRUCTION,
            ]
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn codex_command_terminates_options_before_dangerous_instruction() {
        let spec = build("codex", DANGEROUS_INSTRUCTION, SummaryPurpose::Label, "gpt-5.4-mini");
        assert_eq!(spec.command.get_program(), "codex");
        let args: Vec<_> = spec
            .command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "exec",
                "--ignore-user-config",
                "--ignore-rules",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--model",
                "gpt-5.4-mini",
                "--config",
                "model_reasoning_effort=\"low\"",
                "--skip-git-repo-check",
                "--color",
                "never",
                "--",
                DANGEROUS_INSTRUCTION,
            ]
        );
    }

    /// 실 CLI 스모크 — `codex debug models`의 출력 형식이 바뀌지 않았는지
    /// 사람이 확인할 때 쓴다(`debug`는 안정성 보장이 없는 서브커맨드다).
    #[tokio::test]
    #[ignore = "codex CLI 실행 필요(수동 스모크)"]
    async fn live_catalog_smoke() {
        let models = list_models(std::time::Duration::from_secs(30), "codex").await;
        assert!(
            !models.is_empty(),
            "빈 목록 -- 출력 형식이 바뀌었을 수 있다"
        );
        println!("{models:?}");
    }
}
