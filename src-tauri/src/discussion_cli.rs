//! `agent-office discuss` 오프라인 서브커맨드(`office-discuss` shim).
//! 토의 참가자가 이번 차례 발언을 앱 데이터에 남긴다. 어느 참가자인지는 세션 env
//! (`AGENT_OFFICE_DISCUSSION_SESSION`)가 정하므로 모델이 신원을 적지 않는다.
//! 파일은 참가자마다 하나(`discussions/turns/<세션 agentId>.md`)이고 다시 쓰면 바뀐다.
//! 앱은 그 차례를 보낸 뒤에 쓰인 것만 읽는다(`discussion_session::collect`).

use crate::persistence::work_inbox::{forbidden_control, MAX_BYTES};
use std::io::{Read, Write};
use std::path::PathBuf;

const USAGE: &str = "usage: office-discuss say [text | -]
Submits your utterance for the current discussion turn. Use - to read it from stdin.
Submitting again in the same turn replaces it. Only works inside an Agent Office discussion session.
";

/// 실행 환경. 테스트가 프로세스 env를 건드리지 않도록 값으로 받는다.
#[derive(Default)]
pub struct DiscussEnv {
    pub app_data: Option<PathBuf>,
    pub session: Option<String>,
}
impl DiscussEnv {
    fn from_process() -> Self {
        let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
        Self {
            app_data: var("AGENT_OFFICE_APP_DATA").map(PathBuf::from),
            session: var(crate::discussion_session::SESSION_ENV).map(|v| v.to_string_lossy().into_owned()),
        }
    }
}

pub fn run(args: Vec<String>) -> i32 {
    run_with(&args, &DiscussEnv::from_process(), &mut std::io::stdin(), &mut std::io::stderr())
}

pub fn run_with(args: &[String], env: &DiscussEnv, stdin: &mut dyn Read, err: &mut dyn Write) -> i32 {
    match args.first().map(String::as_str) {
        None | Some("--help" | "-h" | "help") => {
            let _ = write!(err, "{USAGE}");
            if args.is_empty() { 2 } else { 0 }
        }
        Some("say") => say(&args[1..], env, stdin, err),
        Some(other) => usage_error(err, &format!("unknown command {other:?}")),
    }
}

fn say(args: &[String], env: &DiscussEnv, stdin: &mut dyn Read, err: &mut dyn Write) -> i32 {
    let source = match args {
        [] => "-",
        [text] => text.as_str(),
        _ => return usage_error(err, "say takes one argument: [text | -]"),
    };
    let body = if source == "-" {
        let mut bytes = vec![];
        if let Err(e) = stdin.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes) {
            return fail(err, &format!("failed to read stdin: {e}"));
        }
        match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => return fail(err, "stdin is not UTF-8"),
        }
    } else {
        source.to_string()
    };
    let body = body.replace("\r\n", "\n");
    let body = body.trim_matches('\n');
    if body.trim().is_empty() { return fail(err, "the utterance is empty"); }
    if body.len() > MAX_BYTES { return fail(err, &format!("utterance exceeds {MAX_BYTES} bytes")); }
    if let Some(c) = forbidden_control(body) {
        return fail(err, &format!("utterance contains control character U+{:04X}; only tab and newline are allowed", c as u32));
    }
    let (Some(app_data), Some(session)) = (&env.app_data, &env.session) else {
        return fail(err, "not inside an Agent Office discussion session (AGENT_OFFICE_APP_DATA or AGENT_OFFICE_DISCUSSION_SESSION is not set)");
    };
    let Some(path) = crate::discussion_session::turn_file(app_data, session) else {
        return fail(err, &format!("invalid discussion session id: {session:?}"));
    };
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| crate::office_instructions::write_atomic(&path, format!("{body}\n").as_bytes()));
    match written {
        Ok(()) => 0,
        Err(e) => fail(err, &format!("failed to write {}: {e}", path.display())),
    }
}

fn usage_error(err: &mut dyn Write, message: &str) -> i32 {
    let _ = write!(err, "office-discuss: {message}\n{USAGE}");
    2
}
fn fail(err: &mut dyn Write, message: &str) -> i32 {
    let _ = writeln!(err, "office-discuss: {message}");
    1
}

/// `office-discuss` shim 경로.
pub fn shim_path() -> PathBuf {
    crate::work_inbox_cli::office_shim_path("office-discuss")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "discuss-d1.agent_A-1";

    fn call(args: &[&str], env: &DiscussEnv, stdin: &str) -> (i32, String) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let mut err = Vec::new();
        let code = run_with(&args, env, &mut stdin.as_bytes(), &mut err);
        (code, String::from_utf8(err).unwrap())
    }
    fn env(dir: &std::path::Path) -> DiscussEnv {
        DiscussEnv { app_data: Some(dir.to_path_buf()), session: Some(SESSION.into()) }
    }
    fn file(dir: &std::path::Path) -> PathBuf {
        crate::discussion_session::turn_file(dir, SESSION).unwrap()
    }

    #[test]
    fn say_writes_one_file_per_session_and_replaces_it() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(call(&["say", "-"], &env(tmp.path()), "first\r\nline\n\n").0, 0);
        assert_eq!(std::fs::read_to_string(file(tmp.path())).unwrap(), "first\nline\n");
        assert_eq!(call(&["say", "second"], &env(tmp.path()), "").0, 0);
        assert_eq!(std::fs::read_to_string(file(tmp.path())).unwrap(), "second\n");
        // 임시 파일이 남지 않는다.
        let names: Vec<_> = std::fs::read_dir(file(tmp.path()).parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names.len(), 1, "{names:?}");
    }

    #[test]
    fn say_without_text_reads_stdin() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(call(&["say"], &env(tmp.path()), "from stdin").0, 0);
        assert_eq!(std::fs::read_to_string(file(tmp.path())).unwrap(), "from stdin\n");
    }

    #[test]
    fn rejects_bad_input_and_outside_sessions_without_writing() {
        let tmp = tempfile::tempdir().unwrap();
        let (code, err) = call(&["say", "  \n"], &env(tmp.path()), "");
        assert_eq!(code, 1);
        assert!(err.contains("empty"));
        assert_eq!(call(&["say", "a\x1b[2Jb"], &env(tmp.path()), "").0, 1);
        assert_eq!(call(&["say", &"x".repeat(MAX_BYTES + 1)], &env(tmp.path()), "").0, 1);
        assert_eq!(call(&["say", "a", "b"], &env(tmp.path()), "").0, 2);
        assert_eq!(call(&["shout", "a"], &env(tmp.path()), "").0, 2);
        let (code, err) = call(&["say", "hi"], &DiscussEnv { app_data: Some(tmp.path().into()), session: None }, "");
        assert_eq!(code, 1);
        assert!(err.contains("not inside"));
        // 세션 id가 경로를 벗어나지 못한다.
        let bad = DiscussEnv { app_data: Some(tmp.path().into()), session: Some("discuss-../x.y".into()) };
        assert_eq!(call(&["say", "hi"], &bad, "").0, 1);
        let bad = DiscussEnv { app_data: Some(tmp.path().into()), session: Some("task-chat-t1".into()) };
        assert_eq!(call(&["say", "hi"], &bad, "").0, 1);
        assert!(!tmp.path().join("discussions").exists());
    }

    #[test]
    fn help_prints_usage() {
        let (code, err) = call(&["--help"], &DiscussEnv::default(), "");
        assert_eq!(code, 0);
        assert!(err.starts_with("usage: office-discuss"));
        assert_eq!(call(&[], &DiscussEnv::default(), "").0, 2);
    }
}
