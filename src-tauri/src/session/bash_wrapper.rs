// src-tauri/src/session/bash_wrapper.rs
//
// Git Bash time-tracking fix (Windows shell-selection feature; see the
// PowerShell `CLAUDE_WRAPPER_PS`/macOS-Linux `zsh_wrapper` siblings in
// `session::shells`/`session::zsh_wrapper` for the same fix on those
// shells): plain `claude` never gets `--settings <per-session file>`, so
// hooks never fire. bash has a simple, direct fix zsh lacks: `bash -i
// --rcfile <path>` loads exactly the rc file we point it at instead of
// `~/.bashrc`, so there's no ZDOTDIR-style indirection needed -- the shim
// just chain-loads the user's real `~/.bashrc` (if any) and defines a
// `claude` wrapper after it, so ours wins over any same-named alias/function
// the user's own rc defines.

use std::io;
use std::path::{Path, PathBuf};

const BASHRC: &str = r#"[ -f "$HOME/.bashrc" ] && . "$HOME/.bashrc"
claude() {
  if [ -n "$AGENT_OFFICE_SETTINGS" ] && [[ " $* " != *" --settings "* ]]; then
    command claude --settings "$AGENT_OFFICE_SETTINGS" "$@"
  else
    command claude "$@"
  fi
}
"#;

/// Writes the shim `bashrc` file into `base` (created if missing),
/// overwriting any existing copy -- contents are static, so blind overwrite
/// is fine and keeps callers simple. Returns the shim file's path.
pub fn write_shim(base: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(base)?;
    let path = base.join("bashrc");
    std::fs::write(&path, BASHRC)?;
    Ok(path)
}

/// Writes the shim into the process-wide scratch location
/// (`<tmp>/agent-office/bashrc`) and returns its path. Safe to call once per
/// session -- every call rewrites the same static file.
pub fn ensure_bashrc() -> io::Result<PathBuf> {
    let base = std::env::temp_dir().join("agent-office");
    write_shim(&base)
}

/// True when `shell`'s file name is exactly `bash` or `bash.exe` (e.g.
/// `C:\Program Files\Git\bin\bash.exe`, or a bare `bash` resolved via PATH).
/// Mirrors `zsh_wrapper::is_zsh` for API symmetry; unlike that sibling,
/// `session::shells::resolve_with`'s git-bash branch already knows it's
/// resolving bash from the `Some("git-bash")` match arm itself, so nothing
/// in non-test code currently needs to re-derive it from a bare path --
/// kept `pub` (and exercised by the tests below) as part of this module's
/// public surface for callers that only have a shell path in hand.
#[allow(dead_code)]
pub fn is_bash(shell: &str) -> bool {
    matches!(
        Path::new(shell).file_name().and_then(|n| n.to_str()),
        Some("bash") | Some("bash.exe")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> PathBuf {
        std::env::temp_dir().join(format!("agent-office-bash-wrapper-test-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn write_shim_creates_exactly_one_shim_file() {
        let base = scratch_dir();
        write_shim(&base).expect("write_shim succeeds");

        assert!(base.join("bashrc").is_file());

        let names: Vec<String> = std::fs::read_dir(&base)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["bashrc"]);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn bashrc_defines_a_claude_wrapper_function_with_settings_injection_and_skip_guard() {
        let base = scratch_dir();
        let path = write_shim(&base).unwrap();
        let bashrc = std::fs::read_to_string(&path).unwrap();

        assert!(bashrc.contains("claude() {"), "must define claude() function: {bashrc}");
        assert!(
            bashrc.contains("command claude --settings \"$AGENT_OFFICE_SETTINGS\""),
            "must inject --settings from AGENT_OFFICE_SETTINGS: {bashrc}"
        );
        assert!(
            bashrc.contains(r#"" $* " != *" --settings "*"#),
            "must guard against double --settings injection: {bashrc}"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn bashrc_sources_the_users_real_home_bashrc() {
        let base = scratch_dir();
        let path = write_shim(&base).unwrap();
        let bashrc = std::fs::read_to_string(&path).unwrap();

        assert!(
            bashrc.contains(r#". "$HOME/.bashrc""#),
            "must source $HOME/.bashrc: {bashrc}"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn write_shim_is_idempotent_and_overwrites_cleanly() {
        let base = scratch_dir();
        write_shim(&base).unwrap();
        write_shim(&base).unwrap(); // must not error on a 2nd call

        assert!(base.join("bashrc").is_file());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn is_bash_matches_bare_and_full_paths() {
        assert!(is_bash("bash"));
        assert!(is_bash("bash.exe"));
        assert!(is_bash(r"C:\Program Files\Git\bin\bash.exe"));
        assert!(is_bash("/bin/bash"));
    }

    #[test]
    fn is_bash_rejects_other_shells() {
        assert!(!is_bash("zsh"));
        assert!(!is_bash("powershell.exe"));
        assert!(!is_bash(""));
    }
}
