// src-tauri/src/ipc/commands/tts.rs
//
// 확인 요청 대사 TTS 커맨드. 본문에 `.await`가 있으나 락을 잡은 채 양보하지
// 않는다(`tts::speak`는 내부에서 잠깐씩만 잡는다) — 부모 모듈 머리말의
// no-lock-across-await 계약 유지.
//
// `tts_speak`는 설정 `tts_enabled`를 백엔드에서 최종 게이트한다. 렌더러도
// 게이트하지만(불필요한 왕복 제거), 외부 API 비용이 걸린 경로라 백엔드가
// 권위여야 한다. API 키는 어떤 커맨드도 렌더러로 돌려주지 않는다 —
// `tts_key_status`는 존재 여부 bool만 내려준다.

use tauri::State;

use crate::state::AppState;

#[tauri::command(rename_all = "camelCase")]
pub async fn tts_speak(
    app_state: State<'_, AppState>,
    request: crate::tts::SpeakRequest,
) -> Result<crate::tts::SpeakResult, String> {
    // 설정 스냅샷만 값으로 꺼내고 가드는 즉시 놓는다(await 전).
    let (enabled, model, provider) = {
        let s = app_state.settings.read().unwrap();
        (s.tts_enabled, s.tts_rewrite_model, s.tts_rewrite_provider)
    };
    if !enabled {
        return Err(crate::tts::TtsError::Disabled.to_ipc_string());
    }
    crate::tts::speak(&app_state.tts, model, provider, &request)
        .await
        .map_err(|e| {
            // 장식 기능이라 앱 동작에는 영향이 없지만, 원인은 남긴다(키 값은 미포함).
            eprintln!("tts: 발화 실패 ({e})");
            e.to_ipc_string()
        })
}

/// 프로필 다이얼로그의 보이스 드롭다운용 목록. 키 값은 나가지 않고
/// (voiceId, name, 라벨 요약)만 내려간다. `tts_enabled`로 게이트하지 **않는다**
/// — 목록 조회는 캐시된 GET 한 번이라 비용이 없고, 설정을 켜기 전에도 어떤
/// 목소리를 고를 수 있는지 보여주는 편이 낫다. 대신 키가 없으면 에러다.
#[tauri::command(rename_all = "camelCase")]
pub async fn tts_list_voices(
    app_state: State<'_, AppState>,
) -> Result<Vec<crate::tts::VoiceOption>, String> {
    crate::tts::list_voices(&app_state.tts)
        .await
        .map_err(|e| e.to_ipc_string())
}

/// 설정 UI가 표시하는 상태 — 키 값이 아니라 존재 여부 bool + env 유래 여부 +
/// claude CLI 가용성 + 현재 설정으로 실제 선택될 리라이트 경로.
#[tauri::command(rename_all = "camelCase")]
pub async fn tts_key_status(app_state: State<'_, AppState>) -> Result<crate::tts::TtsStatus, String> {
    Ok(tts_status(&app_state))
}

/// `tts_key_status` / `tts_set_keys`가 공유하는 상태 조립. 설정 가드를 잡은
/// 채 `.await`하지 않는다(동기 함수).
fn tts_status(app_state: &AppState) -> crate::tts::TtsStatus {
    let provider = app_state.settings.read().unwrap().tts_rewrite_provider;
    let cli = crate::tts::claude_cli_available();
    let route = crate::tts::resolve_rewrite_route(provider, app_state.tts.keys.anthropic_key(), cli);
    crate::tts::TtsStatus {
        keys: app_state.tts.keys.status(),
        claude_cli_available: cli,
        effective_rewrite_via: route.label(),
    }
}

/// 키 저장. `null`인 필드는 기존 값을 유지하고, 빈 문자열은 삭제다.
/// 반환값은 갱신된 마스킹 상태(렌더러가 즉시 UI를 맞출 수 있게).
#[tauri::command(rename_all = "camelCase")]
pub async fn tts_set_keys(
    app_state: State<'_, AppState>,
    elevenlabs: Option<String>,
    anthropic: Option<String>,
) -> Result<crate::tts::TtsStatus, String> {
    app_state
        .tts
        .keys
        .set(elevenlabs, anthropic)
        .map_err(|e| format!("cache: 키 저장 실패 ({e})"))?;
    Ok(tts_status(&app_state))
}
