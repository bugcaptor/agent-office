use serde::{Deserialize, Serialize};

use crate::types::{AgentId, SessionId, SessionState};

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
}
