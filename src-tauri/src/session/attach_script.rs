// src-tauri/src/session/attach_script.rs
//
// 외부 터미널 attach가 돌려주는 셸 스크립트 렌더러. `ctl attach`가 받아
// `eval "$(agent-office ctl attach <agentId>)"` 형태로 **이미 떠 있는 셸**에
// 그대로 먹인다.
//
// PTY 세션(`shells::resolve_observed`)과 달리 셸을 우리가 띄우지 않으므로
// ZDOTDIR/rcfile shim이 필요 없다 -- env는 `export`로, 명령 래퍼는
// `wrapper_script::render_posix`가 내는 함수 정의(unalias 포함)로 충분하다.
// 그래서 zsh/bash 공용이고 fish는 지원하지 않는다(문법이 다르다).
//
// 값 인용은 PTY 경로와 같은 `sh_quote`를 쓴다 -- persona처럼 작은따옴표·개행이
// 섞인 임의 텍스트가 들어와도 데이터로만 남는다.

use super::manager::PreparedPlan;
use super::wrapper_script::{render_posix, safe_env_identifier, sh_quote};

/// PreparedPlan을 eval용 POSIX 스크립트로 렌더한다.
///
/// `plan.env`에 있는 항목만 export하므로, observer가 꺼져 있으면
/// `AGENT_OFFICE_HOOK_URL`/`AGENT_OFFICE_SETTINGS`가 아예 없고 persona만 남는다
/// (그 경우 상단에 경고 코멘트를 붙인다).
pub fn render_attach_script(agent_name: &str, session_id: &str, plan: &PreparedPlan) -> String {
    use std::fmt::Write as _;

    let mut script = String::new();
    writeln!(
        script,
        "# agent-office attach — session {}, agent {}",
        comment_safe(session_id),
        comment_safe(agent_name),
    )
    .unwrap();
    // 훅 URL이 없으면 forwarder가 POST할 곳이 없다 = 관측 불가. 스크립트는
    // 여전히 유효하지만(성격 주입은 된다) 사용자가 이유를 알 수 있게 알린다.
    if !plan
        .env
        .iter()
        .any(|(key, _)| key == "AGENT_OFFICE_HOOK_URL")
    {
        script.push_str("# 경고: 관측이 비활성입니다 — 알림은 오지 않고 성격만 적용됩니다.\n");
    }

    for (key, value) in &plan.env {
        // eval되는 텍스트라 키는 식별자여야 한다. 우리 코드가 넣는 값뿐이지만,
        // 렌더러가 임의 문자열을 코드로 승격시키지 않도록 게이트를 둔다.
        if !safe_env_identifier(key) {
            continue;
        }
        writeln!(script, "export {key}={}", sh_quote(value)).unwrap();
    }

    // render_posix는 래퍼마다 `unalias …`를 먼저 내므로 여기서 또 만들지 않는다.
    script.push_str(&render_posix(&plan.wrappers));
    script
}

/// tmux 모드(`ctl attach … --tmux <target>`)의 응답 스크립트. 이때 앱은 자기
/// PTY로 tmux 클라이언트를 여는 것이므로 **요청한 셸에 심을 것이 없다** --
/// 출력 계약(성공 시 stdout=eval 대상)을 깨지 않도록 코멘트 두 줄만 낸다.
///
/// 훅·성격은 tmux **pane 안에서** 다시 `eval "$(… ctl attach <id>)"`을 해야
/// 붙는다(그때는 앱 안 PTY 세션이 살아 있어 BindExisting -- 같은 sid에 합류).
pub fn render_tmux_notice(agent_id: &str, target: &str) -> String {
    format!(
        "# agent-office attach — tmux 세션 '{}'에 연결됨(앱 터미널 탭이 미러입니다).\n\
         # 각 pane에서 훅·성격을 붙이려면 그 pane에서: eval \"$(agent-office ctl attach {})\"\n",
        comment_safe(target),
        comment_safe(agent_id),
    )
}

/// `#` 코멘트 한 줄에 안전하게 넣을 수 있게 제어문자(특히 개행)를 없앤다 --
/// 개행이 남으면 코멘트가 끝나고 그 뒤가 코드로 해석된다.
fn comment_safe(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::{CommandWrapperSpec, WrapperArg};

    fn plan(env: Vec<(&str, &str)>, wrappers: Vec<CommandWrapperSpec>) -> PreparedPlan {
        PreparedPlan {
            env: env
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            wrappers,
            cleanup_paths: vec![],
            settings_path: None,
        }
    }

    fn claude_wrapper() -> CommandWrapperSpec {
        CommandWrapperSpec {
            command: "claude".into(),
            prefix_args: vec![
                WrapperArg::Literal("--settings".into()),
                WrapperArg::Env("AGENT_OFFICE_SETTINGS".into()),
                WrapperArg::Literal("--append-system-prompt".into()),
                WrapperArg::Env("AGENT_OFFICE_PERSONA".into()),
            ],
            skip_if_present: vec!["--settings".into()],
            ..Default::default()
        }
    }

    #[test]
    fn renders_exports_before_wrapper_functions() {
        let script = render_attach_script(
            "Ada",
            "sid-1",
            &plan(
                vec![
                    ("AGENT_OFFICE_SESSION", "sid-1"),
                    ("AGENT_OFFICE_HOOK_URL", "http://127.0.0.1:5000/hook"),
                    ("AGENT_OFFICE_SETTINGS", "/tmp/sid-1.settings.json"),
                ],
                vec![claude_wrapper()],
            ),
        );
        assert!(script.starts_with("# agent-office attach — session sid-1, agent Ada\n"));
        assert!(
            script.contains("export AGENT_OFFICE_SESSION='sid-1'"),
            "{script}"
        );
        assert!(
            script.contains("export AGENT_OFFICE_HOOK_URL='http://127.0.0.1:5000/hook'"),
            "{script}",
        );
        assert!(script.contains("claude() {"), "{script}");
        assert!(
            script.contains("unalias 'claude' 2>/dev/null || true"),
            "{script}",
        );
        // 래퍼 함수는 export가 다 끝난 뒤에 정의돼야 한다(함수 본문이 env를 읽는다).
        let last_export = script.rfind("export ").unwrap();
        assert!(last_export < script.find("claude() {").unwrap(), "{script}");
        // unalias는 render_posix가 내는 한 벌뿐(중복 생성 금지).
        assert_eq!(script.matches("unalias 'claude'").count(), 1, "{script}");
        assert!(!script.contains("# 경고"), "{script}");
    }

    #[test]
    fn quotes_persona_containing_single_quotes_as_data() {
        let script = render_attach_script(
            "Ada",
            "sid-2",
            &plan(
                vec![
                    ("AGENT_OFFICE_SESSION", "sid-2"),
                    ("AGENT_OFFICE_HOOK_URL", "http://127.0.0.1:5000/hook"),
                    (
                        "AGENT_OFFICE_PERSONA",
                        "don't $(touch nope) `id` 'quoted'\n둘째 줄",
                    ),
                ],
                vec![],
            ),
        );
        assert!(
            script.contains(
                "export AGENT_OFFICE_PERSONA='don'\"'\"'t $(touch nope) `id` '\"'\"'quoted'\"'\"'\n둘째 줄'"
            ),
            "{script}",
        );
    }

    #[test]
    fn warns_and_skips_hook_env_when_observer_is_off() {
        // observer OFF: prepare_session_plan이 HOOK_URL/SETTINGS를 아예 안 넣고
        // persona 래퍼만 남는다.
        let script = render_attach_script(
            "Ada",
            "sid-3",
            &plan(
                vec![
                    ("AGENT_OFFICE_SESSION", "sid-3"),
                    ("AGENT_OFFICE_PERSONA", "짧게 답한다"),
                ],
                vec![CommandWrapperSpec {
                    command: "claude".into(),
                    prefix_args: vec![
                        WrapperArg::Literal("--append-system-prompt".into()),
                        WrapperArg::Env("AGENT_OFFICE_PERSONA".into()),
                    ],
                    skip_if_present: vec![],
                    ..Default::default()
                }],
            ),
        );
        assert!(script.contains("# 경고: 관측이 비활성입니다"), "{script}");
        assert!(!script.contains("AGENT_OFFICE_HOOK_URL"), "{script}");
        assert!(!script.contains("AGENT_OFFICE_SETTINGS"), "{script}");
        assert!(
            script.contains(
                "command claude '--append-system-prompt' \"${AGENT_OFFICE_PERSONA}\" \"$@\""
            ),
            "{script}",
        );
    }

    #[test]
    fn sanitizes_newlines_in_the_header_comment() {
        let script = render_attach_script("Ada\necho pwned", "sid-4", &plan(vec![], vec![]));
        let first_line = script.lines().next().unwrap();
        assert!(first_line.contains("Ada echo pwned"), "{script}");
        assert!(!script.contains("\necho pwned"), "{script}");
    }

    #[test]
    fn tmux_notice_is_comments_only_and_sanitized() {
        let notice = render_tmux_notice("a1", "work");
        assert!(notice.lines().all(|line| line.starts_with('#')), "{notice}");
        assert!(notice.contains("tmux 세션 'work'"), "{notice}");
        assert!(
            notice.contains("eval \"$(agent-office ctl attach a1)\""),
            "{notice}"
        );
        // 개행이 섞여 들어와도 코멘트를 벗어나지 못한다.
        let evil = render_tmux_notice("a1", "work'\necho pwned");
        assert_eq!(evil.lines().count(), 2, "{evil}");
        assert!(evil.lines().all(|line| line.starts_with('#')), "{evil}");
    }

    #[test]
    fn drops_env_entries_whose_key_is_not_an_identifier() {
        let script = render_attach_script(
            "Ada",
            "sid-5",
            &plan(
                vec![
                    ("AGENT_OFFICE_SESSION", "sid-5"),
                    ("BAD-KEY; touch nope", "x"),
                ],
                vec![],
            ),
        );
        assert!(!script.contains("BAD-KEY"), "{script}");
        assert!(
            script.contains("export AGENT_OFFICE_SESSION='sid-5'"),
            "{script}"
        );
    }
}
