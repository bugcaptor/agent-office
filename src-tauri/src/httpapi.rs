// src-tauri/src/httpapi.rs
//
// 로컬 control 서버(`ctl`)의 응답 봉투와 토큰 파일 보안 규칙.
//
// 예전에는 웹 원격 서버와 공유하던 조각이라 따로 떼어 두었다(웹 원격은
// 점검·정리 §7.D C35로 제거됨). 응답 봉투 `{ok,data}` 모양과 토큰 파일 0600
// 규칙을 한곳에 둔다.
use std::path::Path;

use axum::extract::Json;

use crate::types::SessionState;

/// 토큰 파일을 소유자만 읽을 수 있게 잠근다(0600). Unix 밖에서는 무동작 —
/// 파일 권한 모델이 달라 흉내 내면 오히려 거짓 안심을 준다.
#[cfg(unix)]
pub fn set_owner_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
pub fn set_owner_only(_path: &Path) {}

/// 타이밍 부채널을 줄이는 상수시간 비교(길이는 고정이라 누설 무해).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 세션 상태를 두 서버가 같은 철자로 내보낸다 — 프런트·CLI·브라우저가
/// 이 문자열을 그대로 비교하므로 한쪽만 바꾸면 조용히 어긋난다.
pub fn session_state_str(state: SessionState) -> &'static str {
    match state {
        SessionState::Starting => "starting",
        SessionState::Running => "running",
        SessionState::Exited => "exited",
        SessionState::Disposed => "disposed",
    }
}

// ── 응답 봉투 ────────────────────────────────────────────────────────

pub fn ok<T: serde::Serialize>(data: T) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "data": data }))
}

pub fn fail(msg: impl Into<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": false, "error": msg.into() }))
}
