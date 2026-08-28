// src-tauri/src/control/routes.rs
//
// 세션·알림·설정 핸들러. 전부 대응하는 Tauri command 본문과 **같은 동작**을
// 내도록 짜여 있다 — CLI로 한 일과 앱에서 한 일이 갈라지면 그 순간부터
// 두 경로를 따로 디버깅해야 하기 때문이다.
use std::sync::Arc;

use axum::extract::{Json, State};

use crate::httpapi::{fail, ok, session_state_str};
use crate::persistence::settings_store::AppSettings;
use crate::session::external::ExternalDetachReason;
use crate::session_events::types::AgentEventProfile;
use crate::types::{CreateSessionRequest, SessionState};

use super::protocol::*;
use super::{tmux, ControlContext};

/// catch_unwind 페이로드에서 사람이 읽을 메시지를 뽑는다(commands.rs와 동일).
fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

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
    // 함께 보내는 값이라, CLI로 띄운 세션만 성격 없이 뜨면 안 된다(M3에서
    // 고친 결함: 예전엔 무조건 None이었다). 프로필이 없으면 기존대로 None.
    let personality_prompt = profile_personality(&ctx, &p.agent_id);
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

/// 디스크 프로필의 성격 프롬프트(없으면 None). create/attach가 공유한다.
fn profile_personality(ctx: &ControlContext, agent_id: &str) -> Option<String> {
    ctx.store
        .load()
        .agents
        .into_iter()
        .find(|a| a.id == agent_id)
        .and_then(|a| a.personality_prompt)
}

/// 앱 밖 터미널을 캐릭터에 붙인다 — 응답 `script`를 그 터미널에서 eval하면
/// 그 셸에서 뜬 claude의 훅이 이 캐릭터의 알림으로 흐른다.
///
/// create와 같은 골격이다(observer 선기동 + catch_unwind). persona와 표시용
/// 이름/역할은 디스크 프로필에서 읽는다 — 프로필이 없으면 붙일 캐릭터가 없다.
///
/// `tmux`가 오면 전혀 다른 경로다: 논리 세션 대신 **앱이 자기 PTY로 tmux
/// 클라이언트를 여는 일반 세션**을 만든다(control/tmux.rs 헤더 참조).
pub(super) async fn attach(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<AttachParams>,
) -> Json<serde_json::Value> {
    if ctx.settings.read().unwrap().observer_enabled {
        let _ = ctx.observer_server.ensure(ctx.observer.clone()).await;
    }
    let Some(agent) = ctx
        .store
        .load()
        .agents
        .into_iter()
        .find(|a| a.id == p.agent_id)
    else {
        return fail(format!(
            "알 수 없는 캐릭터입니다: {} — 먼저 앱에서 캐릭터를 만들거나 `agent-office ctl create {}`로 세션을 시작하세요",
            p.agent_id, p.agent_id,
        ));
    };
    let profile = AgentEventProfile {
        name: agent.name.clone(),
        role: Some(agent.role.clone()).filter(|role| !role.is_empty()),
    };

    // tmux 모드: 논리 세션을 만들지 않고 `exec tmux attach-session`으로 도는
    // 일반 PTY 세션을 띄운다. 요청한 셸에는 심을 게 없으므로 `pid`(끊김 감지)도
    // 쓰지 않는다 — 세션 수명은 tmux 클라이언트의 종료(on_exit)가 결정한다.
    if let Some(raw_target) = p.tmux.as_deref() {
        let target = match tmux::validate_target(raw_target) {
            Ok(target) => target,
            Err(e) => return fail(e),
        };
        match (ctx.tmux_probe)(&target) {
            tmux::TmuxStatus::Present => {}
            tmux::TmuxStatus::Missing => {
                return fail(format!(
                    "tmux 세션 '{target}'을 찾을 수 없습니다 — `tmux ls`로 이름을 확인하세요"
                ))
            }
            tmux::TmuxStatus::Unavailable(why) => {
                return fail(format!(
                    "tmux를 실행할 수 없습니다(설치되지 않았거나 앱의 PATH에 없습니다): {why}"
                ))
            }
        }
        let manager = ctx.manager.clone();
        let startup_command = tmux::attach_command(&target);
        let personality_prompt = agent.personality_prompt.clone();
        let agent_id = p.agent_id.clone();
        let cwd = p.cwd.clone();
        // create는 살아 있는 세션이 있으면 새로 띄우지 않고 **재사용**한다 —
        // 그러면 tmux 클라이언트는 뜨지 않으므로 성공으로 위장하면 안 된다.
        // 돌아온 sid가 원래 있던 sid와 같은지로 정확히 가려낸다.
        let existing = ctx.manager.session_id_for(&p.agent_id);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            manager.create_with_profile(
                CreateSessionRequest {
                    agent_id,
                    // 크기는 렌더러가 터미널을 붙일 때 resize로 맞춘다(기본 80x24).
                    cols: None,
                    rows: None,
                    cwd,
                    shell: None,
                    startup_command: Some(startup_command),
                    personality_prompt,
                    autostart_claude: None,
                },
                profile,
            )
        }));
        return match result {
            Ok(Ok(created)) if existing.as_deref() == Some(created.session_id.as_str()) => {
                fail(format!(
                    "'{}'에는 이미 세션이 떠 있어 tmux에 붙지 못했습니다 — 탭을 닫거나 \
                     `agent-office ctl dispose {}` 후 다시 시도하세요",
                    p.agent_id, p.agent_id,
                ))
            }
            Ok(Ok(created)) => ok(AttachResult {
                session_id: created.session_id,
                mode: "tmux".to_string(),
                script: crate::session::attach_script::render_tmux_notice(&p.agent_id, &target),
            }),
            Ok(Err(e)) => fail(e),
            Err(panic) => fail(format!(
                "tmux attach 중 내부 오류(panic): {}",
                panic_message(&panic)
            )),
        };
    }

    let manager = ctx.manager.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        manager
            .attach_external_with_profile(
                &p.agent_id,
                p.pid,
                p.cwd.as_deref(),
                agent.personality_prompt.as_deref(),
                profile,
            )
            .map(|outcome| AttachResult {
                session_id: outcome.session_id().to_string(),
                mode: if outcome.is_new() { "new" } else { "bind" }.to_string(),
                script: crate::session::attach_script::render_attach_script(
                    &agent.name,
                    outcome.session_id(),
                    outcome.plan(),
                ),
            })
    }));
    match result {
        Ok(Ok(attached)) => ok(attached),
        Ok(Err(e)) => fail(e),
        Err(panic) => fail(format!(
            "attach 중 내부 오류(panic): {}",
            panic_message(&panic)
        )),
    }
}

/// 외부 세션을 끊는다. 붙어 있지 않았으면 `detached: false`(무해한 no-op).
pub(super) async fn detach(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<DetachParams>,
) -> Json<serde_json::Value> {
    let detached = ctx
        .manager
        .detach_external(&p.agent_id, ExternalDetachReason::Detach);
    ok(DetachResult { detached })
}

pub(super) async fn send(
    State(ctx): State<Arc<ControlContext>>,
    Json(p): Json<SendParams>,
) -> Json<serde_json::Value> {
    // write_input과 동일 — 존재하지 않는 agentId는 무해한 no-op.
    ctx.manager.write_input(&p.agent_id, &p.data);
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
    // talkEnabled도 같은 이유로 막는다 — 대화 스위치를 에이전트가 스스로 켤 수
    // 있으면 "사용자가 켰을 때만 대화한다"는 계약이 무너진다(권한 상승).
    if obj.contains_key("talkEnabled") || obj.contains_key("talk_enabled") {
        return fail("talkEnabled는 앱 설정에서만 변경할 수 있습니다");
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
        &ctx.talk,
        new.clone(),
    )
    .await
    {
        Ok(()) => ok(new),
        Err(e) => fail(e),
    }
}
