// src-tauri/src/workdir/commands.rs
//
// `#[tauri::command]` 얇은 래퍼 8개. lib.rs의 `tauri::generate_handler![...]`가
// `workdir::workdir_*` 경로로 이 함수들을 직접 등록하므로(mod.rs의
// `pub use commands::*;`로 재수출), 함수 시그니처와 이름은 그대로 유지해야 한다.
//
// 각 래퍼는 테스트 가능한 순수 함수(listing/status/diff)에 위임하고, 시작 폴더
// UI가 `~/dev/foo`류 입력을 허용하므로 세션 생성과 동일한 틸드 확장을 거친다
// (open_in_vscode 관례).

use super::diff::{
    git_commit_files, git_diff_commit, git_diff_file, git_file_history, git_repo_log,
    launch_difftool,
};
use super::listing::list_workdir_files;
use super::model::{
    GitCommitFilesResult, GitDiffResult, GitFileHistoryResult, GitStatusResult, WorkdirListResult,
};
use super::status::collect_git_status;

/// `list_workdir_files`의 Tauri 커맨드 래퍼. 시작 폴더 UI가 `~/dev/foo`류
/// 입력을 허용하므로 세션 생성과 동일한 틸드 확장을 거친다(open_in_vscode 관례).
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_list_files(root: String) -> Result<WorkdirListResult, String> {
    list_workdir_files(&crate::session::manager::expand_tilde(root))
}

/// `collect_git_status`의 Tauri 커맨드 래퍼.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_git_status(root: String) -> Result<GitStatusResult, String> {
    collect_git_status(&crate::session::manager::expand_tilde(root))
}

/// `git_diff_file`의 Tauri 커맨드 래퍼.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_diff_file(
    root: String,
    rel_path: String,
    mode: String,
) -> Result<GitDiffResult, String> {
    git_diff_file(
        &crate::session::manager::expand_tilde(root),
        &rel_path,
        &mode,
    )
}

/// `git_file_history`의 Tauri 커맨드 래퍼.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_file_history(
    root: String,
    rel_path: String,
    limit: usize,
    skip: usize,
) -> Result<GitFileHistoryResult, String> {
    git_file_history(
        &crate::session::manager::expand_tilde(root),
        &rel_path,
        limit,
        skip,
    )
}

/// `git_diff_commit`의 Tauri 커맨드 래퍼.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_diff_commit(
    root: String,
    commit: String,
    rel_path: String,
) -> Result<GitDiffResult, String> {
    git_diff_commit(
        &crate::session::manager::expand_tilde(root),
        &commit,
        &rel_path,
    )
}

/// `git_commit_files`의 Tauri 커맨드 래퍼.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_commit_files(
    root: String,
    commit: String,
    limit: usize,
    skip: usize,
) -> Result<GitCommitFilesResult, String> {
    git_commit_files(
        &crate::session::manager::expand_tilde(root),
        &commit,
        limit,
        skip,
    )
}

/// `git_repo_log`의 Tauri 커맨드 래퍼.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_repo_log(
    root: String,
    limit: usize,
    skip: usize,
    all_branches: bool,
    query: String,
) -> Result<GitFileHistoryResult, String> {
    git_repo_log(
        &crate::session::manager::expand_tilde(root),
        limit,
        skip,
        all_branches,
        &query,
    )
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
