// 이미 실행 중인 Codex/Claude의 로컬 기록을 읽는 연결기 API. 이 경로는 원본
// 프로세스에 입력하거나 설정·훅을 쓰지 않고, 검증된 최소 상태만 허브에 전달한다.

use std::path::Path;
use std::sync::Arc;

use axum::extract::{Json, State};
use uuid::Uuid;

use crate::httpapi::{fail, ok};
use crate::session::external::ObservedEventKind;
use crate::session_events::types::AgentEventProfile;

use super::protocol::*;
use super::ControlContext;

pub(super) async fn attach(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<ObservedAttachParams>,
) -> Json<serde_json::Value> {
    let _configuration = ctx.manager.observed_configuration.lock();
    if let Err(error) = require_enabled(&ctx.settings.read().unwrap()) { return fail(error); }
    if p.source_session_id.trim().is_empty() {
        return fail("observed-source-invalid");
    }
    if Uuid::parse_str(&p.owner_id).is_err() {
        return fail("observed-owner-invalid");
    }
    if p.pid == 0 {
        return fail("observed-pid-invalid");
    }
    if !Path::new(&p.cwd).is_absolute() {
        return fail("observed-cwd-not-absolute");
    }
    let Some(agent) = ctx
        .store
        .load()
        .agents
        .into_iter()
        .find(|agent| agent.id == p.agent_id)
    else {
        return fail("observed-agent-not-found");
    };
    let Some(profile_cwd) = agent.cwd.as_deref().filter(|cwd| !cwd.is_empty()) else {
        return fail("observed-profile-cwd-missing");
    };
    if profile_cwd != p.cwd {
        return fail("observed-cwd-mismatch");
    }
    let profile = AgentEventProfile {
        name: agent.name,
        role: Some(agent.role).filter(|role| !role.is_empty()),
    };
    match ctx.manager.attach_observed(
        &p.agent_id,
        p.provider.as_str(),
        &p.source_session_id,
        &p.cwd,
        &p.owner_id,
        Some(p.pid),
        profile,
    ) {
        Ok(attached) => ok(ObservedAttachResult {
            session_id: attached.session_id,
        }),
        Err(error) => fail(error),
    }
}

pub(super) async fn event(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<ObservedEventParams>,
) -> Json<serde_json::Value> {
    let _configuration = ctx.manager.observed_configuration.lock();
    if let Err(error) = require_enabled(&ctx.settings.read().unwrap()) { return fail(error); }
    let kind = match p.kind {
        ObservedEventKindParam::Prompt => ObservedEventKind::Prompt,
        ObservedEventKindParam::Tool => ObservedEventKind::Tool,
        ObservedEventKindParam::Stop => ObservedEventKind::Stop,
        ObservedEventKindParam::Attention => ObservedEventKind::Attention,
        ObservedEventKindParam::Heartbeat => ObservedEventKind::Heartbeat,
    };
    match ctx.manager.ingest_observed_event(
        &p.agent_id,
        &p.session_id,
        &p.owner_id,
        p.sequence,
        kind,
        p.tool_name.as_deref(),
    ) {
        Ok(accepted) => ok(ObservedEventResult { accepted }),
        Err(error) => fail(error),
    }
}

pub(super) async fn detach(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<ObservedDetachParams>,
) -> Json<serde_json::Value> {
    ok(DetachResult {
        detached: ctx
            .manager
            .detach_observed(&p.agent_id, &p.session_id, &p.owner_id),
    })
}

fn require_enabled(settings: &crate::persistence::settings_store::AppSettings) -> Result<(), &'static str> {
    if !settings.ide_connection_enabled { return Err("ide-connection-disabled"); }
    if !settings.observer_enabled { return Err("observed-observer-disabled"); }
    Ok(())
}
