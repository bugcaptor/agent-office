// src-tauri/src/file_manager.rs
//
// 작업 폴더를 OS 파일 탐색기(macOS Finder / Windows 탐색기 / Linux 기본 파일
// 관리자)로 여는 `open_in_file_manager` 커맨드의 구현부. vscode.rs·terminal.rs와
// 같은 골격 -- OS별 실행 후보를 순서대로 시도해 첫 성공에서 멈춘다.
//
// OS별 사정:
// - macOS: `open <dir>`. LaunchServices 경유라 PATH와 무관하고, 디렉터리를
//   주면 Finder 창으로 연다. 즉시 끝나는 런처라 exit code로 성공 판정한다.
// - Windows: `explorer.exe <dir>`. **explorer는 성공해도 exit code 1을 준다**
//   (창을 이미 떠 있는 인스턴스에 넘기고 자신은 실패처럼 끝난다). 그래서 여기만
//   `wait_for_exit=false`로 spawn 성공 여부만 본다 -- status로 판정하면 정상
//   동작을 실패로 오인해 폴백 후보까지 헛돌린다.
// - 그 외(Linux 등): `xdg-open <dir>`.
//
// 파일이 아니라 **디렉터리**를 여는 것이 목적이라 "파일을 선택된 상태로 보여
// 주기"(`open -R` / `explorer /select,`)는 쓰지 않는다.

use std::path::Path;
use std::process::{Command, Stdio};

/// 실행 후보 하나: 프로그램 + 인자 + 종료 대기 여부(shell_export.rs와 동일).
#[derive(Debug, PartialEq)]
pub struct LaunchCandidate {
    pub program: String,
    pub args: Vec<String>,
    /// true면 짧게 종료를 기다려 exit 0으로 성공 판정. explorer.exe만 false
    /// (성공해도 exit 1을 주기 때문).
    pub wait_for_exit: bool,
}

impl LaunchCandidate {
    fn new(program: &str, args: &[&str], wait_for_exit: bool) -> Self {
        Self {
            program: program.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            wait_for_exit,
        }
    }
}

/// OS별 파일 탐색기 실행 후보를 시도 순서대로 돌려준다. 순수 함수 --
/// `os`는 `std::env::consts::OS` 값.
pub fn launch_candidates(os: &str, dir: &str) -> Vec<LaunchCandidate> {
    match os {
        "macos" => vec![LaunchCandidate::new("open", &[dir], true)],
        // explorer.exe는 성공해도 exit 1 -- spawn 성공만 본다.
        "windows" => vec![LaunchCandidate::new("explorer.exe", &[dir], false)],
        _ => vec![LaunchCandidate::new("xdg-open", &[dir], true)],
    }
}

/// `dir`을 OS 파일 탐색기로 연다. 경로가 없거나 전 후보 실패 시 사용자에게
/// 그대로 보여줄 수 있는 한국어 에러 문자열.
pub fn open_dir_in_file_manager(dir: &str) -> Result<(), String> {
    if !Path::new(dir).exists() {
        return Err(format!("경로를 찾을 수 없습니다: {dir}"));
    }

    for c in launch_candidates(std::env::consts::OS, dir) {
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
        let ok = if c.wait_for_exit {
            matches!(cmd.status(), Ok(s) if s.success())
        } else {
            cmd.spawn().is_ok()
        };
        if ok {
            return Ok(());
        }
    }
    Err("파일 탐색기를 실행하지 못했습니다.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_uses_open_and_waits() {
        let v = launch_candidates("macos", "/Users/me/proj");
        assert_eq!(v, vec![LaunchCandidate::new("open", &["/Users/me/proj"], true)]);
    }

    /// explorer.exe는 성공해도 exit 1을 주므로 종료 코드로 판정하면 안 된다.
    #[test]
    fn windows_uses_explorer_without_waiting_for_exit() {
        let v = launch_candidates("windows", "C:\\work\\proj");
        assert_eq!(
            v,
            vec![LaunchCandidate::new("explorer.exe", &["C:\\work\\proj"], false)]
        );
        assert!(!v[0].wait_for_exit);
    }

    #[test]
    fn other_os_uses_xdg_open() {
        let v = launch_candidates("linux", "/home/me/proj");
        assert_eq!(v, vec![LaunchCandidate::new("xdg-open", &["/home/me/proj"], true)]);
    }

    #[test]
    fn nonexistent_dir_is_rejected_before_any_launch() {
        let err = open_dir_in_file_manager("/definitely/not/a/dir").unwrap_err();
        assert!(err.contains("/definitely/not/a/dir"), "err={err}");
    }

    /// 실제 탐색기 창을 띄우는 수동 스모크. `cargo test file_manager -- --ignored`.
    #[test]
    #[ignore = "실제 파일 탐색기를 실행함 -- 수동 확인 전용"]
    fn manual_smoke_opens_this_crate_dir() {
        open_dir_in_file_manager(env!("CARGO_MANIFEST_DIR")).unwrap();
    }
}
