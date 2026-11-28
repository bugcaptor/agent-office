// src-tauri/src/peer/cli.rs
//
// `agent-office peer` 오프라인 서브커맨드(`office-peer` shim)와 앱 쪽 요청 감시.
// CLI는 앱 데이터의 `peer-requests/<id>.json`에 요청을 쓰고 앱이 0.3초마다 가져가 `<id>.result.json`으로
// 답한다. `ask`는 그 뒤 `peer-replies/<대화id>.json`을 기다린다(가져갈 때 이름을 바꿔 앱과 겨루지 않는다).
// 최근 대화(`threads`)는 앱이 쓰는 `peer-threads.json`을 읽는다.
// 종료 코드: 0 성공, 1 실패, 2 사용법 오류, 8 답을 아직 못 받음(늦게 오면 이 세션에 들어온다).
use std::io::{Read as _, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{PeerState, Thread, REPLIES_DIR, REQUESTS_DIR, THREADS_FILE};

const USAGE: &str = "usage: office-peer agents [--json]
       office-peer status <character> [--json]
       office-peer log <character> [--turns <n>] [--json]
       office-peer ask <character> <message...|-> [--wait <seconds>] [--no-wait] [--json]
       office-peer reply <id> <answer...|->
       office-peer threads [--json]
       office-peer launch <character> [--cli <name>]
       office-peer resume <character>
       office-peer interrupt <character>
       office-peer unqueue <character> <queue-id>
Looks up and talks to other Agent Office characters on this machine and on linked machines.
<character> is a character name or <character>@<machine> (see `agents`). `-` reads the text from stdin.
`ask` puts the message into that character's session and waits for its `reply` (default 100 s,
--wait up to 600); if it does not come in time, the reply is put into your session when it arrives.
";

const EXIT_FAIL: i32 = 1;
const EXIT_USAGE: i32 = 2;
pub const EXIT_NO_REPLY: i32 = 8;
/// 앱이 요청을 가져가기를 기다리는 시간(감시는 0.3초 간격).
const PICKUP: Duration = Duration::from_secs(6);
/// 가져간 요청의 답을 기다리는 시간(다른 머신에서 세션을 띄우는 데 수십 초 걸린다).
const ANSWER: Duration = Duration::from_secs(120);
const POLL: Duration = Duration::from_millis(250);
const DEFAULT_WAIT: u64 = 100;
const MAX_WAIT: u64 = 600;

pub struct PeerEnv {
    pub app_data: Option<PathBuf>,
    pub agent_id: Option<String>,
    pub in_session: bool,
}

impl PeerEnv {
    fn from_process() -> Self {
        let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
        Self {
            app_data: var("AGENT_OFFICE_APP_DATA").map(PathBuf::from).or_else(crate::control::client::default_app_data),
            agent_id: var("AGENT_OFFICE_AGENT_ID").map(|v| v.to_string_lossy().into_owned()),
            in_session: var("AGENT_OFFICE_SESSION").is_some(),
        }
    }
}

pub fn run(args: Vec<String>) -> i32 {
    let mut stdin = String::new();
    let reads_stdin = args.iter().skip(1).any(|a| a == "-");
    if reads_stdin {
        let _ = std::io::stdin().read_to_string(&mut stdin);
    }
    let timing = Timing { pickup: PICKUP, answer: ANSWER };
    run_with(&args, &PeerEnv::from_process(), &stdin, &mut std::io::stdout(), &mut std::io::stderr(), timing)
}

pub fn shim_path() -> PathBuf {
    crate::work_inbox_cli::office_shim_path("office-peer")
}

#[derive(Clone, Copy)]
pub struct Timing {
    pub pickup: Duration,
    pub answer: Duration,
}

#[derive(Default)]
struct Opts {
    json: bool,
    no_wait: bool,
    wait: Option<u64>,
    turns: Option<u64>,
    cli: Option<String>,
    positional: Vec<String>,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => o.json = true,
            "--no-wait" => o.no_wait = true,
            "--wait" => o.wait = Some(it.next().and_then(|v| v.parse().ok()).ok_or("--wait needs seconds")?),
            "--turns" => o.turns = Some(it.next().and_then(|v| v.parse().ok()).ok_or("--turns needs a number")?),
            "--cli" => o.cli = Some(it.next().ok_or("--cli needs a value")?.clone()),
            "--" => o.positional.extend(it.by_ref().cloned()),
            f if f.starts_with("--") => return Err(format!("unknown option {f}")),
            _ => o.positional.push(a.clone()),
        }
    }
    Ok(o)
}

pub fn run_with(args: &[String], env: &PeerEnv, stdin: &str, out: &mut dyn Write, err: &mut dyn Write, timing: Timing) -> i32 {
    let Some(cmd) = args.first().map(String::as_str) else {
        let _ = write!(err, "{USAGE}");
        return EXIT_USAGE;
    };
    if matches!(cmd, "-h" | "--help" | "help") || args.iter().any(|a| a == "--help" || a == "-h") {
        let _ = write!(out, "{USAGE}");
        return 0;
    }
    let o = match parse(&args[1..]) {
        Ok(o) => o,
        Err(e) => return usage(err, &e),
    };
    let Some(data) = env.app_data.as_deref() else {
        return fail(err, "Agent Office data folder not found (AGENT_OFFICE_APP_DATA is not set)");
    };
    let text_of = |rest: &[String]| -> String {
        if rest.len() == 1 && rest[0] == "-" {
            stdin.trim().to_string()
        } else {
            rest.join(" ").trim().to_string()
        }
    };
    let pos = o.positional.as_slice();
    match cmd {
        "agents" => match call(data, json!({ "op": "agents" }), timing) {
            Ok(v) => print_agents(&v, &o, out),
            Err(e) => fail(err, &e),
        },
        "threads" => print_threads(data, &o, out),
        "status" | "log" | "launch" | "resume" | "interrupt" => {
            let [target] = pos else { return usage(err, &format!("{cmd} takes one character")) };
            let mut body = json!({ "op": cmd, "target": target });
            if let Some(n) = o.turns {
                body["turns"] = json!(n);
            }
            if let Some(c) = &o.cli {
                body["cli"] = json!(c);
            }
            match call(data, body, timing) {
                Ok(v) => print_op(cmd, &v, &o, out),
                Err(e) => fail(err, &e),
            }
        }
        "unqueue" => {
            let [target, id] = pos else { return usage(err, "unqueue takes a character and a queue id") };
            match call(data, json!({ "op": "queue-cancel", "target": target, "id": id }), timing) {
                Ok(v) => print_op("unqueue", &v, &o, out),
                Err(e) => fail(err, &e),
            }
        }
        "ask" => {
            let [target, rest @ ..] = pos else { return usage(err, "ask needs a character and a message") };
            let text = text_of(rest);
            if text.is_empty() {
                return usage(err, "ask needs a message");
            }
            if !env.in_session || env.agent_id.is_none() {
                return fail(err, "ask works only inside an Agent Office character session");
            }
            let id = super::new_thread_id();
            let body = json!({ "op": "ask", "id": id, "target": target, "text": text, "fromAgent": env.agent_id });
            let answer = match call(data, body, timing) {
                Ok(v) => v,
                Err(e) => return fail(err, &e),
            };
            let to = answer["thread"]["to"].clone();
            let label = serde_json::from_value::<crate::files::model::Party>(to).map(|p| p.label()).unwrap_or_default();
            let queued = answer["outcome"] == "queued";
            if o.no_wait {
                return done_no_wait(&id, &label, queued, &o, out);
            }
            let wait = Duration::from_secs(o.wait.unwrap_or(DEFAULT_WAIT).min(MAX_WAIT));
            match wait_reply(data, &id, wait) {
                Some(reply) => {
                    if o.json {
                        let _ = writeln!(out, "{}", serde_json::to_string_pretty(&reply).unwrap_or_default());
                    } else {
                        let _ = writeln!(out, "[reply {id} from {label}]\n{}", reply["reply"].as_str().unwrap_or(""));
                    }
                    0
                }
                None => {
                    let _ = writeln!(
                        err,
                        "office-peer: no reply from {label} yet ({id}{}). When it replies, the reply is put into this session; `office-peer threads` also shows it.",
                        if queued { ", the message is still queued until its turn ends" } else { "" }
                    );
                    EXIT_NO_REPLY
                }
            }
        }
        "reply" => {
            let [id, rest @ ..] = pos else { return usage(err, "reply needs a message id and an answer") };
            let text = text_of(rest);
            if text.is_empty() {
                return usage(err, "reply needs an answer");
            }
            if !env.in_session || env.agent_id.is_none() {
                return fail(err, "reply works only inside an Agent Office character session");
            }
            match call(data, json!({ "op": "reply", "id": id, "text": text, "fromAgent": env.agent_id }), timing) {
                Ok(v) => {
                    let label = serde_json::from_value::<crate::files::model::Party>(v["to"].clone()).map(|p| p.label()).unwrap_or_default();
                    let _ = writeln!(out, "replied to {label} ({id})");
                    0
                }
                Err(e) => fail(err, &e),
            }
        }
        other => usage(err, &format!("unknown command {other:?}")),
    }
}

fn usage(err: &mut dyn Write, msg: &str) -> i32 {
    let _ = writeln!(err, "office-peer: {msg}\n{USAGE}");
    EXIT_USAGE
}

fn fail(err: &mut dyn Write, msg: &str) -> i32 {
    let _ = writeln!(err, "office-peer: {}", explain(msg));
    EXIT_FAIL
}

/// 앱 오류 코드에 할 일을 붙인다.
fn explain(code: &str) -> String {
    let (key, detail) = code.split_once(": ").unwrap_or((code, ""));
    let hint = match key {
        "peer-off" => format!("agent talk is turned off on {detail} (Settings › Machine link › Agent talk)"),
        "peer-machine-offline" => format!("{detail} is not connected right now"),
        "peer-machine-unknown" => "that machine is not linked".into(),
        "peer-target-unknown" => format!("no character named {detail:?} (see `office-peer agents`)"),
        "peer-target-ambiguous" => format!("more than one character matches; use <character>@<machine>: {detail}"),
        "peer-unknown-character" => "that character is not on that machine".into(),
        "peer-self" => "that is you".into(),
        "peer-reply-first" => format!("that character is waiting for your reply to {detail}; answer it with `office-peer reply {detail} \"...\"` instead of asking back"),
        "peer-rate-limit" | "peer-pair-limit" => format!("too many messages ({detail}); wait before asking again"),
        "peer-depth-limit" => format!("too many agents are already waiting on each other (depth {detail}); answer what you have instead of asking further"),
        "peer-thread-unknown" => "no message with that id is waiting for your reply".into(),
        "peer-thread-not-yours" => "that message was sent to another character".into(),
        "peer-already-replied" => "that message was already answered".into(),
        "peer-not-in-session" | "peer-not-a-character" => "this works only inside an Agent Office character session".into(),
        "not-running" => "that character has no running session (try `office-peer launch <character>`)".into(),
        "chat-view-no-cli" => "that character's terminal has no agent CLI running (try `office-peer launch <character>`)".into(),
        "clocked-out" => "that character is clocked out; only a person can bring it back".into(),
        "session-open-timeout" => "the character's terminal did not open in time; try again".into(),
        "human-input-pending" | "work-send-human-input" => "a person is typing in that terminal; try later".into(),
        "queue-gone" => "that queued message is already gone".into(),
        "peer-timeout" | "link-request-timeout" => "the other machine did not answer in time".into(),
        _ => return code.to_string(),
    };
    format!("{hint} [{key}]")
}

/// 요청을 쓰고 앱의 답을 기다린다.
fn call(data: &Path, body: Value, timing: Timing) -> Result<Value, String> {
    let dir = data.join(REQUESTS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let id = format!("r-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let req = dir.join(format!("{id}.json"));
    let result = dir.join(format!("{id}.result.json"));
    crate::persistence::durable::write_atomic(&req, body.to_string().as_bytes()).map_err(|e| e.to_string())?;
    let take = |result: &Path| -> Option<Result<Value, String>> {
        let bytes = std::fs::read(result).ok()?;
        let _ = std::fs::remove_file(result);
        Some(match serde_json::from_slice::<Value>(&bytes) {
            Ok(v) => match v.get("error").and_then(Value::as_str) {
                Some(e) => Err(e.to_string()),
                None => Ok(v),
            },
            Err(e) => Err(e.to_string()),
        })
    };
    let start = Instant::now();
    let mut picked = false;
    loop {
        if let Some(r) = take(&result) {
            return r;
        }
        picked |= !req.exists();
        let limit = if picked { timing.pickup + timing.answer } else { timing.pickup };
        if start.elapsed() >= limit {
            break;
        }
        std::thread::sleep(POLL);
    }
    let _ = std::fs::remove_file(&req);
    // 지우는 사이에 앱이 답했을 수 있다.
    if let Some(r) = take(&result) {
        return r;
    }
    if picked {
        Err("Agent Office did not answer in time".into())
    } else {
        Err("Agent Office did not pick up the request (is the app running?)".into())
    }
}

/// 답 파일을 기다려 가져간다(이름을 바꿔 가져가 앱의 늦은 답 넣기와 겨루지 않는다).
fn wait_reply(data: &Path, id: &str, wait: Duration) -> Option<Value> {
    let dir = data.join(REPLIES_DIR);
    let file = dir.join(format!("{id}.json"));
    let mine = dir.join(format!("{id}.cli"));
    let start = Instant::now();
    loop {
        if std::fs::rename(&file, &mine).is_ok() {
            let v = std::fs::read(&mine).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
            let _ = std::fs::remove_file(&mine);
            if v.is_some() {
                return v;
            }
        }
        if start.elapsed() >= wait {
            return None;
        }
        std::thread::sleep(POLL);
    }
}

fn done_no_wait(id: &str, label: &str, queued: bool, o: &Opts, out: &mut dyn Write) -> i32 {
    if o.json {
        let _ = writeln!(out, "{}", json!({ "id": id, "to": label, "queued": queued }));
    } else {
        let state = if queued { "queued until its turn ends" } else { "delivered" };
        let _ = writeln!(out, "{id}\tsent to {label} ({state}); the reply will be put into this session");
    }
    0
}

fn ago(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        _ => format!("{}h{}m", s / 3600, (s % 3600) / 60),
    }
}

/// 캐릭터 한 줄 상태.
fn state_of(c: &Value, now: u64) -> String {
    let since = c["since"].as_u64().map(|t| format!(" {}", ago(now.saturating_sub(t)))).unwrap_or_default();
    let mut s = match (c["session"].as_str(), c["phase"].as_str()) {
        (None, _) => "no session".to_string(),
        (Some("running"), Some("working")) => format!("working{since}"),
        (Some("running"), Some("needs-human")) => format!("waiting for a human{since}"),
        (Some("running"), Some("idle")) => "idle".into(),
        (Some("running"), _) if c["atPrompt"] == true => "shell prompt (no agent CLI)".into(),
        (Some("running"), _) => "running".into(),
        (Some(other), _) => other.to_string(),
    };
    if let Some(n) = c["queued"].as_u64().filter(|n| *n > 0) {
        s.push_str(&format!(", {n} queued"));
    }
    if let Some(n) = c["asks"].as_u64().filter(|n| *n > 0) {
        s.push_str(&format!(", {n} permission request{}", if n == 1 { "" } else { "s" }));
    }
    s
}

fn print_agents(v: &Value, o: &Opts, out: &mut dyn Write) -> i32 {
    if o.json {
        let _ = writeln!(out, "{}", serde_json::to_string_pretty(&v["machines"]).unwrap_or_default());
        return 0;
    }
    let now = super::now_ms();
    for m in v["machines"].as_array().into_iter().flatten() {
        let name = m["machineName"].as_str().unwrap_or("");
        let here = if m["local"] == true { " (this machine)" } else { "" };
        let state = if m["online"] != true {
            " — offline"
        } else if m["enabled"] == false {
            " — agent talk off"
        } else {
            ""
        };
        let _ = writeln!(out, "{name}{here}{state}");
        for c in m["characters"].as_array().into_iter().flatten() {
            let cli = c["cli"].as_str().map(|s| format!("\t{s}")).unwrap_or_default();
            let cwd = c["cwd"].as_str().map(|s| format!("\t{s}")).unwrap_or_default();
            let role = c["role"].as_str().filter(|s| !s.is_empty()).map(|s| format!(" ({s})")).unwrap_or_default();
            let _ = writeln!(out, "  {}@{name}{role}\t{}{cli}{cwd}", c["name"].as_str().unwrap_or(""), state_of(c, now));
        }
    }
    0
}

fn print_op(cmd: &str, v: &Value, o: &Opts, out: &mut dyn Write) -> i32 {
    if o.json {
        let _ = writeln!(out, "{}", serde_json::to_string_pretty(v).unwrap_or_default());
        return 0;
    }
    let label = serde_json::from_value::<crate::files::model::Party>(v["target"].clone()).map(|p| p.label()).unwrap_or_default();
    match cmd {
        "status" => {
            let now = super::now_ms();
            let _ = writeln!(out, "{label}\t{}", state_of(v, now));
            for (k, key) in [("role", "role"), ("cli", "cli"), ("folder", "cwd"), ("waiting for", "waitingFor")] {
                if let Some(s) = v[key].as_str().filter(|s| !s.is_empty()) {
                    let _ = writeln!(out, "{k}: {s}");
                }
            }
            if let Some(t) = v["outputAt"].as_u64() {
                let _ = writeln!(out, "last output: {} ago", ago(now.saturating_sub(t)));
            }
            for q in v["queue"].as_array().into_iter().flatten() {
                let _ = writeln!(out, "queued {}: {}", q["id"].as_str().unwrap_or(""), q["text"].as_str().unwrap_or(""));
            }
            if let Some(w) = v["work"].as_object().filter(|w| !w.is_empty()) {
                let _ = writeln!(out, "work: {}", Value::Object(w.clone()));
            }
        }
        "log" => {
            if v["attached"] == false {
                let _ = writeln!(out, "{label}: no conversation log (no agent CLI transcript in that session)");
                return 0;
            }
            for i in v["items"].as_array().into_iter().flatten() {
                let _ = writeln!(out, "[{}] {}\n", i["role"].as_str().unwrap_or(""), i["text"].as_str().unwrap_or(""));
            }
        }
        "launch" => {
            let what = match v["outcome"].as_str() {
                Some("launched") => "launched; it got a short first prompt, so its state shows once that turn runs",
                Some("already-running") => "already running an agent CLI (nothing sent)",
                _ => "launch requested",
            };
            let _ = writeln!(out, "{label}: {what}");
        }
        _ => {
            let _ = writeln!(out, "{label}: {cmd} done");
        }
    }
    0
}

fn print_threads(data: &Path, o: &Opts, out: &mut dyn Write) -> i32 {
    let list: Vec<Thread> = std::fs::read(data.join(THREADS_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| serde_json::from_value(v["threads"].clone()).ok())
        .unwrap_or_default();
    let mut list = list;
    list.sort_by_key(|t| std::cmp::Reverse(t.created_at));
    if o.json {
        let _ = writeln!(out, "{}", serde_json::to_string_pretty(&list).unwrap_or_default());
        return 0;
    }
    let now = super::now_ms();
    for t in &list {
        let state = match &t.reply {
            Some(_) => "replied".to_string(),
            None => "waiting for reply".to_string(),
        };
        let _ = writeln!(out, "{}\t{} → {}\t{} ago\t{state}\t{}", t.id, t.from.label(), t.to.label(), ago(now.saturating_sub(t.created_at)), super::first_line(&t.text));
        if let Some(r) = &t.reply {
            let _ = writeln!(out, "  reply: {}", super::clamp(r, 600));
        }
    }
    0
}

// ── 앱 쪽: 요청 감시 ──────────────────────────────────────────────

/// `peer-requests/`를 0.3초마다 본다(앱 수명 동안 한 번). 요청마다 따로 돌린다(다른 머신 답을 기다리는 동안 막지 않게).
pub fn spawn_request_watcher(peer: Arc<PeerState>) {
    let Some(dir) = peer.data_dir().map(|d| d.join(REQUESTS_DIR)) else { return };
    let _ = std::fs::create_dir_all(&dir);
    if let Some(replies) = peer.data_dir().map(|d| d.join(REPLIES_DIR)) {
        let _ = std::fs::create_dir_all(replies);
    }
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.filter_map(Result::ok) {
                let path = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if name.ends_with(".result.json") {
                    let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|d| d > Duration::from_secs(600));
                    if old {
                        let _ = std::fs::remove_file(&path);
                    }
                    continue;
                }
                let Some(stem) = name.strip_suffix(".json").filter(|s| !s.contains(".tmp")).map(str::to_string) else { continue };
                let Ok(bytes) = std::fs::read(&path) else { continue };
                let _ = std::fs::remove_file(&path);
                let (peer, dir) = (peer.clone(), dir.clone());
                tauri::async_runtime::spawn(async move {
                    let answer = match serde_json::from_slice::<Value>(&bytes) {
                        Ok(body) => peer.handle_cli(&body).await,
                        Err(_) => json!({ "error": "bad-request" }),
                    };
                    let _ = crate::persistence::durable::write_atomic(&dir.join(format!("{stem}.result.json")), answer.to_string().as_bytes());
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call_cli(args: &[&str], env: &PeerEnv, stdin: &str) -> (i32, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let timing = Timing { pickup: Duration::from_millis(300), answer: Duration::from_millis(300) };
        let code = run_with(&args, env, stdin, &mut out, &mut err, timing);
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }

    #[test]
    fn usage_and_session_checks() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = PeerEnv { app_data: Some(tmp.path().into()), agent_id: None, in_session: false };
        assert_eq!(call_cli(&[], &outside, "").0, 2);
        assert_eq!(call_cli(&["bogus"], &outside, "").0, 2);
        assert_eq!(call_cli(&["ask", "dori"], &outside, "").0, 2);
        assert_eq!(call_cli(&["status"], &outside, "").0, 2);
        let (code, _, err) = call_cli(&["ask", "dori", "hi"], &outside, "");
        assert_eq!(code, 1);
        assert!(err.contains("only inside"), "{err}");
        let (code, out, _) = call_cli(&["--help"], &outside, "");
        assert_eq!(code, 0);
        assert!(out.contains("office-peer ask"));
    }

    #[test]
    fn gives_up_and_cleans_up_without_the_app() {
        let tmp = tempfile::tempdir().unwrap();
        let env = PeerEnv { app_data: Some(tmp.path().into()), agent_id: Some("a1".into()), in_session: true };
        let (code, _, err) = call_cli(&["ask", "dori", "-"], &env, "hello\nthere");
        assert_eq!(code, 1);
        assert!(err.contains("did not pick up"), "{err}");
        assert_eq!(std::fs::read_dir(tmp.path().join(REQUESTS_DIR)).unwrap().count(), 0);
    }

    #[test]
    fn answers_and_replies_are_read_from_files() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().to_path_buf();
        // 가짜 앱: 요청을 가져가 답을 쓰고, 대화 id로 답 파일을 둔다.
        let app = {
            let data = data.clone();
            std::thread::spawn(move || {
                let dir = data.join(REQUESTS_DIR);
                for _ in 0..200 {
                    std::thread::sleep(Duration::from_millis(20));
                    let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                    for e in entries.filter_map(Result::ok) {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let Some(stem) = name.strip_suffix(".json").filter(|s| !s.ends_with(".result")) else { continue };
                        let body: Value = serde_json::from_slice(&std::fs::read(e.path()).unwrap()).unwrap();
                        std::fs::remove_file(e.path()).unwrap();
                        assert_eq!(body["op"], "ask");
                        assert_eq!(body["fromAgent"], "a1");
                        assert_eq!(body["text"], "hello there");
                        let id = body["id"].as_str().unwrap().to_string();
                        let to = json!({ "machineId": "m2", "machineName": "pc", "agentId": "b", "agentName": "dori" });
                        std::fs::write(dir.join(format!("{stem}.result.json")), json!({ "thread": { "to": to }, "outcome": "sent" }).to_string()).unwrap();
                        std::fs::create_dir_all(data.join(REPLIES_DIR)).unwrap();
                        std::fs::write(data.join(REPLIES_DIR).join(format!("{id}.json")), json!({ "id": id, "reply": "sure" }).to_string()).unwrap();
                        return;
                    }
                }
            })
        };
        let env = PeerEnv { app_data: Some(data.clone()), agent_id: Some("a1".into()), in_session: true };
        let (code, out, err) = call_cli(&["ask", "dori", "hello", "there", "--wait", "5"], &env, "");
        app.join().unwrap();
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("from dori@pc") && out.contains("sure"), "{out}");
        assert_eq!(std::fs::read_dir(data.join(REPLIES_DIR)).unwrap().count(), 0, "가져간 답 파일은 지운다");
    }

    #[test]
    fn error_codes_get_hints() {
        assert!(explain("peer-off: pc").contains("turned off on pc"));
        assert!(explain("peer-reply-first: p-1").contains("office-peer reply p-1"));
        assert_eq!(explain("weird"), "weird");
    }

    #[test]
    fn threads_read_the_app_file() {
        let tmp = tempfile::tempdir().unwrap();
        let party = |m: &str, a: &str| crate::files::model::Party { machine_id: m.into(), machine_name: m.into(), agent_id: Some(a.into()), agent_name: Some(a.into()) };
        let t = Thread {
            id: "p-1".into(),
            from: party("mac", "hana"),
            to: party("pc", "dori"),
            text: "how is the build?".into(),
            reply: Some("green".into()),
            created_at: super::super::now_ms(),
            replied_at: None,
            depth: 1,
            outgoing: true,
            incoming: false,
        };
        std::fs::write(tmp.path().join(THREADS_FILE), json!({ "threads": [t] }).to_string()).unwrap();
        let env = PeerEnv { app_data: Some(tmp.path().into()), agent_id: None, in_session: false };
        let (code, out, _) = call_cli(&["threads"], &env, "");
        assert_eq!(code, 0);
        assert!(out.contains("hana@mac → dori@pc") && out.contains("replied") && out.contains("reply: green"), "{out}");
    }
}
