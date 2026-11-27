// src-tauri/src/ipc/commands/link.rs
//
// 머신 연동(`crate::link`) 설정·페어링 커맨드.
use tauri::State;

use crate::link::{LinkConfig, LinkStatus, RemoteOffice};
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

/// 호스트: 붙은 머신들의 캐릭터(오피스 씬의 머신별 층). 호스트 모드가 아니면 비어 있다.
#[tauri::command(rename_all = "camelCase")]
pub fn link_remote(app_state: State<'_, AppState>) -> RemoteOffice {
    app_state.link.remote.office_view()
}

/// 업무 분배에서 호스트가 원격 캐릭터 머신에 묻는 것(`work-send`·`work-followup`·`work-reply`·`usage`).
/// `agentId`는 원격 id(`기기id:캐릭터id`)다. 몸에 그 머신의 캐릭터 id를 넣어 넘긴다.
#[tauri::command(rename_all = "camelCase")]
pub async fn link_work(agent_id: String, kind: String, data: serde_json::Value, app_state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    if !matches!(kind.as_str(), "work-send" | "work-followup" | "work-reply" | "usage") {
        return Err("bad-request".into());
    }
    let remote = app_state.link.remote.clone();
    let (device, agent) = remote.resolve(&agent_id).ok_or("work-send-agent-missing")?;
    let mut data = if data.is_object() { data } else { serde_json::json!({}) };
    data["agentId"] = serde_json::json!(agent);
    if kind == "work-send" {
        // 받은 업무에 보일 호스트 이름.
        data["hostName"] = serde_json::json!(crate::webdash::computer_name());
    }
    remote.request_with(&device, &kind, data, crate::webdash::REMOTE_OP_TIMEOUT).await
}

/// 클라이언트: 받은 업무의 진행 상태를 호스트에 올린다(끊겨 있으면 쌓아 둔다, 업무별로 최신 하나).
/// 세션이 있으면 그 세션의 `office-task`가 호스트 보드를 쓰도록 적어 둔다.
#[tauri::command(rename_all = "camelCase")]
pub fn link_task_report(report: serde_json::Value, app_state: State<'_, AppState>) -> Result<(), String> {
    let link = &app_state.link;
    if link.config().mode != crate::link::LinkMode::Client {
        return Err("link-not-client".into());
    }
    let task = report.get("taskId").and_then(|v| v.as_str()).filter(|id| crate::persistence::work_inbox::valid_task_id(id)).ok_or("bad-request")?.to_string();
    if let Some(session) = report.get("sessionId").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        crate::link::work::ClientWork::new(app_state.app_data_dir.clone()).remember_session(session, &task);
    }
    link.client.enqueue(crate::link::work::KIND_TASK, report, Some(&format!("task:{task}")));
    Ok(())
}

/// 호스트: 붙은 머신들이 마지막으로 올린 받은 업무 진행 상태(렌더러가 뜰 때 한 번 받아 반영한다).
#[tauri::command(rename_all = "camelCase")]
pub fn link_task_states(app_state: State<'_, AppState>) -> Vec<serde_json::Value> {
    app_state.link.task_states.lock().unwrap().values().cloned().collect()
}
