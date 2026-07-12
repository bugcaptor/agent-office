pub mod claude;
pub mod codex;
pub mod event;
pub mod forwarder;
pub mod server;

use std::path::PathBuf;
use std::sync::Arc;

use crate::notification::hub::NotificationHub;
use claude::ClaudeAdapter;
use codex::CodexAdapter;

pub use event::{
    AdapterSessionPlan, CommandWrapperSpec, ObserverAdapterError, ObserverEvent, ObserverProvider,
    ObserverSessionContext, RawObserverHook, WrapperArg,
};

pub trait ObserverAdapter: Send + Sync {
    fn provider(&self) -> ObserverProvider;
    fn prepare_session(
        &self,
        context: &ObserverSessionContext,
    ) -> Result<AdapterSessionPlan, ObserverAdapterError>;
    fn map_hook(&self, raw: &RawObserverHook<'_>) -> Option<ObserverEvent>;
}

pub struct ObserverRuntime {
    hub: Arc<NotificationHub>,
    adapters: Vec<Arc<dyn ObserverAdapter>>,
}

impl ObserverRuntime {
    pub fn production(
        hub: Arc<NotificationHub>,
        settings_dir: PathBuf,
        forwarder_executable: PathBuf,
    ) -> Self {
        Self::new(
            hub,
            vec![
                Arc::new(ClaudeAdapter::new(settings_dir)),
                Arc::new(CodexAdapter::new(forwarder_executable)),
            ],
        )
    }

    pub fn new(hub: Arc<NotificationHub>, adapters: Vec<Arc<dyn ObserverAdapter>>) -> Self {
        Self { hub, adapters }
    }

    pub fn prepare_session(&self, context: &ObserverSessionContext) -> AdapterSessionPlan {
        let mut merged = AdapterSessionPlan::default();
        for adapter in &self.adapters {
            match adapter.prepare_session(context) {
                Ok(plan) => merged.merge(plan),
                Err(error) => eprintln!(
                    "observer adapter preparation failed provider={}: {error}",
                    adapter.provider().as_str(),
                ),
            }
        }
        merged
    }

    pub fn ingest(&self, provider: ObserverProvider, session_id: &str, raw: RawObserverHook<'_>) {
        let Some(adapter) = self
            .adapters
            .iter()
            .find(|adapter| adapter.provider() == provider)
        else {
            return;
        };
        let Some(event) = adapter.map_hook(&raw) else {
            return;
        };
        self.hub.ingest_observer(session_id, event);
    }

    pub fn ingest_pi_source(&self, session_id: &str, source: &str, body: &[u8]) {
        let event = match source {
            "prompt" => ObserverEvent::Prompt {
                text: event::prompt_text(body),
            },
            "tool" => ObserverEvent::Tool,
            "stop" => ObserverEvent::Stop {
                message: event::message(body),
            },
            _ => return,
        };
        self.hub.ingest_observer(session_id, event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification::hub::{NotificationHub, SystemClock};
    use crate::observer::claude::ClaudeAdapter;
    use crate::observer::codex::CodexAdapter;
    use crate::state::fake::RecordingEvents;
    use crate::state::{AppEvents, SessionRegistry};
    use crate::types::SessionState;
    use std::sync::Arc;

    struct FakeAdapter {
        provider: ObserverProvider,
        plan: Result<AdapterSessionPlan, ObserverAdapterError>,
        mapped: Option<ObserverEvent>,
    }

    impl ObserverAdapter for FakeAdapter {
        fn provider(&self) -> ObserverProvider {
            self.provider
        }

        fn prepare_session(
            &self,
            _context: &ObserverSessionContext,
        ) -> Result<AdapterSessionPlan, ObserverAdapterError> {
            self.plan.clone()
        }

        fn map_hook(&self, _raw: &RawObserverHook<'_>) -> Option<ObserverEvent> {
            self.mapped.clone()
        }
    }

    fn test_hub() -> Arc<NotificationHub> {
        let registry = Arc::new(SessionRegistry::new());
        let events: Arc<dyn AppEvents> = Arc::new(RecordingEvents::default());
        Arc::new(NotificationHub::new(
            registry,
            events,
            Arc::new(SystemClock),
            std::time::Duration::from_millis(3_000),
        ))
    }

    fn wrapper(command: &str) -> CommandWrapperSpec {
        CommandWrapperSpec {
            command: command.into(),
            prefix_args: vec![],
            skip_if_present: vec![],
        }
    }

    fn assert_common_mapping(adapter: &dyn ObserverAdapter) {
        let prompt = r#"{"prompt":"버그 고쳐줘"}"#.as_bytes();
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "UserPromptSubmit",
                body: prompt,
            }),
            Some(ObserverEvent::Prompt {
                text: Some("버그 고쳐줘".into()),
            }),
        );
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "PostToolUse",
                body: b"{}",
            }),
            Some(ObserverEvent::Tool),
        );
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "SubagentStart",
                body: b"{}",
            }),
            Some(ObserverEvent::SubStart),
        );
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "SubagentStop",
                body: b"{}",
            }),
            Some(ObserverEvent::SubStop),
        );
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "Unknown",
                body: b"{}",
            }),
            None,
        );
    }

    #[test]
    fn both_adapters_satisfy_the_shared_observer_event_contract() {
        let dir = std::env::temp_dir().join(format!(
            "agent-office-shared-observer-adapter-test-{}",
            uuid::Uuid::new_v4(),
        ));
        let claude = ClaudeAdapter::new(dir.clone());
        let codex = CodexAdapter::new(std::env::current_exe().unwrap());

        assert_eq!(claude.provider(), ObserverProvider::Claude);
        assert_eq!(codex.provider(), ObserverProvider::Codex);
        assert_common_mapping(&claude);
        assert_common_mapping(&codex);

        assert_eq!(
            claude.map_hook(&RawObserverHook {
                event_name: "Notification",
                body: br#"{"message":"claude attention"}"#,
            }),
            Some(ObserverEvent::Attention {
                message: Some("claude attention".into()),
            }),
        );
        assert_eq!(
            claude.map_hook(&RawObserverHook {
                event_name: "Stop",
                body: br#"{"message":"claude stop"}"#,
            }),
            Some(ObserverEvent::Stop {
                message: Some("claude stop".into()),
            }),
        );
        assert_eq!(
            codex.map_hook(&RawObserverHook {
                event_name: "PermissionRequest",
                body: br#"{"tool_input":{"description":"codex attention"}}"#,
            }),
            Some(ObserverEvent::Attention {
                message: Some("codex attention".into()),
            }),
        );
        assert_eq!(
            codex.map_hook(&RawObserverHook {
                event_name: "Stop",
                body: br#"{"last_assistant_message":"must not surface"}"#,
            }),
            Some(ObserverEvent::Stop { message: None }),
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn production_runtime_registers_exactly_one_equal_adapter_per_provider() {
        let settings_dir = std::env::temp_dir().join(format!(
            "agent-office-production-observer-test-{}",
            uuid::Uuid::new_v4(),
        ));
        let runtime = ObserverRuntime::production(
            test_hub(),
            settings_dir.clone(),
            std::env::current_exe().unwrap(),
        );

        assert_eq!(runtime.adapters.len(), 2);
        assert_eq!(
            runtime
                .adapters
                .iter()
                .map(|adapter| adapter.provider())
                .collect::<Vec<_>>(),
            vec![ObserverProvider::Claude, ObserverProvider::Codex],
        );

        let _ = std::fs::remove_dir_all(settings_dir);
    }

    #[test]
    fn one_adapter_failure_keeps_the_other_adapter_plan() {
        let hub = test_hub();
        let claude = Arc::new(FakeAdapter {
            provider: ObserverProvider::Claude,
            plan: Err(ObserverAdapterError::new("settings write failed")),
            mapped: None,
        });
        let codex = Arc::new(FakeAdapter {
            provider: ObserverProvider::Codex,
            plan: Ok(AdapterSessionPlan {
                env: vec![("AO_CODEX".into(), "1".into())],
                wrappers: vec![wrapper("codex")],
                cleanup_paths: vec![],
            }),
            mapped: None,
        });
        let runtime = ObserverRuntime::new(hub, vec![claude, codex]);
        let plan = runtime.prepare_session(&ObserverSessionContext::new(
            "ao-session",
            "http://127.0.0.1:4000/hook",
        ));
        assert_eq!(plan.env, vec![("AO_CODEX".into(), "1".into())]);
        assert_eq!(
            plan.wrappers
                .iter()
                .map(|w| w.command.as_str())
                .collect::<Vec<_>>(),
            vec!["codex"]
        );
    }

    #[test]
    fn codex_failure_keeps_the_claude_adapter_plan() {
        let hub = test_hub();
        let claude = Arc::new(FakeAdapter {
            provider: ObserverProvider::Claude,
            plan: Ok(AdapterSessionPlan {
                env: vec![("AGENT_OFFICE_SETTINGS".into(), "marker.json".into())],
                wrappers: vec![wrapper("claude")],
                cleanup_paths: vec![],
            }),
            mapped: None,
        });
        let codex = Arc::new(FakeAdapter {
            provider: ObserverProvider::Codex,
            plan: Err(ObserverAdapterError::new("forwarder path invalid")),
            mapped: None,
        });
        let runtime = ObserverRuntime::new(hub, vec![claude, codex]);
        let plan = runtime.prepare_session(&ObserverSessionContext::new(
            "ao-session",
            "http://127.0.0.1:4000/hook",
        ));
        assert_eq!(plan.wrappers[0].command, "claude");
        assert_eq!(plan.env[0].0, "AGENT_OFFICE_SETTINGS");
    }

    #[test]
    fn both_adapter_failures_produce_an_empty_fail_open_plan() {
        let hub = test_hub();
        let adapters: Vec<Arc<dyn ObserverAdapter>> =
            [ObserverProvider::Claude, ObserverProvider::Codex]
                .into_iter()
                .map(|provider| {
                    Arc::new(FakeAdapter {
                        provider,
                        plan: Err(ObserverAdapterError::new("injected preparation failure")),
                        mapped: None,
                    }) as Arc<dyn ObserverAdapter>
                })
                .collect();
        let runtime = ObserverRuntime::new(hub, adapters);
        let plan = runtime.prepare_session(&ObserverSessionContext::new(
            "ao-session",
            "http://127.0.0.1:4000/hook",
        ));
        assert_eq!(plan, AdapterSessionPlan::default());
    }

    #[test]
    fn switching_providers_keeps_the_same_agent_office_session_identity() {
        let registry = Arc::new(SessionRegistry::new());
        registry.insert("s1", "a1", SessionState::Running);
        let recorded = Arc::new(RecordingEvents::default());
        let hub = Arc::new(NotificationHub::new(
            registry,
            recorded.clone(),
            Arc::new(SystemClock),
            std::time::Duration::from_millis(3_000),
        ));
        let runtime = ObserverRuntime::new(
            hub,
            vec![
                Arc::new(FakeAdapter {
                    provider: ObserverProvider::Claude,
                    plan: Ok(AdapterSessionPlan::default()),
                    mapped: Some(ObserverEvent::Prompt {
                        text: Some("marker".into()),
                    }),
                }),
                Arc::new(FakeAdapter {
                    provider: ObserverProvider::Codex,
                    plan: Ok(AdapterSessionPlan::default()),
                    mapped: Some(ObserverEvent::Stop { message: None }),
                }),
            ],
        );
        runtime.ingest(
            ObserverProvider::Claude,
            "s1",
            RawObserverHook {
                event_name: "UserPromptSubmit",
                body: br#"{"session_id":"native-claude","prompt":"marker"}"#,
            },
        );
        runtime.ingest(
            ObserverProvider::Codex,
            "s1",
            RawObserverHook {
                event_name: "Stop",
                body: br#"{"session_id":"native-codex","turn_id":"native-turn"}"#,
            },
        );
        assert_eq!(recorded.activities()[0].session_id, "s1");
        assert_eq!(recorded.notifications()[0].session_id, "s1");
    }

    #[test]
    fn missing_provider_does_not_fallback_to_another_adapter() {
        let registry = Arc::new(SessionRegistry::new());
        registry.insert("s1", "a1", SessionState::Running);
        let recorded = Arc::new(RecordingEvents::default());
        let hub = Arc::new(NotificationHub::new(
            registry,
            recorded.clone(),
            Arc::new(SystemClock),
            std::time::Duration::from_millis(3_000),
        ));
        let runtime = ObserverRuntime::new(
            hub,
            vec![Arc::new(FakeAdapter {
                provider: ObserverProvider::Claude,
                plan: Ok(AdapterSessionPlan::default()),
                mapped: Some(ObserverEvent::Tool),
            })],
        );

        runtime.ingest(
            ObserverProvider::Codex,
            "s1",
            RawObserverHook {
                event_name: "PostToolUse",
                body: b"{}",
            },
        );

        assert!(recorded.activities().is_empty());
        assert!(recorded.notifications().is_empty());
    }
}
