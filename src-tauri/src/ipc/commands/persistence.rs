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

/// 포스트잇 메모(#79) 현재 장을 읽는다. 없으면 새 빈 장을 만들어 돌려준다 —
/// 렌더러는 "장이 없는 상태"를 다루지 않는다(항상 쓸 수 있는 한 장이 있다).
#[tauri::command(rename_all = "camelCase")]
pub async fn load_memo(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<crate::types::MemoSheet, String> {
    app_state
        .memo_store
        .load_current(&agent_id)
        .map_err(|e| e.to_string())
}

/// 지목한 장의 본문을 교체하고 `updated`를 갱신한다(디바운스 자동저장의 착지점).
/// `created`와 이미 붙은 `archived` 스탬프는 보존된다.
#[tauri::command(rename_all = "camelCase")]
pub async fn save_memo(
    app_state: State<'_, AppState>,
    agent_id: String,
    sheet_id: String,
    content: String,
) -> Result<(), String> {
    app_state
        .memo_store
        .save(&agent_id, &sheet_id, &content)
        .map_err(|e| e.to_string())
}

/// 현재 장을 통째로 아카이브(삭제 아님 — frontmatter에 `archived` 스탬프 추가)하고,
/// 즉시 만들어진 새 빈 장을 돌려준다.
#[tauri::command(rename_all = "camelCase")]
pub async fn archive_memo_sheet(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<crate::types::MemoSheet, String> {
    app_state
        .memo_store
        .archive_current(&agent_id)
        .map_err(|e| e.to_string())
}

/// 아카이브된 장들의 메타 목록(최신순, 본문 제외).
#[tauri::command(rename_all = "camelCase")]
pub async fn list_memo_archive(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<Vec<crate::types::MemoSheetMeta>, String> {
    app_state
        .memo_store
        .list_archive(&agent_id)
        .map_err(|e| e.to_string())
}

/// 특정 장 전체(본문 포함) — 아카이브 열람용.
#[tauri::command(rename_all = "camelCase")]
pub async fn read_memo_sheet(
    app_state: State<'_, AppState>,
    agent_id: String,
    sheet_id: String,
) -> Result<crate::types::MemoSheet, String> {
    app_state
        .memo_store
        .read_sheet(&agent_id, &sheet_id)
        .map_err(|e| e.to_string())
}

/// 캐릭터 삭제 시 그 캐릭터의 메모 폴더를 통째로 정리한다. 초상/스프라이트
/// 삭제와 같은 브리지(렌더러 `agents` 구독)에서 호출된다. 부재는 무해 통과.
#[tauri::command(rename_all = "camelCase")]
pub async fn delete_memos(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<(), String> {
    app_state
        .memo_store
        .delete_agent(&agent_id)
        .map_err(|e| e.to_string())
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

/// 한 캐릭터의 세션 로그 파일 목록 한 페이지(최신순).
/// docs/session-log-design.md §6. 어떤 실패도 빈 페이지로 흡수한다 — 열람
/// 경로가 에러로 막히지 않게.
#[tauri::command(rename_all = "camelCase")]
pub async fn list_session_logs(
    app_state: State<'_, AppState>,
    agent_id: String,
    offset: usize,
    limit: usize,
) -> Result<crate::types::SessionLogPage, String> {
    // 상한을 둬 렌더러 실수로 수천 개를 한 번에 읽지 않게 한다.
    let limit = limit.clamp(1, 100);
    let (total, items) =
        crate::session_log::store::list_logs(&app_state.session_log_root, &agent_id, offset, limit);
    Ok(crate::types::SessionLogPage { total, items })
}

/// 세션 로그 파일을 외부 에디터로 연다(설정 `externalEditor` 준수).
/// `path`는 반드시 세션 로그 루트 아래여야 한다 — 렌더러가 준 경로를 그대로
/// 믿지 않는다(경로 탈출 차단).
#[tauri::command(rename_all = "camelCase")]
pub async fn open_session_log(
    app_state: State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let file = std::path::PathBuf::from(&path);
    if !crate::session_log::store::is_inside_root(&app_state.session_log_root, &file) {
        return Err("세션 로그 폴더 밖의 경로입니다".to_string());
    }
    let use_vscode = {
        let guard = app_state.settings.read().unwrap();
        matches!(
            guard.external_editor,
            crate::persistence::settings_store::ExternalEditor::Vscode
        )
    };
    crate::shell_export::open_file_in_editor(&file, use_vscode)
}

/// 세션 로그 하나로 회고·학습자료(Markdown)를 만든다(수동 트리거 전용).
/// 요약기 옵트인이 꺼져 있으면 CLI를 부르지 않고 거절한다(일기와 동일 정책).
/// docs/session-log-design.md §5.
#[tauri::command(rename_all = "camelCase")]
pub async fn generate_study_material(
    app_state: State<'_, AppState>,
    agent_id: String,
    path: String,
) -> Result<crate::types::StudyMaterialResult, String> {
    let file = std::path::PathBuf::from(&path);
    if !crate::session_log::store::is_inside_root(&app_state.session_log_root, &file) {
        return Err("세션 로그 폴더 밖의 경로입니다".to_string());
    }
    // 설정 가드는 await 이전에 떨어뜨린다(no-lock-across-await 계약).
    let (provider, models) = {
        let guard = app_state.settings.read().unwrap();
        if !guard.summarizer_enabled {
            return Err("summarizer-disabled".to_string());
        }
        (guard.summary_provider, guard.summary_models.clone())
    };
    // OpenRouter 경로만 API 키를 쓴다(요약기 HTTP 갈래).
    let openrouter_key = if provider
        == crate::persistence::settings_store::SummaryProvider::Openrouter
    {
        app_state.tts.keys.openrouter_key()
    } else {
        None
    };
    let result = crate::session_log::study::generate(
        &app_state.session_log_root,
        &agent_id,
        &file,
        provider,
        &models,
        openrouter_key.as_deref(),
    )
    .await?;
    Ok(crate::types::StudyMaterialResult {
        path: result.path.to_string_lossy().into_owned(),
        dir: result.dir.to_string_lossy().into_owned(),
        file_name: result.file_name,
    })
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
