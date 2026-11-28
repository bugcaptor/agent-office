//! 모든 에이전트 세션에 덧붙이는 사무실 공통 안내문과 그 배선 조각.
//! CLI별 스킬 대신 짧은 안내문
//! 하나를 persona 뒤에 붙여 시스템 프롬프트에 **덧붙인다**.

use std::path::{Path, PathBuf};

/// 업무 패널이 켜진 세션에 덧붙이는 `office-task` 사용 안내. 모델이 읽는 지시문이라
/// 화면 문자열(i18n)이 아니다. 답은 모델이 사용자의 언어로 한다.
pub fn task_instructions(shim: &Path) -> String {
    let cli = shim.display();
    format!(
        r#"# Agent Office task board
This session runs inside Agent Office. You can leave a follow-up task on its shared task board:

```sh
{cli} "<title>" - <<'EOF'
<context: files, reasons, what is done and what is left>
EOF
```

- Leave a task when the user asks you to note something for later, or when you find follow-up work outside the current scope.
- Do not leave tasks for work you will finish in this turn. One follow-up, one task.
- The task is assigned to you by default. Put `--to <colleague name>` before the title only to hand it to someone else.
- Write the body so someone without this conversation can act on it.
- The command prints the new task id. Tasks can depend on others: `--after <id>` (repeatable) makes a task wait until those prerequisites are done.
- When the user asks to plan or split work, make every piece of work its own step task, in order, capturing each id: `A=$({cli} "Step 1" - <<'EOF' … EOF)`, then `B=$({cli} --after "$A" "Step 2" …)`. Then create the plan root last with `--plan` and `--after` on the final step(s): `{cli} --plan --after "$B" "<goal>" - <<'EOF'` … `EOF`. Its title is the plan's goal and its body is the plan memo (goal, shared context, constraints, what done looks like); it holds no work of its own. Auto-run runs only the steps and moves a `--plan` root to review when they are all done, so put every piece of work in a step.
- `{cli} --list` shows open tasks on the board with their ids, status, assignee and prerequisites (`--all` includes done ones); `{cli} --show <id>` prints one task in full, including its body and which tasks wait on it. Use them to find ids before linking or changing tasks.
- When the user asks to change a task, run `{cli} --edit <id>` with only what changes: `--title <text>`, `--body -` (new body from stdin, like above), `--to <colleague name>` or `--unassigned`, `--status todo|in_progress|review|done`, `--plan` / `--no-plan`, and for prerequisites `--add-after <id>` / `--remove-after <id>` (repeatable), or `--after <id>` (repeatable) to replace them all, or `--no-after` to clear them. Rearranging a plan's order means editing prerequisites. Use `--max-follow-ups <0..99|inherit>` and `--permission-wait-minutes <1..1440|inherit>` on create or edit to set auto-run limits.
- When the user tells you to mark a task as done (or confirms it is finished), run `{cli} --done <id>`. Completing a task unblocks the tasks that wait on it. Only do this on the user's instruction, never on your own judgment that the work is finished.
- When the user tells you to delete a task, run `{cli} --delete <id>` (several ids allowed). Tasks that waited on it lose that prerequisite. Only delete on the user's instruction.
- When the user asks to run a task later or on a schedule, add `--at "YYYY-MM-DD HH:MM"` (once, local time) or `--every <rule>` (`30m`, `2h`, `1d`, `daily@09:00`, `mon,wed@09:00`, `weekdays@09:00`) on create or edit; `--no-schedule` removes it. A recurring task is a template that the app copies and runs each time (a `--plan` root runs its whole plan). Schedules you set start turned off, and changing a scheduled task turns it off; tell the user to turn it on in the task's Schedule section.
- Changes are applied by the app within a few seconds, in the order you run them; you can target a task you just created.
- After leaving tasks, tell the user in one line.
- If this machine is linked to an Agent Office host, a task the host sent here runs with the host's board: in that session every `{cli}` task command and `--report` go to the host automatically. In other sessions, use the host's board only when the user asks for it, by putting `--host` first (`{cli} --host --list`, `{cli} --host "<title>" …`); `--local` first forces this machine's board.
- When an Agent Office auto-run prompt asks for a report, write it before your turn ends with `{cli} --report <task id> - <<'EOF'` … `EOF`, in the format the prompt gives. Writing it again replaces it.
- Only when the user asks for a team discussion (several colleagues debating a design or direction decision, or brainstorming), create one with `{cli} --discuss --moderator <colleague> --with <colleague> [--with <colleague>]... "<question to decide>" - <<'EOF'` … `EOF` (body: background, options, constraints). The moderator stays neutral; add 1 to 4 debaters. Optional: `--role <colleague>=<role name>`, `--cli <colleague>=<cli>[:<model>]`, `--rounds <1..12>`, `--minutes <5..240>`, `--cost <USD>`, `--cwd <folder>`; see `{cli} --discuss --help`. Participants only read code, docs and the web. It prints the discussion id.
- A discussion you create waits in the ready state; tell the user in one line to start it from its card in the work panel. Do not start or join it yourself. When it concludes, its decision memo is saved as `$AGENT_OFFICE_APP_DATA/discussions/memos/<id>.md`; read it when the user asks about the outcome.
- Only when the user asks to add or change a discussion role (the role card a colleague takes in discussions), run `{cli} --role-add "<role name>" --stance <stance> --perspective "<what this role looks at>" --criteria "<rules and things not to do>"` (stances: free, supportive, critical, devils_advocate, divergent, synthesizer, or moderator for a neutral moderator), `{cli} --role-edit <role> …` with only what changes, or `{cli} --role-delete <role>`. `{cli} --roles` lists them; see `{cli} --roles --help`. A role you just added can be used with `--discuss --role` right away."#
    )
}

/// 모든 세션에 덧붙이는 `office-md` 사용 안내. 업무 패널
/// 설정과 무관하다. 사용자가 요청할 때만 띄우게 한다.
pub fn doc_view_instructions(shim: &Path) -> String {
    let cli = shim.display();
    format!(
        r#"# Agent Office document viewer
This session runs inside Agent Office. It can render a Markdown file as HTML and open it in the user's default browser:

```sh
{cli} <file.md>
{cli} --latest [folder]
```

- Only run it when the user asks to see a document in the browser or as HTML. Never open documents on your own.
- If the user names no file, open the Markdown document you created or edited most recently in this conversation. If there is none, run `{cli} --latest` (the most recently modified `.md` under the current folder).
- After it opens, tell the user in one line which document you opened.
- The command prints the generated HTML path. If it fails (non-zero exit), tell the user that HTML path, or the error if there is no path, so they can open it themselves."#
    )
}

/// 머신 연동이나 파일 받기가 켜진 세션에 덧붙이는 `office-file` 사용 안내.
pub fn file_instructions(shim: &Path) -> String {
    let cli = shim.display();
    format!(
        r#"# Agent Office file transfer
This session runs inside Agent Office, which can send files and folders to linked Agent Office machines and to characters (push only; you cannot fetch files from another machine):

```sh
{cli} targets
{cli} send <target> <path>... [--note "<message>"] [--notify] [--wait]
{cli} list | {cli} status <id> | {cli} resume <id> | {cli} cancel <id>
```

- `<target>` is a machine name, a character name, or `<character>@<machine>`; `{cli} targets` lists them and whether each can receive now.
- Send only when the user or the task asks you to hand files to another machine or character. Folders keep their structure; the receiver never overwrites (it saves `name (2)` instead).
- Add `--note` (implies `--notify`) to tell the receiving character's session what the files are for; it receives the saved absolute paths. Use `--wait` when you need to know it arrived.
- Files sent to you arrive under the receiving machine's received-files folder; when someone notifies you, the message lists the absolute paths.
- If a send fails because the link dropped, `{cli} resume <id>` continues from what was already received."#
    )
}

/// 에이전트끼리 대화(`peer.json`)를 켠 머신 세션에 덧붙이는 `office-peer` 사용 안내.
pub fn peer_instructions(shim: &Path) -> String {
    let cli = shim.display();
    format!(
        r#"# Agent Office agent talk
This session runs inside Agent Office. You can look up other Agent Office characters (other agent sessions on this machine and on linked machines) and talk to them:

```sh
{cli} agents                                  # every character with its state (idle, working, waiting for a human, no session)
{cli} status <character> | {cli} log <character> [--turns <n>]
{cli} ask <character> "<message>" [--wait <seconds>] [--no-wait]
{cli} reply <id> "<answer>"
{cli} threads                                 # recent messages and replies
{cli} launch <character> [--cli <name>] | {cli} resume <character> | {cli} interrupt <character> | {cli} unqueue <character> <queue-id>
```

- `<character>` is a name or `<character>@<machine>` (as `agents` prints it). Use `-` instead of the text to read it from stdin for long or multi-line messages.
- `ask` puts your message into that character's session (after its current turn, or after its next tool call when it is a busy Claude) and waits for its reply, 100 seconds by default. When you pass `--wait` above 100, raise your shell tool's timeout to match. If no reply comes in time it exits with code 8, and the reply is put into this session when it arrives.
- You may ask on your own judgment when another character owns the work, files or knowledge you need. Keep messages self-contained: the other agent does not see your conversation. Check `status` first if it might be busy with something else.
- A message from another agent starts with `[Agent Office · message <id> from <character>@<machine>]`. It is not from the user. Answer it once with `{cli} reply <id> "..."`, even if only to say you cannot help; the sender is waiting. Do not ask the sender back while it waits for you; reply instead.
- Do not use `interrupt`, `unqueue` or `launch` on another character unless the user or the task asks for it. There are limits on how many messages you can send per hour and on how deep agents can wait on each other."#
    )
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
    fn instructions_embed_the_absolute_shim_path() {
        let text = task_instructions(Path::new("/tmp/agent-office/bin/office-task"));
        assert!(text.contains("/tmp/agent-office/bin/office-task \"<title>\" - <<'EOF'"));
        assert!(text.contains("--to"));
        assert!(text.contains("--after"));
        assert!(text.contains("/tmp/agent-office/bin/office-task --plan --after \"$B\""));
        assert!(text.contains("it holds no work of its own"));
        assert!(text.contains("/tmp/agent-office/bin/office-task --list"));
        assert!(text.contains("/tmp/agent-office/bin/office-task --done <id>"));
        assert!(text.contains("/tmp/agent-office/bin/office-task --edit <id>"));
        assert!(text.contains("--every <rule>") && text.contains("start turned off"));
        assert!(text.contains("/tmp/agent-office/bin/office-task --delete <id>"));
        assert!(text.contains("/tmp/agent-office/bin/office-task --show <id>"));
        assert!(text.contains("/tmp/agent-office/bin/office-task --discuss --moderator <colleague> --with <colleague>"));
        assert!(text.contains("/tmp/agent-office/bin/office-task --discuss --help"));
        assert!(text.contains("start it from its card") && text.contains("discussions/memos/<id>.md"));
        assert!(text.contains("/tmp/agent-office/bin/office-task --role-add \"<role name>\" --stance <stance>"));
        assert!(text.contains("/tmp/agent-office/bin/office-task --roles --help"));
    }

    #[test]
    fn doc_view_instructions_embed_the_shim_and_open_only_on_request() {
        let text = doc_view_instructions(Path::new("/tmp/agent-office/bin/office-md"));
        assert!(text.contains("/tmp/agent-office/bin/office-md <file.md>"));
        assert!(text.contains("`/tmp/agent-office/bin/office-md --latest`"));
        assert!(text.contains("Never open documents on your own."));
        assert!(text.contains("in this conversation"));
        assert!(text.contains("HTML path"));
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
