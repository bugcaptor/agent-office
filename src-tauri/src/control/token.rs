// src-tauri/src/control/token.rs
//
// 2단계 옵트인의 두 번째 단계 — 토큰 파일과 그것을 검사하는 미들웨어.
//
// 서버가 떠 있는 것만으로는 아무 요청도 통하지 않는다. 앱에서 명시적으로
// 승인해야 `control-token`(0600)이 생기고, 미들웨어는 **매 요청마다** 그
// 파일을 다시 읽는다 — 그래서 승인·취소가 서버 재시작 없이 즉시 반영된다.
use std::path::Path;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::httpapi::{ct_eq, fail, set_owner_only};

use super::protocol::*;
use super::ControlContext;

// ── 토큰/포트 파일 헬퍼(승인 커맨드와 서버가 공유) ─────────────────────

pub fn read_token_at(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(TOKEN_FILE)).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 새 토큰을 발급하고 `control-token`(0600)에 기록한다. 승인 시 호출.
pub fn issue_token_at(dir: &Path) -> std::io::Result<String> {
    std::fs::create_dir_all(dir)?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let path = dir.join(TOKEN_FILE);
    std::fs::write(&path, &token)?;
    set_owner_only(&path);
    Ok(token)
}

/// 승인 취소 — 토큰 파일을 지운다. 없으면 무해한 no-op. 이후 모든 요청 401.
pub fn revoke_token_at(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(dir.join(TOKEN_FILE)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

// ── 인증 미들웨어 ────────────────────────────────────────────────────

pub(super) async fn auth(State(ctx): State<Arc<ControlContext>>, req: Request, next: Next) -> Response {
    let presented = req
        .headers()
        .get(TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let authorized = match (presented, ctx.read_token()) {
        (Some(p), Some(expected)) => ct_eq(p.as_bytes(), expected.as_bytes()),
        _ => false,
    };
    if authorized {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            fail("unauthorized: 앱 설정에서 CLI 제어를 승인했는지 확인하세요"),
        )
            .into_response()
    }
}
