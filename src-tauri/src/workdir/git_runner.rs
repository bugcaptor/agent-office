// src-tauri/src/workdir/git_runner.rs
//
// git 서브프로세스 실행과 그 주변 안전장치(경로/커밋 인자 검증, canonical root
// 확보)를 모은다. status/diff 서브모듈이 공용으로 쓰는 하위 레벨 유틸리티라
// `pub(super)`로 workdir 트리 안에서만 보인다.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// git 서브프로세스 1회 실행 결과(제네릭). `spawn_failed`는 git 바이너리 부재
/// 등 실행 자체 실패, `timed_out`은 타임아웃으로 kill, `success`는 exit 0 여부,
/// `stdout`은 종료 코드와 무관하게 리더 스레드가 끝까지 읽은 표준출력.
pub(super) struct GitRun {
    pub(super) spawn_failed: bool,
    pub(super) timed_out: bool,
    pub(super) success: bool,
    pub(super) stdout: Vec<u8>,
}

/// git을 root에서 `args`로 한 번 실행한다. stdout은 별도 스레드로 끝까지 읽어
/// 파이프 교착을 막고(거대 diff는 수 MB), 타임아웃을 넘기면 자식을 죽인다.
/// stderr는 버린다(에러 메시지는 종료 코드/빈 stdout으로 판별).
pub(super) fn run_git(root: &Path, args: &[&str], timeout: Duration) -> GitRun {
    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => {
            return GitRun {
                spawn_failed: true,
                timed_out: false,
                success: false,
                stdout: Vec::new(),
            }
        }
    };

    let mut stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return GitRun {
                spawn_failed: true,
                timed_out: false,
                success: false,
                stdout: Vec::new(),
            };
        }
    };
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                thread::sleep(Duration::from_millis(15));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };

    let buf = rx.recv().unwrap_or_default();
    let _ = reader.join();

    match status {
        Some(s) => GitRun {
            spawn_failed: false,
            timed_out: false,
            success: s.success(),
            stdout: buf,
        },
        None => GitRun {
            spawn_failed: false,
            timed_out: true,
            success: false,
            stdout: buf,
        },
    }
}

/// git 커맨드에 넘길 상대경로를 검증·정규화한다. 절대경로·`..`·루트 컴포넌트를
/// 거부해 root 밖 접근을 막고, 반환값은 '/'로 정규화된 상대경로다. 이 값은 항상
/// `--` 뒤에 pathspec으로 넘겨(옵션 주입 차단) 선행 '-'가 있어도 안전하다.
pub(super) fn sanitize_rel_path(rel: &str) -> Result<String, String> {
    if rel.is_empty() {
        return Err("경로가 비어 있습니다".to_string());
    }
    let p = Path::new(rel);
    let mut parts: Vec<String> = Vec::new();
    for comp in p.components() {
        use std::path::Component;
        match comp {
            Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("작업 폴더 밖의 경로는 접근할 수 없습니다: {rel}"));
            }
        }
    }
    if parts.is_empty() {
        return Err(format!("잘못된 경로입니다: {rel}"));
    }
    Ok(parts.join("/"))
}

/// 커밋 인자가 안전한지(hex 7~40자) 검증한다. git rev로 넘기기 전 옵션·경로
/// 주입을 원천 차단하기 위함.
pub(super) fn valid_commit(commit: &str) -> bool {
    let n = commit.len();
    (7..=40).contains(&n) && commit.bytes().all(|b| b.is_ascii_hexdigit())
}

/// canonical root를 얻고 디렉터리인지 확인한다(diff/history 진입 공통 전처리).
pub(super) fn canon_dir(root: &str) -> Result<std::path::PathBuf, String> {
    let canon_root = std::fs::canonicalize(root)
        .map_err(|e| format!("작업 폴더를 찾을 수 없습니다: {root} ({e})"))?;
    if !canon_root.is_dir() {
        return Err(format!("작업 폴더가 디렉터리가 아닙니다: {root}"));
    }
    Ok(canon_root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rel_path_rejects_escapes() {
        assert!(sanitize_rel_path("").is_err());
        assert!(sanitize_rel_path("/etc/passwd").is_err());
        assert!(sanitize_rel_path("../secret").is_err());
        assert!(sanitize_rel_path("a/../../b").is_err());
        // 정상: 정규화되어 '/' 구분.
        assert_eq!(sanitize_rel_path("src/lib.rs").unwrap(), "src/lib.rs");
        assert_eq!(sanitize_rel_path("./src/./lib.rs").unwrap(), "src/lib.rs");
        // 선행 '-' 파일명은 통과한다(항상 `--` 뒤 pathspec으로 넘겨 안전).
        assert_eq!(sanitize_rel_path("-weird.txt").unwrap(), "-weird.txt");
    }

    #[test]
    fn valid_commit_accepts_only_hex_7_to_40() {
        assert!(valid_commit("dd7c2d8"));
        assert!(valid_commit("dd7c2d861e6c0619e58bed7340efebe2ae7915db"));
        assert!(!valid_commit("dd7c2d")); // 6자
        assert!(!valid_commit("HEAD"));
        assert!(!valid_commit("dd7c2d8; rm -rf /"));
        assert!(!valid_commit("../../etc"));
        // 41자 초과.
        assert!(!valid_commit("dd7c2d861e6c0619e58bed7340efebe2ae7915db0"));
    }
}
