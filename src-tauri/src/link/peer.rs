// src-tauri/src/link/peer.rs
//
// 에이전트끼리 조회·대화(`crate::peer`)의 연동 배선. 클라이언트끼리는 호스트가 중계한다.
// - 클라이언트 → 호스트(`Up::Live` `peer-req {rid, toMachine, op, data}`): 이 머신 에이전트가 다른 머신에 시킨 일.
//   호스트는 보낸 기기의 머신으로 보낸 이를 덮어쓴 뒤(사칭 막기) 그 머신에 넘기고, 답을 `Command`
//   `peer-res {rid, data|error}`로 돌려준다. `toMachine`이 비고 `op`가 `roster-all`이면 호스트가 연동 전체 명단을
//   모아 준다(묻는 머신은 뺀다).
// - 호스트 → 클라이언트 물음(`Request` `peer {op, data}`): 그 머신에서 할 일(`PeerState::exec_local`).
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use super::LinkState;

pub const LIVE_REQ: &str = "peer-req";
pub const CMD_RES: &str = "peer-res";
pub const REQ_PEER: &str = "peer";

type Pending = HashMap<u64, tokio::sync::oneshot::Sender<Result<Value, String>>>;

fn pending() -> &'static Mutex<Pending> {
    static P: OnceLock<Mutex<Pending>> = OnceLock::new();
    P.get_or_init(Default::default)
}

static NEXT: AtomicU64 = AtomicU64::new(1);

/// 클라이언트: 호스트에 일을 넘기고 답을 기다린다.
pub async fn client_rpc(link: &LinkState, body: Value) -> Result<Value, String> {
    let rid = NEXT.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = tokio::sync::oneshot::channel();
    pending().lock().unwrap().insert(rid, tx);
    let mut body = body;
    body["rid"] = json!(rid);
    if link.client.connection().is_none() || !link.client.send_live(LIVE_REQ, body) {
        pending().lock().unwrap().remove(&rid);
        let host = link.client.status().host_name.unwrap_or_default();
        return Err(format!("peer-machine-offline: {host}"));
    }
    // 호스트가 다른 머신을 기다리는 시간보다 조금 더.
    match tokio::time::timeout(crate::peer::ROUTE_TIMEOUT + std::time::Duration::from_secs(10), rx).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(_)) => Err("peer-timeout".into()),
        Err(_) => {
            pending().lock().unwrap().remove(&rid);
            Err("peer-timeout".into())
        }
    }
}

/// 클라이언트: 호스트가 보낸 일(`peer-res`).
pub fn client_command(kind: &str, data: &Value) {
    if kind != CMD_RES {
        return;
    }
    let Some(rid) = data.get("rid").and_then(Value::as_u64) else { return };
    let Some(tx) = pending().lock().unwrap().remove(&rid) else { return };
    let answer = match data.get("error").and_then(Value::as_str) {
        Some(e) => Err(e.to_string()),
        None => Ok(data.get("data").cloned().unwrap_or(Value::Null)),
    };
    let _ = tx.send(answer);
}

/// 클라이언트: 호스트의 물음(`peer`)에 답한다. 이 종류가 아니면 None.
pub async fn answer(kind: &str, data: &Value) -> Option<Result<Value, String>> {
    if kind != REQ_PEER {
        return None;
    }
    let Some(peer) = crate::peer::current() else { return Some(Err("peer-not-ready".into())) };
    let op = data.get("op").and_then(Value::as_str).unwrap_or("").to_string();
    let body = data.get("data").cloned().unwrap_or(Value::Null);
    Some(peer.exec_local(&op, body).await)
}

/// 보낸 이를 붙은 기기의 머신으로 덮어쓴다(그 머신 에이전트만 그 머신 이름으로 말한다).
pub fn stamp_sender(op: &str, data: &mut Value, machine_id: &str, machine_name: &str) {
    let slot = match op {
        "deliver" => data.get_mut("thread").and_then(|t| t.get_mut("from")),
        "reply" => data.get_mut("from"),
        _ => None,
    };
    if let Some(from) = slot.filter(|v| v.is_object()) {
        from["machineId"] = json!(machine_id);
        from["machineName"] = json!(machine_name);
    }
}

/// 호스트: 붙은 기기가 올린 `peer-req`.
pub fn host_event(link: &std::sync::Arc<LinkState>, device: &str, kind: &str, data: &Value) {
    if kind != LIVE_REQ {
        return;
    }
    let Some(rid) = data.get("rid").and_then(Value::as_u64) else { return };
    let Some(d) = link.host.status().devices.into_iter().find(|d| d.id == device) else { return };
    let to = data.get("toMachine").and_then(Value::as_str).unwrap_or("").to_string();
    let op = data.get("op").and_then(Value::as_str).unwrap_or("").to_string();
    let mut body = data.get("data").cloned().unwrap_or(Value::Null);
    stamp_sender(&op, &mut body, &d.machine_id, &d.name);
    let link = link.clone();
    let device = device.to_string();
    tauri::async_runtime::spawn(async move {
        let result = match crate::peer::current() {
            None => Err("peer-not-ready".to_string()),
            Some(peer) if op == "roster-all" && to.is_empty() => Ok(json!(peer.roster_all(Some(&d.machine_id)).await)),
            // 클라이언트는 자기 머신 일을 호스트에 넘기지 않는다.
            Some(_) if to == d.machine_id || to.is_empty() => Err("bad-request".to_string()),
            Some(peer) => peer.route(&to, &op, body).await,
        };
        let res = match result {
            Ok(v) => json!({ "rid": rid, "data": v }),
            Err(e) => json!({ "rid": rid, "error": e }),
        };
        let _ = link.host.send(&device, CMD_RES, res);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_overwrites_only_the_sender() {
        let mut d = json!({ "thread": { "id": "p-1", "from": { "machineId": "fake", "machineName": "x", "agentId": "a" }, "to": { "machineId": "m2" } } });
        stamp_sender("deliver", &mut d, "m1", "pc");
        assert_eq!(d["thread"]["from"]["machineId"], "m1");
        assert_eq!(d["thread"]["from"]["machineName"], "pc");
        assert_eq!(d["thread"]["from"]["agentId"], "a");
        assert_eq!(d["thread"]["to"]["machineId"], "m2");
        let mut r = json!({ "id": "p-1", "from": { "machineId": "fake", "machineName": "x" } });
        stamp_sender("reply", &mut r, "m1", "pc");
        assert_eq!(r["from"]["machineId"], "m1");
        let mut e = json!({ "agentId": "a" });
        stamp_sender("exec", &mut e, "m1", "pc");
        assert_eq!(e, json!({ "agentId": "a" }));
    }

    #[tokio::test]
    async fn responses_resolve_pending_calls() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        pending().lock().unwrap().insert(9_000_001, tx);
        client_command(CMD_RES, &json!({ "rid": 9_000_001, "data": { "ok": 1 } }));
        assert_eq!(rx.await.unwrap(), Ok(json!({ "ok": 1 })));
        let (tx, rx) = tokio::sync::oneshot::channel();
        pending().lock().unwrap().insert(9_000_002, tx);
        client_command(CMD_RES, &json!({ "rid": 9_000_002, "error": "peer-off: pc" }));
        assert_eq!(rx.await.unwrap(), Err("peer-off: pc".into()));
    }
}
