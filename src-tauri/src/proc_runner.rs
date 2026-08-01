// src-tauri/src/proc_runner.rs
//
// 짧게 살다 죽는 서브프로세스 1회 실행기(공용). workdir/git_runner.rs(git),
// bot/gitea.rs(git, slug 감지), file_index/es_runner.rs(es.exe)가 각자 복제해
// 갖고 있던 동일 골격 -- spawn -> stdout을 별도 스레드로 읽어 파이프 교착 방지
// -> `try_wait` 폴 루프 + 데드라인 초과 시 kill -> 리더 스레드 회수 -- 을 한
// 곳으로 모았다.
//
// 세 호출부의 차이(cwd 지정 여부, env 강제, 출력 상한)는 `ProcSpec` 옵션으로
// 남겼고, 그 외 관찰 가능한 동작은 그대로다. 단 하나의 예외가 의도된 수정이다:
// 예전 gitea.rs는 타임아웃 kill 후 리더 스레드를 join하지 않아 스레드를 흘렸다.
// 여기서는 항상 join한다(`rx.recv()`가 이미 리더의 마지막 send를 기다리므로
// join은 사실상 즉시 끝난다).
//
// stderr는 세 호출부 모두 버린다(오류는 종료 코드/빈 stdout으로 판별). 그래서
// 캡처 옵션을 두지 않고 항상 `Stdio::null()`이다 -- 필요해지면 그때 추가한다.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

/// 자식 종료를 확인하는 폴 간격. 세 호출부가 쓰던 값(15ms) 그대로.
const POLL_INTERVAL: Duration = Duration::from_millis(15);
/// 상한 있는 읽기에서 한 번에 읽어들이는 청크 크기.
const READ_CHUNK: usize = 64 * 1024;

/// 실행 1회의 요청 명세. 필드를 전부 명시해 만드는 평범한 구조체다(빌더 없음):
/// 호출부가 셋뿐이라 기본값 뒤에 숨는 옵션보다 나열이 읽기 쉽다.
pub(crate) struct ProcSpec<'a> {
    /// 실행 파일 이름/경로. PATH 탐색은 OS에 맡긴다.
    pub(crate) program: &'a str,
    pub(crate) args: &'a [&'a str],
    /// `None`이면 현재 프로세스의 작업 디렉터리를 그대로 물려받는다.
    pub(crate) cwd: Option<&'a Path>,
    /// 자식에게 덮어쓸 환경변수(그 외는 상속).
    pub(crate) envs: &'a [(&'a str, &'a str)],
    /// 이 시간을 넘기면 자식을 kill하고 `TimedOut`.
    pub(crate) timeout: Duration,
    /// stdout 누적 상한. `Some(n)`이면 n 바이트를 넘는 순간 읽기를 멈추고
    /// 자식을 kill한 뒤 `Overflowed`. `None`이면 EOF까지 무제한으로 읽는다.
    pub(crate) max_stdout_bytes: Option<usize>,
}

/// 실행이 어떻게 끝났는지.
pub(crate) enum ProcOutcome {
    /// spawn 자체 실패(바이너리 부재 등) 또는 stdout 파이프 확보 실패.
    SpawnFailed,
    /// 정상적으로 종료 상태를 회수함.
    Exited { success: bool },
    /// 데드라인 초과 또는 `try_wait` 오류로 kill.
    TimedOut,
    /// `max_stdout_bytes` 초과로 kill.
    Overflowed,
}

/// 실행 결과. `stdout`은 종료 사유와 무관하게 리더 스레드가 읽어낸 만큼이다
/// (타임아웃 시에도 부분 출력이 담긴다 -- git_runner가 기대하던 동작).
pub(crate) struct ProcRun {
    pub(crate) outcome: ProcOutcome,
    pub(crate) stdout: Vec<u8>,
}

/// `spec`대로 한 번 실행하고 끝날 때까지(또는 타임아웃까지) 기다린다.
pub(crate) fn run(spec: ProcSpec) -> ProcRun {
    let mut cmd = Command::new(spec.program);
    cmd.args(spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(dir) = spec.cwd {
        cmd.current_dir(dir);
    }
    for (k, v) in spec.envs {
        cmd.env(k, v);
    }
    // GUI 앱에서 자식 콘솔 창이 깜빡이지 않게.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return spawn_failed(),
    };
    let mut stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return spawn_failed();
        }
    };

    let cap = spec.max_stdout_bytes;
    let overflowed = Arc::new(AtomicBool::new(false));
    let overflowed_reader = overflowed.clone();
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut buf = Vec::new();
        match cap {
            None => {
                let _ = stdout.read_to_end(&mut buf);
            }
            Some(max) => {
                let mut chunk = [0u8; READ_CHUNK];
                loop {
                    match stdout.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            if buf.len() + n > max {
                                overflowed_reader.store(true, Ordering::Relaxed);
                                break;
                            }
                            buf.extend_from_slice(&chunk[..n]);
                        }
                        Err(_) => break,
                    }
                }
            }
        }
        let _ = tx.send(buf);
    });

    let deadline = Instant::now() + spec.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline || overflowed.load(Ordering::Relaxed) {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                thread::sleep(POLL_INTERVAL);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };

    // recv()가 리더의 마지막 send를 기다리므로 join은 곧바로 끝난다. 이
    // join이 예전 gitea.rs에 없어 타임아웃 kill마다 스레드가 샜다.
    let buf = rx.recv().unwrap_or_default();
    let _ = reader.join();

    // 상한 초과는 종료 코드보다 우선한다(초과분을 버린 출력은 신뢰할 수 없다).
    let outcome = if overflowed.load(Ordering::Relaxed) {
        ProcOutcome::Overflowed
    } else {
        match status {
            Some(s) => ProcOutcome::Exited {
                success: s.success(),
            },
            None => ProcOutcome::TimedOut,
        }
    };
    ProcRun {
        outcome,
        stdout: buf,
    }
}

fn spawn_failed() -> ProcRun {
    ProcRun {
        outcome: ProcOutcome::SpawnFailed,
        stdout: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 존재하지 않는 바이너리는 패닉 없이 `SpawnFailed`.
    #[test]
    fn missing_binary_reports_spawn_failed() {
        let run = run(ProcSpec {
            program: "agent-office-no-such-binary-xyz",
            args: &[],
            cwd: None,
            envs: &[],
            timeout: Duration::from_secs(1),
            max_stdout_bytes: None,
        });
        assert!(matches!(run.outcome, ProcOutcome::SpawnFailed));
        assert!(run.stdout.is_empty());
    }

    /// 정상 종료 경로: stdout을 끝까지 읽고 exit 0을 회수한다.
    #[test]
    #[cfg(unix)]
    fn captures_stdout_and_exit_status() {
        let run = run(ProcSpec {
            program: "/bin/sh",
            args: &["-c", "printf hello"],
            cwd: None,
            envs: &[],
            timeout: Duration::from_secs(10),
            max_stdout_bytes: None,
        });
        assert!(matches!(run.outcome, ProcOutcome::Exited { success: true }));
        assert_eq!(run.stdout, b"hello");
    }

    /// 비정상 종료는 `Exited { success: false }` -- 타임아웃과 구분된다.
    #[test]
    #[cfg(unix)]
    fn nonzero_exit_is_not_success() {
        let run = run(ProcSpec {
            program: "/bin/sh",
            args: &["-c", "exit 3"],
            cwd: None,
            envs: &[],
            timeout: Duration::from_secs(10),
            max_stdout_bytes: None,
        });
        assert!(matches!(run.outcome, ProcOutcome::Exited { success: false }));
    }

    /// cwd 옵션이 자식에게 실제로 전달되는지.
    #[test]
    #[cfg(unix)]
    fn cwd_is_applied() {
        let dir = tempfile::tempdir().unwrap();
        let canon = std::fs::canonicalize(dir.path()).unwrap();
        let run = run(ProcSpec {
            program: "/bin/sh",
            args: &["-c", "pwd"],
            cwd: Some(&canon),
            envs: &[],
            timeout: Duration::from_secs(10),
            max_stdout_bytes: None,
        });
        let out = String::from_utf8_lossy(&run.stdout).trim().to_string();
        assert_eq!(out, canon.to_string_lossy());
    }

    /// env 옵션이 자식 환경을 덮어쓰는지(gitea의 LC_ALL=C 강제가 이 경로).
    #[test]
    #[cfg(unix)]
    fn envs_are_applied() {
        let run = run(ProcSpec {
            program: "/bin/sh",
            args: &["-c", "printf %s \"$AGENT_OFFICE_TEST_VAR\""],
            cwd: None,
            envs: &[("AGENT_OFFICE_TEST_VAR", "42")],
            timeout: Duration::from_secs(10),
            max_stdout_bytes: None,
        });
        assert_eq!(run.stdout, b"42");
    }

    /// 데드라인을 넘긴 자식은 kill되고 `TimedOut`. 리더 스레드 join까지
    /// 포함해 교착 없이 돌아와야 한다.
    #[test]
    #[cfg(unix)]
    fn slow_child_times_out_and_is_killed() {
        let run = run(ProcSpec {
            program: "/bin/sh",
            args: &["-c", "sleep 30"],
            cwd: None,
            envs: &[],
            timeout: Duration::from_millis(120),
            max_stdout_bytes: None,
        });
        assert!(matches!(run.outcome, ProcOutcome::TimedOut));
    }

    /// 상한을 넘기면 읽기를 멈추고 kill -> `Overflowed`(종료 코드보다 우선).
    #[test]
    #[cfg(unix)]
    fn output_over_cap_reports_overflow() {
        let run = run(ProcSpec {
            program: "/bin/sh",
            args: &["-c", "yes agent-office"],
            cwd: None,
            envs: &[],
            timeout: Duration::from_secs(10),
            max_stdout_bytes: Some(64 * 1024),
        });
        assert!(matches!(run.outcome, ProcOutcome::Overflowed));
    }
}
