//! `agent-office task` 오프라인 서브커맨드(docs/work-inbox-design.md §에이전트 도구).
//! GUI·tokio 없이 수신함에 파일 하나를 원자적으로 쓴다. control 서버와 토큰이
//! 필요 없어 CLI 제어가 꺼져 있어도 동작한다.

use crate::persistence::work_inbox::MAX_BYTES;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: office-task [--to <name|id> | --unassigned] <title> [body | -]\n";

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
    let mut assignee = Assignee::Default;
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
    let content = render(&title, body.trim_matches('\n'), assignee.as_deref());
    if content.len() > MAX_BYTES { return fail(err, &format!("task exceeds {MAX_BYTES} bytes")); }
    let Some(inbox) = env.inbox() else { return fail(err, "AGENT_OFFICE_WORK_INBOX and AGENT_OFFICE_APP_DATA are not set"); };
    let name = format!("{}-{}.md", now.format("%Y%m%dT%H%M%S%3fZ"), &uuid::Uuid::new_v4().simple().to_string()[..8]);
    match write(&inbox, &name, content.as_bytes()) {
        Ok(path) => { let _ = writeln!(out, "{}", path.display()); 0 }
        Err(e) => fail(err, &format!("failed to write {}: {e}", inbox.display())),
    }
}

/// 제목 줄은 항상 `# `로 시작해 본문이나 빈 제목이 frontmatter·제목으로 잘못 읽히지 않는다.
fn render(title: &str, body: &str, assignee: Option<&str>) -> String {
    let mut text = String::new();
    if let Some(assignee) = assignee { text.push_str(&format!("---\nassignee: {assignee}\n---\n")); }
    text.push_str(&format!("# {title}\n"));
    if !body.is_empty() { text.push_str(body); text.push('\n'); }
    text
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
    use crate::persistence::work_inbox::{parse, scan, Parsed};

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
        assert!(out.trim().ends_with(&name));
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
}
