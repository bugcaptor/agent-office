use std::path::PathBuf;

use super::event::{agent_id, message, prompt_text, running_subagents};
use super::{
    AdapterSessionPlan, CommandWrapperSpec, ObserverAdapter, ObserverAdapterError, ObserverEvent,
    ObserverProvider, ObserverSessionContext, RawObserverHook, WrapperArg,
};

pub struct ClaudeAdapter {
    settings_dir: PathBuf,
}

impl ClaudeAdapter {
    pub fn new(settings_dir: PathBuf) -> Self {
        Self { settings_dir }
    }

    fn hook_command_for(windows: bool, context: &ObserverSessionContext, event: &str) -> String {
        let url = format!(
            "{}?session={}&provider=claude&event={event}",
            context.hook_url, context.session_id,
        );
        if windows {
            format!(
                "curl.exe -sS -m 2 -X POST \"{url}\" -H \"Content-Type: application/json\" --data-binary @-"
            )
        } else {
            format!(
                "curl -sS -m 2 -X POST '{url}' -H 'Content-Type: application/json' --data-binary @- || true"
            )
        }
    }

    fn hook_command(context: &ObserverSessionContext, event: &str) -> String {
        Self::hook_command_for(cfg!(windows), context, event)
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
        let entry = |event: &str| {
            serde_json::json!([{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": Self::hook_command(context, event),
                }],
            }])
        };
        let settings = serde_json::json!({
            "hooks": {
                "UserPromptSubmit": entry("UserPromptSubmit"),
                "PostToolUse": entry("PostToolUse"),
                "Notification": entry("Notification"),
                "Stop": entry("Stop"),
                "SubagentStart": entry("SubagentStart"),
                "SubagentStop": entry("SubagentStop"),
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
            "Stop" => Some(ObserverEvent::Stop {
                message: message(raw.body),
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

    #[test]
    fn claude_plan_writes_four_hooks_and_settings_wrapper() {
        let dir = scratch_dir();
        let adapter = ClaudeAdapter::new(dir.clone());
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

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        for event in ["UserPromptSubmit", "PostToolUse", "Notification", "Stop"] {
            let entry = &json["hooks"][event][0];
            assert_eq!(entry["matcher"], "", "wrong matcher for {event}: {json}");
            assert_eq!(entry["hooks"][0]["type"], "command");
            let command = entry["hooks"][0]["command"].as_str().unwrap();
            assert!(
                command.contains(&format!(
                    "http://127.0.0.1:43123/hook?session=ao-s1&provider=claude&event={event}"
                )),
                "wrong URL for {event}: {command}",
            );
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn claude_plan_preserves_subagent_lifecycle_hooks_from_the_legacy_observer() {
        let dir = scratch_dir();
        let adapter = ClaudeAdapter::new(dir.clone());
        let context = ObserverSessionContext::new("ao-s1", "http://127.0.0.1:43123/hook");

        adapter.prepare_session(&context).unwrap();
        let path = dir.join("ao-s1.settings.json");
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();

        for event in ["SubagentStart", "SubagentStop"] {
            let entry = &json["hooks"][event][0];
            assert_eq!(entry["matcher"], "", "missing {event} hook: {json}");
            let command = entry["hooks"][0]["command"].as_str().unwrap();
            assert!(
                command.contains(&format!("provider=claude&event={event}")),
                "wrong {event} command: {command}",
            );
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn claude_hook_command_preserves_both_os_curl_dialects() {
        let context = ObserverSessionContext::new("ao-s1", "http://127.0.0.1:43123/hook");
        let url = "http://127.0.0.1:43123/hook?session=ao-s1&provider=claude&event=Stop";

        assert_eq!(
            ClaudeAdapter::hook_command_for(false, &context, "Stop"),
            format!(
                "curl -sS -m 2 -X POST '{url}' -H 'Content-Type: application/json' --data-binary @- || true"
            ),
        );
        assert_eq!(
            ClaudeAdapter::hook_command_for(true, &context, "Stop"),
            format!(
                "curl.exe -sS -m 2 -X POST \"{url}\" -H \"Content-Type: application/json\" --data-binary @-"
            ),
        );
    }

    #[test]
    fn claude_missing_messages_defer_to_hub_fallback() {
        let adapter = ClaudeAdapter::new(scratch_dir());

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
    fn subagent_internal_hooks_cannot_open_or_close_the_main_turn() {
        let adapter = ClaudeAdapter::new(scratch_dir());
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
        let adapter = ClaudeAdapter::new(scratch_dir());
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
