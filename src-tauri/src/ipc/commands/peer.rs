// src-tauri/src/ipc/commands/peer.rs
//
// 웹 원격(docs/web-remote-design.md)의 렌더러 커맨드 — 호스트 역할뿐이다
// (페어링 승인·클라이언트 관리·화면 스냅샷 응답). 앱↔앱 피어 접속은 범위
// 밖이라 뷰어 역할 커맨드가 없다.

use tauri::State;

use crate::peer::protocol::PeerPermission;
use crate::state::AppState;

/// 설정 다이얼로그 "웹 원격" 섹션이 한 번에 읽는 호스트 쪽 상태.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerHostStatus {
    pub enabled: bool,
    pub running: bool,
    pub port: Option<u16>,
    pub host_name: String,
    /// 브라우저에 불러 줄 주소 힌트(`100.x.y.z`). 못 구하면 None.
    pub address_hint: Option<String>,
    pub bind: String,
    /// 승인해 준 브라우저들(토큰은 절대 내보내지 않는다).
    pub peers: Vec<PeerSummary>,
    /// 승인 대기 중인 페어링.
    pub pending: Vec<PendingSummary>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerSummary {
    pub peer_id: String,
    pub name: String,
    pub permission: PeerPermission,
    pub created_at: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingSummary {
    pub pairing_id: String,
    pub code: String,
    pub viewer_name: String,
    /// 코드가 만료되기까지 남은 시간(ms).
    pub expires_in_ms: u64,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn peer_host_status(app_state: State<'_, AppState>) -> Result<PeerHostStatus, String> {
    let settings = *app_state.settings.read().unwrap();
    let ctx = &app_state.peer_ctx;
    Ok(PeerHostStatus {
        enabled: settings.web_hosting_enabled,
        running: app_state.peer_server.is_running(),
        port: app_state.peer_server.current_port(),
        host_name: ctx.host_name.clone(),
        address_hint: crate::peer::local_addr_hint(),
        bind: settings.peer_bind.as_str().to_string(),
        peers: ctx
            .tokens
            .load()
            .into_iter()
            .map(|p| PeerSummary {
                peer_id: p.peer_id,
                name: p.name,
                permission: p.permission,
                created_at: p.created_at,
            })
            .collect(),
        pending: ctx
            .pairing
            .list()
            .into_iter()
            .filter(|p| p.decision == crate::peer::pairing::PairingDecision::Pending)
            .map(|p| PendingSummary {
                expires_in_ms: p.remaining_ms(),
                pairing_id: p.pairing_id,
                code: p.code,
                viewer_name: p.viewer_name,
            })
            .collect(),
    })
}

/// 페어링 승인(권한 선택). 브라우저는 이 뒤에 코드를 제시해야 토큰을 받는다.
#[tauri::command(rename_all = "camelCase")]
pub async fn peer_pair_approve(
    app_state: State<'_, AppState>,
    pairing_id: String,
    permission: PeerPermission,
) -> Result<bool, String> {
    Ok(app_state.peer_ctx.pairing.approve(&pairing_id, permission))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn peer_pair_reject(
    app_state: State<'_, AppState>,
    pairing_id: String,
) -> Result<bool, String> {
    Ok(app_state.peer_ctx.pairing.reject(&pairing_id))
}

/// 승인 취소 — 토큰 폐기. 그 클라이언트의 WS는 다음 재연결에서 401로 막힌다.
#[tauri::command(rename_all = "camelCase")]
pub async fn peer_revoke(app_state: State<'_, AppState>, peer_id: String) -> Result<(), String> {
    app_state
        .peer_ctx
        .tokens
        .remove(&peer_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn peer_set_permission(
    app_state: State<'_, AppState>,
    peer_id: String,
    permission: PeerPermission,
) -> Result<(), String> {
    app_state
        .peer_ctx
        .tokens
        .set_permission(&peer_id, permission)
        .map_err(|e| e.to_string())
}

/// 호스트 렌더러가 스냅샷 요청(`peer-snapshot-request`)에 답하는 자리.
#[tauri::command(rename_all = "camelCase")]
pub async fn submit_peer_snapshot(
    app_state: State<'_, AppState>,
    request_id: String,
    snapshot: String,
) -> Result<(), String> {
    app_state
        .peer_ctx
        .hub
        .snapshots
        .submit(&request_id, snapshot);
    Ok(())
}
