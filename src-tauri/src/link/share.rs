// src-tauri/src/link/share.rs
//
// 클라이언트 쪽: 호스트에 붙어 있는 동안 이 앱 캐릭터들의 지금 모습(웹 상태판 `/api/state`의
// `characters`와 같은 것 + 오피스 씬용 외형)을 몇 초마다 올리고(`Up::Live` 종류 `agents`, 바뀌었을 때나
// 다시 붙었을 때, 아니면 가끔), 호스트가 묻는 대화 로그·초상·이미지·파일에 답한다.
//
// 터미널 화면(PTY)은 보내지 않는다 — 대화 보기 수준(전사·권한 요청·대기열·상태)까지만이다.
// 호스트가 시키는 조작도 같은 물음(`Request`)으로 온다: `op`(웹 상태판 `/api/agents/:agent/:op`와 같은 일 —
// 메시지·대기열 빼기·중단·CLI 실행·세션 열기(재시작)·이어하기·닫기)와 `ask`(권한 요청·질문 답). 이 머신의
// 웹 상태판 조작과 같은 함수를 부르므로 이 머신 앱에도 웹 조작처럼 알림이 뜬다. 권한 등급은 없다(페어링 = 전부 허용).
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Value};

use super::remote::KIND_AGENTS;
use super::{LinkMode, LinkState};
use crate::control::ControlContext;
use crate::webdash::WebDashState;

/// 바뀐 것이 있는지 보는 간격.
const TICK: Duration = Duration::from_secs(2);
/// 바뀐 것이 없어도 이만큼마다 다시 올린다(마지막 출력 시각이 흐르게).
const REFRESH: Duration = Duration::from_secs(10);
/// 한 번에 보내는 이미지·파일 상한(base64로 부풀어도 WebSocket 메시지 상한 안에 들게).
const MAX_BLOB: usize = 16 * 1024 * 1024;

/// 요청 처리기와 올리기 루프를 건다(앱 시작 때 한 번).
pub fn install(link: &Arc<LinkState>, ctl: Arc<ControlContext>, dash: Arc<WebDashState>) {
    {
        let (ctl, dash) = (ctl.clone(), dash.clone());
        link.client.set_request_handler(Arc::new(move |kind, data| {
            let (ctl, dash) = (ctl.clone(), dash.clone());
            Box::pin(async move { answer(&ctl, &dash, &kind, data).await })
        }));
    }
    let link = Arc::downgrade(link);
    tauri::async_runtime::spawn(async move {
        let mut last: Option<(u64, u64, Instant)> = None;
        loop {
            tokio::time::sleep(TICK).await;
            let Some(link) = link.upgrade() else { break };
            let Some(conn) = link.client.connection().filter(|_| link.config().mode == LinkMode::Client) else {
                last = None;
                continue;
            };
            let data = snapshot(&ctl, &dash).await;
            let fingerprint = fingerprint(&data["characters"]);
            let due = match last {
                Some((c, f, at)) => c != conn || f != fingerprint || at.elapsed() >= REFRESH,
                None => true,
            };
            if due && link.client.send_live(KIND_AGENTS, data) {
                last = Some((conn, fingerprint, Instant::now()));
            }
        }
    });
}

fn fingerprint(v: &Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.to_string().hash(&mut h);
    h.finish()
}

/// 올릴 스냅숏: `{characters, outputAt, now}`. 캐릭터마다 오피스 씬이 그릴 외형을 붙인다.
async fn snapshot(ctl: &Arc<ControlContext>, dash: &Arc<WebDashState>) -> Value {
    let view = crate::webdash::snapshot::local_view(ctl, dash).await;
    let profiles = ctl.store.load().agents;
    let characters: Vec<Value> = view
        .characters
        .into_iter()
        .map(|mut c| {
            let id = c["agentId"].as_str().unwrap_or("").to_string();
            if let Some(p) = profiles.iter().find(|p| p.id == id) {
                c["appearance"] = json!({ "archetype": p.archetype, "seed": p.seed, "colors": p.colors });
            }
            c
        })
        .collect();
    json!({ "characters": characters, "outputAt": view.output_at, "now": crate::types::now_ms() })
}

fn agent_of(data: &Value) -> Result<String, String> {
    data.get("agentId").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string).ok_or_else(|| "bad-request".into())
}

fn blob(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_BLOB {
        return Err("too-large".into());
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// 호스트의 물음에 답한다.
async fn answer(ctl: &Arc<ControlContext>, dash: &Arc<WebDashState>, kind: &str, data: Value) -> Result<Value, String> {
    if kind == "ask" {
        let id = data.get("askId").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or("bad-request")?.to_string();
        let answer: crate::webdash::AskAnswer = serde_json::from_value(data).map_err(|_| "bad-request".to_string())?;
        crate::webdash::answer_ask_local(ctl.clone(), dash.clone(), &id, answer)?;
        return Ok(json!({}));
    }
    let agent = agent_of(&data)?;
    // 앱 대화 보기(`view: "app"`)는 이미지 끄기와 무관한 수집기를 쓰고, 웹은 이 머신 웹 상태판 설정을 따른다.
    let app = data.get("view").and_then(Value::as_str) == Some("app");
    match kind {
        "log" => {
            let cwd = crate::webdash::snapshot::agent_cwd(ctl, &agent).ok_or("unknown-agent")?;
            let after = data.get("after").and_then(Value::as_u64);
            let epoch = data.get("epoch").and_then(Value::as_u64);
            let tail = data.get("tail").and_then(Value::as_u64).map(|n| n as usize);
            let dash = dash.clone();
            let page = tokio::task::spawn_blocking(move || {
                let logs = if app { &dash.app_logs } else { &dash.logs };
                logs.read(&agent, &cwd, after, epoch)
            })
            .await
            .map_err(|e| e.to_string())?;
            let page = match tail {
                Some(n) => page.keep_last(n),
                None => page,
            };
            serde_json::to_value(page).map_err(|e| e.to_string())
        }
        "portrait" => {
            let encoded = ctl.portrait_store.load(&agent).map_err(|e| e.to_string())?.ok_or("not-found")?;
            Ok(json!({ "data": encoded }))
        }
        "image" => {
            let id = data.get("id").and_then(Value::as_str).ok_or("bad-request")?.to_string();
            let dash = dash.clone();
            let found = tokio::task::spawn_blocking(move || {
                let logs = if app { &dash.app_logs } else { &dash.logs };
                logs.image(&agent, &id)
            })
            .await
            .map_err(|e| e.to_string())?;
            let (media_type, bytes) = found.ok_or("not-found")?;
            Ok(json!({ "mediaType": media_type, "data": blob(&bytes)? }))
        }
        "file" => {
            let num = |k: &str| data.get(k).and_then(Value::as_u64).ok_or_else(|| "bad-request".to_string());
            let (epoch, seq, index) = (num("epoch")?, num("seq")?, num("index")? as usize);
            let dash = dash.clone();
            let found = tokio::task::spawn_blocking(move || {
                let logs = if app { &dash.app_logs } else { &dash.logs };
                logs.file(&agent, epoch, seq, index)
            })
            .await
            .map_err(|e| e.to_string())?;
            let (name, bytes) = found.ok_or("not-found")?;
            Ok(json!({ "name": name, "data": blob(&bytes)? }))
        }
        "asks" => {
            let sid = ctl.manager.session_id_for(&agent);
            Ok(json!(sid.map(|sid| ctl.observer.permissions().list(&sid)).unwrap_or_default()))
        }
        "op" => {
            let op = data.get("op").and_then(Value::as_str).ok_or("bad-request")?.to_string();
            let body: crate::webdash::AgentCommandBody = serde_json::from_value(data).map_err(|_| "bad-request".to_string())?;
            crate::webdash::agent_op(ctl.clone(), dash.clone(), &agent, &op, body).await
        }
        "status" => serde_json::to_value(crate::chat_queue::live_status(ctl, &agent)).map_err(|e| e.to_string()),
        _ => Err("link-unsupported".into()),
    }
}
