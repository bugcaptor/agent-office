//! 모든 에이전트 세션에 덧붙이는 사무실 공통 안내문과 그 배선 조각
//! (docs/work-panels-design.md §업무 수신함). CLI별 스킬 대신 짧은 안내문
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
- The command prints the new task id. Tasks can depend on others: `--after <id>` (repeatable) makes a task wait until those prerequisites are done. When the user asks to plan or split work, structure it as ordered steps: capture each id, e.g. `A=$({cli} "Step 1" - <<'EOF' … EOF)`, then `{cli} --after "$A" "Step 2" …`.
- `{cli} --list` shows open tasks on the board with their ids, status, assignee and prerequisites; use it to link a new task to an existing one.
- When the user tells you to mark a task as done (or confirms it is finished), run `{cli} --done <id>`. Completing a task unblocks the tasks that wait on it. Only do this on the user's instruction, never on your own judgment that the work is finished.
- After leaving tasks, tell the user in one line."#
    )
}

/// persona와 안내문 중 있는 것만 빈 줄로 잇는다. 둘 다 없으면 None.
pub fn appended_prompt(persona: Option<&str>, office: Option<&str>) -> Option<String> {
    let parts: Vec<&str> = [persona, office]
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
/// `config.toml`의 최상위 값을 읽어 수신함을 덧붙인 배열을 만든다. 설정이 없거나
/// 깨졌으면 수신함만 넣는다. 사용자 설정 파일에는 쓰지 않는다.
pub fn codex_writable_roots_config(user_config: Option<&str>, inbox: &Path) -> String {
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
    let inbox = inbox.to_string_lossy().into_owned();
    if !roots.contains(&inbox) {
        roots.push(inbox);
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
    fn instructions_embed_the_absolute_shim_path() {
        let text = task_instructions(Path::new("/tmp/agent-office/bin/office-task"));
        assert!(text.contains("/tmp/agent-office/bin/office-task \"<title>\" - <<'EOF'"));
        assert!(text.contains("--to"));
        assert!(text.contains("--after"));
        assert!(text.contains("/tmp/agent-office/bin/office-task --list"));
        assert!(text.contains("/tmp/agent-office/bin/office-task --done <id>"));
    }

    #[test]
    fn appended_prompt_joins_only_present_parts() {
        assert_eq!(appended_prompt(None, None), None);
        assert_eq!(appended_prompt(Some(" \n "), None), None);
        assert_eq!(appended_prompt(Some("persona"), None).as_deref(), Some("persona"));
        assert_eq!(appended_prompt(None, Some("office")).as_deref(), Some("office"));
        assert_eq!(appended_prompt(Some("persona\n"), Some("office")).as_deref(), Some("persona\n\noffice"));
    }

    #[test]
    fn permissions_fragment_lists_each_enabled_shim() {
        assert_eq!(permissions_fragment(&[]), None);
        let task = PathBuf::from("/tmp/agent-office/bin/office-task");
        let other = PathBuf::from("/tmp/agent-office/bin/office-other");
        let f = permissions_fragment(&[task.clone()]).unwrap();
        assert_eq!(f["permissions"]["allow"], serde_json::json!(["Bash(/tmp/agent-office/bin/office-task:*)"]));
        let f = permissions_fragment(&[task, other]).unwrap();
        assert_eq!(f["permissions"]["allow"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn writable_roots_merge_user_roots_and_tolerate_broken_config() {
        let inbox = Path::new("/data/Agent Office/work-inbox");
        assert_eq!(
            codex_writable_roots_config(None, inbox),
            r#"sandbox_workspace_write.writable_roots=["/data/Agent Office/work-inbox"]"#
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
