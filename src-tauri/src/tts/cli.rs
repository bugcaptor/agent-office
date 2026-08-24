// src-tauri/src/tts/cli.rs
//
// 대사 리라이트의 두 번째 경로: `claude -p` 헤드리스 서브프로세스. API 키가
// 없어도 구독(OAuth) 로그인만으로 리라이트가 된다 — 대신 사용자의 구독 사용량을
// 소모한다.
//
// ─── 훅 격리(필수) ────────────────────────────────────────────────────────
// 이 앱은 claude 훅으로 세션을 감시한다. 앱이 띄운 `claude -p`가 그 훅을
// 발화하면 유령 세션/알림이 생긴다. 격리 근거를 세 겹으로 둔다.
//
//  1) **훅은 전역 등록이 아니다.** `observer/claude.rs`는 세션마다
//     `<app-data>/…/<sessionId>.settings.json`을 쓰고, PTY 세션 셸에 심은
//     `claude` **래퍼 함수**가 `--settings $AGENT_OFFICE_SETTINGS`를 앞에
//     붙여 전달한다(`AdapterSessionPlan.wrappers`). `~/.claude/settings.json`은
//     전혀 건드리지 않는다. 우리는 로그인 셸을 거치지 않고 `claude` 바이너리를
//     직접 spawn하므로 그 래퍼 함수 자체가 존재하지 않는다.
//  2) **`--settings '{"hooks":{}}'` 오버라이드**를 명시적으로 전달한다.
//     `claude --help`: `--settings <file-or-json>`은 JSON 문자열도 받는다.
//  3) **포워더 봉인.** 그래도 훅이 어디선가 발화한다면 실행되는 것은
//     `observer/forwarder.rs`이고, 그것은 `AGENT_OFFICE_SESSION` +
//     (`AGENT_OFFICE_HOOK_URL` | `AGENT_OFFICE_APP_DATA`)가 있어야 허브에 닿는다.
//     자식 env에서 이 세 변수를 **제거**하므로 포워더는 즉시 no-op으로 죽는다.
//
// `--bare`(help: "skip hooks, …")는 쓰지 않는다. 같은 문서가 "Anthropic auth는
// 엄격히 ANTHROPIC_API_KEY 또는 apiKeyHelper이고 OAuth/키체인은 절대 읽지
// 않는다"고 못 박는데, 이 경로의 존재 이유가 바로 "API 키 없이 구독으로
// 리라이트"이므로 --bare는 기능을 무력화한다.
//
// 순수 로직(build_command / parse_output)과 프로세스 실행(rewrite_via_cli)을
// 분리해 서브프로세스 없이 커맨드 형태를 단위 테스트한다(summarizer/claude.rs 관례).

use std::time::Duration;

use crate::i18n::Lang;

use super::rewrite::{sanitize_line, system_prompt, RewriteError, SpeakKind};

/// CLI는 API보다 느리다(프로세스 시작 + 인증). API 경로의 6초보다 넉넉히 준다.
pub const TIMEOUT_SECS: u64 = 20;

/// `--settings`로 넘길 훅 무력화 오버라이드. 인라인 JSON(문서화된 형태).
pub const HOOKS_OFF_SETTINGS: &str = r#"{"hooks":{}}"#;

/// 자식 프로세스에서 반드시 제거할 env — 포워더가 허브에 닿는 유일한 경로다
/// (`observer/forwarder.rs`). 하나만 남아도 유령 알림 가능성이 남는다.
pub const STRIPPED_ENV: &[&str] = &[
    "AGENT_OFFICE_SESSION",
    "AGENT_OFFICE_HOOK_URL",
    "AGENT_OFFICE_APP_DATA",
    "AGENT_OFFICE_SETTINGS",
];

/// non-Windows: `claude` 바이너리를 직접 spawn한다(로그인 셸을 거치지 않으므로
/// 관찰자 래퍼 함수가 적용되지 않는다 — 격리 근거 1).
#[cfg(not(windows))]
pub fn build_command(
    kind: SpeakKind,
    lang: Lang,
    model: &str,
    user_content: &str,
) -> std::process::Command {
    let mut command = std::process::Command::new("claude");
    command.args([
        "-p",
        user_content,
        "--model",
        model,
        "--output-format",
        "text",
        "--max-turns",
        "1",
        // 기본 시스템 프롬프트(코딩 에이전트) 대신 대사 작가 프롬프트로 교체.
        // 확인 요청/완료 보고에 따라 어조 지시가 갈린다(API 경로와 동일).
        "--system-prompt",
        system_prompt(kind, lang),
        // 격리 근거 2.
        "--settings",
        HOOKS_OFF_SETTINGS,
    ]);
    strip_observer_env(&mut command);
    command
}

/// Windows: `claude`는 직접 실행 가능한 이미지가 아니라 셸 shim이라 powershell을
/// 거쳐야 한다(summarizer/claude.rs와 같은 이유·같은 스크립트 관례).
/// 인자는 argv 인용 문제를 피해 전부 env로 넘긴다.
#[cfg(windows)]
const WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
$OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$c = Get-Command claude -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
& $c.Source -p $env:AO_TTS_PROMPT --model $env:AO_TTS_MODEL --output-format text --max-turns 1 --system-prompt $env:AO_TTS_SYSTEM --settings $env:AO_TTS_SETTINGS
exit $LASTEXITCODE"#;

#[cfg(windows)]
pub fn build_command(
    kind: SpeakKind,
    lang: Lang,
    model: &str,
    user_content: &str,
) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]);
    command.creation_flags(CREATE_NO_WINDOW);
    command.env("AO_TTS_PROMPT", user_content);
    command.env("AO_TTS_MODEL", model);
    command.env("AO_TTS_SYSTEM", system_prompt(kind, lang));
    command.env("AO_TTS_SETTINGS", HOOKS_OFF_SETTINGS);
    strip_observer_env(&mut command);
    command
}

/// 격리 근거 3 — 포워더가 허브를 찾는 데 쓰는 env를 자식에서 지운다.
fn strip_observer_env(command: &mut std::process::Command) {
    for name in STRIPPED_ENV {
        command.env_remove(name);
    }
}

/// stdout → 대사. CLI는 가끔 안내 줄을 앞에 붙이므로 sanitize로 한 줄로 접고
/// 비면 EmptyOutput. 순수.
pub fn parse_output(stdout: &str) -> Result<String, RewriteError> {
    let line = sanitize_line(stdout);
    if line.is_empty() {
        Err(RewriteError::EmptyOutput)
    } else {
        Ok(line)
    }
}

/// `claude -p`를 실행해 대사를 얻는다. 실패는 전부 `RewriteError` — 호출측
/// (`tts::speak`)이 원문 발화로 강등한다.
pub async fn rewrite_via_cli(
    kind: SpeakKind,
    lang: Lang,
    model: &str,
    agent_name: &str,
    personality: Option<&str>,
    context: Option<&str>,
    message: &str,
) -> Result<String, RewriteError> {
    let user_content =
        super::rewrite::build_user_content(kind, lang, agent_name, personality, context, message);
    let std_command = build_command(kind, lang, model, &user_content);
    let mut command = tokio::process::Command::from(std_command);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let child = command
        .spawn()
        .map_err(|e| RewriteError::Network(format!("claude CLI 실행 실패: {e}")))?;
    let output = match tokio::time::timeout(
        Duration::from_secs(TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(RewriteError::Network(format!("claude CLI 오류: {e}"))),
        // kill_on_drop이 프로세스를 정리한다.
        Err(_) => {
            return Err(RewriteError::Network(format!(
                "claude CLI 타임아웃({TIMEOUT_SECS}초)"
            )))
        }
    };
    if !output.status.success() {
        // stderr는 통제 불가한 외부 출력 — 200자로 캡한다(키가 실릴 여지 없음).
        let err: String = String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(200)
            .collect();
        return Err(RewriteError::Http(format!(
            "claude CLI exit {:?}: {}",
            output.status.code(),
            err.trim()
        )));
    }
    parse_output(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(command: &std::process::Command) -> String {
        let mut parts = vec![command.get_program().to_string_lossy().into_owned()];
        parts.extend(command.get_args().map(|a| a.to_string_lossy().into_owned()));
        parts.extend(command.get_envs().filter_map(|(k, v)| {
            v.map(|v| format!("{}={}", k.to_string_lossy(), v.to_string_lossy()))
        }));
        parts.join(" ")
    }

    #[test]
    fn command_disables_hooks_via_inline_settings_override() {
        let c = build_command(SpeakKind::Question, Lang::Ko, "claude-haiku-4-5", "content");
        let r = rendered(&c);
        assert!(r.contains("--settings"), "{r}");
        assert!(r.contains(r#"{"hooks":{}}"#), "{r}");
    }

    // 훅 격리의 실질적 보증: 자식 env에서 포워더가 허브를 찾는 변수들이
    // 제거돼야 한다. `env_remove`는 `get_envs()`에 (key, None)으로 나타난다.
    #[test]
    fn command_strips_every_observer_env_var() {
        let c = build_command(SpeakKind::Question, Lang::Ko, "claude-haiku-4-5", "content");
        let removed: Vec<String> = c
            .get_envs()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        for name in STRIPPED_ENV {
            assert!(
                removed.iter().any(|r| r == name),
                "{name} 가 제거되지 않았다 — 유령 알림 경로가 열린다: {removed:?}"
            );
        }
    }

    #[test]
    fn command_is_headless_single_turn_text_output() {
        let c = build_command(SpeakKind::Question, Lang::Ko, "claude-sonnet-5", "content");
        let r = rendered(&c);
        assert!(r.contains("-p"), "{r}");
        assert!(r.contains("--output-format"), "{r}");
        assert!(r.contains("text"), "{r}");
        assert!(r.contains("--max-turns"), "{r}");
        assert!(r.contains("--system-prompt"), "{r}");
        assert!(r.contains("claude-sonnet-5"), "{r}");
    }

    // --bare는 OAuth/키체인을 읽지 않아 "API 키 없이 구독으로" 라는 이 경로의
    // 존재 이유를 무력화한다. 실수로 들어오면 잡는다.
    #[test]
    fn command_does_not_use_bare_mode() {
        let c = build_command(SpeakKind::Question, Lang::Ko, "claude-haiku-4-5", "content");
        assert!(
            !c.get_args().any(|a| a == "--bare"),
            "--bare는 구독 인증을 끊는다"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_spawns_the_binary_directly_not_a_login_shell() {
        // 로그인 셸을 거치면 관찰자 `claude` 래퍼 함수가 --settings를 덧붙여
        // 훅이 살아난다(격리 근거 1).
        let c = build_command(SpeakKind::Question, Lang::Ko, "claude-haiku-4-5", "content");
        assert_eq!(c.get_program(), "claude");
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_passes_user_content_as_the_prompt_arg() {
        let c = build_command(SpeakKind::Question, Lang::Ko, "claude-haiku-4-5", "캐릭터 이름: 무지");
        let args: Vec<String> = c.get_args().map(|a| a.to_string_lossy().into()).collect();
        let i = args.iter().position(|a| a == "-p").unwrap();
        assert_eq!(args[i + 1], "캐릭터 이름: 무지");
    }

    #[cfg(windows)]
    #[test]
    fn windows_uses_powershell_and_passes_args_through_env() {
        let c = build_command(SpeakKind::Question, Lang::Ko, "claude-haiku-4-5", "내용");
        assert_eq!(c.get_program(), "powershell.exe");
        let val = |name: &str| {
            c.get_envs()
                .find(|(k, _)| *k == std::ffi::OsStr::new(name))
                .and_then(|(_, v)| v)
                .map(|v| v.to_string_lossy().into_owned())
        };
        assert_eq!(val("AO_TTS_PROMPT").as_deref(), Some("내용"));
        assert_eq!(val("AO_TTS_MODEL").as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(val("AO_TTS_SETTINGS").as_deref(), Some(HOOKS_OFF_SETTINGS));
    }

    #[cfg(windows)]
    #[test]
    fn windows_script_carries_the_hooks_override_and_single_turn() {
        assert!(WINDOWS_SCRIPT.contains("--settings $env:AO_TTS_SETTINGS"));
        assert!(WINDOWS_SCRIPT.contains("--max-turns 1"));
        assert!(!WINDOWS_SCRIPT.contains("--bare"));
    }

    // 완료 보고는 API 경로와 같은 프롬프트를 써야 한다 — 경로에 따라 어조가
    // 달라지면 사용자에게는 그냥 버그로 보인다.
    #[test]
    fn done_kind_swaps_the_system_prompt() {
        let q = build_command(SpeakKind::Question, Lang::Ko, "claude-haiku-4-5", "c");
        let d = build_command(SpeakKind::Done, Lang::Ko, "claude-haiku-4-5", "c");
        assert_ne!(rendered(&q), rendered(&d));
        assert!(rendered(&d).contains("작업을 마치고"));
    }

    #[test]
    fn parse_output_flattens_and_rejects_blank() {
        assert_eq!(
            parse_output("  [excited] 진행해도 될까요?\n").unwrap(),
            "[excited] 진행해도 될까요?"
        );
        assert_eq!(parse_output("   \n\n"), Err(RewriteError::EmptyOutput));
    }

    #[test]
    fn parse_output_strips_wrapping_quotes_like_the_api_path() {
        assert_eq!(parse_output("\"한 줄\"").unwrap(), "한 줄");
    }
}
