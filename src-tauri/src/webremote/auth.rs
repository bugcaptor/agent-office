// src-tauri/src/webremote/auth.rs
//
// "이 요청이 누구인가" — 인증 계층. 브라우저가 상대라는 점이 이 계층의 모양을
// 전부 정한다: 브라우저의 WebSocket API는 커스텀 헤더를 못 붙이므로 토큰을
// 헤더·쿠키·서브프로토콜 **세 경로**에서 받아야 하고, 쿠키를 쓰는 순간
// 오리진 검사가 필요해진다(남의 페이지가 사용자의 쿠키를 업고 WS를 여는 것).
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::httpapi::fail;

use super::net::bind_policy_allows;
use super::pairing::{self, ClientRecord};
use super::protocol::WEB_REMOTE_TOKEN_HEADER;
use super::{WebRemoteContext, WEB_REMOTE_COOKIE_NAME, WS_TOKEN_PROTOCOL_PREFIX};

pub(super) async fn remote_policy(
    State(ctx): State<Arc<WebRemoteContext>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    if bind_policy_allows(ctx.bind_policy(), addr.ip()) {
        next.run(req).await
    } else {
        (
            StatusCode::FORBIDDEN,
            fail("이 네트워크에서는 접근이 허용되지 않습니다"),
        )
            .into_response()
    }
}

/// 페어링 쿠키 한 줄(순수 함수). `Secure`를 **조건부로만** 붙이는 것이 핵심이다 —
/// 직결 `http://100.x:47800`에서 붙이면 브라우저가 쿠키를 아예 저장하지 않아
/// WS 인증(쿠키 경로)이 통째로 깨진다. https는 tailscale serve 경유일 때만이다.
pub fn cookie_value(token: &str, secure: bool) -> String {
    let mut value = format!(
        "{WEB_REMOTE_COOKIE_NAME}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
        pairing::TOKEN_MAX_AGE_SECS
    );
    if secure {
        value.push_str("; Secure");
    }
    value
}

/// tailscaled의 serve 프록시가 붙여 주는 `X-Forwarded-Proto`. 직결 클라이언트가
/// 이 헤더를 위조해도 **자기 쿠키에 속성이 하나 더 붙을 뿐**이라 무해하다
/// (권한이 아니라 저장 조건이다). 프록시 체인을 대비해 첫 값만 본다.
pub(super) fn forwarded_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(',')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("https")
        })
        .unwrap_or(false)
}

/// 제시된 토큰을 뽑는다. 세 경로를 모두 본다:
///
/// 1. `X-Agent-Office-Web-Remote-Token` 헤더 — 브라우저가 아닌 클라이언트(진단 도구
///    등)의 경로.
/// 2. `Cookie: ao_web_remote_token=…` — **브라우저 경로**. 브라우저의 WebSocket API는
///    커스텀 헤더를 붙일 수 없어서(웹 호스팅 #7m §D) 헤더만 보면 브라우저는
///    아예 붙지 못한다. 페어링 완료 시 HttpOnly 쿠키를 발급해 두면 업그레이드에
///    자동으로 동반된다.
/// 3. `Sec-WebSocket-Protocol: agent-office.token.<token>` — 쿠키를 못 쓰는
///    상황(교차 오리진 등)의 표준 관용 우회. 서버는 고른 서브프로토콜을 응답에
///    그대로 echo 해야 하므로 `ws.protocols(...)`로 되돌려준다.
pub(super) fn presented_token(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get(WEB_REMOTE_TOKEN_HEADER).and_then(|v| v.to_str().ok()) {
        return Some(v.to_string());
    }
    if let Some(cookie) = headers.get(axum::http::header::COOKIE).and_then(|v| v.to_str().ok()) {
        for part in cookie.split(';') {
            if let Some(value) = part.trim().strip_prefix(&format!("{WEB_REMOTE_COOKIE_NAME}=")) {
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    subprotocol_token(headers)
}

/// `Sec-WebSocket-Protocol` 목록에서 `agent-office.token.<token>` 항목을 찾는다.
pub(super) fn subprotocol_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get(axum::http::header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok())?;
    raw.split(',')
        .map(str::trim)
        .find_map(|p| p.strip_prefix(WS_TOKEN_PROTOCOL_PREFIX))
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

pub(super) fn authenticate(ctx: &WebRemoteContext, headers: &HeaderMap) -> Option<ClientRecord> {
    ctx.tokens.authenticate(&presented_token(headers)?)
}

/// 브라우저 클라이언트에 필요한 방어(#7m §D). `Origin`이 없으면
/// 브라우저가 아니다 → 통과. 있으면 우리가 서빙하는 것과 같은 오리진만
/// 허용한다 — 남의 페이지가 사용자의 쿠키를 업고 WS를 여는 것을 막는다.
pub(super) fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(axum::http::header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let Some(host) = headers.get(axum::http::header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let origin_host = origin
        .split("://")
        .nth(1)
        .unwrap_or(origin)
        .trim_end_matches('/');
    origin_host == host
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_gets_secure_only_behind_the_serve_proxy() {
        let plain = cookie_value("tok", false);
        assert!(plain.starts_with(&format!("{WEB_REMOTE_COOKIE_NAME}=tok;")));
        assert!(plain.contains("HttpOnly"));
        assert!(plain.contains("SameSite=Strict"));
        // 직결 http에서 Secure가 붙으면 브라우저가 쿠키를 버린다.
        assert!(!plain.contains("Secure"));

        let secure = cookie_value("tok", true);
        assert!(secure.ends_with("; Secure"), "{secure}");
    }

    #[test]
    fn forwarded_proto_detects_https() {
        let mut https = HeaderMap::new();
        https.insert("x-forwarded-proto", "https".parse().unwrap());
        assert!(forwarded_https(&https));

        // 프록시 체인은 첫 값이 클라이언트에 가장 가깝다.
        let mut chain = HeaderMap::new();
        chain.insert("x-forwarded-proto", "HTTPS, http".parse().unwrap());
        assert!(forwarded_https(&chain));

        let mut plain = HeaderMap::new();
        plain.insert("x-forwarded-proto", "http".parse().unwrap());
        assert!(!forwarded_https(&plain));

        // 직결(헤더 없음) = http.
        assert!(!forwarded_https(&HeaderMap::new()));
    }

    /// 브라우저 경로(#7m §D): 커스텀 헤더를 못 붙이는 클라이언트가 쿠키/
    /// 서브프로토콜로도 인증돼야 한다. 헤더만 보던 시절엔 브라우저가 아예
    /// 붙지 못했다.
    #[test]
    fn token_is_accepted_from_header_cookie_or_subprotocol() {
        let mut headers = HeaderMap::new();
        assert_eq!(presented_token(&headers), None);

        headers.insert(WEB_REMOTE_TOKEN_HEADER, "from-header".parse().unwrap());
        assert_eq!(presented_token(&headers).as_deref(), Some("from-header"));

        let mut cookies = HeaderMap::new();
        cookies.insert(
            axum::http::header::COOKIE,
            format!("other=1; {WEB_REMOTE_COOKIE_NAME}=from-cookie; x=2")
                .parse()
                .unwrap(),
        );
        assert_eq!(presented_token(&cookies).as_deref(), Some("from-cookie"));

        let mut proto = HeaderMap::new();
        proto.insert(
            axum::http::header::SEC_WEBSOCKET_PROTOCOL,
            format!("chat, {WS_TOKEN_PROTOCOL_PREFIX}from-proto")
                .parse()
                .unwrap(),
        );
        assert_eq!(presented_token(&proto).as_deref(), Some("from-proto"));
        assert_eq!(subprotocol_token(&proto).as_deref(), Some("from-proto"));

        // 값이 빈 쿠키/서브프로토콜은 토큰으로 치지 않는다.
        let mut empty = HeaderMap::new();
        empty.insert(
            axum::http::header::COOKIE,
            format!("{WEB_REMOTE_COOKIE_NAME}=").parse().unwrap(),
        );
        assert_eq!(presented_token(&empty), None);
    }

    #[test]
    fn origin_is_only_checked_for_browsers() {
        // Origin 없음 = 브라우저가 아님 → 통과.
        assert!(origin_allowed(&HeaderMap::new()));

        let mut same = HeaderMap::new();
        same.insert(axum::http::header::ORIGIN, "http://100.64.1.2:47800".parse().unwrap());
        same.insert(axum::http::header::HOST, "100.64.1.2:47800".parse().unwrap());
        assert!(origin_allowed(&same));

        // 남의 페이지가 사용자의 쿠키를 업고 여는 WS는 막는다.
        let mut cross = HeaderMap::new();
        cross.insert(axum::http::header::ORIGIN, "http://evil.example".parse().unwrap());
        cross.insert(axum::http::header::HOST, "100.64.1.2:47800".parse().unwrap());
        assert!(!origin_allowed(&cross));

        // Host를 못 읽으면 판단 불가 → 거부(안전 쪽).
        let mut no_host = HeaderMap::new();
        no_host.insert(axum::http::header::ORIGIN, "http://x".parse().unwrap());
        assert!(!origin_allowed(&no_host));
    }
}
