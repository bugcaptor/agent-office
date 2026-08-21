// src-tauri/src/peer/protocol.rs
//
// 웹 원격(docs/web-remote-design.md)의 와이어 계약.
// 페어링은 HTTP(POST /peer/v1/pair/*), 세션 중계는 WebSocket(GET /peer/v1/ws).
// 프레이밍은 WS 텍스트 프레임 안의 JSON(camelCase)이고, 확장은 additive-only다
// (브로커 v2의 협상 관례 준용 — 새 필드는 전부 `#[serde(default)]`).

use serde::{Deserialize, Serialize};

/// 와이어 프로토콜 버전. 호환을 깨지 않는 필드 추가에는 올리지 않는다.
pub const PEER_PROTO_VERSION: u32 = 1;

/// WS 업그레이드/HTTP 요청에 붙는 인증 헤더.
pub const PEER_TOKEN_HEADER: &str = "x-agent-office-peer-token";

/// 기본 수신 포트. 수동 `host:port` 입력이 곧 디스커버리라(§결정 5) 고정값이
/// 필요하다 — 점유 중이면 +1씩 스캔하고 실제 포트를 설정 UI에 표시한다.
pub const DEFAULT_PEER_PORT: u16 = 47800;

/// 호스트가 발급한 peer 토큰 목록(0600).
pub const PEER_TOKENS_FILE: &str = "peer-tokens.json";
/// 웹 RPC 실패 사유. 폐쇄 집합 — 클라이언트가 문자열로 분기한다.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcError {
    pub code: String,
    pub message: String,
}

impl RpcError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
    /// allowlist 테이블에 없는 커맨드 — **존재하는 커맨드라도 미등재면 이것**이다.
    pub fn unknown_cmd(cmd: &str) -> Self {
        Self::new("unknownCmd", format!("허용되지 않은 명령입니다: {cmd}"))
    }
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new("forbidden", message)
    }
    pub fn bad_args(message: impl Into<String>) -> Self {
        Self::new("badArgs", message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("notFound", message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message)
    }
}

/// 뷰어에게 허용할 권한. kill/dispose/resize/생성은 이 체계에 아예 없다(§결정 6·7).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PeerPermission {
    /// 보기만. `input` 메시지는 서버에서 거부된다.
    #[default]
    ReadOnly,
    /// 입력 가능(공동 조작). 그래도 리사이즈·종료는 불가.
    Input,
}

impl PeerPermission {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "readOnly",
            Self::Input => "input",
        }
    }
    pub fn allows_input(self) -> bool {
        matches!(self, Self::Input)
    }
}

// ── 페어링(HTTP) ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairStartRequest {
    /// 클라이언트 쪽 표시 이름(호스트 승인 다이얼로그에 그대로 보인다).
    #[serde(default)]
    pub viewer_name: String,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub proto_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairStartResponse {
    pub pairing_id: String,
    /// 코드 유효 시간(초).
    pub expires_in: u64,
    pub host_name: String,
    pub proto_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairCompleteRequest {
    pub pairing_id: String,
    /// 호스트 화면에 표시된 6자리 코드.
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairCompleteResponse {
    pub peer_token: String,
    pub peer_id: String,
    pub host_name: String,
    pub permission: PeerPermission,
    pub proto_version: u32,
}

// ── 세션 중계(WebSocket) ──────────────────────────────────────────────

/// 뷰어에 보여줄 원격 캐릭터 메타. 프로필 소유권은 호스트에 있고 뷰어는
/// 읽기 캐시다(§결정 4).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerAgent {
    pub agent_id: String,
    pub name: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub seed: String,
    #[serde(default)]
    pub cwd: Option<String>,
    /// 세션 상태 문자열("running"/"exited"/…). 세션이 없으면 None.
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub cols: u16,
    #[serde(default)]
    pub rows: u16,
}

/// 출력 청크(호스트의 `OutputChunk`를 그대로 옮긴 것 + 절대 오프셋).
/// `offset`은 **이 청크가 시작하는 절대 스트림 위치**다 — 뷰어는 마지막으로
/// 적용한 청크의 `offset + bytes`를 재접속 시 `lastOffset`으로 되돌려준다.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerOutput {
    pub agent_id: String,
    pub session_id: String,
    pub seq: u64,
    pub offset: u64,
    pub data: String,
    pub bytes: u64,
}

// `rename_all`은 **variant 이름**에만 걸린다 — 필드까지 camelCase로 내보내려면
// `rename_all_fields`가 따로 필요하다(없으면 와이어에 `agent_id`가 나가 뷰어
// 파싱이 깨진다).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HostMsg {
    Hello {
        host_name: String,
        app_version: String,
        proto_version: u32,
        permission: PeerPermission,
        peer_id: String,
    },
    /// 공유 중인 캐릭터 전량. 변경 시 전량 재송한다(부분 갱신 없음 — 뷰어
    /// 캐시 무효화 규칙을 단순하게 유지).
    Agents {
        agents: Vec<PeerAgent>,
    },
    /// attach 응답. `snapshot`이 Some이면 화면 이미지를 먼저 복원하고,
    /// 이후 도착하는 `output`을 이어 붙인다. `baseOffset`은 복원 직후의 절대
    /// 오프셋이다.
    Restore {
        agent_id: String,
        #[serde(default)]
        snapshot: Option<String>,
        base_offset: u64,
        #[serde(default)]
        cols: u16,
        #[serde(default)]
        rows: u16,
        #[serde(default)]
        session_id: Option<String>,
    },
    Output(PeerOutput),
    /// 호스트의 `activity-event` 원본 JSON(agentId만 뷰어가 다시 쓴다).
    Activity {
        agent_id: String,
        payload: serde_json::Value,
    },
    SessionState {
        agent_id: String,
        payload: serde_json::Value,
    },
    Notification {
        agent_id: String,
        payload: serde_json::Value,
    },
    NotificationCleared {
        agent_id: String,
        ids: Vec<String>,
    },
    Resized {
        agent_id: String,
        cols: u16,
        rows: u16,
    },
    Pong,
    /// 웹 RPC 응답(웹 호스팅 #7m). `ok=false`면 `error`가 채워진다.
    RpcResult {
        id: u64,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<RpcError>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ViewerMsg {
    Attach {
        agent_id: String,
        #[serde(default)]
        last_offset: Option<u64>,
    },
    Detach {
        agent_id: String,
    },
    Input {
        agent_id: String,
        data: String,
    },
    /// 웹 클라이언트의 요청/응답 상관 RPC(웹 호스팅 #7m). 커맨드 80개를
    /// 라우트로 펼치는 대신 이 프레임 하나에 얹고, **allowlist 테이블에 없는
    /// `cmd`는 무조건 거부**한다.
    Rpc {
        id: u64,
        cmd: String,
        #[serde(default)]
        args: serde_json::Value,
    },
    Ping,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_msg_wire_shape_is_tagged() {
        let json = serde_json::to_string(&ViewerMsg::Attach {
            agent_id: "ada".into(),
            last_offset: Some(42),
        })
        .unwrap();
        assert!(json.contains("\"type\":\"attach\""));
        assert!(json.contains("\"lastOffset\":42"));
        let back: ViewerMsg = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, ViewerMsg::Attach { last_offset: Some(42), .. }));
    }

    #[test]
    fn attach_without_last_offset_parses() {
        let back: ViewerMsg =
            serde_json::from_str(r#"{"type":"attach","agentId":"ada"}"#).unwrap();
        assert!(matches!(back, ViewerMsg::Attach { last_offset: None, .. }));
    }

    #[test]
    fn permission_gates_input() {
        assert!(!PeerPermission::ReadOnly.allows_input());
        assert!(PeerPermission::Input.allows_input());
        assert_eq!(
            serde_json::to_string(&PeerPermission::ReadOnly).unwrap(),
            "\"readOnly\""
        );
    }
}
