// src-tauri/src/notification/hook_settings.rs
//
// Writes per-session Claude Code `--settings` JSON files. Each PTY session
// gets a settings file whose Notification/Stop hooks POST (via curl) to the
// local axum HookServer at 127.0.0.1:<port>/hook.
//
// Also writes UserPromptSubmit/PostToolUse hooks — these are the session
// time tracking feature's *activity* hooks, not notifications:
// same curl-POST pattern, distinguished from Notification/Stop by the
// `source=prompt|tool` query value the hook server routes on. `PostToolUse`
// uses the same empty-string matcher as Notification/Stop, which for that
// event means "match every tool" — intended, since the reducer needs a
// heartbeat on every tool call to keep the open turn's `working` phase
// alive, not just a subset of tools.
//
// Schema verified 2026-07-06 against https://code.claude.com/docs/en/hooks:
// top-level `hooks` object; each event name maps to an array of
// `{ matcher, hooks: [...] }` groups; `matcher` is optional/ignored for
// `Notification` and `Stop` (they always fire on every occurrence), so the
// empty-string matcher used below is harmless. Command hook entries use
// `{ "type": "command", "command": "..." }`, matching this module exactly.
//
// The emitted `command` string is two different shell dialects depending on
// host OS (`curl_command` below), because Claude Code hook commands do not
// run under a guaranteed POSIX sh on Windows:
//   - unix: `curl -sS -m 2 -X POST '<url>' -H 'Content-Type: application/json'
//     --data-binary @- || true` — single-quoted, `|| true` swallows failures.
//     Verified working on macOS; left byte-for-byte unchanged.
//   - windows: `curl.exe -sS -m 2 -X POST "<url>" -H "Content-Type:
//     application/json" --data-binary @-` — `curl.exe` (not `curl`) so
//     PowerShell's `Invoke-WebRequest` alias can never intercept it; double
//     quotes (not single) because native Windows hook commands are not
//     guaranteed to run under sh, and under cmd.exe single quotes are literal
//     while the `&` in `?session=...&source=...` is a command separator —
//     double quotes protect it there and are also valid in PowerShell and sh.
//     No `|| true` / no chaining operator at all: none is portable across
//     cmd.exe + PowerShell + sh. Consequence: if the app's hook server is
//     down, the hook exits non-zero and Claude Code prints a one-line "hook
//     error" — accepted tradeoff for portability.

use std::fs;
use std::path::PathBuf;

use serde_json::json;

/// Pure, host-testable curl command builder — see the module header comment
/// for why the two variants differ. `HookSettingsWriter::curl` selects the
/// variant via `cfg!(windows)`; tests exercise both variants directly on any
/// host.
fn curl_command(windows: bool, port: u16, session_id: &str, source: &str) -> String {
    // sessionId is a URL-safe uuid v4, so no percent-encoding needed.
    let url = format!("http://127.0.0.1:{port}/hook?session={session_id}&source={source}");
    if windows {
        format!("curl.exe -sS -m 2 -X POST \"{url}\" -H \"Content-Type: application/json\" --data-binary @-")
    } else {
        format!("curl -sS -m 2 -X POST '{url}' -H 'Content-Type: application/json' --data-binary @- || true")
    }
}

#[derive(Clone)]
pub struct HookSettingsWriter {
    base_dir: PathBuf, // <temp>/agent-office/hooks
}

impl HookSettingsWriter {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn curl(port: u16, session_id: &str, source: &str) -> String {
        curl_command(cfg!(windows), port, session_id, source)
    }

    /// Notification/Stop 알림 훅 + UserPromptSubmit/PostToolUse activity 훅.
    /// activity 훅은 시간 추적용 신호 — 알림과 동일한 curl 패턴,
    /// source 쿼리값만 prompt/tool로 다르다.
    pub fn build(&self, session_id: &str, port: u16) -> serde_json::Value {
        let entry = |source: &str| {
            json!([{
                "matcher": "",
                "hooks": [{ "type": "command", "command": Self::curl(port, session_id, source) }]
            }])
        };
        json!({ "hooks": {
            "Notification": entry("hook"),
            "Stop": entry("stop"),
            "UserPromptSubmit": entry("prompt"),
            "PostToolUse": entry("tool"),
        } })
    }

    pub fn write(&self, session_id: &str, port: u16) -> std::io::Result<PathBuf> {
        fs::create_dir_all(&self.base_dir)?;
        let p = self.path_for(session_id);
        fs::write(
            &p,
            serde_json::to_vec_pretty(&self.build(session_id, port))?,
        )?;
        Ok(p)
    }

    pub fn cleanup(&self, session_id: &str) {
        let _ = fs::remove_file(self.path_for(session_id));
    }

    fn path_for(&self, session_id: &str) -> PathBuf {
        self.base_dir.join(format!("{session_id}.settings.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // Build a unique scratch dir under the OS temp dir without adding a
    // tempfile dependency (uuid is already a normal dependency, so it's
    // available in tests too).
    fn scratch_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("agent-office-test-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn build_produces_notification_and_stop_hooks() {
        let writer = HookSettingsWriter::new(scratch_dir());
        let value = writer.build("sess-1", 52413);

        let hooks = value.get("hooks").expect("top-level `hooks` key");
        assert!(hooks.get("Notification").is_some());
        assert!(hooks.get("Stop").is_some());
    }

    #[test]
    fn build_notification_entry_has_expected_shape() {
        let writer = HookSettingsWriter::new(scratch_dir());
        let value = writer.build("sess-1", 52413);

        let notification = value["hooks"]["Notification"]
            .as_array()
            .expect("Notification is an array");
        assert_eq!(notification.len(), 1);
        let group = &notification[0];
        assert_eq!(group["matcher"], "");
        let inner_hooks = group["hooks"].as_array().expect("hooks array");
        assert_eq!(inner_hooks.len(), 1);
        assert_eq!(inner_hooks[0]["type"], "command");
        assert!(inner_hooks[0]["command"].is_string());
    }

    #[test]
    fn curl_command_contains_required_flags() {
        let writer = HookSettingsWriter::new(scratch_dir());
        let value = writer.build("sess-1", 52413);

        let cmd = value["hooks"]["Notification"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .to_string();

        assert!(cmd.contains("-m 2"), "missing -m 2 timeout: {cmd}");
        assert!(
            cmd.contains("--data-binary @-"),
            "missing --data-binary @-: {cmd}"
        );
        assert!(cmd.contains("curl"), "missing curl invocation: {cmd}");
        // 이 어서션은 호스트(macOS)의 cfg!(windows) 분기를 그대로 반영한다 —
        // Windows CI가 없으므로 unix 변형만 여기서 실측 검증한다. Windows
        // 변형은 아래 curl_command_windows_variant_* 유닛 테스트가 순수
        // 함수를 직접 호출해 검증한다.
        assert!(cmd.ends_with("|| true"), "missing trailing || true: {cmd}");
    }

    // ---- Task A: pure curl_command(windows, ...) — Windows-safe rewrite ----

    #[test]
    fn curl_command_unix_variant_matches_exact_verified_string() {
        let cmd = curl_command(false, 52413, "sess-1", "hook");
        assert_eq!(
            cmd,
            "curl -sS -m 2 -X POST 'http://127.0.0.1:52413/hook?session=sess-1&source=hook' -H 'Content-Type: application/json' --data-binary @- || true"
        );
    }

    #[test]
    fn curl_command_windows_variant_uses_curl_exe_and_double_quotes() {
        let cmd = curl_command(true, 52413, "sess-1", "hook");

        assert!(cmd.starts_with("curl.exe "), "must invoke curl.exe, not curl: {cmd}");
        assert!(
            !cmd.contains('\''),
            "windows variant must contain no single quotes: {cmd}"
        );
        assert!(
            cmd.contains("\"http://127.0.0.1:52413/hook?session=sess-1&source=hook\""),
            "URL must be double-quoted so cmd.exe doesn't split on &: {cmd}"
        );
        assert!(
            !cmd.contains("||") && !cmd.contains("&&") && !cmd.contains(';'),
            "windows variant must not chain with any shell operator: {cmd}"
        );
        assert!(
            cmd.contains("-H \"Content-Type: application/json\""),
            "header must be double-quoted: {cmd}"
        );
        assert!(cmd.contains("--data-binary @-"));
        assert!(cmd.contains("-m 2"));
    }

    #[test]
    fn curl_command_windows_variant_has_no_trailing_or_true() {
        let cmd = curl_command(true, 1, "s", "hook");
        assert!(!cmd.ends_with("|| true"), "no `|| true` on windows: {cmd}");
        assert!(!cmd.contains("|| true"));
    }

    #[test]
    fn curl_command_substitutes_port_and_session_id() {
        let writer = HookSettingsWriter::new(scratch_dir());
        let value = writer.build("my-session-42", 9999);

        let notification_cmd = value["hooks"]["Notification"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        let stop_cmd = value["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();

        assert!(notification_cmd.contains("127.0.0.1:9999"));
        assert!(notification_cmd.contains("session=my-session-42"));
        assert!(notification_cmd.contains("source=hook"));

        assert!(stop_cmd.contains("127.0.0.1:9999"));
        assert!(stop_cmd.contains("session=my-session-42"));
        assert!(stop_cmd.contains("source=stop"));

        // Notification and Stop must hit distinct URLs (differ by `source`).
        assert_ne!(notification_cmd, stop_cmd);
    }

    #[test]
    fn write_creates_base_dir_and_file_with_matching_json() {
        let base = scratch_dir();
        let writer = HookSettingsWriter::new(base.clone());
        assert!(!base.exists());

        let path = writer.write("sess-write", 4000).expect("write succeeds");

        assert!(base.exists(), "base_dir should be created");
        assert!(path.exists(), "settings file should exist");
        assert_eq!(path, base.join("sess-write.settings.json"));

        let contents = fs::read_to_string(&path).expect("read settings file");
        let parsed: serde_json::Value =
            serde_json::from_str(&contents).expect("valid JSON written to disk");
        assert_eq!(parsed, writer.build("sess-write", 4000));

        // cleanup
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn write_is_idempotent_when_called_twice() {
        let base = scratch_dir();
        let writer = HookSettingsWriter::new(base.clone());

        writer.write("sess-a", 1).expect("first write succeeds");
        let path = writer.write("sess-a", 2).expect("second write succeeds");

        let contents = fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed, writer.build("sess-a", 2));

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn cleanup_removes_the_settings_file() {
        let base = scratch_dir();
        let writer = HookSettingsWriter::new(base.clone());
        let path = writer.write("sess-cleanup", 5000).unwrap();
        assert!(path.exists());

        writer.cleanup("sess-cleanup");

        assert!(!path.exists(), "cleanup should remove the settings file");

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn cleanup_on_missing_file_does_not_panic() {
        let base = scratch_dir();
        let writer = HookSettingsWriter::new(base.clone());

        // No write() was called; the file never existed.
        writer.cleanup("never-written");

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn build_produces_activity_hooks_with_correct_sources() {
        let writer = HookSettingsWriter::new(scratch_dir());
        let value = writer.build("sess-1", 52413);
        let hooks = value.get("hooks").expect("top-level `hooks` key");

        // 신규 activity 훅 두 개가 존재하고, 기존 알림 훅도 그대로.
        assert!(hooks.get("UserPromptSubmit").is_some(), "UserPromptSubmit hook present");
        assert!(hooks.get("PostToolUse").is_some(), "PostToolUse hook present");
        assert!(hooks.get("Notification").is_some(), "Notification hook still present");
        assert!(hooks.get("Stop").is_some(), "Stop hook still present");

        let prompt_cmd = value["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        let tool_cmd = value["hooks"]["PostToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(prompt_cmd.contains("source=prompt"), "prompt hook source: {prompt_cmd}");
        assert!(prompt_cmd.contains("session=sess-1") && prompt_cmd.contains("127.0.0.1:52413"));
        assert!(tool_cmd.contains("source=tool"), "tool hook source: {tool_cmd}");
        assert!(tool_cmd.contains("session=sess-1") && tool_cmd.contains("127.0.0.1:52413"));
    }
}
