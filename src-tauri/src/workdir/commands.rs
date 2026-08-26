// src-tauri/src/workdir/commands.rs
//
// `#[tauri::command]` 얇은 래퍼 11개. lib.rs의 `tauri::generate_handler![...]`가
// `workdir::workdir_*` 경로로 이 함수들을 직접 등록하므로(mod.rs의
// `pub use commands::*;`로 재수출), 함수 시그니처와 이름은 그대로 유지해야 한다.
//
// 각 래퍼는 테스트 가능한 순수 함수(listing/status/diff)에 위임하고, 시작 폴더
// UI가 `~/dev/foo`류 입력을 허용하므로 세션 생성과 동일한 틸드 확장을 거친다
// (open_in_vscode 관례).
//
// **git 조회는 blocking + 취소 가능**(작업 폴더 보기 타임아웃 개편): 타임아웃이
// 분 단위(status 120s / diff·log 300s)로 늘어나면서, 그대로 async 함수 본문에서
// git을 돌리면 Tauri async 런타임 워커 하나를 그 시간 내내 점유해 다른 커맨드를
// 굶긴다. 그래서 조회 계열 본문은 전부 `spawn_blocking`으로 옮겼다. 대신 각
// 커맨드는 프런트가 만든 `opId`를 받아 취소 플래그를 등록하고,
// `workdir_git_cancel(opId)`이 그 플래그를 세워 자식 git을 즉시 죽인다 --
// "타임아웃은 백스톱, 1차 탈출구는 사용자 취소" 모델의 IPC 면이다.

use super::diff::{
    git_commit_files, git_diff_commit, git_diff_file, git_file_history, git_repo_log,
    launch_difftool,
};
use super::listing::{list_workdir_files, search_workdir_files};
use super::model::{
    GitBranchResult, GitCommitFilesResult, GitDiffResult, GitFileHistoryResult, GitStatusResult,
    WorkdirListResult, WorkdirSearchResult,
};
use super::status::{collect_git_branch, collect_git_status};

/// blocking 스레드에서 돌린 조회 결과를 커맨드 반환값으로 되돌린다. join 실패
/// (패닉/취소)는 사용자에게 보여줄 한국어 문자열로 바꾼다 -- 정상 흐름에서는
/// 나오지 않는다.
async fn run_blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("git 조회를 실행하지 못했습니다: {e}"))?
}

/// `list_workdir_files`의 Tauri 커맨드 래퍼. 시작 폴더 UI가 `~/dev/foo`류
/// 입력을 허용하므로 세션 생성과 동일한 틸드 확장을 거친다(open_in_vscode 관례).
///
/// `includeIgnored`는 앱 설정에서 읽지 않고 프런트가 그때의 토글 값을 그대로
/// 실어 보낸다 -- 팔레트에서 토글을 누른 직후 곧바로 재조회가 나가는데, 설정
/// 저장이 아직 백엔드에 반영되지 않았을 수 있어 설정을 읽으면 한 번은 옛 값으로
/// 스캔하게 된다. 생략되면(구버전 호출) 기존 동작인 false.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_list_files(
    root: String,
    include_ignored: Option<bool>,
) -> Result<WorkdirListResult, String> {
    list_workdir_files(
        &crate::session::manager::expand_tilde(root),
        include_ignored.unwrap_or(false),
    )
}

/// `search_workdir_files`의 Tauri 커맨드 래퍼(이슈 #67 -- 목록이 5000개
/// 상한에 걸려 잘린 뒤라도 팔레트 검색어로 Everything 인덱스를 다시 훑을 수
/// 있게 한다). 백엔드 설정 게이팅은 여기서 한다: `fileIndexBackend`가
/// `Walker`면 서버 검색을 아예 시도하지 않고 `usedIndex: false` + 빈 목록을
/// 즉시 돌려준다(프런트는 기존 클라이언트 fuzzy 필터로 폴백). `Everything`이면
/// `search_workdir_files`에 위임하고, es.exe 실패/빈 쿼리로 `usedIndex: false`가
/// 와도 그대로 프런트에 전달한다(같은 폴백 신호).
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_search_files(
    root: String,
    query: String,
    include_ignored: Option<bool>,
    app_state: tauri::State<'_, crate::state::AppState>,
) -> Result<WorkdirSearchResult, String> {
    use crate::persistence::settings_store::FileIndexBackend;

    let backend = app_state.settings.read().unwrap().file_index_backend;
    if backend != FileIndexBackend::Everything {
        return Ok(WorkdirSearchResult {
            files: Vec::new(),
            truncated: false,
            used_index: false,
        });
    }
    search_workdir_files(
        &crate::session::manager::expand_tilde(root),
        &query,
        include_ignored.unwrap_or(false),
    )
}

/// `collect_git_status`의 Tauri 커맨드 래퍼. `opId`를 주면 조회 중
/// `workdir_git_cancel`로 끊을 수 있다.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_git_status(
    root: String,
    op_id: Option<String>,
) -> Result<GitStatusResult, String> {
    let root = crate::session::manager::expand_tilde(root);
    run_blocking(move || collect_git_status(&root, op_id.as_deref())).await
}

/// `collect_git_branch`의 Tauri 커맨드 래퍼(라벨의 "프로젝트 (브랜치)" 표시용).
/// 다른 조회와 달리 `opId`가 없다 -- 30초 주기 폴링이라 취소 UI가 없고,
/// 타임아웃도 2초로 짧다. 반환은 항상 `Ok`이며, 실패는 `isRepo=false`로 표현한다
/// (`Result`인 것은 spawn_blocking join 실패를 옮기기 위한 형식일 뿐).
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_git_branch(root: String) -> Result<GitBranchResult, String> {
    let root = crate::session::manager::expand_tilde(root);
    run_blocking(move || Ok(collect_git_branch(&root))).await
}

/// `git_diff_file`의 Tauri 커맨드 래퍼.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_diff_file(
    root: String,
    rel_path: String,
    mode: String,
    op_id: Option<String>,
) -> Result<GitDiffResult, String> {
    let root = crate::session::manager::expand_tilde(root);
    run_blocking(move || git_diff_file(&root, &rel_path, &mode, op_id.as_deref())).await
}

/// `git_file_history`의 Tauri 커맨드 래퍼.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_file_history(
    root: String,
    rel_path: String,
    limit: usize,
    skip: usize,
    op_id: Option<String>,
) -> Result<GitFileHistoryResult, String> {
    let root = crate::session::manager::expand_tilde(root);
    run_blocking(move || git_file_history(&root, &rel_path, limit, skip, op_id.as_deref())).await
}

/// `git_diff_commit`의 Tauri 커맨드 래퍼.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_diff_commit(
    root: String,
    commit: String,
    rel_path: String,
    op_id: Option<String>,
) -> Result<GitDiffResult, String> {
    let root = crate::session::manager::expand_tilde(root);
    run_blocking(move || git_diff_commit(&root, &commit, &rel_path, op_id.as_deref())).await
}

/// `git_commit_files`의 Tauri 커맨드 래퍼.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_commit_files(
    root: String,
    commit: String,
    limit: usize,
    skip: usize,
    op_id: Option<String>,
) -> Result<GitCommitFilesResult, String> {
    let root = crate::session::manager::expand_tilde(root);
    run_blocking(move || git_commit_files(&root, &commit, limit, skip, op_id.as_deref())).await
}

/// `git_repo_log`의 Tauri 커맨드 래퍼.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_repo_log(
    root: String,
    limit: usize,
    skip: usize,
    all_branches: bool,
    query: String,
    op_id: Option<String>,
) -> Result<GitFileHistoryResult, String> {
    let root = crate::session::manager::expand_tilde(root);
    run_blocking(move || {
        git_repo_log(&root, limit, skip, all_branches, &query, op_id.as_deref())
    })
    .await
}

/// 진행 중인 git 조회에 취소를 요청한다(fire-and-forget). 해당 `opId`의 조회가
/// 이미 끝났거나 아직 시작되지 않았으면 조용한 no-op이므로 항상 성공한다 --
/// 프런트가 취소 응답을 기다릴 필요가 없다.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_git_cancel(op_id: String) -> Result<(), String> {
    super::git_runner::request_cancel(&op_id);
    Ok(())
}

/// `launch_difftool`의 Tauri 커맨드 래퍼. `commit`이 빈 문자열/미지정이면 현재
/// 변경을, 아니면 그 커밋의 변경을 외부 도구로 연다.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_difftool(
    root: String,
    rel_path: String,
    mode: String,
    commit: Option<String>,
) -> Result<(), String> {
    let commit_ref = commit.as_deref().filter(|c| !c.is_empty());
    launch_difftool(
        &crate::session::manager::expand_tilde(root),
        &rel_path,
        &mode,
        commit_ref,
    )
}
