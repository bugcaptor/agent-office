//! 일반 에이전트: 캐릭터 없는 사무실 세션 한 칸(`office-agent`). 웹 상태판에서 지시를 받아
//! 앱 안 곁 탭으로 띄운다. 캐릭터 세션처럼 관찰 훅·`office-task`·`office-md` 안내문을 다 싣는다
//! (persona·`AGENT_OFFICE_AGENT_ID`만 없다) — 그래서 이 세션은 업무·토의를 만들며 앱 자체를 다룰 수 있다.
//!
//! 여기는 새 세션을 여는 일만 한다. 입력을 기다리는 CLI에 다음 지시를 잇는 일은 웹 라우트가
//! `WorkSendRuntime::follow_up`으로 한다.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::types::{OfficeAgentRequest, TaskChatResult, WorkSendRequest, OFFICE_AGENT_ID};

/// 지시 한 번의 바이트 상한(웹 입력 상한과 같다).
pub const MAX_PROMPT_BYTES: usize = 32 * 1024;

static OPENING: AtomicBool = AtomicBool::new(false);
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// 마지막 열기가 실패했으면 그 이유 코드. 웹은 열기를 기다리지 않으므로 상태로 본다.
pub fn last_error() -> Option<String> {
    LAST_ERROR.lock().unwrap().clone()
}

pub fn clear_last_error() {
    *LAST_ERROR.lock().unwrap() = None;
}

/// 열고 있는 중인가.
pub fn is_opening() -> bool {
    OPENING.load(Ordering::SeqCst)
}

struct OpeningGuard;
impl OpeningGuard {
    fn acquire() -> Result<Self, String> {
        OPENING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| Self)
            .map_err(|_| "office-agent-busy".to_string())
    }
}
impl Drop for OpeningGuard {
    fn drop(&mut self) {
        OPENING.store(false, Ordering::SeqCst);
    }
}

/// 지시 글 검사. 웹 라우트와 열기가 같이 쓴다.
pub fn check_prompt(prompt: &str) -> Result<(), String> {
    if prompt.trim().is_empty() {
        return Err("empty".into());
    }
    if prompt.len() > MAX_PROMPT_BYTES {
        return Err("too-large".into());
    }
    if prompt.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err("control-char".into());
    }
    Ok(())
}

pub fn validate(request: &OfficeAgentRequest) -> Result<(), String> {
    check_prompt(&request.prompt)?;
    if let Some(cwd) = request.cwd.as_deref() {
        let path = Path::new(cwd);
        if !path.is_absolute() || !path.is_dir() {
            return Err("office-agent-invalid-cwd".into());
        }
    }
    crate::work_send::validated_command(&request.agent, crate::work_send::cli_transition::CommandSyntax::Posix)?;
    Ok(())
}

/// 새 세션을 열고 지시를 첫 프롬프트로 넣는다. `restart`면 살아 있는 세션을 버리고 연다.
pub async fn open(
    app_state: &crate::state::AppState,
    request: OfficeAgentRequest,
    restart: bool,
) -> Result<TaskChatResult, String> {
    let result = open_inner(app_state, request, restart).await;
    *LAST_ERROR.lock().unwrap() = result.as_ref().err().cloned();
    result
}

async fn open_inner(
    app_state: &crate::state::AppState,
    request: OfficeAgentRequest,
    restart: bool,
) -> Result<TaskChatResult, String> {
    validate(&request)?;
    let _guard = OpeningGuard::acquire()?;
    let manager = &app_state.manager;
    if manager.is_running(OFFICE_AGENT_ID) {
        if !restart {
            return Err("office-agent-running".into());
        }
        app_state.work_send_runtime.stop(OFFICE_AGENT_ID);
        manager.dispose_matching(OFFICE_AGENT_ID, None)?;
    }
    let created = crate::ipc::commands::spawn_session(
        manager,
        &app_state.observer,
        &app_state.observer_server,
        &app_state.settings,
        crate::types::CreateSessionRequest {
            agent_id: OFFICE_AGENT_ID.to_string(),
            cols: request.cols,
            rows: request.rows,
            cwd: request.cwd.clone(),
            shell: None,
            startup_command: None,
            personality_prompt: None,
            autostart_claude: None,
            task_chat: false,
        },
        crate::session_events::types::AgentEventProfile { name: "Agent Office".into(), role: None },
    )
    .await?;
    let session_id = created.session_id;
    if created.state != crate::types::SessionState::Running {
        let _ = manager.dispose_matching(OFFICE_AGENT_ID, Some(&session_id));
        return Err("work-send-create-failed".into());
    }
    let sent = app_state
        .work_send_runtime
        .send(
            &app_state.gate,
            app_state.app_data_dir.clone(),
            WorkSendRequest {
                agent_id: OFFICE_AGENT_ID.to_string(),
                session_id: session_id.clone(),
                human_input_revision: app_state.gate.human_input_revision(OFFICE_AGENT_ID),
                fresh_session: true,
                agent: request.agent,
                title: String::new(),
                body: request.prompt,
            },
        )
        .await;
    if let Err(error) = sent {
        let _ = manager.dispose_matching(OFFICE_AGENT_ID, Some(&session_id));
        return Err(error);
    }
    Ok(TaskChatResult { agent_id: OFFICE_AGENT_ID.to_string(), session_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{WorkAgent, WorkCli};

    fn request(prompt: &str, cwd: Option<&str>) -> OfficeAgentRequest {
        OfficeAgentRequest {
            prompt: prompt.into(),
            cwd: cwd.map(str::to_string),
            agent: WorkAgent { cli: WorkCli::Claude, model: None, effort: None },
            cols: None,
            rows: None,
        }
    }

    #[test]
    fn validate_rejects_empty_control_and_bad_folders() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_str().unwrap();
        assert!(validate(&request("do it", Some(dir))).is_ok());
        assert!(validate(&request("do it", None)).is_ok());
        assert_eq!(validate(&request("  \n", None)).unwrap_err(), "empty");
        assert_eq!(validate(&request("a\u{1b}b", None)).unwrap_err(), "control-char");
        assert_eq!(validate(&request("x", Some("relative/path"))).unwrap_err(), "office-agent-invalid-cwd");
        assert_eq!(validate(&request("x", Some("/definitely/not/here/ao"))).unwrap_err(), "office-agent-invalid-cwd");
        assert_eq!(validate(&request(&"x".repeat(MAX_PROMPT_BYTES + 1), None)).unwrap_err(), "too-large");
    }

    #[test]
    fn office_agent_is_a_side_session_but_not_a_chat_or_discussion() {
        assert!(crate::types::is_side_session_id(OFFICE_AGENT_ID));
        assert!(!crate::types::is_task_chat_id(OFFICE_AGENT_ID));
        assert!(!crate::types::is_discussion_session_id(OFFICE_AGENT_ID));
        assert!(!crate::types::is_office_agent_id("office-agent-2"));
    }

    #[test]
    fn request_reads_camel_case_and_rejects_unknown_fields() {
        let r: OfficeAgentRequest =
            serde_json::from_str(r#"{"prompt":"hi","cwd":"/w","agent":{"cli":"codex"},"cols":80}"#).unwrap();
        assert_eq!(r.cwd.as_deref(), Some("/w"));
        assert_eq!(r.cols, Some(80));
        assert!(serde_json::from_str::<OfficeAgentRequest>(r#"{"prompt":"hi","agent":{"cli":"claude"},"x":1}"#).is_err());
    }
}
