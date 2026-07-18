use std::path::PathBuf;

use super::event::{
    agent_id, claude_transcript_message, message, prompt_text, running_subagents,
};
use super::hook_command::forwarder_shell_command;
use super::{
    AdapterSessionPlan, CommandWrapperSpec, ObserverAdapter, ObserverAdapterError, ObserverEvent,
    ObserverProvider, ObserverSessionContext, RawObserverHook, WrapperArg,
};

pub struct ClaudeAdapter {
    settings_dir: PathBuf,
    forwarder_executable: PathBuf,
}

impl ClaudeAdapter {
    pub fn new(settings_dir: PathBuf, forwarder_executable: PathBuf) -> Self {
        Self {
            settings_dir,
            forwarder_executable,
        }
    }

    /// 훅 명령을 앱 바이너리 forwarder(`--observer-forward claude <event>`)로 만든다.
    ///
    /// 예전에는 curl로 훅 URL을 명령에 박아 넣었는데, 그러면 재시작 후 sessiond로
    /// 입양된 세션이 죽은(스폰 시점) 포트를 계속 때리고 `|| true`로 조용히 실패했다
    /// (docs/session-handoff-design.md §핵심 5, 이슈 #30). forwarder는 실행 시점에
    /// 세션 env의 `AGENT_OFFICE_HOOK_URL`을 읽고, 연결이 거부되면
    /// `AGENT_OFFICE_APP_DATA/observer-port` 파일의 최신 포트로 1회 재시도한다.
    ///
    /// SessionStart/SessionEnd처럼 훅 stdout이 대화 컨텍스트로 주입되는 이벤트에도
    /// 같은 명령을 쓴다. forwarder는 stdout에 아무것도 쓰지 않으므로(서버 응답을
    /// 버린다) 예전 curl `-o /dev/null` 변형이 필요 없다.
    fn hook_command(&self, event: &str) -> Result<String, ObserverAdapterError> {
        forwarder_shell_command(&self.forwarder_executable, &["claude", event])
    }
}

impl ObserverAdapter for ClaudeAdapter {
    fn provider(&self) -> ObserverProvider {
        ObserverProvider::Claude
    }

    fn prepare_session(
        &self,
        context: &ObserverSessionContext,
    ) -> Result<AdapterSessionPlan, ObserverAdapterError> {
        std::fs::create_dir_all(&self.settings_dir).map_err(|error| {
            ObserverAdapterError::new(format!("Claude settings directory failed: {error}"))
        })?;
        let path = self
            .settings_dir
            .join(format!("{}.settings.json", context.session_id));
        // forwarder 경로 검증 실패 시 여기서 Err를 전파한다(codex와 동일 계약).
        let entry = |command: String| {
            serde_json::json!([{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": command,
                }],
            }])
        };
        // SessionStart/SessionEnd는 map_hook에서 이벤트로 매핑되지 않지만(허브
        // 무영향), ingest의 리줌 ID 캡처가 body를 본다 — 프롬프트 한 번 없이
        // 시작·종료한 세션도 리줌 ID를 남기기 위한 등록(리뷰 지적 반영,
        // docs/claude-session-resume-design.md §2). 8개 이벤트 모두 forwarder
        // 명령을 쓴다(위 hook_command 주석: 예전 silent 변형은 불필요).
        let settings = serde_json::json!({
            "hooks": {
                "UserPromptSubmit": entry(self.hook_command("UserPromptSubmit")?),
                "PostToolUse": entry(self.hook_command("PostToolUse")?),
                "Notification": entry(self.hook_command("Notification")?),
                "Stop": entry(self.hook_command("Stop")?),
                "SubagentStart": entry(self.hook_command("SubagentStart")?),
                "SubagentStop": entry(self.hook_command("SubagentStop")?),
                "SessionStart": entry(self.hook_command("SessionStart")?),
                "SessionEnd": entry(self.hook_command("SessionEnd")?),
            },
        });
        let contents = serde_json::to_vec_pretty(&settings)
            .expect("serializing Claude hook settings cannot fail");
        std::fs::write(&path, contents).map_err(|error| {
            ObserverAdapterError::new(format!("Claude settings write failed: {error}"))
        })?;

        Ok(AdapterSessionPlan {
            env: vec![(
                "AGENT_OFFICE_SETTINGS".into(),
                path.to_string_lossy().into_owned(),
            )],
            wrappers: vec![CommandWrapperSpec {
                command: "claude".into(),
                prefix_args: vec![
                    WrapperArg::Literal("--settings".into()),
                    WrapperArg::Env("AGENT_OFFICE_SETTINGS".into()),
                ],
                skip_if_present: vec!["--settings".into()],
            }],
            cleanup_paths: vec![path],
        })
    }

    fn map_hook(&self, raw: &RawObserverHook<'_>) -> Option<ObserverEvent> {
        // Subagent-internal hooks (agent_id present) must never open or close the main
        // turn boundary (opening via Prompt or closing via Stop). Every time a Task
        // subagent finishes an internal turn, Claude Code fires a Stop hook with
        // agent_id populated; treating that as main-session termination grays out the
        // label. SubagentStart/Stop are lifecycle signals with agent_id normally
        // present, so let them pass through. Tool (PostToolUse) / Attention
        // (Notification) are heartbeat/attention signals, so let them pass through too.
        if matches!(raw.event_name, "Stop" | "UserPromptSubmit") && agent_id(raw.body).is_some() {
            return None;
        }

        match raw.event_name {
            "UserPromptSubmit" => Some(ObserverEvent::Prompt {
                text: prompt_text(raw.body),
            }),
            "PostToolUse" => Some(ObserverEvent::Tool),
            "SubagentStart" => Some(ObserverEvent::SubStart),
            // 절대 카운트는 self 제외를 위해 top-level agent_id가 반드시 있어야 신뢰할 수
            // 있다(리뷰 지적: agent_id 부재 시 자기 자신까지 세어 off-by-one으로 미니미
            // 잔존). agent_id 또는 background_tasks가 없으면 안전하게 SubStop 델타로 강등.
            "SubagentStop" => Some(match (agent_id(raw.body), running_subagents(raw.body)) {
                (Some(_), Some(running)) => ObserverEvent::SubCount { running },
                _ => ObserverEvent::SubStop,
            }),
            "Notification" => Some(ObserverEvent::Attention {
                message: message(raw.body),
            }),
            // 이슈 #39: Claude Stop 훅 body 엔 message 필드가 없다 → transcript_path
            // (JSONL)의 마지막 assistant 텍스트를 완료 본문으로 뽑는다. 파일 부재/
            // 포맷 이상은 None 폴백 → hub 의 STOP_FALLBACK 유지. body 에 message 가
            // 실려 오는 경로(pi 등 미래 확장)는 그대로 우선한다.
            "Stop" => Some(ObserverEvent::Stop {
                message: message(raw.body).or_else(|| claude_transcript_message(raw.body)),
                running: running_subagents(raw.body),
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ClaudeAdapter;
    use crate::observer::{
        ObserverAdapter, ObserverEvent, ObserverSessionContext, RawObserverHook, WrapperArg,
    };

    fn scratch_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-observer-adapter-test-{}",
            uuid::Uuid::new_v4(),
        ))
    }

    /// 훅 명령 빌더는 절대 경로 forwarder를 요구한다.
    fn forwarder_exe() -> std::path::PathBuf {
        if cfg!(windows) {
            std::path::PathBuf::from(r"C:\Program Files\Agent Office\agent-office.exe")
        } else {
            std::path::PathBuf::from("/opt/agent-office/agent-office")
        }
    }

    #[test]
    fn claude_plan_writes_four_hooks_and_settings_wrapper() {
        let dir = scratch_dir();
        let adapter = ClaudeAdapter::new(dir.clone(), forwarder_exe());
        let context = ObserverSessionContext::new("ao-s1", "http://127.0.0.1:43123/hook");

        let plan = adapter.prepare_session(&context).unwrap();

        let path = dir.join("ao-s1.settings.json");
        assert_eq!(
            plan.env,
            vec![(
                "AGENT_OFFICE_SETTINGS".into(),
                path.to_string_lossy().into_owned(),
            )],
        );
        assert_eq!(plan.cleanup_paths, vec![path.clone()]);
        assert_eq!(plan.wrappers.len(), 1);
        assert_eq!(plan.wrappers[0].command, "claude");
        assert_eq!(
            plan.wrappers[0].prefix_args,
            vec![
                WrapperArg::Literal("--settings".into()),
                WrapperArg::Env("AGENT_OFFICE_SETTINGS".into()),
            ],
        );
        assert_eq!(plan.wrappers[0].skip_if_present, vec!["--settings"]);

        let raw = std::fs::read_to_string(&path).unwrap();
        // 스테일 포트 회귀 방지의 핵심 어서션: 훅 URL/포트가 더 이상 설정 파일에
        // 박히지 않는다 — forwarder가 실행 시점에 최신 포트로 라우팅한다(이슈 #30).
        assert!(
            !raw.contains("127.0.0.1"),
            "settings must not embed a spawn-time observer port: {raw}",
        );
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        for event in ["UserPromptSubmit", "PostToolUse", "Notification", "Stop"] {
            let entry = &json["hooks"][event][0];
            assert_eq!(entry["matcher"], "", "wrong matcher for {event}: {json}");
            assert_eq!(entry["hooks"][0]["type"], "command");
            let command = entry["hooks"][0]["command"].as_str().unwrap();
            // unix는 forwarder 명령이 평문이라 인자를 직접 검증한다. windows는
            // powershell EncodedCommand(base64)라 실행 픽스처로 검증한다(codex와 동일).
            #[cfg(not(windows))]
            {
                assert!(
                    command.contains("--observer-forward")
                        && command.contains("claude")
                        && command.contains(event),
                    "hook must forward via the app binary for {event}: {command}",
                );
            }
            #[cfg(windows)]
            {
                assert!(
                    command.contains("powershell.exe"),
                    "windows hook must use powershell forwarder for {event}: {command}",
                );
            }
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn claude_plan_preserves_subagent_lifecycle_hooks_from_the_legacy_observer() {
        let dir = scratch_dir();
        let adapter = ClaudeAdapter::new(dir.clone(), forwarder_exe());
        let context = ObserverSessionContext::new("ao-s1", "http://127.0.0.1:43123/hook");

        adapter.prepare_session(&context).unwrap();
        let path = dir.join("ao-s1.settings.json");
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();

        for event in ["SubagentStart", "SubagentStop"] {
            let entry = &json["hooks"][event][0];
            assert_eq!(entry["matcher"], "", "missing {event} hook: {json}");
            let command = entry["hooks"][0]["command"].as_str().unwrap();
            #[cfg(not(windows))]
            assert!(
                command.contains("--observer-forward")
                    && command.contains("claude")
                    && command.contains(event),
                "wrong {event} command: {command}",
            );
            #[cfg(windows)]
            assert!(
                command.contains("powershell.exe"),
                "wrong {event} command: {command}",
            );
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn claude_plan_registers_session_lifecycle_hooks_via_forwarder() {
        let dir = scratch_dir();
        let adapter = ClaudeAdapter::new(dir.clone(), forwarder_exe());
        let context = ObserverSessionContext::new("ao-s1", "http://127.0.0.1:43123/hook");

        adapter.prepare_session(&context).unwrap();
        let path = dir.join("ao-s1.settings.json");
        let raw = std::fs::read_to_string(&path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();

        for event in ["SessionStart", "SessionEnd"] {
            let entry = &json["hooks"][event][0];
            assert_eq!(entry["matcher"], "", "missing {event} hook: {json}");
            let command = entry["hooks"][0]["command"].as_str().unwrap();
            // SessionStart/End는 훅 stdout이 세션 컨텍스트로 주입되지만, forwarder는
            // stdout에 아무것도 쓰지 않으므로(서버 응답을 버린다) 예전 curl
            // `-o /dev/null` 변형 없이 나머지 이벤트와 같은 명령을 쓴다.
            #[cfg(not(windows))]
            assert!(
                command.contains("--observer-forward")
                    && command.contains("claude")
                    && command.contains(event),
                "wrong {event} command: {command}",
            );
            #[cfg(windows)]
            assert!(
                command.contains("powershell.exe"),
                "wrong {event} command: {command}",
            );
        }
        // 스테일 포트 회귀 방지(이슈 #30): 어떤 훅에도 포트가 박히지 않는다.
        assert!(!raw.contains("127.0.0.1"), "settings must not embed a port: {raw}");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_lifecycle_hooks_map_to_no_observer_event() {
        // 리줌 ID 캡처 전용 등록 — 허브 이벤트(턴 경계·활동)에는 영향이 없어야 한다.
        let adapter = ClaudeAdapter::new(scratch_dir(), forwarder_exe());
        for event_name in ["SessionStart", "SessionEnd"] {
            assert_eq!(
                adapter.map_hook(&RawObserverHook {
                    event_name,
                    body: br#"{"session_id":"native-1"}"#,
                }),
                None,
            );
        }
    }

    // 훅 명령이 URL/포트를 담지 않고 앱 바이너리 forwarder를 경유하는지 확인한다
    // (이슈 #30 스테일 포트 회귀 방지). unix는 명령이 평문이라 형태를 직접 검증한다.
    #[cfg(not(windows))]
    #[test]
    fn claude_hook_command_forwards_via_app_binary_on_unix() {
        let adapter = ClaudeAdapter::new(
            scratch_dir(),
            std::path::PathBuf::from("/tmp/Agent 'Office'/agent-office"),
        );

        let command = adapter.hook_command("Stop").unwrap();
        assert_eq!(
            command,
            "'/tmp/Agent '\"'\"'Office'\"'\"'/agent-office' --observer-forward claude Stop",
        );
        // 포트가 명령에 없어야 한다 — forwarder가 실행 시점에 라우팅한다.
        assert!(!command.contains("127.0.0.1"));
    }

    // forwarder 경로가 절대경로가 아니면 prepare_session이 Err를 반환한다(codex와 동일 계약).
    #[test]
    fn claude_prepare_session_rejects_relative_forwarder_path() {
        let adapter = ClaudeAdapter::new(scratch_dir(), std::path::PathBuf::from("agent-office"));
        let context = ObserverSessionContext::new("ao-s1", "http://127.0.0.1:43123/hook");
        assert_eq!(
            adapter.prepare_session(&context).unwrap_err().to_string(),
            "observer forwarder path must be absolute",
        );
    }

    #[test]
    fn claude_missing_messages_defer_to_hub_fallback() {
        let adapter = ClaudeAdapter::new(scratch_dir(), forwarder_exe());

        for body in [
            b"{}".as_slice(),
            b"not json".as_slice(),
            br#"{"message":"   "}"#.as_slice(),
        ] {
            assert_eq!(
                adapter.map_hook(&RawObserverHook {
                    event_name: "Notification",
                    body,
                }),
                Some(ObserverEvent::Attention { message: None }),
            );
            assert_eq!(
                adapter.map_hook(&RawObserverHook {
                    event_name: "Stop",
                    body,
                }),
                Some(ObserverEvent::Stop {
                    message: None,
                    running: None,
                }),
            );
        }
    }

    #[test]
    fn claude_stop_reads_completion_from_transcript_tail() {
        // 이슈 #39: message 필드가 없어도 transcript_path 의 마지막 assistant
        // 텍스트를 완료 본문으로 실어야 한다.
        let adapter = ClaudeAdapter::new(scratch_dir(), forwarder_exe());
        let dir = scratch_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");
        let lines = [
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"이전 응답"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"작업을 마쳤습니다"}]}}"#,
        ];
        std::fs::write(&path, lines.join("\n")).unwrap();
        let body = serde_json::json!({ "transcript_path": path.to_string_lossy() })
            .to_string()
            .into_bytes();

        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "Stop",
                body: &body,
            }),
            Some(ObserverEvent::Stop {
                message: Some("작업을 마쳤습니다".into()),
                running: None,
            }),
        );

        // transcript 부재 시엔 None 폴백(hub STOP_FALLBACK).
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "Stop",
                body: br#"{"transcript_path":"/nonexistent/transcript.jsonl"}"#,
            }),
            Some(ObserverEvent::Stop {
                message: None,
                running: None,
            }),
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn subagent_internal_hooks_cannot_open_or_close_the_main_turn() {
        let adapter = ClaudeAdapter::new(scratch_dir(), forwarder_exe());
        let map = |event_name, body| adapter.map_hook(&RawObserverHook { event_name, body });

        assert_eq!(map("Stop", br#"{"agent_id":"sub-1","message":"m"}"#), None,);
        assert_eq!(
            map("Stop", br#"{"message":"m"}"#),
            Some(ObserverEvent::Stop {
                message: Some("m".into()),
                running: None,
            }),
        );
        assert_eq!(
            map("UserPromptSubmit", br#"{"agent_id":"sub-1","prompt":"x"}"#,),
            None,
        );
        assert_eq!(
            map("PostToolUse", br#"{"agent_id":"sub-1"}"#),
            Some(ObserverEvent::Tool),
        );
        assert_eq!(
            map(
                "Notification",
                br#"{"agent_id":"sub-1","message":"needs permission"}"#,
            ),
            Some(ObserverEvent::Attention {
                message: Some("needs permission".into()),
            }),
        );
        assert_eq!(
            map("SubagentStart", br#"{"agent_id":"sub-1"}"#),
            Some(ObserverEvent::SubStart),
        );
        // agent_id 있는 SubagentStop이라도 background_tasks가 없으면 SubStop 델타로 강등.
        assert_eq!(
            map("SubagentStop", br#"{"agent_id":"sub-1"}"#),
            Some(ObserverEvent::SubStop),
        );
        assert_eq!(
            map("Stop", br#"{"agent_id":"","message":"m"}"#),
            Some(ObserverEvent::Stop {
                message: Some("m".into()),
                running: None,
            }),
        );
    }

    #[test]
    fn claude_maps_background_task_snapshots_to_absolute_counts() {
        let adapter = ClaudeAdapter::new(scratch_dir(), forwarder_exe());
        let subagent_body = br#"{
            "agent_id":"self",
            "background_tasks":[
                {"id":"self","type":"subagent","status":"running"},
                {"id":"other","type":"subagent","status":"running"}
            ]
        }"#;
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "SubagentStop",
                body: subagent_body,
            }),
            Some(ObserverEvent::SubCount { running: 1 }),
        );
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "SubagentStop",
                body: b"{}",
            }),
            Some(ObserverEvent::SubStop),
        );

        let stop_body = br#"{
            "message":"done",
            "background_tasks":[
                {"id":"one","type":"subagent","status":"running"},
                {"id":"two","type":"subagent","status":"running"}
            ]
        }"#;
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "Stop",
                body: stop_body,
            }),
            Some(ObserverEvent::Stop {
                message: Some("done".into()),
                running: Some(2),
            }),
        );
    }
}
