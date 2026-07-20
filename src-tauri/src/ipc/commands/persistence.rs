// src-tauri/src/ipc/commands/persistence.rs
//
// State/turns/diary/work-log/session-event/resume-list persistence commands.
// Thin delegations into the various `*Store`s on `AppState` -- see
// `super`(`ipc::commands`) module doc for the shared no-lock-across-await
// contract.

use tauri::State;

use crate::state::AppState;
use crate::types::*;

#[tauri::command(rename_all = "camelCase")]
pub async fn load_state(app_state: State<'_, AppState>) -> Result<PersistedState, String> {
    Ok(app_state.store.load())
}

/// 주의: Tauri `State` 파라미터는 `app_state`, JS 페이로드 `{ state }`는
/// `state` 파라미터로 받는다 (이름 충돌 회피 -- JS 인자 키와 Rust 파라미터명이
/// 일치해야 매핑된다).
#[tauri::command(rename_all = "camelCase")]
pub async fn save_state(
    app_state: State<'_, AppState>,
    state: PersistedState,
) -> Result<(), String> {
    app_state.store.save(&state).map_err(|e| e.to_string())
}

/// 완료된 턴 1건을 로컬 시계열 로그(session-times.jsonl)에 append.
#[tauri::command(rename_all = "camelCase")]
pub async fn append_session_turn(
    app_state: State<'_, AppState>,
    record: crate::types::SessionTurnRecord,
) -> Result<(), String> {
    app_state
        .session_time_store
        .append(&record)
        .map_err(|e| e.to_string())
}

/// 누적된 세션 턴 기록 전체를 읽는다(통계용).
#[tauri::command(rename_all = "camelCase")]
pub async fn load_session_turns(
    app_state: State<'_, AppState>,
) -> Result<Vec<crate::types::SessionTurnRecord>, String> {
    Ok(app_state.session_time_store.load())
}

/// 캐릭터 일기(#56) 한 편을 per-agent 로그(`diaries/<agentId>.jsonl`)에 append.
/// 본문 생성은 렌더러가 `summarize_text`로 이미 마친 상태 — 여기선 저장만 한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn append_diary_entry(
    app_state: State<'_, AppState>,
    agent_id: String,
    entry: crate::types::DiaryEntry,
) -> Result<(), String> {
    app_state
        .diary_store
        .append(&agent_id, &entry)
        .map_err(|e| e.to_string())
}

/// 한 캐릭터의 일기 전체(작성순)를 읽는다(열람 오버레이용).
#[tauri::command(rename_all = "camelCase")]
pub async fn load_diary(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<Vec<crate::types::DiaryEntry>, String> {
    app_state
        .diary_store
        .load(&agent_id)
        .map_err(|e| e.to_string())
}

/// 한 캐릭터의 작업 로그 버퍼 전체를 스냅샷 저장한다(#60). 렌더러가 버퍼 변경 시
/// 디바운스로 호출한다. `items`가 비면 스냅샷 파일을 삭제한다(일기화로 소진된 캐릭터).
#[tauri::command(rename_all = "camelCase")]
pub async fn save_work_log(
    app_state: State<'_, AppState>,
    agent_id: String,
    items: Vec<crate::types::WorkLogItem>,
) -> Result<(), String> {
    app_state
        .work_log_store
        .save(&agent_id, &items)
        .map_err(|e| e.to_string())
}

/// 전 캐릭터의 작업 로그 스냅샷을 읽는다(부팅 복원용). `agentId -> items` 맵.
/// 손상/부재는 조용히 건너뛰므로 항상 성공한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn load_work_logs(
    app_state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, Vec<crate::types::WorkLogItem>>, String> {
    Ok(app_state.work_log_store.load_all())
}

/// 세션 이벤트 시계열에서 `from_at..=to_at`(epoch ms) 범위를 읽는다(분석 패널용).
/// 읽기 전용 — 수집 측 `SessionEventStore`는 건드리지 않는다
/// (docs/session-analytics-design.md §4.2). reader가 없는 파일·손상 줄을
/// 건너뛰므로 반환은 항상 성공한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn load_session_events(
    app_state: State<'_, AppState>,
    from_at: u64,
    to_at: u64,
) -> Result<Vec<crate::session_events::types::SessionEventRecord>, String> {
    Ok(crate::session_events::reader::load_session_events(
        &app_state.session_event_root,
        from_at,
        to_at,
    ))
}

/// 에이전트별 최신 Claude native 세션(리줌) 스냅샷 전체를 읽는다(이어하기
/// 메뉴용). 렌더러는 메뉴를 열 때만 조회하므로 이벤트 푸시가 필요 없다
/// (docs/claude-session-resume-design.md §5). 반환은 항상 성공.
#[tauri::command(rename_all = "camelCase")]
pub async fn list_claude_resume_sessions(
    app_state: State<'_, AppState>,
) -> Result<
    std::collections::HashMap<
        String,
        crate::persistence::claude_resume_store::ClaudeResumeEntry,
    >,
    String,
> {
    Ok(app_state.claude_resume_store.load_all())
}
