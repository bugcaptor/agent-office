// src-tauri/src/session/bash_wrapper.rs
//
// Git Bash observer wrapper support. Bash has a simple, direct mechanism
// zsh lacks: `bash --rcfile <path> -i` loads exactly the rc file we point it
// at instead of
// `~/.bashrc`, so there's no ZDOTDIR-style indirection needed -- the shim
// chain-loads the user's real `~/.bashrc` and appends adapter-provided
// command wrappers.

use std::io;
use std::path::{Path, PathBuf};

use crate::observer::CommandWrapperSpec;
use crate::session::wrapper_script::render_posix;

/// Writes a bash rcfile whose command functions are rendered from the
/// adapter-provided wrapper specs.
pub fn write_observer_shim(base: &Path, wrappers: &[CommandWrapperSpec]) -> io::Result<PathBuf> {
    std::fs::create_dir_all(base)?;
    let path = base.join("bashrc");
    let mut bashrc = String::from("[ -f \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\"\n");
    bashrc.push_str(&render_posix(wrappers));
    std::fs::write(&path, bashrc)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::{CommandWrapperSpec, WrapperArg};

    fn scratch_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-bash-wrapper-test-{}",
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
                ..Default::default()
            },
            CommandWrapperSpec {
                command: "codex".into(),
                prefix_args: vec![
                    WrapperArg::Literal("-c".into()),
                    WrapperArg::Env("AGENT_OFFICE_CODEX_HOOK_STOP".into()),
                ],
                skip_if_present: vec![],
                ..Default::default()
            },
            CommandWrapperSpec {
                command: "pi".into(),
                prefix_args: vec![
                    WrapperArg::Literal("-e".into()),
                    WrapperArg::Env("AGENT_OFFICE_PI_EXT".into()),
                ],
                skip_if_present: vec![],
                ..Default::default()
            },
        ]
    }

    #[test]
    fn write_observer_shim_renders_all_command_functions() {
        let base = scratch_dir();
        let path = write_observer_shim(&base, &observer_wrappers()).unwrap();
        let bashrc = std::fs::read_to_string(path).unwrap();

        assert!(bashrc.contains(r#". "$HOME/.bashrc""#), "{bashrc}");
        assert!(bashrc.contains("claude() {"), "{bashrc}");
        assert!(bashrc.contains("codex() {"), "{bashrc}");
        assert!(bashrc.contains("pi() {"), "{bashrc}");
        assert!(
            bashrc.contains("command codex '-c' \"${AGENT_OFFICE_CODEX_HOOK_STOP}\" \"$@\""),
            "{bashrc}",
        );
        assert!(
            bashrc.contains("command pi '-e' \"${AGENT_OFFICE_PI_EXT}\" \"$@\""),
            "{bashrc}",
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
