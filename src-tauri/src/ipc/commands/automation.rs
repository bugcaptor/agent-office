// src-tauri/src/ipc/commands/automation.rs
//
// 자동화 점검(kbm) start/stop/status 커맨드 -- 얇은 위임(`bot.rs` 관례를
// 그대로 본떴다). 실제 로직은 `AppState::automation_runtime`에 있다.

use tauri::State;

use crate::session::inject::InputSink;
use crate::state::AppState;
use crate::types::*;

/// 편집기의 읽기 전용 CLI 전환 미리보기. 실제 탭의 셸만 조회하며 파일을
/// 만들거나 PTY에 입력하지 않는다. receipt wrapper는 사용자에게 노출하지 않는다.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationCliTransitionPreview {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell_path: Option<String>,
    pub auto_return_supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch_command: Option<String>,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn automation_cli_transition_preview(
    agent_id: String,
    cli: String,
    model: Option<String>,
    effort: Option<String>,
    app_state: State<'_, AppState>,
) -> Result<AutomationCliTransitionPreview, String> {
    let shell = app_state.automation_ctx.sink.shell_path_for(&agent_id);
    let capability = crate::automation::cli_transition::CliTransition::capability_for_shell_path(
        shell.as_deref(),
    );
    let launch_command = if capability.auto_return_supported {
        Some(crate::automation::definition_runner::launch_command(
            &cli,
            model.as_deref(),
            effort.as_deref(),
        )?)
    } else {
        None
    };
    Ok(AutomationCliTransitionPreview {
        shell_path: capability.shell_path,
        auto_return_supported: capability.auto_return_supported,
        unavailable_reason: capability.unavailable_reason,
        launch_command,
    })
}

/// 이 탭의 자동화를 시작한다. 세션 stdin에 LLM CLI를 주입하고 즉시 초기 상태를
/// 반환한다 — 사전 확인(세션 실행 여부·cwd)과 이후 흐름은 태스크가 비동기로
/// 수행하므로 실패는 이후 `automation_status`의 `error`로 드러난다. `cli`
/// 생략 시 기본 `claude`.
#[tauri::command(rename_all = "camelCase")]
pub async fn automation_start(
    agent_id: String,
    cli: Option<AutomationCli>,
    app_state: State<'_, AppState>,
) -> Result<AutomationAgentStatus, String> {
    Ok(app_state.automation_runtime.start(
        app_state.automation_ctx.clone(),
        agent_id,
        cli.unwrap_or_default(),
    ))
}

/// 이 탭의 자동화를 중단한다 — 진행 중인 태스크를 내리고 등록을 지운다.
/// 렌더러가 완료/실패를 확인한 뒤에도 이걸로 스냅샷을 정리한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn automation_stop(
    agent_id: String,
    app_state: State<'_, AppState>,
) -> Result<(), String> {
    app_state.automation_runtime.stop(&agent_id);
    Ok(())
}

/// 자동화가 돌고 있(었)는 탭들의 상태 스냅샷. 렌더러가 폴링해 배지/배너와
/// 완료·실패 알림을 띄운다.
#[tauri::command(rename_all = "camelCase")]
pub async fn automation_status(app_state: State<'_, AppState>) -> Result<AutomationStatus, String> {
    Ok(app_state.automation_runtime.status())
}

/// 타임아웃 결정(연장 / 중단)을 원자적으로 적용한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn automation_decide(
    agent_id: String,
    run_id: String,
    step_execution_id: String,
    decision_id: String,
    choice: AutomationDecisionChoice,
    app_state: State<'_, AppState>,
) -> Result<bool, String> {
    app_state.automation_runtime.decide(
        &agent_id,
        &run_id,
        &step_execution_id,
        &decision_id,
        choice,
    )
}

/// 사람 입력 잔여 상태를 해제하고 보류 중인 자동화가 즉시 재시도할 수 있게 한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn automation_clear_uncommitted(
    agent_id: String,
    app_state: State<'_, AppState>,
) -> Result<(), String> {
    app_state.gate.clear_uncommitted(&agent_id);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn automation_definitions_list(
    app_state: State<'_, AppState>,
) -> Result<Vec<AutomationDefinition>, String> {
    app_state.automation_store.list()
}

#[tauri::command(rename_all = "camelCase")]
pub async fn automation_definitions_save(
    definition: AutomationDefinition,
    app_state: State<'_, AppState>,
) -> Result<AutomationDefinition, String> {
    app_state.automation_store.save(&definition)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn automation_definitions_delete(
    id: String,
    app_state: State<'_, AppState>,
) -> Result<bool, String> {
    app_state.automation_store.delete(&id)
}

/// 가져오기는 저장만 한다. 입력 자동화는 `automation_run_start`가 명시적으로 호출될 때만 시작된다.
#[tauri::command(rename_all = "camelCase")]
pub async fn automation_definitions_import(
    json: String,
    app_state: State<'_, AppState>,
) -> Result<AutomationDefinition, String> {
    app_state.automation_store.import(&json)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn automation_definitions_export(
    id: String,
    app_state: State<'_, AppState>,
) -> Result<String, String> {
    app_state.automation_store.export(&id)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn automation_runs_list(
    app_state: State<'_, AppState>,
) -> Result<Vec<AutomationRunRecord>, String> {
    app_state.automation_store.runs()
}

/// 새 실행 API. Phase 1 점검 start와 분리해 정의를 먼저 읽고, 잘못된 workspace나
/// 세션이면 어떤 입력도 보내지 않는다. 단계 러너가 status에 snapshot을 계속 갱신한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn automation_run_start(
    agent_id: String,
    definition_id: String,
    inputs: std::collections::BTreeMap<String, String>,
    app_state: State<'_, AppState>,
) -> Result<AutomationAgentStatus, String> {
    let definition = app_state.automation_store.get(&definition_id)?;
    crate::automation::definition::validate(&definition)?;
    // `agy models`는 프로세스를 기다린다. 그 사이 재시작되면 이후 runtime이
    // 새 PTY를 잡아 옛 정의를 시작할 수 있으므로, 현재 세션을 먼저 고정하고
    // 조회 뒤에도 같은 실행 세션인지 확인한다.
    let session_id = app_state
        .automation_ctx
        .sink
        .session_id_for(&agent_id)
        .ok_or("automation-session-changed")?;
    let workspace = app_state
        .automation_ctx
        .sink
        .cwd_of(&agent_id)
        .filter(|cwd| !cwd.is_empty())
        .ok_or("automation-cwd-unknown")?;
    validate_agy_models(&definition).await?;
    if !same_running_session(
        &session_id,
        app_state
            .automation_ctx
            .sink
            .session_id_for(&agent_id)
            .as_deref(),
        app_state.automation_ctx.sink.is_running(&agent_id),
    ) {
        return Err("automation-session-changed".into());
    }
    app_state.automation_runtime.start_definition_for_session(
        app_state.automation_ctx.clone(),
        agent_id,
        definition,
        inputs,
        workspace,
        app_state.automation_store.clone(),
        &session_id,
    )
}

#[tauri::command]
pub async fn automation_agy_models() -> Result<Vec<String>, String> {
    Ok(crate::summarizer::list_automation_agy_models().await)
}

/// 자동화 CLI가 현재 로그인한 계정에서 읽어 온 모델 추천 목록. 이 목록은
/// 편집기 선택 UI 전용이며 저장된 Claude/Codex 사용자 별칭의 실행 가능 여부를
/// 제한하지 않는다. 기존 agy 전용 명령은 호환성을 위해 유지한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn automation_cli_models(cli_profile_id: String) -> Result<Vec<String>, String> {
    Ok(crate::summarizer::list_automation_cli_models(&cli_profile_id).await)
}

fn same_running_session(expected: &str, current: Option<&str>, running: bool) -> bool {
    running && current == Some(expected)
}

/// agy의 모델 id는 릴리스마다 달라지고 `--effort`와 독립이다. 저장된 정의도
/// 그대로 실행하기 전에 현재 CLI의 `agy models` 목록과 정확히 대조한다.
/// 목록을 읽지 못하면 추측해 실행하지 않는다. 빈 목록을 허용하면 오래된 id가
/// 다시 CLI 실패로만 드러나기 때문이다.
async fn validate_agy_models(definition: &AutomationDefinition) -> Result<(), String> {
    let requested: Vec<&str> = definition
        .steps
        .iter()
        .filter_map(|step| match step {
            AutomationStep::LaunchCli {
                cli_profile_id,
                model: Some(model),
                ..
            } if cli_profile_id == "agy" => Some(model.as_str()),
            _ => None,
        })
        .collect();
    if requested.is_empty() {
        return Ok(());
    }
    let available = crate::summarizer::list_automation_agy_models().await;
    validate_agy_models_available(&requested, &available)
}

fn validate_agy_models_available(requested: &[&str], available: &[String]) -> Result<(), String> {
    if available.is_empty() {
        return Err("automation-agy-model-catalog-unavailable".into());
    }
    for model in requested {
        if !available
            .iter()
            .any(|available_model| available_model == model)
        {
            return Err(format!("automation-agy-model-unavailable:{model}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agy_model_validation_requires_an_exact_live_id() {
        let models = vec!["gemini-3.8-flash-medium".to_string()];
        assert!(validate_agy_models_available(&["gemini-3.8-flash-medium"], &models).is_ok());
        assert_eq!(
            validate_agy_models_available(&["gemini-3.8-flash"], &models),
            Err("automation-agy-model-unavailable:gemini-3.8-flash".into())
        );
        assert_eq!(
            validate_agy_models_available(&["gemini-3.8-flash-medium"], &[]),
            Err("automation-agy-model-catalog-unavailable".into())
        );
    }

    #[test]
    fn preflight_rejects_a_restarted_or_stopped_session() {
        assert!(same_running_session("old", Some("old"), true));
        assert!(!same_running_session("old", Some("new"), true));
        assert!(!same_running_session("old", Some("old"), false));
        assert!(!same_running_session("old", None, true));
    }
}
