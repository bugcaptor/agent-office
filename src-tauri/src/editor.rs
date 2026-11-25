// 편집기 CLI가 GUI 앱의 PATH에 없을 수 있다. macOS에서는 LaunchServices로,
// Windows에서는 기본 설치 경로의 런처로 다시 시도한다. 명령과 인자는 따로 전달한다.
use crate::persistence::settings_store::FolderEditor;
use std::path::Path;
use std::process::{Command, Stdio};

/// 실행 후보 하나: 프로그램 + 인자 목록.
#[derive(Debug, PartialEq)]
pub struct LaunchCandidate {
    pub program: String,
    pub args: Vec<String>,
}

impl LaunchCandidate {
    fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// OS별 VS Code 실행 후보를 시도 순서대로 돌려준다. 순수 함수 --
/// `os`는 `std::env::consts::OS` 값, `local_app_data`는 Windows의
/// `%LOCALAPPDATA%`(다른 OS에서는 None).
pub fn vscode_candidates(
    os: &str,
    dir: &str,
    local_app_data: Option<&str>,
) -> Vec<LaunchCandidate> {
    match os {
        "macos" => vec![
            LaunchCandidate::new("code", &[dir]),
            LaunchCandidate::new("open", &["-a", "Visual Studio Code", dir]),
        ],
        "windows" => {
            let mut v = vec![LaunchCandidate::new("code.cmd", &[dir])];
            if let Some(lad) = local_app_data {
                v.push(LaunchCandidate::new(
                    &format!("{lad}\\Programs\\Microsoft VS Code\\bin\\code.cmd"),
                    &[dir],
                ));
            }
            v.push(LaunchCandidate::new(
                "C:\\Program Files\\Microsoft VS Code\\bin\\code.cmd",
                &[dir],
            ));
            v
        }
        _ => vec![LaunchCandidate::new("code", &[dir])],
    }
}

fn sublime_candidates(os: &str, dir: &str) -> Vec<LaunchCandidate> {
    match os {
        "macos" => vec![
            LaunchCandidate::new("subl", &["--", dir]),
            LaunchCandidate::new("open", &["-a", "Sublime Text", dir]),
        ],
        "windows" => {
            let mut candidates = vec![LaunchCandidate::new("subl.exe", &["--", dir])];
            for base in ["ProgramFiles", "ProgramFiles(x86)"]
                .iter()
                .filter_map(std::env::var_os)
            {
                let launcher = Path::new(&base).join(r"Sublime Text\subl.exe");
                candidates.push(LaunchCandidate::new(
                    &launcher.to_string_lossy(),
                    &["--", dir],
                ));
            }
            candidates
        }
        _ => vec![LaunchCandidate::new("subl", &["--", dir])],
    }
}

pub fn open_in_editor(dir: &str, editor: FolderEditor) -> Result<(), String> {
    if !Path::new(dir).exists() {
        return Err(format!("경로를 찾을 수 없습니다: {dir}"));
    }

    let os = std::env::consts::OS;
    let candidates = match editor {
        FolderEditor::Vscode => {
            vscode_candidates(os, dir, std::env::var("LOCALAPPDATA").ok().as_deref())
        }
        FolderEditor::Sublime => sublime_candidates(os, dir),
    };
    for c in candidates {
        let mut cmd = Command::new(&c.program);
        cmd.args(&c.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        // 런처(code/open)는 수백 ms 안에 끝난다 -- 성공 판정에 exit code가
        // 필요하므로 spawn이 아니라 status로 기다린다.
        if matches!(cmd.status(), Ok(s) if s.success()) {
            return Ok(());
        }
    }
    let name = match editor {
        FolderEditor::Vscode => "Visual Studio Code",
        FolderEditor::Sublime => "Sublime Text",
    };
    Err(format!(
        "편집기를 실행하지 못했습니다. {name} 설치 여부를 확인해 주세요."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_candidates_are_code_then_open_fallback() {
        let v = vscode_candidates("macos", "/Users/me/proj", None);
        assert_eq!(
            v,
            vec![
                LaunchCandidate::new("code", &["/Users/me/proj"]),
                LaunchCandidate::new("open", &["-a", "Visual Studio Code", "/Users/me/proj"]),
            ]
        );
    }

    #[test]
    fn windows_candidates_include_user_and_system_installs() {
        let v = vscode_candidates(
            "windows",
            "C:\\work\\proj",
            Some("C:\\Users\\me\\AppData\\Local"),
        );
        assert_eq!(
            v,
            vec![
                LaunchCandidate::new("code.cmd", &["C:\\work\\proj"]),
                LaunchCandidate::new(
                    "C:\\Users\\me\\AppData\\Local\\Programs\\Microsoft VS Code\\bin\\code.cmd",
                    &["C:\\work\\proj"],
                ),
                LaunchCandidate::new(
                    "C:\\Program Files\\Microsoft VS Code\\bin\\code.cmd",
                    &["C:\\work\\proj"],
                ),
            ]
        );
    }

    #[test]
    fn windows_without_local_app_data_skips_user_install() {
        let v = vscode_candidates("windows", "C:\\work\\proj", None);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].program, "code.cmd");
        assert_eq!(
            v[1].program,
            "C:\\Program Files\\Microsoft VS Code\\bin\\code.cmd"
        );
    }

    #[test]
    fn other_os_uses_path_code_only() {
        let v = vscode_candidates("linux", "/home/me/proj", None);
        assert_eq!(v, vec![LaunchCandidate::new("code", &["/home/me/proj"])]);
    }

    #[test]
    fn sublime_keeps_a_spaced_folder_in_one_argument_for_both_mac_launchers() {
        let dir = "/Users/me/My Project";
        assert_eq!(
            sublime_candidates("macos", dir),
            vec![
                LaunchCandidate::new("subl", &["--", dir]),
                LaunchCandidate::new("open", &["-a", "Sublime Text", dir]),
            ]
        );
    }

    #[test]
    fn nonexistent_dir_is_rejected_before_any_launch() {
        let err = open_in_editor("/definitely/not/a/dir", FolderEditor::Vscode).unwrap_err();
        assert!(err.contains("/definitely/not/a/dir"), "err={err}");
    }
}
