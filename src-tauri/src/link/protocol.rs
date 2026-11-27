// src-tauri/src/link/protocol.rs
//
// 머신 연동의 말: 프로토콜 버전, WebSocket 메시지 모양, 페어링 코드와 증명(HMAC).
// 소켓·파일을 만지지 않는 순수 부분만 둔다.
//
// 페어링 증명: 클라이언트는 코드를 그대로 보내지 않고 `HMAC(코드, 'pair'·nonce·pin·machineId)`를 보낸다.
// `pin`은 클라이언트가 TLS에서 본 CA 지문이다(LAN 자체 서명일 때). 가운데서 다른 인증서를 내민 쪽은
// 클라이언트가 본 지문과 호스트 지문이 달라 증명을 다시 만들어야 하는데, 그러려면 코드를 알아야 한다.
// 호스트도 `HMAC(코드, 'host'·nonce·pin·deviceId)`로 코드를 안다는 걸 증명해, 아무 요청이나 받아 주는
// 가짜 호스트에 붙지 않게 한다. 코드는 40비트이고 몇 분만 유효하며 틀리면 몇 번 만에 버린다.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 연동 프로토콜 버전. 앱 버전과 별개로, 메시지 모양이나 의미가 바뀌어 옛 앱과 말이 안 통할 때만 올린다.
/// 핸드셰이크에서 정확히 같아야 붙는다.
pub const PROTOCOL: u32 = 1;

/// 페어링 코드 글자(헷갈리는 0·O·1·I를 뺀 32자 → 글자당 5비트).
const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
pub const CODE_LEN: usize = 8;

pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 새 페어링 코드(8자, `ABCD-EFGH`처럼 보일 때는 가운데에 `-`를 넣는다).
pub fn new_code() -> String {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    bytes.iter().take(CODE_LEN).map(|b| CODE_ALPHABET[(*b as usize) % 32] as char).collect()
}

/// 사람이 친 코드를 비교할 모양으로(대문자, 구분 문자 빼기).
pub fn normalize_code(input: &str) -> String {
    input.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect()
}

/// 기기 토큰(호스트가 발급, 클라이언트가 보관).
pub fn new_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

pub fn sha256_hex(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// HMAC-SHA256(RFC 2104).
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner);
    outer.finalize().into()
}

fn proof(code: &str, parts: &[&str]) -> String {
    hex(&hmac_sha256(normalize_code(code).as_bytes(), parts.join("\n").as_bytes()))
}

/// 클라이언트가 코드를 안다는 증명.
pub fn client_proof(code: &str, nonce: &str, pin: Option<&str>, machine_id: &str) -> String {
    proof(code, &["pair", nonce, pin.unwrap_or(""), machine_id])
}

/// 호스트가 코드를 안다는 증명(발급한 기기 id까지 묶는다).
pub fn host_proof(code: &str, nonce: &str, pin: Option<&str>, device_id: &str) -> String {
    proof(code, &["host", nonce, pin.unwrap_or(""), device_id])
}

// ── 페어링 HTTP(`POST /link/pair`) ───────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
    pub protocol: u32,
    pub app_version: String,
    pub machine_id: String,
    pub name: String,
    pub nonce: String,
    /// 클라이언트가 본 LAN CA 지문. 공인 인증서·평문(루프백·tailnet)이면 없다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    pub proof: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairResponse {
    pub device_id: String,
    pub token: String,
    pub host_name: String,
    pub host_machine_id: String,
    pub host_app_version: String,
    pub protocol: u32,
    pub proof: String,
}

/// 거절 응답의 몸(`{ok:false, error, hostProtocol?, hostAppVersion?}`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Refusal {
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_protocol: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_app_version: Option<String>,
}

// ── WebSocket 메시지(텍스트 프레임 하나에 JSON 하나) ─────────────────

/// 클라이언트 → 호스트.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum Up {
    /// 연결하자마자 보낸다. 버전이 다르면 호스트가 `Reject`로 답하고 닫는다.
    #[serde(rename_all = "camelCase")]
    Hello {
        protocol: u32,
        app_version: String,
        machine_id: String,
        name: String,
        /// 보낼 상자(outbox)가 새로 생길 때마다 바뀌는 값. 호스트는 이 값이 같을 때만 받은 seq를 기억한다.
        outbox_epoch: String,
    },
    /// 쌓아 둔 이벤트·보고 하나. 호스트는 처리한 뒤 `Ack`로 답한다.
    Event { seq: u64, kind: String, data: serde_json::Value },
}

/// 호스트 → 클라이언트.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum Down {
    #[serde(rename_all = "camelCase")]
    Welcome {
        protocol: u32,
        host_app_version: String,
        host_name: String,
        host_machine_id: String,
        device_id: String,
        /// 같은 outbox에서 이미 받은 마지막 seq. 클라이언트는 그 이하를 버린다.
        acked: u64,
    },
    #[serde(rename_all = "camelCase")]
    Reject {
        error: String,
        host_protocol: u32,
        host_app_version: String,
    },
    Ack { seq: u64 },
    /// 호스트가 보내는 일(다음 단계들의 세션 열기·입력 등). 클라이언트 처리기에 넘긴다.
    Command { kind: String, data: serde_json::Value },
    /// 호스트에서 이 기기를 해제했다. 다시 붙지 않는다.
    Revoked,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc4231_vectors() {
        let h = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(hex(&h), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        // 블록보다 긴 키(case 6).
        let h = hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First");
        assert_eq!(hex(&h), "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
    }

    #[test]
    fn proofs_bind_the_code_pin_and_direction() {
        let code = new_code();
        assert_eq!(code.len(), CODE_LEN);
        assert!(code.bytes().all(|b| CODE_ALPHABET.contains(&b)));
        let typed = format!("{}-{}", &code[..4], &code[4..]).to_lowercase();
        let p = client_proof(&code, "n1", Some("AB:CD"), "m1");
        assert_eq!(client_proof(&typed, "n1", Some("AB:CD"), "m1"), p, "대소문자·구분 문자는 무시한다");
        assert_ne!(client_proof(&code, "n1", Some("EF:00"), "m1"), p, "다른 인증서를 본 클라이언트의 증명은 다르다");
        assert_ne!(client_proof(&code, "n1", None, "m1"), p);
        assert_ne!(host_proof(&code, "n1", Some("AB:CD"), "m1"), p, "방향이 다르면 다른 값");
    }

    #[test]
    fn messages_are_tagged_json() {
        let up = Up::Event { seq: 3, kind: "report".into(), data: serde_json::json!({"x": 1}) };
        let text = serde_json::to_string(&up).unwrap();
        assert_eq!(text, r#"{"t":"event","seq":3,"kind":"report","data":{"x":1}}"#);
        assert_eq!(serde_json::from_str::<Up>(&text).unwrap(), up);
        let down: Down = serde_json::from_str(r#"{"t":"reject","error":"link-protocol-mismatch","hostProtocol":2,"hostAppVersion":"9.0.0"}"#).unwrap();
        assert!(matches!(down, Down::Reject { host_protocol: 2, .. }));
    }
}
