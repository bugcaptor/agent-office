// src-tauri/src/control/talk.rs
//
// 동료 대화 라우트(docs/agent-talk-design.md §3).
//
// 다른 라우트와 다른 점 하나: 발신자를 **인자가 아니라 세션 헤더**로 정한다.
// 앱이 세션 셸에 심어 둔 `AGENT_OFFICE_SESSION`을 캐릭터로 되짚으므로 앱 밖
// 셸에서는 남을 사칭할 수도, 아예 발신할 수도 없다.
use std::sync::Arc;

use axum::extract::{Json, State};
use axum::http::HeaderMap;

use crate::httpapi::{fail, ok};

use super::protocol::*;
use super::ControlContext;

// ── 동료 대화(docs/agent-talk-design.md §3) ──────────────────────────

/// Err를 그대로 `ok:false` 응답으로 바꾼다(대화 핸들러 전용 축약).
macro_rules! try_or_fail {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(error) => return fail(error),
        }
    };
}

/// 발신자 판정. 인자가 아니라 **세션 헤더**로 정한다 — 앱이 세션 셸에 심어 둔
/// `AGENT_OFFICE_SESSION` 값을 캐릭터로 되짚으므로, 앱 밖 셸에서는 남을 사칭할
/// 수 없고 아예 발신도 못 한다(§1).
fn caller(ctx: &ControlContext, headers: &HeaderMap) -> Result<crate::types::AgentProfile, String> {
    let sid = headers
        .get(SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("오피스 세션이 아닙니다 — 앱이 띄운 캐릭터 터미널에서 실행하세요")?;
    ctx.store
        .load()
        .agents
        .into_iter()
        .find(|a| ctx.manager.session_id_for(&a.id).as_deref() == Some(sid))
        .ok_or_else(|| "이 세션에 붙은 캐릭터를 찾을 수 없습니다".to_string())
}

/// 상대 지정 해석 — agentId 우선, 없으면 이름(정확히 일치). 이름이 겹치면
/// 후보를 돌려주고 거절한다(엉뚱한 사람에게 말이 가는 것보다 낫다).
fn resolve_target(ctx: &ControlContext, to: &str) -> Result<crate::types::AgentProfile, String> {
    let agents = ctx.store.load().agents;
    if let Some(hit) = agents.iter().find(|a| a.id == to) {
        return Ok(hit.clone());
    }
    let by_name: Vec<_> = agents.iter().filter(|a| a.name == to).collect();
    match by_name.len() {
        1 => Ok(by_name[0].clone()),
        0 => Err(format!("그런 동료가 없습니다: {to} (roster로 확인하세요)")),
        _ => Err(format!(
            "이름이 겹칩니다: {to} — id로 지정하세요({})",
            by_name
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// 이 캐릭터에게 말이 닿는지와 그 사유.
fn reachability(ctx: &ControlContext, agent: &crate::types::AgentProfile) -> (bool, bool, Option<String>) {
    let idle_quiet = ctx.talk.config().idle_quiet_ms;
    if agent.talk_receive == Some(false) {
        return (false, false, Some("수신 꺼짐".into()));
    }
    if !ctx.manager.is_running(&agent.id) {
        return (false, false, Some("실행 중인 세션 없음".into()));
    }
    let busy = ctx
        .manager
        .idle_ms(&agent.id)
        .is_none_or(|ms| ms < idle_quiet);
    (true, busy, None)
}

fn talk_gate(ctx: &ControlContext) -> Result<(), String> {
    if ctx.talk.is_enabled() {
        Ok(())
    } else {
        Err("동료 대화가 꺼져 있습니다 — 앱 설정에서 켜세요".into())
    }
}

/// 롱폴링 상한(§3.1). ask의 기본 120초보다 넉넉히 잡되 무한 대기는 없다.
const MAX_WAIT_MS: u64 = 180_000;

pub(super) async fn talk_roster(
    State(ctx): State<Arc<ControlContext>>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    try_or_fail!(talk_gate(&ctx));
    let me = try_or_fail!(caller(&ctx, &headers));
    let entries: Vec<RosterEntry> = ctx
        .store
        .load()
        .agents
        .into_iter()
        .map(|a| {
            let is_me = a.id == me.id;
            let (reachable, busy, reason) = if is_me {
                (false, false, Some("나 자신".into()))
            } else {
                reachability(&ctx, &a)
            };
            RosterEntry {
                agent_id: a.id,
                name: a.name,
                role: a.role,
                cwd: a.cwd,
                reachable,
                busy,
                reason,
                is_me,
            }
        })
        .collect();
    ok(entries)
}

pub(super) async fn talk_send(
    State(ctx): State<Arc<ControlContext>>,
    headers: HeaderMap,
    Json(p): Json<TalkSendParams>,
) -> Json<serde_json::Value> {
    try_or_fail!(talk_gate(&ctx));
    let me = try_or_fail!(caller(&ctx, &headers));
    let target = try_or_fail!(resolve_target(&ctx, &p.to));
    let (reachable, _busy, reason) = reachability(&ctx, &target);
    if !reachable {
        return fail(format!(
            "{}에게는 지금 말이 닿지 않습니다: {}",
            target.name,
            reason.unwrap_or_else(|| "알 수 없음".into())
        ));
    }
    let (conv_id, msg_id) = try_or_fail!(ctx.talk.enqueue(
        &me.id,
        &me.name,
        &target.id,
        &target.name,
        &p.text,
        p.conv_id.as_deref(),
    ));
    let reply = wait_for_reply(&ctx, &me.id, &conv_id, p.wait_ms).await;
    ok(crate::talk::SendOutcome {
        conv_id,
        msg_id,
        reply,
    })
}

pub(super) async fn talk_reply(
    State(ctx): State<Arc<ControlContext>>,
    headers: HeaderMap,
    Json(p): Json<TalkReplyParams>,
) -> Json<serde_json::Value> {
    try_or_fail!(talk_gate(&ctx));
    let me = try_or_fail!(caller(&ctx, &headers));
    let conv = match ctx.talk.conversation(&p.conv_id) {
        Some(c) if c.has(&me.id) => c,
        Some(_) => return fail("이 대화의 참여자가 아닙니다"),
        None => return fail(format!("없는 대화입니다: {}", p.conv_id)),
    };
    let other = conv.other(&me.id).to_string();
    let other_name = ctx
        .store
        .load()
        .agents
        .into_iter()
        .find(|a| a.id == other)
        .map(|a| a.name)
        .unwrap_or_else(|| other.clone());
    let (conv_id, msg_id) = try_or_fail!(ctx.talk.enqueue(
        &me.id,
        &me.name,
        &other,
        &other_name,
        &p.text,
        Some(&p.conv_id),
    ));
    let reply = wait_for_reply(&ctx, &me.id, &conv_id, p.wait_ms).await;
    ok(crate::talk::SendOutcome {
        conv_id,
        msg_id,
        reply,
    })
}

pub(super) async fn talk_inbox(
    State(ctx): State<Arc<ControlContext>>,
    headers: HeaderMap,
    Json(p): Json<TalkInboxParams>,
) -> Json<serde_json::Value> {
    try_or_fail!(talk_gate(&ctx));
    let me = try_or_fail!(caller(&ctx, &headers));
    let wait = p.wait_ms.unwrap_or(0).min(MAX_WAIT_MS);
    ok(ctx.talk.wait(&me.id, None, wait).await)
}

pub(super) async fn talk_end(
    State(ctx): State<Arc<ControlContext>>,
    headers: HeaderMap,
    Json(p): Json<TalkEndParams>,
) -> Json<serde_json::Value> {
    let me = try_or_fail!(caller(&ctx, &headers));
    let reason = p.reason.unwrap_or_else(|| "manual".into());
    try_or_fail!(ctx.talk.end(&me.id, &p.conv_id, &reason));
    ok(serde_json::Value::Null)
}

/// `waitMs`가 있으면 그동안 이 대화의 답장을 기다린다. 대기 중임을 허브가
/// 알기 때문에 배달 워커가 그 답장을 PTY로 밀어 넣지 않는다(§4).
async fn wait_for_reply(
    ctx: &ControlContext,
    me: &str,
    conv_id: &str,
    wait_ms: Option<u64>,
) -> Option<crate::talk::TalkMessage> {
    let wait = wait_ms.unwrap_or(0).min(MAX_WAIT_MS);
    if wait == 0 {
        return None;
    }
    ctx.talk.wait(me, Some(conv_id), wait).await.into_iter().next()
}
