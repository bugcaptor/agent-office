// src-tauri/src/webremote/ws.rs
//
// WS 업그레이드와 한 뷰어 연결의 수명.
//
// 읽기(뷰어 메시지)·쓰기(broadcast 팬아웃)·keepalive를 `serve_ws`의 한 루프에서
// select 한다 — 쓰기 주체가 하나뿐이라 소켓에 배타 잠금이 필요 없다. 연결마다
// 붙는 일련번호(NEXT_CONN_ID)가 채팅 구독의 단위다: 같은 토큰으로 탭을 두 개
// 열면 각각이 따로 구독한다.
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};

use crate::httpapi::fail;

use super::auth::{authenticate, origin_allowed, subprotocol_token};
use super::pairing::ClientRecord;
use super::protocol::*;
use super::rpc;
use super::{WebRemoteContext, NEXT_CONN_ID, WS_IDLE_TIMEOUT, WS_PING_EVERY, WS_TOKEN_PROTOCOL_PREFIX};

pub(super) async fn ws_route(
    State(ctx): State<Arc<WebRemoteContext>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !ctx.rate.auth_allowed(addr.ip()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            fail("인증 실패가 잦습니다. 잠시 후 다시 시도하세요"),
        )
            .into_response();
    }
    if !origin_allowed(&headers) {
        return (
            StatusCode::FORBIDDEN,
            fail("forbidden: 허용되지 않은 오리진입니다"),
        )
            .into_response();
    }
    let Some(record) = authenticate(&ctx, &headers) else {
        ctx.rate.note_auth_failure(addr.ip());
        eprintln!("webremote: WS 인증 실패 from {}", addr.ip());
        return (
            StatusCode::UNAUTHORIZED,
            fail("unauthorized: 페어링이 취소됐거나 토큰이 무효합니다"),
        )
            .into_response();
    };
    // 서브프로토콜로 인증했다면 그 값을 그대로 echo 해야 브라우저가 핸드셰이크를
    // 받아들인다.
    let ws = match subprotocol_token(&headers) {
        Some(token) => ws.protocols([format!("{WS_TOKEN_PROTOCOL_PREFIX}{token}")]),
        None => ws,
    };
    ws.on_upgrade(move |socket| serve_ws(socket, ctx, record))
}

/// 한 뷰어 연결의 수명. 읽기(뷰어 메시지)·쓰기(broadcast 팬아웃)·keepalive를
/// 한 루프에서 select 한다 — 쓰기 주체가 하나라 소켓 배타 잠금이 필요 없다.
async fn serve_ws(socket: WebSocket, ctx: Arc<WebRemoteContext>, client: ClientRecord) {
    let (mut sink, mut stream) = socket.split();
    let mut rx = ctx.hub.subscribe();
    let conn = NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed);
    // agentId → 다음에 기대하는 절대 오프셋(구멍 감지 + 재접속 기준점).
    let mut attached: HashMap<String, u64> = HashMap::new();
    // 채팅을 구독 중인 캐릭터들(터미널 attach와 독립이다 — 채팅 뷰가 주 화면).
    let mut following: HashSet<String> = HashSet::new();
    let mut last_seen = Instant::now();
    let mut ping = tokio::time::interval(WS_PING_EVERY);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    if send_msg(
        &mut sink,
        &HostMsg::Hello {
            host_name: ctx.host_name.clone(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            proto_version: WEB_REMOTE_PROTO_VERSION,
            permission: client.permission,
            client_id: client.client_id.clone(),
        },
    )
    .await
    .is_err()
    {
        return;
    }
    let _ = send_msg(
        &mut sink,
        &HostMsg::Agents {
            agents: ctx.build_agents_for(&client),
        },
    )
    .await;

    loop {
        tokio::select! {
            incoming = stream.next() => {
                let Some(Ok(frame)) = incoming else { break };
                last_seen = Instant::now();
                match frame {
                    Message::Text(text) => {
                        let Ok(msg) = serde_json::from_str::<ClientMsg>(&text) else {
                            let _ = send_msg(&mut sink, &HostMsg::Error {
                                message: "알 수 없는 메시지".into(),
                            }).await;
                            continue;
                        };
                        if handle_client_msg(&mut sink, &ctx, &client, conn, &mut attached, &mut following, msg).await.is_err() {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            event = rx.recv() => match event {
                Ok(msg) => {
                    // 터미널 프레임은 attach한 캐릭터에만, 채팅 프레임은 follow한
                    // 캐릭터에만 간다. 알림·활동·세션 상태는 필터가 없다(§M2).
                    if let Some(agent) = terminal_agent(&msg) {
                        if !attached.contains_key(agent) {
                            continue;
                        }
                    }
                    if let HostMsg::Chat { agent_id, .. } = &*msg {
                        if !following.contains(agent_id) {
                            continue;
                        }
                    }
                    if let HostMsg::Output(out) = &*msg {
                        let expected = attached.get(&out.agent_id).copied().unwrap_or(out.offset);
                        if out.offset != expected {
                            // 구멍(느린 뷰어의 broadcast 유실 등) — 그 캐릭터만
                            // 기준점부터 다시 복원한다.
                            if restore_agent(&mut sink, &ctx, &out.agent_id, Some(expected), &mut attached).await.is_err() {
                                break;
                            }
                            continue;
                        }
                        attached.insert(out.agent_id.clone(), out.offset + out.bytes);
                    }
                    // 세션 상태가 바뀌면 목록 메타(state/크기)도 같이 갱신한다.
                    if matches!(&*msg, HostMsg::SessionState { .. }) {
                        let _ = send_msg(&mut sink, &HostMsg::Agents { agents: ctx.build_agents_for(&client) }).await;
                    }
                    if send_msg(&mut sink, &msg).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // 큐를 놓쳤다 — 붙어 있는 캐릭터 전부를 마지막 지점부터 복원.
                    let agents: Vec<(String, u64)> =
                        attached.iter().map(|(a, o)| (a.clone(), *o)).collect();
                    let mut failed = false;
                    for (agent, offset) in agents {
                        if restore_agent(&mut sink, &ctx, &agent, Some(offset), &mut attached).await.is_err() {
                            failed = true;
                            break;
                        }
                    }
                    if failed { break; }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            _ = ping.tick() => {
                if last_seen.elapsed() > WS_IDLE_TIMEOUT {
                    break;
                }
                if sink.send(Message::Ping(Vec::new())).await.is_err() {
                    break;
                }
            }
        }
    }
    ctx.chat.release(conn);
    let _ = sink.close().await;
}

/// 이 메시지가 **터미널 미러**에 매인 것이면 그 agentId(attach 구독 필터용).
///
/// M2에서 좁혔다: 알림·활동·세션 상태는 채팅 뷰의 재료라 터미널에 붙지 않은
/// 캐릭터도 받아야 한다(단일 사용자 tailnet — 전체 broadcast로 충분).
fn terminal_agent(msg: &HostMsg) -> Option<&str> {
    match msg {
        HostMsg::Output(out) => Some(&out.agent_id),
        HostMsg::Resized { agent_id, .. } | HostMsg::Restore { agent_id, .. } => Some(agent_id),
        _ => None,
    }
}

type WsSink = futures_util::stream::SplitSink<WebSocket, Message>;

async fn send_msg(sink: &mut WsSink, msg: &HostMsg) -> Result<(), ()> {
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    sink.send(Message::Text(text)).await.map_err(|_| ())
}

async fn handle_client_msg(
    sink: &mut WsSink,
    ctx: &Arc<WebRemoteContext>,
    client: &ClientRecord,
    conn: u64,
    attached: &mut HashMap<String, u64>,
    following: &mut HashSet<String>,
    msg: ClientMsg,
) -> Result<(), ()> {
    match msg {
        ClientMsg::Ping => send_msg(sink, &HostMsg::Pong).await,
        ClientMsg::Detach { agent_id } => {
            attached.remove(&agent_id);
            Ok(())
        }
        ClientMsg::Attach {
            agent_id,
            last_offset,
        } => {
            if !ctx.agent_allowed(client, &agent_id) {
                return send_msg(
                    sink,
                    &HostMsg::Error {
                        message: format!("접근할 수 없는 캐릭터입니다: {agent_id}"),
                    },
                )
                .await;
            }
            // 웹 클라이언트는 공유 토글 없이 붙으므로 tap이 아직 없을 수 있다.
            // sink는 agentId 수명이라 세션 전에 달아도 안전하고, share()는 멱등이다.
            ctx.hub.share(&ctx.manager, &agent_id);
            restore_agent(sink, ctx, &agent_id, last_offset, attached).await
        }
        ClientMsg::Input { agent_id, data } => {
            if !client.permission.allows_input() {
                return send_msg(
                    sink,
                    &HostMsg::Error {
                        message: "읽기 전용으로 연결되어 입력할 수 없습니다".into(),
                    },
                )
                .await;
            }
            if !ctx.agent_allowed(client, &agent_id) {
                return Ok(());
            }
            ctx.manager.write_input(&agent_id, &data);
            Ok(())
        }
        ClientMsg::Rpc { id, cmd, args } => {
            // 채팅 구독은 이 연결의 수명에 매인다 — 성공한 follow만 기록해
            // 연결이 끊길 때 정확히 그만큼 놓는다(중복 follow는 registry가
            // 멱등이라 여기서 더 셀 것이 없다).
            let follow_target = (cmd == "chat.follow")
                .then(|| args.get("agentId").and_then(|v| v.as_str()).map(str::to_string))
                .flatten();
            let result = rpc::dispatch(ctx, client, conn, &cmd, args).await;
            if result.is_ok() {
                if let Some(agent_id) = follow_target {
                    following.insert(agent_id);
                }
            }
            let msg = match result {
                Ok(data) => HostMsg::RpcResult {
                    id,
                    ok: true,
                    data: Some(data),
                    error: None,
                },
                Err(error) => HostMsg::RpcResult {
                    id,
                    ok: false,
                    data: None,
                    error: Some(error),
                },
            };
            send_msg(sink, &msg).await
        }
    }
}

/// 복원(스냅샷+델타 또는 델타만)을 보내고 `attached` 기준점을 갱신한다.
async fn restore_agent(
    sink: &mut WsSink,
    ctx: &Arc<WebRemoteContext>,
    agent_id: &str,
    last_offset: Option<u64>,
    attached: &mut HashMap<String, u64>,
) -> Result<(), ()> {
    let Some(plan) = ctx.hub.replay_for(agent_id, last_offset).await else {
        return Ok(());
    };
    let (cols, rows) = ctx.manager.size_of(agent_id).unwrap_or((0, 0));
    send_msg(
        sink,
        &HostMsg::Restore {
            agent_id: agent_id.to_string(),
            snapshot: plan.snapshot.clone(),
            base_offset: plan.base_offset,
            cols,
            rows,
            session_id: ctx.hub.session_id_of(agent_id),
        },
    )
    .await?;
    let mut next = plan.base_offset;
    for chunk in plan.chunks {
        next = chunk.offset + chunk.bytes;
        send_msg(
            sink,
            &HostMsg::Output(RemoteOutput {
                agent_id: agent_id.to_string(),
                session_id: chunk.session_id,
                seq: chunk.seq,
                offset: chunk.offset,
                data: chunk.data,
                bytes: chunk.bytes,
            }),
        )
        .await?;
    }
    attached.insert(agent_id.to_string(), next);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 터미널 프레임만 attach 필터를 탄다. 알림·활동·세션 상태는 채팅 뷰의
    /// 재료라 붙지 않은 캐릭터도 받아야 한다(M2에서 좁힌 규칙).
    #[test]
    fn only_terminal_frames_are_gated_by_attach() {
        let out = HostMsg::Output(RemoteOutput {
            agent_id: "ada".into(),
            session_id: "s".into(),
            seq: 1,
            offset: 0,
            data: "x".into(),
            bytes: 1,
        });
        assert_eq!(terminal_agent(&out), Some("ada"));
        assert_eq!(
            terminal_agent(&HostMsg::Resized {
                agent_id: "ada".into(),
                cols: 80,
                rows: 24
            }),
            Some("ada")
        );
        assert_eq!(terminal_agent(&HostMsg::Pong), None);
        assert_eq!(terminal_agent(&HostMsg::Agents { agents: vec![] }), None);

        // 이 셋은 이제 필터를 타지 않는다.
        for msg in [
            HostMsg::Notification {
                agent_id: "ada".into(),
                payload: serde_json::json!({}),
            },
            HostMsg::Activity {
                agent_id: "ada".into(),
                payload: serde_json::json!({}),
            },
            HostMsg::SessionState {
                agent_id: "ada".into(),
                payload: serde_json::json!({}),
            },
            HostMsg::NotificationCleared {
                agent_id: "ada".into(),
                ids: vec![],
            },
        ] {
            assert_eq!(terminal_agent(&msg), None, "{msg:?}");
        }
        // 채팅 프레임은 별도의 follow 집합으로 거른다.
        assert_eq!(
            terminal_agent(&HostMsg::Chat {
                agent_id: "ada".into(),
                items: vec![],
                backfill: false,
                unavailable: false,
            }),
            None
        );
    }
}
