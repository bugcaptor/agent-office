// src-tauri/src/ipc/commands/peer.rs
//
// 피어 세션 공유(#7k, docs/peer-session-share-design.md)의 렌더러 커맨드.
// 호스트 역할(공유 토글·페어링 승인·피어 관리)과 뷰어 역할(페어링·연결·상태)이
// 한 파일에 있다 — 앱 하나가 양쪽을 동시에 한다.
//
// 세션 IO(`subscribe_output`/`write_input`)는 여기가 아니라 `session.rs`의
// 진입점에서 `peer:` 접두사로 라우팅된다(백엔드에서 갈라야 우회 불가능한 게이트다).

use tauri::State;

use crate::peer::pairing::PeerHostRecord;
use crate::peer::protocol::PeerPermission;
use crate::peer::viewer::{PairStartOutcome, PeerStatus};
use crate::state::AppState;

/// 설정 다이얼로그 "세션 공유" 탭이 한 번에 읽는 호스트 쪽 상태.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerHostStatus {
    pub enabled: bool,
    pub running: bool,
    pub port: Option<u16>,
    pub host_name: String,
    /// 뷰어에게 불러 줄 주소 힌트(`100.x.y.z`). 못 구하면 None.
    pub address_hint: Option<String>,
    pub bind: String,
    /// 공유 중인 캐릭터 agentId.
    pub shared_agents: Vec<String>,
    /// 승인해 준 뷰어들(토큰은 절대 내보내지 않는다).
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
}

#[tauri::command(rename_all = "camelCase")]
pub async fn peer_host_status(app_state: State<'_, AppState>) -> Result<PeerHostStatus, String> {
    let settings = *app_state.settings.read().unwrap();
    let ctx = &app_state.peer_ctx;
    Ok(PeerHostStatus {
        enabled: settings.peer_share_enabled,
        running: app_state.peer_server.is_running(),
        port: app_state.peer_server.current_port(),
        host_name: ctx.host_name.clone(),
        address_hint: crate::peer::local_addr_hint(),
        bind: settings.peer_bind.as_str().to_string(),
        shared_agents: ctx.hub.shared_agents(),
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
                pairing_id: p.pairing_id,
                code: p.code,
                viewer_name: p.viewer_name,
            })
            .collect(),
    })
}

/// 캐릭터별 공유 토글(전체 공유 스위치는 두지 않는다 — §결정 5).
#[tauri::command(rename_all = "camelCase")]
pub async fn peer_set_shared(
    app_state: State<'_, AppState>,
    agent_id: String,
    shared: bool,
) -> Result<(), String> {
    app_state.peer_ctx.set_shared(&agent_id, shared)
}

/// 페어링 승인(권한 선택). 뷰어는 이 뒤에 코드를 제시해야 토큰을 받는다.
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

/// 승인 취소 — 토큰 폐기. 그 피어의 WS는 다음 재연결에서 401로 막힌다.
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

// ── 뷰어 역할 ────────────────────────────────────────────────────────

#[tauri::command(rename_all = "camelCase")]
pub async fn peer_viewer_status(app_state: State<'_, AppState>) -> Result<Vec<PeerStatus>, String> {
    Ok(app_state.peer_viewer.status())
}

/// 페어링 1단계 — 호스트 화면에 코드가 뜬다.
#[tauri::command(rename_all = "camelCase")]
pub async fn peer_pair_start(
    app_state: State<'_, AppState>,
    address: String,
) -> Result<PairStartOutcome, String> {
    let viewer_name = app_state.peer_viewer.viewer_name().to_string();
    crate::peer::viewer::pair_start(&address, &viewer_name).await
}

/// 페어링 2단계 — 코드를 제시한다. 호스트가 아직 승인 버튼을 안 눌렀으면
/// `Ok(false)`라 렌더러는 잠시 후 다시 부르면 된다.
#[tauri::command(rename_all = "camelCase")]
pub async fn peer_pair_finish(
    app_state: State<'_, AppState>,
    address: String,
    pairing_id: String,
    code: String,
) -> Result<bool, String> {
    let record = crate::peer::viewer::pair_complete(&address, &pairing_id, &code).await?;
    let Some(record) = record else {
        return Ok(false);
    };
    app_state.peer_viewer.remember(record.clone())?;
    app_state.peer_viewer.connect(record);
    Ok(true)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn peer_hosts(app_state: State<'_, AppState>) -> Result<Vec<PeerHostRecord>, String> {
    Ok(app_state.peer_viewer.hosts())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn peer_connect(app_state: State<'_, AppState>, peer_id: String) -> Result<(), String> {
    let Some(record) = app_state
        .peer_viewer
        .hosts()
        .into_iter()
        .find(|h| h.peer_id == peer_id)
    else {
        return Err("저장된 피어가 아닙니다".into());
    };
    app_state.peer_viewer.connect(record);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn peer_disconnect(app_state: State<'_, AppState>, peer_id: String) -> Result<(), String> {
    app_state.peer_viewer.disconnect(&peer_id);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn peer_forget_host(
    app_state: State<'_, AppState>,
    peer_id: String,
) -> Result<(), String> {
    app_state.peer_viewer.forget(&peer_id)
}
