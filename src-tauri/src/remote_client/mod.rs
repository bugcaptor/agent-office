//! Desktop-only remote relay. Credentials live only in the socket task; local
//! AppState and remote server state never share a persistence path or event name.
mod credentials;
mod transport;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{ipc::Channel, AppHandle, Emitter, Manager, State, WebviewWindow};
use tokio::sync::{oneshot, watch, Mutex};
use crate::state::AppState;
use transport::Command;

pub const WINDOW_LABEL: &str = "remote-office";
pub struct RemoteClientState {
    connection: Mutex<Option<transport::Connection>>,
    pub(crate) settings_update: Mutex<()>,
    cancellation: watch::Sender<u64>,
}
impl Default for RemoteClientState {
    fn default() -> Self {
        Self { connection: Mutex::new(None), settings_update: Mutex::new(()), cancellation: watch::channel(0).0 }
    }
}
fn require_enabled(app_state: &AppState) -> Result<(), String> {
    if app_state.settings.read().unwrap().remote_server_connection_enabled {
        Ok(())
    } else {
        Err("remote-server-connection-disabled".into())
    }
}
async fn until_disconnected<T>(
    cancellation: &mut watch::Receiver<u64>,
    future: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::select! {
        biased;
        _ = cancellation.changed() => Err("remote-disconnected".into()),
        result = future => result,
    }
}

fn require_remote(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == WINDOW_LABEL {
        Ok(())
    } else {
        Err("remote-window-required".into())
    }
}
impl RemoteClientState {
    async fn send(&self, command: Command) -> Result<(), String> {
        let sender = self
            .connection
            .lock()
            .await
            .as_ref()
            .map(|c| c.tx.clone())
            .ok_or("remote-disconnected")?;
        sender
            .send(command)
            .await
            .map_err(|_| "remote-disconnected".into())
    }
    pub async fn disconnect(&self) {
        self.cancellation.send_modify(|epoch| *epoch = epoch.wrapping_add(1));
        if let Some(connection) = self.connection.lock().await.take() {
            let _ = connection.tx.try_send(Command::Stop);
            // Dropping the task closes the socket. No session disposal RPC exists here.
            drop(connection);
        }
    }
}

#[tauri::command]
pub async fn remote_open_window(app: AppHandle, app_state: State<'_, AppState>) -> Result<(), String> {
    require_enabled(&app_state)?;
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        window.show().map_err(|e| e.to_string())?;
        return window.set_focus().map_err(|e| e.to_string());
    }
    tauri::WebviewWindowBuilder::new(
        &app,
        WINDOW_LABEL,
        tauri::WebviewUrl::App("index.html?remote=1".into()),
    )
    .title("Agent Office")
    .inner_size(1100.0, 760.0)
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn remote_connect(
    window: WebviewWindow,
    app_state: State<'_, AppState>,
    state: State<'_, RemoteClientState>,
    url: String,
    token: String,
) -> Result<Value, String> {
    require_remote(&window)?;
    require_enabled(&app_state)?;
    // Validate before replacing the existing connection.  `connect` performs the
    // same normalization again when it opens the socket.
    transport::socket_url(&url)?;
    if token.trim().is_empty() {
        return Err("remote-invalid-token".into());
    }
    let saved_url = url.trim().to_owned();
    let saved_token = token.trim().to_owned();
    // Serialize simultaneous connect requests so an old task cannot replace a new one.
    let mut cancellation = state.cancellation.subscribe();
    let mut slot = until_disconnected(&mut cancellation, async { Ok(state.connection.lock().await) }).await?;
    require_enabled(&app_state)?;
    slot.take();
    let app = window.app_handle().clone();
    let events: transport::Events = Arc::new(move |name, payload| {
        let _ = app.emit_to(WINDOW_LABEL, name, payload);
    });
    let (connection, hello) = until_disconnected(&mut cancellation, transport::connect(url, token, events)).await?;
    require_enabled(&app_state)?;
    // Persist only an authenticated, successfully handshaken connection.  If
    // this fails, dropping `connection` closes the new socket and the caller can
    // surface the failure instead of silently forgetting its credentials.
    credentials::save(&window.app_handle(), &saved_url, &saved_token)?;
    *slot = Some(connection);
    Ok(json!({"hostName":hello["hostName"],"permission":hello["permission"]}))
}

#[tauri::command]
pub async fn remote_load_connection(
    window: WebviewWindow,
    app_state: State<'_, AppState>,
) -> Result<Option<credentials::StoredConnection>, String> {
    require_remote(&window)?;
    require_enabled(&app_state)?;
    credentials::load(&window.app_handle())
}

#[tauri::command]
pub async fn remote_rpc(
    window: WebviewWindow,
    app_state: State<'_, AppState>,
    state: State<'_, RemoteClientState>,
    cmd: String,
    args: Option<Value>,
) -> Result<Value, String> {
    require_remote(&window)?;
    require_enabled(&app_state)?;
    let (reply, receive) = oneshot::channel();
    state
        .send(Command::Rpc {
            cmd,
            args: args.unwrap_or(Value::Null),
            reply,
        })
        .await?;
    tokio::time::timeout(std::time::Duration::from_secs(35), receive)
        .await
        .map_err(|_| "remote-rpc-timeout")?
        .map_err(|_| "remote-disconnected")?
}

#[tauri::command(rename_all = "camelCase")]
pub async fn remote_subscribe(
    window: WebviewWindow,
    app_state: State<'_, AppState>,
    state: State<'_, RemoteClientState>,
    agent_id: String,
    channel: Channel<Value>,
) -> Result<(), String> {
    require_remote(&window)?;
    require_enabled(&app_state)?;
    state
        .send(Command::Subscribe {
            agent: agent_id,
            frames: Arc::new(move |value| channel.send(value).is_ok()),
        })
        .await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn remote_unsubscribe(
    window: WebviewWindow,
    state: State<'_, RemoteClientState>,
    agent_id: String,
) -> Result<(), String> {
    require_remote(&window)?;
    state.send(Command::Unsubscribe(agent_id)).await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn remote_input(
    window: WebviewWindow,
    app_state: State<'_, AppState>,
    state: State<'_, RemoteClientState>,
    agent_id: String,
    data: String,
) -> Result<(), String> {
    require_remote(&window)?;
    require_enabled(&app_state)?;
    if data.len() > 1024 * 1024 {
        return Err("remote-input-too-large".into());
    }
    state
        .send(Command::Input {
            agent: agent_id,
            data,
        })
        .await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn remote_ack(
    window: WebviewWindow,
    app_state: State<'_, AppState>,
    state: State<'_, RemoteClientState>,
    agent_id: String,
    offset: u64,
    restore_id: Option<String>,
) -> Result<(), String> {
    require_remote(&window)?;
    require_enabled(&app_state)?;
    state
        .send(Command::Ack {
            agent: agent_id,
            offset,
            restore_id,
        })
        .await
}
#[tauri::command]
pub async fn remote_disconnect(
    window: WebviewWindow,
    state: State<'_, RemoteClientState>,
) -> Result<(), String> {
    require_remote(&window)?;
    state.disconnect().await;
    let _ = window.emit("remote-connection", json!({"status":"disconnected"}));
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn disconnect_cancels_pending_handshake_and_waiting_connect() {
        let state = RemoteClientState::default();
        let mut pending = state.cancellation.subscribe();
        let mut queued = state.cancellation.subscribe();
        // Cancellation is retained even if a request has not yet started polling.
        state.disconnect().await;
        for receiver in [&mut pending, &mut queued] {
            let result: Result<(), String> = until_disconnected(receiver, std::future::pending()).await;
            assert_eq!(result.unwrap_err(), "remote-disconnected");
        }
        let mut fresh = state.cancellation.subscribe();
        assert_eq!(until_disconnected(&mut fresh, async { Ok(42) }).await.unwrap(), 42);
    }
    #[tokio::test]
    async fn disconnect_does_not_wait_for_pending_network_io() {
        let state = Arc::new(RemoteClientState::default());
        let mut cancellation = state.cancellation.subscribe();
        let guard = state.connection.lock().await;
        let other = state.clone();
        let disconnect = tokio::spawn(async move { other.disconnect().await });
        let result: Result<(), String> = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            until_disconnected(&mut cancellation, std::future::pending())
        ).await.unwrap();
        assert_eq!(result.unwrap_err(), "remote-disconnected");
        drop(guard);
        disconnect.await.unwrap();
    }
}
