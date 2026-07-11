// src-tauri/src/session/zsh_wrapper.rs
//
// macOS/Linux zsh observer wrapper support. zsh has no equivalent "run this
// snippet before the prompt" spawn flag, but it does have `ZDOTDIR` — the
// directory zsh reads its startup files (`.zshenv`/`.zprofile`/`.zshrc`/
// `.zlogin`) from, in place of `$HOME`.
//
// The observer shell resolver writes a shim and sets `ZDOTDIR` only for zsh
// sessions. The shim files chain-load the user's *real*
// dotfiles from `$HOME` (so nothing about the user's normal shell setup is
// lost, including a user who relocates `ZDOTDIR` themselves in their own
// `.zshenv`), and `.zshrc` appends adapter-provided wrappers after sourcing
// the user's real `.zshrc`. No `.zlogin` shim is written —
// by the time `.zshrc` finishes restoring/unsetting `ZDOTDIR`, zsh moves on
// to reading `.zlogin` from the *restored* `ZDOTDIR` (i.e. the user's real
// one) on its own.

use std::io;
use std::path::{Path, PathBuf};

use crate::observer::CommandWrapperSpec;
use crate::session::wrapper_script::render_posix;

const ZSHENV: &str = r#"# agent-office ZDOTDIR shim — 사용자의 실제 dotfile을 그대로 위임하고,
# .zshrc 단계에서 adapter-provided command wrappers를 정의한다.
AGENT_OFFICE_SHIM_DIR="$ZDOTDIR"
AGENT_OFFICE_REAL_ZDOTDIR="$HOME"
[[ -f "$HOME/.zshenv" ]] && source "$HOME/.zshenv"
if [[ -n "$ZDOTDIR" && "$ZDOTDIR" != "$AGENT_OFFICE_SHIM_DIR" ]]; then
  AGENT_OFFICE_REAL_ZDOTDIR="$ZDOTDIR"
  ZDOTDIR="$AGENT_OFFICE_SHIM_DIR"
fi
"#;

const ZPROFILE: &str = r#"[[ -f "$AGENT_OFFICE_REAL_ZDOTDIR/.zprofile" ]] && source "$AGENT_OFFICE_REAL_ZDOTDIR/.zprofile"
"#;

const OBSERVER_ZSHRC_PREFIX: &str = r#"[[ -f "$AGENT_OFFICE_REAL_ZDOTDIR/.zshrc" ]] && source "$AGENT_OFFICE_REAL_ZDOTDIR/.zshrc"

"#;

const OBSERVER_ZSHRC_SUFFIX: &str = r#"
# 중첩 셸이 사용자의 원래 ZDOTDIR 규칙을 따르도록 복원.
if [[ "$AGENT_OFFICE_REAL_ZDOTDIR" == "$HOME" ]]; then
  unset ZDOTDIR
else
  export ZDOTDIR="$AGENT_OFFICE_REAL_ZDOTDIR"
fi
unset AGENT_OFFICE_SHIM_DIR AGENT_OFFICE_REAL_ZDOTDIR
"#;

/// Writes the existing ZDOTDIR delegation files with command functions
/// rendered from the adapter-provided wrapper specs.
pub fn write_observer_shim(base: &Path, wrappers: &[CommandWrapperSpec]) -> io::Result<PathBuf> {
    std::fs::create_dir_all(base)?;
    std::fs::write(base.join(".zshenv"), ZSHENV)?;
    std::fs::write(base.join(".zprofile"), ZPROFILE)?;

    let mut zshrc = String::from(OBSERVER_ZSHRC_PREFIX);
    zshrc.push_str(&render_posix(wrappers));
    zshrc.push_str(OBSERVER_ZSHRC_SUFFIX);
    std::fs::write(base.join(".zshrc"), zshrc)?;
    Ok(base.to_path_buf())
}

/// True when `shell`'s file name is exactly `zsh` (e.g. `/bin/zsh`,
/// `/opt/homebrew/bin/zsh`, or a bare `zsh` resolved via PATH).
pub fn is_zsh(shell: &str) -> bool {
    Path::new(shell).file_name().and_then(|n| n.to_str()) == Some("zsh")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::{CommandWrapperSpec, WrapperArg};

    fn scratch_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-zsh-wrapper-test-{}",
            uuid::Uuid::new_v4()
        ))
    }

    fn observer_wrappers() -> Vec<CommandWrapperSpec> {
        vec![
            CommandWrapperSpec {
                command: "claude".into(),
                prefix_args: vec![
                    WrapperArg::Literal("--settings".into()),
                    WrapperArg::Env("AGENT_OFFICE_SETTINGS".into()),
                ],
                skip_if_present: vec!["--settings".into()],
            },
            CommandWrapperSpec {
                command: "codex".into(),
                prefix_args: vec![
                    WrapperArg::Literal("-c".into()),
                    WrapperArg::Env("AGENT_OFFICE_CODEX_HOOK_STOP".into()),
                ],
                skip_if_present: vec![],
            },
        ]
    }

    #[test]
    fn write_observer_shim_preserves_delegation_and_renders_both_provider_functions() {
        let base = scratch_dir();
        write_observer_shim(&base, &observer_wrappers()).unwrap();
        let zshrc = std::fs::read_to_string(base.join(".zshrc")).unwrap();

        assert!(
            zshrc.contains(r#"source "$AGENT_OFFICE_REAL_ZDOTDIR/.zshrc""#),
            "{zshrc}",
        );
        assert!(zshrc.contains("claude() {"), "{zshrc}");
        assert!(zshrc.contains("codex() {"), "{zshrc}");
        assert!(
            zshrc.contains("command codex '-c' \"${AGENT_OFFICE_CODEX_HOOK_STOP}\" \"$@\""),
            "{zshrc}",
        );
        assert!(
            zshrc.contains("unset AGENT_OFFICE_SHIM_DIR AGENT_OFFICE_REAL_ZDOTDIR"),
            "{zshrc}",
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn is_zsh_matches_absolute_and_bare_zsh_paths() {
        assert!(is_zsh("/bin/zsh"));
        assert!(is_zsh("zsh"));
        assert!(is_zsh("/opt/homebrew/bin/zsh"));
    }

    #[test]
    fn is_zsh_rejects_other_shells() {
        assert!(!is_zsh("/opt/homebrew/bin/bash"));
        assert!(!is_zsh("/usr/local/bin/fish"));
        assert!(!is_zsh("/bin/sh"));
        assert!(!is_zsh(""));
    }

    // ---- integration: real zsh actually resolves the wrapper & restores ZDOTDIR ----

    #[cfg(unix)]
    #[test]
    fn real_zsh_resolves_claude_as_a_function_and_restores_zdotdir() {
        if !Path::new("/bin/zsh").exists() {
            eprintln!("skipping: /bin/zsh not present on this host");
            return;
        }

        let shim_dir = scratch_dir();
        write_observer_shim(&shim_dir, &observer_wrappers())
            .expect("write_observer_shim succeeds");
        let empty_home = std::env::temp_dir().join(format!(
            "agent-office-zsh-wrapper-home-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&empty_home).expect("create scratch HOME");

        let output = std::process::Command::new("/bin/zsh")
            .arg("-l")
            .arg("-i")
            .arg("-c")
            .arg(r#"whence -w claude; whence -w pi; print -r -- "ZDOTDIR=${ZDOTDIR:-unset}""#)
            .env_clear()
            .env("HOME", &empty_home)
            .env("TERM", "dumb")
            .env("ZDOTDIR", &shim_dir)
            .env("AGENT_OFFICE_SETTINGS", "/tmp/x.json")
            .output()
            .expect("spawn /bin/zsh");

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("claude: function"),
            "expected `claude: function` in zsh output, got: {stdout:?} (stderr: {:?})",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            stdout.contains("pi: function"),
            "expected `pi: function` in zsh output, got: {stdout:?} (stderr: {:?})",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            stdout.contains("ZDOTDIR=unset"),
            "expected ZDOTDIR to be unset after .zshrc restore, got: {stdout:?}"
        );

        let _ = std::fs::remove_dir_all(&shim_dir);
        let _ = std::fs::remove_dir_all(&empty_home);
    }
}
