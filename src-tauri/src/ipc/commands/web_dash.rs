// src-tauri/src/ipc/commands/web_dash.rs
//
// 웹 상태판(`crate::webdash`) 설정·토큰·렌더러 다리 커맨드.
use std::collections::HashMap;

use tauri::State;

use crate::state::AppState;
use crate::webdash::{WebDashConfig, WebDashStatus};

#[tauri::command(rename_all = "camelCase")]
pub fn web_dash_status(app_state: State<'_, AppState>) -> WebDashStatus {
    app_state.web_dash.status()
}

/// 설정을 저장하고 서버를 그에 맞춘다(켜기·끄기·주소·포트 변경).
#[tauri::command(rename_all = "camelCase")]
pub async fn web_dash_configure(
    config: WebDashConfig,
    app_state: State<'_, AppState>,
) -> Result<WebDashStatus, String> {
    let dash = app_state.web_dash.clone();
    dash.configure(config, app_state.control_ctx.clone(), dash.clone()).await
}

/// 토큰을 새로 만든다. 기존 기기는 모두 끊긴다.
#[tauri::command(rename_all = "camelCase")]
pub fn web_dash_rotate_token(app_state: State<'_, AppState>) -> Result<WebDashStatus, String> {
    app_state.web_dash.rotate_token()?;
    Ok(app_state.web_dash.status())
}

/// 렌더러에만 있는 캐릭터 라벨을 웹 상태판에 넘긴다(통째로 교체).
#[tauri::command(rename_all = "camelCase")]
pub fn web_dash_publish_labels(labels: HashMap<String, serde_json::Value>, app_state: State<'_, AppState>) {
    app_state.web_dash.set_labels(labels);
}

/// 웹 명령(`web-dash-control` 이벤트)에 대한 렌더러의 답. `error`가 있으면 거절이다.
#[tauri::command(rename_all = "camelCase")]
pub fn web_dash_control_reply(
    request_id: String,
    error: Option<String>,
    data: Option<serde_json::Value>,
    app_state: State<'_, AppState>,
) -> bool {
    let result = match error {
        Some(e) => Err(e),
        None => Ok(data.unwrap_or(serde_json::Value::Null)),
    };
    app_state.web_dash.bridge.reply(&request_id, result)
}
