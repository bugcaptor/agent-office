// src-tauri/src/ipc/commands/chat_view.rs
//
// 터미널 탭의 "대화 보기". CLI는 PTY에서 그대로 돌고, 보기만 xterm 대신 CLI 전사(JSONL)를
// 그린다 — 웹 상태판과 같은 수집기(`webdash::log`)·권한 중계(`observer::permission`)를 쓴다.
// 이미지는 웹의 이미지 끄기와 무관하게 늘 싣는다(`WebDashState::app_logs`).
//
// 입력은 키 흉내가 아니라 업무 보내기의 후속 지시 관문(`WorkSendRuntime::follow_up`)으로 넣는다.
// CLI가 작업 중이면 대기열(`chat_queue`, 웹 상태판과 공유)에 쌓았다가 턴이 끝날 때 넣는다.
// 권한·선택지는 훅 중계로 답한다 — 터미널 대화상자와 동시에 떠 있어 어느 보기에서 답하든
// 먼저 답한 쪽이 반영된다.
use std::collections::HashMap;

use base64::Engine as _;
use serde::Deserialize;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

use crate::chat_queue::{LiveStatus, SendOutcome};
use crate::observer::permission::{AskDecision, PendingAsk};
use crate::state::AppState;
use crate::webdash::log::LogPage;

/// 전사를 찾을 작업 폴더: 세션이 cd 한 곳, 없으면 캐릭터 프로필, 그것도 없으면 홈.
fn agent_cwd(app_state: &AppState, agent_id: &str) -> String {
    let ctl = &app_state.control_ctx;
    ctl.gate
        .sink()
        .cwd_of(agent_id)
        .or_else(|| {
            ctl.store
                .load()
                .agents
                .into_iter()
                .find(|a| a.id == agent_id)
                .and_then(|a| a.cwd)
                .filter(|c| !c.trim().is_empty())
                .map(crate::session::manager::expand_tilde)
        })
        .unwrap_or_else(crate::session::manager::home_dir)
}

/// 다른 머신 캐릭터(`기기id:캐릭터id`, 호스트 오피스의 머신별 층)면 (원격 보기, 기기, 그 머신의 캐릭터 id).
/// 그 머신에 물어 같은 대화 보기를 그리고, 입력·대기열 빼기·권한 답·세션 조작도 그 머신에 넘긴다
/// (그 머신 웹 상태판 조작과 같은 일, `link/share.rs`). 그 머신이 끊겨 있으면 `link-device-offline`.
fn remote_of(app_state: &AppState, agent_id: &str) -> Option<(std::sync::Arc<crate::link::RemoteView>, String, String)> {
    let remote = app_state.link.remote.clone();
    let (device, agent) = remote.resolve(agent_id)?;
    Some((remote, device, agent))
}

/// 그 머신에 조작 하나를 넘긴다(`op`: message·queue-cancel·restart·resume·terminate·interrupt 따위).
async fn remote_op(
    remote: &crate::link::RemoteView,
    device: &str,
    agent: &str,
    op: &str,
    extra: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut ask = serde_json::json!({ "agentId": agent, "op": op });
    if let (Some(obj), Some(more)) = (ask.as_object_mut(), extra.as_object()) {
        obj.extend(more.clone());
    }
    remote.request_with(device, "op", ask, crate::webdash::REMOTE_OP_TIMEOUT).await
}

/// 대화 한 쪽(`after`·`epoch`는 웹 상태판 `/api/log`와 같은 뜻).
#[tauri::command(rename_all = "camelCase")]
pub async fn chat_view_log(
    agent_id: String,
    after: Option<u64>,
    epoch: Option<u64>,
    app_state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    if let Some((remote, device, agent)) = remote_of(&app_state, &agent_id) {
        let ask = serde_json::json!({ "agentId": agent, "after": after, "epoch": epoch, "view": "app" });
        return remote.request(&device, "log", ask).await;
    }
    let cwd = agent_cwd(&app_state, &agent_id);
    let dash = app_state.web_dash.clone();
    let page: LogPage = tokio::task::spawn_blocking(move || dash.app_logs.read(&agent_id, &cwd, after, epoch))
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(page).map_err(|e| e.to_string())
}

fn data_url(media: &str, bytes: &[u8]) -> String {
    format!("data:{media};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// 로그 항목의 이미지(`data:` URL). 상자에서 밀려났으면 None.
#[tauri::command(rename_all = "camelCase")]
pub async fn chat_view_image(
    agent_id: String,
    image_id: String,
    app_state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    if let Some((remote, device, agent)) = remote_of(&app_state, &agent_id) {
        let found = crate::webdash::remote_image(&remote, &device, &agent, &image_id, "app").await;
        return Ok(found.map(|(media, bytes)| data_url(&media, &bytes)));
    }
    let dash = app_state.web_dash.clone();
    let found = tokio::task::spawn_blocking(move || dash.app_logs.image(&agent_id, &image_id))
        .await
        .map_err(|e| e.to_string())?;
    Ok(found.map(|(media, bytes)| data_url(&media, bytes.as_slice())))
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ChatDownload {
    Image { image_id: String },
    File { epoch: u64, seq: u64, index: usize },
}

/// 대화에 실린 원본 바이트를 네이티브 저장 창에서 고른 경로에 쓴다.
#[tauri::command(rename_all = "camelCase")]
pub async fn chat_view_download(
    app: tauri::AppHandle,
    agent_id: String,
    source: ChatDownload,
    app_state: State<'_, AppState>,
) -> Result<bool, String> {
    let (name, bytes) = if let Some((remote, device, agent)) = remote_of(&app_state, &agent_id) {
        remote_download(&remote, &device, &agent, source).await?
    } else {
        local_download(&app_state, agent_id, source).await?
    };

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_file_name(name).save_file(move |path| {
        let _ = tx.send(path);
    });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    tokio::fs::write(path, bytes.as_slice()).await.map_err(|e| e.to_string())?;
    Ok(true)
}

fn image_name(image_id: &str, media: &str) -> String {
    let ext = match media {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "bin",
    };
    format!("image-{image_id}.{ext}")
}

/// 다른 머신 대화에 실린 원본(그 머신에 묻는다).
async fn remote_download(
    remote: &crate::link::RemoteView,
    device: &str,
    agent: &str,
    source: ChatDownload,
) -> Result<(String, std::sync::Arc<Vec<u8>>), String> {
    match source {
        ChatDownload::Image { image_id } => {
            let (media, bytes) = crate::webdash::remote_image(remote, device, agent, &image_id, "app").await.ok_or("image-gone")?;
            Ok((image_name(&image_id, &media), std::sync::Arc::new(bytes)))
        }
        ChatDownload::File { epoch, seq, index } => {
            let ask = serde_json::json!({ "agentId": agent, "epoch": epoch, "seq": seq, "index": index, "view": "app" });
            let answer = remote.request(device, "file", ask).await.map_err(|e| if e == "not-found" { "file-gone".into() } else { e })?;
            let name = answer.get("name").and_then(serde_json::Value::as_str).unwrap_or("file").to_string();
            let bytes = crate::link::remote::decode_blob(&answer).ok_or("file-gone")?;
            Ok((name, std::sync::Arc::new(bytes)))
        }
    }
}

async fn local_download(
    app_state: &AppState,
    agent_id: String,
    source: ChatDownload,
) -> Result<(String, std::sync::Arc<Vec<u8>>), String> {
    let dash = app_state.web_dash.clone();
    tokio::task::spawn_blocking(move || match source {
        ChatDownload::Image { image_id } => {
            let (media, bytes) = dash.app_logs.image(&agent_id, &image_id).ok_or("image-gone")?;
            Ok((image_name(&image_id, &media), bytes))
        }
        ChatDownload::File { epoch, seq, index } => dash
            .app_logs
            .file(&agent_id, epoch, seq, index)
            .map(|(name, bytes)| (name, std::sync::Arc::new(bytes)))
            .ok_or("file-gone"),
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(str::to_string)
}

/// 이 캐릭터(탭) 세션이 답을 기다리는 권한 요청·질문.
#[tauri::command(rename_all = "camelCase")]
pub async fn chat_view_asks(agent_id: String, app_state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    if let Some((remote, device, agent)) = remote_of(&app_state, &agent_id) {
        return remote.request(&device, "asks", serde_json::json!({ "agentId": agent })).await;
    }
    let asks: Vec<PendingAsk> = match app_state.manager.session_id_for(&agent_id) {
        Some(sid) => app_state.observer.permissions().list(&sid),
        None => Vec::new(),
    };
    serde_json::to_value(asks).map_err(|e| e.to_string())
}

/// 권한 요청·질문에 답한다. 터미널에서 먼저 답했으면 `ask-gone`.
/// `agent_id`가 다른 머신 캐릭터면 그 머신 훅 중계로 답한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn chat_view_answer(
    ask_id: String,
    action: String,
    answers: Option<HashMap<String, String>>,
    message: Option<String>,
    agent_id: Option<String>,
    app_state: State<'_, AppState>,
) -> Result<(), String> {
    if let Some((remote, device, _)) = agent_id.as_deref().and_then(|a| remote_of(&app_state, a)) {
        let ask = serde_json::json!({ "askId": ask_id, "action": action, "answers": answers, "message": message });
        return remote.request(&device, "ask", ask).await.map(|_| ());
    }
    let decision = AskDecision::from_action(&action, answers, message)?;
    app_state.observer.permissions().answer(&ask_id, decision)
}

/// 대화 보기에서 쓴 지시를 같은 세션에 넣는다. CLI가 입력을 기다리면 바로 넣고,
/// 작업 중이면 대기열에 쌓아 턴이 끝날 때 넣는다(`chat_queue`). 답은 `sent`·`queued`.
/// 다른 머신 캐릭터면 그 머신 웹 상태판 메시지와 같다 — 셸 프롬프트만 있으면 CLI를 띄워 첫 프롬프트로
/// 넣는다(`launched`, 이 머신 대화 보기는 터미널이 옆에 있어 그러지 않는다).
#[tauri::command(rename_all = "camelCase")]
pub async fn chat_view_send(
    agent_id: String,
    text: String,
    app_state: State<'_, AppState>,
) -> Result<String, String> {
    if let Some((remote, device, agent)) = remote_of(&app_state, &agent_id) {
        crate::chat_queue::check_text(&text)?;
        let answer = remote_op(&remote, &device, &agent, "message", serde_json::json!({ "text": text })).await?;
        return Ok(answer.get("outcome").and_then(serde_json::Value::as_str).unwrap_or("sent").to_string());
    }
    let ctx = app_state.control_ctx.clone();
    Ok(match ctx.chat_queue.send(&ctx, &agent_id, &text).await? {
        SendOutcome::Sent => "sent",
        SendOutcome::Queued => "queued",
    }
    .to_string())
}

/// 지금 상태(생각 중·멈춤을 가를 재료)와 보낼 메시지 대기열.
#[tauri::command(rename_all = "camelCase")]
pub async fn chat_view_status(agent_id: String, app_state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    if let Some((remote, device, agent)) = remote_of(&app_state, &agent_id) {
        return remote.request(&device, "status", serde_json::json!({ "agentId": agent })).await;
    }
    let status: LiveStatus = crate::chat_queue::live_status(&app_state.control_ctx, &agent_id);
    serde_json::to_value(status).map_err(|e| e.to_string())
}

/// 대기열에 쌓아 둔 메시지 하나를 뺀다. 이미 넣었으면 `queue-gone`.
#[tauri::command(rename_all = "camelCase")]
pub async fn chat_view_cancel_queued(agent_id: String, id: String, app_state: State<'_, AppState>) -> Result<(), String> {
    if let Some((remote, device, agent)) = remote_of(&app_state, &agent_id) {
        return remote_op(&remote, &device, &agent, "queue-cancel", serde_json::json!({ "id": id })).await.map(|_| ());
    }
    if app_state.control_ctx.chat_queue.cancel(&agent_id, &id) {
        Ok(())
    } else {
        Err("queue-gone".into())
    }
}

/// 다른 머신 캐릭터의 세션 조작(호스트 대화 보기 머리 단추): `restart`(세션 열기·다시 열기), `resume`(Claude 이어하기),
/// `terminate`(닫기), `interrupt`(작업 중단 Esc). 이 머신 캐릭터는 터미널 탭에서 하므로 받지 않는다.
#[tauri::command(rename_all = "camelCase")]
pub async fn chat_view_remote_op(agent_id: String, op: String, app_state: State<'_, AppState>) -> Result<(), String> {
    if !REMOTE_SESSION_OPS.contains(&op.as_str()) {
        return Err("unknown-op".into());
    }
    let (remote, device, agent) = remote_of(&app_state, &agent_id).ok_or("link-not-remote")?;
    remote_op(&remote, &device, &agent, &op, serde_json::json!({})).await.map(|_| ())
}

const REMOTE_SESSION_OPS: &[&str] = &["restart", "resume", "terminate", "interrupt"];
