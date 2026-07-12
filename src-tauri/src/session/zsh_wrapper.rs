// src-tauri/src/session/zsh_wrapper.rs
//
// macOS/Linux zsh time-tracking fix (see the Windows `AGENT_WRAPPER_PS`
// sibling in session::manager for the same fix on that platform):
// plain `claude` never gets `--settings <per-session file>`, so hooks never
// fire. Windows solves this by injecting a `claude` PowerShell function into
// the spawned shell's startup command; zsh has no equivalent "run this
// snippet before the prompt" spawn flag, but it does have `ZDOTDIR` — the
// directory zsh reads its startup files (`.zshenv`/`.zprofile`/`.zshrc`/
// `.zlogin`) from, in place of `$HOME`.
//
// `ensure_zdotdir()` writes a small shim directory once and returns its
// path; `SessionManager::create` sets `ZDOTDIR` to that path only for zsh
// sessions (see `is_zsh`). The shim files chain-load the user's *real*
// dotfiles from `$HOME` (so nothing about the user's normal shell setup is
// lost, including a user who relocates `ZDOTDIR` themselves in their own
// `.zshenv`), and `.zshrc` defines a `claude` wrapper function *after*
// sourcing the user's real `.zshrc` so it wins over any same-named
// alias/function the user may have defined. No `.zlogin` shim is written —
// by the time `.zshrc` finishes restoring/unsetting `ZDOTDIR`, zsh moves on
// to reading `.zlogin` from the *restored* `ZDOTDIR` (i.e. the user's real
// one) on its own.

use std::io;
use std::path::{Path, PathBuf};

const ZSHENV: &str = r#"# agent-office ZDOTDIR shim — 사용자의 실제 dotfile을 그대로 위임하고,
# .zshrc 단계에서 `claude` 래퍼(--settings 자동 주입)를 정의한다.
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

const ZSHRC: &str = r#"[[ -f "$AGENT_OFFICE_REAL_ZDOTDIR/.zshrc" ]] && source "$AGENT_OFFICE_REAL_ZDOTDIR/.zshrc"

# claude 래퍼: AGENT_OFFICE_SETTINGS가 있으면 --settings를 투명하게 주입.
# 사용자 rc가 정의한 동명 함수/알리아스보다 나중에 정의되므로 우선한다.
# (Windows의 AGENT_WRAPPER_PS와 동일한 의미론 — --settings를 이미 넘기면 주입 안 함)
claude() {
  if [[ -n "$AGENT_OFFICE_SETTINGS" && " $* " != *" --settings "* ]]; then
    command claude --settings "$AGENT_OFFICE_SETTINGS" "$@"
  else
    command claude "$@"
  fi
}

# pi(pi.dev) 래퍼: AGENT_OFFICE_PI_EXT가 있으면 확장을 -e로 투명 주입.
# claude 래퍼와 동일 의미론 — pi의 -e는 반복 가능/additive라 이중주입 가드 불필요.
pi() {
  if [[ -n "$AGENT_OFFICE_PI_EXT" ]]; then
    command pi -e "$AGENT_OFFICE_PI_EXT" "$@"
  else
    command pi "$@"
  fi
}

# 중첩 셸이 사용자의 원래 ZDOTDIR 규칙을 따르도록 복원.
if [[ "$AGENT_OFFICE_REAL_ZDOTDIR" == "$HOME" ]]; then
  unset ZDOTDIR
else
  export ZDOTDIR="$AGENT_OFFICE_REAL_ZDOTDIR"
fi
unset AGENT_OFFICE_SHIM_DIR AGENT_OFFICE_REAL_ZDOTDIR
"#;

/// Writes the shim `.zshenv`/`.zprofile`/`.zshrc` triplet into `base`
/// (created if missing), overwriting any existing copies — contents are
/// static, so blind overwrite is fine and keeps callers simple. Deliberately
/// does NOT write `.zlogin`; see the module header comment for why.
pub fn write_shim(base: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(base)?;
    std::fs::write(base.join(".zshenv"), ZSHENV)?;
    std::fs::write(base.join(".zprofile"), ZPROFILE)?;
    std::fs::write(base.join(".zshrc"), ZSHRC)?;
    Ok(base.to_path_buf())
}

/// Writes the shim into the process-wide scratch location
/// (`<tmp>/agent-office/zdotdir`) and returns its path. Safe to call once
/// per session — every call rewrites the same static files.
pub fn ensure_zdotdir() -> io::Result<PathBuf> {
    let base = std::env::temp_dir().join("agent-office").join("zdotdir");
    write_shim(&base)
}

/// True when `shell`'s file name is exactly `zsh` (e.g. `/bin/zsh`,
/// `/opt/homebrew/bin/zsh`, or a bare `zsh` resolved via PATH).
pub fn is_zsh(shell: &str) -> bool {
    Path::new(shell).file_name().and_then(|n| n.to_str()) == Some("zsh")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> PathBuf {
        std::env::temp_dir().join(format!("agent-office-zsh-wrapper-test-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn write_shim_creates_exactly_the_three_expected_files() {
        let base = scratch_dir();
        write_shim(&base).expect("write_shim succeeds");

        assert!(base.join(".zshenv").is_file());
        assert!(base.join(".zprofile").is_file());
        assert!(base.join(".zshrc").is_file());
        assert!(!base.join(".zlogin").exists(), "must NOT write a .zlogin shim");

        let mut names: Vec<String> = std::fs::read_dir(&base)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec![".zprofile", ".zshenv", ".zshrc"]);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn zshrc_defines_a_claude_wrapper_function_with_settings_injection_and_skip_guard() {
        let base = scratch_dir();
        write_shim(&base).unwrap();
        let zshrc = std::fs::read_to_string(base.join(".zshrc")).unwrap();

        assert!(zshrc.contains("claude() {"), "must define claude() function: {zshrc}");
        assert!(
            zshrc.contains("command claude --settings \"$AGENT_OFFICE_SETTINGS\""),
            "must inject --settings from AGENT_OFFICE_SETTINGS: {zshrc}"
        );
        assert!(
            zshrc.contains(r#"" $* " != *" --settings "*"#),
            "must guard against double --settings injection: {zshrc}"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn zshrc_defines_a_pi_wrapper_function_that_injects_the_extension() {
        let base = scratch_dir();
        write_shim(&base).unwrap();
        let zshrc = std::fs::read_to_string(base.join(".zshrc")).unwrap();

        assert!(zshrc.contains("pi() {"), "must define pi() function: {zshrc}");
        assert!(
            zshrc.contains("command pi -e \"$AGENT_OFFICE_PI_EXT\""),
            "must inject -e from AGENT_OFFICE_PI_EXT: {zshrc}"
        );
        assert!(
            zshrc.contains("command pi \"$@\""),
            "must fall back to plain pi when the env is unset: {zshrc}"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn zshenv_sources_the_users_real_home_zshenv() {
        let base = scratch_dir();
        write_shim(&base).unwrap();
        let zshenv = std::fs::read_to_string(base.join(".zshenv")).unwrap();

        assert!(
            zshenv.contains(r#"source "$HOME/.zshenv""#),
            "must source $HOME/.zshenv: {zshenv}"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn write_shim_is_idempotent_and_overwrites_cleanly() {
        let base = scratch_dir();
        write_shim(&base).unwrap();
        write_shim(&base).unwrap(); // must not error on a 2nd call

        assert!(base.join(".zshrc").is_file());
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
        write_shim(&shim_dir).expect("write_shim succeeds");
        let empty_home = std::env::temp_dir().join(format!("agent-office-zsh-wrapper-home-{}", uuid::Uuid::new_v4()));
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
