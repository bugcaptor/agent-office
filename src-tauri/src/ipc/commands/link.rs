// src-tauri/src/ipc/commands/link.rs
//
// 머신 연동(`crate::link`) 설정·페어링 커맨드.
use tauri::State;

use crate::link::{LinkConfig, LinkStatus};
use crate::state::AppState;

#[tauri::command(rename_all = "camelCase")]
pub fn link_status(app_state: State<'_, AppState>) -> LinkStatus {
    app_state.link.status()
}

/// 모드(끔·호스트·클라이언트)와 호스트 주소를 저장하고 건다.
#[tauri::command(rename_all = "camelCase")]
pub fn link_configure(config: LinkConfig, app_state: State<'_, AppState>) -> Result<LinkStatus, String> {
    app_state.link.configure(config)
}

/// 호스트: 새 페어링 코드를 띄운다(5분).
#[tauri::command(rename_all = "camelCase")]
pub fn link_start_pairing(app_state: State<'_, AppState>) -> Result<LinkStatus, String> {
    app_state.link.host.start_pairing()?;
    Ok(app_state.link.status())
}

#[tauri::command(rename_all = "camelCase")]
pub fn link_cancel_pairing(app_state: State<'_, AppState>) -> LinkStatus {
    app_state.link.host.cancel_pairing();
    app_state.link.status()
}

/// 호스트: 페어링된 기기 하나를 해제한다(붙어 있으면 끊긴다).
#[tauri::command(rename_all = "camelCase")]
pub fn link_revoke_device(device_id: String, app_state: State<'_, AppState>) -> LinkStatus {
    app_state.link.host.revoke(&device_id);
    app_state.link.status()
}

/// 클라이언트: 호스트 주소와 코드로 페어링한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn link_pair(address: String, code: String, app_state: State<'_, AppState>) -> Result<LinkStatus, String> {
    let link = app_state.link.clone();
    link.pair(&address, &code).await
}

/// 클라이언트: 이 기기의 페어링을 지운다.
#[tauri::command(rename_all = "camelCase")]
pub fn link_forget_host(app_state: State<'_, AppState>) -> Result<LinkStatus, String> {
    app_state.link.client.forget()?;
    Ok(app_state.link.status())
}
