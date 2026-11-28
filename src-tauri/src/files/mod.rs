// src-tauri/src/files/mod.rs
//
// 머신 간 파일 보내기. 연동으로 붙은 호스트·클라이언트 사이(클라이언트끼리는 호스트 중계)와 이 머신 캐릭터
// 사이에서 파일·폴더를 보낸다. 받는 쪽은 수락 없이 받은 파일 폴더(기본 `~/Downloads/Agent Office`)에 두고
// 알린다. 머신마다 **파일 받기**를 켜야 받는다(`files.json`, 기본 꺼짐). 가져오기(pull)는 없다.
//
// 진행은 호스트가 맡는다(`engine.rs`): 원본 머신에서 청크를 읽어(로컬 파일이거나 `file-read` 물음) 받는
// 머신에 쓴다(로컬이거나 `file-chunk` 물음). 클라이언트가 보내면 매니페스트를 만들어 `file-send`로 호스트에
// 넘기고, 호스트가 진행 상태를 `file-update`로 내려 준다. 연동 배선은 `link/files.rs`.
//
// 파일(데이터 폴더): `files.json` 설정, `file-transfers.json` 기록(최근 200건, 보내는 머신의 원본 경로 포함),
// `file-targets.json` 대상 목록(CLI가 읽음), `file-requests/` CLI 요청, `file-uploads/` 웹 업로드 임시 원본.
pub mod cli;
pub mod engine;
pub mod manifest;
pub mod model;
pub mod receive;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::control::ControlContext;
use crate::link::{LinkMode, LinkState};
pub use model::{Party, Target, Transfer, TransferState};

pub const CONFIG_FILE: &str = "files.json";
pub const TRANSFERS_FILE: &str = "file-transfers.json";
pub const TARGETS_FILE: &str = "file-targets.json";
pub const REQUESTS_DIR: &str = "file-requests";
pub const UPLOADS_DIR: &str = "file-uploads";
/// 기록에 남길 전송 수.
const KEEP: usize = 200;
/// 진행 중 기록을 파일에 쓰는 간격.
const SAVE_EVERY_MS: u128 = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FilesConfig {
    /// 다른 머신·캐릭터가 보낸 파일을 받는다.
    #[serde(default)]
    pub receive_enabled: bool,
    /// 받은 파일 폴더. 비우면 `~/Downloads/Agent Office`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receive_dir: Option<String>,
}

/// 렌더러·웹이 보는 상태.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesStatus {
    pub config: FilesConfig,
    /// 실제로 쓰는 받은 파일 폴더.
    pub receive_dir: String,
    pub machine_id: String,
    pub machine_name: String,
    /// 연동이 꺼져 있으면 이 머신 캐릭터끼리만 보낸다.
    pub linked: bool,
}

pub trait FilesEvents: Send + Sync {
    /// 기록이 바뀌었다(렌더러 `file-transfers`).
    fn changed(&self, transfers: &[Transfer]);
    /// 이 머신이 파일을 다 받았다(앱 알림).
    fn received(&self, _t: &Transfer) {}
}

/// 보내기 요청(앱·웹·CLI·ctl 공통).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendRequest {
    /// 대상 글(`머신`, `캐릭터`, `캐릭터@머신`)이나 id.
    #[serde(default)]
    pub target: String,
    /// 대상을 정확히 고른 경우(앱·웹 고르기 창).
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    pub paths: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub notify: bool,
    /// 보내는 에이전트(이 머신 캐릭터 id).
    #[serde(default)]
    pub from_agent: Option<String>,
    #[serde(default)]
    pub via: String,
    /// CLI가 정한 id(요청 파일 이름과 같게).
    #[serde(default)]
    pub id: Option<String>,
    /// 끝나면 지울 임시 원본 폴더(웹 업로드).
    #[serde(skip)]
    pub cleanup: Option<PathBuf>,
}

#[derive(Default)]
struct Store {
    transfers: Vec<Transfer>,
}

pub struct FilesState {
    dir: Option<PathBuf>,
    pub machine_id: String,
    config: Mutex<FilesConfig>,
    store: Mutex<Store>,
    pub receiver: receive::Receiver,
    /// 돌고 있는 전송의 취소 깃발(매니페스트 만들기·진행 모두).
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
    link: OnceLock<Weak<LinkState>>,
    ctl: OnceLock<Arc<ControlContext>>,
    events: RwLock<Option<Arc<dyn FilesEvents>>>,
    /// 호스트: 붙은 기기가 알린 파일 받기 여부(기기 id → 켜짐).
    pub(crate) device_receive: Mutex<HashMap<String, bool>>,
    /// 클라이언트: 호스트가 내려 준 연동 전체 대상 목록.
    pub(crate) host_targets: Mutex<Vec<Target>>,
    last_save: Mutex<Option<Instant>>,
    last_targets: Mutex<Option<String>>,
}

pub fn default_receive_dir() -> PathBuf {
    let home = PathBuf::from(crate::session::manager::home_dir());
    home.join("Downloads").join("Agent Office")
}

fn trimmed(s: &Option<String>) -> Option<String> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

impl FilesState {
    pub fn new(dir: Option<PathBuf>, machine_id: String) -> Arc<Self> {
        let config = dir
            .as_ref()
            .and_then(|d| std::fs::read(d.join(CONFIG_FILE)).ok())
            .and_then(|b| serde_json::from_slice::<FilesConfig>(&b).ok())
            .unwrap_or_default();
        let mut transfers: Vec<Transfer> = dir
            .as_ref()
            .and_then(|d| std::fs::read(d.join(TRANSFERS_FILE)).ok())
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| serde_json::from_value(v["transfers"].clone()).ok())
            .unwrap_or_default();
        // 앱이 꺼질 때 돌던 것은 멈춘 것으로(이어 보내기로 다시).
        for t in &mut transfers {
            if t.state.is_active() {
                t.state = TransferState::Failed;
                t.error = Some("file-interrupted".into());
            }
        }
        Arc::new(Self {
            dir,
            machine_id,
            config: Mutex::new(config),
            store: Mutex::new(Store { transfers }),
            receiver: receive::Receiver::default(),
            running: Mutex::new(HashMap::new()),
            link: OnceLock::new(),
            ctl: OnceLock::new(),
            events: RwLock::new(None),
            device_receive: Mutex::new(HashMap::new()),
            host_targets: Mutex::new(Vec::new()),
            last_save: Mutex::new(None),
            last_targets: Mutex::new(None),
        })
    }

    pub(crate) fn set_link(&self, link: &Arc<LinkState>) {
        let _ = self.link.set(Arc::downgrade(link));
    }

    pub fn attach(&self, link: &Arc<LinkState>, ctl: Arc<ControlContext>, events: Arc<dyn FilesEvents>) {
        self.set_link(link);
        let _ = self.ctl.set(ctl);
        *self.events.write().unwrap() = Some(events);
    }

    pub fn data_dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    pub(crate) fn link(&self) -> Option<Arc<LinkState>> {
        self.link.get().and_then(Weak::upgrade)
    }

    pub(crate) fn ctl(&self) -> Option<&Arc<ControlContext>> {
        self.ctl.get()
    }

    pub fn mode(&self) -> LinkMode {
        self.link().map(|l| l.config().mode).unwrap_or_default()
    }

    // ── 설정 ─────────────────────────────────────────────────────────

    pub fn config(&self) -> FilesConfig {
        self.config.lock().unwrap().clone()
    }

    pub fn receive_root(&self) -> PathBuf {
        trimmed(&self.config().receive_dir).map(PathBuf::from).unwrap_or_else(default_receive_dir)
    }

    pub fn status(&self) -> FilesStatus {
        FilesStatus {
            config: self.config(),
            receive_dir: self.receive_root().to_string_lossy().into_owned(),
            machine_id: self.machine_id.clone(),
            machine_name: crate::webdash::computer_name(),
            linked: self.mode() != LinkMode::Off,
        }
    }

    pub fn configure(&self, mut config: FilesConfig) -> Result<FilesStatus, String> {
        config.receive_dir = trimmed(&config.receive_dir);
        if let Some(d) = &config.receive_dir {
            if !Path::new(d).is_absolute() {
                return Err("file-receive-dir-not-absolute".into());
            }
        }
        if let Some(dir) = &self.dir {
            let bytes = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
            crate::persistence::durable::write_atomic(&dir.join(CONFIG_FILE), &bytes).map_err(|e| e.to_string())?;
        }
        *self.config.lock().unwrap() = config;
        // 클라이언트는 호스트에 받기 여부를 다시 알린다.
        if let Some(link) = self.link() {
            crate::link::files::announce(&link, self);
        }
        self.write_targets();
        Ok(self.status())
    }

    // ── 기록 ─────────────────────────────────────────────────────────

    pub fn list(&self) -> Vec<Transfer> {
        self.store.lock().unwrap().transfers.iter().map(Transfer::public).collect()
    }

    pub fn get(&self, id: &str) -> Option<Transfer> {
        self.store.lock().unwrap().transfers.iter().find(|t| t.id == id).cloned()
    }

    /// 기록을 넣거나 바꾼다. `merge`면 이 머신만 아는 것(원본 경로·임시 폴더)을 지킨다.
    pub fn upsert(&self, mut t: Transfer, merge: bool) {
        {
            let mut store = self.store.lock().unwrap();
            if let Some(old) = store.transfers.iter_mut().find(|x| x.id == t.id) {
                if merge {
                    if t.sources.is_empty() {
                        t.sources = std::mem::take(&mut old.sources);
                    }
                    if t.cleanup.is_none() {
                        t.cleanup = old.cleanup.take();
                    }
                    // 받는 쪽이 먼저 알게 된 저장 경로는 호스트 갱신이 늦어도 지운다.
                    if t.saved.is_empty() {
                        t.saved = std::mem::take(&mut old.saved);
                    }
                }
                *old = t;
            } else {
                store.transfers.push(t);
                prune(&mut store.transfers);
            }
        }
        self.changed(true);
    }

    /// 기록 한 건을 고친다. 없으면 None.
    pub fn update(&self, id: &str, f: impl FnOnce(&mut Transfer)) -> Option<Transfer> {
        let out = {
            let mut store = self.store.lock().unwrap();
            let t = store.transfers.iter_mut().find(|t| t.id == id)?;
            let before = t.state;
            f(t);
            t.updated_at = crate::types::now_ms();
            Some((t.clone(), before != t.state))
        };
        let (t, state_changed) = out?;
        self.changed(state_changed);
        Some(t)
    }

    /// 바뀌었음을 알리고(렌더러) 파일에 쓴다(상태가 바뀌었으면 바로, 진행만이면 가끔).
    fn changed(&self, now: bool) {
        let due = {
            let mut last = self.last_save.lock().unwrap();
            let due = now || last.is_none_or(|at| at.elapsed().as_millis() >= SAVE_EVERY_MS);
            if due {
                *last = Some(Instant::now());
            }
            due
        };
        if !due {
            return;
        }
        let (bytes, public) = {
            let store = self.store.lock().unwrap();
            let bytes = serde_json::to_vec(&serde_json::json!({ "transfers": store.transfers })).unwrap_or_default();
            (bytes, store.transfers.iter().map(Transfer::public).collect::<Vec<_>>())
        };
        if let Some(dir) = &self.dir {
            let _ = crate::persistence::durable::write_atomic(&dir.join(TRANSFERS_FILE), &bytes);
        }
        let events = self.events.read().unwrap().clone();
        if let Some(e) = events {
            e.changed(&public);
        }
    }

    pub(crate) fn notify_received(&self, t: &Transfer) {
        let events = self.events.read().unwrap().clone();
        if let Some(e) = events {
            e.received(t);
        }
    }

    /// 끝난 기록 한 건을 지운다(받은 파일은 그대로 둔다).
    pub fn forget(&self, id: &str) -> Result<(), String> {
        {
            let mut store = self.store.lock().unwrap();
            let Some(i) = store.transfers.iter().position(|t| t.id == id) else { return Err("file-transfer-unknown".into()) };
            if store.transfers[i].state.is_active() || self.is_running(id) {
                return Err("file-busy".into());
            }
            store.transfers.remove(i);
        }
        self.changed(true);
        Ok(())
    }

    // ── 깃발 ─────────────────────────────────────────────────────────

    pub(crate) fn start_running(&self, id: &str) -> Option<Arc<AtomicBool>> {
        let mut running = self.running.lock().unwrap();
        if running.contains_key(id) {
            return None;
        }
        let flag = Arc::new(AtomicBool::new(false));
        running.insert(id.to_string(), flag.clone());
        Some(flag)
    }

    pub(crate) fn stop_running(&self, id: &str) {
        self.running.lock().unwrap().remove(id);
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.running.lock().unwrap().contains_key(id)
    }

    fn flag_cancel(&self, id: &str) -> bool {
        match self.running.lock().unwrap().get(id) {
            Some(f) => {
                f.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    // ── 대상 ─────────────────────────────────────────────────────────

    fn local_targets(&self) -> Vec<Target> {
        let receive = self.config().receive_enabled;
        let name = crate::webdash::computer_name();
        let mut out = vec![Target {
            machine_id: self.machine_id.clone(),
            machine_name: name.clone(),
            agent_id: None,
            agent_name: None,
            device_id: None,
            local: true,
            online: true,
            receive,
        }];
        if let Some(ctl) = self.ctl() {
            for p in ctl.store.load().agents {
                out.push(Target {
                    machine_id: self.machine_id.clone(),
                    machine_name: name.clone(),
                    agent_id: Some(p.id.clone()),
                    agent_name: Some(p.name.clone()),
                    device_id: None,
                    local: true,
                    online: true,
                    receive,
                });
            }
        }
        out
    }

    /// 보낼 수 있는 곳(이 머신이 먼저). 호스트는 붙은 머신과 그 캐릭터, 클라이언트는 호스트가 내려 준 목록.
    pub fn targets(&self) -> Vec<Target> {
        let mut out = self.local_targets();
        let Some(link) = self.link() else { return out };
        match link.config().mode {
            LinkMode::Off => {}
            LinkMode::Host => out.extend(crate::link::files::host_remote_targets(&link, self)),
            LinkMode::Client => {
                let online = link.client.connection().is_some();
                out.extend(
                    self.host_targets
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|t| t.machine_id != self.machine_id)
                        .cloned()
                        .map(|mut t| {
                            t.local = false;
                            t.online &= online;
                            t
                        }),
                );
            }
        }
        out
    }

    /// CLI가 읽는 대상 목록 파일을 바뀌었을 때만 쓴다.
    pub fn write_targets(&self) {
        let Some(dir) = &self.dir else { return };
        let targets = self.targets();
        let text = serde_json::to_string_pretty(&serde_json::json!({ "targets": targets })).unwrap_or_default();
        let mut last = self.last_targets.lock().unwrap();
        if last.as_deref() == Some(text.as_str()) {
            return;
        }
        if crate::persistence::durable::write_atomic(&dir.join(TARGETS_FILE), text.as_bytes()).is_ok() {
            *last = Some(text);
        }
    }

    fn pick_target(&self, req: &SendRequest) -> Result<Target, String> {
        let targets = self.targets();
        let found = match &req.machine_id {
            Some(m) => targets
                .iter()
                .find(|t| &t.machine_id == m && t.agent_id == req.agent_id)
                .cloned()
                .ok_or_else(|| "file-target-unknown".to_string())?,
            None => model::resolve_target(&targets, &req.target)?.clone(),
        };
        if !found.online {
            return Err(format!("file-target-offline: {}", found.spec()));
        }
        if !found.receive {
            return Err(format!("file-target-receive-off: {}", found.machine_name));
        }
        Ok(found)
    }

    // ── 보내기·취소·이어 보내기 ─────────────────────────────────────

    /// 보내기를 시작한다. 대상·경로를 확인하고 기록을 만든 뒤 매니페스트 만들기와 진행은 따로 돈다.
    pub fn send(self: &Arc<Self>, req: SendRequest) -> Result<Transfer, String> {
        let target = self.pick_target(&req)?;
        if req.paths.is_empty() {
            return Err("file-no-paths".into());
        }
        let paths: Vec<PathBuf> = req.paths.iter().map(PathBuf::from).collect();
        for p in &paths {
            if !p.is_absolute() {
                return Err(format!("file-path-not-absolute: {}", p.display()));
            }
            if std::fs::symlink_metadata(p).is_err() {
                return Err(format!("file-not-found: {}", p.display()));
            }
        }
        if let Some(note) = &req.note {
            crate::chat_queue::check_limits(note)?;
        }
        // 원격 대상은 연동이 붙어 있어야 한다(끊겨 있으면 거절).
        if target.machine_id != self.machine_id && self.mode() == LinkMode::Client {
            let connected = self.link().is_some_and(|l| l.client.connection().is_some());
            if !connected {
                return Err("link-offline".into());
            }
        }
        let from_agent = trimmed(&req.from_agent);
        let agent_name = from_agent
            .as_deref()
            .and_then(|id| self.ctl().and_then(|c| c.store.load().agents.into_iter().find(|p| p.id == id).map(|p| p.name)));
        let now = crate::types::now_ms();
        let id = trimmed(&req.id)
            .filter(|s| s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
            .unwrap_or_else(|| format!("f-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]));
        if self.get(&id).is_some() {
            return Err("file-duplicate-id".into());
        }
        let t = Transfer {
            id: id.clone(),
            created_at: now,
            updated_at: now,
            from: Party {
                machine_id: self.machine_id.clone(),
                machine_name: crate::webdash::computer_name(),
                agent_id: from_agent.filter(|_| agent_name.is_some()),
                agent_name,
            },
            to: target.party(),
            items: paths.iter().filter_map(|p| p.file_name()).map(|n| model::safe_name(&n.to_string_lossy())).collect(),
            files: Vec::new(),
            dirs: Vec::new(),
            total_bytes: 0,
            done_bytes: 0,
            progress: Vec::new(),
            state: TransferState::Preparing,
            error: None,
            note: trimmed(&req.note),
            notify: req.notify,
            saved: Vec::new(),
            via: if req.via.is_empty() { "app".into() } else { req.via.clone() },
            sources: Vec::new(),
            cleanup: req.cleanup.as_ref().map(|p| p.to_string_lossy().into_owned()),
        };
        let flag = self.start_running(&id).ok_or("file-duplicate-id")?;
        self.upsert(t.clone(), false);
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            let cancel = flag.clone();
            let built = tokio::task::spawn_blocking(move || manifest::build(&paths, &|| cancel.load(Ordering::SeqCst)))
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            me.stop_running(&id);
            let m = match built {
                Ok(m) => m,
                Err(e) => {
                    let cancelled = e == "cancelled";
                    let t = me.update(&id, |t| {
                        t.state = if cancelled { TransferState::Cancelled } else { TransferState::Failed };
                        t.error = (!cancelled).then_some(e);
                    });
                    if let Some(t) = t {
                        me.cleanup_if_final(&t);
                    }
                    return;
                }
            };
            me.update(&id, |t| {
                t.items = m.items.clone();
                t.total_bytes = m.total();
                t.dirs = m.dirs.clone();
                t.progress = vec![0; m.files.len()];
                t.sources = m.sources.iter().map(|p| p.to_string_lossy().into_owned()).collect();
                t.files = m.files;
            });
            me.dispatch(&id, false).await;
        });
        Ok(t.public())
    }

    /// 매니페스트가 준비된 전송을 진행한다: 이 머신이 진행할 수 있으면(호스트이거나 이 머신 안) 엔진으로,
    /// 아니면(클라이언트 → 다른 머신) 호스트에 넘긴다.
    pub(crate) async fn dispatch(self: &Arc<Self>, id: &str, resume: bool) {
        let Some(t) = self.get(id) else { return };
        let local_only = t.from.machine_id == self.machine_id && t.to.machine_id == self.machine_id;
        if local_only || self.mode() == LinkMode::Host {
            engine::run(self.clone(), id.to_string(), resume).await;
            return;
        }
        let Some(link) = self.link() else { return };
        let kind = if resume { crate::link::files::LIVE_RESUME } else { crate::link::files::LIVE_SEND };
        let sent = link.client.send_live(kind, serde_json::json!({ "transfer": t.public() }));
        if !sent {
            self.update(id, |t| {
                t.state = TransferState::Failed;
                t.error = Some("link-offline".into());
            });
        } else if resume {
            self.update(id, |t| {
                t.state = TransferState::Sending;
                t.error = None;
            });
        }
    }

    pub fn cancel(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let t = self.get(id).ok_or("file-transfer-unknown")?;
        if t.state.is_final() {
            return Ok(());
        }
        if self.flag_cancel(id) {
            return Ok(());
        }
        // 호스트가 진행하는 다른 머신 전송이면 호스트에 맡긴다.
        let ours = self.mode() == LinkMode::Host || (t.from.machine_id == self.machine_id && t.to.machine_id == self.machine_id);
        if !ours {
            let link = self.link().ok_or("link-offline")?;
            if !link.client.send_live(crate::link::files::LIVE_CANCEL, serde_json::json!({ "id": id })) {
                return Err("link-offline".into());
            }
            return Ok(());
        }
        let me = self.clone();
        let id = id.to_string();
        tauri::async_runtime::spawn(async move {
            engine::abort_destination(&me, &t).await;
            if let Some(t) = me.update(&id, |t| {
                t.state = TransferState::Cancelled;
                t.error = None;
            }) {
                engine::broadcast(&me, &t, true);
                me.cleanup_if_final(&t);
            }
        });
        Ok(())
    }

    pub fn resume(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let t = self.get(id).ok_or("file-transfer-unknown")?;
        if t.state != TransferState::Failed {
            return Err("file-not-resumable".into());
        }
        // 매니페스트를 만들기 전에 멈춘 것은 이어 보낼 것이 없다(다시 보낸다).
        if t.files.is_empty() && t.dirs.is_empty() {
            return Err("file-not-resumable".into());
        }
        if self.is_running(id) {
            return Err("file-busy".into());
        }
        let me = self.clone();
        let id = id.to_string();
        tauri::async_runtime::spawn(async move { me.dispatch(&id, true).await });
        Ok(())
    }

    /// 끝난 전송의 임시 원본(웹 업로드)을 지운다.
    pub(crate) fn cleanup_if_final(&self, t: &Transfer) {
        if !t.state.is_final() {
            return;
        }
        let Some(dir) = self.get(&t.id).and_then(|t| t.cleanup) else { return };
        let path = PathBuf::from(&dir);
        // 업로드 폴더 안만 지운다.
        let inside = self.dir.as_ref().is_some_and(|d| path.starts_with(d.join(UPLOADS_DIR)));
        if inside {
            let _ = std::fs::remove_dir_all(&path);
        }
        let id = t.id.clone();
        let mut store = self.store.lock().unwrap();
        if let Some(t) = store.transfers.iter_mut().find(|x| x.id == id) {
            t.cleanup = None;
        }
    }

    // ── 받는 쪽 ─────────────────────────────────────────────────────

    /// 받는 캐릭터나 보낸 머신 이름(받은 파일 폴더 아래 폴더).
    pub(crate) fn sub_folder(&self, t: &Transfer) -> String {
        if let Some(agent) = &t.to.agent_id {
            let name = self
                .ctl()
                .and_then(|c| c.store.load().agents.into_iter().find(|p| &p.id == agent).map(|p| p.name))
                .or_else(|| t.to.agent_name.clone());
            if let Some(n) = name {
                return n;
            }
        }
        t.from.machine_name.clone()
    }

    /// 받는 쪽: 오퍼를 받는다(받기 꺼짐·캐릭터 없음 거절). 블로킹 파일 작업은 따로 돌린다.
    pub(crate) async fn accept_offer(self: &Arc<Self>, t: Transfer, resume: Vec<u64>) -> Result<Vec<u64>, String> {
        if !self.config().receive_enabled {
            return Err("file-receive-off".into());
        }
        if t.to.machine_id != self.machine_id {
            return Err("file-wrong-machine".into());
        }
        if let Some(agent) = &t.to.agent_id {
            let known = self.ctl().is_none_or(|c| c.store.load().agents.iter().any(|p| &p.id == agent));
            if !known {
                return Err("unknown-agent".into());
            }
        }
        let root = self.receive_root();
        let me = self.clone();
        let offer = t.public();
        let offsets = tokio::task::spawn_blocking(move || me.receiver.offer(&root, &offer, &resume))
            .await
            .map_err(|e| e.to_string())??;
        Ok(offsets)
    }

    pub(crate) async fn write_chunk(self: &Arc<Self>, id: String, index: usize, offset: u64, data: Vec<u8>) -> Result<(), String> {
        let root = self.receive_root();
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.receiver.write(&root, &id, index, offset, &data)).await.map_err(|e| e.to_string())?
    }

    /// 받는 쪽: 다 받은 것을 옮기고 알린다. 저장한 맨 위 항목 경로를 돌려준다.
    pub(crate) async fn finish_receive(self: &Arc<Self>, id: &str) -> Result<Vec<String>, String> {
        let root = self.receive_root();
        let t = match self.get(id) {
            Some(t) => t,
            None => return Err("file-transfer-unknown".into()),
        };
        let sub = self.sub_folder(&t);
        let me = self.clone();
        let tid = id.to_string();
        let saved = tokio::task::spawn_blocking(move || me.receiver.finish(&root, &tid, &sub)).await.map_err(|e| e.to_string())??;
        let done = self.update(id, |t| {
            t.saved = saved.clone();
            t.state = TransferState::Done;
            t.done_bytes = t.total_bytes;
            t.error = None;
        });
        if let Some(t) = done {
            self.notify_received(&t);
            if t.notify {
                if let (Some(agent), Some(ctl)) = (t.to.agent_id.clone(), self.ctl().cloned()) {
                    let text = arrival_message(&t);
                    tauri::async_runtime::spawn(async move {
                        let _ = ctl.chat_queue.send(&ctl, &agent, &text).await;
                    });
                }
            }
        }
        Ok(saved)
    }

    pub(crate) fn abort_receive(&self, id: &str) {
        let root = self.receive_root();
        self.receiver.abort(&root, id);
    }
}

/// 받은 캐릭터 세션에 넣을 글.
pub fn arrival_message(t: &Transfer) -> String {
    let count = t.files.len();
    let mut text = format!(
        "[Agent Office] {} sent you {} file{} ({}). Saved at:",
        t.from.label(),
        count,
        if count == 1 { "" } else { "s" },
        human_bytes(t.total_bytes)
    );
    for p in &t.saved {
        text.push_str(&format!("\n- {p}"));
    }
    if let Some(note) = &t.note {
        text.push_str(&format!("\nNote: {note}"));
    }
    text
}

pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// 오래된 끝난 것부터 버린다(돌고 있는 것은 남긴다).
fn prune(list: &mut Vec<Transfer>) {
    while list.len() > KEEP {
        match list.iter().position(|t| !t.state.is_active()) {
            Some(i) => {
                list.remove(i);
            }
            None => break,
        }
    }
}

/// 앱 렌더러로 알린다.
pub struct TauriFilesEvents(pub tauri::AppHandle);

impl FilesEvents for TauriFilesEvents {
    fn changed(&self, transfers: &[Transfer]) {
        use tauri::Emitter;
        let _ = self.0.emit_to("main", "file-transfers", transfers);
    }
    fn received(&self, t: &Transfer) {
        use tauri::Emitter;
        let _ = self.0.emit_to("main", "file-received", t.public());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrival_message_lists_paths_and_note() {
        let mut t = crate::files::receive::tests_support::transfer();
        t.saved = vec!["/x/a.txt".into()];
        t.note = Some("검토 부탁".into());
        let text = arrival_message(&t);
        assert!(text.contains("mac sent you 1 file (5 B)"));
        assert!(text.contains("\n- /x/a.txt"));
        assert!(text.ends_with("Note: 검토 부탁"));
        assert_eq!(human_bytes(1536), "1.5 KB");
    }

    #[test]
    fn interrupted_transfers_load_as_failed_and_prune_keeps_active() {
        let tmp = tempfile::tempdir().unwrap();
        let mut t = crate::files::receive::tests_support::transfer();
        t.state = TransferState::Sending;
        std::fs::write(tmp.path().join(TRANSFERS_FILE), serde_json::to_vec(&serde_json::json!({ "transfers": [t] })).unwrap()).unwrap();
        let s = FilesState::new(Some(tmp.path().to_path_buf()), "m1".into());
        let got = s.get("t").unwrap();
        assert_eq!(got.state, TransferState::Failed);
        assert_eq!(got.error.as_deref(), Some("file-interrupted"));
        let mut list: Vec<Transfer> = (0..KEEP + 3)
            .map(|i| {
                let mut x = got.clone();
                x.id = format!("t{i}");
                x.state = if i == 0 { TransferState::Sending } else { TransferState::Done };
                x
            })
            .collect();
        prune(&mut list);
        assert_eq!(list.len(), KEEP);
        assert_eq!(list[0].id, "t0", "돌고 있는 것은 남는다");
    }
}
