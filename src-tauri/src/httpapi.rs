// src-tauri/src/httpapi.rs
//
// 앱 안에서 도는 두 HTTP 서버(`control`, `webremote`)가 함께 쓰는 조각들.
//
// 두 서버는 리스너도 Router도 일부러 분리돼 있다 — control은 127.0.0.1
// 전용이고 `settings/set` 같은 라우트를 갖는 반면, webremote는 tailnet에
// 노출되며 그런 라우트가 아예 없다(권한 축소를 구조로 보장). 하지만 **응답
// 봉투의 모양**과 **토큰 파일을 다루는 보안 규칙**은 같아야 한다: 한쪽만
// `{ok,data}` 모양을 바꾸면 두 클라이언트의 파싱이 갈라지고, 한쪽만 0600을
// 빠뜨리면 그 서버의 토큰이 새어도 다른 쪽 코드를 봐서는 알 수 없다.
//
// 그래서 "같아야만 하는 것"만 여기 둔다. 서버마다 달라도 되는 것(라우트,
// 인증 정책, 바인딩 주소)은 각자의 모듈에 그대로 남는다.
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
