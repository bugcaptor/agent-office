//! `agent-office task` 오프라인 서브커맨드(docs/work-panels-design.md §업무 수신함).
//! GUI·tokio 없이 수신함에 파일 하나를 원자적으로 쓰거나(`--list`) 저장된 업무 목록을
//! 읽는다. control 서버와 토큰이 필요 없어 CLI 제어가 꺼져 있어도 동작한다.

use crate::persistence::work_inbox::{parse_after, valid_task_id, MAX_BYTES};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: office-task [--to <name|id> | --unassigned] [--after <id>]... <title> [body | -]\n       office-task --list [--all]\nPrints the new task id. Pass it to --after to make a later task wait for this one.\n";

/// 실행 환경. 테스트가 프로세스 env를 건드리지 않도록 값으로 받는다.
#[derive(Default)]
pub struct TaskEnv {
    pub work_inbox: Option<PathBuf>,
    pub app_data: Option<PathBuf>,
    pub agent_id: Option<String>,
}
impl TaskEnv {
    fn from_process() -> Self {
        let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
        Self {
            work_inbox: var("AGENT_OFFICE_WORK_INBOX").map(PathBuf::from),
            app_data: var("AGENT_OFFICE_APP_DATA").map(PathBuf::from),
            agent_id: var("AGENT_OFFICE_AGENT_ID").map(|v| v.to_string_lossy().into_owned()),
        }
    }
    fn inbox(&self) -> Option<PathBuf> {
        self.work_inbox.clone().or_else(|| self.app_data.as_ref().map(|dir| dir.join("work-inbox")))
    }
    /// 업무 목록(`work-items.json`)이 있는 앱 데이터 폴더. 수신함은 그 바로 아래다.
    fn data_dir(&self) -> Option<PathBuf> {
        self.app_data.clone().or_else(|| self.work_inbox.as_ref().and_then(|inbox| inbox.parent().map(Path::to_path_buf)))
    }
}

enum Assignee { Default, To(String), Unassigned }

pub fn run(args: Vec<String>) -> i32 {
    let now = chrono::Utc::now();
    run_with(&args, &TaskEnv::from_process(), &mut std::io::stdin(), &mut std::io::stdout(), &mut std::io::stderr(), now)
}

pub fn run_with(
    args: &[String],
    env: &TaskEnv,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
    now: chrono::DateTime<chrono::Utc>,
) -> i32 {
    if matches!(args.first().map(String::as_str), Some("--list")) {
        return match &args[1..] {
            [] => list(env, false, out, err),
            [all] if all == "--all" => list(env, true, out, err),
            _ => usage_error(err, "--list takes only --all"),
        };
    }
    let mut assignee = Assignee::Default;
    let mut after: Vec<String> = vec![];
    let mut positional = vec![];
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" if positional.is_empty() => { let _ = out.write_all(USAGE.as_bytes()); return 0; }
            "--to" if positional.is_empty() => match iter.next() {
                Some(value) => assignee = Assignee::To(value.clone()),
                None => return usage_error(err, "--to requires a value"),
            },
            "--unassigned" if positional.is_empty() => assignee = Assignee::Unassigned,
            "--after" if positional.is_empty() => match iter.next() {
                Some(value) => {
                    let ids = parse_after(value);
                    // 형식이 틀린 id를 조용히 버리면 의존성이 빠진 채 들어가므로 거절한다.
                    if ids.is_empty() || value.split(|c: char| c == ',' || c.is_whitespace()).any(|id| !id.is_empty() && !valid_task_id(id)) {
                        return usage_error(err, &format!("invalid task id for --after: {value:?}"));
                    }
                    for id in ids { if !after.contains(&id) { after.push(id); } }
                }
                None => return usage_error(err, "--after requires a task id"),
            },
            "--" if positional.is_empty() => positional.extend(iter.by_ref().cloned()),
            _ => positional.push(arg.clone()),
        }
    }
    if positional.is_empty() || positional.len() > 2 { return usage_error(err, "expected <title> [body | -]"); }
    let title = positional[0].split_whitespace().collect::<Vec<_>>().join(" ");
    let body = match positional.get(1).map(String::as_str) {
        Some("-") => {
            let mut bytes = vec![];
            if let Err(e) = stdin.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes) { return fail(err, &format!("failed to read stdin: {e}")); }
            match String::from_utf8(bytes) { Ok(text) => text, Err(_) => return fail(err, "stdin is not UTF-8") }
        }
        Some(body) => body.to_string(),
        None => String::new(),
    };
    let body = body.replace("\r\n", "\n");
    if title.is_empty() && body.trim().is_empty() { return fail(err, "title and body are both empty"); }
    let assignee = match assignee {
        Assignee::To(value) if value.contains(['\n', '\r']) => return usage_error(err, "--to must be a single line"),
        Assignee::To(value) => Some(value.trim().to_string()).filter(|v| !v.is_empty()),
        Assignee::Unassigned => None,
        Assignee::Default => {
            let id = env.agent_id.as_deref().map(|id| id.split_whitespace().collect::<String>()).filter(|id| !id.is_empty());
            if id.is_none() { let _ = writeln!(err, "office-task: AGENT_OFFICE_AGENT_ID is not set; leaving the task unassigned"); }
            id
        }
    };
    let random = uuid::Uuid::new_v4().simple().to_string();
    let id = format!("t-{}", &random[..8]);
    let content = render(&id, &title, body.trim_matches('\n'), assignee.as_deref(), &after);
    if content.len() > MAX_BYTES { return fail(err, &format!("task exceeds {MAX_BYTES} bytes")); }
    let Some(inbox) = env.inbox() else { return fail(err, "AGENT_OFFICE_WORK_INBOX and AGENT_OFFICE_APP_DATA are not set"); };
    let name = format!("{}-{}.md", now.format("%Y%m%dT%H%M%S%3fZ"), &random[8..16]);
    match write(&inbox, &name, content.as_bytes()) {
        // 다음 명령의 `--after "$(…)"`에 그대로 쓰도록 id만 출력한다.
        Ok(_) => { let _ = writeln!(out, "{id}"); 0 }
        Err(e) => fail(err, &format!("failed to write {}: {e}", inbox.display())),
    }
}

/// 제목 줄은 항상 `# `로 시작해 본문이나 빈 제목이 frontmatter·제목으로 잘못 읽히지 않는다.
fn render(id: &str, title: &str, body: &str, assignee: Option<&str>, after: &[String]) -> String {
    let mut text = format!("---\nid: {id}\n");
    if let Some(assignee) = assignee { text.push_str(&format!("assignee: {assignee}\n")); }
    if !after.is_empty() { text.push_str(&format!("after: {}\n", after.join(", "))); }
    text.push_str(&format!("---\n# {title}\n"));
    if !body.is_empty() { text.push_str(body); text.push('\n'); }
    text
}

const LIST_STATUS_ORDER: [&str; 4] = ["review", "in_progress", "todo", "done"];

/// 저장된 업무 목록을 한 줄씩 보여준다. 앱이 아직 가져가지 않은 수신함 파일은 개수만 알린다.
fn list(env: &TaskEnv, all: bool, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let Some(dir) = env.data_dir() else { return fail(err, "AGENT_OFFICE_WORK_INBOX and AGENT_OFFICE_APP_DATA are not set"); };
    let items: Vec<serde_json::Value> = match std::fs::read(dir.join("work-items.json")) {
        Ok(bytes) => match serde_json::from_slice(&bytes) { Ok(items) => items, Err(e) => return fail(err, &format!("failed to read work-items.json: {e}")) },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(e) => return fail(err, &format!("failed to read work-items.json: {e}")),
    };
    let names: std::collections::HashMap<String, String> = std::fs::read(dir.join("profiles.json")).ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|state| state.get("agents")?.as_array().cloned())
        .unwrap_or_default().iter()
        .filter_map(|agent| Some((agent.get("id")?.as_str()?.to_string(), agent.get("name")?.as_str()?.to_string())))
        .collect();
    let field = |item: &serde_json::Value, key: &str| item.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let status_of: std::collections::HashMap<String, String> = items.iter().map(|item| (field(item, "id"), field(item, "status"))).collect();
    let mut shown: Vec<&serde_json::Value> = items.iter().filter(|item| all || field(item, "status") != "done").collect();
    let rank = |item: &serde_json::Value| LIST_STATUS_ORDER.iter().position(|s| *s == field(item, "status")).unwrap_or(LIST_STATUS_ORDER.len());
    shown.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.get("order").and_then(|v| v.as_f64()).unwrap_or(0.0).total_cmp(&b.get("order").and_then(|v| v.as_f64()).unwrap_or(0.0))));
    for item in &shown {
        let title = match field(item, "title").trim() {
            "" => { let body = field(item, "body").split_whitespace().collect::<Vec<_>>().join(" "); body.chars().take(40).collect() }
            title => title.to_string(),
        };
        let assignee = match field(item, "assigneeAgentId") {
            id if id.is_empty() => "unassigned".to_string(),
            id => names.get(&id).cloned().unwrap_or(id),
        };
        let mut line = format!("{}  [{}]  @{}  {}", field(item, "id"), field(item, "status"), assignee, title);
        let after: Vec<String> = item.get("dependsOn").and_then(|v| v.as_array()).map(|ids| ids.iter().filter_map(|id| id.as_str().map(String::from)).collect()).unwrap_or_default();
        if !after.is_empty() {
            let waiting = after.iter().any(|id| status_of.get(id).is_some_and(|status| status != "done"));
            line.push_str(&format!("  after: {}{}", after.join(", "), if waiting { " (blocked)" } else { "" }));
        }
        let _ = writeln!(out, "{line}");
    }
    if shown.is_empty() { let _ = writeln!(out, "(no tasks)"); }
    let pending = env.inbox().and_then(|inbox| std::fs::read_dir(inbox).ok()).map_or(0, |entries| entries.flatten()
        .filter(|entry| entry.file_name().to_str().is_some_and(|name| !name.starts_with('.') && name.ends_with(".md"))).count());
    if pending > 0 { let _ = writeln!(out, "({pending} new task(s) in the inbox are not on the board yet)"); }
    0
}

/// 쓰기 규약: 같은 폴더의 `.<최종 이름>.tmp`에 끝까지 쓰고 rename한다.
fn write(inbox: &Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(inbox)?;
    let tmp = inbox.join(format!(".{name}.tmp"));
    let target = inbox.join(name);
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, &target)
    })();
    if result.is_err() { let _ = std::fs::remove_file(&tmp); }
    result.map(|()| target)
}

fn usage_error(err: &mut dyn Write, message: &str) -> i32 {
    let _ = write!(err, "office-task: {message}\n{USAGE}");
    2
}
fn fail(err: &mut dyn Write, message: &str) -> i32 {
    let _ = writeln!(err, "office-task: {message}");
    1
}

/// `office-task` shim 경로. 동료 대화 shim 옆, 공백 없는 OS temp에 둔다(권한 규칙에 박힌다).
pub fn shim_path() -> PathBuf {
    let name = if cfg!(windows) { "office-task.cmd" } else { "office-task" };
    std::env::temp_dir().join("agent-office").join("bin").join(name)
}

/// shim을 `path`에 (재)작성한다. OS temp는 청소될 수 있어 세션 준비마다 부른다.
pub fn ensure_shim(path: &Path, exe: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let exe = exe.display();
    let body = if cfg!(windows) {
        format!("@echo off\r\n\"{exe}\" task %*\r\n")
    } else {
        format!("#!/bin/sh\n# agent-office가 생성. 편집 금지.\nexec \"{exe}\" task \"$@\"\n")
    };
    crate::talk::skill::write_atomic(path, body.as_bytes())?;
    crate::talk::skill::set_executable(path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::work_inbox::{parse, scan, valid_task_id, Parsed};

    fn now() -> chrono::DateTime<chrono::Utc> { "2026-10-12T09:15:00.123Z".parse().unwrap() }
    fn call(args: &[&str], env: &TaskEnv, stdin: &str) -> (i32, String, String) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (mut out, mut err) = (vec![], vec![]);
        let code = run_with(&args, env, &mut stdin.as_bytes(), &mut out, &mut err, now());
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }
    fn env(dir: &Path, agent: Option<&str>) -> TaskEnv {
        TaskEnv { work_inbox: Some(dir.to_path_buf()), app_data: None, agent_id: agent.map(Into::into) }
    }
    fn written(dir: &Path) -> Vec<(String, Parsed)> {
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        files.sort();
        files.into_iter().map(|name| { let bytes = std::fs::read(dir.join(&name)).unwrap(); let parsed = parse(&name, &bytes).unwrap(); (name, parsed) }).collect()
    }

    #[test]
    fn assigns_to_flag_unassigned_or_session_agent() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path(), Some("agent-1"));
        assert_eq!(call(&["Mine"], &env, "").0, 0);
        assert_eq!(call(&["--to", " 하나 ", "Theirs"], &env, "").0, 0);
        assert_eq!(call(&["--unassigned", "Nobody"], &env, "").0, 0);
        let mut got: Vec<_> = written(dir.path()).into_iter().map(|(_, p)| (p.title, p.assignee)).collect();
        got.sort();
        assert_eq!(got, vec![("Mine".into(), Some("agent-1".into())), ("Nobody".into(), None), ("Theirs".into(), Some("하나".into()))]);
    }

    #[test]
    fn missing_agent_id_warns_and_writes_unassigned() {
        let dir = tempfile::tempdir().unwrap();
        let (code, out, err) = call(&["Task"], &env(dir.path(), None), "");
        assert_eq!(code, 0);
        assert!(err.contains("AGENT_OFFICE_AGENT_ID"));
        let (name, parsed) = written(dir.path()).remove(0);
        assert_eq!(Some(out.trim()), parsed.id.as_deref());
        assert!(name.starts_with("20261012T091500123Z-") && name.ends_with(".md"));
        assert_eq!(parsed.assignee, None);
    }

    #[test]
    fn inbox_falls_back_to_app_data_and_fails_without_either() {
        let dir = tempfile::tempdir().unwrap();
        let env = TaskEnv { work_inbox: None, app_data: Some(dir.path().into()), agent_id: None };
        assert_eq!(call(&["Task"], &env, "").0, 0);
        assert_eq!(written(&dir.path().join("work-inbox")).len(), 1);
        let (code, _, err) = call(&["Task"], &TaskEnv::default(), "");
        assert_eq!(code, 1);
        assert!(err.contains("AGENT_OFFICE_WORK_INBOX"));
    }

    #[test]
    fn reads_body_from_stdin_and_round_trips_through_scan() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path(), Some("a"));
        assert_eq!(call(&["Title\nwith break", "-"], &env, "\r\n---\n  indented\r\n\n").0, 0);
        assert_eq!(call(&["", "Body only"], &env, "").0, 0);
        let entries = scan(dir.path(), std::time::SystemTime::now()).unwrap();
        let mut got: Vec<_> = entries.iter().map(|e| (e.title.as_str(), e.body.as_str())).collect();
        got.sort();
        assert_eq!(got, vec![("", "Body only"), ("Title with break", "---\n  indented")]);
        // 임시 파일이 남지 않고 거부된 파일도 없다.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn rejects_empty_oversized_and_bad_arguments_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path(), Some("a"));
        assert_eq!(call(&[" ", "\n"], &env, "").0, 1);
        assert_eq!(call(&["Big", "-"], &env, &"a".repeat(MAX_BYTES)).0, 1);
        assert_eq!(call(&["--to", "a\nb", "Task"], &env, "").0, 2);
        assert_eq!(call(&["--to"], &env, "").0, 2);
        assert_eq!(call(&[], &env, "").0, 2);
        assert_eq!(call(&["a", "b", "c"], &env, "").0, 2);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        assert_eq!(call(&["--help"], &env, "").0, 0);
    }

    #[test]
    fn flags_after_title_are_text_and_double_dash_ends_flags() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path(), Some("a"));
        assert_eq!(call(&["--", "--to", "--unassigned"], &env, "").0, 0);
        let (_, parsed) = written(dir.path()).remove(0);
        assert_eq!((parsed.title.as_str(), parsed.body.as_str(), parsed.assignee.as_deref()), ("--to", "--unassigned", Some("a")));
    }

    #[test]
    fn after_links_prerequisites_by_the_printed_id() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path(), Some("a"));
        let (code, first, _) = call(&["Step 1"], &env, "");
        assert_eq!(code, 0);
        let first = first.trim().to_string();
        assert!(first.starts_with("t-") && valid_task_id(&first));
        let (code, second, _) = call(&["--after", &first, "--after", &format!("w-x,{first}"), "Step 2"], &env, "");
        assert_eq!(code, 0);
        let entries = scan(dir.path(), std::time::SystemTime::now()).unwrap();
        let step2 = entries.iter().find(|e| e.title == "Step 2").unwrap();
        assert_eq!(step2.id.as_deref(), Some(second.trim()));
        assert_eq!(step2.after, vec![first.clone(), "w-x".to_string()]);
        assert_eq!(entries.iter().find(|e| e.title == "Step 1").unwrap().id.as_deref(), Some(first.as_str()));
        assert_eq!(call(&["--after", "bad id!", "Step 3"], &env, "").0, 2);
        assert_eq!(call(&["--after"], &env, "").0, 2);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn list_shows_open_tasks_with_names_and_blocked_prerequisites() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("profiles.json"), r#"{"agents":[{"id":"a1","name":"하나"}]}"#).unwrap();
        std::fs::write(dir.path().join("work-items.json"), r#"[
            {"id":"w1","title":"Design","body":"","status":"todo","order":0,"assigneeAgentId":"a1"},
            {"id":"w2","title":"","body":"Build   the\nthing","status":"todo","order":1,"dependsOn":["w1"]},
            {"id":"w0","title":"Old","body":"","status":"done","order":0},
            {"id":"w3","title":"Ship","body":"","status":"review","order":0,"dependsOn":["w0"]}
        ]"#).unwrap();
        let env = TaskEnv { work_inbox: Some(dir.path().join("work-inbox")), app_data: None, agent_id: None };
        assert_eq!(call(&["Pending"], &env, "").0, 0);
        let (code, out, _) = call(&["--list"], &env, "");
        assert_eq!(code, 0);
        assert_eq!(out, "w3  [review]  @unassigned  Ship  after: w0\nw1  [todo]  @하나  Design\nw2  [todo]  @unassigned  Build the thing  after: w1 (blocked)\n(1 new task(s) in the inbox are not on the board yet)\n");
        assert!(call(&["--list", "--all"], &env, "").1.contains("w0  [done]"));
        assert_eq!(call(&["--list", "x"], &env, "").0, 2);
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(call(&["--list"], &TaskEnv { work_inbox: None, app_data: Some(empty.path().into()), agent_id: None }, "").1, "(no tasks)\n");
    }
}
