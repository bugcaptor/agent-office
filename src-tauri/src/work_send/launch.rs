//! 업무 보내기가 터미널에 입력하는 CLI 기동 문장. 예전 터미널 입력 자동화
//! (`automation/`, 점검·정리 §7.D C29로 제거)의 launchCli 단계와 같은 규칙을
//! 그대로 옮겨 왔다 — 에러 코드 문자열(`automation-*`)도 렌더러 카탈로그와
//! 맞물려 있어 바꾸지 않는다.
use super::cli_transition::{ps_quote, CommandSyntax};

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// CLI별로 받아 주는 노력/사고 수준인지 확인한다.
pub(crate) fn validate_launch_effort(cli: &str, effort: Option<&str>) -> Result<(), String> {
    let Some(effort) = effort else {
        return Ok(());
    };
    let valid = match cli {
        "agy" => matches!(effort, "low" | "medium" | "high"),
        "claude" | "codex" => matches!(effort, "low" | "medium" | "high" | "xhigh" | "max"),
        // Kilo's interactive TUI has no reasoning-effort flag. Rejecting a
        // saved value here avoids launching a command the CLI cannot accept.
        "kilo" => false,
        "pi" => matches!(effort, "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"),
        _ => return Err("automation-cli-profile-unsupported".into()),
    };
    if valid {
        Ok(())
    } else if cli == "agy" {
        Err("automation-agy-effort-invalid".into())
    } else {
        Err("automation-effort-invalid".into())
    }
}

/// `syntax` 셸에서 그대로 실행할 CLI 호출 문장. 인자 인용만 셸별로 다르다.
pub(crate) fn launch_command_for(
    cli: &str,
    model: Option<&str>,
    effort: Option<&str>,
    syntax: CommandSyntax,
) -> Result<String, String> {
    validate_launch_effort(cli, effort)?;
    let shell_quote = |value: &str| match syntax {
        CommandSyntax::Posix => shell_quote(value),
        CommandSyntax::PowerShell => ps_quote(value),
    };
    let mut command = match cli {
        "claude" => "claude".to_string(),
        "codex" => "codex".to_string(),
        "agy" => "agy --dangerously-skip-permissions".to_string(),
        // Kilo's interactive TUI accepts provider/model via --model. --auto
        // is required for unattended automation to progress through tool use.
        "kilo" => "kilo --auto".to_string(),
        // `--approve` is pi's unattended approval mode. Automation otherwise
        // stalls at tool-use approval prompts before it can write result.json.
        "pi" => "pi --approve".to_string(),
        _ => return Err("automation-cli-profile-unsupported".into()),
    };
    match cli {
        "claude" => {
            if let Some(model) = model {
                command.push_str(" --model ");
                command.push_str(&shell_quote(model));
            }
            if let Some(effort) = effort {
                command.push_str(" --effort ");
                command.push_str(&shell_quote(effort));
            }
        }
        "codex" => {
            if let Some(model) = model {
                command.push_str(" --model ");
                command.push_str(&shell_quote(model));
            }
            if let Some(effort) = effort {
                command.push_str(" -c ");
                // PowerShell 5.1은 네이티브 인자 안의 큰따옴표를 지운다. codex는 TOML로
                // 못 읽는 값을 문자열로 받으므로 PowerShell에서는 따옴표 없이 넘긴다.
                command.push_str(&shell_quote(&match syntax {
                    CommandSyntax::Posix => format!("model_reasoning_effort=\"{effort}\""),
                    CommandSyntax::PowerShell => format!("model_reasoning_effort={effort}"),
                }));
            }
        }
        "agy" => {
            if let Some(model) = model {
                command.push_str(" --model ");
                command.push_str(&shell_quote(model));
            }
            if let Some(effort) = effort {
                command.push_str(" --effort ");
                command.push_str(&shell_quote(effort));
            }
        }
        "kilo" => {
            if let Some(model) = model {
                command.push_str(" --model ");
                command.push_str(&shell_quote(model));
            }
        }
        "pi" => {
            if let Some(model) = model {
                command.push_str(" --model ");
                command.push_str(&shell_quote(model));
            }
            if let Some(effort) = effort {
                command.push_str(" --thinking ");
                command.push_str(&shell_quote(effort));
            }
        }
        _ => unreachable!(),
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn posix(cli: &str, model: Option<&str>, effort: Option<&str>) -> Result<String, String> {
        launch_command_for(cli, model, effort, CommandSyntax::Posix)
    }

    #[test]
    fn agy_accepts_its_documented_effort_levels() {
        for effort in ["low", "medium", "high"] {
            assert!(validate_launch_effort("agy", Some(effort)).is_ok(), "{effort}");
        }
    }

    #[test]
    fn kilo_rejects_unsupported_effort() {
        assert_eq!(
            validate_launch_effort("kilo", Some("high")),
            Err("automation-effort-invalid".into())
        );
    }

    #[test]
    fn pi_accepts_its_documented_thinking_levels() {
        for effort in ["off", "minimal", "low", "medium", "high", "xhigh", "max"] {
            assert!(validate_launch_effort("pi", Some(effort)).is_ok(), "{effort}");
        }
        assert_eq!(
            validate_launch_effort("pi", Some("ultra")),
            Err("automation-effort-invalid".into())
        );
    }

    #[test]
    fn launch_commands_preserve_model_and_effort() {
        assert_eq!(
            posix("claude", Some("opus"), Some("high")).unwrap(),
            "claude --model 'opus' --effort 'high'"
        );
        assert_eq!(
            posix("codex", Some("gpt-5"), Some("xhigh")).unwrap(),
            "codex --model 'gpt-5' -c 'model_reasoning_effort=\"xhigh\"'"
        );
        assert_eq!(
            posix("agy", Some("gemini-3.1-pro-low"), Some("high")).unwrap(),
            "agy --dangerously-skip-permissions --model 'gemini-3.1-pro-low' --effort 'high'"
        );
        assert_eq!(
            posix("agy", Some("gemini-3.8-flash-medium"), Some("medium")).unwrap(),
            "agy --dangerously-skip-permissions --model 'gemini-3.8-flash-medium' --effort 'medium'"
        );
        assert_eq!(
            posix("kilo", Some("openai/gpt-5"), None).unwrap(),
            "kilo --auto --model 'openai/gpt-5'"
        );
        assert_eq!(
            posix("pi", Some("openai/gpt-5"), Some("xhigh")).unwrap(),
            "pi --approve --model 'openai/gpt-5' --thinking 'xhigh'"
        );
        assert_eq!(
            posix("agy", None, Some("xhigh")),
            Err("automation-agy-effort-invalid".into())
        );
        assert!(posix("claude", None, Some("unsafe")).is_err());
    }

    #[test]
    fn powershell_launch_commands_use_powershell_quoting() {
        let ps = |cli, model, effort| launch_command_for(cli, model, effort, CommandSyntax::PowerShell);
        assert_eq!(
            ps("claude", Some("it's"), Some("high")).unwrap(),
            "claude --model 'it''s' --effort 'high'"
        );
        assert_eq!(
            ps("codex", Some("gpt-5"), Some("xhigh")).unwrap(),
            "codex --model 'gpt-5' -c 'model_reasoning_effort=xhigh'"
        );
        assert_eq!(
            ps("pi", Some("openai/gpt-5"), Some("xhigh")).unwrap(),
            "pi --approve --model 'openai/gpt-5' --thinking 'xhigh'"
        );
        assert!(ps("claude", None, Some("unsafe")).is_err());
    }
}
