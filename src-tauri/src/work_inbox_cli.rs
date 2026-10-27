//! `agent-office task` 오프라인 서브커맨드.
//! GUI·tokio 없이 수신함에 파일 하나를 원자적으로 쓰거나(`--list`) 저장된 업무 목록을
//! 읽는다. control 서버와 토큰이 필요 없어 CLI 제어가 꺼져 있어도 동작한다.

use crate::persistence::work_inbox::{forbidden_control, parse_after, valid_task_id, MAX_BYTES, STATUSES};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: office-task [--to <name|id> | --unassigned] [--after <id>]... <title> [body | -]
       office-task --list [--all]
       office-task --show <id>
       office-task --edit <id> [--title <title>] [--body <body | ->] [--to <name|id> | --unassigned]
                   [--status todo|in_progress|review|done]
                   [--after <id>... | --no-after | --add-after <id>... --remove-after <id>...]
       office-task --done <id>...
       office-task --delete <id>...
       office-task --report <id> [text | -]
Prints the new task id. Pass it to --after to make a later task wait for this one.
--edit --after replaces all prerequisites; --add-after/--remove-after change them one by one.
--report leaves the auto-run report for a task; writing it again replaces it.
";

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
    match args.first().map(String::as_str) {
        Some("--done") => return done(&args[1..], env, out, err, now),
        Some("--delete") => return delete(&args[1..], env, out, err, now),
        Some("--edit") => return edit(&args[1..], env, stdin, out, err, now),
        Some("--report") => return report(&args[1..], env, stdin, err),
        Some("--show") => return match &args[1..] {
            [id] => show(id, env, out, err),
            _ => usage_error(err, "--show takes one task id"),
        },
        _ => {}
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
            "--after" if positional.is_empty() => if let Err(code) = push_ids(&mut after, iter.next(), "--after", err) { return code; },
            "--" if positional.is_empty() => positional.extend(iter.by_ref().cloned()),
            _ => positional.push(arg.clone()),
        }
    }
    if positional.is_empty() || positional.len() > 2 { return usage_error(err, "expected <title> [body | -]"); }
    let title = positional[0].split_whitespace().collect::<Vec<_>>().join(" ");
    let body = match positional.get(1) {
        Some(value) => match read_body(value, stdin, err) { Ok(body) => body, Err(code) => return code },
        None => String::new(),
    };
    if title.is_empty() && body.trim().is_empty() { return fail(err, "title and body are both empty"); }
    // 앱 수신함이 거절할 파일을 쓰지 않는다.
    if let Some(c) = forbidden_control(&title).or_else(|| forbidden_control(&body)) {
        return fail(err, &format!("title or body contains control character U+{:04X}; only tab and newline are allowed", c as u32));
    }
    let explicit = matches!(assignee, Assignee::To(_));
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
    warn_unknown_references(env, assignee.as_deref().filter(|_| explicit), &after, err);
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

/// `--after` 같은 id 목록 인자 하나를 `ids`에 더한다(쉼표·공백 구분, 중복 합침).
/// 형식이 틀린 id를 조용히 버리면 의존성이 빠진 채 들어가므로 거절한다.
fn push_ids(ids: &mut Vec<String>, value: Option<&String>, flag: &str, err: &mut dyn Write) -> Result<(), i32> {
    let Some(value) = value else { return Err(usage_error(err, &format!("{flag} requires a task id"))) };
    let parsed = parse_after(value);
    if parsed.is_empty() || value.split(|c: char| c == ',' || c.is_whitespace()).any(|id| !id.is_empty() && !valid_task_id(id)) {
        return Err(usage_error(err, &format!("invalid task id for {flag}: {value:?}")));
    }
    for id in parsed { if !ids.contains(&id) { ids.push(id); } }
    Ok(())
}

/// 본문 인자. `-`이면 stdin을 읽는다. CRLF는 LF로 바꾼다.
fn read_body(value: &str, stdin: &mut dyn Read, err: &mut dyn Write) -> Result<String, i32> {
    let body = if value == "-" {
        let mut bytes = vec![];
        if let Err(e) = stdin.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes) { return Err(fail(err, &format!("failed to read stdin: {e}"))); }
        String::from_utf8(bytes).map_err(|_| fail(err, "stdin is not UTF-8"))?
    } else { value.to_string() };
    Ok(body.replace("\r\n", "\n"))
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

/// `profiles.json`의 (id, 이름). 없거나 읽을 수 없으면 None.
fn load_profiles(dir: &Path) -> Option<Vec<(String, String)>> {
    let state: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("profiles.json")).ok()?).ok()?;
    Some(state.get("agents")?.as_array()?.iter()
        .filter_map(|agent| Some((agent.get("id")?.as_str()?.to_string(), agent.get("name")?.as_str()?.to_string())))
        .collect())
}

/// 앱이 아직 가져가지 않은 수신함 파일의 업무 id.
fn pending_ids(inbox: &Path) -> Vec<String> {
    std::fs::read_dir(inbox).map(|entries| entries.flatten()
        .filter(|entry| entry.file_name().to_str().is_some_and(|name| !name.starts_with('.') && name.ends_with(".md")))
        .filter_map(|entry| crate::persistence::work_inbox::parse("", &std::fs::read(entry.path()).ok()?).ok()?.id)
        .collect()).unwrap_or_default()
}

/// 앱은 모르는 담당을 미배정으로, 없는 선행을 빼고 받는다. 에이전트가 바로 고칠 수 있게
/// 쓰기 전에 stderr로 알린다. 업무는 그대로 쓴다(rc 0). 앱 데이터를 읽지 못하면 확인하지 않는다.
fn warn_unknown_references(env: &TaskEnv, to: Option<&str>, after: &[String], err: &mut dyn Write) {
    let Some(dir) = env.data_dir() else { return };
    if let (Some(to), Some(profiles)) = (to, load_profiles(&dir)) {
        let by_name = profiles.iter().filter(|(_, name)| name.trim() == to).count();
        if !profiles.iter().any(|(id, _)| id == to) && by_name != 1 {
            let why = if by_name == 0 { "no character has this name or id" } else { "several characters share this name; use the id" };
            let _ = writeln!(err, "office-task: warning: --to {to:?}: {why}; the task will be unassigned");
        }
    }
    if after.is_empty() { return; }
    let Ok(items) = load_items(&dir) else { return };
    let pending = env.inbox().map(|inbox| pending_ids(&inbox)).unwrap_or_default();
    let unknown: Vec<&str> = after.iter().map(String::as_str)
        .filter(|id| !items.iter().any(|item| item.get("id").and_then(|v| v.as_str()) == Some(id)) && !pending.iter().any(|p| p == id))
        .collect();
    if !unknown.is_empty() {
        let _ = writeln!(err, "office-task: warning: --after {}: no such task (see office-task --list); the app will drop it", unknown.join(", "));
    }
}

fn load_items(dir: &Path) -> Result<Vec<serde_json::Value>, String> {
    match std::fs::read(dir.join("work-items.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("failed to read work-items.json: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(format!("failed to read work-items.json: {e}")),
    }
}

/// 저장된 업무 목록과 앱이 아직 가져가지 않은 수신함 업무 id. 대상 id를 확인할 때 쓴다.
struct Board { inbox: PathBuf, items: Vec<serde_json::Value>, pending: Vec<String> }
impl Board {
    fn load(env: &TaskEnv, err: &mut dyn Write) -> Result<Self, i32> {
        let (Some(dir), Some(inbox)) = (env.data_dir(), env.inbox()) else { return Err(fail(err, "AGENT_OFFICE_WORK_INBOX and AGENT_OFFICE_APP_DATA are not set")); };
        let items = load_items(&dir).map_err(|e| fail(err, &e))?;
        let pending = pending_ids(&inbox);
        Ok(Self { inbox, items, pending })
    }
    fn item(&self, id: &str) -> Option<&serde_json::Value> {
        self.items.iter().find(|item| item.get("id").and_then(|v| v.as_str()) == Some(id))
    }
    fn status(&self, id: &str) -> Option<String> {
        self.item(id).map(|item| item.get("status").and_then(|v| v.as_str()).unwrap_or("").to_string())
    }
    /// 보드에도 수신함에도 없는 id. 방금 남겨 아직 앱이 가져가지 않은 업무도 대상으로 삼을 수 있다.
    /// 앱은 같은 주기에서 추가를 먼저 반영한다.
    fn unknown<'a>(&self, ids: &'a [String]) -> Vec<&'a str> {
        ids.iter().map(String::as_str).filter(|id| self.item(id).is_none() && !self.pending.iter().any(|p| p == id)).collect()
    }
    /// 이 업무를 선행으로 기다리는 보드의 업무 id.
    fn dependents(&self, id: &str) -> Vec<String> {
        self.items.iter()
            .filter(|item| item.get("dependsOn").and_then(|v| v.as_array()).is_some_and(|ids| ids.iter().any(|d| d.as_str() == Some(id))))
            .filter_map(|item| item.get("id").and_then(|v| v.as_str()).map(String::from))
            .collect()
    }
}

/// 요청 파일 하나를 수신함에 쓴다. 이름은 시각 순이라 앞서 쓴 요청이 먼저 적용된다.
fn write_request(board: &Board, content: &str, now: chrono::DateTime<chrono::Utc>, err: &mut dyn Write) -> Result<(), i32> {
    if content.len() > MAX_BYTES { return Err(fail(err, &format!("request exceeds {MAX_BYTES} bytes"))); }
    let random = uuid::Uuid::new_v4().simple().to_string();
    let name = format!("{}-{}.md", now.format("%Y%m%dT%H%M%S%3fZ"), &random[..8]);
    write(&board.inbox, &name, content.as_bytes()).map(|_| ()).map_err(|e| fail(err, &format!("failed to write {}: {e}", board.inbox.display())))
}

/// 자동 진행 보고. 수신함 요청이 아니라 `reports/<id>.md`
/// 하나를 덮어쓴다. 판단자가 그 턴에 쓰인 것만 읽는다.
fn report(args: &[String], env: &TaskEnv, stdin: &mut dyn Read, err: &mut dyn Write) -> i32 {
    let (id, text) = match args {
        [id] => (id, "-"),
        [id, text] => (id, text.as_str()),
        _ => return usage_error(err, "--report takes a task id and [text | -]"),
    };
    if !valid_task_id(id) { return usage_error(err, &format!("invalid task id for --report: {id:?}")); }
    let body = match read_body(text, stdin, err) { Ok(body) => body, Err(code) => return code };
    let body = body.trim_matches('\n');
    if body.trim().is_empty() { return fail(err, "the report is empty"); }
    if body.len() > MAX_BYTES { return fail(err, &format!("report exceeds {MAX_BYTES} bytes")); }
    if let Some(c) = forbidden_control(body) {
        return fail(err, &format!("report contains control character U+{:04X}; only tab and newline are allowed", c as u32));
    }
    let Some(inbox) = env.inbox() else { return fail(err, "AGENT_OFFICE_WORK_INBOX and AGENT_OFFICE_APP_DATA are not set"); };
    let dir = crate::auto_run::reports_dir(&inbox);
    match write(&dir, &format!("{id}.md"), format!("{body}\n").as_bytes()) {
        Ok(_) => 0,
        Err(e) => fail(err, &format!("failed to write {}: {e}", dir.display())),
    }
}

fn target_ids(args: &[String], flag: &str, err: &mut dyn Write) -> Result<Vec<String>, i32> {
    if args.is_empty() { return Err(usage_error(err, &format!("{flag} requires a task id"))); }
    let mut ids = vec![];
    for value in args { push_ids(&mut ids, Some(value), flag, err)?; }
    Ok(ids)
}

/// 완료 요청 파일을 쓴다. 보드나 수신함에 없는 id면 아무것도 쓰지 않고 1로 끝나
/// 에이전트가 잘못된 id를 바로 알 수 있게 한다. 이미 완료된 업무는 건너뛴다.
fn done(args: &[String], env: &TaskEnv, out: &mut dyn Write, err: &mut dyn Write, now: chrono::DateTime<chrono::Utc>) -> i32 {
    let requested = match target_ids(args, "--done", err) { Ok(ids) => ids, Err(code) => return code };
    let board = match Board::load(env, err) { Ok(board) => board, Err(code) => return code };
    let unknown = board.unknown(&requested);
    if !unknown.is_empty() { return fail(err, &format!("no such task: {} (see office-task --list)", unknown.join(", "))); }
    let (already, targets): (Vec<String>, Vec<String>) = requested.into_iter().partition(|id| board.status(id).as_deref() == Some("done"));
    for id in &already { let _ = writeln!(err, "office-task: {id} is already done"); }
    if targets.is_empty() { return 0; }
    match write_request(&board, &format!("---\ndone: {}\n---\n", targets.join(", ")), now, err) {
        Ok(()) => { for id in &targets { let _ = writeln!(out, "{id}"); } 0 }
        Err(code) => code,
    }
}

/// 삭제 요청 파일을 쓴다. 없는 id가 하나라도 있으면 아무것도 쓰지 않고 1이다.
/// 지운 업무를 기다리던 후속 업무는 그 선행만 풀리므로 stderr로 알린다.
fn delete(args: &[String], env: &TaskEnv, out: &mut dyn Write, err: &mut dyn Write, now: chrono::DateTime<chrono::Utc>) -> i32 {
    let targets = match target_ids(args, "--delete", err) { Ok(ids) => ids, Err(code) => return code };
    let board = match Board::load(env, err) { Ok(board) => board, Err(code) => return code };
    let unknown = board.unknown(&targets);
    if !unknown.is_empty() { return fail(err, &format!("no such task: {} (see office-task --list)", unknown.join(", "))); }
    if let Err(code) = write_request(&board, &format!("---\ndelete: {}\n---\n", targets.join(", ")), now, err) { return code; }
    for id in &targets {
        let waiting: Vec<String> = board.dependents(id).into_iter().filter(|d| !targets.contains(d)).collect();
        if !waiting.is_empty() { let _ = writeln!(err, "office-task: {} no longer wait(s) on {id}", waiting.join(", ")); }
        let _ = writeln!(out, "{id}");
    }
    0
}

/// 수정 요청 파일을 쓴다. 준 필드만 바꾼다. 대상이 없으면 1, 인자 오류는 2다.
fn edit(args: &[String], env: &TaskEnv, stdin: &mut dyn Read, out: &mut dyn Write, err: &mut dyn Write, now: chrono::DateTime<chrono::Utc>) -> i32 {
    let Some((id, args)) = args.split_first() else { return usage_error(err, "--edit requires a task id") };
    if !valid_task_id(id) { return usage_error(err, &format!("invalid task id for --edit: {id:?}")); }
    let mut title: Option<String> = None;
    let mut body: Option<String> = None;
    let mut assignee: Option<Option<String>> = None;
    let mut status: Option<String> = None;
    let (mut after, mut no_after) = (vec![], false);
    let (mut add, mut remove) = (vec![], vec![]);
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let result = match arg.as_str() {
            "--title" => match iter.next() {
                Some(value) => { title = Some(value.split_whitespace().collect::<Vec<_>>().join(" ")); Ok(()) }
                None => Err(usage_error(err, "--title requires a value")),
            },
            "--body" => match iter.next() {
                Some(value) => read_body(value, stdin, err).map(|text| body = Some(text.trim_matches('\n').to_string())),
                None => Err(usage_error(err, "--body requires a value or -")),
            },
            "--to" => match iter.next() {
                Some(value) if value.contains(['\n', '\r']) => Err(usage_error(err, "--to must be a single line")),
                Some(value) if value.trim().is_empty() => Err(usage_error(err, "--to requires a value; use --unassigned to clear")),
                Some(value) => { assignee = Some(Some(value.trim().to_string())); Ok(()) }
                None => Err(usage_error(err, "--to requires a value")),
            },
            "--unassigned" => { assignee = Some(None); Ok(()) }
            "--status" => match iter.next() {
                Some(value) if STATUSES.contains(&value.as_str()) => { status = Some(value.clone()); Ok(()) }
                Some(value) => Err(usage_error(err, &format!("invalid --status {value:?}; use one of {}", STATUSES.join(", ")))),
                None => Err(usage_error(err, "--status requires a value")),
            },
            "--after" => push_ids(&mut after, iter.next(), "--after", err),
            "--no-after" => { no_after = true; Ok(()) }
            "--add-after" => push_ids(&mut add, iter.next(), "--add-after", err),
            "--remove-after" => push_ids(&mut remove, iter.next(), "--remove-after", err),
            other => Err(usage_error(err, &format!("unknown argument for --edit: {other:?}"))),
        };
        if let Err(code) = result { return code; }
    }
    if no_after && !after.is_empty() { return usage_error(err, "--no-after cannot be combined with --after"); }
    // 선행 전체 교체(`after`)와 하나씩 더하고 빼기는 섞지 않는다.
    let after = (no_after || !after.is_empty()).then_some(after);
    if after.is_some() && !(add.is_empty() && remove.is_empty()) {
        return usage_error(err, "--after/--no-after replace all prerequisites; do not combine them with --add-after/--remove-after");
    }
    if title.is_none() && body.is_none() && assignee.is_none() && status.is_none() && after.is_none() && add.is_empty() && remove.is_empty() {
        return usage_error(err, "--edit needs at least one change");
    }
    if let Some(c) = title.as_deref().and_then(forbidden_control).or_else(|| body.as_deref().and_then(forbidden_control)) {
        return fail(err, &format!("title or body contains control character U+{:04X}; only tab and newline are allowed", c as u32));
    }
    let board = match Board::load(env, err) { Ok(board) => board, Err(code) => return code };
    if !board.unknown(std::slice::from_ref(id)).is_empty() { return fail(err, &format!("no such task: {id} (see office-task --list)")); }
    if let Some(item) = board.item(id) {
        let field = |key: &str| item.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let next_title = title.clone().unwrap_or_else(|| field("title"));
        let next_body = body.clone().unwrap_or_else(|| field("body"));
        if next_title.trim().is_empty() && next_body.trim().is_empty() { return fail(err, "the task would have no title and no body"); }
    }
    let linked: Vec<String> = after.iter().flatten().chain(&add).cloned().collect();
    if linked.contains(id) { return usage_error(err, &format!("{id} cannot wait on itself")); }
    warn_unknown_references(env, assignee.as_ref().and_then(|a| a.as_deref()), &linked, err);
    let mut text = format!("---\nedit: {id}\n");
    if let Some(title) = &title { text.push_str(&format!("title: {title}\n")); }
    if let Some(assignee) = &assignee { text.push_str(&format!("assignee: {}\n", assignee.as_deref().unwrap_or(""))); }
    if let Some(status) = &status { text.push_str(&format!("status: {status}\n")); }
    if let Some(after) = &after { text.push_str(&format!("after: {}\n", after.join(", "))); }
    if !add.is_empty() { text.push_str(&format!("add-after: {}\n", add.join(", "))); }
    if !remove.is_empty() { text.push_str(&format!("remove-after: {}\n", remove.join(", "))); }
    if body.is_some() { text.push_str("body: replace\n"); }
    text.push_str("---\n");
    if let Some(body) = body.as_deref().filter(|b| !b.is_empty()) { text.push_str(body); text.push('\n'); }
    match write_request(&board, &text, now, err) {
        Ok(()) => { let _ = writeln!(out, "{id}"); 0 }
        Err(code) => code,
    }
}

/// 업무 하나를 본문까지 모두 보여 준다. 수신함에만 있는 업무는 앱이 가져간 뒤에 볼 수 있다.
fn show(id: &str, env: &TaskEnv, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let board = match Board::load(env, err) { Ok(board) => board, Err(code) => return code };
    let Some(item) = board.item(id) else {
        if board.pending.iter().any(|p| p == id) { return fail(err, &format!("{id} is in the inbox and not on the board yet; try again in a few seconds")); }
        return fail(err, &format!("no such task: {id} (see office-task --list)"));
    };
    let dir = env.data_dir().unwrap_or_default();
    let names: std::collections::HashMap<String, String> = load_profiles(&dir).unwrap_or_default().into_iter().collect();
    let field = |key: &str| item.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let assignee = match field("assigneeAgentId") {
        id if id.is_empty() => "unassigned".to_string(),
        id => match names.get(&id) { Some(name) => format!("{name} ({id})"), None => format!("{id} (no such character)") },
    };
    let mut lines = vec![format!("id: {id}"), format!("status: {}", field("status")), format!("assignee: {assignee}"), format!("title: {}", field("title"))];
    let after: Vec<String> = item.get("dependsOn").and_then(|v| v.as_array()).map(|ids| ids.iter().filter_map(|id| id.as_str())
        .map(|d| format!("{d} [{}]", board.status(d).unwrap_or_else(|| "deleted".into()))).collect()).unwrap_or_default();
    if !after.is_empty() { lines.push(format!("after: {}", after.join(", "))); }
    let dependents = board.dependents(id);
    if !dependents.is_empty() { lines.push(format!("waited on by: {}", dependents.join(", "))); }
    for line in lines { let _ = writeln!(out, "{}", escape_controls(&line)); }
    let body = field("body");
    if !body.is_empty() {
        // 본문의 줄바꿈·탭은 그대로 두고 나머지 제어문자만 이스케이프한다.
        let body: String = body.split('\n').map(|line| line.split('\t').map(escape_controls).collect::<Vec<_>>().join("\t")).collect::<Vec<_>>().join("\n");
        let _ = writeln!(out, "\n{body}");
    }
    0
}

const LIST_STATUS_ORDER: [&str; 4] = ["review", "in_progress", "todo", "done"];

/// 저장된 업무 목록을 한 줄씩 보여준다. 앱이 아직 가져가지 않은 수신함 파일은 개수만 알린다.
fn list(env: &TaskEnv, all: bool, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let Some(dir) = env.data_dir() else { return fail(err, "AGENT_OFFICE_WORK_INBOX and AGENT_OFFICE_APP_DATA are not set"); };
    let items = match load_items(&dir) { Ok(items) => items, Err(e) => return fail(err, &e) };
    let names: std::collections::HashMap<String, String> = load_profiles(&dir).unwrap_or_default().into_iter().collect();
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
        let _ = writeln!(out, "{}", escape_controls(&line));
    }
    if shown.is_empty() { let _ = writeln!(out, "(no tasks)"); }
    let pending = env.inbox().and_then(|inbox| std::fs::read_dir(inbox).ok()).map_or(0, |entries| entries.flatten()
        .filter(|entry| entry.file_name().to_str().is_some_and(|name| !name.starts_with('.') && name.ends_with(".md"))).count());
    if pending > 0 { let _ = writeln!(out, "({pending} inbox request(s) are not applied to the board yet)"); }
    0
}

/// 저장된 값의 제어문자가 터미널에 해석되지 않게(ESC 시퀀스, OSC 52 등) `\x1b`처럼 적는다.
fn escape_controls(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() { escaped.push_str(&format!("\\x{:02x}", c as u32)); } else { escaped.push(c); }
    }
    escaped
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

/// 사무실 shim 폴더. 공백 없는 OS temp에 둔다(권한 규칙·안내문에 경로가 박힌다).
pub fn shim_dir() -> PathBuf {
    std::env::temp_dir().join("agent-office").join("bin")
}

/// `shim_dir()` 아래 `name` shim 경로(Windows는 `.cmd`를 붙인다).
pub fn office_shim_path(name: &str) -> PathBuf {
    let name = if cfg!(windows) { format!("{name}.cmd") } else { name.to_string() };
    shim_dir().join(name)
}

/// `office-task` shim 경로.
pub fn shim_path() -> PathBuf {
    office_shim_path("office-task")
}

fn shim_body(exe: &Path, subcommand: &str) -> String {
    let exe = exe.display();
    if cfg!(windows) {
        format!("@echo off\r\n\"{exe}\" {subcommand} %*\r\n")
    } else {
        format!("#!/bin/sh\n# agent-office가 생성. 편집 금지.\nexec \"{exe}\" {subcommand} \"$@\"\n")
    }
}

/// `exe <subcommand> …`를 실행하는 shim을 `path`에 (재)작성한다. OS temp는 청소될 수 있어 세션 준비마다 부른다.
pub fn ensure_shim(path: &Path, exe: &Path, subcommand: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    crate::office_instructions::write_atomic(path, shim_body(exe, subcommand).as_bytes())?;
    crate::office_instructions::set_executable(path);
    Ok(())
}

/// shim이 없거나 내용이 다를 때만 다시 쓴다. 다시 썼으면 true.
/// 이미 떠 있는 세션은 env·안내문에 박힌 경로를 계속 쓰므로, 앱이 주기적으로 불러 OS temp 청소에서 되살린다.
pub fn refresh_shim(path: &Path, exe: &Path, subcommand: &str) -> std::io::Result<bool> {
    if std::fs::read(path).is_ok_and(|bytes| bytes == shim_body(exe, subcommand).as_bytes()) { return Ok(false); }
    ensure_shim(path, exe, subcommand).map(|()| true)
}

/// `interval`마다 `shims`(경로, 서브커맨드) 각각에 [`refresh_shim`]을 부르는 백그라운드 스레드를 띄운다(앱 수명 동안).
pub fn spawn_shim_keeper(shims: Vec<(PathBuf, &'static str)>, exe: PathBuf, interval: std::time::Duration) {
    let _ = std::thread::Builder::new().name("office-shims".into()).spawn(move || loop {
        for (path, subcommand) in &shims {
            if let Err(error) = refresh_shim(path, &exe, subcommand) {
                eprintln!("agent-office: failed to refresh {} shim: {error}", path.display());
            }
        }
        std::thread::sleep(interval);
    });
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
    fn report_overwrites_one_file_per_task_outside_the_request_queue() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path(), Some("a"));
        assert_eq!(call(&["--report", "t-1", "-"], &env, "status: done\r\nchanged: x\n\n"), (0, String::new(), String::new()));
        assert_eq!(call(&["--report", "t-1", "status: blocked"], &env, "").0, 0);
        let report = crate::auto_run::reports_dir(dir.path()).join("t-1.md");
        assert_eq!(std::fs::read_to_string(&report).unwrap(), "status: blocked\n");
        // 수신함 scan은 하위 폴더를 건너뛰므로 보고가 요청으로 읽히지 않는다.
        assert!(scan(dir.path(), std::time::SystemTime::now()).unwrap().is_empty());
        assert_eq!(call(&["--report", "t-1"], &env, "  \n").0, 1);
        assert_eq!(call(&["--report", "../x", "hi"], &env, "").0, 2);
        assert_eq!(call(&["--report"], &env, "").0, 2);
        assert_eq!(call(&["--report", "t-1", "bad\u{7}"], &env, "").0, 1);
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
        let (code, _, err) = call(&["Esc\u{1b}[8m"], &env, "");
        assert!(code == 1 && err.contains("U+001B"), "{err}");
        assert_eq!(call(&["Body", "-"], &env, "line\rover").0, 1);
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
        assert_eq!(out, "w3  [review]  @unassigned  Ship  after: w0\nw1  [todo]  @하나  Design\nw2  [todo]  @unassigned  Build the thing  after: w1 (blocked)\n(1 inbox request(s) are not applied to the board yet)\n");
        assert!(call(&["--list", "--all"], &env, "").1.contains("w0  [done]"));
        assert_eq!(call(&["--list", "x"], &env, "").0, 2);
        let empty = tempfile::tempdir().unwrap();
        std::fs::write(empty.path().join("work-items.json"), r#"[{"id":"w1","title":"a\u001b]52;c;aGk=\u0007b\tc\u009b","body":"","status":"todo","order":0}]"#).unwrap();
        let env = TaskEnv { work_inbox: None, app_data: Some(empty.path().into()), agent_id: None };
        assert_eq!(call(&["--list"], &env, "").1, "w1  [todo]  @unassigned  a\\x1b]52;c;aGk=\\x07b\\x09c\\x9b\n");
        std::fs::remove_file(empty.path().join("work-items.json")).unwrap();
        assert_eq!(call(&["--list"], &TaskEnv { work_inbox: None, app_data: Some(empty.path().into()), agent_id: None }, "").1, "(no tasks)\n");
    }

    #[test]
    fn warns_about_unknown_assignee_and_prerequisites_but_still_writes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("profiles.json"), r#"{"agents":[{"id":"p1","name":"하나"},{"id":"p2","name":"둘"},{"id":"p3","name":"둘"}]}"#).unwrap();
        std::fs::write(dir.path().join("work-items.json"), r#"[{"id":"w1","title":"A","body":"","status":"todo","order":0}]"#).unwrap();
        let env = TaskEnv { work_inbox: Some(dir.path().join("work-inbox")), app_data: None, agent_id: Some("p1".into()) };
        let (_, pending, _) = call(&["Pending"], &env, "");
        // 아는 담당(id·유일한 이름)과 보드·수신함에 있는 선행은 조용하다. 기본 담당은 확인하지 않는다.
        for args in [vec!["--to", "p2", "x"], vec!["--to", " 하나 ", "x"], vec!["--after", &format!("w1,{}", pending.trim()), "x"], vec!["x"]] {
            let (code, _, err) = call(&args, &env, "");
            assert_eq!((code, err.as_str()), (0, ""), "{args:?}");
        }
        let (code, out, err) = call(&["--to", "없는사람", "x"], &env, "");
        assert!(code == 0 && out.starts_with("t-") && err.contains("--to \"없는사람\": no character") && err.contains("unassigned"), "{err}");
        let (code, _, err) = call(&["--to", "둘", "x"], &env, "");
        assert!(code == 0 && err.contains("several characters"), "{err}");
        let (code, _, err) = call(&["--after", "w1", "--after", "t-ghost0000", "x"], &env, "");
        assert!(code == 0 && err.contains("--after t-ghost0000: no such task") && !err.contains("w1"), "{err}");
        assert_eq!(std::fs::read_dir(dir.path().join("work-inbox")).unwrap().count(), 8);
    }

    #[test]
    fn done_writes_a_completion_request_only_for_known_open_tasks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("work-items.json"), r#"[
            {"id":"w1","title":"A","body":"","status":"in_progress","order":0},
            {"id":"w0","title":"Old","body":"","status":"done","order":0}
        ]"#).unwrap();
        let env = TaskEnv { work_inbox: Some(dir.path().join("work-inbox")), app_data: None, agent_id: Some("a".into()) };
        let (code, _, err) = call(&["--done", "w1", "nope"], &env, "");
        assert_eq!(code, 1);
        assert!(err.contains("no such task: nope"));
        assert!(!dir.path().join("work-inbox").exists());
        let (code, out, err) = call(&["--done", "w1,w0"], &env, "");
        assert_eq!((code, out.as_str()), (0, "w1\n"));
        assert!(err.contains("w0 is already done"));
        // 수신함에만 있는 업무도 완료할 수 있다.
        let (_, pending, _) = call(&["Fresh"], &env, "");
        assert_eq!(call(&["--done", pending.trim()], &env, "").0, 0);
        let entries = scan(&dir.path().join("work-inbox"), std::time::SystemTime::now()).unwrap();
        // 테스트 시각이 고정이라 파일 이름순이 임의값 순서다.
        let mut requests: Vec<_> = entries.iter().filter(|e| !e.done.is_empty()).map(|e| e.done.clone()).collect();
        requests.sort();
        let mut expected = vec![vec!["w1".to_string()], vec![pending.trim().to_string()]];
        expected.sort();
        assert_eq!(requests, expected);
        assert_eq!(call(&["--done", "w0"], &env, "").0, 0);
        assert_eq!(call(&["--done"], &env, "").0, 2);
        assert_eq!(call(&["--done", "bad id!"], &env, "").0, 2);
    }

    fn board(items: &str) -> (tempfile::TempDir, TaskEnv) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("profiles.json"), r#"{"agents":[{"id":"a1","name":"하나"}]}"#).unwrap();
        std::fs::write(dir.path().join("work-items.json"), items).unwrap();
        let env = TaskEnv { work_inbox: Some(dir.path().join("work-inbox")), app_data: None, agent_id: Some("a1".into()) };
        (dir, env)
    }
    fn requests(dir: &Path) -> Vec<Parsed> {
        written(&dir.join("work-inbox")).into_iter().map(|(_, parsed)| parsed).collect()
    }

    #[test]
    fn delete_writes_a_request_only_for_known_tasks_and_names_dependents() {
        let (dir, env) = board(r#"[
            {"id":"w1","title":"A","body":"","status":"todo","order":0},
            {"id":"w2","title":"B","body":"","status":"todo","order":1,"dependsOn":["w1"]}
        ]"#);
        let (code, _, err) = call(&["--delete", "w1", "nope"], &env, "");
        assert!(code == 1 && err.contains("no such task: nope"), "{err}");
        assert!(!dir.path().join("work-inbox").exists());
        let (code, out, err) = call(&["--delete", "w1"], &env, "");
        assert_eq!((code, out.as_str()), (0, "w1\n"));
        assert!(err.contains("w2 no longer wait(s) on w1"), "{err}");
        // 함께 지우는 후속 업무는 알리지 않는다.
        let (_, _, err) = call(&["--delete", "w1,w2"], &env, "");
        assert_eq!(err, "");
        let deletes: Vec<_> = requests(dir.path()).into_iter().map(|p| p.delete).collect();
        assert!(deletes.contains(&vec!["w1".to_string()]) && deletes.contains(&vec!["w1".to_string(), "w2".to_string()]));
        assert_eq!(call(&["--delete"], &env, "").0, 2);
    }

    #[test]
    fn edit_writes_only_the_given_fields() {
        let (dir, env) = board(r#"[
            {"id":"w1","title":"A","body":"old","status":"todo","order":0,"dependsOn":["w0"]},
            {"id":"w0","title":"Z","body":"","status":"todo","order":1},
            {"id":"w3","title":"","body":"only body","status":"todo","order":2}
        ]"#);
        let (code, out, err) = call(&["--edit", "w1", "--title", " New   title ", "--to", "하나", "--status", "review", "--add-after", "w3", "--remove-after", "w0", "--body", "-"], &env, "# heading\r\n  kept\n");
        assert_eq!((code, out.as_str(), err.as_str()), (0, "w1\n", ""));
        let edit = requests(dir.path()).remove(0).edit.unwrap();
        assert_eq!(edit, crate::persistence::work_inbox::Edit {
            id: "w1".into(), title: Some("New title".into()), body: Some("# heading\n  kept".into()), assignee: Some("하나".into()),
            after: None, add_after: vec!["w3".into()], remove_after: vec!["w0".into()], status: Some("review".into()),
        });
        std::fs::remove_dir_all(dir.path().join("work-inbox")).unwrap();
        assert_eq!(call(&["--edit", "w1", "--unassigned", "--no-after"], &env, "").0, 0);
        let edit = requests(dir.path()).remove(0).edit.unwrap();
        assert_eq!((edit.assignee.as_deref(), edit.after, edit.title, edit.body), (Some(""), Some(vec![]), None, None));
        std::fs::remove_dir_all(dir.path().join("work-inbox")).unwrap();
        // 잘못된 요청은 아무것도 쓰지 않는다.
        for (args, code) in [
            (vec!["--edit", "w1"], 2), (vec!["--edit"], 2), (vec!["--edit", "w1", "--status", "later"], 2),
            (vec!["--edit", "w1", "--after", "w0", "--no-after"], 2), (vec!["--edit", "w1", "--after", "w0", "--add-after", "w3"], 2),
            (vec!["--edit", "w1", "--after", "w1"], 2), (vec!["--edit", "w1", "--bogus"], 2), (vec!["--edit", "w1", "--to", " "], 2),
            (vec!["--edit", "nope", "--title", "x"], 1), (vec!["--edit", "w3", "--body", ""], 1), (vec!["--edit", "w1", "--title", "a\u{1b}b"], 1),
        ] {
            assert_eq!(call(&args, &env, "").0, code, "{args:?}");
        }
        assert!(!dir.path().join("work-inbox").exists());
        // 모르는 담당·선행은 알리되 쓴다. 수신함에만 있는 업무도 고칠 수 있다.
        let (_, pending, _) = call(&["Fresh"], &env, "");
        let (code, _, err) = call(&["--edit", pending.trim(), "--to", "없는사람", "--add-after", "t-ghost"], &env, "");
        assert!(code == 0 && err.contains("no character") && err.contains("t-ghost"), "{err}");
    }

    #[test]
    fn show_prints_the_whole_task_with_links() {
        let (_dir, env) = board(r#"[
            {"id":"w1","title":"A","body":"line 1\n\tline 2\u001b[8m","status":"todo","order":0,"assigneeAgentId":"a1","dependsOn":["w0","gone"]},
            {"id":"w0","title":"Z","body":"","status":"done","order":0},
            {"id":"w2","title":"B","body":"","status":"todo","order":1,"dependsOn":["w1"]}
        ]"#);
        let (code, out, _) = call(&["--show", "w1"], &env, "");
        assert_eq!(code, 0);
        assert_eq!(out, "id: w1\nstatus: todo\nassignee: 하나 (a1)\ntitle: A\nafter: w0 [done], gone [deleted]\nwaited on by: w2\n\nline 1\n\tline 2\\x1b[8m\n");
        assert_eq!(call(&["--show", "w0"], &env, "").1, "id: w0\nstatus: done\nassignee: unassigned\ntitle: Z\nwaited on by: w1\n");
        assert_eq!(call(&["--show", "nope"], &env, "").0, 1);
        let (_, pending, _) = call(&["Fresh"], &env, "");
        let (code, _, err) = call(&["--show", pending.trim()], &env, "");
        assert!(code == 1 && err.contains("not on the board yet"), "{err}");
        assert_eq!(call(&["--show"], &env, "").0, 2);
        assert_eq!(call(&["--show", "w1", "w2"], &env, "").0, 2);
    }

    #[test]
    fn refresh_shim_recreates_missing_or_stale_shim_only() {
        let dir = tempfile::tempdir().unwrap();
        let shim = dir.path().join("bin").join("office-task");
        let exe = Path::new("/opt/agent-office");
        assert!(refresh_shim(&shim, exe, "task").unwrap());
        assert!(!refresh_shim(&shim, exe, "task").unwrap());
        std::fs::remove_dir_all(dir.path().join("bin")).unwrap();
        assert!(refresh_shim(&shim, exe, "task").unwrap());
        assert_eq!(std::fs::read_to_string(&shim).unwrap(), shim_body(exe, "task"));
        assert!(refresh_shim(&shim, Path::new("/opt/other"), "task").unwrap());
        assert!(std::fs::read_to_string(&shim).unwrap().contains("/opt/other"));
        assert!(refresh_shim(&shim, Path::new("/opt/other"), "md").unwrap());
        assert!(std::fs::read_to_string(&shim).unwrap().contains("\" md "));
    }

    #[test]
    fn shim_keeper_restores_every_deleted_shim() {
        let dir = tempfile::tempdir().unwrap();
        let task = dir.path().join("bin").join("office-task");
        let md = dir.path().join("bin").join("office-md");
        spawn_shim_keeper(vec![(task.clone(), "task"), (md.clone(), "md")], "/opt/agent-office".into(), std::time::Duration::from_millis(20));
        let wait = || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while !(task.exists() && md.exists()) { assert!(std::time::Instant::now() < deadline, "shim not restored"); std::thread::sleep(std::time::Duration::from_millis(10)); }
        };
        wait();
        std::fs::remove_dir_all(dir.path().join("bin")).unwrap();
        wait();
        assert!(std::fs::read_to_string(&task).unwrap().contains("\"/opt/agent-office\" task "));
        assert!(std::fs::read_to_string(&md).unwrap().contains("\"/opt/agent-office\" md "));
    }
}
