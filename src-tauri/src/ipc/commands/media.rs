// src-tauri/src/ipc/commands/media.rs
//
// Portrait/sprite PNG storage plus the summarizer and PixelLab sprite
// generation commands. See `super`(`ipc::commands`) module doc for the
// shared no-lock-across-await contract (`summarize_text` and
// `generate_sprite_image` are the two exceptions that `.await` while
// holding no lock).

use tauri::State;

use crate::state::AppState;

#[tauri::command(rename_all = "camelCase")]
pub async fn save_portrait(
    app_state: State<'_, AppState>,
    agent_id: String,
    png_base64: String,
) -> Result<(), String> {
    let ids: Vec<String> = app_state
        .store
        .load()
        .agents
        .iter()
        .map(|a| a.id.clone())
        .collect();
    app_state
        .portrait_store
        .save(&agent_id, &png_base64, &ids)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn load_portrait(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<Option<String>, String> {
    app_state
        .portrait_store
        .load(&agent_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn delete_portrait(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<(), String> {
    app_state
        .portrait_store
        .delete(&agent_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn save_sprite(
    app_state: State<'_, AppState>,
    agent_id: String,
    png_base64: String,
) -> Result<(), String> {
    let ids: Vec<String> = app_state
        .store
        .load()
        .agents
        .iter()
        .map(|a| a.id.clone())
        .collect();
    app_state
        .sprite_store
        .save(&agent_id, &png_base64, &ids)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn load_sprite(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<Option<String>, String> {
    app_state
        .sprite_store
        .load(&agent_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn delete_sprite(app_state: State<'_, AppState>, agent_id: String) -> Result<(), String> {
    app_state
        .sprite_store
        .delete(&agent_id)
        .map_err(|e| e.to_string())
}

/// 머리 위 라벨 요약: 요청 시작 시 렌더러가 캡처한 provider의 로컬 CLI를
/// 호출한다. 유저 크레딧을 소모하므로 opt-in — 설정 OFF면
/// "summarizer-disabled"로 거절하고 렌더러가 원문 폴백으로 처리한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn summarize_text(
    app_state: State<'_, AppState>,
    provider: crate::persistence::settings_store::SummaryProvider,
    instruction: String,
    text: String,
    // 목적별 타임아웃(#66). 미전달(구 렌더러)이면 라벨(20초)로 취급.
    purpose: Option<crate::summarizer::SummaryPurpose>,
) -> Result<String, String> {
    // 설정 가드는 .await 이전에 떨어뜨린다(no-lock-across-await 계약) — 모델
    // 오버라이드는 값으로 복제해 들고 나온다.
    let models = {
        let guard = app_state.settings.read().unwrap();
        if !guard.summarizer_enabled {
            return Err("summarizer-disabled".to_string());
        }
        guard.summary_models.clone()
    };
    // OpenRouter 경로만 API 키를 쓴다 — 다른 provider일 때 굳이 키 파일을
    // 읽지 않는다(라벨 요약은 자주 도는 경로다).
    let openrouter_key = if provider
        == crate::persistence::settings_store::SummaryProvider::Openrouter
    {
        app_state.tts.keys.openrouter_key()
    } else {
        None
    };
    crate::summarizer::summarize(
        provider,
        purpose.unwrap_or_default(),
        &instruction,
        &text,
        &models,
        openrouter_key.as_deref(),
    )
    .await
}

/// PixelLab로 64×64 스프라이트 1장 생성. AppState 비의존
/// (stateless) — 이 command만은 본문에 .await가 있으나 락을 전혀 잡지
/// 않으므로 파일 머리말의 "no lock across await" 계약과 무관하다.
#[tauri::command(rename_all = "camelCase")]
pub async fn generate_sprite_image(
    description: String,
) -> Result<crate::pixellab::GeneratedImage, String> {
    let trimmed = description.trim();
    if trimmed.is_empty() {
        return Err("validation: description is empty".to_string());
    }
    // pixen maxLength 2000 — 초과분은 뒤를 자른다 (char 경계 안전).
    let capped: String = trimmed
        .chars()
        .take(crate::pixellab::DESCRIPTION_MAX_CHARS)
        .collect();
    crate::pixellab::generate_image(&capped)
        .await
        .map_err(|e| e.to_ipc_string())
}
