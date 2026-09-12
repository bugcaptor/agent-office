//! One authenticated socket owns RPC ordering and reconnects. Input and mutations
//! are never retried: a lost reply may mean the server already executed them.
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    net::TcpStream,
    sync::{mpsc, oneshot},
    time::Instant,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
    MaybeTlsStream, WebSocketStream,
};

pub type Events = Arc<dyn Fn(&str, Value) + Send + Sync>;
pub type Frames = Arc<dyn Fn(Value) -> bool + Send + Sync>;
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Reply = oneshot::Sender<Result<Value, String>>;

pub enum Command {
    Rpc {
        cmd: String,
        args: Value,
        reply: Reply,
    },
    Subscribe {
        agent: String,
        frames: Frames,
    },
    Unsubscribe(String),
    Input {
        agent: String,
        data: String,
    },
    Ack {
        agent: String,
        offset: u64,
        restore_id: Option<String>,
    },
    Stop,
}
struct Subscription {
    frames: Frames,
    offset: Option<u64>,
    sent_offset: u64,
    restore_id: Option<String>,
    session: Option<String>,
}
impl Subscription {
    fn acknowledge(&mut self, offset: u64, restore_id: Option<&str>) {
        if restore_id.is_some()
            && restore_id == self.restore_id.as_deref()
            && offset <= self.sent_offset
        {
            self.offset = Some(offset);
        }
    }
}
struct Pending {
    reply: Reply,
    deadline: Instant,
}

pub struct Connection {
    pub tx: mpsc::Sender<Command>,
    pub task: tauri::async_runtime::JoinHandle<()>,
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Keep credentials out of URLs, redirect handling, and error messages.
pub fn socket_url(input: &str) -> Result<String, String> {
    let mut url = reqwest::Url::parse(input.trim()).map_err(|_| "remote-invalid-url")?;
    let scheme = match url.scheme() {
        "http" | "ws" => "ws",
        "https" | "wss" => "wss",
        _ => return Err("remote-invalid-url".into()),
    };
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("remote-invalid-url".into());
    }
    if url.path() != "/" && url.path() != "/webremote/v1/ws" {
        return Err("remote-invalid-url".into());
    }
    url.set_scheme(scheme).map_err(|_| "remote-invalid-url")?;
    url.set_path("/webremote/v1/ws");
    Ok(url.to_string())
}

async fn open(url: &str, token: &str) -> Result<(Socket, Value), String> {
    let mut request = url
        .into_client_request()
        .map_err(|_| "remote-invalid-url")?;
    let mut header = token
        .parse::<tokio_tungstenite::tungstenite::http::HeaderValue>()
        .map_err(|_| "remote-invalid-token")?;
    header.set_sensitive(true);
    request
        .headers_mut()
        .insert(crate::webremote::protocol::WEB_REMOTE_TOKEN_HEADER, header);
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), connect_async(request))
        .await
        .map_err(|_| "remote-connect-timeout")?
        .map_err(|e| match e {
            tokio_tungstenite::tungstenite::Error::Http(r)
                if r.status().as_u16() == 401 || r.status().as_u16() == 403 =>
            {
                "remote-auth-failed"
            }
            _ => "remote-connect-failed",
        })?;
    let hello = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    let value: Value =
                        serde_json::from_str(&text).map_err(|_| "remote-invalid-protocol")?;
                    if value["type"] == "hello" {
                        return Ok::<_, &str>(value);
                    }
                    return Err("remote-invalid-protocol");
                }
                Some(Ok(Message::Ping(bytes))) => {
                    socket
                        .send(Message::Pong(bytes))
                        .await
                        .map_err(|_| "remote-disconnected")?;
                }
                _ => return Err("remote-disconnected"),
            }
        }
    })
    .await
    .map_err(|_| "remote-connect-timeout")?
    .map_err(str::to_owned)?;
    if hello["protoVersion"].as_u64()
        != Some(crate::webremote::protocol::WEB_REMOTE_PROTO_VERSION as u64)
    {
        return Err("remote-protocol-mismatch".into());
    }
    Ok((socket, hello))
}

pub async fn connect(
    url: String,
    token: String,
    events: Events,
) -> Result<(Connection, Value), String> {
    let url = socket_url(&url)?;
    if token.trim().is_empty() {
        return Err("remote-invalid-token".into());
    }
    let token = token.trim().to_owned();
    let (socket, hello) = open(&url, &token).await?;
    let (tx, rx) = mpsc::channel(256);
    let task = tauri::async_runtime::spawn(run(
        socket,
        url,
        token,
        hello["instanceId"].clone(),
        events,
        rx,
    ));
    Ok((Connection { tx, task }, hello))
}

async fn send(socket: &mut Socket, value: Value) -> Result<(), ()> {
    tokio::time::timeout(
        Duration::from_secs(10),
        socket.send(Message::Text(value.to_string())),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())
}
fn attach(agent: &str, offset: Option<u64>, session: Option<&str>) -> Value {
    json!({"type":"attach","agentId":agent,"lastOffset":offset,"lastSessionId":session})
}

fn connection_state(events: &Events, status: &str) {
    events("remote-connection", json!({"status":status}));
}

async fn run(
    mut socket: Socket,
    url: String,
    token: String,
    mut instance: Value,
    events: Events,
    mut rx: mpsc::Receiver<Command>,
) {
    let mut subscriptions: HashMap<String, Subscription> = HashMap::new();
    let mut pending: HashMap<u64, Pending> = HashMap::new();
    let mut next_id = 1u64;
    'connection: loop {
        connection_state(&events, "connected");
        for (agent, sub) in &subscriptions {
            if send(
                &mut socket,
                attach(agent, sub.offset, sub.session.as_deref()),
            )
            .await
            .is_err()
            {
                break;
            }
        }
        let mut last_seen = Instant::now();
        let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
        loop {
            tokio::select! {
                command = rx.recv() => {
                    match command {
                        None | Some(Command::Stop) => { let _ = socket.close(None).await; break 'connection; }
                        Some(Command::Ack { agent, offset, restore_id }) => {
                            if let Some(s) = subscriptions.get_mut(&agent) { s.acknowledge(offset, restore_id.as_deref()); }
                        }
                        Some(Command::Subscribe { agent, frames }) => {
                            subscriptions.insert(agent.clone(), Subscription { frames, offset: None, sent_offset: 0, restore_id: None, session: None });
                            if send(&mut socket, attach(&agent, None, None)).await.is_err() { break; }
                        }
                        Some(Command::Unsubscribe(agent)) => {
                            subscriptions.remove(&agent);
                            if send(&mut socket, json!({"type":"detach","agentId":agent})).await.is_err() { break; }
                        }
                        Some(Command::Input { agent, data }) => {
                            if send(&mut socket, json!({"type":"input","agentId":agent,"data":data})).await.is_err() { break; }
                        }
                        Some(Command::Rpc { cmd, args, reply }) => {
                            if pending.len() >= 256 { let _ = reply.send(Err("remote-busy".into())); continue; }
                            let id = next_id; next_id += 1;
                            pending.insert(id, Pending { reply, deadline: Instant::now() + Duration::from_secs(30) });
                            if send(&mut socket, json!({"type":"rpc","id":id,"cmd":cmd,"args":args})).await.is_err() { break; }
                        }
                    }
                }
                incoming = socket.next(), if !subscriptions.values().any(|s| s.sent_offset.saturating_sub(s.offset.unwrap_or(0)) > 1024 * 1024) => {
                    last_seen = Instant::now();
                    match incoming {
                        Some(Ok(Message::Text(text))) => {
                            let Ok(value) = serde_json::from_str::<Value>(&text) else { break; };
                            receive(value, &events, &mut subscriptions, &mut pending);
                        }
                        Some(Ok(Message::Ping(data))) => { if socket.send(Message::Pong(data)).await.is_err() { break; } }
                        Some(Ok(Message::Pong(_))) => {}
                        Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                        _ => {}
                    }
                }
                _ = heartbeat.tick() => {
                    if last_seen.elapsed() > Duration::from_secs(35) { break; }
                    pending.retain(|_, p| !p.reply.is_closed());
                    let expired: Vec<_> = pending.iter().filter(|(_, p)| p.deadline <= Instant::now()).map(|(id,_)| *id).collect();
                    for id in expired { if let Some(p) = pending.remove(&id) { let _ = p.reply.send(Err("remote-rpc-timeout".into())); } }
                    if send(&mut socket, json!({"type":"ping"})).await.is_err() { break; }
                }
            }
        }
        for (_, p) in pending.drain() {
            let _ = p.reply.send(Err("remote-disconnected".into()));
        }
        connection_state(&events, "reconnecting");
        // Drop all disconnected mutations immediately, retaining only subscriptions
        // and render acknowledgements. A reconnection cannot replay user input.
        let mut backoff = 1;
        loop {
            let delay = backoff;
            let attempt = async {
                tokio::time::sleep(Duration::from_secs(delay)).await;
                open(&url, &token).await
            };
            tokio::pin!(attempt);
            let result = loop {
                tokio::select! {
                    result = &mut attempt => break result,
                    command = rx.recv() => match command {
                        None | Some(Command::Stop) => break 'connection,
                        Some(Command::Rpc { reply, .. }) => { let _ = reply.send(Err("remote-disconnected".into())); }
                        Some(Command::Input { .. }) => {},
                        Some(Command::Subscribe { agent, frames }) => { subscriptions.insert(agent, Subscription { frames, offset: None, sent_offset: 0, restore_id: None, session: None }); }
                        Some(Command::Unsubscribe(agent)) => { subscriptions.remove(&agent); }
                        Some(Command::Ack { agent, offset, restore_id }) => { if let Some(s) = subscriptions.get_mut(&agent) { s.acknowledge(offset, restore_id.as_deref()); } }
                    }
                }
            };
            match result {
                Ok((new_socket, hello)) => {
                    if hello["instanceId"] != instance {
                        instance = hello["instanceId"].clone();
                        for sub in subscriptions.values_mut() {
                            sub.offset = None;
                            sub.sent_offset = 0;
                            sub.session = None;
                        }
                    }
                    socket = new_socket;
                    continue 'connection;
                }
                Err(error)
                    if error == "remote-auth-failed" || error == "remote-protocol-mismatch" =>
                {
                    events(
                        "remote-connection",
                        json!({"status":"disconnected","error":error}),
                    );
                    return;
                }
                Err(_) => {
                    backoff = (backoff * 2).min(10);
                }
            }
        }
    }
    for (_, p) in pending.drain() {
        let _ = p.reply.send(Err("remote-disconnected".into()));
    }
    connection_state(&events, "disconnected");
}

fn receive(
    value: Value,
    events: &Events,
    subscriptions: &mut HashMap<String, Subscription>,
    pending: &mut HashMap<u64, Pending>,
) {
    let kind = value["type"].as_str().unwrap_or_default();
    if kind == "rpcResult" {
        if let Some(p) = value["id"].as_u64().and_then(|id| pending.remove(&id)) {
            let result = if value["ok"] == true {
                Ok(value["data"].clone())
            } else {
                Err(value["error"].to_string())
            };
            let _ = p.reply.send(result);
        }
        return;
    }
    if let Some(agent) = value["agentId"].as_str() {
        if matches!(
            kind,
            "restore" | "output" | "resized" | "replay" | "journal"
        ) {
            if let Some(sub) = subscriptions.get_mut(agent) {
                if kind == "restore" {
                    let session = value["sessionId"].as_str().map(str::to_owned);
                    if session != sub.session || !value["snapshot"].is_null() {
                        sub.offset = None;
                        sub.session = session;
                    }
                    sub.sent_offset = value["baseOffset"].as_u64().unwrap_or(0);
                    sub.restore_id = Some(uuid::Uuid::new_v4().to_string());
                }
                if kind == "output" {
                    sub.sent_offset = value["offset"]
                        .as_u64()
                        .unwrap_or(0)
                        .saturating_add(value["bytes"].as_u64().unwrap_or(0));
                }
                let mut frame = value.clone();
                if kind == "restore" {
                    frame["restoreId"] = json!(sub.restore_id);
                }
                if !(sub.frames)(frame) {
                    subscriptions.remove(agent);
                }
            }
            return;
        }
    }
    let event = match kind {
        "agents" => "remote:agents",
        "sessionState" => "remote:session-state",
        "notification" => "remote:notification-new",
        "notificationCleared" => "remote:notification-cleared",
        "activity" => "remote:activity-event",
        "turnUsage" => "remote:turn-usage",
        "talkMessage" => "remote:talk-message",
        "error" => "remote:error",
        _ => return,
    };
    let payload = if value.get("payload").is_some() {
        value["payload"].clone()
    } else if kind == "notificationCleared" {
        json!({"agentId":value["agentId"],"ids":value["ids"]})
    } else {
        value
    };
    events(event, payload);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_contains_no_credentials_and_has_exact_endpoint() {
        assert_eq!(
            socket_url("https://host:443").unwrap(),
            "wss://host/webremote/v1/ws"
        );
        assert_eq!(
            socket_url("http://127.0.0.1:1234").unwrap(),
            "ws://127.0.0.1:1234/webremote/v1/ws"
        );
        for url in [
            "file:///tmp/a",
            "https://user:token@host",
            "https://host/?token=secret",
            "https://host/#token",
            "https://host/other",
        ] {
            assert!(socket_url(url).is_err());
        }
    }
    #[test]
    fn local_events_are_never_used_for_remote_sessions() {
        let received = Arc::new(std::sync::Mutex::new(Vec::new()));
        let copy = received.clone();
        let sink: Events =
            Arc::new(move |name, payload| copy.lock().unwrap().push((name.to_string(), payload)));
        receive(
            json!({"type":"sessionState","agentId":"a","payload":{"agentId":"a","state":"running"}}),
            &sink,
            &mut HashMap::new(),
            &mut HashMap::new(),
        );
        assert_eq!(received.lock().unwrap()[0].0, "remote:session-state");
    }
    #[test]
    fn receiving_output_does_not_acknowledge_unrendered_bytes() {
        let mut subscriptions = HashMap::from([(
            "a".into(),
            Subscription {
                frames: Arc::new(|_| true),
                offset: Some(12),
                sent_offset: 12,
                restore_id: Some("current".into()),
                session: Some("s".into()),
            },
        )]);
        let sink: Events = Arc::new(|_, _| {});
        receive(
            json!({"type":"output","agentId":"a","sessionId":"s","offset":12,"bytes":5,"data":"hello"}),
            &sink,
            &mut subscriptions,
            &mut HashMap::new(),
        );
        assert_eq!(subscriptions["a"].offset, Some(12));
        subscriptions
            .get_mut("a")
            .unwrap()
            .acknowledge(17, Some("previous"));
        assert_eq!(subscriptions["a"].offset, Some(12));
        subscriptions
            .get_mut("a")
            .unwrap()
            .acknowledge(17, Some("current"));
        assert_eq!(subscriptions["a"].offset, Some(17));
    }
    #[tokio::test]
    async fn reconnect_uses_render_ack_and_does_not_repeat_rpc() {
        use tokio::net::TcpListener;
        use tokio_tungstenite::accept_async;
        async fn receive_kind(socket: &mut WebSocketStream<TcpStream>, kind: &str) -> Value {
            loop {
                let message = tokio::time::timeout(Duration::from_secs(8), socket.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                if let Message::Text(text) = message {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    if value["type"] == kind {
                        return value;
                    }
                }
            }
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (close_tx, close_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let hello = json!({"type":"hello","hostName":"test","permission":"input","protoVersion":crate::webremote::protocol::WEB_REMOTE_PROTO_VERSION,"instanceId":"same"});
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(tcp).await.unwrap();
            socket.send(Message::Text(hello.to_string())).await.unwrap();
            let subscribe = receive_kind(&mut socket, "attach").await;
            assert!(subscribe["lastOffset"].is_null());
            socket.send(Message::Text(json!({"type":"restore","agentId":"a","sessionId":"s","snapshot":"","baseOffset":0,"cols":80,"rows":24}).to_string())).await.unwrap();
            socket.send(Message::Text(json!({"type":"output","agentId":"a","sessionId":"s","offset":0,"bytes":5,"seq":1,"data":"hello"}).to_string())).await.unwrap();
            let rpc = receive_kind(&mut socket, "rpc").await;
            socket
                .send(Message::Text(
                    json!({"type":"rpcResult","id":rpc["id"],"ok":true,"data":{"saved":true}})
                        .to_string(),
                ))
                .await
                .unwrap();
            close_rx.await.unwrap();
            socket.close(None).await.unwrap();
            drop(socket);
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(tcp).await.unwrap();
            socket.send(Message::Text(hello.to_string())).await.unwrap();
            let subscribe = receive_kind(&mut socket, "attach").await;
            assert_eq!(subscribe["lastOffset"], 5);
            // No previously completed mutating RPC is queued for this connection.
            socket.send(Message::Text(json!({"type":"output","agentId":"a","sessionId":"s","offset":5,"bytes":1,"seq":2,"data":"!"}).to_string())).await.unwrap();
        });
        let (frames_tx, mut frames_rx) = mpsc::unbounded_channel();
        let events: Events = Arc::new(|_, _| {});
        let (connection, _) = connect(format!("http://{addr}"), "test-token".into(), events)
            .await
            .unwrap();
        connection
            .tx
            .send(Command::Subscribe {
                agent: "a".into(),
                frames: Arc::new(move |v| frames_tx.send(v).is_ok()),
            })
            .await
            .unwrap();
        let restore = frames_rx.recv().await.unwrap();
        assert_eq!(restore["type"], "restore");
        let output = frames_rx.recv().await.unwrap();
        assert_eq!(output["data"], "hello");
        connection
            .tx
            .send(Command::Ack {
                agent: "a".into(),
                offset: 5,
                restore_id: restore["restoreId"].as_str().map(str::to_owned),
            })
            .await
            .unwrap();
        let (reply, response) = oneshot::channel();
        connection
            .tx
            .send(Command::Rpc {
                cmd: "office.saveState".into(),
                args: json!({}),
                reply,
            })
            .await
            .unwrap();
        assert_eq!(response.await.unwrap().unwrap()["saved"], true);
        close_tx.send(()).unwrap();
        let tail = tokio::time::timeout(Duration::from_secs(8), frames_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tail["data"], "!");
        server.await.unwrap();
        drop(connection);
    }
}
