// src-tauri/src/ipc/commands/chat_view.rs
//
// 터미널 탭의 "대화 보기". CLI는 PTY에서 그대로 돌고, 보기만 xterm 대신 CLI 전사(JSONL)를
// 그린다 — 웹 상태판과 같은 수집기(`webdash::log`)·권한 중계(`observer::permission`)를 쓴다.
// 이미지는 웹의 이미지 끄기와 무관하게 늘 싣는다(`WebDashState::app_logs`).
//
// 입력은 키 흉내가 아니라 업무 보내기의 후속 지시 관문(`WorkSendRuntime::follow_up`)으로 넣고,
// CLI가 턴을 마치고 입력을 기다릴 때만 받는다. 권한·선택지는 훅 중계로 답한다 — 터미널
// 대화상자와 동시에 떠 있어 어느 보기에서 답하든 먼저 답한 쪽이 반영된다.
use std::collections::HashMap;

use base64::Engine as _;
use serde::Deserialize;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

use crate::control::turns::TurnPhase;
use crate::observer::permission::{AskDecision, PendingAsk};
use crate::state::AppState;
use crate::types::WorkFollowUpRequest;
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

/// 대화 한 쪽(`after`·`epoch`는 웹 상태판 `/api/log`와 같은 뜻).
#[tauri::command(rename_all = "camelCase")]
pub async fn chat_view_log(
    agent_id: String,
    after: Option<u64>,
    epoch: Option<u64>,
    app_state: State<'_, AppState>,
) -> Result<LogPage, String> {
    let cwd = agent_cwd(&app_state, &agent_id);
    let dash = app_state.web_dash.clone();
    tokio::task::spawn_blocking(move || dash.app_logs.read(&agent_id, &cwd, after, epoch))
        .await
        .map_err(|e| e.to_string())
}

/// 로그 항목의 이미지(`data:` URL). 상자에서 밀려났으면 None.
#[tauri::command(rename_all = "camelCase")]
pub async fn chat_view_image(
    agent_id: String,
    image_id: String,
    app_state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let dash = app_state.web_dash.clone();
    let found = tokio::task::spawn_blocking(move || dash.app_logs.image(&agent_id, &image_id))
        .await
        .map_err(|e| e.to_string())?;
    Ok(found.map(|(media, bytes)| {
        format!("data:{media};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes.as_slice()))
    }))
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
    let dash = app_state.web_dash.clone();
    let (name, bytes) = tokio::task::spawn_blocking(move || match source {
        ChatDownload::Image { image_id } => {
            let (media, bytes) = dash.app_logs.image(&agent_id, &image_id).ok_or("image-gone")?;
            let ext = match media.as_str() {
                "image/png" => "png",
                "image/jpeg" => "jpg",
                "image/gif" => "gif",
                "image/webp" => "webp",
                _ => "bin",
            };
            Ok((format!("image-{image_id}.{ext}"), bytes))
        }
        ChatDownload::File { epoch, seq, index } => dash
            .app_logs
            .file(&agent_id, epoch, seq, index)
            .map(|(name, bytes)| (name, std::sync::Arc::new(bytes)))
            .ok_or("file-gone"),
    })
    .await
    .map_err(|e| e.to_string())??;

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

/// 이 캐릭터(탭) 세션이 답을 기다리는 권한 요청·질문.
#[tauri::command(rename_all = "camelCase")]
pub fn chat_view_asks(agent_id: String, app_state: State<'_, AppState>) -> Vec<PendingAsk> {
    match app_state.manager.session_id_for(&agent_id) {
        Some(sid) => app_state.observer.permissions().list(&sid),
        None => Vec::new(),
    }
}

/// 권한 요청·질문에 답한다. 터미널에서 먼저 답했으면 `ask-gone`.
#[tauri::command(rename_all = "camelCase")]
pub fn chat_view_answer(
    ask_id: String,
    action: String,
    answers: Option<HashMap<String, String>>,
    message: Option<String>,
    app_state: State<'_, AppState>,
) -> Result<(), String> {
    let decision = AskDecision::from_action(&action, answers, message)?;
    app_state.observer.permissions().answer(&ask_id, decision)
}

/// 대화 보기에서 쓴 지시를 같은 세션에 넣는다. CLI가 입력을 기다릴 때만 받는다.
#[tauri::command(rename_all = "camelCase")]
pub async fn chat_view_send(
    agent_id: String,
    text: String,
    app_state: State<'_, AppState>,
) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("empty".into());
    }
    let session_id = app_state.manager.session_id_for(&agent_id).ok_or("not-running")?;
    match app_state.control_ctx.turns.get(&session_id).map(|t| t.phase) {
        Some(TurnPhase::Idle) => {}
        Some(TurnPhase::NeedsHuman) => return Err("chat-view-needs-human".into()),
        Some(_) => return Err("chat-view-working".into()),
        None => return Err("chat-view-no-cli".into()),
    }
    app_state
        .work_send_runtime
        .follow_up(&app_state.gate, WorkFollowUpRequest { agent_id, session_id, text })
        .await
        .map(|_| ())
}
