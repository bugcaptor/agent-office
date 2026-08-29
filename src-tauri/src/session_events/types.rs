use serde::{Deserialize, Serialize};

use crate::types::{AgentId, SessionId, SessionState};

pub use crate::types::SessionEventTokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    SessionStarted,
    SessionState,
    Prompt,
    Tool,
    Notification,
    Bell,
    Stop,
    /// 한 턴의 토큰 사용량 전용 레코드("usage"). 알림(Stop)과 분리된 채널
    /// `AppEvents::turn_usage`가 기록한다 — 과거 파일은 이 kind가 없고
    /// 대신 kind=Stop 레코드의 `tokens`에 실려 있었다(session-analytics-design §9.1).
    Usage,
}

/// 프롬프트를 누가 넣었는지. 사람이 직접 친 프롬프트는 표식이 없다(필드 자체가
/// 생략된다) — 봇 주입만 명시적으로 남긴다.
///
/// 봇은 별도 세션을 띄우지 않는다. `bot/runner.rs::inject`가 이미 떠 있는
/// 터미널에 `write_input`으로 프롬프트를 밀어넣으므로 세션도 agentId도 사람이
/// 쓸 때와 같다. 구분선은 **턴 단위**에만 있어서, 턴을 여는 prompt 이벤트에
/// 출처를 실어 둔다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptOrigin {
    Bot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEventProfile {
    pub name: String,
    pub role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStartedEvent {
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub agent_name: String,
    pub agent_role: Option<String>,
    pub cwd: String,
    pub shell: String,
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEventDraft {
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub kind: SessionEventKind,
    pub at: u64,
    pub agent_name: Option<String>,
    pub agent_role: Option<String>,
    pub cwd: Option<String>,
    pub shell: Option<String>,
    pub state: Option<SessionState>,
    /// kind=Usage일 때 그 턴이 쓴 토큰(추출 성공 시에만). 과거 파일은 이 값이
    /// kind=Stop 드래프트에 실렸다 — 소비자는 kind가 아니라 tokens의 유무로
    /// 합산해야 신구 파일을 모두 커버한다.
    pub tokens: Option<SessionEventTokens>,
    /// kind=Prompt일 때 그 프롬프트의 출처(봇 주입만 표식, 사람은 None).
    pub origin: Option<PromptOrigin>,
}

impl SessionEventDraft {
    pub fn simple(
        agent_id: impl Into<String>,
        session_id: impl Into<String>,
        kind: SessionEventKind,
        at: u64,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            session_id: session_id.into(),
            kind,
            at,
            agent_name: None,
            agent_role: None,
            cwd: None,
            shell: None,
            state: None,
            tokens: None,
            origin: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEventRecord {
    pub schema_version: u8,
    pub run_id: String,
    pub seq: u64,
    pub at: u64,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub kind: SessionEventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<SessionState>,
    /// 한 턴이 쓴 토큰. 과거 파일은 kind=Stop 레코드에 실렸고, 신규 파일은
    /// kind=Usage 레코드에 실린다(kind=Stop엔 더 이상 안 실린다) — 소비자는
    /// **kind가 아니라 tokens 유무**로 합산해야 신구 파일이 모두 커버된다.
    /// 옵션 추가라 schemaVersion은 1을 유지한다 — 토큰이 없는 과거 파일과
    /// 섞여도 그대로 읽힌다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<SessionEventTokens>,
    /// kind=Prompt일 때 그 프롬프트의 출처. `tokens`와 같은 선례로 옵션 추가라
    /// schemaVersion은 1을 유지한다 — 출처가 없는 과거 파일과 섞여도 그대로
    /// 읽히고, 그 프롬프트는 전부 사람 몫으로 집계된다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<PromptOrigin>,
}
