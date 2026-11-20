//! `office-task --roles` · `--role-add` · `--role-edit` · `--role-delete`: 토의 역할 목록 보기·바꾸기.
//! 바꾸기는 수신함(`<work-inbox>/roles/`)에 요청 파일 하나를 쓰고 앱이 몇 초 안에 적용한다
//! (`persistence::role_inbox`). 목록과 이름 찾기는 아직 적용되지 않은 요청까지 겹쳐 본다.

use crate::persistence::durable::write_atomic;
use crate::persistence::role_inbox::{self, RoleOp, RolePatch, RoleRequest, REQUEST_VERSION};
use crate::persistence::work_inbox::MAX_BYTES;
use crate::types::{DiscussionRole, DiscussionStance};
use std::io::{Read, Write};
use std::path::PathBuf;

pub const USAGE: &str = "usage: office-task --roles [--json]
       office-task --role-add <name> --stance <stance> [--perspective <text | ->] [--criteria <text | ->]
                   [--cli <cli>[:<model>]]
       office-task --role-edit <role> [--name <name>] [--stance <stance>] [--perspective <text | ->]
                   [--criteria <text | ->] [--cli <cli>[:<model>] | --no-cli]
       office-task --role-delete <role>...
Lists or changes the discussion roles (role cards) that members take in a team discussion.
--role-add prints the new role id. <role> is a role name or id. Changes are applied by the app within a few
seconds, in the order you run them; a role you just added can already be used with --discuss --role.
Stances: moderator (neutral; only a moderator can take it), free, supportive, critical, devils_advocate,
divergent, synthesizer. --perspective is the role's perspective and responsibility, --criteria its criteria
and things not to do; use - to read one of them from stdin. --cli sets the CLI (claude, codex, agy, kilo or pi)
and model a member defaults to when taking this role.
";

/// 실행 환경. 테스트가 프로세스 env를 건드리지 않도록 값으로 받는다.
pub struct RoleEnv {
    pub inbox: Option<PathBuf>,
    /// `roles.json`이 있는 앱 데이터 폴더.
    pub data_dir: Option<PathBuf>,
    pub agent_id: Option<String>,
}

/// `office-task <--roles | --role-add | --role-edit | --role-delete> <args>`.
pub fn run(
    command: &str,
    args: &[String],
    env: &RoleEnv,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
    now: chrono::DateTime<chrono::Utc>,
) -> i32 {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        let _ = out.write_all(USAGE.as_bytes());
        return 0;
    }
    let (Some(inbox), Some(data_dir)) = (env.inbox.as_deref(), env.data_dir.as_deref()) else {
        return fail(err, "AGENT_OFFICE_WORK_INBOX and AGENT_OFFICE_APP_DATA are not set");
    };
    let saved = match crate::persistence::role_store::RoleStore::new(data_dir.to_path_buf()).load() {
        Ok(file) => file.map(|f| f.roles),
        Err(e) => return fail(err, &format!("cannot read discussion roles (roles.json): {e}")),
    };
    let seeded = saved.is_some();
    let roles = role_inbox::with_pending(inbox, saved.unwrap_or_default());
    let ops = match command {
        "--roles" => return list(args, &roles, seeded, out, err),
        "--role-add" => add(args, &roles, stdin),
        "--role-edit" => edit(args, &roles, stdin),
        "--role-delete" => delete(args, &roles),
        _ => Err(Problem::Usage(format!("unknown command {command}"))),
    };
    let ops = match ops {
        Ok(ops) => ops,
        Err(Problem::Usage(message)) => return usage_error(err, &message),
        Err(Problem::Fail(message)) => return fail(err, &message),
    };
    // 앱과 같은 검사. 여기서 통과하면(rc 0) 앱이 거부하지 않는다.
    let mut check = roles.clone();
    for op in &ops {
        if let Err(code) = role_inbox::apply(&mut check, op.clone()) {
            return fail(err, &format!("the app would reject this change ({code})"));
        }
    }
    let dir = role_inbox::inbox_dir(inbox);
    for op in ops {
        let printed = match &op { RoleOp::Add { role } => Some(role.id.clone()), _ => None };
        let request = RoleRequest { version: REQUEST_VERSION, op, requested_by: env.agent_id.clone().filter(|id| !id.trim().is_empty()) };
        let bytes = serde_json::to_vec_pretty(&request).expect("role request serializes");
        if bytes.len() > MAX_BYTES { return fail(err, &format!("the role exceeds {MAX_BYTES} bytes")); }
        let random = uuid::Uuid::new_v4().simple().to_string();
        let path = dir.join(format!("{}-{}.json", now.format("%Y%m%dT%H%M%S%3fZ"), &random[..8]));
        if let Err(e) = write_atomic(&path, &bytes) {
            return fail(err, &format!("failed to write {}: {e}", path.display()));
        }
        if let Some(id) = printed { let _ = writeln!(out, "{id}"); }
    }
    if !seeded {
        let _ = writeln!(err, "office-task: no roles are saved yet; the app applies this after it seeds the preset roles (when it is running)");
    }
    0
}

enum Problem { Usage(String), Fail(String) }

fn list(args: &[String], roles: &[DiscussionRole], seeded: bool, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let json = match args {
        [] => false,
        [flag] if flag == "--json" => true,
        _ => return usage_error(err, "--roles takes only --json"),
    };
    if !seeded {
        let _ = writeln!(err, "office-task: no roles are saved yet; the app seeds the preset roles when it opens");
    }
    if json {
        let _ = writeln!(out, "{}", serde_json::to_string_pretty(roles).expect("roles serialize"));
        return 0;
    }
    for role in roles {
        let cli = role.agent.as_ref().map(|a| {
            let cli = serde_json::to_value(&a.cli).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            match &a.model { Some(model) => format!("\t{cli}:{model}"), None => format!("\t{cli}") }
        }).unwrap_or_default();
        let _ = writeln!(out, "{}\t{}\t{}{cli}", role.id, stance_name(role.stance), role.name);
    }
    0
}

fn add(args: &[String], roles: &[DiscussionRole], stdin: &mut dyn Read) -> Result<Vec<RoleOp>, Problem> {
    let mut fields = Fields::parse(args, stdin, false)?;
    let name = match fields.positional.as_slice() {
        [name] => name.clone(),
        _ => return Err(Problem::Usage("expected one <name>".into())),
    };
    if fields.name.is_some() || fields.clear_agent { return Err(Problem::Usage("--name and --no-cli are for --role-edit".into())); }
    let stance = fields.patch.stance.take().ok_or_else(|| Problem::Usage("--stance is required".into()))?;
    if roles.iter().any(|r| r.name.trim() == name.trim()) {
        return Err(Problem::Fail(format!("a role named {:?} already exists; change it with --role-edit", name.trim())));
    }
    let random = uuid::Uuid::new_v4().simple().to_string();
    let role = DiscussionRole {
        id: format!("role-{}", &random[..8]),
        name,
        perspective: fields.patch.perspective.unwrap_or_default(),
        stance,
        criteria: fields.patch.criteria.unwrap_or_default(),
        agent: fields.patch.agent,
    };
    Ok(vec![RoleOp::Add { role }])
}

fn edit(args: &[String], roles: &[DiscussionRole], stdin: &mut dyn Read) -> Result<Vec<RoleOp>, Problem> {
    let mut fields = Fields::parse(args, stdin, true)?;
    let query = match fields.positional.as_slice() {
        [query] => query.clone(),
        _ => return Err(Problem::Usage("expected one <role>".into())),
    };
    let id = crate::discussion_cli::resolve_role(&query, roles).map_err(Problem::Fail)?.id.clone();
    fields.patch.name = fields.name;
    fields.patch.clear_agent = fields.clear_agent;
    if fields.patch == RolePatch::default() {
        return Err(Problem::Usage("nothing to change; give --name, --stance, --perspective, --criteria, --cli or --no-cli".into()));
    }
    Ok(vec![RoleOp::Edit { id, patch: fields.patch }])
}

fn delete(args: &[String], roles: &[DiscussionRole]) -> Result<Vec<RoleOp>, Problem> {
    if args.is_empty() { return Err(Problem::Usage("expected <role>...".into())); }
    let mut ids: Vec<String> = vec![];
    for query in args {
        if query.starts_with("--") { return Err(Problem::Usage(format!("unknown option {query}"))); }
        let id = crate::discussion_cli::resolve_role(query, roles).map_err(Problem::Fail)?.id.clone();
        if !ids.contains(&id) { ids.push(id); }
    }
    Ok(ids.into_iter().map(|id| RoleOp::Delete { id }).collect())
}

/// `--role-add`·`--role-edit` 공통 옵션.
struct Fields { positional: Vec<String>, name: Option<String>, clear_agent: bool, patch: RolePatch }

impl Fields {
    fn parse(args: &[String], stdin: &mut dyn Read, editing: bool) -> Result<Self, Problem> {
        let mut fields = Fields { positional: vec![], name: None, clear_agent: false, patch: RolePatch::default() };
        let mut stdin_used = false;
        let mut text = |value: String, flag: &str| -> Result<String, Problem> {
            if value != "-" { return Ok(value); }
            if stdin_used { return Err(Problem::Usage("only one of --perspective and --criteria can read stdin (-)".into())); }
            stdin_used = true;
            crate::discussion_cli::read_stdin(stdin, flag).map(|t| t.replace("\r\n", "\n")).map_err(Problem::Fail)
        };
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            let mut value = |flag: &str| iter.next().cloned().ok_or_else(|| Problem::Usage(format!("{flag} requires a value")));
            match arg.as_str() {
                "--name" if editing => fields.name = Some(value(arg)?),
                "--stance" => fields.patch.stance = Some(parse_stance(&value(arg)?)?),
                "--perspective" => fields.patch.perspective = Some(text(value(arg)?, "perspective")?),
                "--criteria" => fields.patch.criteria = Some(text(value(arg)?, "criteria")?),
                "--cli" => fields.patch.agent = Some(crate::discussion_cli::parse_agent(&value(arg)?).map_err(Problem::Usage)?),
                "--no-cli" if editing => fields.clear_agent = true,
                "--" => fields.positional.extend(iter.by_ref().cloned()),
                other if other.starts_with("--") => return Err(Problem::Usage(format!("unknown option {other}"))),
                _ => fields.positional.push(arg.clone()),
            }
        }
        if fields.clear_agent && fields.patch.agent.is_some() { return Err(Problem::Usage("give --cli or --no-cli, not both".into())); }
        Ok(fields)
    }
}

const STANCES: [(&str, DiscussionStance); 7] = [
    ("moderator", DiscussionStance::Moderator),
    ("free", DiscussionStance::Free),
    ("supportive", DiscussionStance::Supportive),
    ("critical", DiscussionStance::Critical),
    ("devils_advocate", DiscussionStance::DevilsAdvocate),
    ("divergent", DiscussionStance::Divergent),
    ("synthesizer", DiscussionStance::Synthesizer),
];

fn parse_stance(value: &str) -> Result<DiscussionStance, Problem> {
    let key = value.trim().to_ascii_lowercase().replace('-', "_");
    STANCES.iter().find(|(name, _)| *name == key).map(|(_, s)| *s).ok_or_else(|| {
        Problem::Usage(format!("--stance: unknown stance {value:?} ({})", STANCES.map(|(n, _)| n).join(", ")))
    })
}

fn stance_name(stance: DiscussionStance) -> &'static str {
    STANCES.iter().find(|(_, s)| *s == stance).map(|(n, _)| *n).unwrap_or("free")
}

fn usage_error(err: &mut dyn Write, message: &str) -> i32 {
    let _ = write!(err, "office-task: {message}\n{USAGE}");
    2
}
fn fail(err: &mut dyn Write, message: &str) -> i32 {
    let _ = writeln!(err, "office-task: {message}");
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::role_store::RoleStore;
    use crate::types::{RolesFile, WorkCli};

    struct Fixture { _tmp: tempfile::TempDir, env: RoleEnv, store: RoleStore }
    fn fixture(seeded: bool) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let store = RoleStore::new(tmp.path().to_path_buf());
        if seeded {
            let critic = DiscussionRole { id: "preset-critic".into(), name: "비판자".into(), perspective: "약점을 찾는다".into(), stance: DiscussionStance::Critical, criteria: String::new(), agent: None };
            store.save(&RolesFile { version: 1, roles: vec![critic] }).unwrap();
        }
        let env = RoleEnv { inbox: Some(tmp.path().join("work-inbox")), data_dir: Some(tmp.path().to_path_buf()), agent_id: Some("a1".into()) };
        Fixture { _tmp: tmp, env, store }
    }
    fn call(f: &Fixture, command: &str, args: &[&str], stdin: &str) -> (i32, String, String) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(command, &args, &f.env, &mut stdin.as_bytes(), &mut out, &mut err, chrono::Utc::now());
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }
    fn import(f: &Fixture) -> Vec<DiscussionRole> {
        role_inbox::import(f.env.inbox.as_deref().unwrap(), &f.store).unwrap();
        f.store.load().unwrap().unwrap().roles
    }

    #[test]
    fn add_then_edit_then_delete_round_trip_through_the_app_inbox() {
        let f = fixture(true);
        let (code, out, err) = call(&f, "--role-add", &["보안 검토자", "--stance", "critical", "--perspective", "-", "--cli", "codex:gpt-5"], "공격 경로를 본다\r\n");
        assert_eq!(code, 0, "{err}");
        let id = out.trim().to_string();
        assert!(id.starts_with("role-"), "{out}");
        // 앱이 적용하기 전에도 목록·이름 찾기에 보인다.
        let (_, listed, _) = call(&f, "--roles", &[], "");
        assert!(listed.contains(&format!("{id}\tcritical\t보안 검토자\tcodex:gpt-5")), "{listed}");
        assert_eq!(call(&f, "--role-edit", &["보안 검토자", "--stance", "devils-advocate", "--no-cli"], "").0, 0);

        let roles = import(&f);
        let added = roles.iter().find(|r| r.id == id).unwrap();
        assert_eq!((added.perspective.as_str(), added.stance, added.agent.clone()), ("공격 경로를 본다", DiscussionStance::DevilsAdvocate, None));

        assert_eq!(call(&f, "--role-delete", &[&id, "비판자"], "").0, 0);
        assert!(import(&f).is_empty());
    }

    #[test]
    fn add_rejects_a_taken_name_a_missing_stance_and_two_stdin_fields_without_writing() {
        let f = fixture(true);
        let (code, _, err) = call(&f, "--role-add", &[" 비판자 ", "--stance", "free"], "");
        assert_eq!(code, 1);
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(call(&f, "--role-add", &["새 역할"], "").0, 2);
        assert_eq!(call(&f, "--role-add", &["새 역할", "--stance", "angry"], "").0, 2);
        assert_eq!(call(&f, "--role-add", &["새 역할", "--stance", "free", "--perspective", "-", "--criteria", "-"], "x").0, 2);
        assert_eq!(call(&f, "--role-edit", &["비판자"], "").0, 2, "nothing to change");
        assert_eq!(call(&f, "--role-edit", &["없는 역할", "--stance", "free"], "").0, 1);
        assert_eq!(call(&f, "--role-add", &["a\u{7}b", "--stance", "free"], "").0, 1, "the app would reject it");
        assert!(!role_inbox::inbox_dir(f.env.inbox.as_deref().unwrap()).exists());
    }

    #[test]
    fn before_presets_are_seeded_the_request_waits_and_says_so() {
        let f = fixture(false);
        let (code, out, err) = call(&f, "--role-add", &["새 역할", "--stance", "synthesizer", "--cli", "claude"], "");
        assert_eq!(code, 0);
        assert!(err.contains("no roles are saved yet"), "{err}");
        let (_, json, _) = call(&f, "--roles", &["--json"], "");
        let roles: Vec<DiscussionRole> = serde_json::from_str(&json).unwrap();
        assert_eq!(roles.len(), 1);
        assert_eq!((roles[0].id.as_str(), roles[0].agent.as_ref().map(|a| a.cli.clone())), (out.trim(), Some(WorkCli::Claude)));
        assert_eq!(role_inbox::import(f.env.inbox.as_deref().unwrap(), &f.store).unwrap(), None);
    }
}
