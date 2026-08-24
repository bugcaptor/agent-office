// src-tauri/src/ipc/commands/media.rs
//
// Portrait/sprite/minimi PNG storage plus the summarizer and codex image
// generation commands. See `super`(`ipc::commands`) module doc for the
// shared no-lock-across-await contract (`summarize_text` and
// `generate_codex_image` are the two exceptions that `.await` while
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

#[tauri::command(rename_all = "camelCase")]
pub async fn save_minimi(
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
        .minimi_store
        .save(&agent_id, &png_base64, &ids)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn load_minimi(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<Option<String>, String> {
    app_state
        .minimi_store
        .load(&agent_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn delete_minimi(app_state: State<'_, AppState>, agent_id: String) -> Result<(), String> {
    app_state
        .minimi_store
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
    let (models, lang) = {
        let guard = app_state.settings.read().unwrap();
        if !guard.summarizer_enabled {
            return Err("summarizer-disabled".to_string());
        }
        (guard.summary_models.clone(), crate::i18n::ui_lang(&guard))
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
        lang,
    )
    .await
}

/// 설정 화면의 provider별 모델 카탈로그 조회(요약·TTS 공용). 옛
/// `openrouter_list_models`를 일곱 개 provider로 일반화한 것 — `claude`/
/// `anthropic`만 저장된 Anthropic 키를 읽으므로 그때만 `app_state`를 만진다.
/// `summarizer_enabled`로 게이트하지 **않는다**(`tts_list_voices`와 같은
/// 판단: 기능을 켜기 전에도 어떤 모델을 고를 수 있는지 보여주는 편이 낫다).
///
/// 알 수 없는 provider·라이브 소스가 없는 provider·키 없음·조회 실패는 전부
/// 빈 목록이다(오류가 아니다) — 렌더러는 이를 정적 프리셋 폴백으로 조용히
/// 강등한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn list_provider_models(
    app_state: State<'_, AppState>,
    provider: String,
) -> Result<Vec<String>, String> {
    // 키 조회는 `claude`/`anthropic`일 때만 의미가 있지만, 문자열 비교
    // 하나 아끼자고 model_catalog에 키 스토어 자체를 넘기는 결합을 만들지
    // 않는다 — 값으로 떠서 넘기면 이 커맨드는 계속 AppState 비의존인 다른
    // 커맨드들과 같은 얇은 위임 모양을 유지한다.
    let anthropic_key = app_state.tts.keys.anthropic_key();
    Ok(crate::summarizer::list_provider_models(&provider, anthropic_key.as_deref()).await)
}

/// 로컬 codex CLI 설치 여부. AppState 비의존 — 프로필 편집의 "Codex로 생성"
/// 섹션이 버튼 활성/설치 안내를 정하는 데 쓴다. 미설치는 오류가 아니라
/// `available: false`다(캐시하지 않는다 — 설치 직후 재시도가 바로 통해야 한다).
#[tauri::command(rename_all = "camelCase")]
pub async fn codex_image_status() -> Result<crate::codex_imagegen::CodexImageStatus, String> {
    Ok(crate::codex_imagegen::status().await)
}

/// codex CLI의 내장 이미지 생성으로 PNG 1장 생성. AppState 비의존
/// (stateless) — 이 command만은 본문에 .await가 있으나 락을 전혀 잡지
/// 않으므로 파일 머리말의 "no lock across await" 계약과 무관하다.
/// 저장 경로 계약은 백엔드(`codex_imagegen`)가 소유한다 — 렌더러는 프롬프트만 준다.
#[tauri::command(rename_all = "camelCase")]
pub async fn generate_codex_image(
    prompt: String,
) -> Result<crate::codex_imagegen::GeneratedCodexImage, String> {
    crate::codex_imagegen::generate_image(&prompt)
        .await
        .map_err(|e| e.to_ipc_string())
}
