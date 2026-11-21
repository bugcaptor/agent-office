// src-tauri/src/webdash/mod.rs
//
// 웹 상태판. 앱이 도는 동안 tailnet·LAN에 작은 HTTP 서버를 띄워, 폰이나 다른
// PC 브라우저에서 캐릭터들의 상태와 에이전트 대화 로그를 본다.
//
// 터미널을 그대로 비추지 않는다(예전에 해 보니 폰에서 쓰기 나빴다). 대화는
// CLI 전사(JSONL)를 **읽기만** 하고, 손을 대는 일은 간접 명령으로만 한다:
// 터미널 재시작·종료·이어하기, 작업 중단, Claude/Codex 실행, 업무 만들기·보내기·
// 자동 진행, 토의 시작·답하기. 권한 승인 같은 CLI 안 입력은 다루지 않는다.
// 예외 하나: 일반 에이전트(캐릭터 없는 `office-agent` 곁 세션, `crate::office_agent`)에는 지시를 쓰고,
// CLI가 턴을 마치고 입력을 기다릴 때만 같은 세션에 다음 지시를 잇는다. 머리에는 컴퓨터 이름을 보인다.
//
// 보안: 설정에서 켜야 뜬다(기본 꺼짐). 고른 네트워크(tailnet/LAN)의 주소와
// 루프백에만 귀를 열고, 모든 API는 앱이 발급한 토큰(`web-dash-token`, 0600)을
// 요구한다. 접속 주소에 토큰을 실어 건네고, 페이지는 받은 토큰을 브라우저에
// 저장한 뒤 주소에서 지운다. 토큰을 새로 만들면 기존 기기는 모두 끊긴다.
//
// 명령 가운데 렌더러가 주인인 일(재시작·업무·토의)은 `web-dash-control`
// 이벤트로 렌더러에 넘기고 답을 기다린다(`ctl` 자동 진행 다리와 같은 방식).
// 앱 화면에는 명령마다 `web-dash-activity`로 짧은 알림을 띄운다.

pub mod log;
pub mod net;
mod routes;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::control::autorun::AutoRunBridge;
use crate::control::ControlContext;
use crate::httpapi::set_owner_only;

pub use net::WebDashBind;

pub const CONFIG_FILE: &str = "web-dash.json";
pub const TOKEN_FILE: &str = "web-dash-token";
pub const DEFAULT_PORT: u16 = 7788;
/// 이만큼 요청이 없던 브라우저는 접속 수에서 뺀다(페이지는 몇 초마다 묻는다).
const CLIENT_TTL: Duration = Duration::from_secs(30);
/// 인터페이스 주소가 바뀌었는지(Tailscale 켜짐·DHCP) 다시 보는 간격.
const WATCH_INTERVAL: Duration = Duration::from_secs(15);

fn default_port() -> u16 {
    DEFAULT_PORT
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebDashConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub bind: WebDashBind,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for WebDashConfig {
    fn default() -> Self {
        Self { enabled: false, bind: WebDashBind::default(), port: DEFAULT_PORT }
    }
}

/// 설정 화면이 보는 상태.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebDashStatus {
    pub config: WebDashConfig,
    pub running: bool,
    /// 접속 주소(토큰 포함). 루프백 주소는 맨 뒤에 둔다.
    pub urls: Vec<String>,
    /// 골라 둔 네트워크의 주소를 못 찾았거나 포트를 못 열었을 때의 이유 코드.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 최근 30초 안에 요청한 브라우저 주소 수.
    pub clients: usize,
}

/// 렌더러에 알리는 일. 테스트는 기록만 한다.
pub trait WebDashEvents: Send + Sync {
    /// 상태(접속 수·주소)가 바뀌었다.
    fn status(&self, status: &WebDashStatus);
    /// 웹에서 명령이 들어왔다(앱 화면 알림용).
    fn activity(&self, activity: &serde_json::Value);
}

/// 렌더러 창(`main`)으로 보내는 실제 구현.
pub struct TauriDashEvents(pub tauri::AppHandle);

impl WebDashEvents for TauriDashEvents {
    fn status(&self, status: &WebDashStatus) {
        use tauri::Emitter;
        let _ = self.0.emit_to("main", "web-dash-status", status);
    }
    fn activity(&self, activity: &serde_json::Value) {
        use tauri::Emitter;
        let _ = self.0.emit_to("main", "web-dash-activity", activity);
    }
}

struct Installed {
    addrs: Vec<Ipv4Addr>,
    port: u16,
    shutdowns: Vec<oneshot::Sender<()>>,
}

pub struct WebDashState {
    data_dir: PathBuf,
    config: Mutex<WebDashConfig>,
    token: RwLock<Option<String>>,
    installed: Mutex<Option<Installed>>,
    error: Mutex<Option<String>>,
    start_gate: tokio::sync::Mutex<()>,
    clients: Mutex<HashMap<IpAddr, Instant>>,
    last_status: Mutex<Option<WebDashStatus>>,
    /// 렌더러가 밀어 준 캐릭터별 라벨(작업 라벨은 렌더러에만 있다).
    labels: RwLock<HashMap<String, serde_json::Value>>,
    pub logs: log::LogCache,
    pub bridge: Arc<AutoRunBridge>,
    events: Arc<dyn WebDashEvents>,
}

impl WebDashState {
    pub fn new(
        data_dir: PathBuf,
        sources: log::SourceFactory,
        bridge: Arc<AutoRunBridge>,
        events: Arc<dyn WebDashEvents>,
    ) -> Self {
        let config = std::fs::read(data_dir.join(CONFIG_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice::<WebDashConfig>(&b).ok())
            .unwrap_or_default();
        let token = std::fs::read_to_string(data_dir.join(TOKEN_FILE))
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        Self {
            data_dir,
            config: Mutex::new(config),
            token: RwLock::new(token),
            installed: Mutex::new(None),
            error: Mutex::new(None),
            start_gate: tokio::sync::Mutex::new(()),
            clients: Mutex::new(HashMap::new()),
            last_status: Mutex::new(None),
            labels: RwLock::new(HashMap::new()),
            logs: log::LogCache::new(sources),
            bridge,
            events,
        }
    }

    pub fn config(&self) -> WebDashConfig {
        self.config.lock().unwrap().clone()
    }

    pub fn token(&self) -> Option<String> {
        self.token.read().unwrap().clone()
    }

    /// 토큰이 없으면 만든다(켤 때 부른다).
    fn ensure_token(&self) -> Result<String, String> {
        if let Some(t) = self.token() {
            return Ok(t);
        }
        self.rotate_token()
    }

    /// 새 토큰을 발급한다. 기존 기기는 다음 요청부터 401이다.
    pub fn rotate_token(&self) -> Result<String, String> {
        std::fs::create_dir_all(&self.data_dir).map_err(|e| e.to_string())?;
        let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let path = self.data_dir.join(TOKEN_FILE);
        std::fs::write(&path, &token).map_err(|e| e.to_string())?;
        set_owner_only(&path);
        *self.token.write().unwrap() = Some(token.clone());
        self.clients.lock().unwrap().clear();
        self.publish_status();
        Ok(token)
    }

    pub fn authorized(&self, presented: Option<&str>) -> bool {
        match (presented, self.token()) {
            (Some(p), Some(t)) => crate::httpapi::ct_eq(p.as_bytes(), t.as_bytes()),
            _ => false,
        }
    }

    pub fn note_client(&self, ip: IpAddr) {
        let fresh = self.clients.lock().unwrap().insert(ip, Instant::now()).is_none();
        if fresh {
            self.publish_status();
        }
    }

    fn client_count(&self) -> usize {
        let mut clients = self.clients.lock().unwrap();
        clients.retain(|_, seen| seen.elapsed() < CLIENT_TTL);
        clients.len()
    }

    pub fn set_labels(&self, labels: HashMap<String, serde_json::Value>) {
        *self.labels.write().unwrap() = labels;
    }

    pub fn label(&self, agent_id: &str) -> Option<serde_json::Value> {
        self.labels.read().unwrap().get(agent_id).cloned()
    }

    pub fn status(&self) -> WebDashStatus {
        let config = self.config();
        let (running, mut addrs, port) = match self.installed.lock().unwrap().as_ref() {
            Some(i) => (true, i.addrs.clone(), i.port),
            None => (false, Vec::new(), config.port),
        };
        // 루프백은 이 머신에서만 쓰므로 맨 뒤로.
        addrs.sort_by_key(|ip| ip.is_loopback());
        let token = self.token().unwrap_or_default();
        WebDashStatus {
            config,
            running,
            urls: addrs.iter().map(|ip| format!("http://{ip}:{port}/#t={token}")).collect(),
            error: self.error.lock().unwrap().clone(),
            clients: self.client_count(),
        }
    }

    /// 바뀐 경우에만 렌더러에 알린다.
    fn publish_status(&self) {
        let status = self.status();
        let mut last = self.last_status.lock().unwrap();
        if last.as_ref() != Some(&status) {
            *last = Some(status.clone());
            drop(last);
            self.events.status(&status);
        }
    }

    pub fn activity(&self, activity: serde_json::Value) {
        self.events.activity(&activity);
    }

    /// 설정을 저장하고 서버를 그에 맞춘다.
    pub async fn configure(&self, config: WebDashConfig, ctx: Arc<ControlContext>, me: Arc<WebDashState>) -> Result<WebDashStatus, String> {
        if config.port < 1024 {
            return Err("web-dash-port-invalid".into());
        }
        std::fs::create_dir_all(&self.data_dir).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
        std::fs::write(self.data_dir.join(CONFIG_FILE), bytes).map_err(|e| e.to_string())?;
        *self.config.lock().unwrap() = config;
        self.reconcile(ctx, me).await;
        Ok(self.status())
    }

    /// 설정·인터페이스 주소에 맞게 서버를 띄우거나 내리거나 다시 붙인다(멱등).
    pub async fn reconcile(&self, ctx: Arc<ControlContext>, me: Arc<WebDashState>) {
        let _gate = self.start_gate.lock().await;
        let config = self.config();
        if !config.enabled {
            self.stop();
            *self.error.lock().unwrap() = None;
            self.publish_status();
            return;
        }
        if let Err(e) = self.ensure_token() {
            *self.error.lock().unwrap() = Some(format!("web-dash-token-failed: {e}"));
            self.publish_status();
            return;
        }
        let wanted = net::bind_addrs(config.bind, &net::local_ip_addrs());
        let same = self
            .installed
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|i| i.port == config.port && i.addrs == wanted);
        if same {
            self.publish_status();
            return;
        }
        self.stop();
        let mut bound = Vec::new();
        let mut shutdowns = Vec::new();
        let mut failed = None;
        for ip in &wanted {
            match routes::serve(*ip, config.port, ctx.clone(), me.clone()).await {
                Ok(shutdown) => {
                    bound.push(*ip);
                    shutdowns.push(shutdown);
                }
                Err(e) => {
                    eprintln!("web-dash: bind {ip}:{} failed: {e}", config.port);
                    failed.get_or_insert_with(|| format!("web-dash-bind-failed: {ip}:{} {e}", config.port));
                }
            }
        }
        let network_missing = bound.iter().all(|ip| ip.is_loopback());
        *self.error.lock().unwrap() = failed.or_else(|| network_missing.then(|| "web-dash-no-network".to_string()));
        if !bound.is_empty() {
            *self.installed.lock().unwrap() = Some(Installed { addrs: bound, port: config.port, shutdowns });
        }
        self.publish_status();
    }

    pub fn stop(&self) {
        if let Some(installed) = self.installed.lock().unwrap().take() {
            for s in installed.shutdowns {
                let _ = s.send(());
            }
        }
    }

    /// 주소 변화와 접속 수를 주기적으로 본다. 앱 수명 동안 한 번 띄운다.
    pub fn spawn_watch(me: Arc<WebDashState>, ctx: Arc<ControlContext>) {
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(WATCH_INTERVAL).await;
                if me.config().enabled {
                    me.reconcile(ctx.clone(), me.clone()).await;
                } else {
                    me.publish_status();
                }
            }
        });
    }
}

/// 이 컴퓨터의 이름. 웹 머리에 보여 어느 기기의 앱인지 가린다. macOS는 공유 설정의 컴퓨터 이름,
/// Windows는 `COMPUTERNAME`, 그 밖은 호스트 이름이다. 처음 한 번만 읽는다(블로킹).
pub fn computer_name() -> String {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| read_computer_name().unwrap_or_default()).clone()
}

fn read_computer_name() -> Option<String> {
    let run = |program: &str, args: &[&str]| -> Option<String> {
        let out = std::process::Command::new(program).args(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
    };
    #[cfg(target_os = "macos")]
    if let Some(name) = run("/usr/sbin/scutil", &["--get", "ComputerName"]) {
        return Some(name);
    }
    #[cfg(windows)]
    if let Some(name) = std::env::var("COMPUTERNAME").ok().filter(|s| !s.trim().is_empty()) {
        return Some(name.trim().to_string());
    }
    #[cfg(target_os = "linux")]
    if let Some(name) = std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        return Some(name);
    }
    run("hostname", &[])
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;

    #[derive(Default)]
    pub struct RecordingDashEvents {
        pub statuses: Mutex<Vec<WebDashStatus>>,
        pub activities: Mutex<Vec<serde_json::Value>>,
    }

    impl WebDashEvents for RecordingDashEvents {
        fn status(&self, status: &WebDashStatus) {
            self.statuses.lock().unwrap().push(status.clone());
        }
        fn activity(&self, activity: &serde_json::Value) {
            self.activities.lock().unwrap().push(activity.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(dir: &std::path::Path) -> WebDashState {
        WebDashState::new(
            dir.to_path_buf(),
            Arc::new(Vec::new),
            Arc::new(AutoRunBridge::new(|_| Ok(()))),
            Arc::new(fake::RecordingDashEvents::default()),
        )
    }

    #[test]
    fn token_rotation_invalidates_previous_token_and_persists() {
        let dir = std::env::temp_dir().join(format!("webdash-{}", uuid::Uuid::new_v4()));
        let s = state(&dir);
        assert!(!s.authorized(Some("x")));
        let first = s.rotate_token().unwrap();
        assert!(s.authorized(Some(&first)));
        let second = s.rotate_token().unwrap();
        assert!(!s.authorized(Some(&first)));
        assert!(s.authorized(Some(&second)));
        assert!(!s.authorized(None));
        // 다시 띄워도 같은 토큰을 읽는다.
        assert!(state(&dir).authorized(Some(&second)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_defaults_when_missing_or_partial() {
        let c: WebDashConfig = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert_eq!(c, WebDashConfig { enabled: true, bind: WebDashBind::Tailnet, port: DEFAULT_PORT });
        let c: WebDashConfig = serde_json::from_str(r#"{"bind":"both","port":9000}"#).unwrap();
        assert_eq!(c.bind, WebDashBind::Both);
        assert!(!c.enabled);
    }
}
