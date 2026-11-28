// src-tauri/src/files/cli.rs
//
// `agent-office file` 오프라인 서브커맨드(`office-file` shim)와 앱 쪽 요청 감시.
// CLI는 앱 데이터의 `file-requests/<id>.json`에 요청을 쓰고 앱이 0.5초마다 가져가 `<id>.result.json`으로
// 답한다. 목록·상태·대상은 앱이 쓰는 `file-transfers.json`·`file-targets.json`을 읽는다.
// 종료 코드: 0 성공, 1 실패, 2 사용법 오류.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::model::{Target, Transfer, TransferState};
use super::{human_bytes, FilesState, SendRequest, REQUESTS_DIR, TARGETS_FILE, TRANSFERS_FILE};

const USAGE: &str = "usage: office-file targets [--json]
       office-file send <target> <path>... [--note <text>] [--notify] [--wait] [--json]
       office-file list [--sent | --received] [--json]
       office-file status <id> [--json]
       office-file resume <id> [--wait]
       office-file cancel <id>
Sends files or folders to a linked Agent Office machine or a character (no pulling).
<target> is a machine name, a character name, or <character>@<machine> (see `targets`).
The receiver saves them under its received-files folder; --notify (or --note) also tells the
receiving character's session where they were saved. --wait blocks until the transfer ends.
";

const EXIT_FAIL: i32 = 1;
const EXIT_USAGE: i32 = 2;
/// 앱 감시는 0.5초 간격이다.
const APP_WAIT: Duration = Duration::from_secs(6);
const POLL: Duration = Duration::from_millis(250);

pub struct FileEnv {
    pub app_data: Option<PathBuf>,
    pub cwd: PathBuf,
    pub agent_id: Option<String>,
    pub in_session: bool,
}

impl FileEnv {
    fn from_process() -> Self {
        let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
        Self {
            app_data: var("AGENT_OFFICE_APP_DATA").map(PathBuf::from).or_else(crate::control::client::default_app_data),
            cwd: std::env::current_dir().unwrap_or_default(),
            agent_id: var("AGENT_OFFICE_AGENT_ID").map(|v| v.to_string_lossy().into_owned()),
            in_session: var("AGENT_OFFICE_SESSION").is_some(),
        }
    }
}

pub fn run(args: Vec<String>) -> i32 {
    run_with(&args, &FileEnv::from_process(), &mut std::io::stdout(), &mut std::io::stderr(), APP_WAIT)
}

/// 세션에 `office-file` 안내를 실을까: 머신 연동이 켜져 있거나 파일 받기를 켰을 때.
pub fn instructions_wanted(data: &Path) -> bool {
    let linked = std::fs::read(data.join(crate::link::CONFIG_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<crate::link::LinkConfig>(&b).ok())
        .is_some_and(|c| c.mode != crate::link::LinkMode::Off);
    let receive = std::fs::read(data.join(super::CONFIG_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<super::FilesConfig>(&b).ok())
        .is_some_and(|c| c.receive_enabled);
    linked || receive
}

pub fn shim_path() -> PathBuf {
    crate::work_inbox_cli::office_shim_path("office-file")
}

struct Opts {
    json: bool,
    wait: bool,
    notify: bool,
    note: Option<String>,
    sent: bool,
    received: bool,
    positional: Vec<String>,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { json: false, wait: false, notify: false, note: None, sent: false, received: false, positional: vec![] };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => o.json = true,
            "--wait" => o.wait = true,
            "--notify" => o.notify = true,
            "--sent" => o.sent = true,
            "--received" => o.received = true,
            "--note" => o.note = Some(it.next().ok_or("--note needs a value")?.clone()),
            "--" => o.positional.extend(it.by_ref().cloned()),
            f if f.starts_with("--") => return Err(format!("unknown option {f}")),
            _ => o.positional.push(a.clone()),
        }
    }
    Ok(o)
}

pub fn run_with(args: &[String], env: &FileEnv, out: &mut dyn Write, err: &mut dyn Write, app_wait: Duration) -> i32 {
    let Some(cmd) = args.first().map(String::as_str) else {
        let _ = write!(err, "{USAGE}");
        return EXIT_USAGE;
    };
    if matches!(cmd, "-h" | "--help" | "help") || args.iter().any(|a| a == "--help" || a == "-h") {
        let _ = write!(out, "{USAGE}");
        return 0;
    }
    let opts = match parse(&args[1..]) {
        Ok(o) => o,
        Err(e) => return usage(err, &e),
    };
    let Some(data) = env.app_data.as_deref() else {
        return fail(err, "Agent Office data folder not found (AGENT_OFFICE_APP_DATA is not set)");
    };
    match cmd {
        "targets" => targets(data, &opts, out, err),
        "send" => send(data, env, &opts, out, err, app_wait),
        "list" => list(data, &opts, out),
        "status" => match opts.positional.as_slice() {
            [id] => status(data, id, &opts, out, err),
            _ => usage(err, "status takes one transfer id"),
        },
        "resume" | "cancel" => match opts.positional.as_slice() {
            [id] => control(data, cmd, id, &opts, out, err, app_wait),
            _ => usage(err, &format!("{cmd} takes one transfer id")),
        },
        other => usage(err, &format!("unknown command {other:?}")),
    }
}

fn usage(err: &mut dyn Write, msg: &str) -> i32 {
    let _ = writeln!(err, "office-file: {msg}\n{USAGE}");
    EXIT_USAGE
}

fn fail(err: &mut dyn Write, msg: &str) -> i32 {
    let _ = writeln!(err, "office-file: {msg}");
    EXIT_FAIL
}

fn read_targets(data: &Path) -> Vec<Target> {
    std::fs::read(data.join(TARGETS_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| serde_json::from_value(v["targets"].clone()).ok())
        .unwrap_or_default()
}

fn read_transfers(data: &Path) -> Vec<Transfer> {
    std::fs::read(data.join(TRANSFERS_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| serde_json::from_value::<Vec<Transfer>>(v["transfers"].clone()).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.public())
        .collect()
}

fn machine_id(data: &Path) -> String {
    std::fs::read_to_string(data.join(crate::link::MACHINE_ID_FILE)).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn targets(data: &Path, o: &Opts, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let list = read_targets(data);
    if o.json {
        let _ = writeln!(out, "{}", serde_json::to_string_pretty(&list).unwrap_or_default());
        return 0;
    }
    if list.is_empty() {
        return fail(err, "no targets yet (is Agent Office running?)");
    }
    for t in &list {
        let state = match (t.online, t.receive) {
            (false, _) => "offline",
            (true, false) => "receiving off",
            (true, true) => "ready",
        };
        let kind = if t.agent_id.is_some() { "character" } else { "machine" };
        let here = if t.local { " (this machine)" } else { "" };
        let _ = writeln!(out, "{}\t{kind}{here}\t{state}", t.spec());
    }
    0
}

fn line(t: &Transfer, me: &str) -> String {
    let dir = if t.to.machine_id == me && t.from.machine_id != me {
        format!("from {}", t.from.label())
    } else {
        format!("to {}", t.to.label())
    };
    let state = match t.state {
        TransferState::Preparing => "preparing".to_string(),
        TransferState::Sending => format!("sending {}%", if t.total_bytes == 0 { 0 } else { t.done_bytes * 100 / t.total_bytes }),
        TransferState::Done => "done".into(),
        TransferState::Failed => format!("failed: {}", t.error.as_deref().unwrap_or("")),
        TransferState::Cancelled => "cancelled".into(),
    };
    format!("{}\t{dir}\t{}\t{} files, {}\t{state}", t.id, t.items.join(", "), t.files.len(), human_bytes(t.total_bytes))
}

fn list(data: &Path, o: &Opts, out: &mut dyn Write) -> i32 {
    let me = machine_id(data);
    let mut list: Vec<Transfer> = read_transfers(data)
        .into_iter()
        .filter(|t| {
            let received = t.to.machine_id == me;
            let sent = t.from.machine_id == me;
            (!o.sent || sent) && (!o.received || received)
        })
        .collect();
    list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    if o.json {
        let _ = writeln!(out, "{}", serde_json::to_string_pretty(&list).unwrap_or_default());
    } else {
        for t in &list {
            let _ = writeln!(out, "{}", line(t, &me));
        }
    }
    0
}

fn print_transfer(t: &Transfer, me: &str, o: &Opts, out: &mut dyn Write) {
    if o.json {
        let _ = writeln!(out, "{}", serde_json::to_string_pretty(t).unwrap_or_default());
        return;
    }
    let _ = writeln!(out, "{}", line(t, me));
    for p in &t.saved {
        let _ = writeln!(out, "saved: {p}");
    }
}

fn status(data: &Path, id: &str, o: &Opts, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match read_transfers(data).into_iter().find(|t| t.id == id) {
        Some(t) => {
            print_transfer(&t, &machine_id(data), o, out);
            0
        }
        None => fail(err, &format!("unknown transfer {id}")),
    }
}

/// 요청을 쓰고 앱의 답을 기다린다.
fn request(data: &Path, id: &str, body: Value, wait: Duration) -> Result<Value, String> {
    let dir = data.join(REQUESTS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let req = dir.join(format!("{id}.json"));
    let result = dir.join(format!("{id}.result.json"));
    crate::persistence::durable::write_atomic(&req, body.to_string().as_bytes()).map_err(|e| e.to_string())?;
    let start = Instant::now();
    while start.elapsed() < wait {
        if let Ok(bytes) = std::fs::read(&result) {
            let _ = std::fs::remove_file(&result);
            return serde_json::from_slice(&bytes).map_err(|e| e.to_string());
        }
        std::thread::sleep(POLL);
    }
    let _ = std::fs::remove_file(&req);
    // 지우는 사이에 앱이 가져갔을 수 있다.
    if let Ok(bytes) = std::fs::read(&result) {
        let _ = std::fs::remove_file(&result);
        return serde_json::from_slice(&bytes).map_err(|e| e.to_string());
    }
    Err("Agent Office did not pick up the request (is the app running?)".into())
}

fn wait_done(data: &Path, id: &str, out: &mut dyn Write, o: &Opts) -> i32 {
    let me = machine_id(data);
    loop {
        let found = read_transfers(data).into_iter().find(|t| t.id == id);
        if let Some(t) = found.filter(|t| matches!(t.state, TransferState::Done | TransferState::Cancelled | TransferState::Failed)) {
            print_transfer(&t, &me, o, out);
            return if t.state == TransferState::Done { 0 } else { EXIT_FAIL };
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn send(data: &Path, env: &FileEnv, o: &Opts, out: &mut dyn Write, err: &mut dyn Write, wait: Duration) -> i32 {
    let [target, paths @ ..] = o.positional.as_slice() else {
        return usage(err, "send needs a target and at least one path");
    };
    if paths.is_empty() {
        return usage(err, "send needs at least one path");
    }
    let mut abs = Vec::new();
    for p in paths {
        let path = env.cwd.join(p);
        if std::fs::symlink_metadata(&path).is_err() {
            return fail(err, &format!("not found: {}", path.display()));
        }
        // `..`·`.`를 정리한다(링크는 그대로 둔다).
        let clean = std::path::absolute(&path).unwrap_or(path);
        abs.push(clean.to_string_lossy().into_owned());
    }
    let id = format!("f-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let body = json!({
        "op": "send",
        "id": id,
        "target": target,
        "paths": abs,
        "note": o.note,
        "notify": o.notify || o.note.is_some(),
        "fromAgent": env.agent_id,
        "via": if env.in_session { "agent" } else { "cli" },
    });
    let answer = match request(data, &id, body, wait) {
        Ok(a) => a,
        Err(e) => return fail(err, &e),
    };
    if let Some(e) = answer.get("error").and_then(Value::as_str) {
        return fail(err, e);
    }
    if o.wait {
        return wait_done(data, &id, out, o);
    }
    if o.json {
        let _ = writeln!(out, "{}", serde_json::to_string_pretty(&answer["transfer"]).unwrap_or_default());
    } else {
        let to = answer["transfer"]["to"].clone();
        let label = serde_json::from_value::<super::model::Party>(to).map(|p| p.label()).unwrap_or_default();
        let _ = writeln!(out, "{id}\tsending to {label} (check with `office-file status {id}`)");
    }
    0
}

fn control(data: &Path, op: &str, id: &str, o: &Opts, out: &mut dyn Write, err: &mut dyn Write, wait: Duration) -> i32 {
    let rid = format!("r-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let answer = match request(data, &rid, json!({ "op": op, "id": id }), wait) {
        Ok(a) => a,
        Err(e) => return fail(err, &e),
    };
    if let Some(e) = answer.get("error").and_then(Value::as_str) {
        return fail(err, e);
    }
    if op == "resume" && o.wait {
        return wait_done(data, id, out, o);
    }
    let _ = writeln!(out, "{id}\t{op} requested");
    0
}

// ── 앱 쪽: 요청 감시 ──────────────────────────────────────────────

/// 요청 하나를 처리한다(앱).
pub fn handle_request(files: &Arc<FilesState>, body: &Value) -> Value {
    let op = body.get("op").and_then(Value::as_str).unwrap_or("");
    let id = body.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let result = match op {
        "send" => {
            let req = SendRequest {
                target: body.get("target").and_then(Value::as_str).unwrap_or("").to_string(),
                paths: serde_json::from_value(body.get("paths").cloned().unwrap_or_default()).unwrap_or_default(),
                note: body.get("note").and_then(Value::as_str).map(str::to_string),
                notify: body.get("notify").and_then(Value::as_bool).unwrap_or(false),
                from_agent: body.get("fromAgent").and_then(Value::as_str).map(str::to_string),
                via: body.get("via").and_then(Value::as_str).unwrap_or("cli").to_string(),
                id: Some(id),
                ..Default::default()
            };
            files.send(req).map(|t| json!({ "transfer": t }))
        }
        "cancel" => files.cancel(&id).map(|_| json!({})),
        "resume" => files.resume(&id).map(|_| json!({})),
        _ => Err("bad-request".into()),
    };
    match result {
        Ok(v) => v,
        Err(e) => json!({ "error": e }),
    }
}

/// `file-requests/`를 0.5초마다 본다(앱 수명 동안 한 번).
pub fn spawn_request_watcher(files: Arc<FilesState>) {
    let Some(dir) = files.data_dir().map(|d| d.join(REQUESTS_DIR)) else { return };
    let _ = std::fs::create_dir_all(&dir);
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.filter_map(Result::ok) {
                let path = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if name.ends_with(".result.json") {
                    // CLI가 안 가져간 오래된 답.
                    let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|d| d > Duration::from_secs(600));
                    if old {
                        let _ = std::fs::remove_file(&path);
                    }
                    continue;
                }
                let Some(stem) = name.strip_suffix(".json").filter(|s| !s.contains(".tmp")) else { continue };
                let Ok(bytes) = std::fs::read(&path) else { continue };
                let _ = std::fs::remove_file(&path);
                let answer = match serde_json::from_slice::<Value>(&bytes) {
                    Ok(body) => handle_request(&files, &body),
                    Err(_) => json!({ "error": "bad-request" }),
                };
                let _ = crate::persistence::durable::write_atomic(&dir.join(format!("{stem}.result.json")), answer.to_string().as_bytes());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(args: &[&str], env: &FileEnv) -> (i32, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let code = run_with(&args, env, &mut out, &mut err, Duration::from_millis(300));
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }

    #[test]
    fn send_writes_a_request_and_gives_up_without_the_app() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), b"x").unwrap();
        let env = FileEnv { app_data: Some(tmp.path().join("data")), cwd: tmp.path().into(), agent_id: Some("a1".into()), in_session: true };
        let (code, _, err) = call(&["send", "pc", "a.txt"], &env);
        assert_eq!(code, 1);
        assert!(err.contains("did not pick up"));
        // 앱이 안 가져간 요청은 지운다.
        assert_eq!(std::fs::read_dir(tmp.path().join("data").join(REQUESTS_DIR)).unwrap().count(), 0);
        let (code, _, err) = call(&["send", "pc", "missing.txt"], &env);
        assert_eq!(code, 1);
        assert!(err.contains("not found"));
        assert_eq!(call(&["send", "pc"], &env).0, 2);
        assert_eq!(call(&["bogus"], &env).0, 2);
    }

    #[test]
    fn list_and_targets_read_app_files() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path();
        std::fs::write(data.join(crate::link::MACHINE_ID_FILE), "m9").unwrap();
        let mut t = crate::files::receive::tests_support::transfer();
        t.state = TransferState::Done;
        t.saved = vec!["/r/a.txt".into()];
        t.to = super::super::Party { machine_id: "m9".into(), machine_name: "pc".into(), ..Default::default() };
        std::fs::write(data.join(TRANSFERS_FILE), json!({ "transfers": [t] }).to_string()).unwrap();
        let target = Target { machine_id: "m2".into(), machine_name: "mac".into(), agent_id: None, agent_name: None, device_id: None, local: false, online: true, receive: false };
        std::fs::write(data.join(TARGETS_FILE), json!({ "targets": [target] }).to_string()).unwrap();
        let env = FileEnv { app_data: Some(data.into()), cwd: data.into(), agent_id: None, in_session: false };
        let (code, out, _) = call(&["list", "--received"], &env);
        assert_eq!(code, 0);
        assert!(out.contains("from mac") && out.contains("done"), "{out}");
        assert_eq!(call(&["list", "--sent"], &env).1, "");
        let (_, out, _) = call(&["status", "t"], &env);
        assert!(out.contains("saved: /r/a.txt"));
        let (_, out, _) = call(&["targets"], &env);
        assert_eq!(out, "mac\tmachine\treceiving off\n");
    }
}
