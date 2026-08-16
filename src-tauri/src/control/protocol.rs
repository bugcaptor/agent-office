// src-tauri/src/control/protocol.rs
//
// CLI 제어(#55, docs/cli-control-design.md)의 와이어 계약. 앱 안의 로컬
// control 서버(`control/mod.rs`)와 얇은 `ctl` 클라이언트(`control/client.rs`)가
// 공유한다. 전송은 `POST http://127.0.0.1:<port>/v1/<command>` + 헤더
// `X-Agent-Office-Token: <token>` + JSON 본문. 응답 봉투는 항상
// `{ "ok": true, "data": … }` 또는 `{ "ok": false, "error": … }`.

use serde::{Deserialize, Serialize};

/// 토큰 헤더 이름(소문자 — axum HeaderMap은 대소문자 무시지만 상수는 소문자로).
/// 커스텀 헤더 필수는 브라우저發 단순 폼 POST(토큰 없이 CSRF 시도)를 막는
/// 부가 방어이기도 하다 — 크로스오리진에서 커스텀 헤더는 프리플라이트를
/// 요구하고, 우리는 CORS 허용 헤더를 내보내지 않으므로 브라우저가 차단한다.
pub const TOKEN_HEADER: &str = "x-agent-office-token";

/// `<app_data_dir>` 하위 파일명. 서버가 뜨면 포트를, 승인 시 토큰을 기록한다.
pub const PORT_FILE: &str = "control-port";
pub const TOKEN_FILE: &str = "control-token";

/// `create` 파라미터 — 렌더러 `SessionOpts`의 CLI 부분집합.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateParams {
    pub agent_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default)]
    pub startup_command: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub rows: Option<u16>,
}

/// `dispose`/`notifications` 파라미터.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentParams {
    pub agent_id: String,
}

/// `attach` 파라미터 — 앱 밖 터미널을 캐릭터에 붙인다. `pid`는 그 터미널
/// 셸의 PID(끊김 감지용 스윕 대상), `cwd`는 타임라인 표시용 작업 폴더로,
/// 둘 다 `ctl`이 자동 수집해 보낸다(unix 외에는 pid가 빠진다).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachParams {
    pub agent_id: String,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub cwd: Option<String>,
}

/// `attach` 응답 — `script`는 요청한 셸에서 그대로 eval할 POSIX 스크립트다.
/// `mode`는 새 논리 세션이면 "new", 앱 안 PTY 세션에 합류했으면 "bind".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachResult {
    pub session_id: String,
    pub mode: String,
    pub script: String,
}

/// `detach` 파라미터.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetachParams {
    pub agent_id: String,
}

/// `detach` 응답 — 실제로 붙어 있던 외부 세션을 끊었는지(없었으면 false).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetachResult {
    pub detached: bool,
}

/// `send` 파라미터 — `data`는 세션 stdin에 그대로 주입된다(개행 포함 여부는
/// 클라이언트가 `--enter`로 결정).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendParams {
    pub agent_id: String,
    pub data: String,
}

/// `clear` 파라미터 — ids 미지정이면 해당 세션 알림 전체 클리어.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearParams {
    pub agent_id: String,
    #[serde(default)]
    pub ids: Option<Vec<String>>,
}

/// `list` 응답의 한 항목 — 프로필(디스크)과 실행 중 세션 상태(레지스트리)의 병합.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListEntry {
    pub agent_id: String,
    pub name: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// 실행 중 세션이 있으면 그 상태("running"/"starting"/…), 없으면 null.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// `ping` 응답 — 연결·인증 확인용(버전과 세션 수).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PingResult {
    pub app_version: String,
    pub agent_count: usize,
    pub running_count: usize,
}
