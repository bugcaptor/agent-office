// src-tauri/src/ipc/commands/web_remote.rs
//
// 웹 원격(docs/web-remote-design.md)의 렌더러 커맨드 — 호스트 역할뿐이다
// (페어링 승인·클라이언트 관리·화면 스냅샷 응답). 앱↔앱 접속은 범위
// 밖이라 뷰어 역할 커맨드가 없다.

use tauri::State;

use crate::webremote::protocol::ClientPermission;
use crate::state::AppState;

/// 설정 다이얼로그 "웹 원격" 섹션이 한 번에 읽는 호스트 쪽 상태.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebRemoteStatus {
    pub enabled: bool,
    pub running: bool,
    pub port: Option<u16>,
    pub host_name: String,
    /// 브라우저에 불러 줄 주소. 서버가 떠 있으면 **실제 바인드 주소**이고,
    /// 아직 안 떴으면 추정치다. 못 구하면 None.
    pub address_hint: Option<String>,
    pub bind: String,
    /// 로컬 인터페이스에서 tailscale 주소를 찾았는지. `bind == "tailnet"`인데
    /// false면 리스너가 루프백에만 열려 있다는 뜻이라 UI가 그 사실을 알린다.
    pub tailnet_found: bool,
    /// 승인해 준 브라우저들(토큰은 절대 내보내지 않는다).
    pub clients: Vec<ClientSummary>,
    /// 승인 대기 중인 페어링.
    pub pending: Vec<PendingSummary>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientSummary {
    pub client_id: String,
    pub name: String,
    pub permission: ClientPermission,
    pub created_at: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingSummary {
    pub pairing_id: String,
    pub code: String,
    pub client_name: String,
    /// 코드가 만료되기까지 남은 시간(ms).
    pub expires_in_ms: u64,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn web_remote_status(app_state: State<'_, AppState>) -> Result<WebRemoteStatus, String> {
    let settings = *app_state.settings.read().unwrap();
    let ctx = &app_state.web_remote_ctx;
    let bound = app_state.web_remote_server.current_bound();
    // 서버가 떠 있으면 실제로 바인드된 주소가 정답이다(0.0.0.0은 사람에게
    // 불러 줄 주소가 아니므로 그때만 추정치로 되돌아간다).
    let address_hint = bound
        .map(|b| b.ip)
        .filter(|ip| !ip.is_unspecified())
        .map(|ip| ip.to_string())
        .or_else(crate::webremote::local_addr_hint);
    Ok(WebRemoteStatus {
        enabled: settings.web_remote_enabled,
        running: app_state.web_remote_server.is_running(),
        port: app_state.web_remote_server.current_port(),
        host_name: ctx.host_name.clone(),
        address_hint,
        bind: settings.web_remote_bind.as_str().to_string(),
        tailnet_found: match bound {
            Some(b) => b.tailnet_found,
            None => crate::webremote::choose_bind_ip(
                settings.web_remote_bind,
                &crate::webremote::local_ip_addrs(),
            )
            .tailnet_found,
        },
        clients: ctx
            .tokens
            .load()
            .into_iter()
            .map(|p| ClientSummary {
                client_id: p.client_id,
                name: p.name,
                permission: p.permission,
                created_at: p.created_at,
            })
            .collect(),
        pending: ctx
            .pairing
            .list()
            .into_iter()
            .filter(|p| p.decision == crate::webremote::pairing::PairingDecision::Pending)
            .map(|p| PendingSummary {
                expires_in_ms: p.remaining_ms(),
                pairing_id: p.pairing_id,
                code: p.code,
                client_name: p.client_name,
            })
            .collect(),
    })
}

/// 페어링 승인(권한 선택). 브라우저는 이 뒤에 코드를 제시해야 토큰을 받는다.
#[tauri::command(rename_all = "camelCase")]
pub async fn web_remote_pair_approve(
    app_state: State<'_, AppState>,
    pairing_id: String,
    permission: ClientPermission,
) -> Result<bool, String> {
    Ok(app_state.web_remote_ctx.pairing.approve(&pairing_id, permission))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn web_remote_pair_reject(
    app_state: State<'_, AppState>,
    pairing_id: String,
) -> Result<bool, String> {
    Ok(app_state.web_remote_ctx.pairing.reject(&pairing_id))
}

/// 승인 취소 — 토큰 폐기. 그 클라이언트의 WS는 다음 재연결에서 401로 막힌다.
#[tauri::command(rename_all = "camelCase")]
pub async fn web_remote_revoke(app_state: State<'_, AppState>, client_id: String) -> Result<(), String> {
    app_state
        .web_remote_ctx
        .tokens
        .remove(&client_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn web_remote_set_permission(
    app_state: State<'_, AppState>,
    client_id: String,
    permission: ClientPermission,
) -> Result<(), String> {
    app_state
        .web_remote_ctx
        .tokens
        .set_permission(&client_id, permission)
        .map_err(|e| e.to_string())
}

/// 호스트 렌더러가 스냅샷 요청(`web-remote-snapshot-request`)에 답하는 자리.
#[tauri::command(rename_all = "camelCase")]
pub async fn web_remote_submit_snapshot(
    app_state: State<'_, AppState>,
    request_id: String,
    snapshot: String,
) -> Result<(), String> {
    app_state
        .web_remote_ctx
        .hub
        .snapshots
        .submit(&request_id, snapshot);
    Ok(())
}
