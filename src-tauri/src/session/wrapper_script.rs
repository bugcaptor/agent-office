use crate::observer::{CommandWrapperSpec, WrapperArg};

fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// POSIX 셸 인용(작은따옴표 안은 확장이 전혀 없다 → 임의 텍스트가 데이터로만
/// 남는다). pub(crate): attach_script.rs가 export 값 인용에 같은 규칙을 쓴다.
pub(crate) fn sh_quote(value: &str) -> String {
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

/// pub(crate): attach_script.rs가 export할 env 키를 같은 기준으로 거른다.
pub(crate) fn safe_env_identifier(value: &str) -> bool {
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
    if let Some(name) = &wrapper.export_cwd_env {
        assert!(
            safe_env_identifier(name),
            "invalid wrapper environment name"
        );
    }
    for (target, source) in &wrapper.set_env_from_env {
        assert!(
            safe_env_identifier(target) && safe_env_identifier(source),
            "invalid wrapper environment name"
        );
    }
    if let Some(name) = &wrapper.skip_if_env_set {
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

        // kilo: 사용자가 이미 자기 KILO_CONFIG를 쓰고 있으면 관찰을 포기하고
        // 원본 명령을 그대로 실행한다(사용자 설정을 덮어쓰지 않는다).
        if let Some(env_name) = &wrapper.skip_if_env_set {
            writeln!(script, "    if ($env:{env_name}) {{").unwrap();
            writeln!(
                script,
                "        Write-Warning {}",
                ps_quote(&format!(
                    "agent-office: {env_name} already set; running {} unobserved",
                    wrapper.command,
                )),
            )
            .unwrap();
            writeln!(script, "        & $cmd.Source @args").unwrap();
            writeln!(script, "        return").unwrap();
            writeln!(script, "    }}").unwrap();
        }

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
        // 이슈 #40: prefix env가 가리키는 설정 파일이 없으면 prefix/set_env
        // 대입을 붙이지 않고 원본 명령을 실행한다(관찰 없이 실행 보장). kilo는
        // prefix가 비어 있고 set_env_from_env만 있으므로, 이 가드도 그 경우를
        // 함께 다룬다.
        if !prefix.is_empty() || !wrapper.set_env_from_env.is_empty() {
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
        // set→try/finally 복원: 그 한 호출에만 대상 env를 소스 env 값으로
        // 세팅하고, 호출이 끝나면(성공/실패 불문) 원래 상태로 되돌린다.
        //
        // 리뷰 지적: `skip_if_env_set` 가드(위 블록)가 대상 env가 이미 값을
        // 가진 경우 여기 도달하기 전에 return하므로, 이 지점에서
        // `$_ao_prev_{target}`는 사실상 항상 `$null`이다(가드 없는 래퍼를
        // 새로 만들면 값이 있을 수도 있으니 일반적으로 다룬다). 또한
        // Windows PowerShell 5.1은 `$env:X = $null`을 대입해도 env가
        // 지워지지 않고 **빈 문자열로 남는다**(PowerShell 7+의 `$env:X =
        // $null`은 삭제와 같지만 5.1은 다르다) — 그래서 원래 값이 없었을
        // 때는 대입이 아니라 `Remove-Item Env:X`로 명시적으로 지운다.
        for (target, source) in &wrapper.set_env_from_env {
            writeln!(script, "    $_ao_prev_{target} = $env:{target}").unwrap();
            writeln!(script, "    $env:{target} = $env:{source}").unwrap();
        }
        if !wrapper.set_env_from_env.is_empty() {
            writeln!(script, "    try {{").unwrap();
        }
        if prefix.is_empty() {
            writeln!(script, "    & $cmd.Source @args").unwrap();
        } else {
            writeln!(script, "    & $cmd.Source {prefix} @args").unwrap();
        }
        if !wrapper.set_env_from_env.is_empty() {
            writeln!(script, "    }} finally {{").unwrap();
            for (target, _) in &wrapper.set_env_from_env {
                writeln!(
                    script,
                    "        if ($null -eq $_ao_prev_{target}) {{ Remove-Item Env:{target} -ErrorAction Ignore }} else {{ $env:{target} = $_ao_prev_{target} }}",
                )
                .unwrap();
            }
            writeln!(script, "    }}").unwrap();
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

        // 리뷰 지적: `export`로 셸 함수 전체 스코프에 심는 대신, 호출 한 번에만
        // 붙는 앞자리 대입(`NAME="$PWD" command ...`)으로 준다 — export는 함수가
        // 끝난 뒤에도 값이 남아 같은 셸에서 도는 다른 명령에 새는데, 앞자리
        // 대입은 그 명령 하나의 환경에만 적용되고 사라진다(POSIX 셸 표준 동작).
        let cwd_prefix = wrapper
            .export_cwd_env
            .as_ref()
            .map(|name| format!("{name}=\"$PWD\" "))
            .unwrap_or_default();

        // kilo: 사용자가 이미 자기 KILO_CONFIG를 쓰고 있으면 관찰을 포기하고
        // 원본 명령을 그대로 실행한다(사용자 설정을 덮어쓰지 않는다).
        if let Some(env_name) = &wrapper.skip_if_env_set {
            writeln!(script, "  if [ -n \"${{{env_name}:-}}\" ]; then").unwrap();
            writeln!(
                script,
                "    echo 'agent-office: {env_name} already set; running {} unobserved' >&2",
                wrapper.command,
            )
            .unwrap();
            writeln!(script, "    {cwd_prefix}command {} \"$@\"; return", wrapper.command).unwrap();
            writeln!(script, "  fi").unwrap();
        }

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
                "      {patterns}) {cwd_prefix}command {} \"$@\"; return ;;",
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
        // 앞자리 env 대입(kilo: `KILO_CONFIG="${AGENT_OFFICE_KILO_CONFIG}"`).
        // set_env_from_env가 붙는 명령 하나에만 적용되고 함수가 끝나면 사라진다
        // (cwd_prefix와 같은 POSIX 앞자리 대입 규약).
        let set_env_prefix: String = wrapper
            .set_env_from_env
            .iter()
            .map(|(target, source)| format!("{target}=\"${{{source}}}\" "))
            .collect();
        // 이슈 #40: prefix env가 가리키는 설정 파일이 없으면 prefix/set_env
        // 대입을 붙이지 않고 원본 명령을 실행한다(관찰 없이 실행 보장). kilo는
        // prefix가 비어 있고 set_env_from_env만 있으므로, 이 가드도 그 경우를
        // 함께 다룬다 — 둘 다 비어 있으면 의미가 없어 건너뛴다.
        if !prefix.is_empty() || !wrapper.set_env_from_env.is_empty() {
            if let Some(env_name) = &wrapper.skip_prefix_if_env_file_missing {
                writeln!(script, "  if [ ! -f \"${{{env_name}}}\" ]; then").unwrap();
                writeln!(
                    script,
                    "    echo 'agent-office: observer settings missing; running {} unobserved' >&2",
                    wrapper.command,
                )
                .unwrap();
                writeln!(script, "    {cwd_prefix}command {} \"$@\"; return", wrapper.command)
                    .unwrap();
                writeln!(script, "  fi").unwrap();
            }
        }
        if prefix.is_empty() {
            writeln!(
                script,
                "  {cwd_prefix}{set_env_prefix}command {} \"$@\"",
                wrapper.command,
            )
            .unwrap();
        } else {
            writeln!(
                script,
                "  {cwd_prefix}{set_env_prefix}command {} {prefix} \"$@\"",
                wrapper.command,
            )
            .unwrap();
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

    /// agy(§4 스파이크 실측: workspacePaths가 빈 배열로 올 수 있음) 대비 --
    /// export_cwd_env가 있으면 원본 명령을 부르기 전에 호출 시점 `$PWD`를
    /// export한다.
    #[test]
    fn posix_renderer_sets_pwd_only_for_the_single_invocation_when_export_cwd_env_is_set() {
        let script = render_posix(&[CommandWrapperSpec {
            command: "agy".into(),
            prefix_args: vec![],
            skip_if_present: vec![],
            export_cwd_env: Some("AGENT_OFFICE_AGY_CWD".into()),
            ..Default::default()
        }]);
        assert!(script.contains("agy() {"), "{script}");
        // export가 아니라 그 호출 한 번에만 붙는 앞자리 대입이어야 한다 --
        // export는 함수가 끝난 뒤에도 값이 남아 같은 셸의 다른 명령에 샌다.
        assert!(
            script.contains("AGENT_OFFICE_AGY_CWD=\"$PWD\" command agy \"$@\""),
            "{script}",
        );
        assert!(!script.contains("export"), "must not use export: {script}");

        // export_cwd_env가 없는 기본 래퍼(wrappers())에는 이 대입이 없어야 한다(무회귀).
        assert!(!render_posix(&wrappers()).contains("AGENT_OFFICE_AGY_CWD"));
    }

    #[test]
    #[should_panic(expected = "invalid wrapper environment name")]
    fn posix_renderer_rejects_export_cwd_env_identifier_injection() {
        render_posix(&[CommandWrapperSpec {
            command: "agy".into(),
            prefix_args: vec![],
            skip_if_present: vec![],
            export_cwd_env: Some("SAFE}; touch /tmp/nope; #".into()),
            ..Default::default()
        }]);
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
            ..Default::default()
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

    // ── kilo: set_env_from_env / skip_if_env_set (그 호출 한 번에만 env 대입) ──

    fn kilo_wrapper() -> CommandWrapperSpec {
        CommandWrapperSpec {
            command: "kilo".into(),
            prefix_args: vec![],
            set_env_from_env: vec![("KILO_CONFIG".into(), "AGENT_OFFICE_KILO_CONFIG".into())],
            skip_if_env_set: Some("KILO_CONFIG".into()),
            skip_prefix_if_env_file_missing: Some("AGENT_OFFICE_KILO_CONFIG".into()),
            ..Default::default()
        }
    }

    #[test]
    fn posix_renderer_assigns_set_env_from_env_only_for_the_single_invocation() {
        let script = render_posix(&[kilo_wrapper()]);
        assert!(script.contains("kilo() {"), "{script}");
        assert!(
            script.contains(r#"KILO_CONFIG="${AGENT_OFFICE_KILO_CONFIG}" command kilo "$@""#),
            "{script}",
        );
        // 강등 경로(파일 부재)에는 env 대입을 붙이지 않는다.
        assert!(
            script.contains("command kilo \"$@\"; return"),
            "{script}",
        );
        assert!(!script.contains("export"), "must not use export: {script}");
        // set_env_from_env가 없는 기본 래퍼에는 이 대입이 없어야 한다(무회귀).
        assert!(!render_posix(&wrappers()).contains("KILO_CONFIG"));
    }

    #[test]
    fn powershell_renderer_sets_env_then_restores_it_in_a_finally_block() {
        let script = render_powershell(&[kilo_wrapper()]);
        assert!(script.contains("function global:kilo"), "{script}");
        assert!(
            script.contains("$_ao_prev_KILO_CONFIG = $env:KILO_CONFIG"),
            "{script}",
        );
        assert!(
            script.contains("$env:KILO_CONFIG = $env:AGENT_OFFICE_KILO_CONFIG"),
            "{script}",
        );
        assert!(script.contains("try {"), "{script}");
        assert!(script.contains("} finally {"), "{script}");
        // Windows PowerShell 5.1은 `$env:X = $null`을 대입해도 지워지지 않고
        // 빈 문자열로 남는다 — 원래 값이 없었으면(=$null) Remove-Item으로
        // 명시적으로 지우고, 있었으면 그 값으로 복원한다.
        assert!(
            script.contains(
                "if ($null -eq $_ao_prev_KILO_CONFIG) { Remove-Item Env:KILO_CONFIG -ErrorAction Ignore } else { $env:KILO_CONFIG = $_ao_prev_KILO_CONFIG }"
            ),
            "{script}",
        );
    }

    #[test]
    fn posix_renderer_skips_observation_when_the_env_is_already_set() {
        let script = render_posix(&[kilo_wrapper()]);
        assert!(
            script.contains("if [ -n \"${KILO_CONFIG:-}\" ]; then"),
            "{script}",
        );
        assert!(script.contains("command kilo \"$@\"; return"), "{script}");
        // 옵션 없는 기본 래퍼에는 이 분기가 없어야 한다(무회귀).
        assert!(!render_posix(&wrappers()).contains("already set"));
    }

    #[test]
    fn powershell_renderer_skips_observation_when_the_env_is_already_set() {
        let script = render_powershell(&[kilo_wrapper()]);
        assert!(script.contains("if ($env:KILO_CONFIG) {"), "{script}");
        assert!(!render_powershell(&wrappers()).contains("already set"));
    }

    #[test]
    fn posix_renderer_degrades_without_assigning_env_when_config_file_is_missing() {
        // prefix가 비어 있어도(kilo는 prefix_args가 없다) set_env_from_env만으로
        // 파일-부재 강등 분기가 렌더돼야 한다(가드 완화 회귀 방지).
        let script = render_posix(&[kilo_wrapper()]);
        assert!(
            script.contains("if [ ! -f \"${AGENT_OFFICE_KILO_CONFIG}\" ]; then"),
            "{script}",
        );
    }
}
