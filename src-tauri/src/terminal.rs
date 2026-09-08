// src-tauri/src/terminal.rs
//
// 에이전트 작업 폴더를 OS 기본 터미널 앱으로 여는 `open_in_terminal`
// 커맨드의 구현부(vscode.rs와 동일 골격). OS별 실행 후보를 순서대로
// 시도해 첫 성공에서 멈춘다:
// - macOS: `open -a Terminal <dir>` — LaunchServices 경유라 번들 GUI 앱의
//   최소 PATH와 무관하게 동작하고, Terminal.app은 OS 기본 제공이다.
// - Windows: 기존 Windows Terminal 창(`wt -w 0 new-tab`) -> 기존 `wt -d`
//   동작 -> `cmd /c start` 순으로 폴백한다. 모두 런처라 곧바로 종료한다.
//   콘솔 창이 튀지 않도록 CREATE_NO_WINDOW를 건다(vscode.rs와 동일 관례).
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
/// `os`는 `std::env::consts::OS` 값이다. macOS iTerm 우선 처리는 탭 생성의
/// 성공 여부를 구분해야 하므로 `open_dir_in_terminal`에서 별도로 처리한다.
pub fn launch_candidates(os: &str, dir: &str, _prefer_iterm: bool) -> Vec<LaunchCandidate> {
    match os {
        // iTerm의 새 탭 처리는 open_dir_in_terminal에서 AppleScript로 한다.
        // 여기에는 해당 시도가 실패하기 전까지 쓰지 않을 Terminal.app 폴백만 둔다.
        "macos" => vec![LaunchCandidate::new("open", &["-a", "Terminal", dir], true)],
        "windows" => vec![
            // `-w 0`은 가장 최근 Windows Terminal 창을 대상으로 한다. 창이
            // 없으면 wt가 새 창을 만들고, `new-tab`은 그 창의 새 탭을 연다.
            LaunchCandidate::new("wt.exe", &["-w", "0", "new-tab", "-d", dir], true),
            // 구버전 wt나 `-w 0 new-tab`을 받지 않는 설치의 기존 동작 폴백.
            LaunchCandidate::new("wt.exe", &["-d", dir], true),
            // start의 빈 문자열은 창 제목 자리 — 생략하면 경로가 제목으로 먹힌다.
            LaunchCandidate::new("cmd.exe", &["/c", "start", "", "/d", dir, "cmd.exe"], true),
        ],
        _ => vec![
            LaunchCandidate::new(
                "gnome-terminal",
                &[&format!("--working-directory={dir}")],
                true,
            ),
            LaunchCandidate::new("konsole", &["--workdir", dir], false),
            LaunchCandidate::new(
                "xfce4-terminal",
                &[&format!("--working-directory={dir}")],
                false,
            ),
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
    if std::env::consts::OS == "macos" && prefer_iterm {
        return finish_iterm_open(
            dir,
            run_iterm_tab_command(&iterm_shell_command(dir)),
            run_candidate,
        );
    }
    open_candidates_with(
        dir,
        launch_candidates(std::env::consts::OS, dir, prefer_iterm),
        |_| true,
        run_candidate,
    )
}

/// iTerm 결과별 후속 동작을 실행 방식에서 분리한다. 테스트에서는 `launch`를
/// 주입해 실제 OS 터미널을 열지 않고 중단·폴백 순서를 검증한다.
fn finish_iterm_open<Launch>(
    dir: &str,
    result: ItermTabResult,
    launch: Launch,
) -> Result<(), String>
where
    Launch: FnMut(&LaunchCandidate, &str) -> bool,
{
    match result {
        ItermTabResult::Opened => Ok(()),
        // iTerm이 실행 중이 아니거나 생성 전 Apple Event가 거절된 경우에는
        // 기존 LaunchServices 열기를 먼저 시도한다.
        ItermTabResult::Fallback => {
            let mut candidates = vec![LaunchCandidate::new("open", &["-a", "iTerm", dir], true)];
            candidates.extend(launch_candidates("macos", dir, false));
            open_candidates_with(dir, candidates, |_| true, launch)
        }
        // create tab/window 호출 뒤에는 성공 여부가 불명확하다. Terminal.app을
        // 더 열면 사용자가 의도하지 않은 중복 창이 생길 수 있으므로 중단한다.
        ItermTabResult::Uncertain => Err(terminal_launch_error()),
    }
}

fn open_candidates_with<IsDir, Launch>(
    dir: &str,
    candidates: Vec<LaunchCandidate>,
    is_dir: IsDir,
    mut launch: Launch,
) -> Result<(), String>
where
    IsDir: FnOnce(&str) -> bool,
    Launch: FnMut(&LaunchCandidate, &str) -> bool,
{
    if !is_dir(dir) {
        return Err(format!("작업 폴더를 찾을 수 없습니다: {dir}"));
    }

    for c in candidates {
        if launch(&c, dir) {
            return Ok(());
        }
    }
    Err(terminal_launch_error())
}

fn terminal_launch_error() -> String {
    "터미널 앱을 실행하지 못했습니다. 기본 터미널 설치 여부를 확인해 주세요.".to_string()
}

#[derive(Debug, PartialEq)]
enum ItermTabResult {
    Opened,
    Fallback,
    Uncertain,
}

enum ItermScriptExecution {
    SpawnFailed,
    WaitFailed,
    Exited { success: bool, stdout: String },
}

const ITERM_SHELL_START_COMMAND: &str = r#"cd -- "$1" && exec "${SHELL:-/bin/zsh}" -l"#;

fn iterm_shell_command(dir: &str) -> String {
    [
        "/bin/sh",
        "-c",
        ITERM_SHELL_START_COMMAND,
        "agent-office",
        dir,
    ]
    .into_iter()
    .map(iterm_quote_arg)
    .collect::<Vec<_>>()
    .join(" ")
}

/// iTerm은 AppleScript `command` 문자열을 자체 토크나이저로 다시 나눈다.
/// 각 argv를 큰따옴표로 감싸고 그 안의 백슬래시와 큰따옴표만 이스케이프해
/// 디렉터리 경로가 `/bin/sh`의 `$1` 데이터로 전달되게 한다.
fn iterm_quote_arg(arg: &str) -> String {
    format!("\"{}\"", arg.replace('\\', "\\\\").replace('\"', "\\\""))
}

fn run_iterm_tab_command(command: &str) -> ItermTabResult {
    // iTerm 미설치·구버전에서 나는 AppleScript 컴파일 오류는 생성 시도 전
    // 실패다. 별도 컴파일 단계로 분리해 기존 open 후보가 계속 동작하게 한다.
    let temp_dir = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(_) => return ItermTabResult::Fallback,
    };
    let source_path = temp_dir.path().join("open_iterm_tab.applescript");
    let compiled_path = temp_dir.path().join("open_iterm_tab.scpt");
    if std::fs::write(&source_path, include_str!("open_iterm_tab.applescript")).is_err() {
        return ItermTabResult::Fallback;
    }
    if !matches!(
        Command::new("/usr/bin/osacompile")
            .arg("-o")
            .arg(&compiled_path)
            .arg(&source_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status(),
        Ok(status) if status.success()
    ) {
        return ItermTabResult::Fallback;
    }

    let execution = match Command::new("/usr/bin/osascript")
        .arg(&compiled_path)
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => match child.wait_with_output() {
            Ok(output) => ItermScriptExecution::Exited {
                success: output.status.success(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            },
            Err(_) => ItermScriptExecution::WaitFailed,
        },
        // spawn 전에 실패했으므로 AppleScript는 iTerm 생성 호출에 이르지 못했다.
        Err(_) => ItermScriptExecution::SpawnFailed,
    };
    classify_iterm_execution(execution)
}

fn classify_iterm_execution(execution: ItermScriptExecution) -> ItermTabResult {
    match execution {
        // spawn 전에 실패했으므로 AppleScript는 iTerm 생성 호출에 이르지 못했다.
        ItermScriptExecution::SpawnFailed => ItermTabResult::Fallback,
        // spawn 뒤 입출력 오류는 AppleScript 실행 여부를 알 수 없다.
        ItermScriptExecution::WaitFailed => ItermTabResult::Uncertain,
        ItermScriptExecution::Exited {
            success: true,
            stdout,
        } => classify_iterm_result(&stdout),
        // 컴파일된 스크립트의 비정상 종료도 생성 이후일 수 있다.
        ItermScriptExecution::Exited { success: false, .. } => ItermTabResult::Uncertain,
    }
}

fn classify_iterm_result(output: &str) -> ItermTabResult {
    let output = output.trim();
    if output == "opened" {
        ItermTabResult::Opened
    } else if output == "fallback" || output.starts_with("fallback:") {
        ItermTabResult::Fallback
    } else {
        ItermTabResult::Uncertain
    }
}

fn run_candidate(c: &LaunchCandidate, dir: &str) -> bool {
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
        matches!(cmd.status(), Ok(s) if s.success())
    } else {
        cmd.spawn().is_ok()
    }
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
    fn macos_candidates_keep_terminal_as_the_safe_fallback() {
        let v = launch_candidates("macos", "/Users/me/proj", true);
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
    fn iterm_script_result_classifies_only_explicit_precreation_failures_as_fallback() {
        assert_eq!(classify_iterm_result("opened\n"), ItermTabResult::Opened);
        assert_eq!(
            classify_iterm_result("fallback:-1743:no automation"),
            ItermTabResult::Fallback
        );
        assert_eq!(
            classify_iterm_result("uncertain:-1712:timeout"),
            ItermTabResult::Uncertain
        );
        assert_eq!(
            classify_iterm_result("unexpected"),
            ItermTabResult::Uncertain
        );
    }

    #[test]
    fn iterm_execution_mock_only_falls_back_before_script_spawn() {
        assert_eq!(
            classify_iterm_execution(ItermScriptExecution::SpawnFailed),
            ItermTabResult::Fallback
        );
        assert_eq!(
            classify_iterm_execution(ItermScriptExecution::WaitFailed),
            ItermTabResult::Uncertain
        );
        assert_eq!(
            classify_iterm_execution(ItermScriptExecution::Exited {
                success: false,
                stdout: String::new(),
            }),
            ItermTabResult::Uncertain
        );
        assert_eq!(
            classify_iterm_execution(ItermScriptExecution::Exited {
                success: true,
                stdout: "fallback".to_string(),
            }),
            ItermTabResult::Fallback
        );
    }

    #[test]
    fn iterm_command_keeps_special_directory_as_shell_positional_argument() {
        let dir = r#"/tmp/dragon ' " $HOME `tick` 영애 \\ space"#;
        let command = iterm_shell_command(dir);

        assert!(
            command.ends_with(&iterm_quote_arg(dir)),
            "command={command}"
        );
        assert!(command.contains(&iterm_quote_arg(ITERM_SHELL_START_COMMAND)));
    }

    #[cfg(unix)]
    #[test]
    fn iterm_shell_start_command_preserves_special_directory_as_positional_argument() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::tempdir().unwrap();
        let dir = temp_dir
            .path()
            .join("dragon ' \" $HOME `tick` 영애 \\ space");
        std::fs::create_dir(&dir).unwrap();
        let fake_shell = temp_dir.path().join("fake-shell");
        std::fs::write(&fake_shell, "#!/bin/sh\npwd\n").unwrap();
        std::fs::set_permissions(&fake_shell, std::fs::Permissions::from_mode(0o755)).unwrap();

        let output = Command::new("/bin/sh")
            .arg("-c")
            .arg(ITERM_SHELL_START_COMMAND)
            .arg("agent-office")
            .arg(&dir)
            .env("SHELL", &fake_shell)
            .output()
            .unwrap();

        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            dir.display().to_string()
        );
    }

    #[test]
    fn iterm_opened_and_uncertain_results_do_not_launch_fallback_candidates() {
        let mut opened_launches = 0;
        let opened = finish_iterm_open("/tmp/project", ItermTabResult::Opened, |_, _| {
            opened_launches += 1;
            true
        });
        assert!(opened.is_ok());
        assert_eq!(opened_launches, 0);

        let mut uncertain_launches = 0;
        let uncertain = finish_iterm_open("/tmp/project", ItermTabResult::Uncertain, |_, _| {
            uncertain_launches += 1;
            true
        });
        assert_eq!(uncertain.unwrap_err(), terminal_launch_error());
        assert_eq!(uncertain_launches, 0);
    }

    #[test]
    fn iterm_explicit_fallback_tries_iterm_then_terminal_and_stops_on_success() {
        let mut attempted = Vec::new();

        finish_iterm_open("/tmp/project", ItermTabResult::Fallback, |candidate, _| {
            attempted.push(candidate.args.clone());
            attempted.len() == 2
        })
        .unwrap();

        assert_eq!(
            attempted,
            vec![
                vec!["-a", "iTerm", "/tmp/project"],
                vec!["-a", "Terminal", "/tmp/project"],
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
    fn windows_tries_existing_window_tab_then_wt_then_classic_cmd() {
        let v = launch_candidates("windows", "C:\\work\\proj", false);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].program, "wt.exe");
        assert_eq!(
            v[0].args,
            vec!["-w", "0", "new-tab", "-d", "C:\\work\\proj"]
        );
        assert!(v[0].wait_for_exit);
        assert_eq!(v[1].program, "wt.exe");
        assert_eq!(v[1].args, vec!["-d", "C:\\work\\proj"]);
        assert_eq!(v[2].program, "cmd.exe");
        assert_eq!(
            v[2].args,
            vec!["/c", "start", "", "/d", "C:\\work\\proj", "cmd.exe"]
        );
    }

    #[test]
    fn successful_candidate_stops_before_later_candidates() {
        let candidates = launch_candidates("windows", "C:\\work", false);
        let mut attempted = Vec::new();

        open_candidates_with(
            "C:\\work",
            candidates,
            |_| true,
            |candidate, _| {
                attempted.push(candidate.program.clone());
                attempted.len() == 2
            },
        )
        .unwrap();

        assert_eq!(attempted, vec!["wt.exe", "wt.exe"]);
    }

    #[test]
    fn failed_wt_variants_fall_back_to_cmd() {
        let candidates = launch_candidates("windows", "C:\\work", false);
        let mut attempted = Vec::new();

        open_candidates_with(
            "C:\\work",
            candidates,
            |_| true,
            |candidate, _| {
                attempted.push(candidate.program.clone());
                candidate.program == "cmd.exe"
            },
        )
        .unwrap();

        assert_eq!(attempted, vec!["wt.exe", "wt.exe", "cmd.exe"]);
    }

    #[test]
    fn all_failed_candidates_return_error() {
        let candidates = launch_candidates("windows", "C:\\work", false);
        let mut attempted = 0;

        let err = open_candidates_with(
            "C:\\work",
            candidates,
            |_| true,
            |_, _| {
                attempted += 1;
                false
            },
        )
        .unwrap_err();

        assert_eq!(attempted, 3);
        assert!(err.contains("터미널 앱"), "err={err}");
    }

    #[test]
    fn invalid_dir_does_not_launch_any_candidate() {
        let candidates = launch_candidates("windows", "C:\\missing", false);
        let mut launched = false;

        let err = open_candidates_with(
            "C:\\missing",
            candidates,
            |_| false,
            |_, _| {
                launched = true;
                true
            },
        )
        .unwrap_err();

        assert!(!launched);
        assert!(err.contains("C:\\missing"), "err={err}");
    }

    #[test]
    fn special_path_is_kept_as_a_single_argv_value() {
        let dir = r"C:\\work & tools\\영애's project";
        let candidates = launch_candidates("windows", dir, false);

        assert_eq!(candidates[0].args[4], dir);
        assert_eq!(candidates[1].args[1], dir);
        assert_eq!(candidates[2].args[4], dir);
    }

    #[test]
    fn linux_candidates_only_wait_on_delegating_launchers() {
        let v = launch_candidates("linux", "/home/me/proj", false);
        assert_eq!(
            v.iter().map(|c| c.program.as_str()).collect::<Vec<_>>(),
            vec![
                "gnome-terminal",
                "konsole",
                "xfce4-terminal",
                "x-terminal-emulator"
            ]
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
