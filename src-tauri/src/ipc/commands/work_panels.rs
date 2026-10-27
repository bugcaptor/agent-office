use crate::{
    state::AppState,
    types::{AutoRunJudgeRequest, AutoRunJudgeResult, WorkFollowUpRequest, WorkSendRequest, WorkSendResult, WorkSendTarget},
};
use tauri::State;

fn validate_target(app_state: &AppState, agent_id: &str) -> Result<(), String> {
    let profiles = app_state.store.load();
    let profile = profiles
        .agents
        .iter()
        .find(|p| p.id == agent_id)
        .ok_or("work-send-agent-missing")?;
    if profile.clocked_out == Some(true) {
        return Err("work-send-clocked-out".into());
    }
    if profile
        .startup_command
        .as_ref()
        .is_some_and(|c| !c.trim().is_empty())
    {
        return Err("work-send-profile-unsupported".into());
    }
    Ok(())
}

/// 업무 기본 에이전트의 모델 칸에 띄울 추천 목록. 그 CLI가 현재 로그인한
/// 계정에서 읽어 온다. 실행 허용 목록이 아니다 — 목록에 없는 사용자 별칭도
/// 그대로 실행에 넘긴다.
#[tauri::command(rename_all = "camelCase")]
pub async fn work_agent_models(cli: String) -> Result<Vec<String>, String> {
    Ok(crate::summarizer::list_cli_models(&cli).await)
}
#[tauri::command(rename_all = "camelCase")]
pub async fn inspect_work_send_target(
    agent_id: String,
    app_state: State<'_, AppState>,
) -> Result<WorkSendTarget, String> {
    validate_target(&app_state, &agent_id)?;
    app_state
        .work_send_runtime
        .inspect(&app_state.gate, &agent_id)
}
#[tauri::command(rename_all = "camelCase")]
pub async fn send_work_item(
    request: WorkSendRequest,
    app_state: State<'_, AppState>,
) -> Result<WorkSendResult, String> {
    validate_target(&app_state, &request.agent_id)?;
    app_state
        .work_send_runtime
        .send(
            &app_state.gate,
            app_state.app_data_dir.clone(),
            request,
        )
        .await
}

/// 자동 진행의 후속 지시(docs/auto-run-design.md). 실행 중인 CLI에 프롬프트 하나를 넣는다.
#[tauri::command(rename_all = "camelCase")]
pub async fn send_work_follow_up(
    request: WorkFollowUpRequest,
    app_state: State<'_, AppState>,
) -> Result<WorkSendResult, String> {
    validate_target(&app_state, &request.agent_id)?;
    app_state.work_send_runtime.follow_up(&app_state.gate, request).await
}
/// 그 캐릭터 세션의 마지막 에이전트 응답. Claude는 훅이 알려 준 전사, Codex는 cwd의 최신 rollout.
#[tauri::command(rename_all = "camelCase")]
pub async fn work_last_reply(
    agent_id: String,
    cli: String,
    cwd: String,
    app_state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let lookup: std::sync::Arc<dyn crate::session_log::agent_transcript::AgentSessionLookup> =
        app_state.claude_resume_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        use crate::session_log::agent_transcript::{claude, codex};
        let mut source = match cli.as_str() {
            "claude" => claude::source(lookup),
            "codex" => codex::source(),
            _ => None,
        }?;
        crate::auto_run::last_reply(source.as_mut(), &agent_id, &cwd)
    })
    .await
    .map_err(|e| e.to_string())
}
/// 작업자가 `office-task --report`로 남긴 그 태스크의 보고. `since`(epoch ms) 전에 쓰인 것은 없는 것으로 친다.
#[tauri::command(rename_all = "camelCase")]
pub fn auto_run_report(task_id: String, since: u64, app_state: State<'_, AppState>) -> Option<String> {
    crate::auto_run::read_report(&app_state.app_data_dir.join("work-inbox"), &task_id, since, std::time::SystemTime::now())
}
/// 자동 진행 판단자 호출 1회. 실행 파일은 요약 설정의 Claude 명령을 따른다.
#[tauri::command(rename_all = "camelCase")]
pub async fn auto_run_judge(
    request: AutoRunJudgeRequest,
    app_state: State<'_, AppState>,
) -> Result<AutoRunJudgeResult, String> {
    let program = {
        let guard = app_state.settings.read().unwrap();
        let picked = guard
            .summary_models
            .for_provider(crate::persistence::settings_store::SummaryProvider::Claude)
            .command
            .trim()
            .to_string();
        if picked.is_empty() { "claude".to_string() } else { picked }
    };
    let dir = crate::auto_run::judge_dir(&app_state.app_data_dir);
    crate::auto_run::judge(&program, &dir, request).await
}

#[tauri::command]
pub fn scan_work_inbox(app_state: State<'_, AppState>) -> Result<Vec<crate::persistence::work_inbox::InboxEntry>, String> {
    crate::persistence::work_inbox::scan(&app_state.app_data_dir.join("work-inbox"), std::time::SystemTime::now())
}
#[tauri::command(rename_all = "camelCase")]
pub fn ack_work_inbox(file_names: Vec<String>, app_state: State<'_, AppState>) -> Result<(), String> {
    crate::persistence::work_inbox::ack(&app_state.app_data_dir.join("work-inbox"), &file_names)
}
#[tauri::command]
pub fn reject_work_inbox(rejections: Vec<crate::persistence::work_inbox::Rejection>, app_state: State<'_, AppState>) -> Result<(), String> {
    crate::persistence::work_inbox::reject_files(&app_state.app_data_dir.join("work-inbox"), &rejections)
}
#[tauri::command]
pub fn warn_work_inbox(warnings: Vec<crate::persistence::work_inbox::Warning>, app_state: State<'_, AppState>) -> Result<(), String> {
    crate::persistence::work_inbox::append_warnings(&app_state.app_data_dir.join("work-inbox"), &warnings, chrono::Utc::now())
}

/// 업무 목록 내보내기. 직렬화는 렌더러가 하고, 여기서는 고른 위치에 쓰기만 한다.
/// `filter_name`은 다이얼로그 형식 이름(현재 언어). 저장한 경로, 취소 시 None.
#[tauri::command(rename_all = "camelCase")]
pub async fn export_work_items_file(
    app: tauri::AppHandle,
    default_name: String,
    filter_name: String,
    content: String,
) -> Result<Option<String>, String> {
    super::save_text_with_dialog(&app, default_name, &filter_name, content).await
}
/// 업무 목록 가져오기. 고른 파일의 텍스트만 돌려주고 파싱·검증은 렌더러가 한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn import_work_items_file(
    app: tauri::AppHandle,
    filter_name: String,
) -> Result<Option<String>, String> {
    super::open_text_with_dialog(&app, &filter_name).await
}

/// 태스크 대화 열기(docs/task-chat-design.md §기동). `task-chat-<taskId>` 세션을 만들고 맥락 묶음을
/// 첫 프롬프트로 넣는다. 캐릭터 프로필 검사(`validate_target`)는 하지 않는다 — 대화 세션은 캐릭터가 없다.
#[tauri::command(rename_all = "camelCase")]
pub async fn open_task_chat(
    request: crate::types::TaskChatRequest,
    app_state: State<'_, AppState>,
) -> Result<crate::types::TaskChatResult, String> {
    crate::task_chat::open(&app_state, request).await
}
