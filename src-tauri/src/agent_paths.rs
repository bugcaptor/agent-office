// src-tauri/src/agent_paths.rs
//
// 에이전트 CLI(Claude Code·Codex)가 자기 데이터를 두는 루트 결정 규칙 한 곳.
//
// 두 CLI 모두 홈 아래 기본 위치를 표준 환경변수로 옮길 수 있다:
//
//   Claude Code : `CLAUDE_CONFIG_DIR` → 설정 디렉터리 전체(`projects/` 전사,
//                 `.claude.json`, `.credentials.json` …)
//   Codex       : `CODEX_HOME`        → `sessions/` 롤아웃 등
//
// 그런데 앱 곳곳이 `~/.claude`·`~/.codex`를 각자 조립하고 있었고 오버라이드를
// 존중하는 곳은 usage 뿐이었다. 그래서 설정 디렉터리를 옮겨 쓰는 사용자는
// 세션 로그에서 에이전트 대화(JSONL 전사)가 통째로 빠지고, 그 로그로 만든
// 학습자료에도 셸 출력만 남았다. 규칙을 여기 모아 모든 호출자가 같은 답을
// 보게 한다.
//
// GUI 기동 주의: Finder/launchd로 띄운 번들 앱의 프로세스 env에는 사용자가
// `.zshrc`에 export한 `CLAUDE_CONFIG_DIR`이 **없다**. `session::env_capture`가
// 부팅 시 로그인 셸에서 이 키들을 캡처해 프로세스 env에 심으므로(#58과 같은
// 처방) 여기서 `std::env`를 읽어도 되고, 우리가 스폰하는 `claude -p`(요약기·
// 학습자료)도 사용자가 실제로 쓰는 설정 디렉터리를 보게 된다.

use std::path::{Path, PathBuf};

/// 빈 문자열/공백 env는 미설정으로 취급한다(일부 셸·런처가 unset 대신 빈
/// 문자열을 넘긴다).
fn override_path(value: Option<&str>) -> Option<PathBuf> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// Claude Code 설정 디렉터리: `CLAUDE_CONFIG_DIR`(설정 시) 또는 `~/.claude`.
pub fn claude_config_dir(home: &Path, env: Option<&str>) -> PathBuf {
    override_path(env).unwrap_or_else(|| home.join(".claude"))
}

/// Codex 홈: `CODEX_HOME`(설정 시) 또는 `~/.codex`.
pub fn codex_home(home: &Path, env: Option<&str>) -> PathBuf {
    override_path(env).unwrap_or_else(|| home.join(".codex"))
}

/// 프로세스 env로 결정한 Claude 설정 디렉터리. 오버라이드도 HOME도 없으면
/// None(그 경우 호출자는 해당 경로 기능을 조용히 끈다).
pub fn claude_config_dir_from_env() -> Option<PathBuf> {
    if let Some(dir) = override_path(std::env::var("CLAUDE_CONFIG_DIR").ok().as_deref()) {
        return Some(dir);
    }
    Some(claude_config_dir(&home()?, None))
}

/// 프로세스 env로 결정한 Codex 홈. 오버라이드도 HOME도 없으면 None.
pub fn codex_home_from_env() -> Option<PathBuf> {
    if let Some(dir) = override_path(std::env::var("CODEX_HOME").ok().as_deref()) {
        return Some(dir);
    }
    Some(codex_home(&home()?, None))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_sit_under_home() {
        let home = Path::new("/home/u");
        assert_eq!(claude_config_dir(home, None), PathBuf::from("/home/u/.claude"));
        assert_eq!(codex_home(home, None), PathBuf::from("/home/u/.codex"));
    }

    #[test]
    fn override_wins_over_home() {
        let home = Path::new("/home/u");
        assert_eq!(
            claude_config_dir(home, Some("/data/claude-work")),
            PathBuf::from("/data/claude-work")
        );
        assert_eq!(
            codex_home(home, Some("/data/codex-work")),
            PathBuf::from("/data/codex-work")
        );
    }

    #[test]
    fn blank_override_is_treated_as_unset() {
        let home = Path::new("/home/u");
        for blank in ["", "   "] {
            assert_eq!(
                claude_config_dir(home, Some(blank)),
                PathBuf::from("/home/u/.claude"),
                "{blank:?}"
            );
            assert_eq!(
                codex_home(home, Some(blank)),
                PathBuf::from("/home/u/.codex"),
                "{blank:?}"
            );
        }
    }
}
