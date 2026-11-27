// src-tauri/src/link/mod.rs
//
// 머신 연동(tailnet·LAN 안의 Agent Office끼리 호스트-클라이언트). 이 단계는 링크만 둔다: 설정(끔·호스트·
// 클라이언트, 클라이언트의 호스트 주소 — 자동 발견 없음), 페어링 코드 → 기기별 토큰, 기기별 해제,
// 클라이언트 → 호스트 웹 상태판 HTTPS의 WebSocket 하나, 프로토콜 버전 비교, 재연결(백오프)과 보낼 것 버퍼,
// 연결 상태 표시. 그 위의 일(보기·세션·업무)은 `LinkHost::set_event_handler`·`LinkHost::send`,
// `LinkClient::enqueue`·`set_command_handler`에 건다.
//
// 모드는 하나만 고른다 — 클라이언트는 호스트 하나에만 붙고, 호스트는 다른 호스트에 붙지 못한다.
// 권한 등급은 없다(페어링한 기기는 전부 허용).
//
// 파일(데이터 폴더): `link.json` 설정, `link-machine-id` 이 기계의 id, 호스트 `link-devices.json`,
// 클라이언트 `link-client.json`(토큰·고정 CA)과 `link-outbox.json`.
pub mod address;
pub mod client;
mod client_tls;
pub mod host;
pub mod outbox;
pub mod protocol;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use serde::{Deserialize, Serialize};

pub use address::HostAddress;
pub use client::{LinkClient, LinkClientState, LinkClientStatus};
pub use host::{LinkDevice, LinkHost, LinkHostStatus, LinkPairing};

pub const CONFIG_FILE: &str = "link.json";
pub const MACHINE_ID_FILE: &str = "link-machine-id";

/// 상태가 바뀌었음을 알리는 콜백. 잠금을 쥔 채 부르지 않는다(받는 쪽이 상태를 다시 읽는다).
pub type Changed = Arc<dyn Fn() + Send + Sync>;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LinkMode {
    #[default]
    Off,
    Host,
    Client,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LinkConfig {
    #[serde(default)]
    pub mode: LinkMode,
    /// 클라이언트가 붙을 호스트 주소(정리한 모양). 비어 있으면 붙지 않는다.
    #[serde(default)]
    pub host_address: String,
}

/// 설정 화면·표시기가 보는 상태(`link-status` 이벤트).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkStatus {
    pub config: LinkConfig,
    /// 이 앱의 연동 프로토콜 버전.
    pub protocol: u32,
    pub app_version: String,
    pub machine_id: String,
    /// 호스트 모드일 때만.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<LinkHostStatus>,
    /// 클라이언트 모드일 때만.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<LinkClientStatus>,
}

pub trait LinkEvents: Send + Sync {
    fn status(&self, status: &LinkStatus);
}

pub struct TauriLinkEvents(pub tauri::AppHandle);

impl LinkEvents for TauriLinkEvents {
    fn status(&self, status: &LinkStatus) {
        use tauri::Emitter;
        let _ = self.0.emit_to("main", "link-status", status);
    }
}

pub struct LinkState {
    dir: Option<PathBuf>,
    config: Mutex<LinkConfig>,
    machine_id: String,
    pub host: Arc<LinkHost>,
    pub client: Arc<LinkClient>,
    events: Arc<dyn LinkEvents>,
    last: Mutex<Option<LinkStatus>>,
}

/// 이 기계의 id. 처음 한 번 만들어 둔다(다시 페어링해도 같은 기기로 보이게).
fn load_machine_id(dir: Option<&Path>) -> String {
    let Some(dir) = dir else { return uuid::Uuid::new_v4().to_string() };
    let path = dir.join(MACHINE_ID_FILE);
    if let Some(id) = std::fs::read_to_string(&path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        return id;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&path, &id);
    id
}

impl LinkState {
    /// `dir`가 없으면 파일을 쓰지 않는다(시험). `name`은 이 기계 이름(상대에게 보인다).
    pub fn new(
        dir: Option<PathBuf>,
        name: Arc<dyn Fn() -> String + Send + Sync>,
        events: Arc<dyn LinkEvents>,
    ) -> Arc<Self> {
        let config = dir
            .as_ref()
            .and_then(|d| std::fs::read(d.join(CONFIG_FILE)).ok())
            .and_then(|b| serde_json::from_slice::<LinkConfig>(&b).ok())
            .unwrap_or_default();
        let machine_id = load_machine_id(dir.as_deref());
        let state = Arc::new_cyclic(|me: &Weak<LinkState>| {
            let me = me.clone();
            let changed: Changed = Arc::new(move || {
                if let Some(s) = me.upgrade() {
                    s.publish();
                }
            });
            let host = Arc::new(LinkHost::new(
                dir.as_ref().map(|d| d.join(host::DEVICES_FILE)),
                machine_id.clone(),
                name.clone(),
                changed.clone(),
            ));
            let client = Arc::new(LinkClient::new(dir.clone(), machine_id.clone(), name(), changed));
            LinkState { dir, config: Mutex::new(config), machine_id, host, client, events, last: Mutex::new(None) }
        });
        state.apply();
        state
    }

    pub fn config(&self) -> LinkConfig {
        self.config.lock().unwrap().clone()
    }

    /// 지금 설정을 호스트·클라이언트에 건다.
    fn apply(&self) {
        let config = self.config();
        self.host.set_enabled(config.mode == LinkMode::Host);
        let address = HostAddress::parse(&config.host_address).ok();
        self.client.set_target(config.mode == LinkMode::Client, address);
    }

    pub fn configure(&self, mut config: LinkConfig) -> Result<LinkStatus, String> {
        config.host_address = match config.host_address.trim() {
            "" => String::new(),
            raw => HostAddress::parse(raw)?.display(),
        };
        if let Some(dir) = &self.dir {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let bytes = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
            std::fs::write(dir.join(CONFIG_FILE), bytes).map_err(|e| e.to_string())?;
        }
        *self.config.lock().unwrap() = config;
        self.apply();
        Ok(self.status())
    }

    /// 클라이언트: 주소와 코드로 페어링하고, 성공하면 그 주소를 설정에 저장한다.
    pub async fn pair(&self, address: &str, code: &str) -> Result<LinkStatus, String> {
        if self.config().mode != LinkMode::Client {
            return Err("link-not-client".into());
        }
        let address = HostAddress::parse(address)?;
        self.client.pair(&address, code).await?;
        let config = LinkConfig { host_address: address.display(), ..self.config() };
        self.configure(config)
    }

    pub fn status(&self) -> LinkStatus {
        let config = self.config();
        LinkStatus {
            protocol: protocol::PROTOCOL,
            app_version: protocol::app_version().into(),
            machine_id: self.machine_id.clone(),
            host: (config.mode == LinkMode::Host).then(|| self.host.status()),
            client: (config.mode == LinkMode::Client).then(|| self.client.status()),
            config,
        }
    }

    /// 바뀐 경우에만 렌더러에 알린다.
    fn publish(&self) {
        let status = self.status();
        let mut last = self.last.lock().unwrap();
        if last.as_ref() != Some(&status) {
            *last = Some(status.clone());
            drop(last);
            self.events.status(&status);
        }
    }

    /// 클라이언트 연결 루프와 코드 만료 감시를 띄운다. 앱 수명 동안 한 번.
    pub fn start(self: &Arc<Self>) {
        let client = self.client.clone();
        tauri::async_runtime::spawn(async move { client.run().await });
        let me = Arc::downgrade(self);
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let Some(me) = me.upgrade() else { break };
                // 코드 만료처럼 시간만 흘러 바뀌는 것을 반영한다.
                me.publish();
            }
        });
    }
}


#[cfg(test)]
pub(crate) mod fake {
    use super::*;

    pub struct NoLinkEvents;
    impl LinkEvents for NoLinkEvents {
        fn status(&self, _status: &LinkStatus) {}
    }

    #[derive(Default)]
    pub struct RecordingLinkEvents(pub Mutex<Vec<LinkStatus>>);
    impl LinkEvents for RecordingLinkEvents {
        fn status(&self, status: &LinkStatus) {
            self.0.lock().unwrap().push(status.clone());
        }
    }
}

#[cfg(test)]
mod tests;
