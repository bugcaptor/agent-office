// src-tauri/src/automation/marker.rs
//
// 자동화 결과 파일(result.json) 및 프롬프트 파일 경로 생성과 읽기/검증.
// 계약(kbm #2t9 §3, §4):
// - 결과 파일 경로: <workspace>/.agent-office/automation-runs/<runId>/<stepExecutionId>/result.json
// - 심링크/경로 이탈(traversal) 거절 -- 마지막 요소뿐 아니라 실행 폴더 전체를 본다
// - 64KiB 크기 상한
// - runId·stepExecutionId 일치 확인 (남의 파일 거절)
// - 부분 JSON 등 파싱 에러는 무시(다음 폴링 틱에 다시 읽음)
// - 우리 실행의 파일인데 모르는 스키마 버전이면 **빠르게 실패**한다 — 조용히 무시하면
//   대기 시간을 통째로 쓰고 나서야 "시간 초과"로 보이고, 사용자는 CLI가 멈춘 줄 안다

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 결과 파일 크기 상한 (64KiB).
pub const MAX_RESULT_FILE_BYTES: u64 = 64 * 1024;

/// 결과 파일 내 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutomationResultStatus {
    Done,
    Complete,
    Blocked,
}

/// 결과 파일을 읽어 본 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultRead {
    /// 없거나 아직 온전히 못 읽는다(부분 쓰기 등). 다음 틱에 다시 본다.
    Pending,
    /// 남의 실행이 남긴 파일이다. 무시한다.
    Foreign,
    /// 우리 실행의 파일인데 모르는 스키마 버전이다. 여기서 실패로 끊는다.
    UnsupportedVersion(u64),
    /// 완전한 JSON이지만 v1 본문 규격이 잘못됐다. 수정 후 재검증할 수 있다.
    ProtocolError(String),
    /// 우리 파일이고 읽을 수 있다.
    Accepted(AutomationResultFile),
}

/// 버전과 실행 식별자만 먼저 본다. `status` 같은 나머지 필드가 우리가 모르는
/// 모양이어도 "누구 것인지"와 "몇 버전인지"는 알아야 판정이 선다.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResultEnvelope {
    version: Option<u64>,
    run_id: Option<String>,
    step_execution_id: Option<String>,
}

/// 결과 파일 스키마 (`version = 1`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutomationResultFile {
    pub version: u32,
    pub run_id: String,
    pub step_execution_id: String,
    pub status: AutomationResultStatus,
    #[serde(default)]
    pub summary: Option<String>,
}

/// 경로 이탈(`..` 또는 디렉터리 구분자) 방지 검증.
fn is_valid_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 128 {
        return false;
    }
    // 영숫자, 하이픈, 언더스코어만 허용 (uuid 등 안전한 식별자)
    id.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 특정 실행 단계의 실행 디렉터리 반환.
pub fn step_dir(
    workspace: &Path,
    run_id: &str,
    step_execution_id: &str,
) -> Result<PathBuf, String> {
    if !is_valid_id(run_id) {
        return Err(format!("invalid run_id: {run_id}"));
    }
    if !is_valid_id(step_execution_id) {
        return Err(format!("invalid step_execution_id: {step_execution_id}"));
    }
    Ok(workspace
        .join(".agent-office")
        .join("automation-runs")
        .join(run_id)
        .join(step_execution_id))
}

/// 결과 파일 절대경로 반환.
pub fn result_path(
    workspace: &Path,
    run_id: &str,
    step_execution_id: &str,
) -> Result<PathBuf, String> {
    Ok(step_dir(workspace, run_id, step_execution_id)?.join("result.json"))
}

/// 프롬프트 파일 절대경로 반환.
pub fn prompt_path(
    workspace: &Path,
    run_id: &str,
    step_execution_id: &str,
) -> Result<PathBuf, String> {
    Ok(step_dir(workspace, run_id, step_execution_id)?.join("prompt.md"))
}

/// 실행 폴더가 workspace 안에 실제로 들어 있는지 확인한다.
///
/// `is_valid_id` 는 우리가 만든 식별자에 `..` 나 구분자가 섞이는 것만 막는다.
/// 정작 위험한 쪽은 그 위다 -- `<workspace>/.agent-office` 가 심링크면
/// `create_dir_all` 이 그대로 따라가, 프롬프트와 결과 파일이 workspace 밖에
/// 만들어지고 그 절대경로가 CLI 지시에 실려 나간다. 그래서 폴더를 만든 **뒤**
/// 실경로(canonicalize)가 workspace 아래인지 한 번 더 본다.
pub fn verify_within_workspace(workspace: &Path, step_dir: &Path) -> Result<(), String> {
    let root = fs::canonicalize(workspace).map_err(|e| format!("workspace not resolvable: {e}"))?;
    let real = fs::canonicalize(step_dir).map_err(|e| format!("run dir not resolvable: {e}"))?;
    if !real.starts_with(&root) {
        return Err(format!("run dir escapes workspace: {}", real.display()));
    }
    Ok(())
}

/// `<workspace>/.agent-office/.gitignore` 가 없으면 만들어 폴더 전체를 무시하게 한다.
///
/// 실행 산출물(prompt.md·result.json)은 사용자 저장소 안에 쌓인다. 그대로 두면 남의
/// 리포에서 우리 제어 파일이 커밋 후보로 뜬다. 실패는 무시한다 — 무시 파일 하나
/// 못 썼다고 자동화를 멈출 이유는 없다.
pub fn ensure_ignored(workspace: &Path) {
    let dir = workspace.join(".agent-office");
    let ignore = dir.join(".gitignore");
    if ignore.exists() {
        return;
    }
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(&ignore, "# agent-office 자동화 실행 산출물\n*\n");
}

/// 결과 파일을 읽고 검증한다.
///
/// - 파일이 없거나 아직 온전치 않으면 `Pending`
/// - 심링크, 비정규 파일, 64KiB 초과도 `Pending`(다음 틱에 다시 본다)
/// - runId/stepExecutionId가 다르면 `Foreign`
/// - 우리 파일인데 `version != 1`이면 `UnsupportedVersion`
pub fn read_result_file(
    path: &Path,
    expected_run_id: &str,
    expected_step_execution_id: &str,
) -> ResultRead {
    let symlink_meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return ResultRead::Pending,
    };

    // 심링크 거부
    if symlink_meta.file_type().is_symlink() {
        return ResultRead::Pending;
    }
    // 일반 파일만 허용
    if !symlink_meta.is_file() {
        return ResultRead::Pending;
    }
    // 크기 상한 검사
    if symlink_meta.len() > MAX_RESULT_FILE_BYTES {
        return ResultRead::Pending;
    }

    // 검사와 읽기 사이에 파일이 심링크로 바뀌는 창(TOCTOU)을 줄이려고, 연 핸들
    // 자체의 메타데이터로 한 번 더 본다 -- `open` 은 심링크를 따라가므로 여기서
    // 걸리는 것은 "열고 보니 일반 파일이 아니거나 커진" 경우다.
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return ResultRead::Pending,
    };
    match file.metadata() {
        Ok(m) if m.is_file() && m.len() <= MAX_RESULT_FILE_BYTES => {}
        _ => return ResultRead::Pending,
    }
    let mut contents = String::new();
    let mut limited = std::io::Read::take(&mut file, MAX_RESULT_FILE_BYTES + 1);
    if std::io::Read::read_to_string(&mut limited, &mut contents).is_err()
        || contents.len() as u64 > MAX_RESULT_FILE_BYTES
    {
        return ResultRead::Pending;
    }

    // 1) 봉투부터 본다. 여기가 깨지면 아직 다 안 쓴 파일이라고 보고 다음 틱에 다시 읽는다.
    let envelope: ResultEnvelope = match serde_json::from_str(&contents) {
        Ok(e) => e,
        Err(_) => return ResultRead::Pending,
    };
    // 2) 누구 것인지. 남의 실행 파일이면 버전을 따지지 않는다 -- 남의 파일 때문에
    //    우리 실행이 죽으면 안 된다.
    if envelope.run_id.as_deref() != Some(expected_run_id)
        || envelope.step_execution_id.as_deref() != Some(expected_step_execution_id)
    {
        return ResultRead::Foreign;
    }
    // 3) 우리 것인데 모르는 버전이면 여기서 끊는다.
    match envelope.version {
        Some(1) => {}
        Some(v) => return ResultRead::UnsupportedVersion(v),
        None => return ResultRead::ProtocolError("missing result version".into()),
    }

    // 완전한 JSON과 실행 식별자를 확인했으므로 본문 오류는 프로토콜 오류다.
    match serde_json::from_str::<AutomationResultFile>(&contents) {
        Ok(parsed) => ResultRead::Accepted(parsed),
        Err(error) => ResultRead::ProtocolError(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn valid_id_checks() {
        assert!(is_valid_id("run-123_abc"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("../etc"));
        assert!(!is_valid_id("foo/bar"));
        assert!(!is_valid_id("foo\\bar"));
    }

    #[test]
    fn paths_formed_correctly() {
        let ws = Path::new("/workspace");
        let res = result_path(ws, "run-1", "step-1").unwrap();
        assert_eq!(
            res,
            Path::new("/workspace/.agent-office/automation-runs/run-1/step-1/result.json")
        );
        let pr = prompt_path(ws, "run-1", "step-1").unwrap();
        assert_eq!(
            pr,
            Path::new("/workspace/.agent-office/automation-runs/run-1/step-1/prompt.md")
        );
    }

    #[test]
    fn paths_reject_traversal() {
        let ws = Path::new("/workspace");
        assert!(result_path(ws, "../run", "step").is_err());
        assert!(result_path(ws, "run", "../step").is_err());
    }

    #[test]
    fn ensure_ignored_writes_self_ignoring_gitignore() {
        let dir = tempdir().unwrap();
        ensure_ignored(dir.path());
        let ignore = dir.path().join(".agent-office").join(".gitignore");
        let body = fs::read_to_string(&ignore).unwrap();
        assert!(body.contains("*"));

        // 이미 있으면 덮어쓰지 않는다.
        fs::write(&ignore, "keep-me").unwrap();
        ensure_ignored(dir.path());
        assert_eq!(fs::read_to_string(&ignore).unwrap(), "keep-me");
    }

    #[cfg(unix)]
    #[test]
    fn verify_within_workspace_rejects_symlinked_run_dir() {
        use std::os::unix::fs::symlink;
        let ws = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::create_dir_all(outside.path().join("elsewhere")).unwrap();
        // .agent-office 가 통째로 바깥을 가리키는 심링크인 경우.
        symlink(outside.path(), ws.path().join(".agent-office")).unwrap();

        let dir = step_dir(ws.path(), "run-1", "step-1").unwrap();
        fs::create_dir_all(&dir).unwrap();
        assert!(verify_within_workspace(ws.path(), &dir).is_err());
    }

    #[test]
    fn verify_within_workspace_accepts_normal_run_dir() {
        let ws = tempdir().unwrap();
        let dir = step_dir(ws.path(), "run-1", "step-1").unwrap();
        fs::create_dir_all(&dir).unwrap();
        assert!(verify_within_workspace(ws.path(), &dir).is_ok());
    }

    #[test]
    fn read_result_file_success() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("result.json");
        let content = r#"{
            "version": 1,
            "runId": "run-1",
            "stepExecutionId": "step-1",
            "status": "done",
            "summary": "작업 완료"
        }"#;
        fs::write(&path, content).unwrap();

        let ResultRead::Accepted(parsed) = read_result_file(&path, "run-1", "step-1") else {
            panic!("정상 파일은 Accepted 여야 한다");
        };
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.run_id, "run-1");
        assert_eq!(parsed.step_execution_id, "step-1");
        assert_eq!(parsed.status, AutomationResultStatus::Done);
        assert_eq!(parsed.summary.as_deref(), Some("작업 완료"));
    }

    #[test]
    fn read_result_file_mismatched_id_returns_none() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("result.json");
        let content = r#"{
            "version": 1,
            "runId": "other-run",
            "stepExecutionId": "step-1",
            "status": "done"
        }"#;
        fs::write(&path, content).unwrap();

        // 남의 실행 파일은 무시할 뿐 실패로 끊지 않는다.
        assert_eq!(
            read_result_file(&path, "run-1", "step-1"),
            ResultRead::Foreign
        );
    }

    #[test]
    fn read_result_file_unsupported_version_fails_fast() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("result.json");
        // 우리 실행의 파일인데 모르는 버전 -- 조용히 무시하면 대기 시간을 통째로 쓴다.
        fs::write(
            &path,
            r#"{"version":2,"runId":"run-1","stepExecutionId":"step-1","status":"done"}"#,
        )
        .unwrap();

        assert_eq!(
            read_result_file(&path, "run-1", "step-1"),
            ResultRead::UnsupportedVersion(2)
        );
    }

    #[test]
    fn read_result_file_unsupported_version_of_other_run_is_foreign() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("result.json");
        // 남의 파일이면 버전을 따지지 않는다 -- 남의 파일 때문에 우리가 죽으면 안 된다.
        fs::write(
            &path,
            r#"{"version":99,"runId":"other-run","stepExecutionId":"step-1","status":"done"}"#,
        )
        .unwrap();

        assert_eq!(
            read_result_file(&path, "run-1", "step-1"),
            ResultRead::Foreign
        );
    }

    #[test]
    fn read_result_file_unknown_status_reports_repairable_protocol_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("result.json");
        // 버전은 맞는데 status 를 모르겠다 -- 쓰다 만 파일일 수 있으니 다음 틱에 다시 본다.
        fs::write(
            &path,
            r#"{"version":1,"runId":"run-1","stepExecutionId":"step-1","status":"weird"}"#,
        )
        .unwrap();

        assert!(matches!(
            read_result_file(&path, "run-1", "step-1"),
            ResultRead::ProtocolError(_)
        ));
    }

    #[test]
    fn read_result_file_invalid_json_returns_none() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("result.json");
        fs::write(&path, "{\"version\": 1, partial").unwrap();

        assert_eq!(
            read_result_file(&path, "run-1", "step-1"),
            ResultRead::Pending
        );
    }

    #[test]
    fn read_result_file_missing_returns_none() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        assert_eq!(
            read_result_file(&path, "run-1", "step-1"),
            ResultRead::Pending
        );
    }

    #[test]
    fn read_result_file_oversized_returns_none() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("result.json");
        let oversized = vec![b' '; (MAX_RESULT_FILE_BYTES + 10) as usize];
        fs::write(&path, oversized).unwrap();

        assert_eq!(
            read_result_file(&path, "run-1", "step-1"),
            ResultRead::Pending
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_result_file_symlink_rejected() {
        use std::os::unix::fs::symlink;
        let dir = tempdir().unwrap();
        let target = dir.path().join("real.json");
        fs::write(
            &target,
            r#"{"version":1,"runId":"r","stepExecutionId":"s","status":"done"}"#,
        )
        .unwrap();

        let link = dir.path().join("link.json");
        symlink(&target, &link).unwrap();

        assert_eq!(read_result_file(&link, "r", "s"), ResultRead::Pending);
    }
}
