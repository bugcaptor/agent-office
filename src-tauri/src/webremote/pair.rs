// src-tauri/src/webremote/pair.rs
//
// 페어링 HTTP 핸들러 두 개(`pair/start`, `pair/complete`). 상태 기계와 코드
// 검증은 `pairing.rs`가 갖고 있고, 여기는 그 위에 얹힌 **HTTP 껍데기**다 —
// 레이트리밋, 토글 확인, 쿠키 발급 같은 전송 계층 관심사만 다룬다.
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Json, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::httpapi::{fail, ok};

use super::auth::{cookie_value, forwarded_https};
use super::pairing::{self, ClientRecord, PairingOutcome};
use super::protocol::*;
use super::WebRemoteContext;

pub(super) async fn pair_start(
    State(ctx): State<Arc<WebRemoteContext>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<PairStartRequest>,
) -> Response {
    // 브라우저 클라이언트가 생기면서 페어링 표면이 커졌다 — 시작 자체를
    // IP별로 제한하지 않으면 "새 페어링을 계속 열어 코드를 무한 시도"가 된다.
    if !ctx.rate.allow_start(addr.ip()) {
        eprintln!("webremote: pair/start 레이트리밋 초과 from {}", addr.ip());
        return (
            StatusCode::TOO_MANY_REQUESTS,
            fail("요청이 너무 잦습니다. 잠시 후 다시 시도하세요"),
        )
            .into_response();
    }
    if !ctx.web_remote_enabled() {
        return (
            StatusCode::FORBIDDEN,
            fail("웹 원격이 꺼져 있습니다"),
        )
            .into_response();
    }
    let name = if req.client_name.trim().is_empty() {
        "이름 없는 손님".to_string()
    } else {
        req.client_name.trim().chars().take(60).collect()
    };
    let Some(pending) = ctx.pairing.start(&name) else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            fail("대기 중인 연결 요청이 너무 많습니다"),
        )
            .into_response();
    };
    ctx.notify_pairing(&pending);
    ok(PairStartResponse {
        pairing_id: pending.pairing_id,
        expires_in: pairing::PAIRING_TTL.as_secs(),
        host_name: ctx.host_name.clone(),
        proto_version: WEB_REMOTE_PROTO_VERSION,
    })
    .into_response()
}

pub(super) async fn pair_complete(
    State(ctx): State<Arc<WebRemoteContext>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<PairCompleteRequest>,
) -> Response {
    if !ctx.rate.auth_allowed(addr.ip()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            fail("인증 실패가 잦습니다. 잠시 후 다시 시도하세요"),
        )
            .into_response();
    }
    match ctx.pairing.complete(&req.pairing_id, &req.code) {
        PairingOutcome::Approved(permission) => {
            let record = ClientRecord {
                client_id: pairing::new_client_id(),
                name: ctx
                    .pairing
                    .list()
                    .into_iter()
                    .find(|p| p.pairing_id == req.pairing_id)
                    .map(|p| p.client_name)
                    .unwrap_or_else(|| "브라우저".into()),
                token: pairing::new_token(),
                permission,
                created_at: pairing::now_ms(),
            };
            if let Err(e) = ctx.tokens.insert(record.clone()) {
                return fail(format!("토큰 저장 실패: {e}")).into_response();
            }
            let token = record.token.clone();
            let mut resp = ok(PairCompleteResponse {
                client_token: record.token,
                client_id: record.client_id,
                host_name: ctx.host_name.clone(),
                permission,
                proto_version: WEB_REMOTE_PROTO_VERSION,
            })
            .into_response();
            // 브라우저 인증 쿠키. tailscale serve 경유(https)일 때만 `Secure`가
            // 붙는다(M3 §10.3) — 직결 http 접속에서 붙이면 쿠키가 저장되지 않는다.
            if let Ok(value) = cookie_value(&token, forwarded_https(&headers)).parse() {
                resp.headers_mut()
                    .insert(axum::http::header::SET_COOKIE, value);
            }
            resp
        }
        PairingOutcome::AwaitingApproval => (
            StatusCode::ACCEPTED,
            fail("호스트에서 승인을 기다리는 중입니다"),
        )
            .into_response(),
        PairingOutcome::Rejected => {
            (StatusCode::FORBIDDEN, fail("호스트가 연결을 거부했습니다")).into_response()
        }
        PairingOutcome::WrongCode { remaining } => {
            ctx.rate.note_auth_failure(addr.ip());
            eprintln!("webremote: 페어링 코드 불일치 from {}", addr.ip());
            (
                StatusCode::UNAUTHORIZED,
                fail(format!("코드가 맞지 않습니다(남은 시도 {remaining}회)")),
            )
                .into_response()
        }
        PairingOutcome::Expired => (
            StatusCode::GONE,
            fail("페어링이 만료됐습니다. 다시 시도하세요"),
        )
            .into_response(),
    }
}
