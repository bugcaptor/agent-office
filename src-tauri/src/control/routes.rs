// src-tauri/src/control/routes.rs
//
// 세션·알림·설정 핸들러. 전부 대응하는 Tauri command 본문과 **같은 동작**을
// 내도록 짜여 있다 — CLI로 한 일과 앱에서 한 일이 갈라지면 그 순간부터
// 두 경로를 따로 디버깅해야 하기 때문이다.
use std::sync::Arc;

use axum::extract::{Json, State};

use crate::httpapi::{fail, ok, session_state_str};
use crate::persistence::settings_store::AppSettings;
use crate::session_events::types::AgentEventProfile;
use crate::types::{CreateSessionRequest, SessionState};

use super::protocol::*;
use super::ControlContext;

// ── 핸들러(기존 command 본문 재사용) ─────────────────────────────────

pub(super) async fn ping(State(ctx): State<Arc<ControlContext>>) -> Json<serde_json::Value> {
    let profiles = ctx.store.load();
    let running = ctx
        .registry
        .snapshot()
        .into_iter()
        .filter(|(_, _, s)| matches!(s, SessionState::Running | SessionState::Starting))
        .count();
    ok(PingResult {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        agent_count: profiles.agents.len(),
        running_count: running,
    })
}

pub(super) async fn list(State(ctx): State<Arc<ControlContext>>) -> Json<serde_json::Value> {
    let profiles = ctx.store.load();
    let mut by_agent: std::collections::HashMap<String, (String, SessionState)> =
        std::collections::HashMap::new();
    for (sid, agent, state) in ctx.registry.snapshot() {
        by_agent.insert(agent, (sid, state));
    }
    let entries: Vec<ListEntry> = profiles
        .agents
        .iter()
        .map(|a| {
            let live = by_agent.get(&a.id);
            ListEntry {
                agent_id: a.id.clone(),
                name: a.name.clone(),
                role: a.role.clone(),
                cwd: a.cwd.clone(),
                state: live.map(|(_, s)| session_state_str(*s).to_string()),
                session_id: live.map(|(sid, _)| sid.clone()),
            }
        })
        .collect();
    ok(entries)
}

pub(super) async fn create(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<CreateParams>,
) -> Json<serde_json::Value> {
    // 스폰 본문은 `ipc::commands::spawn_session` 하나뿐이다 — 여기서 복제하던
    // observer ensure + catch_unwind + create_with_profile를 그쪽으로 합쳤다.
    let profile = AgentEventProfile {
        name: p.name.clone().unwrap_or_else(|| p.agent_id.clone()),
        role: p.role.clone(),
    };
    // 성격 프롬프트는 디스크 프로필에서 읽는다 — 렌더러 create_session이 늘
    // 함께 보내는 값이라, CLI로 띄운 세션만 성격 없이 뜨면 안 된다(M3에서 고친
    // 결함: 예전엔 무조건 None이었다). 프로필이 없으면 기존대로 None.
    let personality_prompt = profile_personality_prompt(&ctx, &p.agent_id);
    match crate::ipc::commands::spawn_session(
        &ctx.manager,
        &ctx.observer,
        &ctx.observer_server,
        &ctx.settings,
        CreateSessionRequest {
            agent_id: p.agent_id,
            cols: p.cols,
            rows: p.rows,
            cwd: p.cwd,
            shell: p.shell,
            startup_command: p.startup_command,
            personality_prompt,
            autostart_claude: None,
        },
        profile,
    )
    .await
    {
        Ok(created) => ok(created),
        Err(e) => fail(e),
    }
}

/// 디스크 프로필의 성격 프롬프트(없으면 None). `create`가 쓴다.
fn profile_personality_prompt(ctx: &ControlContext, agent_id: &str) -> Option<String> {
    ctx.store
        .load()
        .agents
        .into_iter()
        .find(|a| a.id == agent_id)
        .and_then(|a| a.personality_prompt)
}

pub(super) async fn send(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<SendParams>,
) -> Json<serde_json::Value> {
    // ctl 입력은 사람 입력으로 취급(kbm #2t9 Phase 2) — 관문 경유.
    ctx.gate.note_human(&p.agent_id, &p.data);
    ok(serde_json::Value::Null)
}

pub(super) async fn dispose(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<AgentParams>,
) -> Json<serde_json::Value> {
    ctx.manager.dispose(&p.agent_id);
    ok(serde_json::Value::Null)
}

pub(super) async fn notifications(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<AgentParams>,
) -> Json<serde_json::Value> {
    ok(ctx.manager.pending_notifications(&p.agent_id))
}

pub(super) async fn clear(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<ClearParams>,
) -> Json<serde_json::Value> {
    // clear_notifications과 동일: agentId→sessionId 해석 후 hub.clear.
    if let Some(sid) = ctx.manager.session_id_for(&p.agent_id) {
        ctx.hub.clear(&sid, p.ids);
    }
    ok(serde_json::Value::Null)
}

pub(super) async fn settings_get(State(ctx): State<Arc<ControlContext>>) -> Json<serde_json::Value> {
    ok(ctx.settings.read().unwrap().clone())
}

pub(super) async fn settings_set(
    State(ctx): State<Arc<ControlContext>>,
    Json(patch): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let Some(obj) = patch.as_object() else {
        return fail("본문은 JSON 객체여야 합니다");
    };
    // cli_enabled는 CLI로 바꿀 수 없다 — 자기 자신을 켜고/끄는 권한 상승을
    // 막는 보안 결정. GUI에서만 토글한다.
    if obj.contains_key("cliEnabled") || obj.contains_key("cli_enabled") {
        return fail("cliEnabled는 앱 설정에서만 변경할 수 있습니다");
    }
    let current = ctx.settings.read().unwrap().clone();
    let mut merged = match serde_json::to_value(current) {
        Ok(v) => v,
        Err(e) => return fail(e.to_string()),
    };
    if let Some(map) = merged.as_object_mut() {
        for (k, v) in obj {
            map.insert(k.clone(), v.clone());
        }
    }
    let new: AppSettings = match serde_json::from_value(merged) {
        Ok(s) => s,
        Err(e) => return fail(format!("설정 파싱 실패: {e}")),
    };
    match crate::ipc::commands::apply_settings_effects(
        &ctx.settings_store,
        &ctx.settings,
        &ctx.hub,
        &ctx.observer_server,
        &ctx.observer,
        new.clone(),
    )
    .await
    {
        Ok(()) => ok(ctx.settings.read().unwrap().clone()),
        Err(e) => fail(e),
    }
}
