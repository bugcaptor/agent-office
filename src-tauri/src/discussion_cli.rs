//! `agent-office discuss` 오프라인 서브커맨드(`office-discuss` shim).
//! 토의 참가자가 이번 차례 발언을 앱 데이터에 남긴다. 어느 참가자인지는 세션 env
//! (`AGENT_OFFICE_DISCUSSION_SESSION`)가 정하므로 모델이 신원을 적지 않는다.
//! 파일은 참가자마다 하나(`discussions/turns/<세션 agentId>.md`)이고 다시 쓰면 바뀐다.
//! 앱은 그 차례를 보낸 뒤에 쓰인 것만 읽는다(`discussion_session::collect`).

use crate::persistence::work_inbox::{forbidden_control, MAX_BYTES};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

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

// ── 토의 만들기(`office-task --discuss`) ─────────────────────────────

pub const CREATE_USAGE: &str = "usage: office-task --discuss --moderator <name|id> --with <name|id>... [--role <name|id>=<role|none>]...
                   [--cli <name|id>=<cli>[:<model>]]... [--rounds N] [--minutes N] [--cost USD]
                   [--no-usage-guard] [--cwd DIR] <topic> [context | -]
Creates a team discussion in the ready state and prints its id. A person starts it from its card in the app.
--moderator picks the neutral moderator; --with adds a debater (1 to 4, repeatable).
--role sets a member's discussion role by role name or id (none = no role); members default to their profile role.
The moderator needs a moderator role and debaters a non-moderator role.
--cli sets a member's CLI (claude, codex, agy, kilo or pi) and optional model; it defaults to the role's, then the profile's.
Limits default to 4 rounds, 30 minutes and $5 (ranges 1-12, 5-240, 0.5-50). --cwd defaults to the current folder.
Participants run read-only: they can read code, docs and the web but must not change files.
";

/// `office-task --discuss`의 실행 환경.
pub struct CreateEnv {
    /// 업무 수신함 폴더. 요청은 그 아래 `discussions/`에 쓴다.
    pub inbox: Option<PathBuf>,
    /// 프로필·역할이 있는 앱 데이터 폴더.
    pub data_dir: Option<PathBuf>,
    pub agent_id: Option<String>,
    pub cwd: Option<PathBuf>,
}

struct Member { query: String, role: Option<Option<String>>, agent: Option<crate::types::WorkAgent> }

/// `office-task --discuss <args>`. 수신함에 요청 파일 하나를 쓰고 토의 id를 출력한다.
pub fn create(
    args: &[String],
    env: &CreateEnv,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
    now: chrono::DateTime<chrono::Utc>,
) -> i32 {
    use crate::persistence::discussion_store::{MAX_COST_USD_RANGE, MAX_MINUTES_RANGE, MAX_ROUNDS_RANGE};
    let mut moderator: Option<String> = None;
    let mut debaters: Vec<String> = vec![];
    let mut roles: Vec<(String, Option<String>)> = vec![];
    let mut clis: Vec<(String, crate::types::WorkAgent)> = vec![];
    let mut limits = crate::types::DiscussionLimits::default();
    let mut cwd: Option<String> = None;
    let mut positional: Vec<String> = vec![];
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if !positional.is_empty() {
            positional.push(arg.clone());
            continue;
        }
        if arg == "--" {
            positional.extend(iter.by_ref().cloned());
            break;
        }
        let mut value = |flag: &str| iter.next().cloned().ok_or_else(|| format!("{flag} requires a value"));
        let step: Result<(), String> = match arg.as_str() {
            "-h" | "--help" => { let _ = out.write_all(CREATE_USAGE.as_bytes()); return 0; }
            "--moderator" => value(arg).map(|v| moderator = Some(v)),
            "--with" => value(arg).map(|v| debaters.push(v)),
            "--role" => value(arg).and_then(|v| {
                let (who, role) = split_pair(&v, "--role")?;
                roles.push((who, (!role.eq_ignore_ascii_case("none")).then_some(role)));
                Ok(())
            }),
            "--cli" => value(arg).and_then(|v| {
                let (who, spec) = split_pair(&v, "--cli")?;
                clis.push((who, parse_agent(&spec)?));
                Ok(())
            }),
            "--rounds" => value(arg).and_then(|v| parse_in(&v, "--rounds", MAX_ROUNDS_RANGE).map(|n| limits.max_rounds = n)),
            "--minutes" => value(arg).and_then(|v| parse_in(&v, "--minutes", MAX_MINUTES_RANGE).map(|n| limits.max_minutes = n)),
            "--cost" => value(arg).and_then(|v| {
                let cost: f64 = v.trim().trim_start_matches('$').parse().map_err(|_| format!("--cost must be a number of USD: {v:?}"))?;
                if !cost.is_finite() || cost < MAX_COST_USD_RANGE.0 || cost > MAX_COST_USD_RANGE.1 {
                    return Err(format!("--cost must be between {} and {}", MAX_COST_USD_RANGE.0, MAX_COST_USD_RANGE.1));
                }
                limits.max_cost_usd = cost;
                Ok(())
            }),
            "--no-usage-guard" => { limits.usage_guard = false; Ok(()) }
            "--cwd" => value(arg).map(|v| cwd = Some(v)),
            other if other.starts_with("--") => Err(format!("unknown option {other}")),
            _ => { positional.push(arg.clone()); Ok(()) }
        };
        if let Err(message) = step { return create_usage_error(err, &message); }
    }
    if positional.is_empty() || positional.len() > 2 { return create_usage_error(err, "expected <topic> [context | -]"); }
    let Some(moderator) = moderator else { return create_usage_error(err, "--moderator is required"); };
    if debaters.is_empty() { return create_usage_error(err, "add at least one debater with --with"); }
    if debaters.len() > crate::persistence::discussion_store::DISCUSSION_MAX_DEBATERS {
        return create_usage_error(err, &format!("at most {} debaters (plus the moderator)", crate::persistence::discussion_store::DISCUSSION_MAX_DEBATERS));
    }
    let topic = positional[0].split_whitespace().collect::<Vec<_>>().join(" ");
    if topic.is_empty() { return create_fail(err, "the topic is empty"); }
    let context = match positional.get(1).map(String::as_str) {
        None => String::new(),
        Some("-") => {
            let mut bytes = vec![];
            if let Err(e) = stdin.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes) { return create_fail(err, &format!("failed to read stdin: {e}")); }
            match String::from_utf8(bytes) { Ok(t) => t, Err(_) => return create_fail(err, "stdin is not UTF-8") }
        }
        Some(text) => text.to_string(),
    };
    let context = context.replace("\r\n", "\n").trim().to_string();
    if let Some(c) = forbidden_control(&topic).or_else(|| forbidden_control(&context)) {
        return create_fail(err, &format!("topic or context contains control character U+{:04X}; only tab and newline are allowed", c as u32));
    }
    let Some(data_dir) = env.data_dir.as_deref() else { return create_fail(err, "AGENT_OFFICE_WORK_INBOX and AGENT_OFFICE_APP_DATA are not set"); };
    let Some(inbox) = env.inbox.as_deref() else { return create_fail(err, "AGENT_OFFICE_WORK_INBOX and AGENT_OFFICE_APP_DATA are not set"); };
    let profiles: Vec<crate::types::AgentProfile> = match std::fs::read(data_dir.join("profiles.json")) {
        Ok(bytes) => match serde_json::from_slice::<crate::types::PersistedState>(&bytes) {
            Ok(state) => state.agents,
            Err(e) => return create_fail(err, &format!("cannot read team members (profiles.json): {e}")),
        },
        Err(e) => return create_fail(err, &format!("cannot read team members (profiles.json): {e}")),
    };
    let role_list = match crate::persistence::role_store::RoleStore::new(data_dir.to_path_buf()).load() {
        Ok(file) => file.map(|f| f.roles),
        Err(e) => return create_fail(err, &format!("cannot read discussion roles (roles.json): {e}")),
    };
    // 사회자 먼저, 토론자는 받은 순서대로. 같은 팀원이 두 번 들어가면 거절한다.
    let mut members: Vec<Member> = std::iter::once(&moderator).chain(&debaters)
        .map(|q| Member { query: q.clone(), role: None, agent: None }).collect();
    let mut ids: Vec<String> = vec![];
    for member in &members {
        match resolve_member(&member.query, &profiles) {
            Ok(id) if ids.contains(&id) => return create_usage_error(err, &format!("{:?} is in the discussion twice", member.query)),
            Ok(id) => ids.push(id),
            Err(message) => return create_fail(err, &message),
        }
    }
    for (who, role) in roles {
        let Some(at) = position_of(&who, &profiles, &ids) else { return create_usage_error(err, &format!("--role: {who:?} is not in this discussion")); };
        members[at].role = Some(role);
    }
    for (who, agent) in clis {
        let Some(at) = position_of(&who, &profiles, &ids) else { return create_usage_error(err, &format!("--cli: {who:?} is not in this discussion")); };
        members[at].agent = Some(agent);
    }
    let roles_known = role_list.as_deref().unwrap_or(&[]);
    if role_list.is_none() && members.iter().any(|m| matches!(m.role, Some(Some(_)))) {
        return create_fail(err, "no discussion roles exist yet; open the role list in the app once, or leave --role out");
    }
    let mut participants = vec![];
    for (at, member) in members.iter().enumerate() {
        let is_moderator = at == 0;
        let profile = profiles.iter().find(|p| p.id == ids[at]);
        let fits = |r: &&crate::types::DiscussionRole| (r.stance == crate::types::DiscussionStance::Moderator) == is_moderator;
        let role_id = match &member.role {
            Some(None) => None,
            Some(Some(query)) => match resolve_role(query, roles_known) {
                Ok(role) if fits(&role) => Some(role.id.clone()),
                Ok(role) => return create_usage_error(err, &format!(
                    "role {:?} cannot be used by {}: {}", role.name, member.query,
                    if is_moderator { "the moderator needs a moderator role" } else { "a debater cannot take a moderator role" })),
                Err(message) => return create_fail(err, &message),
            },
            // 렌더러 `defaultRoleId`와 같다: 프로필 기본 역할이 칸에 맞으면 그것, 사회자는 첫 사회자 역할.
            None => roles_known.iter().filter(fits).find(|r| Some(&r.id) == profile.and_then(|p| p.default_role_id.as_ref()))
                .or_else(|| if is_moderator { roles_known.iter().find(fits) } else { None })
                .map(|r| r.id.clone()),
        };
        // 렌더러 `defaultAgent`와 같다: 역할 CLI → 프로필 기본 CLI → claude.
        let agent = member.agent.clone()
            .or_else(|| role_id.as_ref().and_then(|id| roles_known.iter().find(|r| &r.id == id)).and_then(|r| r.agent.clone()))
            .or_else(|| profile.and_then(|p| p.default_agent.clone()))
            .unwrap_or(crate::types::WorkAgent { cli: crate::types::WorkCli::Claude, model: None, effort: None });
        participants.push(crate::types::DiscussionParticipant { agent_id: ids[at].clone(), role_id, agent });
    }
    let cwd = cwd.map(|c| c.trim().to_string()).filter(|c| !c.is_empty())
        .or_else(|| env.cwd.as_ref().map(|c| c.to_string_lossy().into_owned()));
    if cwd.as_deref().is_some_and(|c| !Path::new(c).is_absolute()) {
        return create_fail(err, "--cwd must be an absolute path");
    }
    if let Some(c) = cwd.as_deref().and_then(forbidden_control) {
        return create_fail(err, &format!("--cwd contains control character U+{:04X}", c as u32));
    }
    let random = uuid::Uuid::new_v4().simple().to_string();
    let request = crate::persistence::discussion_inbox::DiscussionRequest {
        version: crate::persistence::discussion_inbox::REQUEST_VERSION,
        id: format!("disc-{}", &random[..8]),
        topic,
        context,
        cwd,
        moderator_agent_id: ids[0].clone(),
        participants,
        limits,
        requested_by: env.agent_id.clone().filter(|id| !id.trim().is_empty()),
    };
    let bytes = serde_json::to_vec_pretty(&request).expect("discussion request serializes");
    if bytes.len() > MAX_BYTES { return create_fail(err, &format!("discussion exceeds {MAX_BYTES} bytes")); }
    let dir = crate::persistence::discussion_inbox::inbox_dir(inbox);
    let path = dir.join(format!("{}-{}.json", now.format("%Y%m%dT%H%M%S%3fZ"), &random[8..16]));
    let written = std::fs::create_dir_all(&dir).and_then(|()| crate::office_instructions::write_atomic(&path, &bytes));
    match written {
        Ok(()) => {
            let _ = writeln!(err, "office-task: the discussion is ready; a person starts it from its card in the app");
            let _ = writeln!(out, "{}", request.id);
            0
        }
        Err(e) => create_fail(err, &format!("failed to write {}: {e}", path.display())),
    }
}

fn split_pair(value: &str, flag: &str) -> Result<(String, String), String> {
    let (who, rest) = value.split_once('=').ok_or_else(|| format!("{flag} takes <member>=<value>: {value:?}"))?;
    let (who, rest) = (who.trim(), rest.trim());
    if who.is_empty() || rest.is_empty() { return Err(format!("{flag} takes <member>=<value>: {value:?}")); }
    Ok((who.to_string(), rest.to_string()))
}

fn parse_agent(spec: &str) -> Result<crate::types::WorkAgent, String> {
    let (cli, model) = match spec.split_once(':') {
        Some((cli, model)) => (cli.trim(), Some(model.trim().to_string()).filter(|m| !m.is_empty())),
        None => (spec.trim(), None),
    };
    if model.as_deref().is_some_and(|m| m.len() > 128 || m.chars().any(char::is_control)) {
        return Err("--cli: the model must be one short line".into());
    }
    let cli: crate::types::WorkCli = serde_json::from_value(serde_json::Value::String(cli.to_ascii_lowercase()))
        .map_err(|_| format!("--cli: unknown CLI {cli:?} (claude, codex, agy, kilo or pi)"))?;
    Ok(crate::types::WorkAgent { cli, model, effort: None })
}

fn parse_in(value: &str, flag: &str, (lo, hi): (u32, u32)) -> Result<u32, String> {
    value.trim().parse::<u32>().ok().filter(|n| (lo..=hi).contains(n)).ok_or_else(|| format!("{flag} must be a whole number from {lo} to {hi}: {value:?}"))
}

/// 팀원 id 또는 (하나뿐인) 이름.
fn resolve_member(query: &str, profiles: &[crate::types::AgentProfile]) -> Result<String, String> {
    let query = query.trim();
    if let Some(p) = profiles.iter().find(|p| p.id == query) { return Ok(p.id.clone()); }
    let named: Vec<_> = profiles.iter().filter(|p| p.name.trim() == query).collect();
    match named.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => Err(format!("no team member named {query:?}; use a team member name or id as shown in the app")),
        _ => Err(format!("several team members are named {query:?}; use the member id")),
    }
}

fn position_of(query: &str, profiles: &[crate::types::AgentProfile], ids: &[String]) -> Option<usize> {
    let id = resolve_member(query, profiles).ok()?;
    ids.iter().position(|i| *i == id)
}

fn resolve_role<'a>(query: &str, roles: &'a [crate::types::DiscussionRole]) -> Result<&'a crate::types::DiscussionRole, String> {
    let query = query.trim();
    if let Some(r) = roles.iter().find(|r| r.id == query) { return Ok(r); }
    let named: Vec<_> = roles.iter().filter(|r| r.name.trim() == query).collect();
    match named.as_slice() {
        [one] => Ok(one),
        [] => {
            let names: Vec<&str> = roles.iter().map(|r| r.name.as_str()).collect();
            Err(format!("no discussion role named {query:?}; roles: {}", names.join(", ")))
        }
        _ => Err(format!("several roles are named {query:?}; use the role id")),
    }
}

fn create_usage_error(err: &mut dyn Write, message: &str) -> i32 {
    let _ = write!(err, "office-task: {message}\n{CREATE_USAGE}");
    2
}
fn create_fail(err: &mut dyn Write, message: &str) -> i32 {
    let _ = writeln!(err, "office-task: {message}");
    1
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

    fn profile(id: &str, name: &str, extra: serde_json::Value) -> serde_json::Value {
        let mut v = serde_json::json!({ "id": id, "name": name, "role": "dev", "seed": "s", "createdAt": 1, "deskIndex": 0 });
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        v
    }
    /// 팀원 넷(이름이 겹치는 둘 포함)과, `roles`가 있으면 역할 목록을 둔 앱 데이터.
    fn team(roles: bool) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let agents = serde_json::json!([
            profile("p-mod", "Mo", serde_json::json!({ "defaultRoleId": "r-mod" })),
            profile("p-ada", "Ada", serde_json::json!({ "defaultRoleId": "r-critic", "defaultAgent": { "cli": "codex", "model": "gpt-x" } })),
            profile("p-bo1", "Bo", serde_json::json!({})),
            profile("p-bo2", "Bo", serde_json::json!({})),
        ]);
        std::fs::write(tmp.path().join("profiles.json"), serde_json::json!({ "version": 1, "agents": agents }).to_string()).unwrap();
        if roles {
            let roles = serde_json::json!({ "version": 1, "roles": [
                { "id": "r-mod", "name": "사회자", "stance": "moderator" },
                { "id": "r-critic", "name": "비판자", "stance": "critical", "agent": { "cli": "pi" } },
                { "id": "r-free", "name": "자유", "stance": "free" },
            ] });
            std::fs::write(tmp.path().join("roles.json"), roles.to_string()).unwrap();
        }
        tmp
    }
    fn create_env(dir: &std::path::Path) -> CreateEnv {
        CreateEnv { inbox: Some(dir.join("work-inbox")), data_dir: Some(dir.to_path_buf()), agent_id: Some("p-ada".into()), cwd: Some(dir.to_path_buf()) }
    }
    fn create_call(args: &[&str], env: &CreateEnv, stdin: &str) -> (i32, String, String) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let now = "2026-11-20T09:00:00.000Z".parse().unwrap();
        let code = create(&args, env, &mut stdin.as_bytes(), &mut out, &mut err, now);
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }
    fn requests(dir: &std::path::Path) -> Vec<crate::persistence::discussion_inbox::DiscussionRequest> {
        let inbox = crate::persistence::discussion_inbox::inbox_dir(&dir.join("work-inbox"));
        let Ok(entries) = std::fs::read_dir(inbox) else { return vec![] };
        entries.flatten().filter(|e| e.path().is_file()).map(|e| serde_json::from_slice(&std::fs::read(e.path()).unwrap()).unwrap()).collect()
    }

    #[test]
    fn create_writes_a_ready_request_with_defaults_the_app_would_pick() {
        let tmp = team(true);
        let (code, out, err) = create_call(&["--moderator", "Mo", "--with", "Ada", "--with", "p-bo1", "저장 형식을  바꿀까", "-"], &create_env(tmp.path()), "맥락\r\n둘째 줄\n");
        assert_eq!(code, 0, "{err}");
        assert!(err.contains("a person starts it"), "{err}");
        let [request] = requests(tmp.path()).try_into().unwrap();
        assert_eq!(out, format!("{}\n", request.id));
        assert!(request.id.starts_with("disc-") && crate::persistence::work_inbox::valid_task_id(&request.id));
        assert_eq!((request.topic.as_str(), request.context.as_str()), ("저장 형식을 바꿀까", "맥락\n둘째 줄"));
        assert_eq!(request.cwd.as_deref(), tmp.path().to_str());
        assert_eq!(request.moderator_agent_id, "p-mod");
        assert_eq!(request.requested_by.as_deref(), Some("p-ada"));
        assert_eq!(request.limits, crate::types::DiscussionLimits::default());
        let rows: Vec<_> = request.participants.iter().map(|p| (p.agent_id.as_str(), p.role_id.as_deref(), p.agent.cli.clone(), p.agent.model.as_deref())).collect();
        use crate::types::WorkCli::*;
        // 사회자: 프로필 기본 역할(사회자). Ada: 프로필 역할(비판자)과 그 역할 CLI(pi). Bo: 역할 없음, claude.
        assert_eq!(rows, vec![("p-mod", Some("r-mod"), Claude, None), ("p-ada", Some("r-critic"), Pi, None), ("p-bo1", None, Claude, None)]);
        // 앱 수신함이 준비 상태로 받는다.
        let store = crate::persistence::discussion_store::DiscussionStore::new(tmp.path().to_path_buf());
        let imported = crate::persistence::discussion_inbox::import(&tmp.path().join("work-inbox"), &store, &|_| true, 9).unwrap();
        assert_eq!(imported[0].state, crate::types::DiscussionState::Ready);
    }

    #[test]
    fn create_applies_roles_clis_and_limits() {
        let tmp = team(true);
        let (code, _, err) = create_call(&[
            "--moderator", "p-mod", "--with", "Ada", "--with", "p-bo2",
            "--role", "Ada=none", "--role", "p-bo2=비판자", "--cli", "Ada=codex:gpt-5.5", "--cli", "p-bo2=claude",
            "--rounds", "2", "--minutes", "15", "--cost", "$1.5", "--no-usage-guard", "--cwd", tmp.path().to_str().unwrap(),
            "주제", "짧은 맥락",
        ], &create_env(tmp.path()), "");
        assert_eq!(code, 0, "{err}");
        let [r] = requests(tmp.path()).try_into().unwrap();
        assert_eq!((r.limits.max_rounds, r.limits.max_minutes, r.limits.max_cost_usd, r.limits.usage_guard), (2, 15, 1.5, false));
        assert_eq!(r.cwd.as_deref(), tmp.path().to_str());
        assert_eq!(r.context, "짧은 맥락");
        let ada = &r.participants[1];
        assert_eq!((ada.role_id.as_deref(), ada.agent.cli.clone(), ada.agent.model.as_deref()), (None, crate::types::WorkCli::Codex, Some("gpt-5.5")));
        // 직접 준 CLI가 역할 CLI보다 먼저다.
        let bo = &r.participants[2];
        assert_eq!((bo.role_id.as_deref(), bo.agent.cli.clone()), (Some("r-critic"), crate::types::WorkCli::Claude));
    }

    #[test]
    fn create_rejects_bad_arguments_without_writing() {
        let tmp = team(true);
        let env = create_env(tmp.path());
        let usage: &[&[&str]] = &[
            &["주제"],                                             // 사회자 없음
            &["--moderator", "Mo", "주제"],                         // 토론자 없음
            &["--moderator", "Mo", "--with", "Mo", "주제"],          // 같은 팀원 두 번
            &["--moderator", "Mo", "--with", "Ada", "--with", "p-bo1", "--with", "p-bo2", "--with", "x", "--with", "y", "주제"],
            &["--moderator", "Mo", "--with", "Ada", "--rounds", "13", "주제"],
            &["--moderator", "Mo", "--with", "Ada", "--minutes", "4", "주제"],
            &["--moderator", "Mo", "--with", "Ada", "--cost", "100", "주제"],
            &["--moderator", "Mo", "--with", "Ada", "--cli", "Ada=gpt", "주제"],
            &["--moderator", "Mo", "--with", "Ada", "--cli", "Ada", "주제"],
            &["--moderator", "Mo", "--with", "Ada", "--role", "p-bo1=비판자", "주제"], // 참가자가 아님
            &["--moderator", "Mo", "--with", "Ada", "--role", "Ada=사회자", "주제"],   // 토론자에 사회자 역할
            &["--moderator", "Mo", "--with", "Ada", "--role", "Mo=비판자", "주제"],    // 사회자에 토론 역할
            &["--moderator", "Mo", "--with", "Ada", "--bogus", "주제"],
            &["--moderator", "Mo", "--with", "Ada"],
        ];
        for args in usage {
            assert_eq!(create_call(args, &env, "").0, 2, "{args:?}");
        }
        let failures: &[(&[&str], &str)] = &[
            (&["--moderator", "Mo", "--with", "Ada", "--cwd", "./repo", "주제"], "absolute path"),
            (&["--moderator", "Mo", "--with", "Bo", "주제"], "several team members"),
            (&["--moderator", "Mo", "--with", "Zed", "주제"], "no team member"),
            (&["--moderator", "Mo", "--with", "Ada", "--role", "Ada=없는역할", "주제"], "no discussion role"),
            (&["--moderator", "Mo", "--with", "Ada", "a\x1b[2Jb"], "control character"),
            (&["--moderator", "Mo", "--with", "Ada", "  "], "topic is empty"),
        ];
        for (args, message) in failures {
            let (code, _, err) = create_call(args, &env, "");
            assert_eq!(code, 1, "{args:?}");
            assert!(err.contains(message), "{args:?}: {err}");
        }
        assert!(requests(tmp.path()).is_empty());
        // 프로필 파일이 없으면 만들 수 없다.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(create_call(&["--moderator", "Mo", "--with", "Ada", "주제"], &create_env(empty.path()), "").0, 1);
        assert_eq!(create_call(&["--help"], &env, "").0, 0);
    }

    #[test]
    fn create_without_a_role_list_leaves_roles_empty_but_refuses_named_roles() {
        let tmp = team(false);
        let env = create_env(tmp.path());
        let (code, _, err) = create_call(&["--moderator", "Mo", "--with", "Ada", "--role", "Ada=비판자", "주제"], &env, "");
        assert_eq!(code, 1);
        assert!(err.contains("no discussion roles exist yet"), "{err}");
        assert_eq!(create_call(&["--moderator", "Mo", "--with", "Ada", "주제"], &env, "").0, 0);
        let [r] = requests(tmp.path()).try_into().unwrap();
        assert!(r.participants.iter().all(|p| p.role_id.is_none()));
        // 역할이 없으면 프로필 기본 CLI를 쓴다.
        assert_eq!(r.participants[1].agent.model.as_deref(), Some("gpt-x"));
    }

    #[test]
    fn office_task_routes_discuss_to_create() {
        let tmp = team(true);
        let env = crate::work_inbox_cli::TaskEnv { work_inbox: None, app_data: Some(tmp.path().into()), agent_id: None };
        let args: Vec<String> = ["--discuss", "--moderator", "Mo", "--with", "Ada", "주제"].iter().map(|s| s.to_string()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = crate::work_inbox_cli::run_with(&args, &env, &mut "".as_bytes(), &mut out, &mut err, chrono::Utc::now());
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
        assert_eq!(requests(tmp.path()).len(), 1);
        // 업무 수신함 파일로는 보이지 않는다(하위 폴더).
        assert!(crate::persistence::work_inbox::scan(&tmp.path().join("work-inbox"), std::time::SystemTime::now()).unwrap().is_empty());
    }

    #[test]
    fn help_prints_usage() {
        let (code, err) = call(&["--help"], &DiscussEnv::default(), "");
        assert_eq!(code, 0);
        assert!(err.starts_with("usage: office-discuss"));
        assert_eq!(call(&[], &DiscussEnv::default(), "").0, 2);
    }
}
