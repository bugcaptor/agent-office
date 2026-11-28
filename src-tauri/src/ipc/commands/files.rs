// src-tauri/src/ipc/commands/files.rs
//
// 머신 간 파일 보내기(`crate::files`) 커맨드: 설정·대상·기록·보내기·취소·이어 보내기, 파일 고르기 창, 폴더 열기.
use tauri::State;

use crate::files::{FilesConfig, FilesStatus, SendRequest, Target, Transfer};
use crate::state::AppState;

#[tauri::command]
pub fn files_status(app_state: State<'_, AppState>) -> FilesStatus {
    app_state.files.status()
}

#[tauri::command]
pub fn files_configure(config: FilesConfig, app_state: State<'_, AppState>) -> Result<FilesStatus, String> {
    app_state.files.configure(config)
}

#[tauri::command]
pub fn files_targets(app_state: State<'_, AppState>) -> Vec<Target> {
    app_state.files.targets()
}

#[tauri::command]
pub fn files_list(app_state: State<'_, AppState>) -> Vec<Transfer> {
    app_state.files.list()
}

/// 앱에서 보낸다. `via`가 비면 `app`.
#[tauri::command]
pub fn files_send(mut request: SendRequest, app_state: State<'_, AppState>) -> Result<Transfer, String> {
    if request.via.is_empty() {
        request.via = "app".into();
    }
    request.id = None;
    app_state.files.send(request)
}

#[tauri::command]
pub fn files_cancel(id: String, app_state: State<'_, AppState>) -> Result<(), String> {
    app_state.files.cancel(&id)
}

#[tauri::command]
pub fn files_resume(id: String, app_state: State<'_, AppState>) -> Result<(), String> {
    app_state.files.resume(&id)
}

/// 기록 한 건을 지운다(끝난 것만).
#[tauri::command]
pub fn files_forget(id: String, app_state: State<'_, AppState>) -> Result<(), String> {
    app_state.files.forget(&id)
}

/// 보낼 파일(여럿)이나 폴더(여럿)를 고른다. 취소하면 빈 목록.
#[tauri::command]
pub async fn files_pick(folders: bool, app: tauri::AppHandle) -> Result<Vec<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let builder = app.dialog().file();
    if folders {
        builder.pick_folders(move |picked| {
            let _ = tx.send(picked);
        });
    } else {
        builder.pick_files(move |picked| {
            let _ = tx.send(picked);
        });
    }
    let picked = rx.await.map_err(|_| "dialog-closed".to_string())?;
    Ok(picked
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
        .collect())
}

/// 받은 파일(또는 받은 파일 폴더)을 Finder·탐색기에서 보인다. 받은 파일 폴더 안만.
#[tauri::command]
pub fn files_reveal(path: Option<String>, app_state: State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let root = app_state.files.receive_root();
    let target = match path {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            return app.opener().open_path(root.to_string_lossy().into_owned(), None::<&str>).map_err(|e| e.to_string());
        }
    };
    if !target.starts_with(&root) || !target.exists() {
        return Err("not-found".into());
    }
    app.opener().reveal_item_in_dir(target).map_err(|e| e.to_string())
}

/// 에이전트끼리 조회·대화(`crate::peer`) 설정.
#[tauri::command]
pub fn peer_config(app_state: State<'_, AppState>) -> crate::peer::PeerConfig {
    app_state.peer.config()
}

#[tauri::command]
pub fn peer_configure(config: crate::peer::PeerConfig, app_state: State<'_, AppState>) -> Result<crate::peer::PeerConfig, String> {
    app_state.peer.configure(config)
}
