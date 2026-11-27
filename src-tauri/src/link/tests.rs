// src-tauri/src/link/tests.rs
//
// 두 앱(호스트·클라이언트 LinkState)을 한 프로세스에 띄워 실제 LAN 자체 서명 HTTPS(webdash/lan_tls.rs)와
// WebSocket으로 페어링·연결·재연결·버전 거절·해제를 확인한다.
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

use super::protocol::{Down, PairRequest, Up};
use super::*;
use crate::webdash::{lan_tls, tls};

fn temp(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("link-{tag}-{}", uuid::Uuid::new_v4()))
}

struct Host {
    state: Arc<LinkState>,
    dir: PathBuf,
    fingerprint: String,
    port: u16,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    events: Arc<Mutex<Vec<(String, String, serde_json::Value)>>>,
}

impl Host {
    fn new() -> Self {
        let dir = temp("host");
        let state = LinkState::new(Some(dir.clone()), Arc::new(|| "host-mac".to_string()), Arc::new(fake::NoLinkEvents));
        state.configure(LinkConfig { mode: LinkMode::Host, host_address: String::new() }).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        state.host.set_event_handler(Arc::new(move |device: &str, kind: &str, data: &serde_json::Value| {
            sink.lock().unwrap().push((device.to_string(), kind.to_string(), data.clone()));
        }));
        let fingerprint = lan_tls::prepare(&dir, &[Ipv4Addr::LOCALHOST]).unwrap().fingerprint;
        Self { state, dir, fingerprint, port: 0, stop: None, events }
    }

    /// 웹 상태판과 같은 TLS 리스너로 연동 라우트를 띄운다. 같은 포트로 다시 띄울 수 있다.
    async fn serve(&mut self) {
        let lan = lan_tls::prepare(&self.dir, &[Ipv4Addr::LOCALHOST]).unwrap();
        let listener = loop {
            match tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, self.port)).await {
                Ok(l) => break l,
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        };
        self.port = listener.local_addr().unwrap().port();
        let fp = self.fingerprint.clone();
        let router = host::router(self.state.host.clone(), Arc::new(move || Some(fp.clone())));
        self.stop = Some(tls::serve_listener(listener, router, "127.0.0.1".into(), Arc::new(tls::SwapCert::new(lan.key))));
    }

    fn halt(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        // 웹 상태판 `stop`과 같이.
        self.state.host.disconnect_all();
    }

    fn address(&self) -> String {
        format!("https://127.0.0.1:{}", self.port)
    }

    fn code(&self) -> String {
        self.state.host.start_pairing().unwrap().code
    }

    fn devices(&self) -> Vec<LinkDevice> {
        self.state.host.status().devices
    }
}

fn client(dir: &Path) -> Arc<LinkState> {
    let state = LinkState::new(Some(dir.to_path_buf()), Arc::new(|| "client-pc".to_string()), Arc::new(fake::NoLinkEvents));
    state.configure(LinkConfig { mode: LinkMode::Client, host_address: String::new() }).unwrap();
    state
}

async fn until(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..300 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

fn client_state(c: &LinkState) -> LinkClientStatus {
    c.status().client.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairs_over_lan_tls_reconnects_as_the_same_device_and_delivers_buffered_events() {
    let mut host = Host::new();
    host.serve().await;
    let dir = temp("client");
    let c = client(&dir);
    let status = c.pair(&host.address(), &host.code()).await.unwrap();
    let cs = status.client.unwrap();
    assert!(cs.paired);
    assert_eq!(cs.host_name.as_deref(), Some("host-mac"));
    assert_eq!(cs.pinned_fingerprint.as_deref(), Some(host.fingerprint.as_str()), "페어링 때 본 호스트 CA를 고정한다");
    assert_eq!(c.config().host_address, host.address());
    assert!(host.state.host.status().pairing.is_none(), "코드는 한 번만 쓴다");

    // 붙기 전에 쌓은 것도 붙으면 순서대로 간다.
    c.client.enqueue("report", json!({"n": 1}), None);
    tokio::spawn(c.client.clone().run());
    until("connected", || client_state(&c).state == LinkClientState::Connected).await;
    c.client.enqueue("report", json!({"n": 2}), None);
    until("two events", || host.events.lock().unwrap().len() == 2).await;
    until("acked", || client_state(&c).pending == 0).await;
    let devices = host.devices();
    assert_eq!(devices.len(), 1);
    let device_id = devices[0].id.clone();
    assert!(devices[0].connected);
    assert_eq!(devices[0].name, "client-pc");
    assert_eq!(
        host.events.lock().unwrap().iter().map(|(d, _, data)| (d.clone(), data["n"].clone())).collect::<Vec<_>>(),
        [(device_id.clone(), json!(1)), (device_id.clone(), json!(2))]
    );

    // 호스트가 내려간 동안 쌓고, 다시 뜨면 같은 기기로 붙어 쌓인 것만 보낸다.
    host.halt();
    until("dropped", || client_state(&c).state == LinkClientState::Waiting).await;
    assert!(!host.devices()[0].connected);
    c.client.enqueue("report", json!({"n": 3}), None);
    host.serve().await;
    until("reconnected", || client_state(&c).state == LinkClientState::Connected).await;
    until("third event", || host.events.lock().unwrap().len() == 3).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(host.events.lock().unwrap().len(), 3, "이미 받은 것은 다시 오지 않는다");
    assert_eq!(host.devices().len(), 1);
    assert_eq!(host.devices()[0].id, device_id);

    // 호스트가 클라이언트에 일을 보낸다.
    let got = Arc::new(Mutex::new(None));
    let sink = got.clone();
    c.client.set_command_handler(Arc::new(move |kind: &str, data: &serde_json::Value| {
        *sink.lock().unwrap() = Some((kind.to_string(), data.clone()));
    }));
    host.state.host.send(&device_id, "ping", json!({"x": 1})).unwrap();
    until("command", || got.lock().unwrap().is_some()).await;

    // 같은 기계가 다시 페어링해도 기기 id는 그대로, 옛 토큰은 무효.
    let old_token = std::fs::read_to_string(dir.join(client::PAIRING_FILE)).unwrap();
    let again = c.pair(&host.address(), &host.code()).await.unwrap().client.unwrap();
    assert_eq!(again.device_id.as_deref(), Some(device_id.as_str()));
    assert_eq!(host.devices().len(), 1);
    let old: client::StoredPairing = serde_json::from_str(&old_token).unwrap();
    let res = raw_connect(&host, Some(&old.token)).await;
    assert!(matches!(res, Err(401)), "다시 페어링하면 옛 토큰은 거절된다");
    until("reconnected after re-pair", || client_state(&c).state == LinkClientState::Connected).await;

    host.halt();
    let _ = std::fs::remove_dir_all(&host.dir);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoking_a_device_disconnects_it_and_stops_retries() {
    let mut host = Host::new();
    host.serve().await;
    let dir = temp("client");
    let c = client(&dir);
    c.pair(&host.address(), &host.code()).await.unwrap();
    tokio::spawn(c.client.clone().run());
    until("connected", || client_state(&c).state == LinkClientState::Connected).await;
    let id = host.devices()[0].id.clone();
    assert!(host.state.host.revoke(&id));
    until("rejected", || client_state(&c).state == LinkClientState::Rejected).await;
    assert_eq!(client_state(&c).error.as_deref(), Some("link-unauthorized"));
    assert!(host.devices().is_empty());
    host.halt();
    let _ = std::fs::remove_dir_all(&host.dir);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 저수준 WS 연결: 성공하면 소켓, HTTP 거절이면 상태 코드.
async fn raw_connect(
    host: &Host,
    token: Option<&str>,
) -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, u16> {
    let ca = lan_tls::prepare(&host.dir, &[Ipv4Addr::LOCALHOST]).unwrap().ca_pem;
    let config = client_tls::pinned_config(&ca).unwrap();
    let mut req = format!("wss://127.0.0.1:{}/link/ws", host.port).into_client_request().unwrap();
    if let Some(t) = token {
        req.headers_mut().insert("authorization", HeaderValue::from_str(&format!("Bearer {t}")).unwrap());
    }
    let connector = tokio_tungstenite::Connector::Rustls(Arc::new(config));
    match tokio_tungstenite::connect_async_tls_with_config(req, None, false, Some(connector)).await {
        Ok((ws, _)) => Ok(ws),
        Err(tokio_tungstenite::tungstenite::Error::Http(res)) => Err(res.status().as_u16()),
        Err(e) => panic!("connect: {e}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refuses_a_different_protocol_version_with_update_info() {
    let mut host = Host::new();
    host.serve().await;
    let dir = temp("client");
    let c = client(&dir);
    c.pair(&host.address(), &host.code()).await.unwrap();
    let token: client::StoredPairing = serde_json::from_slice(&std::fs::read(dir.join(client::PAIRING_FILE)).unwrap()).unwrap();

    assert!(matches!(raw_connect(&host, None).await, Err(401)), "토큰 없이는 올리지 않는다");
    let mut ws = raw_connect(&host, Some(&token.token)).await.unwrap();
    let hello = Up::Hello {
        protocol: protocol::PROTOCOL + 1,
        app_version: "99.0.0".into(),
        machine_id: "m".into(),
        name: "n".into(),
        outbox_epoch: "e".into(),
    };
    ws.send(Message::Text(serde_json::to_string(&hello).unwrap())).await.unwrap();
    let reply = loop {
        match ws.next().await.unwrap().unwrap() {
            Message::Text(t) => break serde_json::from_str::<Down>(&t).unwrap(),
            _ => continue,
        }
    };
    assert_eq!(
        reply,
        Down::Reject {
            error: "link-protocol-mismatch".into(),
            host_protocol: protocol::PROTOCOL,
            host_app_version: protocol::app_version().into(),
        }
    );
    assert!(!host.devices()[0].connected, "거절한 연결은 붙은 기기로 치지 않는다");

    // 페어링도 버전이 다르면 거절하고 호스트 버전을 알려 준다.
    let code = host.code();
    let req = PairRequest {
        protocol: protocol::PROTOCOL + 1,
        app_version: "99.0.0".into(),
        machine_id: "other".into(),
        name: "n".into(),
        nonce: "x".into(),
        pin: None,
        proof: protocol::client_proof(&code, "x", None, "other"),
    };
    let (status, refusal) = host.state.host.pair(req, Some(host.fingerprint.clone())).await.unwrap_err();
    assert_eq!(status.as_u16(), 409);
    assert_eq!(refusal.host_protocol, Some(protocol::PROTOCOL));
    host.halt();
    let _ = std::fs::remove_dir_all(&host.dir);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wrong_codes_and_foreign_certificates_are_refused() {
    let host = Host::new();
    let code = host.code();
    let pair = |code: &str, pin: Option<&str>| PairRequest {
        protocol: protocol::PROTOCOL,
        app_version: "1".into(),
        machine_id: "m1".into(),
        name: "n".into(),
        nonce: "nonce".into(),
        pin: pin.map(str::to_string),
        proof: protocol::client_proof(code, "nonce", pin, "m1"),
    };
    let own = Some(host.fingerprint.clone());
    for _ in 0..4 {
        let (_, r) = host.state.host.pair(pair("WRONGCOD", own.as_deref()), own.clone()).await.unwrap_err();
        assert_eq!(r.error, "link-pair-code-invalid");
    }
    assert!(host.state.host.status().pairing.is_some(), "몇 번은 틀려도 코드는 남는다");
    let (_, r) = host.state.host.pair(pair("WRONGCOD", own.as_deref()), own.clone()).await.unwrap_err();
    assert_eq!(r.error, "link-pair-code-invalid");
    assert!(host.state.host.status().pairing.is_none(), "다섯 번 틀리면 코드를 버린다");
    let (_, r) = host.state.host.pair(pair(&code, own.as_deref()), own.clone()).await.unwrap_err();
    assert_eq!(r.error, "link-pair-no-code");

    // 코드는 맞지만 클라이언트가 본 인증서가 이 호스트 것이 아니다(가운데서 바꿔치기).
    let code = host.code();
    let (_, r) = host.state.host.pair(pair(&code, Some("EV:IL")), own.clone()).await.unwrap_err();
    assert_eq!(r.error, "link-tls-pin-mismatch");
    assert!(host.state.host.status().pairing.is_none());

    // 호스트 모드가 아니면 받지 않는다.
    let code = host.code();
    host.state.configure(LinkConfig::default()).unwrap();
    let (status, _) = host.state.host.pair(pair(&code, None), own).await.unwrap_err();
    assert_eq!(status.as_u16(), 404);
    let _ = std::fs::remove_dir_all(&host.dir);
}

#[tokio::test]
async fn a_client_refuses_a_host_that_cannot_prove_the_code() {
    // 아무 요청이나 받아 주는 가짜 호스트.
    async fn fake_pair(axum::Json(req): axum::Json<PairRequest>) -> axum::Json<serde_json::Value> {
        crate::httpapi::ok(protocol::PairResponse {
            device_id: "d-1".into(),
            token: "t".into(),
            host_name: "evil".into(),
            host_machine_id: "evil".into(),
            host_app_version: "1".into(),
            protocol: req.protocol,
            proof: "00".into(),
        })
    }
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = axum::Router::new().route("/link/pair", axum::routing::post(fake_pair));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await;
    });
    let dir = temp("client");
    let c = client(&dir);
    let err = c.pair(&format!("http://127.0.0.1:{port}"), "ABCD-EFGH").await.unwrap_err();
    assert_eq!(err, "link-pair-host-unverified");
    assert!(!client_state(&c).paired);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn config_reads_defaults_and_rejects_bad_addresses() {
    let c: LinkConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(c, LinkConfig { mode: LinkMode::Off, host_address: String::new() });
    let state = LinkState::new(None, Arc::new(|| "x".to_string()), Arc::new(fake::NoLinkEvents));
    let err = state.configure(LinkConfig { mode: LinkMode::Client, host_address: "http://192.168.0.2".into() }).unwrap_err();
    assert_eq!(err, "link-address-insecure");
    let status = state.configure(LinkConfig { mode: LinkMode::Client, host_address: "192.168.0.2".into() }).unwrap();
    assert_eq!(status.config.host_address, "https://192.168.0.2:7788");
    assert!(status.host.is_none() && status.client.is_some(), "모드는 하나만 — 클라이언트면 호스트 상태가 없다");
    let status = state.configure(LinkConfig { mode: LinkMode::Host, ..status.config }).unwrap();
    assert!(status.host.is_some() && status.client.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_snapshots_reach_the_host_view_and_host_requests_get_answers() {
    let mut host = Host::new();
    // 실제 앱처럼 받은 것을 원격 보기로 넘긴다(시험용 기록 처리기 대신).
    let remote = Arc::downgrade(&host.state.remote);
    host.state.host.set_event_handler(Arc::new(move |device: &str, kind: &str, data: &serde_json::Value| {
        if let Some(r) = remote.upgrade() {
            r.ingest(device, kind, data);
        }
    }));
    host.serve().await;
    let dir = temp("client");
    let c = client(&dir);
    c.pair(&host.address(), &host.code()).await.unwrap();
    c.client.set_request_handler(Arc::new(|kind, data| {
        Box::pin(async move {
            match kind.as_str() {
                "log" => Ok(json!({ "epoch": 1, "last": 0, "attached": true, "items": [], "asked": data["agentId"] })),
                _ => Err("link-unsupported".to_string()),
            }
        })
    }));
    assert!(!c.client.send_live("agents", json!({})), "붙기 전에는 버린다");
    tokio::spawn(c.client.clone().run());
    until("connected", || client_state(&c).state == LinkClientState::Connected).await;
    let device = host.devices()[0].id.clone();
    until("host sees connection", || host.state.host.is_connected(&device)).await;

    let snap = json!({ "now": now_ms(), "outputAt": {}, "characters": [{ "agentId": "a1", "name": "Ada", "role": "",
        "session": { "sessionId": "s", "state": "running" }, "asks": [] }] });
    assert!(c.client.send_live("agents", snap));
    until("snapshot", || host.state.remote.office_view().machines.first().is_some_and(|m| m.online && m.characters.len() == 1)).await;
    let web = host.state.remote.web_view();
    assert_eq!(web.characters[0]["agentId"], json!(format!("{device}:a1")));
    assert_eq!(web.characters[0]["machine"]["name"], json!("client-pc"));
    assert_eq!(client_state(&c).pending, 0, "스냅숏은 outbox에 쌓지 않는다");

    let (dev, agent) = host.state.remote.resolve(&format!("{device}:a1")).unwrap();
    let page = host.state.remote.request(&dev, "log", json!({ "agentId": agent })).await.unwrap();
    assert_eq!(page["asked"], json!("a1"));
    let err = host.state.remote.request(&dev, "nope", json!({ "agentId": agent })).await.unwrap_err();
    assert_eq!(err, "link-unsupported");

    // 끊기면 그 머신은 꺼진 모습이 되고, 물음은 바로 실패한다.
    host.halt();
    until("offline", || !host.state.remote.office_view().machines[0].online).await;
    let err = host.state.remote.request(&dev, "log", json!({ "agentId": "a1" })).await.unwrap_err();
    assert_eq!(err, "link-device-offline");
    assert_eq!(host.state.remote.web_view().characters[0]["name"], json!("Ada"), "끊겨도 명단은 남는다");
}

#[tokio::test]
async fn host_requests_time_out_and_ignore_replies_from_other_devices() {
    let host = LinkHost::new(None, "h".into(), Arc::new(|| "Host".into()), Arc::new(|| {}));
    host.set_enabled(true);
    let a = host.add_test_device("m-a", "A");
    let b = host.add_test_device("m-b", "B");
    let mut rx = host.set_test_connected(&a, true).unwrap();
    let host = Arc::new(host);
    let h = host.clone();
    let a2 = a.clone();
    let ask = tokio::spawn(async move { h.request(&a2, "log", json!({}), Duration::from_millis(300)).await });
    let Some(Down::Request { id, kind, .. }) = rx.recv().await else { panic!("no request") };
    assert_eq!(kind, "log");
    // 다른 기기가 같은 id로 답해도 받지 않는다.
    host.test_reply(&b, id, Ok(json!("forged")));
    assert_eq!(ask.await.unwrap().unwrap_err(), "link-request-timeout");
    assert_eq!(host.request(&b, "log", json!({}), Duration::from_millis(50)).await.unwrap_err(), "link-device-offline");
}
