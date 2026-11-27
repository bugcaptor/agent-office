// src-tauri/src/webdash/snapshot.rs
//
// 이 앱의 캐릭터들 지금 모습(웹 상태판 `/api/state`의 `characters`·`officeAgent`·`outputAt`). 웹 라우트와
// 머신 연동 클라이언트(`link::share`, 호스트로 올림)가 같은 것을 쓴다.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::WebDashState;
use crate::control::ControlContext;
use crate::httpapi::session_state_str;
use crate::types::OFFICE_AGENT_ID;

/// 터미널 포그라운드 조회(ps·pgrep)를 다시 하기 전 간격. 여러 기기가 몇 초마다 물어도
/// 캐릭터마다 이 간격에 한 번만 프로세스를 띄운다. 상태판 주기(2.5초, `dash.html`의
/// `setInterval(tick, …)`)보다 길어야 브라우저 하나만 열어도 캐시가 맞는다. 웹에서 명령을
/// 보내면(`activity`) 캐시를 비워 바로 다시 본다.
const PROBE_TTL: Duration = Duration::from_secs(3);

/// 터미널에서 지금 무엇이 도는가.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ShellProbe {
    /// 포그라운드에서 도는 에이전트 CLI(`claude`·`codex`).
    pub cli: Option<&'static str>,
    /// 셸 프롬프트인가. 모르면 None.
    pub at_prompt: Option<bool>,
}

/// 캐릭터별 마지막 조회(`PROBE_TTL` 동안 다시 쓴다).
#[derive(Default)]
pub(crate) struct ProbeCache(Mutex<HashMap<String, (Instant, ShellProbe)>>);

impl ProbeCache {
    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

/// 캐릭터들의 터미널 포그라운드를 본다(블로킹 — ps·pgrep을 띄운다).
pub(crate) fn probe_shells(ctl: &ControlContext, cache: &ProbeCache, agents: &[String]) -> HashMap<String, ShellProbe> {
    let mut out = HashMap::new();
    for agent in agents {
        let cached = cache.0.lock().unwrap().get(agent).filter(|(at, _)| at.elapsed() < PROBE_TTL).map(|(_, p)| *p);
        let probe = cached.unwrap_or_else(|| {
            let command = ctl.manager.foreground_command(agent);
            let probe = ShellProbe {
                cli: super::foreground_cli(&ctl.manager, agent, command.as_deref()),
                at_prompt: if command.is_some() { Some(false) } else { ctl.manager.is_shell_prompt(agent) },
            };
            cache.0.lock().unwrap().insert(agent.clone(), (Instant::now(), probe));
            probe
        });
        out.insert(agent.clone(), probe);
    }
    out
}

pub(crate) struct LocalView {
    pub characters: Vec<Value>,
    pub office_agent: Value,
    /// 캐릭터별 마지막 PTY 출력 시각(epoch ms).
    pub output_at: serde_json::Map<String, Value>,
}

/// 이 앱 캐릭터들의 지금 모습.
pub(crate) async fn local_view(ctl: &Arc<ControlContext>, dash: &Arc<WebDashState>) -> LocalView {
    let profiles = ctl.store.load();
    let mut live = HashMap::new();
    for (sid, agent, st) in ctl.registry.snapshot() {
        live.insert(agent, (sid, st));
    }
    let running: Vec<String> = live
        .iter()
        .filter(|(_, (_, st))| session_state_str(*st) == "running")
        .map(|(agent, _)| agent.clone())
        .collect();
    let probes = {
        let (ctl, dash) = (ctl.clone(), dash.clone());
        tokio::task::spawn_blocking(move || probe_shells(&ctl, &dash.probes, &running)).await.unwrap_or_default()
    };
    let work_items = ctl.work_panels.load_work_items().unwrap_or_default();
    let characters: Vec<Value> = profiles
        .agents
        .iter()
        .map(|a| {
            let session = live.get(&a.id);
            let turn = session.and_then(|(sid, _)| ctl.turns.get(sid));
            let probe = probes.get(&a.id).copied().unwrap_or_default();
            let asks = session.map(|(sid, _)| ctl.observer.permissions().list(sid)).unwrap_or_default();
            let queue = ctl.chat_queue.view(ctl, &a.id, session.map(|(sid, _)| sid.as_str()));
            json!({
                "agentId": a.id,
                "name": a.name,
                "role": a.role,
                "cwd": a.cwd,
                "clockedOut": a.clocked_out,
                "hasStartupCommand": a.startup_command.as_ref().is_some_and(|c| !c.trim().is_empty()),
                "portrait": a.portrait_updated_at,
                "defaultCli": a.default_agent.as_ref().map(|w| w.cli.clone()),
                "defaultAgent": a.default_agent,
                "session": session.map(|(sid, st)| json!({ "sessionId": sid, "state": session_state_str(*st) })),
                "turn": turn,
                "asks": asks,
                "queue": queue,
                "work": super::work_progress::agent_work(&work_items, &a.id),
                "label": dash.label(&a.id),
                "cli": probe.cli,
                "atPrompt": probe.at_prompt,
                "shellAssist": a.shell_assist == Some(true),
            })
        })
        .collect();
    let office_agent = {
        let session = live.get(OFFICE_AGENT_ID);
        let probe = probes.get(OFFICE_AGENT_ID).copied().unwrap_or_default();
        json!({
            "session": session.map(|(sid, st)| json!({ "sessionId": sid, "state": session_state_str(*st) })),
            "turn": session.and_then(|(sid, _)| ctl.turns.get(sid)),
            "asks": session.map(|(sid, _)| ctl.observer.permissions().list(sid)).unwrap_or_default(),
            "queue": ctl.chat_queue.view(ctl, OFFICE_AGENT_ID, session.map(|(sid, _)| sid.as_str())),
            "cli": probe.cli,
            "atPrompt": probe.at_prompt,
            "cwd": session.and_then(|_| ctl.gate.sink().cwd_of(OFFICE_AGENT_ID)),
            "label": dash.label(OFFICE_AGENT_ID),
            "opening": crate::office_agent::is_opening(),
            "error": crate::office_agent::last_error(),
        })
    };
    let output_at = live
        .keys()
        .filter_map(|agent| ctl.manager.last_output_ms(agent).map(|ms| (agent.clone(), json!(ms))))
        .collect();
    LocalView { characters, office_agent, output_at }
}

/// 전사를 찾을 작업 폴더: 세션이 cd 한 곳, 없으면 캐릭터 프로필, 그것도 없으면 홈. 모르는 캐릭터면 None
/// (일반 에이전트는 프로필이 없어 세션 것만 본다).
pub(crate) fn agent_cwd(ctl: &ControlContext, agent: &str) -> Option<String> {
    let profile_cwd = if crate::types::is_office_agent_id(agent) {
        None
    } else {
        ctl.store.load().agents.into_iter().find(|a| a.id == agent)?.cwd
    };
    // Codex 전사는 작업 폴더로 찾는다. 세션이 cd 했으면 그 폴더가 맞다.
    Some(ctl.gate.sink().cwd_of(agent).or(profile_cwd).unwrap_or_else(crate::session::manager::home_dir))
}
