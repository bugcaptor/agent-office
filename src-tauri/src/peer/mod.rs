// src-tauri/src/peer/mod.rs
//
// 에이전트끼리 상태 조회·대화(`office-peer`). 세션 안 에이전트가 이 머신·연동한 다른 머신 캐릭터의 지금 상태와
// 대화 로그를 보고, 메시지를 넣어 답을 받고, 세션 띄우기·이어하기·중단·대기열 빼기를 한다. 사람 화면은 없다 —
// 주고받은 글은 양쪽 세션 대화에만 남는다. 권한 요청 답은 하지 않는다.
//
// - 머신마다 **에이전트끼리 대화**를 켜야 한다(`peer.json`, 기본 꺼짐). 꺼진 머신은 이 기능으로 닿지 않고, 그
//   머신 세션에는 안내도 실리지 않는다.
// - 대화: `ask`가 상대 세션 보낼 대기열(파일 알림과 같은 길)에 글과 대화 id를 넣는다. 답은 받은 에이전트가
//   `office-peer reply <id> "…"`를 실행해야 성립한다(전사로 짐작하지 않는다). 보낸 CLI는 답을 기다리다가 상한을
//   넘기면 끝나고, 늦게 온 답은 보낸 세션 대기열에 넣는다(`REPLIES_DIR` 파일을 CLI가 몇 초 안에 안 가져가면).
// - 폭주 막기: 에이전트마다 시간당 보내기 수, 같은 상대에게 시간당 보내기 수, 답하는 중에 되묻는 깊이를 막고,
//   나에게 답을 기다리는 상대에게는 묻지 말고 답하라고 돌려보낸다(서로 기다리는 교착 방지).
// - 길: 같은 머신은 바로, 다른 머신은 연동으로(클라이언트끼리는 호스트 중계, `link/peer.rs`). 대상 이름 풀기는
//   파일 보내기 대상 목록(`files.targets()`, 호스트가 내려 준 연동 전체 명단)을 쓴다.
//
// 파일(데이터 폴더): `peer.json` 설정, `peer-threads.json` 최근 대화(CLI가 읽음), `peer-requests/` CLI 요청,
// `peer-replies/` 보낸 CLI가 기다리는 답.
pub mod cli;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::control::ControlContext;
use crate::files::model::Party;
use crate::link::{LinkMode, LinkState};
use crate::webdash::WebDashState;

pub const CONFIG_FILE: &str = "peer.json";
pub const THREADS_FILE: &str = "peer-threads.json";
pub const REQUESTS_DIR: &str = "peer-requests";
pub const REPLIES_DIR: &str = "peer-replies";

/// 기록에 남길 대화 수.
const KEEP: usize = 200;
/// 에이전트 하나가 한 시간에 보낼 수 있는 메시지.
pub const MAX_SENDS_PER_HOUR: usize = 30;
/// 같은 상대에게 한 시간에 보낼 수 있는 메시지.
pub const MAX_PAIR_PER_HOUR: usize = 10;
/// 답하는 중에 다시 묻는 깊이(A가 B에게, B가 답하려고 C에게, C가 D에게 = 3).
pub const MAX_DEPTH: u32 = 3;
/// 답하지 않은 받은 대화를 깊이 계산에 넣는 시간.
const DEPTH_WINDOW_MS: u64 = 30 * 60 * 1000;
const HOUR_MS: u64 = 60 * 60 * 1000;
/// 보낸 CLI가 답 파일을 가져가길 기다리는 시간. 넘기면 보낸 세션 대기열에 넣는다.
const CLI_CLAIM: Duration = Duration::from_secs(5);
/// 다른 머신에 묻는 제한 시간(세션을 띄워 첫 프롬프트를 넣는 데 수십 초 걸린다).
pub const ROUTE_TIMEOUT: Duration = crate::webdash::REMOTE_OP_TIMEOUT;
/// 명단을 모을 때 머신 하나를 기다리는 시간.
const ROSTER_TIMEOUT: Duration = Duration::from_secs(6);
/// 기록에 남기는 글 길이(글자).
const KEEP_CHARS: usize = 4000;
/// `log`가 돌려주는 항목 상한과 항목당 글자 상한.
const MAX_LOG_ITEMS: usize = 40;
const LOG_ITEM_CHARS: usize = 2000;

/// 상대에게 시킬 수 있는 조작(웹 상태판 조작 이름). 권한 답·끝내기·재시작은 넣지 않는다.
const AGENT_OPS: &[&str] = &["launch", "resume", "interrupt", "queue-cancel"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PeerConfig {
    /// 이 머신 에이전트가 다른 캐릭터와 조회·대화하고, 다른 캐릭터가 이 머신 캐릭터에게 닿는다.
    #[serde(default)]
    pub enabled: bool,
}

/// 대화 하나(물음 하나와 답 하나).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    pub from: Party,
    pub to: Party,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replied_at: Option<u64>,
    #[serde(default)]
    pub depth: u32,
    /// 보낸 에이전트가 이 머신에 있다.
    #[serde(default)]
    pub outgoing: bool,
    /// 받은 에이전트가 이 머신에 있다.
    #[serde(default)]
    pub incoming: bool,
}

pub struct PeerState {
    dir: Option<PathBuf>,
    pub machine_id: String,
    config: Mutex<PeerConfig>,
    threads: Mutex<Vec<Thread>>,
    ctx: OnceLock<(Arc<ControlContext>, Arc<WebDashState>)>,
    link: OnceLock<std::sync::Weak<LinkState>>,
}

static CURRENT: OnceLock<RwLock<std::sync::Weak<PeerState>>> = OnceLock::new();

pub fn current() -> Option<Arc<PeerState>> {
    CURRENT.get()?.read().unwrap().upgrade()
}

/// 세션에 `office-peer` 안내를 실을까: 이 머신이 켰을 때.
pub fn instructions_wanted(data: &std::path::Path) -> bool {
    read_config(data).enabled
}

fn read_config(data: &std::path::Path) -> PeerConfig {
    std::fs::read(data.join(CONFIG_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn now_ms() -> u64 {
    crate::types::now_ms()
}

fn clamp(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(chars).collect();
    out.push_str(" …");
    out
}

fn first_line(text: &str) -> String {
    clamp(text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim(), 80)
}

impl PeerState {
    pub fn new(dir: Option<PathBuf>, machine_id: String) -> Arc<Self> {
        let config = dir.as_deref().map(read_config).unwrap_or_default();
        let threads = dir
            .as_ref()
            .and_then(|d| std::fs::read(d.join(THREADS_FILE)).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| serde_json::from_value::<Vec<Thread>>(v["threads"].clone()).ok())
            .unwrap_or_default();
        let me = Arc::new(Self {
            dir,
            machine_id,
            config: Mutex::new(config),
            threads: Mutex::new(threads),
            ctx: OnceLock::new(),
            link: OnceLock::new(),
        });
        *CURRENT.get_or_init(Default::default).write().unwrap() = Arc::downgrade(&me);
        me
    }

    pub fn attach(&self, link: &Arc<LinkState>, ctl: Arc<ControlContext>, dash: Arc<WebDashState>) {
        let _ = self.link.set(Arc::downgrade(link));
        let _ = self.ctx.set((ctl, dash));
    }

    pub fn data_dir(&self) -> Option<&std::path::Path> {
        self.dir.as_deref()
    }

    fn link(&self) -> Option<Arc<LinkState>> {
        self.link.get()?.upgrade()
    }

    fn ctx(&self) -> Result<&(Arc<ControlContext>, Arc<WebDashState>), String> {
        self.ctx.get().ok_or_else(|| "peer-not-ready".to_string())
    }

    pub fn config(&self) -> PeerConfig {
        self.config.lock().unwrap().clone()
    }

    pub fn configure(&self, config: PeerConfig) -> Result<PeerConfig, String> {
        if let Some(dir) = &self.dir {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let bytes = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
            crate::persistence::durable::write_atomic(&dir.join(CONFIG_FILE), &bytes).map_err(|e| e.to_string())?;
        }
        *self.config.lock().unwrap() = config.clone();
        Ok(config)
    }

    fn require_enabled(&self) -> Result<(), String> {
        if self.config().enabled {
            Ok(())
        } else {
            Err(format!("peer-off: {}", machine_name()))
        }
    }

    // ── 기록 ─────────────────────────────────────────────────────────

    pub fn threads(&self) -> Vec<Thread> {
        self.threads.lock().unwrap().clone()
    }

    fn save_threads(&self, list: &[Thread]) {
        let Some(dir) = &self.dir else { return };
        let text = serde_json::to_string_pretty(&json!({ "threads": list })).unwrap_or_default();
        let _ = crate::persistence::durable::write_atomic(&dir.join(THREADS_FILE), text.as_bytes());
    }

    /// 같은 id가 있으면 합친다(같은 머신끼리면 보낸 쪽·받은 쪽 기록이 하나다).
    fn upsert(&self, t: Thread) {
        let mut list = self.threads.lock().unwrap();
        match list.iter_mut().find(|x| x.id == t.id) {
            Some(old) => {
                old.outgoing |= t.outgoing;
                old.incoming |= t.incoming;
                if old.reply.is_none() {
                    old.reply = t.reply;
                    old.replied_at = t.replied_at;
                }
            }
            None => list.push(t),
        }
        if list.len() > KEEP {
            let cut = list.len() - KEEP;
            list.drain(..cut);
        }
        self.save_threads(&list);
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut Thread)) -> Option<Thread> {
        let mut list = self.threads.lock().unwrap();
        let t = list.iter_mut().find(|t| t.id == id)?;
        f(t);
        let out = t.clone();
        self.save_threads(&list);
        Some(out)
    }

    fn remove_role(&self, id: &str, incoming: bool) {
        let mut list = self.threads.lock().unwrap();
        if let Some(t) = list.iter_mut().find(|t| t.id == id) {
            if incoming {
                t.incoming = false;
            } else {
                t.outgoing = false;
            }
        }
        list.retain(|t| t.incoming || t.outgoing);
        self.save_threads(&list);
    }

    /// 보내기 전 상한 검사. 통과하면 새 대화의 깊이를 돌려준다.
    fn check_limits(&self, from_agent: &str, to: &Party, now: u64) -> Result<u32, String> {
        let list = self.threads.lock().unwrap();
        // 이 에이전트에게 답을 기다리는 상대에게 되묻지 않는다(서로 기다리는 교착).
        if let Some(t) = list.iter().find(|t| {
            t.incoming
                && t.reply.is_none()
                && t.to.agent_id.as_deref() == Some(from_agent)
                && t.from.machine_id == to.machine_id
                && t.from.agent_id == to.agent_id
                && now.saturating_sub(t.created_at) < DEPTH_WINDOW_MS
        }) {
            return Err(format!("peer-reply-first: {}", t.id));
        }
        let sent: Vec<&Thread> = list
            .iter()
            .filter(|t| t.outgoing && t.from.agent_id.as_deref() == Some(from_agent) && now.saturating_sub(t.created_at) < HOUR_MS)
            .collect();
        if sent.len() >= MAX_SENDS_PER_HOUR {
            return Err(format!("peer-rate-limit: {MAX_SENDS_PER_HOUR} messages per hour"));
        }
        let pair = sent.iter().filter(|t| t.to.machine_id == to.machine_id && t.to.agent_id == to.agent_id).count();
        if pair >= MAX_PAIR_PER_HOUR {
            return Err(format!("peer-pair-limit: {MAX_PAIR_PER_HOUR} messages per hour to {}", to.label()));
        }
        // 답하지 않은 받은 대화가 있으면 그 깊이 + 1.
        let depth = list
            .iter()
            .filter(|t| {
                t.incoming
                    && t.reply.is_none()
                    && t.to.agent_id.as_deref() == Some(from_agent)
                    && now.saturating_sub(t.created_at) < DEPTH_WINDOW_MS
            })
            .map(|t| t.depth)
            .max()
            .unwrap_or(0)
            + 1;
        if depth > MAX_DEPTH {
            return Err(format!("peer-depth-limit: {MAX_DEPTH}"));
        }
        Ok(depth)
    }

    // ── 이 머신 캐릭터 ────────────────────────────────────────────────

    fn local_party(&self, agent_id: &str) -> Result<Party, String> {
        let (ctl, _) = self.ctx()?;
        let p = ctl.store.load().agents.into_iter().find(|a| a.id == agent_id).ok_or("peer-not-a-character")?;
        Ok(Party { machine_id: self.machine_id.clone(), machine_name: machine_name(), agent_id: Some(p.id), agent_name: Some(p.name) })
    }

    /// 이름(`캐릭터`, `캐릭터@머신`)을 연동 전체 명단에서 찾는다.
    fn resolve(&self, spec: &str) -> Result<Party, String> {
        let files = crate::link::files::current().ok_or("peer-not-ready")?;
        let targets: Vec<crate::files::Target> = files.targets().into_iter().filter(|t| t.agent_id.is_some()).collect();
        let t = crate::files::model::resolve_target(&targets, spec).map_err(|e| e.replacen("file-target-", "peer-target-", 1))?;
        if !t.online {
            return Err(format!("peer-machine-offline: {}", t.machine_name));
        }
        Ok(Party {
            machine_id: t.machine_id.clone(),
            machine_name: t.machine_name.clone(),
            agent_id: t.agent_id.clone(),
            agent_name: t.agent_name.clone(),
        })
    }

    /// 이 머신 명단 한 칸(`roster`).
    async fn local_roster(&self) -> Value {
        let base = json!({ "machineId": self.machine_id, "machineName": machine_name(), "online": true });
        let mut out = base;
        if !self.config().enabled {
            out["enabled"] = json!(false);
            out["characters"] = json!([]);
            return out;
        }
        out["enabled"] = json!(true);
        let Ok((ctl, dash)) = self.ctx() else {
            out["characters"] = json!([]);
            return out;
        };
        let view = crate::webdash::snapshot::local_view(ctl, dash).await;
        let characters: Vec<Value> = view.characters.iter().map(|c| summary(c, false)).collect();
        out["characters"] = json!(characters);
        out
    }

    async fn local_character(&self, agent: &str) -> Result<Value, String> {
        let (ctl, dash) = self.ctx()?;
        let view = crate::webdash::snapshot::local_view(ctl, dash).await;
        let c = view.characters.iter().find(|c| c["agentId"].as_str() == Some(agent)).ok_or("peer-unknown-character")?;
        let mut out = summary(c, true);
        out["machineId"] = json!(self.machine_id);
        out["machineName"] = json!(machine_name());
        out["outputAt"] = view.output_at.get(agent).cloned().unwrap_or(Value::Null);
        Ok(out)
    }

    async fn local_log(&self, agent: &str, turns: usize) -> Result<Value, String> {
        let (ctl, dash) = self.ctx()?;
        let cwd = crate::webdash::snapshot::agent_cwd(ctl, agent).ok_or("peer-unknown-character")?;
        let dash = dash.clone();
        let agent = agent.to_string();
        let page = tokio::task::spawn_blocking(move || dash.app_logs.read(&agent, &cwd, None, None)).await.map_err(|e| e.to_string())?;
        if !page.attached {
            return Ok(json!({ "attached": false, "items": [] }));
        }
        use crate::session_log::agent_transcript::{ItemKind, ItemRole};
        let speech: Vec<Value> = page
            .items
            .iter()
            .filter(|i| i.item.kind == ItemKind::Text && !i.item.sidechain && !i.item.text.trim().is_empty())
            .map(|i| {
                let role = if i.item.role == ItemRole::User { "user" } else { "assistant" };
                json!({ "role": role, "text": clamp(i.item.text.trim(), LOG_ITEM_CHARS) })
            })
            .collect();
        // 사람 말 `turns`개부터(한 턴 = 사람 말 + 그 뒤 에이전트 말).
        let mut start = speech.len();
        let mut seen = 0;
        while start > 0 && seen < turns {
            start -= 1;
            if speech[start]["role"] == "user" {
                seen += 1;
            }
        }
        let mut items: Vec<Value> = speech[start..].to_vec();
        if items.len() > MAX_LOG_ITEMS {
            items.drain(..items.len() - MAX_LOG_ITEMS);
        }
        Ok(json!({ "attached": true, "items": items }))
    }

    // ── 머신 사이 일(이 머신에서 한다) ─────────────────────────────────

    /// 다른 머신(또는 이 머신 CLI)이 시킨 일을 이 머신에서 한다.
    pub async fn exec_local(self: &Arc<Self>, op: &str, data: Value) -> Result<Value, String> {
        match op {
            "roster" => Ok(self.local_roster().await),
            "deliver" => {
                self.require_enabled()?;
                let t: Thread = serde_json::from_value(data.get("thread").cloned().unwrap_or_default()).map_err(|_| "bad-request".to_string())?;
                self.deliver(t).await
            }
            "reply" => self.receive_reply(&data),
            "exec" => {
                self.require_enabled()?;
                let agent = data.get("agentId").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or("bad-request")?.to_string();
                let what = data.get("op").and_then(Value::as_str).unwrap_or("");
                match what {
                    "status" => self.local_character(&agent).await,
                    "log" => {
                        let turns = data.get("turns").and_then(Value::as_u64).unwrap_or(3).clamp(1, 20) as usize;
                        self.local_log(&agent, turns).await
                    }
                    op if AGENT_OPS.contains(&op) => {
                        let (ctl, dash) = self.ctx()?;
                        let body = crate::webdash::AgentCommandBody {
                            cli: data.get("cli").and_then(Value::as_str).map(str::to_string),
                            text: None,
                            id: data.get("id").and_then(Value::as_str).map(str::to_string),
                            now: false,
                            quiet: true,
                        };
                        crate::webdash::agent_op(ctl.clone(), dash.clone(), &agent, op, body).await.map(|v| if v.is_null() { json!({}) } else { v })
                    }
                    _ => Err("peer-unknown-op".into()),
                }
            }
            _ => Err("peer-unknown-op".into()),
        }
    }

    /// 받은 대화를 기록하고 상대 세션에 넣는다.
    async fn deliver(self: &Arc<Self>, mut t: Thread) -> Result<Value, String> {
        let agent = t.to.agent_id.clone().ok_or("bad-request")?;
        let party = self.local_party(&agent).map_err(|_| "peer-unknown-character".to_string())?;
        t.to = party;
        t.incoming = true;
        t.outgoing = t.from.machine_id == self.machine_id;
        t.reply = None;
        t.replied_at = None;
        // 같은 id를 두 번 받으면(다시 보내기) 앞 기록을 지우지 않는다.
        let existed = self.threads.lock().unwrap().iter().any(|x| x.id == t.id && x.incoming);
        self.upsert(t.clone());
        let text = arrival_message(&t, &cli::shim_path());
        let (ctl, dash) = self.ctx()?;
        let body = crate::webdash::AgentCommandBody { cli: None, text: Some(text), id: None, now: false, quiet: true };
        match crate::webdash::agent_op(ctl.clone(), dash.clone(), &agent, "message", body).await {
            Ok(v) => Ok(json!({ "outcome": v.get("outcome").cloned().unwrap_or(Value::Null) })),
            Err(e) => {
                if !existed {
                    self.remove_role(&t.id, true);
                }
                Err(e)
            }
        }
    }

    /// 보낸 쪽 머신: 답이 왔다. 기다리는 CLI에 파일로 주고, 몇 초 안에 안 가져가면 보낸 세션에 넣는다.
    fn receive_reply(self: &Arc<Self>, data: &Value) -> Result<Value, String> {
        let id = data.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or("bad-request")?.to_string();
        let text = data.get("text").and_then(Value::as_str).ok_or("bad-request")?.to_string();
        let from: Party = serde_json::from_value(data.get("from").cloned().unwrap_or_default()).map_err(|_| "bad-request".to_string())?;
        let t = self.threads().into_iter().find(|t| t.id == id && t.outgoing).ok_or("peer-thread-unknown")?;
        if t.to.machine_id != from.machine_id || t.to.agent_id != from.agent_id {
            return Err("peer-thread-not-yours".into());
        }
        if t.reply.is_some() {
            return Err("peer-already-replied".into());
        }
        let now = now_ms();
        let t = self
            .update(&id, |t| {
                t.reply = Some(clamp(&text, KEEP_CHARS));
                t.replied_at = Some(now);
            })
            .ok_or("peer-thread-unknown")?;
        let Some(dir) = self.dir.as_ref().map(|d| d.join(REPLIES_DIR)) else { return Ok(json!({})) };
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join(format!("{id}.json"));
        let body = json!({ "id": id, "from": t.to, "reply": text });
        crate::persistence::durable::write_atomic(&file, body.to_string().as_bytes()).map_err(|e| e.to_string())?;
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(CLI_CLAIM).await;
            // 기다리는 CLI가 먼저 가져갔으면(이름을 바꿔 가져간다) 끝.
            let late = file.with_extension("late");
            if std::fs::rename(&file, &late).is_err() {
                return;
            }
            let _ = std::fs::remove_file(&late);
            let Some(agent) = t.from.agent_id.clone() else { return };
            let Ok((ctl, _)) = me.ctx() else { return };
            let _ = ctl.chat_queue.send(ctl, &agent, &late_reply_message(&t, &text)).await;
        });
        Ok(json!({}))
    }

    /// 머신 하나에 일을 넘긴다(이 머신이면 바로).
    pub async fn route(self: &Arc<Self>, machine: &str, op: &str, data: Value) -> Result<Value, String> {
        if machine == self.machine_id {
            return self.exec_local(op, data).await;
        }
        let link = self.link().ok_or("peer-machine-unknown")?;
        match link.config().mode {
            LinkMode::Off => Err("peer-machine-unknown".into()),
            LinkMode::Host => {
                let device = link
                    .host
                    .status()
                    .devices
                    .into_iter()
                    .find(|d| d.machine_id == machine)
                    .ok_or("peer-machine-unknown")?;
                link.remote
                    .request_with(&device.id, crate::link::peer::REQ_PEER, json!({ "op": op, "data": data }), ROUTE_TIMEOUT)
                    .await
                    .map_err(|e| if e == "link-device-offline" { format!("peer-machine-offline: {}", device.name) } else { e })
            }
            LinkMode::Client => crate::link::peer::client_rpc(&link, json!({ "toMachine": machine, "op": op, "data": data })).await,
        }
    }

    /// 연동 전체 명단(이 머신이 먼저). `skip`은 빼고 모은다(호스트가 묻는 클라이언트를 뺄 때).
    pub async fn roster_all(self: &Arc<Self>, skip: Option<&str>) -> Vec<Value> {
        let mut out = Vec::new();
        if skip != Some(self.machine_id.as_str()) {
            let mut me = self.local_roster().await;
            me["local"] = json!(true);
            out.push(me);
        }
        let Some(link) = self.link() else { return out };
        match link.config().mode {
            LinkMode::Off => {}
            LinkMode::Host => {
                let devices: Vec<_> = link.host.status().devices.into_iter().filter(|d| Some(d.machine_id.as_str()) != skip).collect();
                let asks = devices.iter().map(|d| {
                    let link = link.clone();
                    let d = d.clone();
                    async move {
                        let offline = json!({ "machineId": d.machine_id, "machineName": d.name, "online": false, "characters": [] });
                        if !d.connected {
                            return offline;
                        }
                        let ask = json!({ "op": "roster", "data": {} });
                        match link.remote.request_with(&d.id, crate::link::peer::REQ_PEER, ask, ROSTER_TIMEOUT).await {
                            Ok(v) => v,
                            Err(_) => offline,
                        }
                    }
                });
                out.extend(futures_util::future::join_all(asks).await);
            }
            LinkMode::Client => {
                match crate::link::peer::client_rpc(&link, json!({ "toMachine": "", "op": "roster-all", "data": {} })).await {
                    Ok(Value::Array(list)) => out.extend(list),
                    _ => {
                        let name = link.client.status().host_name.unwrap_or_default();
                        out.push(json!({ "machineId": "", "machineName": name, "online": false, "characters": [] }));
                    }
                }
            }
        }
        out
    }

    // ── CLI 요청(이 머신 에이전트) ───────────────────────────────────

    /// `office-peer` 요청 하나를 처리한다.
    pub async fn handle_cli(self: &Arc<Self>, body: &Value) -> Value {
        match self.cli_op(body).await {
            Ok(v) => v,
            Err(e) => json!({ "error": e }),
        }
    }

    async fn cli_op(self: &Arc<Self>, body: &Value) -> Result<Value, String> {
        self.require_enabled()?;
        let op = body.get("op").and_then(Value::as_str).unwrap_or("");
        let from_agent = body.get("fromAgent").and_then(Value::as_str).filter(|s| !s.is_empty());
        let target = || body.get("target").and_then(Value::as_str).unwrap_or("");
        match op {
            "agents" => Ok(json!({ "machines": self.roster_all(None).await })),
            "ask" => {
                let from_agent = from_agent.ok_or("peer-not-in-session")?;
                let from = self.local_party(from_agent)?;
                let to = self.resolve(target())?;
                if to.machine_id == from.machine_id && to.agent_id == from.agent_id {
                    return Err("peer-self".into());
                }
                let text = body.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string();
                crate::chat_queue::check_text(&text)?;
                let now = now_ms();
                let depth = self.check_limits(from_agent, &to, now)?;
                let id = body
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(new_thread_id);
                let thread = Thread {
                    id: id.clone(),
                    from,
                    to: to.clone(),
                    text: clamp(&text, KEEP_CHARS),
                    reply: None,
                    created_at: now,
                    replied_at: None,
                    depth,
                    outgoing: true,
                    incoming: false,
                };
                // 받는 머신에는 글 전체를 보낸다(기록만 자른다).
                let mut wire = serde_json::to_value(&thread).map_err(|e| e.to_string())?;
                wire["text"] = json!(text);
                // 기다리는 동안 답이 먼저 와도 찾게 보내기 전에 남긴다.
                self.upsert(thread.clone());
                match self.route(&to.machine_id, "deliver", json!({ "thread": wire })).await {
                    Ok(v) => Ok(json!({ "thread": thread, "outcome": v.get("outcome").cloned().unwrap_or(Value::Null) })),
                    Err(e) => {
                        self.remove_role(&id, false);
                        Err(e)
                    }
                }
            }
            "reply" => {
                let from_agent = from_agent.ok_or("peer-not-in-session")?;
                let id = body.get("id").and_then(Value::as_str).unwrap_or("").trim().to_string();
                let text = body.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string();
                crate::chat_queue::check_text(&text)?;
                let t = self.threads().into_iter().find(|t| t.id == id && t.incoming).ok_or("peer-thread-unknown")?;
                if t.to.agent_id.as_deref() != Some(from_agent) {
                    return Err("peer-thread-not-yours".into());
                }
                if t.reply.is_some() {
                    return Err("peer-already-replied".into());
                }
                let me = self.local_party(from_agent)?;
                self.route(&t.from.machine_id, "reply", json!({ "id": id, "text": text, "from": me })).await?;
                let now = now_ms();
                self.update(&id, |t| {
                    if t.reply.is_none() {
                        t.reply = Some(clamp(&text, KEEP_CHARS));
                        t.replied_at = Some(now);
                    }
                });
                Ok(json!({ "to": t.from }))
            }
            "status" | "log" | "launch" | "resume" | "interrupt" | "queue-cancel" => {
                let to = self.resolve(target())?;
                let mut data = json!({ "agentId": to.agent_id, "op": op });
                for key in ["cli", "id", "turns"] {
                    if let Some(v) = body.get(key).filter(|v| !v.is_null()) {
                        data[key] = v.clone();
                    }
                }
                let mut v = self.route(&to.machine_id, "exec", data).await?;
                if v.is_object() {
                    v["target"] = json!(to);
                }
                Ok(v)
            }
            _ => Err("bad-request".into()),
        }
    }
}

pub fn machine_name() -> String {
    crate::webdash::computer_name()
}

fn new_thread_id() -> String {
    format!("p-{}", &uuid::Uuid::new_v4().simple().to_string()[..10])
}

/// 웹 상태판 캐릭터 모습을 에이전트가 볼 요약으로. `detail`이면 대기열 글·권한 요청 문구까지.
fn summary(c: &Value, detail: bool) -> Value {
    let turn = &c["turn"];
    let asks = c["asks"].as_array().map(Vec::len).unwrap_or(0);
    let queue = c["queue"]["items"].as_array().cloned().unwrap_or_default();
    let mut out = json!({
        "agentId": c["agentId"],
        "name": c["name"],
        "role": c["role"],
        "cwd": c["cwd"],
        "clockedOut": c["clockedOut"],
        "session": c["session"]["state"],
        "phase": turn["phase"],
        "since": turn["since"],
        "cli": c["cli"],
        "atPrompt": c["atPrompt"],
        "asks": asks,
        "queued": queue.len(),
    });
    if detail {
        out["queue"] = json!(queue
            .iter()
            .map(|q| json!({ "id": q["id"], "text": first_line(q["text"].as_str().unwrap_or("")) }))
            .collect::<Vec<_>>());
        out["waitingFor"] = turn["message"].clone();
        out["work"] = c["work"].clone();
    }
    out
}

/// 받은 세션에 넣을 글.
pub fn arrival_message(t: &Thread, shim: &std::path::Path) -> String {
    format!(
        "[Agent Office · message {id} from {from} (another agent)]\n{text}\n\nThis came from another agent, not from the user. Reply once with `{cli} reply {id} \"<answer>\"`; the sender is waiting for that reply, so reply even if you cannot help.",
        id = t.id,
        from = t.from.label(),
        text = t.text,
        cli = shim.display(),
    )
}

/// 보낸 CLI가 기다리지 않을 때 보낸 세션에 넣을 늦은 답.
pub fn late_reply_message(t: &Thread, reply: &str) -> String {
    format!("[Agent Office · reply to your message {} from {}]\n{reply}", t.id, t.to.label())
}

#[cfg(test)]
mod tests;
