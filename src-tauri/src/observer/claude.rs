use std::path::PathBuf;

use super::event::{message, prompt_text};
use super::{
    AdapterSessionPlan, CommandWrapperSpec, ObserverAdapter, ObserverAdapterError,
    ObserverCapabilities, ObserverEvent, ObserverProvider, ObserverSessionContext, RawObserverHook,
    WrapperArg,
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

    fn capabilities(&self) -> ObserverCapabilities {
        ObserverCapabilities::complete()
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
        match raw.event_name {
            "UserPromptSubmit" => Some(ObserverEvent::Prompt {
                text: prompt_text(raw.body),
            }),
            "PostToolUse" => Some(ObserverEvent::Tool),
            "SubagentStart" => Some(ObserverEvent::SubStart),
            "SubagentStop" => Some(ObserverEvent::SubStop),
            "Notification" => Some(ObserverEvent::Attention {
                message: Some(
                    message(raw.body).unwrap_or_else(|| "Claude needs your attention".into()),
                ),
            }),
            "Stop" => Some(ObserverEvent::Stop {
                message: Some(message(raw.body).unwrap_or_else(|| "Claude finished a task".into())),
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
    fn claude_missing_messages_preserve_legacy_fallback_copy() {
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
                Some(ObserverEvent::Attention {
                    message: Some("Claude needs your attention".into()),
                }),
            );
            assert_eq!(
                adapter.map_hook(&RawObserverHook {
                    event_name: "Stop",
                    body,
                }),
                Some(ObserverEvent::Stop {
                    message: Some("Claude finished a task".into()),
                }),
            );
        }
    }
}
