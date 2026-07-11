// src-tauri/src/vscode.rs
//
// 에이전트 작업 폴더를 Visual Studio Code로 여는 `open_in_vscode` 커맨드의
// 구현부. OS별 실행 후보를 순서대로 시도해 첫 성공에서 멈춘다:
// - macOS: PATH의 `code` CLI -> `open -a "Visual Studio Code"` 폴백.
//   번들 GUI 앱은 PATH가 최소셋(/usr/bin 등)이라 `code`가 안 잡히는 경우가
//   흔한데, `open -a`는 LaunchServices 경유라 PATH와 무관하게 동작한다.
// - Windows: PATH의 `code.cmd` -> 사용자 설치(%LOCALAPPDATA%) ->
//   시스템 설치(Program Files) 경로 순. .cmd는 Rust std가 cmd.exe 경유로
//   안전하게(인젝션 이스케이프 포함) 실행해 준다. 콘솔 창이 튀지 않도록
//   CREATE_NO_WINDOW를 건다(claude_cli.rs와 동일 관례).
// - 그 외(Linux 등): PATH의 `code`.
//
// `code`/`open` 런처는 앱을 띄운 뒤 곧바로 종료하므로 `.status()`로 짧게
// 기다려 성공(exit 0) 여부를 판정한다 -- spawn만 하면 `open -a`가 "앱
// 없음"으로 실패해도 감지할 수 없다.

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
pub fn launch_candidates(
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

/// `dir`을 VS Code로 연다. 디렉터리가 아니거나 전 후보 실패 시 사용자에게
/// 그대로 보여줄 수 있는 한국어 에러 문자열을 돌려준다.
pub fn open_dir_in_vscode(dir: &str) -> Result<(), String> {
    if !Path::new(dir).is_dir() {
        return Err(format!("작업 폴더를 찾을 수 없습니다: {dir}"));
    }

    let local_app_data = std::env::var("LOCALAPPDATA").ok();
    for c in launch_candidates(std::env::consts::OS, dir, local_app_data.as_deref()) {
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
    Err("Visual Studio Code를 찾을 수 없습니다. VS Code 설치 여부를 확인해 주세요.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_candidates_are_code_then_open_fallback() {
        let v = launch_candidates("macos", "/Users/me/proj", None);
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
        let v = launch_candidates(
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
        let v = launch_candidates("windows", "C:\\work\\proj", None);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].program, "code.cmd");
        assert_eq!(
            v[1].program,
            "C:\\Program Files\\Microsoft VS Code\\bin\\code.cmd"
        );
    }

    #[test]
    fn other_os_uses_path_code_only() {
        let v = launch_candidates("linux", "/home/me/proj", None);
        assert_eq!(v, vec![LaunchCandidate::new("code", &["/home/me/proj"])]);
    }

    #[test]
    fn nonexistent_dir_is_rejected_before_any_launch() {
        let err = open_dir_in_vscode("/definitely/not/a/dir").unwrap_err();
        assert!(err.contains("/definitely/not/a/dir"), "err={err}");
    }

    /// 실제 VS Code 창을 띄우는 수동 스모크(호스트 OS 경로 검증용).
    /// `cargo test vscode -- --ignored`로 실행한다.
    #[test]
    #[ignore = "실제 VS Code를 실행함 -- 수동 확인 전용"]
    fn manual_smoke_opens_this_crate_dir() {
        open_dir_in_vscode(env!("CARGO_MANIFEST_DIR")).unwrap();
    }
}
