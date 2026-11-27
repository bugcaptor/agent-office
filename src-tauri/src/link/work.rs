// src-tauri/src/link/work.rs
//
// 머신 연동 업무 분배. 호스트 보드가 정본이고 자동 진행도 호스트 렌더러가 돌린다(`autoRun.ts`). 여기는
// 두 머신 사이를 잇는 부분만 둔다.
//
// 클라이언트
// - 이 머신의 클론 목록(정규화한 원격 주소, `repos.rs`)을 `Up::Live` 종류 `repos`로 알린다(붙을 때·바뀔 때).
// - 호스트가 묻는 `work-send`(저장소로 작업 폴더를 정하고 렌더러에 넘겨 "받은 업무"로 보낸다 — 클론이 없으면
//   이유 코드로 거절), `work-followup`(같은 세션에 후속 지시), `work-reply`(마지막 응답), `usage`(이 머신 구독
//   사용량)에 답한다.
// - 받은 업무의 진행 상태(렌더러가 `link_task_report`로 알림)를 쌓아 올린다(`task`, 업무별로 최신 하나).
// - 호스트 보드 사본(`link-host-board/`: `work-items.json`·`profiles.json`·`host.json`)을 호스트가 보낼 때마다
//   쓴다. `office-task --host`와 받은 업무 세션(`link-received.json`에 적힌 세션)은 이 사본을 읽고
//   `link-host-board/inbox/`에 쓴다. 그 수신함 파일과 보고(`inbox/reports/`)를 쌓아 호스트로 올린다
//   (`inbox`·`report`). 끊겨 있어도 쌓아 두었다가(outbox) 다시 붙으면 올린다.
//
// 호스트
// - `task`는 렌더러에 `link-task` 이벤트로 넘기고, `inbox`·`report`는 이 머신 수신함에 파일로 쓴다(그 뒤는 로컬
//   `office-task`와 같다).
// - 붙은 머신마다 보드 사본을 바뀔 때(와 다시 붙을 때) 보낸다(`Down::Command` 종류 `board`).
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{LinkMode, LinkState};
use crate::control::ControlContext;
use crate::webdash::WebDashState;

/// 클라이언트가 올리는 클론 목록(`Up::Live`).
pub const KIND_REPOS: &str = "repos";
/// 받은 업무의 진행 상태(`Up::Event`, 업무별 key).
pub const KIND_TASK: &str = "task";
/// `office-task` 수신함 파일 하나(`Up::Event`).
pub const KIND_INBOX: &str = "inbox";
/// 자동 진행 보고(`Up::Event`, 업무별 key).
pub const KIND_REPORT: &str = "report";
/// 호스트 보드 사본(`Down::Command`).
pub const CMD_BOARD: &str = "board";

/// 클라이언트의 호스트 보드 사본 폴더(앱 데이터 아래).
pub const HOST_BOARD_DIR: &str = "link-host-board";
/// 사본 폴더 안의 호스트 정보(`{deviceId, hostName, at}`). 있으면 `--host`를 쓸 수 있다.
pub const HOST_INFO_FILE: &str = "host.json";
/// 받은 업무를 실행하는 세션들(`{sessions: {세션id: 업무id}}`). `office-task`가 이 세션이면 호스트 보드를 쓴다.
pub const RECEIVED_FILE: &str = "link-received.json";
/// 기억하는 받은 업무 세션 수.
const MAX_RECEIVED_SESSIONS: usize = 100;
/// 호스트가 들고 있는 받은 업무 진행 상태 수.
const MAX_TASK_STATES: usize = 500;

/// 클론 목록을 다시 찾는 간격.
const REPO_RESCAN: Duration = Duration::from_secs(5 * 60);
/// 수신함·보드를 살피는 간격.
const TICK: Duration = Duration::from_secs(2);
/// 렌더러에 받은 업무를 넘기고 답을 기다리는 시간은 다리(`AutoRunBridge`)가 정한다(5초).

pub fn host_board_dir(app_data: &Path) -> PathBuf {
    app_data.join(HOST_BOARD_DIR)
}

pub fn host_board_inbox(app_data: &Path) -> PathBuf {
    host_board_dir(app_data).join("inbox")
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// 수신함 파일 이름으로 받을 수 있는 것(`office-task`가 쓰는 모양: 영숫자·`-`·`_`·`.`, `.md`로 끝남).
fn valid_inbox_name(name: &str) -> bool {
    name.len() <= 128
        && name.ends_with(".md")
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

// ── 클라이언트 ─────────────────────────────────────────────────────────

pub struct ClientWork {
    app_data: PathBuf,
    /// 정규화한 원격 주소 → 클론 최상위들.
    repos: Mutex<Option<(Instant, BTreeMap<String, Vec<PathBuf>>)>>,
}

impl ClientWork {
    pub fn new(app_data: PathBuf) -> Arc<Self> {
        Arc::new(Self { app_data, repos: Mutex::new(None) })
    }

    fn rescan(&self, ctl: &ControlContext) -> BTreeMap<String, Vec<PathBuf>> {
        let starts: Vec<PathBuf> = ctl.store.load().agents.into_iter().filter_map(|a| a.cwd).map(PathBuf::from).collect();
        let event_root = self.app_data.join("session-events").join("v1");
        let found = super::repos::scan(&super::repos::candidates(&starts, Some(&event_root)));
        *self.repos.lock().unwrap() = Some((Instant::now(), found.clone()));
        found
    }

    /// 아는 클론 목록. 오래됐거나 `force`면 다시 찾는다(블로킹 — git을 부른다).
    fn repos(&self, ctl: &ControlContext, force: bool) -> BTreeMap<String, Vec<PathBuf>> {
        if !force {
            if let Some((at, repos)) = self.repos.lock().unwrap().as_ref() {
                if at.elapsed() < REPO_RESCAN {
                    return repos.clone();
                }
            }
        }
        self.rescan(ctl)
    }

    /// 받은 업무를 실행하는 세션을 적는다(`office-task`가 호스트 보드를 고르게).
    pub fn remember_session(&self, session_id: &str, task_id: &str) {
        let path = self.app_data.join(RECEIVED_FILE);
        let mut sessions: Vec<(String, String)> = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.get("order").and_then(Value::as_array).cloned())
            .map(|order| {
                order
                    .iter()
                    .filter_map(|e| Some((e.get(0)?.as_str()?.to_string(), e.get(1)?.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        if sessions.iter().any(|(s, t)| s == session_id && t == task_id) {
            return;
        }
        sessions.retain(|(s, _)| s != session_id);
        sessions.push((session_id.to_string(), task_id.to_string()));
        if sessions.len() > MAX_RECEIVED_SESSIONS {
            let drop = sessions.len() - MAX_RECEIVED_SESSIONS;
            sessions.drain(..drop);
        }
        let map: serde_json::Map<String, Value> = sessions.iter().map(|(s, t)| (s.clone(), json!(t))).collect();
        let file = json!({ "sessions": map, "order": sessions.iter().map(|(s, t)| json!([s, t])).collect::<Vec<_>>() });
        if let Ok(bytes) = serde_json::to_vec_pretty(&file) {
            let _ = write_atomic(&path, &bytes);
        }
    }

    /// 호스트가 보낸 보드 사본을 쓴다.
    pub fn write_board(&self, data: &Value) {
        let dir = host_board_dir(&self.app_data);
        let items = data.get("items").cloned().unwrap_or_else(|| json!([]));
        let agents = data.get("agents").cloned().unwrap_or_else(|| json!([]));
        let info = json!({
            "deviceId": data.get("deviceId").cloned().unwrap_or(Value::Null),
            "hostName": data.get("hostName").cloned().unwrap_or(Value::Null),
            "at": crate::types::now_ms(),
        });
        let _ = std::fs::create_dir_all(dir.join("inbox").join("reports"));
        let _ = write_atomic(&dir.join("work-items.json"), &serde_json::to_vec(&items).unwrap_or_default());
        let _ = write_atomic(&dir.join("profiles.json"), &serde_json::to_vec(&json!({ "agents": agents })).unwrap_or_default());
        let _ = write_atomic(&dir.join(HOST_INFO_FILE), &serde_json::to_vec_pretty(&info).unwrap_or_default());
    }

    /// 호스트 보드 수신함에 쓰인 파일·보고를 쌓아 올리고 지운다. 올린 수.
    pub fn forward_inbox(&self, link: &LinkState) -> usize {
        let inbox = host_board_inbox(&self.app_data);
        let mut sent = 0;
        let mut names: Vec<PathBuf> = std::fs::read_dir(&inbox)
            .map(|e| e.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect())
            .unwrap_or_default();
        // 시각 순 이름이라 앞서 쓴 요청이 먼저 간다.
        names.sort();
        for path in names {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if !valid_inbox_name(&name) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            link.client.enqueue(KIND_INBOX, json!({ "name": name, "text": text }), None);
            let _ = std::fs::remove_file(&path);
            sent += 1;
        }
        let reports = crate::auto_run::reports_dir(&inbox);
        for entry in std::fs::read_dir(&reports).map(|e| e.flatten().collect::<Vec<_>>()).unwrap_or_default() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".md").filter(|id| crate::persistence::work_inbox::valid_task_id(id)) else { continue };
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            link.client.enqueue(KIND_REPORT, json!({ "taskId": id, "text": text }), Some(&format!("report:{id}")));
            let _ = std::fs::remove_file(&path);
            sent += 1;
        }
        sent
    }

    /// 업무 하나의 작업 폴더(블로킹). 저장소가 없으면 캐릭터 시작 폴더.
    fn work_dir(&self, ctl: &ControlContext, task_id: &str, repo: Option<&Value>, start: Option<&str>) -> Result<String, String> {
        let Some(repo) = repo.filter(|r| r.get("url").and_then(Value::as_str).is_some_and(|u| !u.trim().is_empty())) else {
            return start.map(str::to_string).filter(|s| !s.is_empty()).ok_or_else(|| "link-no-work-dir".to_string());
        };
        let url = repo.get("url").and_then(Value::as_str).unwrap_or("");
        let key = super::repos::normalize_remote(url).ok_or("link-repo-invalid")?;
        let mut roots = self.repos(ctl, false).get(&key).cloned();
        if roots.is_none() {
            // 방금 클론했을 수 있다.
            roots = self.repos(ctl, true).get(&key).cloned();
        }
        let roots = roots.filter(|r| !r.is_empty()).ok_or_else(|| format!("link-repo-missing: {key}"))?;
        // 같은 저장소의 클론이 여럿이면 캐릭터 시작 폴더 안의 것을 먼저 쓴다.
        let root = start
            .and_then(|s| roots.iter().find(|r| Path::new(s).starts_with(r) || r.starts_with(s)))
            .unwrap_or(&roots[0])
            .clone();
        let text = |k: &str| repo.get(k).and_then(Value::as_str);
        let worktree = repo.get("worktree").and_then(Value::as_bool).unwrap_or(false);
        super::repos::prepare(&root, task_id, text("subpath"), text("branch"), worktree).map(|p| p.to_string_lossy().into_owned())
    }
}

fn str_field<'a>(data: &'a Value, key: &str) -> Result<&'a str, String> {
    data.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| "bad-request".into())
}

/// 호스트 보드에서 완료된 받은 업무를 이 머신 보드에서도 완료로 둔다(렌더러가 외부 변경으로 합친다).
/// 호스트에서 지워진 것은 건드리지 않는다(사본이 늦게 와 막 받은 업무가 빠져 있을 수 있다). 바꿨으면 true.
pub fn sync_received(store: &crate::persistence::work_panels_store::WorkPanelsStore, board: &Value) -> bool {
    use crate::types::WorkStatus;
    let Some(host) = board.get("items").and_then(Value::as_array) else { return false };
    let done: std::collections::HashSet<&str> = host
        .iter()
        .filter(|i| i.get("status").and_then(Value::as_str) == Some("done"))
        .filter_map(|i| i.get("id").and_then(Value::as_str))
        .collect();
    let Ok(before) = store.load_work_items() else { return false };
    let mut items = before.clone();
    let mut order = items.iter().filter(|i| i.status == WorkStatus::Done).map(|i| i.order).fold(-1.0_f64, f64::max) + 1.0;
    let mut changed = false;
    for item in items.iter_mut().filter(|i| i.received.is_some() && i.status != WorkStatus::Done && done.contains(i.id.as_str())) {
        item.status = WorkStatus::Done;
        item.order = order;
        item.updated_at = crate::types::now_ms();
        order += 1.0;
        changed = true;
    }
    changed && store.save_work_items_if_unchanged(&items, Some(&before)).is_ok()
}

/// 호스트의 업무 물음에 답한다. 이 모듈이 맡지 않는 종류면 None.
pub async fn answer(
    work: &Arc<ClientWork>,
    ctl: &Arc<ControlContext>,
    dash: &Arc<WebDashState>,
    kind: &str,
    data: &Value,
) -> Option<Result<Value, String>> {
    Some(match kind {
        "work-send" => work_send(work, ctl, dash, data).await,
        "work-followup" => follow_up(ctl, data).await,
        "work-reply" => last_reply(ctl, data).await,
        "usage" => {
            let openrouter = ctl.key_store.openrouter_key();
            let opencode = ctl.key_store.opencode_key();
            let keys = crate::usage::ApiKeyRequest { openrouter: openrouter.as_deref(), opencode: opencode.as_deref(), force: false };
            let snapshot = crate::ipc::commands::load_usage_snapshot_body(&ctl.live_usage, keys, chrono::Utc::now().timestamp_millis()).await;
            serde_json::to_value(snapshot).map_err(|e| e.to_string())
        }
        _ => return None,
    })
}

async fn work_send(work: &Arc<ClientWork>, ctl: &Arc<ControlContext>, dash: &Arc<WebDashState>, data: &Value) -> Result<Value, String> {
    let task_id = str_field(data, "taskId")?.to_string();
    if !crate::persistence::work_inbox::valid_task_id(&task_id) {
        return Err("bad-request".into());
    }
    let agent = str_field(data, "agentId")?.to_string();
    let profile = ctl.store.load().agents.into_iter().find(|p| p.id == agent).ok_or("work-send-agent-missing")?;
    // 이 머신의 관찰 훅이 꺼져 있으면 턴이 끝난 것을 알 수 없어 호스트 자동 진행이 멈춘다.
    if !ctl.settings.read().unwrap().observer_enabled {
        return Err("auto-run-observer-off".into());
    }
    let repo = data.get("repo").cloned();
    let start = profile.cwd.clone();
    let cwd = {
        let (work, ctl, task_id) = (work.clone(), ctl.clone(), task_id.clone());
        tokio::task::spawn_blocking(move || work.work_dir(&ctl, &task_id, repo.as_ref(), start.as_deref()))
            .await
            .map_err(|e| e.to_string())??
    };
    let mut request = json!({
        "op": "link-work-receive",
        "itemId": task_id,
        "agentId": agent,
        "title": data.get("title").cloned().unwrap_or(json!("")),
        "body": data.get("body").cloned().unwrap_or(json!("")),
        "cwd": cwd,
        "auto": data.get("auto").and_then(Value::as_bool).unwrap_or(false),
        "hostName": data.get("hostName").cloned().unwrap_or(json!("")),
        "sendSeq": data.get("sendSeq").cloned().unwrap_or(Value::Null),
    });
    if let Some(agent) = data.get("agent").filter(|a| a.is_object()) {
        request["agent"] = agent.clone();
    }
    // 렌더러가 받은 업무로 넣고 보내기를 시작한 뒤 바로 답한다. 결과(세션·오류)는 `task` 이벤트로 올라간다.
    dash.bridge.send(request).await?;
    Ok(json!({ "cwd": cwd }))
}

async fn follow_up(ctl: &Arc<ControlContext>, data: &Value) -> Result<Value, String> {
    let agent = str_field(data, "agentId")?.to_string();
    let session = str_field(data, "sessionId")?.to_string();
    let text = data.get("text").and_then(Value::as_str).unwrap_or("").to_string();
    if !ctl.store.load().agents.iter().any(|p| p.id == agent) {
        return Err("work-send-agent-missing".into());
    }
    // 앱의 자동 진행 후속 지시와 같다: 오래 묵은 미제출 표식은 풀고 넣는다.
    ctl.gate.release_stale_uncommitted(&agent);
    let result = ctl
        .work_send
        .follow_up(&ctl.gate, crate::types::WorkFollowUpRequest { agent_id: agent, session_id: session, text })
        .await?;
    serde_json::to_value(result).map_err(|e| e.to_string())
}

async fn last_reply(ctl: &Arc<ControlContext>, data: &Value) -> Result<Value, String> {
    let agent = str_field(data, "agentId")?.to_string();
    // 받은 업무의 작업 폴더(Codex 전사는 폴더로 찾는다)와 CLI(업무에 없으면 이 머신 캐릭터의 기본 CLI —
    // 호스트는 이 머신 프로필을 모른다). 받은 업무가 없으면 호스트가 짐작한 CLI와 캐릭터의 지금 폴더.
    let task_id = data.get("taskId").and_then(Value::as_str).unwrap_or("");
    let item = ctl.work_panels.load_work_items().ok().and_then(|items| items.into_iter().find(|i| i.id == task_id));
    let profile_cli = ctl.store.load().agents.into_iter().find(|p| p.id == agent).and_then(|p| p.default_agent).map(|a| a.cli);
    let cli = item
        .as_ref()
        .and_then(|i| i.received.as_ref().map(|_| i.agent.as_ref().map(|a| a.cli.clone()).or(profile_cli.clone()).unwrap_or(crate::types::WorkCli::Claude)))
        .and_then(|c| serde_json::to_value(c).ok())
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| data.get("cli").and_then(Value::as_str).unwrap_or("claude").to_string());
    let cwd = item
        .and_then(|i| i.received)
        .and_then(|r| r.get("cwd").and_then(Value::as_str).map(str::to_string))
        .or_else(|| crate::webdash::snapshot::agent_cwd(ctl, &agent))
        .unwrap_or_default();
    let lookup = ctl.agent_session_lookup.clone();
    let reply = tokio::task::spawn_blocking(move || {
        use crate::session_log::agent_transcript::{claude, codex};
        let mut source = match cli.as_str() {
            "claude" => claude::source(lookup),
            "codex" => codex::source(),
            _ => None,
        }?;
        crate::auto_run::last_reply(source.as_mut(), &agent, &cwd)
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(json!({ "reply": reply }))
}

/// 클라이언트 쪽을 건다: 보드 사본 받기, 클론 목록 알리기, 호스트 보드 수신함 올리기.
pub fn install_client(link: &Arc<LinkState>, work: Arc<ClientWork>, ctl: Arc<ControlContext>) {
    {
        let (work, ctl) = (work.clone(), ctl.clone());
        link.client.set_command_handler(Arc::new(move |kind, data| {
            if kind == CMD_BOARD {
                work.write_board(data);
                sync_received(&ctl.work_panels, data);
            }
        }));
    }
    let link = Arc::downgrade(link);
    tauri::async_runtime::spawn(async move {
        let mut last: Option<(u64, Vec<String>, Instant)> = None;
        loop {
            tokio::time::sleep(TICK).await;
            let Some(link) = link.upgrade() else { break };
            if link.config().mode != LinkMode::Client {
                last = None;
                continue;
            }
            // 끊겨 있어도 쌓는다(outbox가 파일로 남기고 다시 붙으면 보낸다).
            work.forward_inbox(&link);
            let Some(conn) = link.client.connection() else { continue };
            let due = match &last {
                Some((c, _, at)) => *c != conn || at.elapsed() >= REPO_RESCAN,
                None => true,
            };
            if !due {
                continue;
            }
            let (w, c) = (work.clone(), ctl.clone());
            let Ok(repos) = tokio::task::spawn_blocking(move || w.repos(&c, true)).await else { continue };
            let urls: Vec<String> = repos.into_keys().collect();
            if link.client.send_live(KIND_REPOS, json!({ "repos": urls })) {
                last = Some((conn, urls, Instant::now()));
            }
        }
    });
}

// ── 호스트 ────────────────────────────────────────────────────────────

/// 붙은 머신이 올린 업무 이벤트를 처리한다(`LinkState`의 이벤트 처리기가 부른다).
pub fn host_event(
    app_data: Option<&Path>,
    events: &dyn super::LinkEvents,
    states: &Mutex<HashMap<String, Value>>,
    device: &str,
    kind: &str,
    data: &Value,
) {
    match kind {
        KIND_TASK => {
            let mut payload = data.clone();
            let task = data.get("taskId").and_then(Value::as_str).unwrap_or("").to_string();
            if let Some(obj) = payload.as_object_mut().filter(|_| !task.is_empty()) {
                obj.insert("deviceId".into(), json!(device));
                obj.insert("receivedAt".into(), json!(crate::types::now_ms()));
                {
                    let mut states = states.lock().unwrap();
                    states.insert(format!("{device}:{task}"), payload.clone());
                    // 오래 붙어 있어도 끝없이 늘지 않게 한다(오래된 것부터 버린다).
                    if states.len() > MAX_TASK_STATES {
                        let oldest = states.iter().min_by_key(|(_, v)| v.get("receivedAt").and_then(Value::as_u64).unwrap_or(0)).map(|(k, _)| k.clone());
                        if let Some(k) = oldest {
                            states.remove(&k);
                        }
                    }
                }
                events.task(&payload);
            }
        }
        KIND_INBOX => {
            let Some(dir) = app_data else { return };
            let name = data.get("name").and_then(Value::as_str).unwrap_or("");
            let text = data.get("text").and_then(Value::as_str).unwrap_or("");
            if !valid_inbox_name(name) || text.len() > crate::persistence::work_inbox::MAX_BYTES {
                return;
            }
            let inbox = dir.join("work-inbox");
            // 같은 이름(다른 머신이 같은 밀리초에 쓴 것)이면 뒤에 기기 id를 붙인다.
            let mut path = inbox.join(name);
            if path.exists() {
                path = inbox.join(format!("{}-{device}.md", name.trim_end_matches(".md")));
            }
            let _ = write_atomic(&path, text.as_bytes());
        }
        KIND_REPORT => {
            let Some(dir) = app_data else { return };
            let id = data.get("taskId").and_then(Value::as_str).unwrap_or("");
            let text = data.get("text").and_then(Value::as_str).unwrap_or("");
            if !crate::persistence::work_inbox::valid_task_id(id) || text.len() > crate::persistence::work_inbox::MAX_BYTES {
                return;
            }
            let reports = crate::auto_run::reports_dir(&dir.join("work-inbox"));
            let _ = write_atomic(&reports.join(format!("{id}.md")), text.as_bytes());
        }
        _ => {}
    }
}

/// 머신에 보낼 보드 사본(그 머신 기기 id를 넣어 보낸다). 담당 후보는 이 머신 캐릭터와 붙은 머신 캐릭터(원격 id).
pub fn board_snapshot(ctl: &ControlContext, remote: &super::RemoteView) -> Value {
    let items = ctl.work_panels.load_work_items().unwrap_or_default();
    let mut agents: Vec<Value> = ctl.store.load().agents.into_iter().map(|a| json!({ "id": a.id, "name": a.name })).collect();
    for machine in remote.office_view().machines {
        for c in machine.characters {
            agents.push(json!({ "id": c.id, "name": c.name, "machine": machine.name }));
        }
    }
    json!({ "items": items, "agents": agents })
}

fn fingerprint(v: &Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.to_string().hash(&mut h);
    h.finish()
}

/// 호스트 쪽을 건다: 붙은 머신마다 보드 사본을 바뀔 때·다시 붙을 때 보낸다.
pub fn install_host(link: &Arc<LinkState>, ctl: Arc<ControlContext>) {
    let link = Arc::downgrade(link);
    tauri::async_runtime::spawn(async move {
        // 기기 id → (붙은 시각, 보낸 사본 지문).
        let mut sent: HashMap<String, (Option<u64>, u64)> = HashMap::new();
        let mut cached: Option<(Instant, Value, u64)> = None;
        loop {
            tokio::time::sleep(TICK).await;
            let Some(link) = link.upgrade() else { break };
            if !link.host.enabled() {
                sent.clear();
                continue;
            }
            let devices: Vec<_> = link.host.status().devices.into_iter().filter(|d| d.connected).collect();
            sent.retain(|id, _| devices.iter().any(|d| &d.id == id));
            if devices.is_empty() {
                continue;
            }
            // 사본은 한 주기에 한 번만 만든다.
            let fresh = cached.as_ref().is_some_and(|(at, _, _)| at.elapsed() < TICK / 2);
            if !fresh {
                let snapshot = board_snapshot(&ctl, &link.remote);
                let fp = fingerprint(&snapshot);
                cached = Some((Instant::now(), snapshot, fp));
            }
            let Some((_, snapshot, fp)) = cached.as_ref() else { continue };
            let host_name = crate::webdash::computer_name();
            for d in devices {
                if sent.get(&d.id).is_some_and(|(at, f)| *at == d.connected_at && f == fp) {
                    continue;
                }
                let mut data = snapshot.clone();
                data["deviceId"] = json!(d.id);
                data["hostName"] = json!(host_name);
                if link.host.send(&d.id, CMD_BOARD, data).is_ok() {
                    sent.insert(d.id.clone(), (d.connected_at, *fp));
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct Tasks(StdMutex<Vec<Value>>);
    impl super::super::LinkEvents for Tasks {
        fn status(&self, _status: &super::super::LinkStatus) {}
        fn remote(&self, _office: &super::super::RemoteOffice) {}
        fn task(&self, data: &Value) {
            self.0.lock().unwrap().push(data.clone());
        }
    }

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("link-work-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn host_writes_forwarded_inbox_files_and_reports() {
        let dir = temp();
        let events = Tasks::default();
        let states = Mutex::new(HashMap::new());
        host_event(Some(&dir), &events, &states, "d-1", KIND_INBOX, &json!({ "name": "20261127T000000000Z-ab.md", "text": "---\nid: t-1\n---\n# hi\n" }));
        host_event(Some(&dir), &events, &states, "d-2", KIND_INBOX, &json!({ "name": "20261127T000000000Z-ab.md", "text": "second" }));
        host_event(Some(&dir), &events, &states, "d-1", KIND_INBOX, &json!({ "name": "../escape.md", "text": "x" }));
        host_event(Some(&dir), &events, &states, "d-1", KIND_INBOX, &json!({ "name": ".hidden.md", "text": "x" }));
        let inbox = dir.join("work-inbox");
        assert_eq!(std::fs::read_to_string(inbox.join("20261127T000000000Z-ab.md")).unwrap(), "---\nid: t-1\n---\n# hi\n");
        assert_eq!(std::fs::read_to_string(inbox.join("20261127T000000000Z-ab-d-2.md")).unwrap(), "second", "같은 이름은 덮지 않는다");
        assert!(!dir.join("escape.md").exists());
        let mut names: Vec<_> = std::fs::read_dir(&inbox).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, vec!["20261127T000000000Z-ab-d-2.md", "20261127T000000000Z-ab.md"]);

        host_event(Some(&dir), &events, &states, "d-1", KIND_REPORT, &json!({ "taskId": "t-abc", "text": "status: done\n" }));
        host_event(Some(&dir), &events, &states, "d-1", KIND_REPORT, &json!({ "taskId": "../t", "text": "x" }));
        assert_eq!(std::fs::read_to_string(inbox.join("reports").join("t-abc.md")).unwrap(), "status: done\n");
        assert_eq!(std::fs::read_dir(inbox.join("reports")).unwrap().count(), 1);

        host_event(Some(&dir), &events, &states, "d-1", KIND_TASK, &json!({ "taskId": "t-abc", "status": "review" }));
        let tasks = events.0.lock().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["deviceId"], json!("d-1"));
        assert_eq!(tasks[0]["status"], json!("review"));
        assert_eq!(states.lock().unwrap()["d-1:t-abc"]["status"], json!("review"), "렌더러가 늦게 떠도 줄 수 있게 들고 있다");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn client_board_copy_and_received_sessions() {
        let dir = temp();
        let work = ClientWork::new(dir.clone());
        work.write_board(&json!({ "deviceId": "d-9", "hostName": "Studio", "items": [{ "id": "t-1" }], "agents": [{ "id": "a", "name": "Ada" }] }));
        let board = host_board_dir(&dir);
        assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(board.join("work-items.json")).unwrap()).unwrap(), json!([{ "id": "t-1" }]));
        assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(board.join("profiles.json")).unwrap()).unwrap()["agents"][0]["name"], json!("Ada"));
        let info: Value = serde_json::from_slice(&std::fs::read(board.join(HOST_INFO_FILE)).unwrap()).unwrap();
        assert_eq!(info["deviceId"], json!("d-9"));
        assert!(host_board_inbox(&dir).join("reports").is_dir());

        work.remember_session("s-1", "t-1");
        work.remember_session("s-2", "t-2");
        work.remember_session("s-1", "t-3");
        let file: Value = serde_json::from_slice(&std::fs::read(dir.join(RECEIVED_FILE)).unwrap()).unwrap();
        assert_eq!(file["sessions"], json!({ "s-1": "t-3", "s-2": "t-2" }));
        for n in 0..MAX_RECEIVED_SESSIONS + 5 {
            work.remember_session(&format!("x-{n}"), "t-9");
        }
        let file: Value = serde_json::from_slice(&std::fs::read(dir.join(RECEIVED_FILE)).unwrap()).unwrap();
        assert_eq!(file["sessions"].as_object().unwrap().len(), MAX_RECEIVED_SESSIONS);
        assert!(file["sessions"].get("s-1").is_none(), "오래된 세션부터 잊는다");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn received_tasks_done_on_the_host_become_done_here() {
        let dir = temp();
        let store = crate::persistence::work_panels_store::WorkPanelsStore::new(dir.clone());
        let item = |id: &str, received: bool| -> crate::types::WorkItem {
            serde_json::from_value(json!({
                "id": id, "title": id, "body": "", "status": "review", "order": 0.0, "sessionIds": [],
                "createdAt": 1, "updatedAt": 1,
                "received": if received { json!({ "hostName": "H", "cwd": "/x", "receivedAt": 1 }) } else { Value::Null },
            })).unwrap()
        };
        store.save_work_items(&[item("t-a", true), item("t-b", true), item("t-c", false), item("t-new", true)]).unwrap();
        let board = json!({ "items": [
            { "id": "t-a", "status": "done" }, { "id": "t-b", "status": "review" }, { "id": "t-c", "status": "done" },
        ] });
        assert!(sync_received(&store, &board));
        let status = |id: &str| serde_json::to_value(store.load_work_items().unwrap().into_iter().find(|i| i.id == id).unwrap().status).unwrap();
        assert_eq!(status("t-a"), json!("done"));
        assert_eq!(status("t-b"), json!("review"));
        assert_eq!(status("t-c"), json!("review"), "받은 업무가 아니면 두고");
        assert_eq!(status("t-new"), json!("review"), "사본에 없는(막 받은) 업무도 둔다");
        assert!(!sync_received(&store, &board), "바뀔 것이 없으면 쓰지 않는다");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn client_forwards_host_board_inbox_and_reports() {
        let dir = temp();
        let link = LinkState::new(None, Arc::new(|| "c".into()), Arc::new(super::super::fake::NoLinkEvents));
        let work = ClientWork::new(dir.clone());
        let inbox = host_board_inbox(&dir);
        std::fs::create_dir_all(inbox.join("reports")).unwrap();
        std::fs::write(inbox.join("20261127T000000001Z-b.md"), "second").unwrap();
        std::fs::write(inbox.join("20261127T000000000Z-a.md"), "first").unwrap();
        std::fs::write(inbox.join(".20261127T000000002Z-c.md.tmp"), "writing").unwrap();
        std::fs::write(inbox.join("reports").join("t-1.md"), "status: done").unwrap();
        assert_eq!(work.forward_inbox(&link), 3);
        assert!(inbox.join(".20261127T000000002Z-c.md.tmp").exists(), "쓰는 중인 임시 파일은 두고");
        assert!(!inbox.join("20261127T000000000Z-a.md").exists(), "보낸 것은 지운다");
        assert_eq!(link.client.status().pending, 3);
        assert_eq!(work.forward_inbox(&link), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
