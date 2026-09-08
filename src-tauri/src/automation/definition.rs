use crate::types::{AutomationDefinition, AutomationStep};
use std::collections::HashSet;

pub const AUTOMATION_SCHEMA_VERSION_V1: u32 = 1;
pub const AUTOMATION_SCHEMA_VERSION_V2: u32 = 2;
const RESERVED_INPUT_KEYS: [&str; 4] = ["workspace", "run", "cycle", "previousResult"];

/// A statically matched v2 CLI interval.  The runner owns dynamic launch IDs;
/// this plan only binds the definition's stable step IDs and positions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliSegment {
    pub launch_step_index: usize,
    pub launch_step_id: String,
    pub exit_step_index: usize,
    pub exit_step_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefinitionPlan {
    V1,
    V2 { segments: Vec<CliSegment> },
}

pub fn validate(definition: &AutomationDefinition) -> Result<(), String> {
    compile(definition).map(|_| ())
}

/// Validates a saved definition and builds the scheduling contract consumed by
/// the v2 runner.  It deliberately does not normalize v1 into v2.
pub fn compile(definition: &AutomationDefinition) -> Result<DefinitionPlan, String> {
    if !matches!(
        definition.schema_version,
        AUTOMATION_SCHEMA_VERSION_V1 | AUTOMATION_SCHEMA_VERSION_V2
    ) {
        return Err("automation-schema-version-unsupported".into());
    }
    if definition.id.trim().is_empty() || definition.name.trim().is_empty() {
        return Err("automation-definition-name-required".into());
    }
    if definition.steps.is_empty() {
        return Err("automation-definition-steps-required".into());
    }
    if definition
        .repeat
        .as_ref()
        .is_some_and(|r| r.max_cycles == 0)
    {
        return Err("automation-repeat-must-be-finite-positive".into());
    }
    let mut ids = HashSet::new();
    for input in &definition.inputs {
        if input.key.trim().is_empty()
            || RESERVED_INPUT_KEYS.contains(&input.key.as_str())
            || !ids.insert(format!("input:{}", input.key))
        {
            return Err("automation-input-key-invalid".into());
        }
    }
    ids.clear();
    for step in &definition.steps {
        if let AutomationStep::LaunchCli {
            cli_profile_id,
            effort,
            ..
        } = step
        {
            validate_launch_effort(cli_profile_id, effort.as_deref())?;
        }
        let id = match step {
            AutomationStep::LaunchCli { id, .. }
            | AutomationStep::LlmTask { id, .. }
            | AutomationStep::Wait { id, .. }
            | AutomationStep::Confirm { id, .. }
            | AutomationStep::ExitCli { id, .. } => id,
        };
        if id.trim().is_empty() || !ids.insert(id.to_string()) {
            return Err("automation-step-id-invalid".into());
        }
        match step {
            AutomationStep::LaunchCli {
                cli_profile_id,
                startup_wait_ms,
                ..
            } if !matches!(cli_profile_id.as_str(), "claude" | "codex" | "agy" | "kilo" | "pi")
                || startup_wait_ms.is_some_and(|v| v == 0) =>
            {
                return Err("automation-launch-invalid".into())
            }
            AutomationStep::LlmTask {
                label,
                prompt_template,
                wait_timeout_ms,
                completion_grace_ms,
                ..
            } if label.trim().is_empty()
                || prompt_template.trim().is_empty()
                || wait_timeout_ms.is_some_and(|v| v == 0)
                || completion_grace_ms.is_some_and(|v| v == 0) =>
            {
                return Err("automation-llm-task-invalid".into())
            }
            AutomationStep::Wait { duration_ms, .. } if *duration_ms == 0 => {
                return Err("automation-wait-invalid".into())
            }
            AutomationStep::Confirm { message, .. } if message.trim().is_empty() => {
                return Err("automation-confirm-invalid".into())
            }
            AutomationStep::ExitCli {
                command,
                exit_wait_ms,
                ..
            } if command.trim().is_empty() || exit_wait_ms.is_some_and(|value| value == 0) => {
                return Err("automation-exit-invalid".into())
            }
            _ => {}
        }
    }
    match definition.schema_version {
        AUTOMATION_SCHEMA_VERSION_V1 => compile_v1(definition),
        AUTOMATION_SCHEMA_VERSION_V2 => compile_v2(definition),
        _ => unreachable!("schema version checked above"),
    }
}

/// CLI마다 실제로 받는 추론 강도만 허용한다. agy는 `low`/`medium`/`high`
/// 셋뿐인데 공용 입력 검증이 Codex의 `xhigh`/`max`까지 통과시켜, 실행 때에야
/// CLI 오류가 나던 문제를 막는다.
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

fn compile_v1(definition: &AutomationDefinition) -> Result<DefinitionPlan, String> {
    for (index, step) in definition.steps.iter().enumerate() {
        match step {
            AutomationStep::LaunchCli { .. } if index != 0 => {
                return Err("automation-launch-must-be-first".into());
            }
            AutomationStep::ExitCli { .. } if index + 1 != definition.steps.len() => {
                return Err("automation-exit-must-be-last".into());
            }
            AutomationStep::ExitCli {
                return_mode: Some(_),
                ..
            }
            | AutomationStep::ExitCli {
                exit_wait_ms: Some(_),
                ..
            } => return Err("automation-v1-exit-options-unsupported".into()),
            _ => {}
        }
    }
    Ok(DefinitionPlan::V1)
}

fn v2_error(index: usize, id: &str, reason: &str) -> String {
    format!("automation-v2-invalid:{index}:{id}:{reason}")
}

fn compile_v2(definition: &AutomationDefinition) -> Result<DefinitionPlan, String> {
    let mut open: Option<(usize, String)> = None;
    let mut segments = Vec::new();
    for (index, step) in definition.steps.iter().enumerate() {
        match step {
            AutomationStep::LaunchCli { id, .. } => {
                if open.is_some() {
                    return Err(v2_error(index, id, "launch-not-shell"));
                }
                open = Some((index, id.clone()));
            }
            AutomationStep::LlmTask { id, .. } if open.is_none() => {
                return Err(v2_error(index, id, "llm-task-not-cli"));
            }
            AutomationStep::ExitCli { id, .. } => {
                let Some((launch_step_index, launch_step_id)) = open.take() else {
                    return Err(v2_error(index, id, "exit-not-cli"));
                };
                segments.push(CliSegment {
                    launch_step_index,
                    launch_step_id,
                    exit_step_index: index,
                    exit_step_id: id.clone(),
                });
            }
            _ => {}
        }
    }
    if let Some((index, id)) = open {
        return Err(v2_error(index, &id, "cli-not-closed"));
    }
    if segments.is_empty() {
        return Err("automation-v2-cli-segment-required".into());
    }
    Ok(DefinitionPlan::V2 { segments })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    #[test]
    fn rejects_unbounded_repeat_and_bad_wait() {
        let d = AutomationDefinition {
            schema_version: 1,
            id: "a".into(),
            revision: 1,
            name: "a".into(),
            inputs: vec![],
            steps: vec![AutomationStep::Wait {
                id: "s".into(),
                duration_ms: 0,
            }],
            repeat: Some(AutomationRepeat { max_cycles: 0 }),
            workspace: None,
            input_values: None,
        };
        assert!(validate(&d).is_err());
    }

    #[test]
    fn rejects_reserved_inputs_and_unknown_cli() {
        let base = |key: &str, cli: &str| AutomationDefinition {
            schema_version: 1,
            id: "a".into(),
            revision: 1,
            name: "a".into(),
            inputs: vec![AutomationInput {
                key: key.into(),
                label: "x".into(),
                default: None,
            }],
            steps: vec![AutomationStep::LaunchCli {
                id: "s".into(),
                cli_profile_id: cli.into(),
                model: None,
                effort: None,
                startup_wait_ms: None,
            }],
            repeat: None,
            workspace: None,
            input_values: None,
        };
        assert_eq!(
            validate(&base("workspace", "claude")),
            Err("automation-input-key-invalid".into())
        );
        assert_eq!(
            validate(&base("task", "other")),
            Err("automation-launch-invalid".into())
        );
        assert!(validate(&base("task", "codex")).is_ok());
        assert!(validate(&base("task", "agy")).is_ok());
        assert!(validate(&base("task", "kilo")).is_ok());
        assert!(validate(&base("task", "pi")).is_ok());
        assert!(validate(&base("previousResult", "codex")).is_err());
        let mut misplaced = base("task", "claude");
        misplaced.steps.insert(
            0,
            AutomationStep::Wait {
                id: "wait".into(),
                duration_ms: 1,
            },
        );
        assert_eq!(
            validate(&misplaced),
            Err("automation-launch-must-be-first".into())
        );
        misplaced.steps = vec![
            AutomationStep::ExitCli {
                id: "exit".into(),
                command: "/exit".into(),
                return_mode: None,
                exit_wait_ms: None,
            },
            AutomationStep::Wait {
                id: "wait".into(),
                duration_ms: 1,
            },
        ];
        assert_eq!(
            validate(&misplaced),
            Err("automation-exit-must-be-last".into())
        );
        misplaced = base("task", "claude");
        if let AutomationStep::LaunchCli { effort, .. } = &mut misplaced.steps[0] {
            *effort = Some("invalid".into());
        }
        assert_eq!(
            validate(&misplaced),
            Err("automation-effort-invalid".into())
        );
        if let AutomationStep::LaunchCli { cli_profile_id, effort, .. } = &mut misplaced.steps[0] {
            *cli_profile_id = "agy".into();
            *effort = Some("xhigh".into());
        }
        assert_eq!(
            validate(&misplaced),
            Err("automation-agy-effort-invalid".into())
        );
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
    fn v2_builds_paired_cli_segments_and_allows_shell_steps() {
        let definition = AutomationDefinition {
            schema_version: 2,
            id: "switch".into(),
            revision: 1,
            name: "switch".into(),
            inputs: vec![],
            steps: vec![
                AutomationStep::LaunchCli {
                    id: "claude-launch".into(),
                    cli_profile_id: "claude".into(),
                    model: None,
                    effort: None,
                    startup_wait_ms: None,
                },
                AutomationStep::LlmTask {
                    id: "claude-task".into(),
                    label: "design".into(),
                    prompt_template: "design".into(),
                    wait_timeout_ms: None,
                    completion_grace_ms: None,
                    allow_early_complete: None,
                },
                AutomationStep::ExitCli {
                    id: "claude-exit".into(),
                    command: "/exit".into(),
                    return_mode: Some(AutomationCliReturnMode::Auto),
                    exit_wait_ms: Some(30_000),
                },
                AutomationStep::Wait {
                    id: "between".into(),
                    duration_ms: 1,
                },
                AutomationStep::LaunchCli {
                    id: "agy-launch".into(),
                    cli_profile_id: "agy".into(),
                    model: None,
                    effort: None,
                    startup_wait_ms: None,
                },
                AutomationStep::Confirm {
                    id: "check".into(),
                    message: "continue".into(),
                },
                AutomationStep::LlmTask {
                    id: "agy-task".into(),
                    label: "implement".into(),
                    prompt_template: "implement".into(),
                    wait_timeout_ms: None,
                    completion_grace_ms: None,
                    allow_early_complete: None,
                },
                AutomationStep::ExitCli {
                    id: "agy-exit".into(),
                    command: "/exit".into(),
                    return_mode: Some(AutomationCliReturnMode::Manual),
                    exit_wait_ms: None,
                },
            ],
            repeat: Some(AutomationRepeat { max_cycles: 2 }),
            workspace: None,
            input_values: None,
        };
        assert_eq!(
            compile(&definition),
            Ok(DefinitionPlan::V2 {
                segments: vec![
                    CliSegment {
                        launch_step_index: 0,
                        launch_step_id: "claude-launch".into(),
                        exit_step_index: 2,
                        exit_step_id: "claude-exit".into()
                    },
                    CliSegment {
                        launch_step_index: 4,
                        launch_step_id: "agy-launch".into(),
                        exit_step_index: 7,
                        exit_step_id: "agy-exit".into()
                    },
                ]
            })
        );
    }

    #[test]
    fn v2_rejects_unpaired_or_outside_cli_steps() {
        let base = |steps| AutomationDefinition {
            schema_version: 2,
            id: "switch".into(),
            revision: 1,
            name: "switch".into(),
            inputs: vec![],
            steps,
            repeat: None,
            workspace: None,
            input_values: None,
        };
        assert_eq!(
            validate(&base(vec![AutomationStep::LlmTask {
                id: "task".into(),
                label: "x".into(),
                prompt_template: "x".into(),
                wait_timeout_ms: None,
                completion_grace_ms: None,
                allow_early_complete: None
            }])),
            Err("automation-v2-invalid:0:task:llm-task-not-cli".into())
        );
        assert_eq!(
            validate(&base(vec![AutomationStep::ExitCli {
                id: "exit".into(),
                command: "/exit".into(),
                return_mode: None,
                exit_wait_ms: None
            }])),
            Err("automation-v2-invalid:0:exit:exit-not-cli".into())
        );
        assert_eq!(
            validate(&base(vec![AutomationStep::LaunchCli {
                id: "launch".into(),
                cli_profile_id: "claude".into(),
                model: None,
                effort: None,
                startup_wait_ms: None
            }])),
            Err("automation-v2-invalid:0:launch:cli-not-closed".into())
        );
        assert_eq!(
            validate(&base(vec![AutomationStep::Wait {
                id: "wait".into(),
                duration_ms: 1
            }])),
            Err("automation-v2-cli-segment-required".into())
        );
    }

    #[test]
    fn v1_rejects_v2_exit_options() {
        let definition = AutomationDefinition {
            schema_version: 1,
            id: "one".into(),
            revision: 1,
            name: "one".into(),
            inputs: vec![],
            steps: vec![AutomationStep::ExitCli {
                id: "exit".into(),
                command: "/exit".into(),
                return_mode: Some(AutomationCliReturnMode::Auto),
                exit_wait_ms: None,
            }],
            repeat: None,
            workspace: None,
            input_values: None,
        };
        assert_eq!(
            validate(&definition),
            Err("automation-v1-exit-options-unsupported".into())
        );
    }
}
