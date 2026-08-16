// src-tauri/src/control/tmux.rs
//
// `ctl attach <agentId> --tmux <target>`(M3) 지원 조각. tmux는 **앱이 자기
// PTY로 tmux 클라이언트를 하나 더 여는** 방식으로 붙인다 — 일반 세션을
// `exec tmux attach-session -t '<target>'`으로 띄우면 출력 미러링·입력 주입·
// resize·on_exit·세션 로그·봇 inject가 기존 PTY 파이프라인 그대로 동작한다
// (pipe-pane + send-keys 대안은 병렬 파이프라인 신설이라 기각).
//
// 여기 있는 건 (1) 대상 이름 검증, (2) 시작 명령 렌더, (3) tmux 존재 확인기다.
// 확인기는 클로저 주입(`SessionManager::shell_resolver`와 같은 관례) — control
// 테스트가 실제 tmux 설치 여부에 좌우되면 안 되기 때문이다.

use std::sync::Arc;

use crate::session::wrapper_script::sh_quote;

/// `tmux has-session` 판정.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxStatus {
    /// 그 이름의 tmux 세션이 있다.
    Present,
    /// tmux는 돌았는데 그런 세션이 없다(비영 종료).
    Missing,
    /// tmux 자체를 실행하지 못했다(미설치 또는 PATH 밖). 문자열은 원인.
    Unavailable(String),
}

/// tmux 세션 존재 확인기. 프로덕션은 `system_probe`, 테스트는 가짜를 넣는다.
pub type TmuxProbe = Arc<dyn Fn(&str) -> TmuxStatus + Send + Sync>;

/// 실제 `tmux has-session -t <target>`를 돌리는 확인기.
///
/// stdout/stderr는 버린다 — 세션이 없을 때 tmux가 stderr로 찍는 문구가 GUI
/// 앱 콘솔을 더럽힐 이유가 없고, 우리는 종료 코드만 본다.
pub fn system_probe() -> TmuxProbe {
    Arc::new(|target: &str| {
        let status = std::process::Command::new("tmux")
            .arg("has-session")
            .arg("-t")
            .arg(target)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match status {
            Ok(s) if s.success() => TmuxStatus::Present,
            Ok(_) => TmuxStatus::Missing,
            Err(e) => TmuxStatus::Unavailable(e.to_string()),
        }
    })
}

/// `--tmux` 대상 이름 검증. 이 값은 셸 stdin에 실려 나가므로(아래 `attach_command`)
/// 인용만으로는 막을 수 없는 개행/제어문자를 여기서 원천 차단한다 — 인용된
/// 작은따옴표 안이라도 개행이 들어가면 "여러 줄"이 되어 의도가 흐려진다.
pub fn validate_target(raw: &str) -> Result<String, String> {
    let target = raw.trim();
    if target.is_empty() {
        return Err("--tmux: 대상 tmux 세션 이름이 비어 있습니다".into());
    }
    if target.chars().any(char::is_control) {
        return Err("--tmux: 대상 이름에 개행 등 제어문자를 쓸 수 없습니다".into());
    }
    Ok(target.to_string())
}

/// 새 PTY 세션의 시작 명령. `exec`이라 tmux 클라이언트가 끝나면 셸도 함께
/// 끝나 기존 `on_exit`(Exited 전이)가 그대로 걸린다.
pub fn attach_command(target: &str) -> String {
    format!("exec tmux attach-session -t {}", sh_quote(target))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_target_rejects_empty_and_control_characters() {
        assert!(validate_target("").is_err());
        assert!(validate_target("   ").is_err());
        assert!(validate_target("work\nrm -rf /").is_err());
        assert!(validate_target("work\rfoo").is_err());
        assert!(validate_target("work\tfoo").is_err());
        // 앞뒤 공백은 다듬고 통과시킨다.
        assert_eq!(validate_target("  work  ").unwrap(), "work");
        assert_eq!(validate_target("my session-1").unwrap(), "my session-1");
    }

    #[test]
    fn attach_command_quotes_the_target_as_data() {
        assert_eq!(attach_command("work"), "exec tmux attach-session -t 'work'");
        // 작은따옴표·공백·메타문자가 섞여도 코드로 승격되지 않는다.
        assert_eq!(
            attach_command("it's $(touch nope) `id`"),
            "exec tmux attach-session -t 'it'\"'\"'s $(touch nope) `id`'"
        );
    }
}
