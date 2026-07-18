// src-tauri/src/shell_export.rs
//
// 이슈 #42: 터미널 탭 화면의 현재 버퍼(스크롤백 포함)를 plain text로 받아
// 임시 .txt 파일로 쓰고, 사용자가 설정한 외부 에디터로 여는
// `export_terminal_output` 커맨드의 구현부(vscode.rs/terminal.rs와 동일 골격).
//
// 파일은 매번 새로 쓴다 -- 파일명에 timestamp가 들어가므로 덮어쓰기가
// 없다(같은 에이전트를 여러 번 내보내도 각각 별도 파일). 쓰기는
// settings_store.rs와 같은 temp+rename 원자 쓰기라 에디터가 반쯤 쓰인 파일을
// 열 일이 없다.
//
// 에디터 실행은 두 갈래다:
// - VS Code: vscode.rs의 후보(code CLI -> open -a / code.cmd 경로)를 재사용.
//   전부 즉시 끝나는 런처이므로 `.status()`로 성공 판정한다(wait_for_exit=true).
// - 시스템 기본: macOS `open`, Windows `cmd /c start`, 그 외 `xdg-open`.
//   이들도 파일을 기본 연결 앱에 넘기고 곧바로 종료하는 런처다.
// VS Code 후보가 전부 실패하면(미설치 등) 시스템 기본 후보로 폴백까지 시도해,
// "VS Code 선택했지만 미설치"인 사용자도 파일은 어떻게든 열리게 한다.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// 셸 출력 .txt를 쌓는 임시 디렉터리. observer temp와 같은 관례로
/// OS temp 아래 `agent-office/shell-output`.
pub fn export_dir() -> PathBuf {
    std::env::temp_dir()
        .join("agent-office")
        .join("shell-output")
}

/// 파일명 안전화: 영숫자·한글·`-`·`_`만 유지하고 나머지(공백·슬래시·콜론 등)는
/// `-`로 치환한다. 결과가 비면(전부 특수문자였던 경우) `"agent"`로 폴백하고,
/// 파일명이 지나치게 길어지지 않게 40자(chars 기준)로 자른다.
pub fn sanitize_agent_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars().take(40) {
        // 영숫자(유니코드 포함 -> 한글 alphabetic이 살아남는다)·`-`·`_`만 통과.
        if ch.is_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    if out.is_empty() {
        "agent".to_string()
    } else {
        out
    }
}

/// `export_dir()/<sanitized>-<timestamp>.txt`에 원자적으로 쓰고 그 경로를
/// 돌려준다. 디렉터리는 없으면 만든다. 에러는 사용자에게 그대로 보여줄 수
/// 있는 한국어 문자열.
pub fn write_export_file(
    agent_name: &str,
    content: &str,
    timestamp: &str,
) -> Result<PathBuf, String> {
    let dir = export_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("내보내기 폴더를 만들지 못했습니다: {e}"))?;

    let file = dir.join(format!("{}-{}.txt", sanitize_agent_name(agent_name), timestamp));
    // temp+rename 원자 쓰기(settings_store.rs와 동일 관례) -- 에디터가 반쯤
    // 쓰인 파일을 여는 것을 막는다.
    let tmp = dir.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, content.as_bytes())
        .map_err(|e| format!("셸 출력을 파일로 쓰지 못했습니다: {e}"))?;
    if let Err(e) = std::fs::rename(&tmp, &file) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("셸 출력 파일 저장에 실패했습니다: {e}"));
    }
    Ok(file)
}

/// 실행 후보 하나: 프로그램 + 인자 목록 + 종료 대기 여부(terminal.rs와 동일).
#[derive(Debug, PartialEq)]
pub struct LaunchCandidate {
    pub program: String,
    pub args: Vec<String>,
    /// true면 짧게 종료를 기다려 exit 0으로 성공 판정(즉시 끝나는 런처).
    /// 셸 출력 내보내기의 모든 후보(code/open/xdg-open/start)는 런처라 true.
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

/// OS·에디터별 .txt 열기 후보를 시도 순서대로 돌려준다. 순수 함수 --
/// `os`는 `std::env::consts::OS`, `local_app_data`는 Windows `%LOCALAPPDATA%`.
///
/// `use_vscode`=true면 vscode.rs의 후보를 재사용해 wait_for_exit=true로 감싼다
/// (vscode 후보는 전부 즉시 끝나는 런처). false면 OS 기본 연결로 연다:
/// macOS `open`, Windows `cmd /c start`, 그 외 `xdg-open`.
pub fn launch_candidates(
    os: &str,
    file: &str,
    use_vscode: bool,
    local_app_data: Option<&str>,
) -> Vec<LaunchCandidate> {
    if use_vscode {
        return crate::vscode::launch_candidates(os, file, local_app_data)
            .into_iter()
            .map(|c| LaunchCandidate {
                program: c.program,
                args: c.args,
                wait_for_exit: true,
            })
            .collect();
    }
    match os {
        "macos" => vec![LaunchCandidate::new("open", &[file], true)],
        "windows" => vec![
            // start의 빈 문자열은 창 제목 자리 -- 생략하면 경로가 제목으로 먹힌다.
            LaunchCandidate::new("cmd.exe", &["/c", "start", "", file], true),
        ],
        _ => vec![LaunchCandidate::new("xdg-open", &[file], true)],
    }
}

/// 후보 하나를 실행해 성공(런처가 exit 0으로 끝남)이면 true.
fn try_launch(c: &LaunchCandidate) -> bool {
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
    if c.wait_for_exit {
        matches!(cmd.status(), Ok(s) if s.success())
    } else {
        cmd.spawn().is_ok()
    }
}

/// `file`을 외부 에디터로 연다. `use_vscode`=true면 VS Code 후보를 먼저 시도하고,
/// 전부 실패하면(미설치 등) 시스템 기본 후보로 폴백까지 시도한다. 그래도 전부
/// 실패하면 사용자에게 그대로 보여줄 수 있는 한국어 에러 문자열.
pub fn open_file_in_editor(file: &Path, use_vscode: bool) -> Result<(), String> {
    let file_str = file.to_string_lossy();
    let local_app_data = std::env::var("LOCALAPPDATA").ok();
    let os = std::env::consts::OS;

    for c in launch_candidates(os, &file_str, use_vscode, local_app_data.as_deref()) {
        if try_launch(&c) {
            return Ok(());
        }
    }
    // VS Code를 골랐지만 후보가 전부 실패했으면(미설치 등) 시스템 기본으로
    // 폴백 -- 최소한 파일은 열리게 한다.
    if use_vscode {
        for c in launch_candidates(os, &file_str, false, local_app_data.as_deref()) {
            if try_launch(&c) {
                return Ok(());
            }
        }
    }
    Err("셸 출력을 열 에디터를 실행하지 못했습니다. 에디터 설정을 확인해 주세요.".to_string())
}

/// `export_dir()` 안에서 수정 시각이 `max_age`보다 오래된 파일을 지운다.
/// 디렉터리 부재·개별 파일 에러는 조용히 무시(GC는 best-effort, 부팅을
/// 막거나 실패를 사용자에게 알릴 필요가 없다).
pub fn gc_old_exports(max_age: std::time::Duration) {
    gc_dir(&export_dir(), max_age);
}

/// GC 본체 -- 디렉터리를 인자로 받아 테스트가 공유 `export_dir()` 대신
/// 스크래치 디렉터리로 격리할 수 있게 한다(병렬 테스트끼리 파일을 지우는
/// 경합 방지).
fn gc_dir(dir: &Path, max_age: std::time::Duration) {
    let now = std::time::SystemTime::now();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return; // 디렉터리 없음 = 지울 것도 없음
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else { continue };
        // now - modified >= max_age 면 삭제. duration_since는 modified가
        // 미래(시계 역행)면 Err -> 이 경우 최신 취급해 남긴다.
        if let Ok(age) = now.duration_since(modified) {
            if age >= max_age {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_alnum_hangul_dash_underscore() {
        assert_eq!(sanitize_agent_name("Ada_Backend-1"), "Ada_Backend-1");
        assert_eq!(sanitize_agent_name("백엔드담당"), "백엔드담당");
        assert_eq!(sanitize_agent_name("한글Mix_9"), "한글Mix_9");
    }

    #[test]
    fn sanitize_replaces_specials_with_dash() {
        assert_eq!(sanitize_agent_name("a b/c:d"), "a-b-c-d");
        assert_eq!(sanitize_agent_name("../etc"), "---etc");
    }

    #[test]
    fn sanitize_empty_falls_back_to_agent() {
        // 빈 입력만 "agent"로 폴백. 특수문자만 있어도 치환 결과가 비지 않으면
        // 그대로 대시로 남는다(빈 문자열이 아니므로 폴백 안 함).
        assert_eq!(sanitize_agent_name(""), "agent");
        assert_eq!(sanitize_agent_name("///"), "---");
        assert_eq!(sanitize_agent_name("   "), "---");
    }

    #[test]
    fn sanitize_truncates_to_40_chars() {
        let long = "가".repeat(100);
        let out = sanitize_agent_name(&long);
        assert_eq!(out.chars().count(), 40);
        assert!(out.chars().all(|c| c == '가'));
    }

    #[test]
    fn macos_system_candidate_is_open() {
        let v = launch_candidates("macos", "/tmp/x.txt", false, None);
        assert_eq!(
            v,
            vec![LaunchCandidate::new("open", &["/tmp/x.txt"], true)]
        );
    }

    #[test]
    fn windows_system_candidate_is_cmd_start() {
        let v = launch_candidates("windows", "C:\\tmp\\x.txt", false, None);
        assert_eq!(
            v,
            vec![LaunchCandidate::new(
                "cmd.exe",
                &["/c", "start", "", "C:\\tmp\\x.txt"],
                true
            )]
        );
    }

    #[test]
    fn linux_system_candidate_is_xdg_open() {
        let v = launch_candidates("linux", "/tmp/x.txt", false, None);
        assert_eq!(
            v,
            vec![LaunchCandidate::new("xdg-open", &["/tmp/x.txt"], true)]
        );
    }

    #[test]
    fn vscode_candidates_delegate_to_vscode_module_all_waiting() {
        let v = launch_candidates("macos", "/tmp/x.txt", true, None);
        assert_eq!(
            v,
            vec![
                LaunchCandidate::new("code", &["/tmp/x.txt"], true),
                LaunchCandidate::new(
                    "open",
                    &["-a", "Visual Studio Code", "/tmp/x.txt"],
                    true
                ),
            ]
        );
    }

    #[test]
    fn vscode_windows_candidates_include_user_and_system_installs() {
        let v = launch_candidates(
            "windows",
            "C:\\tmp\\x.txt",
            true,
            Some("C:\\Users\\me\\AppData\\Local"),
        );
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].program, "code.cmd");
        assert!(v.iter().all(|c| c.wait_for_exit));
    }

    #[test]
    fn write_export_file_roundtrips_and_leaves_no_temp() {
        let name = format!("test-agent-{}", uuid::Uuid::new_v4());
        let ts = "20260718-120000";
        let content = "line1\n한글 줄\nline3\n";
        let path = write_export_file(&name, content, ts).expect("write succeeds");

        assert!(path.exists());
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            format!("{}-{}.txt", sanitize_agent_name(&name), ts)
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);

        // 원자성: 같은 디렉터리에 .tmp- 잔여물이 없어야 한다.
        let has_tmp = std::fs::read_dir(export_dir())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with(".tmp-"));
        assert!(!has_tmp, "no temp file should remain");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn gc_removes_all_with_zero_max_age_but_keeps_with_large_max_age() {
        // 파일 mtime을 임의로 조작하기 어려우므로 두 극단으로 검증한다:
        // max_age=0이면 모든 파일이 "충분히 오래됨"으로 삭제되고,
        // 아주 큰 max_age면 방금 쓴 파일은 남는다. 공유 export_dir()가 아닌
        // 스크래치 디렉터리에 gc_dir로 격리 -- 병렬로 도는 다른 테스트의
        // 파일을 지우지 않는다.
        let dir = std::env::temp_dir().join(format!("shell-export-gc-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let p1 = dir.join("old.txt");
        std::fs::write(&p1, "x").unwrap();
        gc_dir(&dir, std::time::Duration::from_secs(0));
        assert!(!p1.exists(), "max_age=0이면 전부 삭제되어야 한다");

        let p2 = dir.join("fresh.txt");
        std::fs::write(&p2, "y").unwrap();
        gc_dir(&dir, std::time::Duration::from_secs(365 * 24 * 3600));
        assert!(p2.exists(), "큰 max_age면 방금 쓴 파일은 남아야 한다");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gc_on_missing_dir_is_noop() {
        // 디렉터리가 아직 없어도(첫 실행) 패닉/에러 없이 조용히 통과.
        let missing = std::env::temp_dir().join(format!("shell-export-gc-missing-{}", uuid::Uuid::new_v4()));
        gc_dir(&missing, std::time::Duration::from_secs(0));
    }

    /// 실제 에디터로 파일을 여는 수동 스모크(호스트 OS 검증용).
    /// `cargo test shell_export -- --ignored`로 실행한다.
    #[test]
    #[ignore = "실제 에디터를 실행함 -- 수동 확인 전용"]
    fn manual_smoke_opens_written_file() {
        let path = write_export_file("스모크", "hello\nworld\n", "manual").unwrap();
        open_file_in_editor(&path, false).unwrap();
    }
}
