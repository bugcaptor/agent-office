use crate::observer::{CommandWrapperSpec, WrapperArg};

fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn ps_arg(value: &WrapperArg) -> String {
    match value {
        WrapperArg::Literal(value) => ps_quote(value),
        WrapperArg::Env(name) => format!("$env:{name}"),
    }
}

fn sh_arg(value: &WrapperArg) -> String {
    match value {
        WrapperArg::Literal(value) => sh_quote(value),
        WrapperArg::Env(name) => format!("\"${{{name}}}\""),
    }
}

fn safe_command_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn safe_env_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn validate_wrapper(wrapper: &CommandWrapperSpec) {
    assert!(
        safe_command_identifier(&wrapper.command),
        "invalid wrapper command"
    );
    for arg in &wrapper.prefix_args {
        if let WrapperArg::Env(name) = arg {
            assert!(
                safe_env_identifier(name),
                "invalid wrapper environment name"
            );
        }
    }
    if let Some(name) = &wrapper.skip_prefix_if_env_file_missing {
        assert!(
            safe_env_identifier(name),
            "invalid wrapper environment name"
        );
    }
}

pub fn render_powershell(wrappers: &[CommandWrapperSpec]) -> String {
    use std::fmt::Write as _;

    let mut script = String::new();
    for wrapper in wrappers {
        validate_wrapper(wrapper);

        writeln!(
            script,
            "Remove-Item Alias:{} -Force -ErrorAction Ignore",
            wrapper.command
        )
        .unwrap();
        writeln!(script, "function global:{} {{", wrapper.command).unwrap();
        writeln!(
            script,
            "    $cmd = Get-Command {} -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1",
            ps_quote(&wrapper.command),
        )
        .unwrap();
        writeln!(
            script,
            "    if (-not $cmd) {{ Write-Error {}; return }}",
            ps_quote(&format!("{} executable not found on PATH", wrapper.command)),
        )
        .unwrap();

        if !wrapper.skip_if_present.is_empty() {
            let condition = wrapper
                .skip_if_present
                .iter()
                .map(|arg| format!("$args -contains {}", ps_quote(arg)))
                .collect::<Vec<_>>()
                .join(" -or ");
            writeln!(script, "    if ({condition}) {{").unwrap();
            writeln!(script, "        & $cmd.Source @args").unwrap();
            writeln!(script, "        return").unwrap();
            writeln!(script, "    }}").unwrap();
        }

        let prefix = wrapper
            .prefix_args
            .iter()
            .map(ps_arg)
            .collect::<Vec<_>>()
            .join(" ");
        // 이슈 #40: prefix env가 가리키는 설정 파일이 없으면 prefix를 붙이지 않고
        // 원본 명령을 실행한다(관찰 없이 실행 보장). prefix가 비면 무의미해 건너뛴다.
        if !prefix.is_empty() {
            if let Some(env_name) = &wrapper.skip_prefix_if_env_file_missing {
                writeln!(
                    script,
                    "    if (-not $env:{env_name} -or -not (Test-Path -LiteralPath $env:{env_name})) {{",
                )
                .unwrap();
                writeln!(
                    script,
                    "        Write-Warning {}",
                    ps_quote(&format!(
                        "agent-office: observer settings missing; running {} unobserved",
                        wrapper.command,
                    )),
                )
                .unwrap();
                writeln!(script, "        & $cmd.Source @args").unwrap();
                writeln!(script, "        return").unwrap();
                writeln!(script, "    }}").unwrap();
            }
        }
        if prefix.is_empty() {
            writeln!(script, "    & $cmd.Source @args").unwrap();
        } else {
            writeln!(script, "    & $cmd.Source {prefix} @args").unwrap();
        }
        writeln!(script, "}}").unwrap();
    }
    script
}

pub fn render_posix(wrappers: &[CommandWrapperSpec]) -> String {
    use std::fmt::Write as _;

    let mut script = String::new();
    for wrapper in wrappers {
        validate_wrapper(wrapper);

        writeln!(script, "unalias '{}' 2>/dev/null || true", wrapper.command).unwrap();
        writeln!(script, "{}() {{", wrapper.command).unwrap();
        if !wrapper.skip_if_present.is_empty() {
            let patterns = wrapper
                .skip_if_present
                .iter()
                .map(|value| sh_quote(value))
                .collect::<Vec<_>>()
                .join("|");
            writeln!(script, "  for _ao_arg in \"$@\"; do").unwrap();
            writeln!(script, "    case \"$_ao_arg\" in").unwrap();
            writeln!(
                script,
                "      {patterns}) command {} \"$@\"; return ;;",
                wrapper.command,
            )
            .unwrap();
            writeln!(script, "    esac").unwrap();
            writeln!(script, "  done").unwrap();
        }

        let prefix = wrapper
            .prefix_args
            .iter()
            .map(sh_arg)
            .collect::<Vec<_>>()
            .join(" ");
        // 이슈 #40: prefix env가 가리키는 설정 파일이 없으면 prefix를 붙이지 않고
        // 원본 명령을 실행한다(관찰 없이 실행 보장). prefix가 비면 무의미해 건너뛴다.
        if !prefix.is_empty() {
            if let Some(env_name) = &wrapper.skip_prefix_if_env_file_missing {
                writeln!(script, "  if [ ! -f \"${{{env_name}}}\" ]; then").unwrap();
                writeln!(
                    script,
                    "    echo 'agent-office: observer settings missing; running {} unobserved' >&2",
                    wrapper.command,
                )
                .unwrap();
                writeln!(script, "    command {} \"$@\"; return", wrapper.command).unwrap();
                writeln!(script, "  fi").unwrap();
            }
        }
        if prefix.is_empty() {
            writeln!(script, "  command {} \"$@\"", wrapper.command).unwrap();
        } else {
            writeln!(script, "  command {} {prefix} \"$@\"", wrapper.command).unwrap();
        }
        writeln!(script, "}}").unwrap();
    }
    script
}

#[cfg(test)]
mod tests {
    use super::{render_posix, render_powershell};
    use crate::observer::{CommandWrapperSpec, WrapperArg};

    fn wrappers() -> Vec<CommandWrapperSpec> {
        vec![
            CommandWrapperSpec {
                command: "claude".into(),
                prefix_args: vec![
                    WrapperArg::Literal("--settings".into()),
                    WrapperArg::Env("AGENT_OFFICE_SETTINGS".into()),
                ],
                skip_if_present: vec!["--settings".into()],
                ..Default::default()
            },
            CommandWrapperSpec {
                command: "codex".into(),
                prefix_args: vec![
                    WrapperArg::Literal("--enable".into()),
                    WrapperArg::Literal("hooks".into()),
                    WrapperArg::Literal("-c".into()),
                    WrapperArg::Env("AGENT_OFFICE_CODEX_HOOK_STOP".into()),
                ],
                skip_if_present: vec![],
                ..Default::default()
            },
        ]
    }

    #[test]
    fn powershell_renderer_defines_equal_external_command_wrappers() {
        let script = render_powershell(&wrappers());
        assert!(script.contains("function global:claude"), "{script}");
        assert!(script.contains("function global:codex"), "{script}");
        assert!(
            script.contains("Remove-Item Alias:codex -Force -ErrorAction Ignore"),
            "{script}"
        );
        assert!(
            script.contains("-CommandType Application,ExternalScript"),
            "{script}"
        );
        assert!(script.contains("$args -contains '--settings'"), "{script}");
        assert!(script.contains("$env:AGENT_OFFICE_SETTINGS"), "{script}");
        assert!(
            script.contains("$env:AGENT_OFFICE_CODEX_HOOK_STOP"),
            "{script}"
        );
        assert!(script.contains("@args"), "{script}");
    }

    #[test]
    fn powershell_renderer_preserves_exact_prefix_and_user_argument_order() {
        let script = render_powershell(&wrappers());
        assert!(
            script.contains(
                "& $cmd.Source '--enable' 'hooks' '-c' $env:AGENT_OFFICE_CODEX_HOOK_STOP @args"
            ),
            "{script}",
        );
        assert!(
            script.contains("& $cmd.Source '--settings' $env:AGENT_OFFICE_SETTINGS @args"),
            "{script}",
        );
    }

    #[test]
    fn posix_renderer_preserves_user_argument_suffix() {
        let script = render_posix(&wrappers());
        assert!(script.contains("claude() {"), "{script}");
        assert!(script.contains("codex() {"), "{script}");
        assert!(script.contains("command claude"), "{script}");
        assert!(script.contains("command codex"), "{script}");
        assert!(
            script.contains("unalias 'codex' 2>/dev/null || true"),
            "{script}"
        );
        assert!(script.contains("\"$@\""), "{script}");
    }

    #[test]
    fn posix_renderer_guards_each_user_argument_without_flattening() {
        let script = render_posix(&wrappers());
        assert!(script.contains("for _ao_arg in \"$@\"; do"), "{script}");
        assert!(script.contains("case \"$_ao_arg\" in"), "{script}");
        assert!(
            script.contains("'--settings') command claude \"$@\"; return ;;"),
            "{script}",
        );
        assert!(
            !script.contains("$*"),
            "must not flatten arguments: {script}"
        );
        assert!(
            script.contains(
                "command codex '--enable' 'hooks' '-c' \"${AGENT_OFFICE_CODEX_HOOK_STOP}\" \"$@\""
            ),
            "{script}",
        );
    }

    #[test]
    fn renderers_quote_adapter_literals_as_data() {
        let wrappers = vec![CommandWrapperSpec {
            command: "safe-tool".into(),
            prefix_args: vec![WrapperArg::Literal("a'b; $(touch nope)".into())],
            skip_if_present: vec!["--flag'; Remove-Item nope".into()],
            ..Default::default()
        }];

        let powershell = render_powershell(&wrappers);
        assert!(powershell.contains("'a''b; $(touch nope)'"), "{powershell}",);
        assert!(
            powershell.contains("$args -contains '--flag''; Remove-Item nope'"),
            "{powershell}",
        );

        let posix = render_posix(&wrappers);
        assert!(posix.contains("'a'\"'\"'b; $(touch nope)'"), "{posix}");
        assert!(
            posix.contains("'--flag'\"'\"'; Remove-Item nope')"),
            "{posix}",
        );
    }

    #[test]
    #[should_panic(expected = "invalid wrapper command")]
    fn powershell_renderer_rejects_command_identifier_injection() {
        render_powershell(&[CommandWrapperSpec {
            command: "claude; Remove-Item C:\\".into(),
            prefix_args: vec![],
            skip_if_present: vec![],
            ..Default::default()
        }]);
    }

    #[test]
    #[should_panic(expected = "invalid wrapper environment name")]
    fn posix_renderer_rejects_environment_identifier_injection() {
        render_posix(&[CommandWrapperSpec {
            command: "codex".into(),
            prefix_args: vec![WrapperArg::Env("SAFE}; touch /tmp/nope; #".into())],
            skip_if_present: vec![],
            ..Default::default()
        }]);
    }

    // 이슈 #40: skip_prefix_if_env_file_missing 가드가 렌더된 래퍼에 파일-부재
    // 강등 분기를 넣는지(그리고 옵션이 None이면 안 넣는지) 검증한다.
    fn guarded_claude() -> Vec<CommandWrapperSpec> {
        vec![CommandWrapperSpec {
            command: "claude".into(),
            prefix_args: vec![
                WrapperArg::Literal("--settings".into()),
                WrapperArg::Env("AGENT_OFFICE_SETTINGS".into()),
            ],
            skip_if_present: vec!["--settings".into()],
            skip_prefix_if_env_file_missing: Some("AGENT_OFFICE_SETTINGS".into()),
        }]
    }

    #[test]
    fn posix_renderer_degrades_to_unobserved_when_settings_file_missing() {
        let script = render_posix(&guarded_claude());
        assert!(
            script.contains("if [ ! -f \"${AGENT_OFFICE_SETTINGS}\" ]; then"),
            "{script}",
        );
        assert!(
            script.contains("command claude \"$@\"; return"),
            "{script}",
        );
        // 가드 없는 기본 래퍼(wrappers())에는 이 분기가 없어야 한다(무회귀).
        assert!(
            !render_posix(&wrappers()).contains("if [ ! -f"),
            "guard must not appear without the option",
        );
    }

    #[test]
    fn powershell_renderer_degrades_to_unobserved_when_settings_file_missing() {
        let script = render_powershell(&guarded_claude());
        assert!(
            script.contains(
                "if (-not $env:AGENT_OFFICE_SETTINGS -or -not (Test-Path -LiteralPath $env:AGENT_OFFICE_SETTINGS))"
            ),
            "{script}",
        );
        assert!(script.contains("Write-Warning"), "{script}");
        assert!(
            !render_powershell(&wrappers()).contains("Test-Path -LiteralPath"),
            "guard must not appear without the option",
        );
    }
}
