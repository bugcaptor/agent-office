// src-tauri/src/link/files.rs
//
// 파일 보내기(`crate::files`)의 연동 배선.
//
// 호스트가 모든 머신 간 전송을 진행한다. 클라이언트끼리는 호스트가 청크를 디스크에 두지 않고 흘려 보낸다.
// - 클라이언트 → 호스트(`Up::Live`): `file-caps`(파일 받기 여부, 붙을 때마다), `file-send`(매니페스트가 준비된
//   전송), `file-resume`, `file-cancel`.
// - 호스트 → 클라이언트 일(`Command`): `file-update`(전송 상태), `file-targets`(연동 전체 대상 목록).
// - 호스트 → 클라이언트 물음(`Request`): 받는 쪽 `file-offer`·`file-chunk`·`file-finish`·`file-abort`,
//   원본 쪽 `file-read`, 웹 상태판의 머신 간 지시 `file-browse`·`file-prepare`, 받은 파일 내려받기 `file-saved`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock, Weak};
use std::time::Duration;

use base64::Engine as _;
use serde_json::{json, Value};

use super::{LinkMode, LinkState};
use crate::files::model::{Party, Target, Transfer, TransferState};
use crate::files::{FilesState, SendRequest};

pub const LIVE_CAPS: &str = "file-caps";
pub const LIVE_SEND: &str = "file-send";
pub const LIVE_RESUME: &str = "file-resume";
pub const LIVE_CANCEL: &str = "file-cancel";
pub const CMD_UPDATE: &str = "file-update";
pub const CMD_TARGETS: &str = "file-targets";
pub const REQ_OFFER: &str = "file-offer";
pub const REQ_CHUNK: &str = "file-chunk";
pub const REQ_FINISH: &str = "file-finish";
pub const REQ_ABORT: &str = "file-abort";
pub const REQ_READ: &str = "file-read";
pub const REQ_BROWSE: &str = "file-browse";
pub const REQ_PREPARE: &str = "file-prepare";
pub const REQ_SAVED: &str = "file-saved";
pub const REQ_LIST: &str = "file-list";

const TICK: Duration = Duration::from_secs(2);
/// 웹 탐색에서 한 폴더에 보일 항목 수.
const MAX_ENTRIES: usize = 2000;

static CURRENT: OnceLock<RwLock<Weak<FilesState>>> = OnceLock::new();

pub fn register(files: &Arc<FilesState>) {
    *CURRENT.get_or_init(Default::default).write().unwrap() = Arc::downgrade(files);
}

pub fn current() -> Option<Arc<FilesState>> {
    CURRENT.get()?.read().unwrap().upgrade()
}

/// 클라이언트: 호스트에 파일 받기 여부를 알린다.
pub fn announce(link: &LinkState, files: &FilesState) {
    if link.config().mode == LinkMode::Client {
        link.client.send_live(LIVE_CAPS, json!({ "receive": files.config().receive_enabled }));
    }
}

/// 호스트: 붙은(페어링된) 머신과 그 캐릭터.
pub fn host_remote_targets(link: &LinkState, files: &FilesState) -> Vec<Target> {
    let caps = files.device_receive.lock().unwrap().clone();
    let office = link.remote.office_view();
    let mut out = Vec::new();
    for d in link.host.status().devices {
        let receive = d.connected && caps.get(&d.id).copied().unwrap_or(false);
        out.push(Target {
            machine_id: d.machine_id.clone(),
            machine_name: d.name.clone(),
            agent_id: None,
            agent_name: None,
            local: false,
            online: d.connected,
            receive,
        });
        if let Some(m) = office.machines.iter().find(|m| m.device_id == d.id) {
            for c in &m.characters {
                out.push(Target {
                    machine_id: d.machine_id.clone(),
                    machine_name: d.name.clone(),
                    agent_id: Some(c.agent_id.clone()),
                    agent_name: Some(c.name.clone()),
                    local: false,
                    online: d.connected,
                    receive,
                });
            }
        }
    }
    out
}

fn device_machine(link: &LinkState, device: &str) -> Option<String> {
    link.host.status().devices.into_iter().find(|d| d.id == device).map(|d| d.machine_id)
}

fn transfer_of(data: &Value) -> Option<Transfer> {
    serde_json::from_value::<Transfer>(data.get("transfer")?.clone()).ok().map(|t| Transfer { sources: Vec::new(), cleanup: None, ..t })
}

/// 호스트: 붙은 기기가 올린 것(`LinkState`의 이벤트 처리기가 부른다).
pub fn host_event(device: &str, kind: &str, data: &Value) {
    if !kind.starts_with("file-") {
        return;
    }
    let Some(files) = current() else { return };
    let Some(link) = files.link() else { return };
    let Some(machine) = device_machine(&link, device) else { return };
    match kind {
        LIVE_CAPS => {
            let on = data.get("receive").and_then(Value::as_bool).unwrap_or(false);
            files.device_receive.lock().unwrap().insert(device.to_string(), on);
            files.write_targets();
        }
        LIVE_SEND | LIVE_RESUME => {
            let Some(mut t) = transfer_of(data) else { return };
            // 보낸 머신만 자기 전송을 올릴 수 있다.
            if t.from.machine_id != machine {
                return;
            }
            let resume = kind == LIVE_RESUME;
            if files.is_running(&t.id) {
                return;
            }
            match files.get(&t.id) {
                Some(old) if old.state.is_final() => return,
                // 호스트가 진행 상태를 더 잘 안다.
                Some(old) if resume => t.progress = old.progress,
                _ => {}
            }
            if resume {
                t.state = TransferState::Failed;
            }
            files.upsert(t.clone(), true);
            let id = t.id.clone();
            tauri::async_runtime::spawn(async move { crate::files::engine::run(files, id, resume).await });
        }
        LIVE_CANCEL => {
            let Some(id) = data.get("id").and_then(Value::as_str) else { return };
            let Some(t) = files.get(id) else { return };
            if t.from.machine_id == machine || t.to.machine_id == machine {
                let _ = files.cancel(id);
            }
        }
        _ => {}
    }
}

/// 클라이언트: 호스트가 보낸 일.
pub fn client_command(kind: &str, data: &Value) {
    let Some(files) = current() else { return };
    match kind {
        CMD_UPDATE => {
            let Some(t) = transfer_of(data) else { return };
            // 이 머신이 끼지 않은 전송은 기록하지 않는다.
            if t.from.machine_id != files.machine_id && t.to.machine_id != files.machine_id {
                return;
            }
            files.upsert(t.clone(), true);
            files.cleanup_if_final(&t);
        }
        CMD_TARGETS => {
            let targets: Vec<Target> = serde_json::from_value(data.get("targets").cloned().unwrap_or_default()).unwrap_or_default();
            *files.host_targets.lock().unwrap() = targets;
            files.write_targets();
        }
        _ => {}
    }
}

fn str_of<'a>(data: &'a Value, key: &str) -> Result<&'a str, String> {
    data.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| "bad-request".into())
}

fn num_of(data: &Value, key: &str) -> Result<u64, String> {
    data.get(key).and_then(Value::as_u64).ok_or_else(|| "bad-request".into())
}

/// 웹 탐색·머신 간 지시가 고를 수 있는 곳: 받은 파일 폴더와 캐릭터 작업 폴더.
pub fn allowed_roots(files: &FilesState) -> Vec<(String, PathBuf)> {
    let mut out = vec![("받은 파일".to_string(), files.receive_root())];
    if let Some(ctl) = files.ctl() {
        for p in ctl.store.load().agents {
            if let Some(cwd) = p.cwd.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                let path = PathBuf::from(cwd);
                if !out.iter().any(|(_, r)| r == &path) {
                    out.push((p.name.clone(), path));
                }
            }
        }
    }
    out
}

fn inside_allowed(files: &FilesState, path: &Path) -> bool {
    let Ok(path) = path.canonicalize() else { return false };
    allowed_roots(files).iter().filter_map(|(_, r)| r.canonicalize().ok()).any(|r| path.starts_with(&r))
}

/// 폴더 보기(웹 머신 간 지시). `path`가 없으면 고를 수 있는 곳 목록.
pub fn browse(files: &FilesState, path: Option<&str>) -> Result<Value, String> {
    let roots: Vec<Value> = allowed_roots(files)
        .into_iter()
        .map(|(name, p)| json!({ "name": name, "path": p.to_string_lossy(), "exists": p.is_dir() }))
        .collect();
    let Some(path) = path.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(json!({ "roots": roots }));
    };
    let dir = PathBuf::from(path);
    if !inside_allowed(files, &dir) {
        return Err("file-path-not-allowed".into());
    }
    let mut entries: Vec<Value> = std::fs::read_dir(&dir)
        .map_err(|e| format!("file-read-failed: {e}"))?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let ft = e.file_type().ok()?;
            if ft.is_symlink() {
                return None;
            }
            let meta = e.metadata().ok()?;
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".incoming" {
                return None;
            }
            Some(json!({
                "name": name,
                "path": e.path().to_string_lossy(),
                "dir": ft.is_dir(),
                "size": if ft.is_dir() { 0 } else { meta.len() },
                "mtime": crate::files::manifest::mtime_ms(&meta),
            }))
        })
        .collect();
    entries.sort_by(|a, b| {
        b["dir"].as_bool().cmp(&a["dir"].as_bool()).then_with(|| {
            a["name"].as_str().unwrap_or("").to_lowercase().cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
        })
    });
    let truncated = entries.len() > MAX_ENTRIES;
    entries.truncate(MAX_ENTRIES);
    let parent = dir.parent().filter(|p| inside_allowed(files, p)).map(|p| p.to_string_lossy().into_owned());
    Ok(json!({ "roots": roots, "path": dir.to_string_lossy(), "parent": parent, "entries": entries, "truncated": truncated }))
}

/// 다른 머신(웹)의 지시로 이 머신 파일을 보낸다. 받은 파일·캐릭터 작업 폴더 안만.
pub fn prepare(files: &Arc<FilesState>, data: &Value) -> Result<Value, String> {
    let paths: Vec<String> = serde_json::from_value(data.get("paths").cloned().unwrap_or_default()).map_err(|_| "bad-request".to_string())?;
    if paths.is_empty() {
        return Err("file-no-paths".into());
    }
    for p in &paths {
        if !inside_allowed(files, Path::new(p)) {
            return Err(format!("file-path-not-allowed: {p}"));
        }
    }
    let to: Party = serde_json::from_value(data.get("to").cloned().unwrap_or_default()).map_err(|_| "bad-request".to_string())?;
    let req = SendRequest {
        target: String::new(),
        machine_id: Some(to.machine_id),
        agent_id: to.agent_id,
        paths,
        note: data.get("note").and_then(Value::as_str).map(str::to_string),
        notify: data.get("notify").and_then(Value::as_bool).unwrap_or(false),
        from_agent: None,
        via: data.get("via").and_then(Value::as_str).unwrap_or("web").to_string(),
        id: None,
        cleanup: None,
    };
    let t = files.send(req)?;
    Ok(json!({ "transfer": t }))
}

/// 받은 전송의 파일 한 조각(웹 내려받기). 받은 파일 폴더 안의 기록된 파일만.
pub fn read_saved(files: &FilesState, id: &str, index: usize, offset: u64, len: u64) -> Result<(Vec<u8>, u64, String), String> {
    let t = files.get(id).ok_or("file-transfer-unknown")?;
    if t.state != TransferState::Done || t.to.machine_id != files.machine_id {
        return Err("not-found".into());
    }
    let path = crate::files::receive::saved_path(&t, index).ok_or("not-found")?;
    let meta = std::fs::metadata(&path).map_err(|_| "not-found".to_string())?;
    let size = meta.len();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if offset > size {
        return Err("bad-request".into());
    }
    let len = len.min(size - offset).min(crate::files::model::CHUNK * 2);
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf).map_err(|e| e.to_string())?;
    Ok((buf, size, name))
}

/// 클라이언트: 호스트의 물음 중 파일 보내기 것에 답한다(아니면 None).
pub async fn answer(kind: &str, data: &Value) -> Option<Result<Value, String>> {
    if !kind.starts_with("file-") {
        return None;
    }
    let files = match current() {
        Some(f) => f,
        None => return Some(Err("link-unsupported".into())),
    };
    Some(answer_with(&files, kind, data).await)
}

async fn answer_with(files: &Arc<FilesState>, kind: &str, data: &Value) -> Result<Value, String> {
    match kind {
        REQ_OFFER => {
            let mut t = transfer_of(data).ok_or("bad-request")?;
            let resume: Vec<u64> = serde_json::from_value(data.get("resume").cloned().unwrap_or_default()).unwrap_or_default();
            t.state = TransferState::Sending;
            let offsets = files.accept_offer(t.clone(), resume).await?;
            files.upsert(t, true);
            Ok(json!({ "offsets": offsets }))
        }
        REQ_CHUNK => {
            let id = str_of(data, "id")?.to_string();
            let index = num_of(data, "index")? as usize;
            let offset = num_of(data, "offset")?;
            let bytes = base64::engine::general_purpose::STANDARD.decode(str_of(data, "data")?).map_err(|_| "bad-request".to_string())?;
            files.write_chunk(id, index, offset, bytes).await?;
            Ok(json!({}))
        }
        REQ_FINISH => {
            let saved = files.finish_receive(str_of(data, "id")?).await?;
            Ok(json!({ "saved": saved }))
        }
        REQ_ABORT => {
            let id = str_of(data, "id")?;
            files.abort_receive(id);
            files.update(id, |t| {
                t.state = TransferState::Cancelled;
            });
            Ok(json!({}))
        }
        REQ_READ => {
            let id = str_of(data, "id")?;
            let index = num_of(data, "index")? as usize;
            let (offset, len) = (num_of(data, "offset")?, num_of(data, "len")?);
            let t = files.get(id).ok_or("file-transfer-unknown")?;
            if t.from.machine_id != files.machine_id {
                return Err("file-not-source".into());
            }
            let source = t.sources.get(index).cloned().ok_or("file-source-missing")?;
            let entry = t.files.get(index).cloned().ok_or("bad-request")?;
            let bytes = tokio::task::spawn_blocking(move || crate::files::manifest::read_chunk(Path::new(&source), &entry, offset, len))
                .await
                .map_err(|e| e.to_string())??;
            Ok(json!({ "data": base64::engine::general_purpose::STANDARD.encode(bytes) }))
        }
        REQ_BROWSE => {
            let files = files.clone();
            let path = data.get("path").and_then(Value::as_str).map(str::to_string);
            tokio::task::spawn_blocking(move || browse(&files, path.as_deref())).await.map_err(|e| e.to_string())?
        }
        REQ_PREPARE => prepare(files, data),
        REQ_SAVED => {
            let id = str_of(data, "id")?.to_string();
            let index = num_of(data, "index")? as usize;
            let (offset, len) = (num_of(data, "offset")?, num_of(data, "len")?);
            let files = files.clone();
            let (bytes, size, name) =
                tokio::task::spawn_blocking(move || read_saved(&files, &id, index, offset, len)).await.map_err(|e| e.to_string())??;
            Ok(json!({ "data": base64::engine::general_purpose::STANDARD.encode(bytes), "size": size, "name": name }))
        }
        REQ_LIST => Ok(json!({ "transfers": files.list(), "status": files.status() })),
        _ => Err("link-unsupported".into()),
    }
}

/// 대상 목록 파일 쓰기, 호스트의 대상 목록 내려 보내기, 클라이언트의 받기 여부 알리기를 돌린다.
pub fn install(link: &Arc<LinkState>, files: &Arc<FilesState>) {
    register(files);
    let link = Arc::downgrade(link);
    let files = Arc::downgrade(files);
    tauri::async_runtime::spawn(async move {
        // 기기 id → (붙은 시각, 보낸 목록).
        let mut sent: HashMap<String, (Option<u64>, String)> = HashMap::new();
        let mut announced: Option<u64> = None;
        loop {
            tokio::time::sleep(TICK).await;
            let (Some(link), Some(files)) = (link.upgrade(), files.upgrade()) else { break };
            files.write_targets();
            match link.config().mode {
                LinkMode::Host => {
                    let devices: Vec<_> = link.host.status().devices.into_iter().filter(|d| d.connected).collect();
                    sent.retain(|id, _| devices.iter().any(|d| &d.id == id));
                    if devices.is_empty() {
                        continue;
                    }
                    let targets = files.targets();
                    let text = serde_json::to_string(&targets).unwrap_or_default();
                    for d in devices {
                        if sent.get(&d.id).is_some_and(|(at, t)| *at == d.connected_at && *t == text) {
                            continue;
                        }
                        if link.host.send(&d.id, CMD_TARGETS, json!({ "targets": targets })).is_ok() {
                            sent.insert(d.id.clone(), (d.connected_at, text.clone()));
                        }
                    }
                }
                LinkMode::Client => {
                    let conn = link.client.connection();
                    if conn.is_some() && conn != announced {
                        announce(&link, &files);
                        announced = conn;
                    } else if conn.is_none() {
                        announced = None;
                    }
                }
                LinkMode::Off => {}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::manifest;
    use crate::link::protocol::Down;

    async fn wait_state(files: &FilesState, id: &str) -> Transfer {
        for _ in 0..400 {
            if let Some(t) = files.get(id).filter(|t| !t.state.is_active()) {
                return t;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("transfer {id} did not finish: {:?}", files.get(id).map(|t| (t.state, t.error)));
    }

    fn receiving(dir: &Path, machine: &str) -> Arc<FilesState> {
        let f = FilesState::new(Some(dir.join(machine)), machine.into());
        f.configure(crate::files::FilesConfig { receive_enabled: true, receive_dir: Some(dir.join(format!("recv-{machine}")).to_string_lossy().into()) }).unwrap();
        f
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_sends_to_a_client_and_pulls_from_it_in_chunks() {
        let tmp = tempfile::tempdir().unwrap();
        let link = LinkState::new(None, Arc::new(|| "host".to_string()), Arc::new(crate::link::fake::NoLinkEvents));
        link.configure(crate::link::LinkConfig { mode: LinkMode::Host, host_address: String::new() }).unwrap();
        let device = link.host.add_test_device("mc", "client-pc");
        let mut rx = link.host.set_test_connected(&device, true).unwrap();
        let host = receiving(tmp.path(), "mh");
        host.set_link(&link);
        host.device_receive.lock().unwrap().insert(device.clone(), true);
        let client = receiving(tmp.path(), "mc");
        {
            let (link, client, device) = (link.clone(), client.clone(), device.clone());
            tokio::spawn(async move {
                while let Some(down) = rx.recv().await {
                    if let Down::Request { id, kind, data } = down {
                        let (link, client, device) = (link.clone(), client.clone(), device.clone());
                        tokio::spawn(async move {
                            let answer = answer_with(&client, &kind, &data).await;
                            link.host.test_reply(&device, id, answer);
                        });
                    }
                }
            });
        }
        // 호스트 → 클라이언트: 여러 청크짜리 파일과 폴더.
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("proj/sub")).unwrap();
        let big: Vec<u8> = (0..(crate::files::model::CHUNK as usize * 2 + 123)).map(|i| (i * 7 % 251) as u8).collect();
        std::fs::write(src.join("big.bin"), &big).unwrap();
        std::fs::write(src.join("proj/sub/a.txt"), b"hi").unwrap();
        let req = SendRequest {
            target: "client-pc".into(),
            paths: vec![src.join("big.bin").to_string_lossy().into(), src.join("proj").to_string_lossy().into()],
            ..Default::default()
        };
        let t = host.send(req).unwrap();
        let done = wait_state(&host, &t.id).await;
        assert_eq!(done.state, TransferState::Done, "{:?}", done.error);
        assert_eq!(done.saved.len(), 2);
        assert_eq!(std::fs::read(&done.saved[0]).unwrap(), big);
        assert_eq!(std::fs::read(Path::new(&done.saved[1]).join("sub/a.txt")).unwrap(), b"hi");
        assert!(done.saved[0].contains("recv-mc"), "클라이언트 받은 파일 폴더에 저장: {}", done.saved[0]);
        // 클라이언트도 받은 기록을 가진다.
        assert_eq!(client.get(&t.id).unwrap().saved, done.saved);

        // 클라이언트 → 호스트: 클라이언트가 만든 매니페스트로 호스트가 청크를 읽어 온다.
        let m = manifest::build(&[src.join("big.bin")], &|| false).unwrap();
        let mut up = client.get(&t.id).unwrap();
        up.id = "f-up".into();
        up.from = crate::files::Party { machine_id: "mc".into(), machine_name: "client-pc".into(), ..Default::default() };
        up.to = crate::files::Party { machine_id: "mh".into(), machine_name: "host".into(), ..Default::default() };
        up.items = m.items.clone();
        up.total_bytes = m.total();
        up.progress = vec![0; m.files.len()];
        up.sources = m.sources.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        up.files = m.files;
        up.dirs = vec![];
        up.saved = vec![];
        up.state = TransferState::Preparing;
        client.upsert(up.clone(), false);
        host.upsert(up.public(), false);
        crate::files::engine::run(host.clone(), "f-up".into(), false).await;
        let got = host.get("f-up").unwrap();
        assert_eq!(got.state, TransferState::Done, "{:?}", got.error);
        assert_eq!(std::fs::read(&got.saved[0]).unwrap(), big);
        assert!(got.saved[0].contains("recv-mh") && got.saved[0].contains("client-pc"));

        // 끊긴 머신에는 보내지 않는다.
        link.host.set_test_connected(&device, false);
        let req = SendRequest { target: "client-pc".into(), paths: vec![src.join("big.bin").to_string_lossy().into()], ..Default::default() };
        assert!(host.send(req).unwrap_err().starts_with("file-target-offline"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn local_transfers_and_receive_off_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let files = FilesState::new(Some(tmp.path().join("d")), "m1".into());
        std::fs::write(tmp.path().join("x.txt"), b"x").unwrap();
        let req = || SendRequest { target: crate::webdash::computer_name(), paths: vec![tmp.path().join("x.txt").to_string_lossy().into()], ..Default::default() };
        assert!(files.send(req()).unwrap_err().starts_with("file-target-receive-off"));
        files.configure(crate::files::FilesConfig { receive_enabled: true, receive_dir: Some(tmp.path().join("r").to_string_lossy().into()) }).unwrap();
        let t = files.send(req()).unwrap();
        let done = wait_state(&files, &t.id).await;
        assert_eq!(done.state, TransferState::Done, "{:?}", done.error);
        assert_eq!(std::fs::read(&done.saved[0]).unwrap(), b"x");
        // 두 번 보내면 덮어쓰지 않는다.
        let t2 = files.send(req()).unwrap();
        let done2 = wait_state(&files, &t2.id).await;
        assert!(done2.saved[0].ends_with("x (2).txt"), "{}", done2.saved[0]);
    }
}
