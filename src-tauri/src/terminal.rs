// src-tauri/src/terminal.rs
//
// 에이전트 작업 폴더를 OS 기본 터미널 앱으로 여는 `open_in_terminal`
// 커맨드의 구현부(vscode.rs와 동일 골격). OS별 실행 후보를 순서대로
// 시도해 첫 성공에서 멈춘다:
// - macOS: `open -a Terminal <dir>` — LaunchServices 경유라 번들 GUI 앱의
//   최소 PATH와 무관하게 동작하고, Terminal.app은 OS 기본 제공이다.
// - Windows: PATH의 `wt.exe`(Windows Terminal) -> `cmd /c start`로 클래식
//   cmd 창 폴백. 둘 다 런처라 곧바로 종료한다. 콘솔 창이 튀지 않도록
//   CREATE_NO_WINDOW를 건다(vscode.rs와 동일 관례).
// - 그 외(Linux 등): gnome-terminal -> konsole -> xfce4-terminal ->
//   x-terminal-emulator 순.
//
// vscode.rs와 달리 후보마다 `wait_for_exit`가 있다: konsole·xterm류는
// 창이 닫힐 때까지 프로세스가 살아있어 `.status()`로 기다리면 커맨드가
// 창 수명만큼 블록된다 — 이런 후보는 spawn 성공(바이너리 실행됨) 자체를
// 성공으로 판정한다. 실행 파일 부재가 주 실패 모드이므로 spawn 에러만으로
// 다음 후보 폴백이 충분하다.

use std::path::Path;
use std::process::{Command, Stdio};

/// 실행 후보 하나: 프로그램 + 인자 목록 + 종료 대기 여부.
#[derive(Debug, PartialEq)]
pub struct LaunchCandidate {
    pub program: String,
    pub args: Vec<String>,
    /// true면 짧게 종료를 기다려 exit 0으로 성공 판정(즉시 끝나는 런처),
    /// false면 spawn 성공 자체를 성공으로 본다(창 수명만큼 살아있는 프로세스).
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

/// OS별 터미널 실행 후보를 시도 순서대로 돌려준다. 순수 함수 --
/// `os`는 `std::env::consts::OS` 값, `prefer_iterm`은 앱 설정
/// `externalTerminal == "iterm"`(macOS에서만 의미 — iTerm을 먼저 시도하고
/// 미설치로 실패하면 Terminal.app 폴백).
pub fn launch_candidates(os: &str, dir: &str, prefer_iterm: bool) -> Vec<LaunchCandidate> {
    match os {
        "macos" => {
            let mut v = Vec::new();
            if prefer_iterm {
                v.push(LaunchCandidate::new("open", &["-a", "iTerm", dir], true));
            }
            v.push(LaunchCandidate::new("open", &["-a", "Terminal", dir], true));
            v
        }
        "windows" => vec![
            LaunchCandidate::new("wt.exe", &["-d", dir], true),
            // start의 빈 문자열은 창 제목 자리 — 생략하면 경로가 제목으로 먹힌다.
            LaunchCandidate::new(
                "cmd.exe",
                &["/c", "start", "", "/d", dir, "cmd.exe"],
                true,
            ),
        ],
        _ => vec![
            LaunchCandidate::new("gnome-terminal", &[&format!("--working-directory={dir}")], true),
            LaunchCandidate::new("konsole", &["--workdir", dir], false),
            LaunchCandidate::new("xfce4-terminal", &[&format!("--working-directory={dir}")], false),
            // 작업 폴더 플래그가 표준화돼 있지 않다 — 자식의 cwd 상속에 맡긴다.
            LaunchCandidate::new("x-terminal-emulator", &[], false),
        ],
    }
}

/// `dir`을 외부 터미널 앱으로 연다. 디렉터리가 아니거나 전 후보 실패 시
/// 사용자에게 그대로 보여줄 수 있는 한국어 에러 문자열을 돌려준다.
pub fn open_dir_in_terminal(dir: &str, prefer_iterm: bool) -> Result<(), String> {
    if !Path::new(dir).is_dir() {
        return Err(format!("작업 폴더를 찾을 수 없습니다: {dir}"));
    }

    for c in launch_candidates(std::env::consts::OS, dir, prefer_iterm) {
        let mut cmd = Command::new(&c.program);
        cmd.args(&c.args)
            // 플래그 없는 후보(x-terminal-emulator)가 시작 폴더를 물려받도록
            // 모든 후보에 cwd를 건다 — 명시 플래그가 있는 후보에는 무해.
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        if c.wait_for_exit {
            if matches!(cmd.status(), Ok(s) if s.success()) {
                return Ok(());
            }
        } else if cmd.spawn().is_ok() {
            return Ok(());
        }
    }
    Err("터미널 앱을 실행하지 못했습니다. 기본 터미널 설치 여부를 확인해 주세요.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_uses_launchservices_open() {
        let v = launch_candidates("macos", "/Users/me/proj", false);
        assert_eq!(
            v,
            vec![LaunchCandidate::new(
                "open",
                &["-a", "Terminal", "/Users/me/proj"],
                true
            )]
        );
    }

    #[test]
    fn macos_prefer_iterm_tries_iterm_then_terminal_fallback() {
        let v = launch_candidates("macos", "/Users/me/proj", true);
        assert_eq!(
            v,
            vec![
                LaunchCandidate::new("open", &["-a", "iTerm", "/Users/me/proj"], true),
                LaunchCandidate::new("open", &["-a", "Terminal", "/Users/me/proj"], true),
            ]
        );
    }

    #[test]
    fn prefer_iterm_does_not_affect_other_oses() {
        assert_eq!(
            launch_candidates("windows", "C:\\work\\proj", true),
            launch_candidates("windows", "C:\\work\\proj", false)
        );
        assert_eq!(
            launch_candidates("linux", "/home/me/proj", true),
            launch_candidates("linux", "/home/me/proj", false)
        );
    }

    #[test]
    fn windows_tries_wt_then_classic_cmd() {
        let v = launch_candidates("windows", "C:\\work\\proj", false);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].program, "wt.exe");
        assert_eq!(v[0].args, vec!["-d", "C:\\work\\proj"]);
        assert!(v[0].wait_for_exit);
        assert_eq!(v[1].program, "cmd.exe");
        assert_eq!(
            v[1].args,
            vec!["/c", "start", "", "/d", "C:\\work\\proj", "cmd.exe"]
        );
    }

    #[test]
    fn linux_candidates_only_wait_on_delegating_launchers() {
        let v = launch_candidates("linux", "/home/me/proj", false);
        assert_eq!(
            v.iter().map(|c| c.program.as_str()).collect::<Vec<_>>(),
            vec!["gnome-terminal", "konsole", "xfce4-terminal", "x-terminal-emulator"]
        );
        // gnome-terminal만 서버에 위임하고 즉시 종료 — 나머지는 창 수명만큼
        // 살아있으므로 기다리면 안 된다.
        assert_eq!(
            v.iter().map(|c| c.wait_for_exit).collect::<Vec<_>>(),
            vec![true, false, false, false]
        );
    }

    #[test]
    fn nonexistent_dir_is_rejected_before_any_launch() {
        let err = open_dir_in_terminal("/definitely/not/a/dir", false).unwrap_err();
        assert!(err.contains("/definitely/not/a/dir"), "err={err}");
    }

    /// 실제 터미널 창을 띄우는 수동 스모크(호스트 OS 경로 검증용).
    /// `cargo test terminal -- --ignored`로 실행한다.
    #[test]
    #[ignore = "실제 터미널을 실행함 -- 수동 확인 전용"]
    fn manual_smoke_opens_this_crate_dir() {
        open_dir_in_terminal(env!("CARGO_MANIFEST_DIR"), false).unwrap();
    }

    /// iTerm 우선 경로의 수동 스모크 — iTerm 설치 기기에서는 iTerm이,
    /// 미설치 기기에서는 Terminal 폴백이 떠야 한다.
    #[test]
    #[ignore = "실제 터미널을 실행함 -- 수동 확인 전용"]
    fn manual_smoke_opens_with_iterm_preference() {
        open_dir_in_terminal(env!("CARGO_MANIFEST_DIR"), true).unwrap();
    }
}
