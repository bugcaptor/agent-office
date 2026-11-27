// src-tauri/src/link/remote.rs
//
// 호스트 쪽: 붙은 머신들이 올린 캐릭터 모습(`Up::Live` 종류 `agents`, 클라이언트 `share.rs`가 몇 초마다
// 보낸다)을 들고 있다가 웹 상태판(`/api/state`)과 오피스 씬(`link-remote` 이벤트)에 내준다.
//
// 원격 캐릭터 id는 `기기id:캐릭터id`다(`d-1a2b3c4d:agent-…`). 기기 id는 호스트가 페어링 때 발급해
// 다시 페어링해도 그대로라 머신마다 겹치지 않는다. 캐릭터 이름은 머신끼리 겹쳐도 그대로 두고 머신 이름으로 가른다.
//
// 끊긴 머신은 마지막으로 본 명단(`link-remote.json`)만 꺼진 모습으로 보인다 — 세션·턴·권한 요청은 비운다.
// 대화 로그·이미지는 들고 있지 않고 볼 때 그 머신에 묻는다(`request`). 초상·커스텀 시트·미니미는 판(epoch ms)이
// 같은 동안 한 번 받은 것을 들고 있는다(`image`).
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::host::{LinkHost, REQUEST_TIMEOUT};
use super::now_ms;

pub const FILE: &str = "link-remote.json";
/// 클라이언트가 올리는 스냅숏 종류.
pub const KIND_AGENTS: &str = "agents";
/// 머신 하나에서 받는 캐릭터 상한(지나치게 큰 스냅숏을 막는다).
const MAX_CHARACTERS: usize = 200;
/// 머신 하나에서 받는 저장소 상한.
const MAX_REPOS: usize = 1000;
/// 명단에 남기는 키(끊긴 동안 보일 모습).
const ROSTER_KEYS: &[&str] = &["agentId", "name", "role", "cwd", "clockedOut", "portrait", "appearance", "shellAssist", "label"];

/// 오피스 씬이 보는 원격 캐릭터 하나.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCharacter {
    /// `기기id:캐릭터id`.
    pub id: String,
    /// 그 머신에서의 캐릭터 id.
    pub agent_id: String,
    pub name: String,
    pub role: String,
    /// 그 머신에서의 시작 폴더(그 머신 경로). 오피스 hover 카드에 말단 이름을 보인다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// 외형(`archetype`·`seed`·`colors`·`spriteUpdatedAt`·`minimiUpdatedAt`). 커스텀 시트·미니미는 판만 싣고
    /// 그림은 `load_sprite`·`load_minimi`(원격 id)로 그 머신에 묻는다.
    pub appearance: Value,
    /// 초상 판(epoch ms). 없으면 초상 없음. 그림은 `load_portrait`(원격 id)로 묻는다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portrait: Option<u64>,
    /// 세션 상태(`starting`·`running`·`exited`). 세션이 없거나 끊긴 머신이면 None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_state: Option<String>,
    /// 답을 기다림(권한 요청이 걸렸거나 턴이 사람을 기다림).
    pub needs_human: bool,
    pub clocked_out: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteMachine {
    pub device_id: String,
    pub name: String,
    pub online: bool,
    pub characters: Vec<RemoteCharacter>,
    /// 그 머신에 클론이 있는 저장소(정규화한 git 원격 주소). 업무를 맡기기 전에 본다. 끊겨도 마지막 목록을 둔다.
    pub repos: Vec<String>,
}

/// `link-remote` 이벤트와 `link_remote` 커맨드의 몸.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RemoteOffice {
    pub machines: Vec<RemoteMachine>,
}

/// 웹 상태판 `/api/state`에 더할 것.
#[derive(Debug, Default)]
pub struct RemoteWeb {
    pub characters: Vec<Value>,
    pub output_at: serde_json::Map<String, Value>,
    /// `[{id, name, online, count}]`(머신 이름 순).
    pub machines: Vec<Value>,
}

#[derive(Default)]
struct Machine {
    /// 이번 연결에서 받은 마지막 캐릭터들(끊기면 비운다).
    characters: Option<Vec<Value>>,
    output_at: serde_json::Map<String, Value>,
    /// 끊긴 동안 보일 명단.
    roster: Vec<Value>,
    /// 클론이 있는 저장소(마지막으로 받은 것).
    repos: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RosterFile {
    #[serde(default)]
    devices: HashMap<String, Vec<Value>>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    repos: HashMap<String, Vec<String>>,
}

pub type OfficeListener = Arc<dyn Fn(&RemoteOffice) + Send + Sync>;

pub struct RemoteView {
    host: Arc<LinkHost>,
    path: Option<PathBuf>,
    machines: Mutex<HashMap<String, Machine>>,
    listener: RwLock<Option<OfficeListener>>,
    last_office: Mutex<Option<RemoteOffice>>,
    /// 그림 캐시((기기, 캐릭터, 종류) → (판, PNG)). 종류는 `portrait`·`sprite`·`minimi`.
    images: Mutex<HashMap<(String, String, &'static str), (Value, Arc<Vec<u8>>)>>,
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// 원격 캐릭터 id(`기기id:캐릭터id`).
pub fn remote_id(device_id: &str, agent_id: &str) -> String {
    format!("{device_id}:{agent_id}")
}

/// 명단에 남길 모양.
fn roster_entry(c: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for key in ROSTER_KEYS {
        if let Some(v) = c.get(*key).filter(|v| !v.is_null()) {
            out.insert((*key).into(), v.clone());
        }
    }
    Value::Object(out)
}

/// 웹에 보일 원격 캐릭터 하나. 끊긴 머신이면 지금 상태(세션·턴·요청·대기열)를 비운다.
fn web_character(device_id: &str, name: &str, online: bool, c: &Value) -> Value {
    let mut c = c.clone();
    let Some(obj) = c.as_object_mut() else { return c };
    let agent = obj.get("agentId").and_then(Value::as_str).unwrap_or("").to_string();
    obj.insert("agentId".into(), json!(remote_id(device_id, &agent)));
    obj.insert("remote".into(), json!(true));
    obj.insert("machine".into(), json!({ "id": device_id, "name": name, "online": online }));
    // 외형은 오피스 씬만 쓴다.
    obj.remove("appearance");
    if !online {
        for key in ["session", "turn", "queue", "cli", "atPrompt", "work"] {
            obj.insert(key.into(), Value::Null);
        }
        obj.insert("asks".into(), json!([]));
    }
    c
}

fn office_character(device_id: &str, online: bool, c: &Value) -> RemoteCharacter {
    let agent_id = str_of(c, "agentId").to_string();
    let session_state = online
        .then(|| c.get("session").and_then(|s| s.get("state")).and_then(Value::as_str).map(str::to_string))
        .flatten();
    let asks = c.get("asks").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
    let waiting = c.get("turn").and_then(|t| t.get("phase")).and_then(Value::as_str) == Some("needs-human");
    RemoteCharacter {
        id: remote_id(device_id, &agent_id),
        agent_id,
        name: str_of(c, "name").to_string(),
        role: str_of(c, "role").to_string(),
        cwd: c.get("cwd").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_string),
        appearance: c.get("appearance").cloned().unwrap_or(Value::Null),
        portrait: c.get("portrait").and_then(Value::as_u64),
        needs_human: online && session_state.is_some() && (asks || waiting),
        session_state,
        clocked_out: c.get("clockedOut").and_then(Value::as_bool).unwrap_or(false),
    }
}

/// 앱의 호스트 쪽 원격 보기(이름 찾기용). `LinkState::new`가 건다.
static CURRENT: std::sync::OnceLock<RwLock<std::sync::Weak<RemoteView>>> = std::sync::OnceLock::new();

/// 이름 찾기에 쓸 원격 보기를 건다.
pub fn register(view: &Arc<RemoteView>) {
    *CURRENT.get_or_init(Default::default).write().unwrap() = Arc::downgrade(view);
}

/// 원격 캐릭터 id(`기기id:캐릭터id`)의 이름(`이름 · 머신`). 모르는 id면 None. 토의 상태·웹 상태판의 참가자 이름에 쓴다.
pub fn remote_name(id: &str) -> Option<String> {
    if !id.contains(':') {
        return None;
    }
    let view = CURRENT.get()?.read().unwrap().upgrade()?;
    view.office_view().machines.into_iter().find_map(|m| {
        m.characters.iter().find(|c| c.id == id).map(|c| format!("{} · {}", c.name, m.name))
    })
}

impl RemoteView {
    pub fn new(host: Arc<LinkHost>, path: Option<PathBuf>) -> Self {
        let roster = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<RosterFile>(&b).ok())
            .unwrap_or_default();
        let mut repos = roster.repos;
        let mut machines: HashMap<String, Machine> = roster
            .devices
            .into_iter()
            .map(|(id, roster)| {
                let repos = repos.remove(&id).unwrap_or_default();
                (id, Machine { roster, repos, ..Default::default() })
            })
            .collect();
        for (id, repos) in repos {
            machines.entry(id).or_default().repos = repos;
        }
        Self {
            host,
            path,
            machines: Mutex::new(machines),
            listener: RwLock::new(None),
            last_office: Mutex::new(None),
            images: Mutex::new(HashMap::new()),
        }
    }

    /// 오피스 씬에 알릴 곳(렌더러 이벤트). 바뀐 경우에만 부른다.
    pub fn set_listener(&self, listener: OfficeListener) {
        *self.listener.write().unwrap() = Some(listener);
    }

    fn save_roster(&self, machines: &HashMap<String, Machine>) {
        let Some(path) = &self.path else { return };
        let file = RosterFile {
            devices: machines.iter().filter(|(_, m)| !m.roster.is_empty()).map(|(id, m)| (id.clone(), m.roster.clone())).collect(),
            repos: machines.iter().filter(|(_, m)| !m.repos.is_empty()).map(|(id, m)| (id.clone(), m.repos.clone())).collect(),
        };
        if let Ok(bytes) = serde_json::to_vec_pretty(&file) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    /// 붙은 기기가 올린 것(호스트 이벤트 처리기가 부른다).
    pub fn ingest(&self, device_id: &str, kind: &str, data: &Value) {
        if kind == super::work::KIND_REPOS {
            return self.ingest_repos(device_id, data);
        }
        if kind != KIND_AGENTS {
            return;
        }
        let Some(list) = data.get("characters").and_then(Value::as_array) else { return };
        let characters: Vec<Value> = list
            .iter()
            .filter(|c| c.is_object() && !str_of(c, "agentId").is_empty())
            .take(MAX_CHARACTERS)
            .cloned()
            .collect();
        // 두 머신의 시계가 어긋나도 "마지막 출력 뒤 몇 초"가 맞게 호스트 시계로 옮긴다.
        let skew = data.get("now").and_then(Value::as_i64).map(|t| now_ms() as i64 - t).unwrap_or(0);
        let output_at = data
            .get("outputAt")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(agent, ms)| ms.as_i64().map(|ms| (agent.clone(), json!(ms + skew))))
                    .collect()
            })
            .unwrap_or_default();
        {
            let mut machines = self.machines.lock().unwrap();
            let m = machines.entry(device_id.to_string()).or_default();
            let roster: Vec<Value> = characters.iter().map(roster_entry).collect();
            let roster_changed = roster != m.roster;
            m.roster = roster;
            m.characters = Some(characters);
            m.output_at = output_at;
            if roster_changed {
                self.save_roster(&machines);
            }
        }
        self.publish();
    }

    fn ingest_repos(&self, device_id: &str, data: &Value) {
        let Some(list) = data.get("repos").and_then(Value::as_array) else { return };
        let mut repos: Vec<String> = list.iter().filter_map(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 512).take(MAX_REPOS).map(str::to_string).collect();
        repos.sort();
        repos.dedup();
        {
            let mut machines = self.machines.lock().unwrap();
            let m = machines.entry(device_id.to_string()).or_default();
            if m.repos == repos {
                return;
            }
            m.repos = repos;
            self.save_roster(&machines);
        }
        self.publish();
    }

    /// 그 머신에 클론이 있는 저장소(정규화한 원격 주소).
    pub fn repos_of(&self, device_id: &str) -> Vec<String> {
        self.machines.lock().unwrap().get(device_id).map(|m| m.repos.clone()).unwrap_or_default()
    }

    /// 페어링된 기기(이름 순)와 붙어 있는지.
    fn devices(&self) -> Vec<(String, String, bool)> {
        // 호스트 모드가 아니면 다른 머신을 보이지 않는다(명단은 남겨 둔다).
        if !self.host.enabled() {
            return Vec::new();
        }
        let mut devices: Vec<(String, String, bool)> =
            self.host.status().devices.into_iter().map(|d| (d.id, d.name, d.connected)).collect();
        devices.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()).then(a.0.cmp(&b.0)));
        devices
    }

    /// 끊긴 기기의 지금 상태와 해제된 기기를 버린다. 바뀌었으면 true.
    fn prune(&self, devices: &[(String, String, bool)]) {
        if !self.host.enabled() {
            return;
        }
        let mut machines = self.machines.lock().unwrap();
        let before = machines.len();
        machines.retain(|id, _| devices.iter().any(|(d, _, _)| d == id));
        let removed = machines.len() != before;
        for (id, _, connected) in devices {
            if !connected {
                if let Some(m) = machines.get_mut(id) {
                    m.characters = None;
                    m.output_at.clear();
                }
            }
        }
        if removed {
            self.save_roster(&machines);
            self.images.lock().unwrap().retain(|(d, _, _), _| machines.contains_key(d));
        }
    }

    /// 머신별로 (기기 id, 이름, 붙어 있음, 보일 캐릭터들, 출력 시각).
    fn each<T>(&self, mut f: impl FnMut(&str, &str, bool, &[Value], &serde_json::Map<String, Value>) -> T) -> Vec<T> {
        let devices = self.devices();
        self.prune(&devices);
        let machines = self.machines.lock().unwrap();
        devices
            .iter()
            .map(|(id, name, connected)| {
                let m = machines.get(id);
                let live = m.and_then(|m| m.characters.as_ref()).filter(|_| *connected);
                let empty = serde_json::Map::new();
                match live {
                    Some(chars) => f(id, name, true, chars, m.map(|m| &m.output_at).unwrap_or(&empty)),
                    None => f(id, name, false, m.map(|m| m.roster.as_slice()).unwrap_or(&[]), &empty),
                }
            })
            .collect()
    }

    pub fn web_view(&self) -> RemoteWeb {
        let mut out = RemoteWeb::default();
        let parts = self.each(|id, name, online, chars, output| {
            let characters: Vec<Value> = chars.iter().map(|c| web_character(id, name, online, c)).collect();
            let output: Vec<(String, Value)> = output.iter().map(|(a, ms)| (remote_id(id, a), ms.clone())).collect();
            let machine = json!({ "id": id, "name": name, "online": online, "count": characters.len() });
            (characters, output, machine)
        });
        for (characters, output, machine) in parts {
            out.characters.extend(characters);
            out.output_at.extend(output);
            out.machines.push(machine);
        }
        out
    }

    pub fn office_view(&self) -> RemoteOffice {
        let mut machines = self.each(|id, name, online, chars, _| RemoteMachine {
            device_id: id.to_string(),
            name: name.to_string(),
            online,
            characters: chars.iter().map(|c| office_character(id, online, c)).collect(),
            repos: Vec::new(),
        });
        {
            let known = self.machines.lock().unwrap();
            for m in &mut machines {
                m.repos = known.get(&m.device_id).map(|k| k.repos.clone()).unwrap_or_default();
            }
        }
        RemoteOffice { machines }
    }

    /// 오피스 씬 모습이 바뀌었으면 알린다(받은 스냅숏, 연결·해제·이름 바뀜마다 부른다).
    pub fn publish(&self) {
        let view = self.office_view();
        let mut last = self.last_office.lock().unwrap();
        if last.as_ref() == Some(&view) {
            return;
        }
        *last = Some(view.clone());
        drop(last);
        let listener = self.listener.read().unwrap().clone();
        if let Some(l) = listener {
            l(&view);
        }
    }

    /// `기기id:캐릭터id`를 가른다. 페어링된 기기가 아니면 None(이 머신의 캐릭터 id다).
    pub fn resolve(&self, id: &str) -> Option<(String, String)> {
        let (device, agent) = id.split_once(':')?;
        if agent.is_empty() || !self.host.enabled() || !self.host.status().devices.iter().any(|d| d.id == device) {
            return None;
        }
        Some((device.to_string(), agent.to_string()))
    }

    /// 그 머신에 묻는다.
    pub async fn request(&self, device_id: &str, kind: &str, data: Value) -> Result<Value, String> {
        self.host.request(device_id, kind, data, REQUEST_TIMEOUT).await
    }

    /// 제한 시간을 정해 묻는다(오래 걸리는 조작).
    pub async fn request_with(&self, device_id: &str, kind: &str, data: Value, timeout: std::time::Duration) -> Result<Value, String> {
        self.host.request(device_id, kind, data, timeout).await
    }

    /// 이 권한 요청 id가 걸린 머신(붙어 있는 머신이 마지막으로 올린 모습에서 찾는다).
    pub fn device_of_ask(&self, ask_id: &str) -> Option<String> {
        if !self.host.enabled() {
            return None;
        }
        let machines = self.machines.lock().unwrap();
        machines.iter().find_map(|(device, m)| {
            let found = m.characters.as_deref()?.iter().any(|c| {
                c.get("asks").and_then(Value::as_array).is_some_and(|asks| asks.iter().any(|a| str_of(a, "id") == ask_id))
            });
            found.then(|| device.clone())
        })
    }

    /// 원격 캐릭터 초상(PNG). 같은 판이면 다시 묻지 않는다.
    pub async fn portrait(&self, device_id: &str, agent_id: &str) -> Option<Arc<Vec<u8>>> {
        self.image(device_id, agent_id, "portrait").await.ok().flatten()
    }

    /// 원격 캐릭터 그림(PNG): `portrait`(초상)·`sprite`(커스텀 시트)·`minimi`(미니미). 같은 판이면 다시 묻지 않는다.
    /// 그 캐릭터에 그 그림이 없으면(판이 없거나 그 머신에 파일이 없거나 그 머신 앱이 그 종류를 모르면) `Ok(None)`,
    /// 물을 수 없으면(끊김·제한 시간) `Err`.
    pub async fn image(&self, device_id: &str, agent_id: &str, kind: &'static str) -> Result<Option<Arc<Vec<u8>>>, String> {
        let version = {
            let machines = self.machines.lock().unwrap();
            let Some(m) = machines.get(device_id) else { return Ok(None) };
            let list = m.characters.as_deref().unwrap_or(&m.roster);
            let Some(c) = list.iter().find(|c| str_of(c, "agentId") == agent_id) else { return Ok(None) };
            image_version(c, kind)
        };
        if version.is_null() {
            return Ok(None);
        }
        let key = (device_id.to_string(), agent_id.to_string(), kind);
        if let Some((v, png)) = self.images.lock().unwrap().get(&key) {
            if *v == version {
                return Ok(Some(png.clone()));
            }
        }
        let answer = match self.request(device_id, kind, json!({ "agentId": agent_id })).await {
            Ok(answer) => answer,
            Err(e) if e == "not-found" || e == "link-unsupported" => return Ok(None),
            Err(e) => return Err(e),
        };
        let Some(png) = decode_blob(&answer) else { return Ok(None) };
        let png = Arc::new(png);
        self.images.lock().unwrap().insert(key, (version, png.clone()));
        Ok(Some(png))
    }
}

/// 그 캐릭터 그림의 판(없으면 Null).
fn image_version(c: &Value, kind: &str) -> Value {
    let v = match kind {
        "portrait" => c.get("portrait"),
        "sprite" => c.get("appearance").and_then(|a| a.get("spriteUpdatedAt")),
        "minimi" => c.get("appearance").and_then(|a| a.get("minimiUpdatedAt")),
        _ => None,
    };
    v.cloned().unwrap_or(Value::Null)
}

/// `{data: base64}` 답의 바이트.
pub fn decode_blob(answer: &Value) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let data = answer.get("data").and_then(Value::as_str)?;
    base64::engine::general_purpose::STANDARD.decode(data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_with_device() -> (Arc<LinkHost>, String) {
        let host = Arc::new(LinkHost::new(None, "host-m".into(), Arc::new(|| "Host".into()), Arc::new(|| {})));
        host.set_enabled(true);
        let id = host.add_test_device("m-1", "Laptop");
        (host, id)
    }

    fn snapshot() -> Value {
        json!({
            "now": now_ms(),
            "outputAt": { "a1": now_ms() },
            "characters": [{
                "agentId": "a1", "name": "Ada", "role": "dev", "portrait": 5, "cwd": "/Users/ada/dev/app",
                "appearance": { "seed": "s1", "archetype": "human", "spriteUpdatedAt": 7 },
                "session": { "sessionId": "s", "state": "running" },
                "turn": { "phase": "needs-human" },
                "asks": [{ "id": "k1" }],
                "queue": { "items": [] },
            }]
        })
    }

    #[test]
    fn remote_ids_are_prefixed_and_resolve_only_for_paired_devices() {
        let (host, dev) = host_with_device();
        let view = RemoteView::new(host, None);
        assert_eq!(view.resolve(&format!("{dev}:a1")), Some((dev.clone(), "a1".into())));
        assert_eq!(view.resolve("a1"), None, "콜론이 없으면 이 머신의 캐릭터");
        assert_eq!(view.resolve("d-unknown:a1"), None);
        assert_eq!(view.resolve(&format!("{dev}:")), None);
    }

    #[test]
    fn offline_machines_show_the_roster_without_live_state() {
        let (host, dev) = host_with_device();
        let view = RemoteView::new(host.clone(), None);
        // 붙어 있지 않은 동안 받은 것(연결이 바로 끊겼다)은 명단으로만 남는다.
        view.ingest(&dev, KIND_AGENTS, &snapshot());
        let web = view.web_view();
        assert_eq!(web.machines, vec![json!({ "id": dev, "name": "Laptop", "online": false, "count": 1 })]);
        let c = &web.characters[0];
        assert_eq!(c["agentId"], json!(format!("{dev}:a1")));
        assert_eq!(c["machine"]["online"], json!(false));
        assert_eq!(c["remote"], json!(true));
        assert!(c["session"].is_null() && c["turn"].is_null());
        assert_eq!(c["asks"], json!([]));
        assert!(c.get("appearance").is_none());
        assert!(web.output_at.is_empty());
        let office = view.office_view();
        let oc = &office.machines[0].characters[0];
        assert_eq!(oc.session_state, None);
        assert!(!oc.needs_human);
        assert_eq!(oc.appearance["seed"], json!("s1"));
        // 끊긴 동안에도 초상·커스텀 시트 판과 시작 폴더는 명단에 남는다(받아 둔 그림·폴더 이름을 보인다).
        assert_eq!(oc.portrait, Some(5));
        assert_eq!(oc.appearance["spriteUpdatedAt"], json!(7));
        assert_eq!(oc.cwd.as_deref(), Some("/Users/ada/dev/app"));
        assert_eq!(image_version(&snapshot()["characters"][0], "sprite"), json!(7));
        assert!(image_version(&snapshot()["characters"][0], "minimi").is_null());
    }

    #[test]
    fn online_machines_show_live_state_and_shift_output_times() {
        let (host, dev) = host_with_device();
        host.set_test_connected(&dev, true);
        let view = RemoteView::new(host.clone(), None);
        let mut snap = snapshot();
        // 클라이언트 시계가 10초 늦다.
        let client_now = now_ms() as i64 - 10_000;
        snap["now"] = json!(client_now);
        snap["outputAt"] = json!({ "a1": client_now - 1_000 });
        view.ingest(&dev, KIND_AGENTS, &snap);
        let web = view.web_view();
        let c = &web.characters[0];
        assert_eq!(c["machine"]["online"], json!(true));
        assert_eq!(c["session"]["state"], json!("running"));
        assert_eq!(c["asks"][0]["id"], json!("k1"));
        let shifted = web.output_at[&format!("{dev}:a1")].as_i64().unwrap();
        assert!((shifted - (now_ms() as i64 - 1_000)).abs() < 2_000, "호스트 시계로 옮긴다: {shifted}");
        let oc = view.office_view().machines[0].characters[0].clone();
        assert_eq!(oc.session_state.as_deref(), Some("running"));
        assert!(oc.needs_human);
        // 끊기면 지금 상태를 버리고 명단만 남는다.
        host.set_test_connected(&dev, false);
        let web = view.web_view();
        assert_eq!(web.characters[0]["machine"]["online"], json!(false));
        assert!(web.characters[0]["session"].is_null());
    }

    #[test]
    fn publish_notifies_only_on_change_and_roster_persists() {
        let dir = std::env::temp_dir().join(format!("link-remote-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (host, dev) = host_with_device();
        host.set_test_connected(&dev, true);
        let view = RemoteView::new(host.clone(), Some(dir.join(FILE)));
        let seen = Arc::new(Mutex::new(Vec::<RemoteOffice>::new()));
        let s = seen.clone();
        view.set_listener(Arc::new(move |o| s.lock().unwrap().push(o.clone())));
        view.ingest(&dev, KIND_AGENTS, &snapshot());
        view.ingest(&dev, KIND_AGENTS, &snapshot());
        assert_eq!(seen.lock().unwrap().len(), 1, "같은 모습이면 다시 알리지 않는다");
        // 앱을 다시 켜면 끊긴 머신의 명단을 읽는다.
        host.set_test_connected(&dev, false);
        let again = RemoteView::new(host.clone(), Some(dir.join(FILE)));
        let office = again.office_view();
        assert_eq!(office.machines[0].characters[0].name, "Ada");
        assert!(!office.machines[0].online);
        // 해제한 기기는 사라진다.
        host.revoke(&dev);
        assert!(again.office_view().machines.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshots_without_characters_or_ids_are_ignored() {
        let (host, dev) = host_with_device();
        host.set_test_connected(&dev, true);
        let view = RemoteView::new(host, None);
        view.ingest(&dev, KIND_AGENTS, &json!({ "characters": "nope" }));
        view.ingest(&dev, "report", &snapshot());
        assert!(view.office_view().machines[0].characters.is_empty());
        view.ingest(&dev, KIND_AGENTS, &json!({ "characters": [{ "name": "no id" }, { "agentId": "a2", "name": "Bo" }] }));
        let chars = &view.office_view().machines[0].characters;
        assert_eq!(chars.len(), 1);
        assert_eq!(chars[0].agent_id, "a2");
    }
}
