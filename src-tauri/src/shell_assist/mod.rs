// src-tauri/src/shell_assist/mod.rs
//
// 셸 캐릭터: 프로필에서 `shellAssist`를 켠 캐릭터는 터미널에 CLI 없이 셸만 두고, 사람이 웹
// 상태판(또는 앱 입력줄)에 말로 시키면 LLM(주로 로컬 모델)이 명령 하나를 제안한다. 실행은
// 늘 사람이 누른다. 명령마다 출력 블록을 따로 모아 웹에 보이고, 끝나면 결과를 한두 줄로 설명한다.
//
// 흐름:
//   세션 시작 → `session_started`가 셸 훅 파일을 읽히는 한 줄을 돌려준다(manager가 stdin에 씀)
//   PTY 출력 → `feed`(출력 펌프) → 기록기가 표식으로 블록을 가른다 → 끝나면 설명 요청
//   웹 → `suggest`(말 → 명령) · `run`(명령을 셸에 넣음) · `snapshot`(블록 목록)

pub mod config;
pub mod llm;
pub mod marks;
pub mod recorder;
pub mod scripts;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use parking_lot::{Mutex, RwLock};
use serde_json::{json, Value};

use crate::control::ControlContext;
use crate::types::AgentProfile;
use config::{ConfigStore, ShellAssistConfig};
use recorder::{Event, Explanation, Phase, Recorder};

/// 자연어 요청 상한.
pub const MAX_REQUEST_CHARS: usize = 2_000;
/// 실행할 명령 상한.
pub const MAX_COMMAND_BYTES: usize = 8 * 1024;
/// 명령 제안에 실을 최근 블록 수와 글자 상한.
const CONTEXT_BLOCKS: usize = 3;
const CONTEXT_MAX_CHARS: usize = 4_000;
/// 설명 요청에 실을 출력 줄(앞·뒤).
const EXPLAIN_HEAD: usize = 20;
const EXPLAIN_TAIL: usize = 80;
const EXPLAIN_MAX_CHARS: usize = 6_000;

struct Runtime {
    ctl: Arc<ControlContext>,
    config: ConfigStore,
}

fn runtime() -> Option<&'static Runtime> {
    static_runtime().get()
}

fn static_runtime() -> &'static OnceLock<Runtime> {
    static R: OnceLock<Runtime> = OnceLock::new();
    &R
}

fn agents() -> &'static RwLock<HashSet<String>> {
    static A: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();
    A.get_or_init(Default::default)
}

fn recorders() -> &'static Mutex<HashMap<String, Recorder>> {
    static R: OnceLock<Mutex<HashMap<String, Recorder>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// 부팅 때 한 번. 이 전에는 설명·제안·실행이 모두 꺼져 있다(기록만 한다).
pub fn install(ctl: Arc<ControlContext>, app_data_dir: &std::path::Path) {
    set_agents(&ctl.store.load().agents);
    let _ = static_runtime().set(Runtime { ctl, config: ConfigStore::in_app_data(app_data_dir) });
}

/// 프로필 저장 때마다 셸 캐릭터 목록을 다시 잡는다.
pub fn set_agents(profiles: &[AgentProfile]) {
    let next: HashSet<String> = profiles.iter().filter(|p| p.shell_assist == Some(true)).map(|p| p.id.clone()).collect();
    recorders().lock().retain(|agent, _| next.contains(agent));
    *agents().write() = next;
}

pub fn is_enabled(agent_id: &str) -> bool {
    agents().read().contains(agent_id)
}

pub fn load_config() -> ShellAssistConfig {
    runtime().map(|r| r.config.load()).unwrap_or_default()
}

pub fn save_config(config: ShellAssistConfig) -> Result<ShellAssistConfig, String> {
    let rt = runtime().ok_or("not-ready")?;
    let config = config.normalized();
    rt.config.save(&config).map_err(|e| e.to_string())?;
    Ok(config)
}

/// 이 앱이 새로 띄운 세션. 셸 캐릭터면 훅 파일을 쓰고 stdin에 넣을 줄을 돌려준다.
pub fn session_started(agent_id: &str, session_id: &str, program: &str) -> Option<String> {
    if !is_enabled(agent_id) {
        return None;
    }
    let kind = scripts::ShellKind::from_program(program);
    {
        let mut map = recorders().lock();
        let rec = map
            .entry(agent_id.to_string())
            .or_insert_with(|| Recorder::new(session_id, None));
        if rec.session_id != session_id {
            *rec = Recorder::new(session_id, None);
        }
        rec.shell = kind.map(|k| k.name().to_string());
        rec.rev += 1;
    }
    let kind = kind?;
    let base = std::env::temp_dir().join("agent-office");
    match scripts::write_script(&base, kind) {
        Ok(path) => Some(scripts::inject_line(kind, &path)),
        Err(e) => {
            eprintln!("agent-office: shell marks script write failed: {e}");
            None
        }
    }
}

/// 출력 펌프가 PTY 바이트마다 부른다. 셸 캐릭터가 아니면 곧바로 돌아간다.
pub fn feed(agent_id: &str, session_id: &str, bytes: &[u8]) {
    if !is_enabled(agent_id) {
        return;
    }
    let now = crate::types::now_ms();
    let finished: Vec<u64> = {
        let mut map = recorders().lock();
        let rec = map
            .entry(agent_id.to_string())
            .or_insert_with(|| Recorder::new(session_id, None));
        if rec.session_id != session_id {
            // 앱 재시작 뒤 입양한 세션 등: 새 기록으로 시작한다.
            *rec = Recorder::new(session_id, None);
        }
        rec.feed(bytes, now)
            .into_iter()
            .map(|Event::Finished(id)| id)
            .collect()
    };
    for id in finished {
        on_finished(agent_id, session_id, id);
    }
}

fn deps(rt: &Runtime) -> llm::LlmDeps {
    let config = rt.config.load();
    let (summary_models, lang) = {
        let s = rt.ctl.settings.read().unwrap();
        (s.summary_models.clone(), crate::i18n::ui_lang(&s))
    };
    let openrouter_key = (config.fallback_enabled
        && config.fallback_provider == crate::persistence::settings_store::SummaryProvider::Openrouter)
        .then(|| rt.ctl.key_store.openrouter_key())
        .flatten();
    llm::LlmDeps { config, summary_models, openrouter_key, lang }
}

fn set_explanation(agent_id: &str, session_id: &str, id: u64, value: Explanation) {
    let mut map = recorders().lock();
    let Some(rec) = map.get_mut(agent_id).filter(|r| r.session_id == session_id) else { return };
    if let Some(block) = rec.block_mut(id) {
        block.explanation = Some(value);
        rec.rev += 1;
    }
}

/// 출력 없이 끝난 명령은 LLM을 부르지 않고 바로 적는다.
fn quiet_text(lang: crate::i18n::Lang, exit: Option<i32>) -> String {
    use crate::i18n::Lang;
    match (lang, exit) {
        (Lang::Ko, Some(0)) => "출력 없이 성공했습니다.".into(),
        (Lang::Ko, Some(code)) => format!("출력 없이 실패했습니다(종료 코드 {code})."),
        (Lang::Ko, None) => "출력 없이 끝났습니다.".into(),
        (Lang::En, Some(0)) => "Succeeded with no output.".into(),
        (Lang::En, Some(code)) => format!("Failed with no output (exit code {code})."),
        (Lang::En, None) => "Finished with no output.".into(),
    }
}

fn on_finished(agent_id: &str, session_id: &str, id: u64) {
    let Some(rt) = runtime() else { return };
    let deps = deps(rt);
    if !deps.config.explain {
        return;
    }
    let (user, quiet) = {
        let map = recorders().lock();
        let Some(rec) = map.get(agent_id).filter(|r| r.session_id == session_id) else { return };
        let Some(block) = rec.blocks.iter().find(|b| b.id == id) else { return };
        if block.command.trim().is_empty() {
            return;
        }
        let quiet = block.lines.is_empty() && !block.fullscreen;
        let exit = block.exit.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into());
        let mut user = format!(
            "Shell: {}\nDirectory: {}\nCommand: {}\nExit code: {exit}\n",
            rec.shell.as_deref().unwrap_or("unknown"),
            block.cwd.as_deref().unwrap_or("unknown"),
            block.command,
        );
        if block.fullscreen {
            user.push_str("(The command used a full-screen terminal UI; its screen was not recorded.)\n");
        }
        user.push_str("Output:\n");
        user.push_str(&tail_chars(&block.output_excerpt(EXPLAIN_HEAD, EXPLAIN_TAIL), EXPLAIN_MAX_CHARS));
        (user, quiet.then_some(block.exit))
    };
    if let Some(exit) = quiet {
        let text = quiet_text(deps.lang, exit);
        set_explanation(agent_id, session_id, id, Explanation::Done { text, source: String::new() });
        return;
    }
    if !deps.config.any_llm() {
        set_explanation(agent_id, session_id, id, Explanation::Off);
        return;
    }
    set_explanation(agent_id, session_id, id, Explanation::Pending);
    let (agent, session) = (agent_id.to_string(), session_id.to_string());
    tauri::async_runtime::spawn(async move {
        let system = llm::explain_prompt(deps.lang);
        let value = match llm::complete(&deps, &system, &user, 400).await {
            Ok(answer) => Explanation::Done { text: llm::clean_explanation(&answer.text), source: answer.source },
            Err(error) => Explanation::Failed { error },
        };
        set_explanation(&agent, &session, id, value);
    });
}

fn tail_chars(text: &str, max: usize) -> String {
    let n = text.chars().count();
    if n <= max {
        return text.to_string();
    }
    let skip = n - max;
    format!("…{}", text.chars().skip(skip).collect::<String>())
}

/// 웹에 줄 지금 상태. `known_rev`가 지금과 같으면 `{same:true}`만.
pub fn snapshot(agent_id: &str, known_rev: Option<u64>) -> Value {
    let config = load_config();
    let llm = config.any_llm();
    let map = recorders().lock();
    let Some(rec) = map.get(agent_id) else {
        return json!({ "sessionId": null, "phase": Phase::Unknown, "blocks": [], "rev": 0, "llm": llm });
    };
    if known_rev == Some(rec.rev) {
        return json!({ "same": true, "rev": rec.rev });
    }
    let blocks: Vec<Value> = rec
        .blocks
        .iter()
        .map(|b| {
            json!({
                "id": b.id,
                "command": b.command,
                "cwd": b.cwd,
                "startedAt": b.started_at,
                "endedAt": b.ended_at,
                "exit": b.exit,
                "output": b.lines.iter().cloned().collect::<Vec<_>>().join("\n"),
                "lines": b.lines.len() + b.dropped,
                "dropped": b.dropped,
                "fullscreen": b.fullscreen,
                "origin": b.origin,
                "explanation": b.explanation,
            })
        })
        .collect();
    json!({
        "sessionId": rec.session_id,
        "shell": rec.shell,
        "cwd": rec.cwd,
        "phase": rec.phase,
        "blocks": blocks,
        "live": rec.live_tail(),
        "rev": rec.rev,
        "llm": llm,
        "explain": config.explain,
    })
}

fn check_text(text: &str, max_bytes: usize) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("empty".into());
    }
    if text.len() > max_bytes {
        return Err("too-large".into());
    }
    if text.chars().any(|c| c.is_control() && c != '\t') {
        return Err(if text.contains('\n') { "shell-multiline".into() } else { "control-char".into() });
    }
    Ok(())
}

/// 지금 셸이 새 명령을 받을 수 있는가(프롬프트에 있고 앱 터미널에 쓰다 만 입력이 없다).
fn ready_for_command(ctl: &ControlContext, agent_id: &str) -> Result<String, String> {
    if !ctl.manager.is_running(agent_id) {
        return Err("not-running".into());
    }
    let session_id = ctl.manager.session_id_for(agent_id).ok_or("not-running")?;
    let phase = recorders().lock().get(agent_id).filter(|r| r.session_id == session_id).map(|r| r.phase);
    match phase {
        Some(Phase::Running) => return Err("shell-running".into()),
        Some(Phase::Prompt) => {}
        // 표식이 없는 셸(fish·WSL, 표식을 걸러 내는 콘솔)은 포그라운드로만 판단한다.
        _ => {
            if ctl.manager.is_shell_prompt(agent_id) == Some(false) {
                return Err("shell-running".into());
            }
        }
    }
    if ctl.gate.has_uncommitted(agent_id) {
        return Err("human-input-pending".into());
    }
    Ok(session_id)
}

/// 웹에서 [실행]: 명령을 셸에 넣고 Enter.
pub fn run(agent_id: &str, command: &str) -> Result<(), String> {
    let rt = runtime().ok_or("not-ready")?;
    if !is_enabled(agent_id) {
        return Err("shell-not-enabled".into());
    }
    let command = command.trim();
    check_text(command, MAX_COMMAND_BYTES)?;
    let session_id = ready_for_command(&rt.ctl, agent_id)?;
    if let Some(rec) = recorders().lock().get_mut(agent_id).filter(|r| r.session_id == session_id) {
        rec.note_web_command(command, crate::types::now_ms());
    }
    // 사람이 웹에서 고른 입력이라 사람 입력 경로로 넣는다(자동 제출이 잠근 동안은 줄을 선다).
    rt.ctl.gate.note_human(agent_id, &format!("{command}\r"));
    Ok(())
}

fn context_of(rec: &Recorder) -> String {
    let mut parts: Vec<String> = Vec::new();
    for b in rec.blocks.iter().rev().take(CONTEXT_BLOCKS).collect::<Vec<_>>().into_iter().rev() {
        let exit = match (b.ended_at, b.exit) {
            (None, _) => " (still running)".to_string(),
            (Some(_), Some(c)) => format!(" (exit {c})"),
            (Some(_), None) => String::new(),
        };
        let mut part = format!("$ {}{exit}\n", b.command);
        if b.fullscreen {
            part.push_str("(full-screen app)\n");
        }
        part.push_str(&b.output_excerpt(8, 30));
        parts.push(part);
    }
    tail_chars(&parts.join("\n\n"), CONTEXT_MAX_CHARS)
}

/// 말 → 명령 하나. 실행하지 않는다.
pub async fn suggest(agent_id: &str, request: &str) -> Result<llm::Suggestion, String> {
    let rt = runtime().ok_or("not-ready")?;
    if !is_enabled(agent_id) {
        return Err("shell-not-enabled".into());
    }
    let request = request.trim();
    check_text(&request.replace('\n', " "), MAX_REQUEST_CHARS * 4)?;
    if request.chars().count() > MAX_REQUEST_CHARS {
        return Err("too-large".into());
    }
    let (shell, cwd, context) = {
        let map = recorders().lock();
        match map.get(agent_id) {
            Some(rec) => (rec.shell.clone(), rec.cwd.clone(), context_of(rec)),
            None => (None, None, String::new()),
        }
    };
    let cwd = cwd
        .or_else(|| rt.ctl.manager.cwd_of(agent_id))
        .unwrap_or_else(crate::session::manager::home_dir);
    let shell = shell.unwrap_or_else(|| if cfg!(windows) { "pwsh".into() } else { "zsh".into() });
    let deps = deps(rt);
    let env = llm::ShellEnv { shell: &shell, os: llm::os_name(), cwd: &cwd };
    llm::suggest(&deps, &env, request, &context).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_text_rules() {
        assert_eq!(check_text("  ", 10).unwrap_err(), "empty");
        assert_eq!(check_text("ls\nrm x", 100).unwrap_err(), "shell-multiline");
        assert_eq!(check_text("ls\x1b", 100).unwrap_err(), "control-char");
        assert_eq!(check_text("abcdef", 3).unwrap_err(), "too-large");
        assert!(check_text("echo\t'a b'", 100).is_ok());
    }

    #[test]
    fn context_lists_recent_blocks_oldest_first() {
        let mut rec = Recorder::new("s", None);
        for i in 0..5 {
            rec.feed(format!("\x1b]6973;C;cmd{i}\x07out{i}\r\n\x1b]6973;D;{i}\x07").as_bytes(), i);
        }
        rec.feed(b"\x1b]6973;C;tail -f log\x07", 9);
        let ctx = context_of(&rec);
        assert!(!ctx.contains("cmd2"));
        let a = ctx.find("$ cmd3 (exit 3)").unwrap();
        let b = ctx.find("$ cmd4 (exit 4)\nout4").unwrap();
        let c = ctx.find("$ tail -f log (still running)").unwrap();
        assert!(a < b && b < c, "{ctx}");
    }

    #[test]
    fn tail_chars_keeps_the_end() {
        assert_eq!(tail_chars("abcdef", 3), "…def");
        assert_eq!(tail_chars("abc", 3), "abc");
    }

    #[test]
    fn quiet_text_by_exit() {
        assert_eq!(quiet_text(crate::i18n::Lang::Ko, Some(0)), "출력 없이 성공했습니다.");
        assert!(quiet_text(crate::i18n::Lang::En, Some(2)).contains("exit code 2"));
    }
}
