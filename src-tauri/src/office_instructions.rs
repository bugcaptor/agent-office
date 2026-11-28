//! 모든 에이전트 세션에 덧붙이는 사무실 공통 안내문과 그 배선 조각.
//! CLI별 스킬 대신 짧은 안내문
//! 하나를 persona 뒤에 붙여 시스템 프롬프트에 **덧붙인다**.

use std::path::{Path, PathBuf};

/// 세션에 실을 사무실 기능. 켜진 것만 `Some(shim 경로)`.
#[derive(Default, Clone, Copy)]
pub struct Guide<'a> {
    pub task: Option<&'a Path>,
    pub md: Option<&'a Path>,
    pub file: Option<&'a Path>,
    pub peer: Option<&'a Path>,
    pub help: Option<&'a Path>,
}

/// 모든 에이전트 세션에 덧붙이는 사무실 안내 하나. 모델이 읽는 지시문이라 화면 문자열(i18n)이 아니다.
/// "하고 싶은 일 → 명령" 목차와 꼭 지킬 규칙만 싣고, 옵션은 각 명령의 `--help`, 지금 켜진 기능·머신·캐릭터
/// 상태는 `office-help`가 실행 시점에 보여 준다(세션을 띄운 뒤 바뀐 것도 거기서 보인다). 켜진 기능이 없으면 None.
pub fn guide(g: &Guide) -> Option<String> {
    if g.task.is_none() && g.md.is_none() && g.file.is_none() && g.peer.is_none() {
        return None;
    }
    let p = |path: &Path| path.display().to_string();
    let mut out = String::from("# Agent Office\nThis session runs inside Agent Office. Run these commands by their full paths:\n\n");
    for (name, path) in [("office-task", g.task), ("office-peer", g.peer), ("office-file", g.file), ("office-md", g.md), ("office-help", g.help)] {
        if let Some(path) = path {
            out.push_str(&format!("- `{name}` = `{}`\n", p(path)));
        }
    }
    out.push_str("\n`<command> --help` lists every option.");
    if g.help.is_some() {
        out.push_str(" `office-help` shows what is turned on right now, the linked machines and each character's state; check it when this guide does not cover what you need.");
    }
    out.push_str("\n\n## What you want → command\n");
    if g.peer.is_some() {
        out.push_str("- Have another character (on this machine or a linked one) do work now, or ask it something → `office-peer ask <character> \"<message>\"`. It works even when that character has no session or no agent CLI running: Agent Office opens its terminal, starts its CLI and gives your message as the first prompt. Write the message so it stands alone (the other agent does not see your conversation), and add `--no-wait` when the work takes longer than a quick answer.\n");
    }
    if g.task.is_some() {
        out.push_str("- Leave a follow-up for later, split work into a plan, or schedule a run → `office-task` (the shared task board).\n");
    }
    if g.file.is_some() {
        out.push_str(&format!(
            "- Hand files or folders to another machine or character → `office-file send <target> <path>...`. Files only: {}\n",
            if g.peer.is_some() { "to give work, use `office-peer ask` and name the paths in the message." } else { "it does not give anyone work." }
        ));
    }
    if g.md.is_some() {
        out.push_str("- Show a Markdown document in the browser → `office-md <file.md>` (`--latest` for the newest `.md` here). Only when the user asks.\n");
    }
    if g.task.is_some() {
        out.push_str("- A team discussion between colleagues → `office-task --discuss` (see `office-task --discuss --help`). Only when the user asks.\n");
    }
    if g.task.is_some() {
        out.push_str(
            r#"
## Task board rules
- Leave a task when the user asks you to note something for later, or for follow-up work outside the current scope; not for work you finish in this turn. One follow-up, one task, written so someone without this conversation can act on it. It is yours unless you pass `--to <colleague>`. Tell the user in one line after leaving tasks.
- Plans: make each piece of work its own step task, chained with `--after <id>` (each create prints its id), then create the root last with `--plan --after <last step>`; its body is the plan memo and it holds no work.
- `--done` and `--delete` only on the user's instruction. Schedules (`--at`, `--every`) you set start turned off; tell the user to turn them on in the task's Schedule section.
- When an auto-run prompt asks for a report, write it with `office-task --report <id> -` before your turn ends.
- In a session running a task sent from the host, task commands go to the host's board; elsewhere use `--host` only when the user asks.
- A discussion you create waits in the ready state: tell the user to start it from its card; its decision memo is saved as `$AGENT_OFFICE_APP_DATA/discussions/memos/<id>.md`.
"#,
        );
    }
    if g.peer.is_some() {
        out.push_str(
            r#"
## Agent talk rules
- A message starting with `[Agent Office · message <id> from <character>@<machine>]` comes from another agent, not the user. Answer it once with `office-peer reply <id> "<answer>"`, even if only to say you cannot help; the sender is waiting. Do not ask the sender back while it waits.
- `ask` waits 100 s for the reply by default (`--wait` up to 600; raise your shell timeout to match). Without a reply in time it exits with code 8 and the reply is put into this session when it arrives.
- `office-peer agents` lists every character and its state; `status`, `log`, `threads` show more. You may `ask`, `launch` and `resume` on your own judgment; use `interrupt` and `unqueue` on another character only when the user or the task asks. Hourly and depth limits apply.
"#,
        );
    }
    if g.md.is_some() {
        out.push_str("\n## Document viewer\nWith no file named, open the Markdown document you last created or edited here, else `office-md --latest`. Tell the user in one line which document opened; if it fails, give them the printed HTML path or the error.\n");
    }
    Some(out.trim_end().to_string())
}

/// persona와 안내문들 중 있는 것만 빈 줄로 잇는다. 모두 없으면 None.
pub fn appended_prompt(persona: Option<&str>, office: Option<&str>, doc_view: Option<&str>) -> Option<String> {
    let parts: Vec<&str> = [persona, office, doc_view]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// Claude 세션 `--settings`에 얹을 권한 조각. 켜진 기능의 shim 실행만 사전 승인한다.
pub fn permissions_fragment(shims: &[PathBuf]) -> Option<serde_json::Value> {
    if shims.is_empty() {
        return None;
    }
    let allow: Vec<String> = shims
        .iter()
        .map(|shim| format!("Bash({}:*)", shim.to_string_lossy()))
        .collect();
    Some(serde_json::json!({ "permissions": { "allow": allow } }))
}

/// 태스크 대화 세션의 `--settings` 조각. 훅은 없고 shim 실행과
/// `read_dirs`(세션 로그·작업자 보고 폴더) 아래 읽기만 사전 승인한다. 쓰기 도구는 넣지 않는다.
pub fn task_chat_permissions_fragment(shims: &[PathBuf], read_dirs: &[PathBuf]) -> Option<serde_json::Value> {
    let allow: Vec<String> = shims
        .iter()
        .map(|shim| format!("Bash({}:*)", shim.to_string_lossy()))
        .chain(read_dirs.iter().map(|dir| format!("Read({}/**)", claude_absolute_path(dir))))
        .collect();
    (!allow.is_empty()).then(|| serde_json::json!({ "permissions": { "allow": allow } }))
}

/// Claude 권한 규칙의 절대 경로 표기. 규칙의 `/x`는 설정 파일 기준 상대 경로라 절대 경로는
/// `//x`로 쓴다. Windows 경로는 Claude가 POSIX 꼴(`C:\a` → `/c/a`)로 바꿔 맞추므로 그 꼴로 쓴다.
pub fn claude_absolute_path(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.trim_end_matches('/');
    let bytes = text.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        return format!("//{drive}/{}", text[2..].trim_start_matches('/'));
    }
    format!("/{}", text)
}

/// TOML 기본 문자열(큰따옴표) 하나로 감싼다. 제어 문자는 버리고 줄바꿈·탭은 이스케이프한다.
pub fn toml_basic_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Codex `-c` 인자 하나: `sandbox_workspace_write.writable_roots=[…]`.
/// 이 `-c`는 사용자 설정의 같은 배열을 **덮어쓰므로**(codex-cli 0.162 실측) 사용자
/// `config.toml`의 최상위 값을 읽어 `extra`(업무 수신함·문서 뷰어 캐시)를 덧붙인 배열을
/// 만든다. 설정이 없거나 깨졌으면 `extra`만 넣는다. 사용자 설정 파일에는 쓰지 않는다.
pub fn codex_writable_roots_config(user_config: Option<&str>, extra: &[&Path]) -> String {
    let mut roots: Vec<String> = user_config
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|table| {
            table
                .get("sandbox_workspace_write")?
                .get("writable_roots")?
                .as_array()
                .map(|roots| roots.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        })
        .unwrap_or_default();
    for root in extra {
        let root = root.to_string_lossy().into_owned();
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    let roots: Vec<String> = roots.iter().map(|root| toml_basic_string(root)).collect();
    format!("sandbox_workspace_write.writable_roots=[{}]", roots.join(","))
}

/// 관찰(observer)이 꺼져 있어 훅 설정 파일이 없는 세션을 위한 **권한 조각 전용**
/// 세션 설정 파일(업무 수신함 shim 승인). 관찰이 켜져 있으면 훅 설정 파일에 같은
/// 조각이 합쳐지므로(`ClaudeAdapter::with_extra_settings`) 이 경로는 쓰이지 않는다.
/// 파일명이 `.settings.json`으로 끝나 기존 GC가 그대로 청소한다.
pub fn write_permissions_only_settings(
    dir: &Path,
    session_id: &str,
    fragment: &serde_json::Value,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{session_id}.settings.json"));
    let body = serde_json::to_vec_pretty(fragment).expect("settings fragment serializes");
    write_atomic(&path, &body)?;
    Ok(path)
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("aotmp");
    std::fs::write(&tmp, bytes)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(unix)]
pub(crate) fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
}
#[cfg(not(unix))]
pub(crate) fn set_executable(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_chat_fragment_allows_shims_and_reads_without_writes() {
        let fragment = task_chat_permissions_fragment(
            &[PathBuf::from("/tmp/ao/bin/office-task")],
            &[PathBuf::from("/data/session-logs/v1"), PathBuf::from("/data/work-inbox/reports/")],
        )
        .unwrap();
        assert_eq!(
            fragment,
            serde_json::json!({ "permissions": { "allow": [
                "Bash(/tmp/ao/bin/office-task:*)",
                "Read(//data/session-logs/v1/**)",
                "Read(//data/work-inbox/reports/**)",
            ] } })
        );
        assert!(fragment.get("hooks").is_none());
        assert_eq!(task_chat_permissions_fragment(&[], &[]), None);
    }

    #[test]
    fn claude_absolute_path_uses_double_slash_and_posix_drive_form() {
        assert_eq!(claude_absolute_path(Path::new("/Users/me/Library/App Support/x")), "//Users/me/Library/App Support/x");
        assert_eq!(claude_absolute_path(Path::new("C:\\Users\\me\\AppData\\x")), "//c/Users/me/AppData/x");
        assert_eq!(claude_absolute_path(Path::new("D:/data/")), "//d/data");
    }

    #[test]
    fn guide_lists_only_enabled_commands_with_full_paths() {
        let (task, md, file, peer, help) = (
            Path::new("/tmp/ao/bin/office-task"),
            Path::new("/tmp/ao/bin/office-md"),
            Path::new("/tmp/ao/bin/office-file"),
            Path::new("/tmp/ao/bin/office-peer"),
            Path::new("/tmp/ao/bin/office-help"),
        );
        assert_eq!(guide(&Guide::default()), None);
        assert_eq!(guide(&Guide { help: Some(help), ..Guide::default() }), None, "도움말만으로는 싣지 않는다");

        let all = guide(&Guide { task: Some(task), md: Some(md), file: Some(file), peer: Some(peer), help: Some(help) }).unwrap();
        assert!(all.starts_with("# Agent Office\n"));
        for line in ["`office-task` = `/tmp/ao/bin/office-task`", "`office-peer` = `/tmp/ao/bin/office-peer`", "`office-file` = `/tmp/ao/bin/office-file`", "`office-md` = `/tmp/ao/bin/office-md`", "`office-help` = `/tmp/ao/bin/office-help`"] {
            assert!(all.contains(line), "{line}");
        }
        // 일 맡기기는 peer, 파일 보내기는 일 지시가 아님을 목차에서 못박는다.
        assert!(all.contains("even when that character has no session"));
        assert!(all.contains("to give work, use `office-peer ask`"));
        assert!(all.contains("office-peer reply <id>"));
        assert!(all.contains("`--done` and `--delete` only on the user's instruction"));
        assert!(all.contains("--plan --after <last step>"));
        assert!(all.contains("discussions/memos/<id>.md"));
        assert!(all.contains("Only when the user asks"));

        // 문서 뷰어만 켜진 세션: 다른 기능 이름이 나오지 않는다.
        let md_only = guide(&Guide { md: Some(md), help: Some(help), ..Guide::default() }).unwrap();
        assert!(md_only.contains("office-md <file.md>"));
        for absent in ["office-task", "office-peer", "office-file", "Task board rules", "Agent talk rules"] {
            assert!(!md_only.contains(absent), "{absent}");
        }
        // peer가 꺼지면 파일 보내기가 일 지시 통로가 아니라고만 말한다.
        let no_peer = guide(&Guide { file: Some(file), md: Some(md), ..Guide::default() }).unwrap();
        assert!(no_peer.contains("it does not give anyone work") && !no_peer.contains("office-help"));
    }

    #[test]
    fn appended_prompt_joins_only_present_parts() {
        assert_eq!(appended_prompt(None, None, None), None);
        assert_eq!(appended_prompt(Some(" \n "), None, None), None);
        assert_eq!(appended_prompt(Some("persona"), None, None).as_deref(), Some("persona"));
        assert_eq!(appended_prompt(None, Some("office"), None).as_deref(), Some("office"));
        assert_eq!(appended_prompt(Some("persona\n"), Some("office"), None).as_deref(), Some("persona\n\noffice"));
        assert_eq!(appended_prompt(Some("persona"), None, Some("docs")).as_deref(), Some("persona\n\ndocs"));
        assert_eq!(appended_prompt(Some("p"), Some("o"), Some("d")).as_deref(), Some("p\n\no\n\nd"));
    }

    #[test]
    fn permissions_fragment_lists_each_enabled_shim() {
        assert_eq!(permissions_fragment(&[]), None);
        let task = PathBuf::from("/tmp/agent-office/bin/office-task");
        let md = PathBuf::from("/tmp/agent-office/bin/office-md");
        let f = permissions_fragment(&[task.clone()]).unwrap();
        assert_eq!(f["permissions"]["allow"], serde_json::json!(["Bash(/tmp/agent-office/bin/office-task:*)"]));
        let f = permissions_fragment(&[task, md]).unwrap();
        assert_eq!(f["permissions"]["allow"][1], "Bash(/tmp/agent-office/bin/office-md:*)");
    }

    #[test]
    fn writable_roots_merge_user_roots_and_tolerate_broken_config() {
        let inbox: &[&Path] = &[Path::new("/data/Agent Office/work-inbox")];
        assert_eq!(
            codex_writable_roots_config(None, inbox),
            r#"sandbox_workspace_write.writable_roots=["/data/Agent Office/work-inbox"]"#
        );
        let both: &[&Path] = &[Path::new("/data/work-inbox"), Path::new("/data/doc-view")];
        assert_eq!(
            codex_writable_roots_config(Some("[sandbox_workspace_write]\nwritable_roots = [\"/data/doc-view\"]\n"), both),
            r#"sandbox_workspace_write.writable_roots=["/data/doc-view","/data/work-inbox"]"#
        );
        let user = "sandbox_mode = \"workspace-write\"\n[sandbox_workspace_write]\nwritable_roots = [\"/a\", \"/b \\\"q\\\"\"]\n";
        let merged = codex_writable_roots_config(Some(user), inbox);
        assert_eq!(
            merged,
            r#"sandbox_workspace_write.writable_roots=["/a","/b \"q\"","/data/Agent Office/work-inbox"]"#
        );
        // 결과는 codex가 TOML로 읽는 값이어야 한다.
        let value = merged.split_once('=').unwrap().1;
        let parsed: toml::Table = format!("v = {value}").parse().unwrap();
        assert_eq!(parsed["v"].as_array().unwrap()[1].as_str(), Some("/b \"q\""));
        assert_eq!(codex_writable_roots_config(Some("not = [toml"), inbox), codex_writable_roots_config(None, inbox));
        let already = "[sandbox_workspace_write]\nwritable_roots = [\"/data/Agent Office/work-inbox\"]\n";
        assert_eq!(codex_writable_roots_config(Some(already), inbox), codex_writable_roots_config(None, inbox));
    }
}
