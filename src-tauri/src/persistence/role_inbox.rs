// src-tauri/src/persistence/role_inbox.rs
//
// 에이전트·CLI(`office-task --role-add` 등)가 남긴 토의 역할 변경의 수신함.
// `<work-inbox>/roles/<시각>-<난수>.json`에 요청 하나를 원자적으로 쓰고, 렌더러의 수신함 주기가
// `scan_role_inbox`로 `roles.json`에 적용한다. CLI가 `roles.json`을 직접 쓰지 않는 까닭:
// 렌더러는 목록 전체를 캐시에서 저장하므로 CLI 변경을 덮고, 파일이 없을 때만 프리셋을 시드하며,
// Codex 샌드박스는 수신함 밖 쓰기를 막는다.
//
// - 파일은 누구나 쓸 수 있으므로 요청을 믿지 않는다. 적용할 때 다시 검사하고, 어긋나면 `rejected/`로 옮긴다.
// - `roles.json`이 아직 없으면 적용하지 않고 둔다. 렌더러가 프리셋을 시드한 뒤 적용한다.
// - CLI도 아직 적용되지 않은 요청을 겹쳐 본다(`with_pending`). 방금 만든 역할을 바로 `--discuss --role`에 쓸 수 있다.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::discussion_inbox::request_names;
use super::role_store::RoleStore;
use super::work_inbox::{forbidden_control, reject, valid_task_id, MAX_BYTES};
use crate::types::{DiscussionRole, DiscussionStance, RolesFile, WorkAgent};

/// 요청 파일 형식 버전.
pub const REQUEST_VERSION: u32 = 1;
/// 한 번에 적용하는 파일 수.
const MAX_BATCH: usize = 32;
/// 역할 이름 상한(문자).
pub const MAX_NAME_CHARS: usize = 80;
/// 관점·기준 칸 상한(문자).
pub const MAX_TEXT_CHARS: usize = 4000;

/// 고칠 칸만 담는다. `clear_agent`는 기본 CLI를 지운다(`agent`보다 먼저).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RolePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")] pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub perspective: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub stance: Option<DiscussionStance>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub criteria: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub agent: Option<WorkAgent>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")] pub clear_agent: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum RoleOp {
    Add { role: DiscussionRole },
    Edit { id: String, patch: RolePatch },
    Delete { id: String },
}

/// 수신함 요청 하나. Rust 안에서만(CLI가 쓰고 앱이 읽는다) 쓰는 형식이라 TS 미러가 없다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleRequest {
    pub version: u32,
    #[serde(flatten)]
    pub op: RoleOp,
    /// 만든 에이전트(세션 env). 앱은 저장하지 않는다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_by: Option<String>,
}

/// 수신함 폴더(`<work-inbox>/roles`).
pub fn inbox_dir(work_inbox: &Path) -> PathBuf {
    work_inbox.join("roles")
}

/// 역할 하나의 규칙 위반 코드.
fn role_problem(role: &DiscussionRole) -> Option<&'static str> {
    if !valid_task_id(&role.id) {
        return Some("invalid-id");
    }
    if role.name.trim().is_empty() {
        return Some("name");
    }
    if role.name.chars().count() > MAX_NAME_CHARS || role.name.contains('\n') {
        return Some("name-too-long");
    }
    if [&role.perspective, &role.criteria].iter().any(|t| t.chars().count() > MAX_TEXT_CHARS) {
        return Some("too-long");
    }
    if [&role.name, &role.perspective, &role.criteria].iter().any(|t| forbidden_control(t).is_some()) {
        return Some("control-char");
    }
    let short = |v: &Option<String>| v.as_deref().is_none_or(|t| !t.is_empty() && t.len() <= 128 && !t.chars().any(char::is_control));
    if role.agent.as_ref().is_some_and(|a| !short(&a.model) || !short(&a.effort)) {
        return Some("invalid-agent");
    }
    None
}

fn tidy(role: &mut DiscussionRole) {
    role.name = role.name.trim().to_string();
    role.perspective = role.perspective.trim().to_string();
    role.criteria = role.criteria.trim().to_string();
}

fn name_taken(roles: &[DiscussionRole], name: &str, except: Option<&str>) -> bool {
    roles.iter().any(|r| Some(r.id.as_str()) != except && r.name.trim() == name.trim())
}

/// 요청 하나를 목록에 적용한다. 어긋나면 목록을 건드리지 않고 이유 코드.
pub fn apply(roles: &mut Vec<DiscussionRole>, op: RoleOp) -> Result<(), &'static str> {
    match op {
        RoleOp::Add { mut role } => {
            tidy(&mut role);
            if let Some(problem) = role_problem(&role) {
                return Err(problem);
            }
            if roles.iter().any(|r| r.id == role.id) {
                return Err("duplicate-id");
            }
            if name_taken(roles, &role.name, None) {
                return Err("duplicate-name");
            }
            roles.push(role);
        }
        RoleOp::Edit { id, patch } => {
            let at = roles.iter().position(|r| r.id == id).ok_or("unknown-role")?;
            let mut role = roles[at].clone();
            if let Some(name) = patch.name { role.name = name; }
            if let Some(perspective) = patch.perspective { role.perspective = perspective; }
            if let Some(stance) = patch.stance { role.stance = stance; }
            if let Some(criteria) = patch.criteria { role.criteria = criteria; }
            if patch.clear_agent { role.agent = None; }
            if let Some(agent) = patch.agent { role.agent = Some(agent); }
            tidy(&mut role);
            if let Some(problem) = role_problem(&role) {
                return Err(problem);
            }
            if name_taken(roles, &role.name, Some(&id)) {
                return Err("duplicate-name");
            }
            roles[at] = role;
        }
        RoleOp::Delete { id } => {
            let at = roles.iter().position(|r| r.id == id).ok_or("unknown-role")?;
            roles.remove(at);
        }
    }
    Ok(())
}

fn read_request(path: &Path) -> Result<RoleRequest, &'static str> {
    let file = fs::File::open(path).map_err(|_| "unreadable")?;
    let mut bytes = vec![];
    file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes).map_err(|_| "unreadable")?;
    if bytes.len() > MAX_BYTES {
        return Err("too-large");
    }
    let request: RoleRequest = serde_json::from_slice(&bytes).map_err(|_| "invalid-json")?;
    if request.version > REQUEST_VERSION {
        return Err("unsupported-version");
    }
    Ok(request)
}

/// 아직 적용되지 않은 요청을 `roles`에 겹친 목록. 어긋나는 요청은 건너뛴다(앱도 거부한다).
pub fn with_pending(work_inbox: &Path, mut roles: Vec<DiscussionRole>) -> Vec<DiscussionRole> {
    let dir = inbox_dir(work_inbox);
    for name in request_names(&dir).unwrap_or_default() {
        if let Ok(request) = read_request(&dir.join(name)) {
            let _ = apply(&mut roles, request.op);
        }
    }
    roles
}

/// 수신함을 `roles.json`에 적용한다. 바뀐 게 있으면 새 목록, 없으면 None. 파일 이름(시각) 순.
/// `roles.json`이 없으면(프리셋 시드 전) 아무것도 하지 않는다. 저장에 실패하면 요청 파일을 그대로 둔다.
pub fn import(work_inbox: &Path, store: &RoleStore) -> Result<Option<RolesFile>, String> {
    let dir = inbox_dir(work_inbox);
    let names = request_names(&dir).map_err(|e| e.to_string())?;
    if names.is_empty() {
        return Ok(None);
    }
    let mut applied: Vec<PathBuf> = vec![];
    let mut rejected: Vec<(String, &'static str)> = vec![];
    let updated = store.update(|file| {
        for name in names.into_iter().take(MAX_BATCH) {
            let path = dir.join(&name);
            match read_request(&path).and_then(|r| apply(&mut file.roles, r.op)) {
                Ok(()) => applied.push(path),
                Err("unreadable") => {}
                Err(code) => rejected.push((name, code)),
            }
        }
        !applied.is_empty()
    })?;
    if updated.is_none() {
        return Ok(None);
    }
    for (name, code) in &rejected {
        if let Err(e) = reject(&dir, name, code, None) {
            eprintln!("agent-office: 역할 수신함 {name} 거부 실패({code}): {e}");
        }
    }
    for path in &applied {
        if let Err(e) = fs::remove_file(path).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }) {
            eprintln!("agent-office: 역할 수신함 {} 삭제 실패: {e}", path.display());
        }
    }
    Ok(updated.filter(|_| !applied.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::durable::write_atomic;
    use crate::types::WorkCli;

    fn role(id: &str, name: &str) -> DiscussionRole {
        DiscussionRole { id: id.into(), name: name.into(), perspective: String::new(), stance: DiscussionStance::Critical, criteria: String::new(), agent: None }
    }
    fn put(inbox: &Path, file: &str, op: RoleOp) {
        let request = RoleRequest { version: REQUEST_VERSION, op, requested_by: None };
        write_atomic(&inbox_dir(inbox).join(file), &serde_json::to_vec(&request).unwrap()).unwrap();
    }
    fn names(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = fs::read_dir(dir).map(|d| d.flatten().map(|e| e.file_name().into_string().unwrap()).collect()).unwrap_or_default();
        out.sort();
        out
    }

    #[test]
    fn apply_validates_names_ids_and_lengths_without_touching_the_list() {
        let mut roles = vec![role("r1", "비판자")];
        assert_eq!(apply(&mut roles, RoleOp::Add { role: role("r2", " 비판자 ") }), Err("duplicate-name"));
        assert_eq!(apply(&mut roles, RoleOp::Add { role: role("r1", "새 역할") }), Err("duplicate-id"));
        assert_eq!(apply(&mut roles, RoleOp::Add { role: role("bad id", "새 역할") }), Err("invalid-id"));
        assert_eq!(apply(&mut roles, RoleOp::Add { role: role("r2", "  ") }), Err("name"));
        assert_eq!(apply(&mut roles, RoleOp::Add { role: role("r2", "a\u{1b}b") }), Err("control-char"));
        let mut long = role("r2", "긴 역할");
        long.criteria = "가".repeat(MAX_TEXT_CHARS + 1);
        assert_eq!(apply(&mut roles, RoleOp::Add { role: long }), Err("too-long"));
        assert_eq!(roles, vec![role("r1", "비판자")]);
        assert_eq!(apply(&mut roles, RoleOp::Edit { id: "nope".into(), patch: RolePatch::default() }), Err("unknown-role"));
        assert_eq!(apply(&mut roles, RoleOp::Delete { id: "nope".into() }), Err("unknown-role"));
    }

    #[test]
    fn edit_changes_only_given_fields_and_can_clear_the_cli() {
        let mut first = role("r1", "비판자");
        first.perspective = "위험을 본다".into();
        first.agent = Some(WorkAgent { cli: WorkCli::Codex, model: None, effort: None });
        let mut roles = vec![first, role("r2", "사용자")];
        let patch = RolePatch { stance: Some(DiscussionStance::Divergent), clear_agent: true, ..Default::default() };
        apply(&mut roles, RoleOp::Edit { id: "r1".into(), patch }).unwrap();
        assert_eq!((roles[0].stance, roles[0].perspective.as_str(), roles[0].agent.clone()), (DiscussionStance::Divergent, "위험을 본다", None));
        // 자기 이름은 그대로 둬도 되지만 다른 역할 이름은 못 쓴다.
        let rename = |name: &str| RoleOp::Edit { id: "r1".into(), patch: RolePatch { name: Some(name.into()), ..Default::default() } };
        assert_eq!(apply(&mut roles, rename("비판자")), Ok(()));
        assert_eq!(apply(&mut roles, rename("사용자")), Err("duplicate-name"));
    }

    #[test]
    fn import_waits_for_the_seeded_file_then_applies_in_order_and_rejects_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let (inbox, store) = (tmp.path().join("work-inbox"), RoleStore::new(tmp.path().to_path_buf()));
        put(&inbox, "1-a.json", RoleOp::Add { role: role("r2", "새 역할") });
        put(&inbox, "2-b.json", RoleOp::Edit { id: "r2".into(), patch: RolePatch { criteria: Some(" 근거를 댄다 ".into()), ..Default::default() } });
        put(&inbox, "3-c.json", RoleOp::Add { role: role("r3", "비판자") });
        // 프리셋 시드 전에는 손대지 않는다.
        assert_eq!(import(&inbox, &store).unwrap(), None);
        assert_eq!(names(&inbox_dir(&inbox)).len(), 3);
        // CLI는 적용 전 요청을 겹쳐 본다.
        assert_eq!(with_pending(&inbox, vec![role("r1", "비판자")]).iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["r1", "r2"]);

        store.save(&RolesFile { version: 1, roles: vec![role("r1", "비판자")] }).unwrap();
        let file = import(&inbox, &store).unwrap().expect("changed");
        assert_eq!(file.roles.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["r1", "r2"]);
        assert_eq!(file.roles[1].criteria, "근거를 댄다");
        assert_eq!(store.load().unwrap(), Some(file));
        // 적용한 요청은 지우고, 이름이 겹친 요청은 이유와 함께 rejected/로 옮긴다.
        assert_eq!(names(&inbox_dir(&inbox)), ["rejected"]);
        assert_eq!(names(&inbox_dir(&inbox).join("rejected")), ["3-c.json", "3-c.json.error"]);
        assert_eq!(import(&inbox, &store).unwrap(), None);
    }

    #[test]
    fn a_batch_of_only_rejected_requests_leaves_the_file_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let (inbox, store) = (tmp.path().join("work-inbox"), RoleStore::new(tmp.path().to_path_buf()));
        store.save(&RolesFile { version: 1, roles: vec![role("r1", "비판자")] }).unwrap();
        put(&inbox, "1-a.json", RoleOp::Delete { id: "nope".into() });
        assert_eq!(import(&inbox, &store).unwrap(), None);
        assert_eq!(names(&inbox_dir(&inbox).join("rejected")), ["1-a.json", "1-a.json.error"]);
    }
}
