// src-tauri/src/workdir/git_runner.rs
//
// git 서브프로세스 실행과 그 주변 안전장치(경로/커밋 인자 검증, canonical root
// 확보)를 모은다. status/diff 서브모듈이 공용으로 쓰는 하위 레벨 유틸리티라
// `pub(super)`로 workdir 트리 안에서만 보인다.
//
// 취소 모델: 거대 저장소의 git 조회는 분 단위로 걸릴 수 있어 타임아웃만으로는
// "무조건 중단"밖에 못 준다. 그래서 프런트가 조회마다 만든 `opId`를 함께 넘기고,
// 여기 프로세스 전역 레지스트리에 `AtomicBool` 플래그를 등록한다. 별도 커맨드
// (`workdir_git_cancel`)가 같은 opId로 그 플래그를 세우면 폴 루프가 자식 git을
// 죽이고 `canceled` 결과를 돌려준다. 등록 해제는 `CancelGuard`의 Drop이 맡아
// 조회가 어떤 경로로 끝나든(성공/에러/패닉) 레지스트리에 항목이 남지 않는다.

use crate::proc_runner::{self, ProcOutcome, ProcSpec};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// 진행 중인 git 조회의 취소 플래그 레지스트리(opId → 플래그). 조회가 끝나면
/// `CancelGuard`가 항목을 지우므로 크기는 동시 진행 조회 수를 넘지 않는다.
fn cancel_registry() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static REG: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 등록된 취소 플래그의 수명 가드. `flag()`를 `run_git`에 넘겨 쓰고, 드롭되는
/// 순간 레지스트리에서 자신을 지운다(조회 종료 = 등록 해제).
pub(super) struct CancelGuard {
    /// `None`이면 opId 없이 호출된 조회 -- 플래그는 있지만 아무도 세울 수 없다.
    op_id: Option<String>,
    flag: Arc<AtomicBool>,
}

impl CancelGuard {
    /// `run_git`에 넘길 취소 플래그.
    pub(super) fn flag(&self) -> &AtomicBool {
        &self.flag
    }
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        if let Some(id) = &self.op_id {
            if let Ok(mut reg) = cancel_registry().lock() {
                reg.remove(id);
            }
        }
    }
}

/// `op_id`로 취소 플래그를 등록하고 가드를 돌려준다. `op_id`가 `None`이면
/// 등록 없이 세울 수 없는 플래그만 만든다(테스트·내부 호출용).
///
/// 같은 op_id가 이미 있으면 덮어쓴다 -- 프런트가 조회마다 새 UUID를 만들므로
/// 정상 흐름에서는 충돌하지 않는다.
pub(super) fn register_cancel(op_id: Option<&str>) -> CancelGuard {
    let flag = Arc::new(AtomicBool::new(false));
    let op_id = op_id.map(|s| s.to_string());
    if let Some(id) = &op_id {
        if let Ok(mut reg) = cancel_registry().lock() {
            reg.insert(id.clone(), flag.clone());
        }
    }
    CancelGuard { op_id, flag }
}

/// `op_id`의 조회에 취소를 요청한다. 등록된 조회가 없으면(이미 끝났거나 아직
/// 시작 전이면) 조용한 no-op다.
pub(super) fn request_cancel(op_id: &str) {
    if let Ok(reg) = cancel_registry().lock() {
        if let Some(flag) = reg.get(op_id) {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

/// git 서브프로세스 1회 실행 결과(제네릭). `spawn_failed`는 git 바이너리 부재
/// 등 실행 자체 실패, `timed_out`은 타임아웃으로 kill, `canceled`는 사용자가
/// 취소해서 kill, `success`는 exit 0 여부, `stdout`은 종료 코드와 무관하게 리더
/// 스레드가 끝까지 읽은 표준출력.
pub(super) struct GitRun {
    pub(super) spawn_failed: bool,
    pub(super) timed_out: bool,
    pub(super) canceled: bool,
    pub(super) success: bool,
    pub(super) stdout: Vec<u8>,
}

/// git을 root에서 `args`로 한 번 실행한다. stdout은 별도 스레드로 끝까지 읽어
/// 파이프 교착을 막고(거대 diff는 수 MB), 타임아웃을 넘기거나 `cancel` 플래그가
/// 서면 자식을 죽인다. stderr는 버린다(에러 메시지는 종료 코드/빈 stdout으로 판별).
///
/// 실행 골격은 `crate::proc_runner`가 갖고, 여기서는 workdir이 기대하는
/// `GitRun` 표현으로만 옮긴다.
pub(super) fn run_git(
    root: &Path,
    args: &[&str],
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> GitRun {
    let run = proc_runner::run(ProcSpec {
        program: "git",
        args,
        cwd: Some(root),
        envs: &[],
        timeout,
        // 출력 상한 없음 -- 거대 diff도 끝까지 받는다(호출부가 자체 절단).
        max_stdout_bytes: None,
        cancel,
    });
    match run.outcome {
        ProcOutcome::SpawnFailed => GitRun {
            spawn_failed: true,
            timed_out: false,
            canceled: false,
            success: false,
            stdout: Vec::new(),
        },
        ProcOutcome::Exited { success } => GitRun {
            spawn_failed: false,
            timed_out: false,
            canceled: false,
            success,
            stdout: run.stdout,
        },
        // 취소는 타임아웃과 골격이 같지만 사유가 달라 UI 문구가 갈린다.
        ProcOutcome::Canceled => GitRun {
            spawn_failed: false,
            timed_out: false,
            canceled: true,
            success: false,
            stdout: run.stdout,
        },
        // Overflowed는 상한을 걸지 않았으니 나올 수 없다. 나온다면 타임아웃과
        // 똑같이 "결과를 믿을 수 없음"으로 취급하면 된다.
        ProcOutcome::TimedOut | ProcOutcome::Overflowed => GitRun {
            spawn_failed: false,
            timed_out: true,
            canceled: false,
            success: false,
            stdout: run.stdout,
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

    /// 등록된 op_id로 취소를 요청하면 그 조회의 플래그가 선다.
    #[test]
    fn request_cancel_sets_registered_flag() {
        let guard = register_cancel(Some("op-test-set"));
        assert!(!guard.flag().load(Ordering::Relaxed));
        request_cancel("op-test-set");
        assert!(guard.flag().load(Ordering::Relaxed));
    }

    /// 없는 op_id는 조용한 no-op(패닉 없음), 가드가 드롭되면 등록도 사라진다.
    #[test]
    fn unknown_cancel_is_noop_and_guard_unregisters() {
        request_cancel("op-test-never-registered"); // 패닉 없이 통과해야 한다.
        {
            let _guard = register_cancel(Some("op-test-drop"));
            assert!(cancel_registry().lock().unwrap().contains_key("op-test-drop"));
        }
        // 가드가 드롭되면 레지스트리에 남지 않는다 -- 이후 취소는 no-op.
        assert!(!cancel_registry().lock().unwrap().contains_key("op-test-drop"));
        request_cancel("op-test-drop");
    }

    /// op_id 없이 등록하면 레지스트리에 들어가지 않아 아무도 세울 수 없다
    /// (플래그는 항상 false로 남는다).
    #[test]
    fn register_without_op_id_is_unreachable() {
        let guard = register_cancel(None);
        assert!(!guard.flag().load(Ordering::Relaxed));
        // 레지스트리 어디에도 이 가드의 플래그가 없다(다른 테스트와 병렬이라
        // 크기 대신 포인터 동일성으로 확인).
        let reg = cancel_registry().lock().unwrap();
        assert!(!reg.values().any(|f| std::ptr::eq(&**f, guard.flag())));
    }
}
