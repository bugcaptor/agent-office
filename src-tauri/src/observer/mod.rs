pub mod claude;
pub mod claude_resume_recorder;
pub mod codex;
pub mod event;
pub mod forwarder;
pub mod hook_command;
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

    /// 입양 시 세션 아티팩트(예: Claude 설정 파일) 복구(이슈 #40). `path`가 이
    /// 어댑터의 소관이면 멱등 재작성 후 `Ok(true)`, 아니면 `Ok(false)`. 기본은
    /// no-op이라 아티팩트가 없는 어댑터(codex 등)는 구현할 필요가 없다.
    fn restore_session_artifact(
        &self,
        _path: &std::path::Path,
    ) -> Result<bool, ObserverAdapterError> {
        Ok(false)
    }
}

/// Claude 훅 body에서 뽑은 native 세션 ID를 소비하는 주입점(리줌 기능).
/// 프로덕션 구현은 `claude_resume_recorder::ClaudeResumeRecorder`, 테스트는
/// 페이크. 부재 시 `ingest`는 캡처를 no-op으로 건너뛴다.
pub trait ClaudeSessionSink: Send + Sync {
    /// ao_session_id = agent-office UUID(훅 라우팅 키), native = Claude 세션 ID.
    fn record(&self, ao_session_id: &str, native_session_id: &str, cwd: Option<&str>);
}

pub struct ObserverRuntime {
    hub: Arc<NotificationHub>,
    adapters: Vec<Arc<dyn ObserverAdapter>>,
    claude_session_sink: Option<Arc<dyn ClaudeSessionSink>>,
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
                Arc::new(ClaudeAdapter::new(settings_dir, forwarder_executable.clone())),
                Arc::new(CodexAdapter::new(forwarder_executable)),
            ],
        )
    }

    pub fn new(hub: Arc<NotificationHub>, adapters: Vec<Arc<dyn ObserverAdapter>>) -> Self {
        Self {
            hub,
            adapters,
            claude_session_sink: None,
        }
    }

    /// Claude 리줌 캡처 sink를 배선한다(builder 스타일 — production/new의
    /// 기존 시그니처를 깨지 않으려고 선택 주입으로 뒀다).
    pub fn with_claude_session_sink(mut self, sink: Arc<dyn ClaudeSessionSink>) -> Self {
        self.claude_session_sink = Some(sink);
        self
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

    /// 입양 시 세션 아티팩트 복구(이슈 #40). `cleanup_paths`가 비면 no-op이라
    /// observer OFF 세션·codex-only 세션은 자연히 건너뛴다(claude만 cleanup_paths에
    /// 파일을 싣는다). 각 path를 소관 어댑터가 **멱등 재작성**한다 — 앱이 꺼진 사이
    /// 파일이 사라졌거나 낡은 forwarder 경로가 남아 있어도 현재 값으로 복원된다.
    /// 실패는 로그만 남기고 입양을 막지 않는다(claude 재실행만 다음 부트까지 제한).
    pub fn restore_session_artifacts(&self, session_id: &str, cleanup_paths: &[PathBuf]) {
        for path in cleanup_paths {
            let existed = path.exists();
            for adapter in &self.adapters {
                match adapter.restore_session_artifact(path) {
                    Ok(false) => continue,
                    Ok(true) => {
                        eprintln!(
                            "agent-office: observer artifact restore session={session_id} provider={} path={} existed={existed}",
                            adapter.provider().as_str(),
                            path.display(),
                        );
                        break;
                    }
                    Err(error) => {
                        eprintln!(
                            "agent-office: observer artifact restore failed session={session_id} provider={} path={} error={error}",
                            adapter.provider().as_str(),
                            path.display(),
                        );
                        break;
                    }
                }
            }
        }
    }

    pub fn ingest(&self, provider: ObserverProvider, session_id: &str, raw: RawObserverHook<'_>) {
        let Some(adapter) = self
            .adapters
            .iter()
            .find(|adapter| adapter.provider() == provider)
        else {
            return;
        };
        // Claude 훅 body에는 모든 이벤트마다 native session_id가 실려 온다.
        // map_hook이 None으로 걸러내는 서브에이전트 훅(agent_id 있는 Stop 등)이라도
        // session_id는 메인 세션 것이므로 리줌 기록엔 유효 — map_hook 결과와
        // 무관하게 여기서 먼저 캡처한다(docs/claude-session-resume-design.md §2).
        if provider == ObserverProvider::Claude {
            if let (Some(sink), Some(native)) =
                (&self.claude_session_sink, event::native_session_id(raw.body))
            {
                sink.record(session_id, &native, event::hook_cwd(raw.body).as_deref());
            }
        }
        let Some(event) = adapter.map_hook(&raw) else {
            return;
        };
        self.hub.ingest_observer(session_id, event);
    }

    pub fn ingest_pi_source(&self, session_id: &str, source: &str, body: &[u8]) {
        let event = match source {
            "prompt" => ObserverEvent::Prompt {
                text: event::prompt_text(body),
                cwd: event::hook_cwd(body),
            },
            "tool" => ObserverEvent::Tool {
                text: None,
                assistant: None,
            },
            "stop" => ObserverEvent::Stop {
                message: event::message(body),
                running: None,
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
            ..Default::default()
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
                // body에 cwd가 없으므로 hook_cwd → None(양 어댑터 공통).
                cwd: None,
            }),
        );
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "PostToolUse",
                body: b"{}",
            }),
            Some(ObserverEvent::Tool {
                text: None,
                assistant: None,
            }),
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
        let claude = ClaudeAdapter::new(dir.clone(), std::env::current_exe().unwrap());
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
                running: None,
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
        // 이슈 #39: 이제 codex Stop 의 last_assistant_message 는 완료 본문으로
        // 노출된다(예전엔 의도적으로 버려 message: None 이었다).
        assert_eq!(
            codex.map_hook(&RawObserverHook {
                event_name: "Stop",
                body: r#"{"last_assistant_message":"코덱스 작업 완료"}"#.as_bytes(),
            }),
            Some(ObserverEvent::Stop {
                message: Some("코덱스 작업 완료".into()),
                running: None,
            }),
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
                        cwd: None,
                    }),
                }),
                Arc::new(FakeAdapter {
                    provider: ObserverProvider::Codex,
                    plan: Ok(AdapterSessionPlan::default()),
                    mapped: Some(ObserverEvent::Stop {
                        message: None,
                        running: None,
                    }),
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

    #[derive(Default)]
    struct FakeSink {
        calls: std::sync::Mutex<Vec<(String, String, Option<String>)>>,
    }

    impl ClaudeSessionSink for FakeSink {
        fn record(&self, ao_session_id: &str, native_session_id: &str, cwd: Option<&str>) {
            self.calls.lock().unwrap().push((
                ao_session_id.to_string(),
                native_session_id.to_string(),
                cwd.map(str::to_string),
            ));
        }
    }

    impl FakeSink {
        fn calls(&self) -> Vec<(String, String, Option<String>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    fn claude_runtime_with_sink(
        mapped: Option<ObserverEvent>,
        sink: Arc<FakeSink>,
    ) -> ObserverRuntime {
        ObserverRuntime::new(
            test_hub(),
            vec![Arc::new(FakeAdapter {
                provider: ObserverProvider::Claude,
                plan: Ok(AdapterSessionPlan::default()),
                mapped,
            })],
        )
        .with_claude_session_sink(sink)
    }

    #[test]
    fn claude_hook_records_native_session_id_and_cwd_to_sink() {
        let sink = Arc::new(FakeSink::default());
        let runtime = claude_runtime_with_sink(
            Some(ObserverEvent::Tool {
                text: None,
                assistant: None,
            }),
            sink.clone(),
        );
        runtime.ingest(
            ObserverProvider::Claude,
            "s1",
            RawObserverHook {
                event_name: "PostToolUse",
                body: br#"{"session_id":"native-1","cwd":"/w/project"}"#,
            },
        );
        assert_eq!(
            sink.calls(),
            vec![("s1".into(), "native-1".into(), Some("/w/project".into()))]
        );
    }

    #[test]
    fn claude_hook_records_even_when_map_hook_filters_the_event() {
        // map_hook이 None을 반환해도(서브에이전트 훅 등) session_id는 캡처돼야 한다.
        let sink = Arc::new(FakeSink::default());
        let runtime = claude_runtime_with_sink(None, sink.clone());
        runtime.ingest(
            ObserverProvider::Claude,
            "s1",
            RawObserverHook {
                event_name: "SubagentStop",
                body: br#"{"session_id":"native-2","agent_id":"sub-a"}"#,
            },
        );
        assert_eq!(
            sink.calls(),
            vec![("s1".into(), "native-2".into(), None)]
        );
    }

    #[test]
    fn codex_hook_does_not_record_to_claude_sink() {
        let sink = Arc::new(FakeSink::default());
        let runtime = ObserverRuntime::new(
            test_hub(),
            vec![Arc::new(FakeAdapter {
                provider: ObserverProvider::Codex,
                plan: Ok(AdapterSessionPlan::default()),
                mapped: Some(ObserverEvent::Stop {
                    message: None,
                    running: None,
                }),
            })],
        )
        .with_claude_session_sink(sink.clone());
        runtime.ingest(
            ObserverProvider::Codex,
            "s1",
            RawObserverHook {
                event_name: "Stop",
                body: br#"{"session_id":"native-codex"}"#,
            },
        );
        assert!(sink.calls().is_empty());
    }

    #[test]
    fn claude_hook_without_session_id_is_a_sink_noop() {
        let sink = Arc::new(FakeSink::default());
        let runtime = claude_runtime_with_sink(
            Some(ObserverEvent::Tool {
                text: None,
                assistant: None,
            }),
            sink.clone(),
        );
        runtime.ingest(
            ObserverProvider::Claude,
            "s1",
            RawObserverHook {
                event_name: "PostToolUse",
                body: br#"{"cwd":"/w/project"}"#,
            },
        );
        assert!(sink.calls().is_empty());
    }

    /// 이슈 #40: restore_session_artifact 호출을 기록하고 소관 파일이면 Ok(true)를
    /// 돌려주는 어댑터(runtime.restore_session_artifacts 위임 검증용).
    struct RestoreAdapter {
        provider: ObserverProvider,
        owns_suffix: &'static str,
        calls: std::sync::Mutex<Vec<std::path::PathBuf>>,
    }

    impl ObserverAdapter for RestoreAdapter {
        fn provider(&self) -> ObserverProvider {
            self.provider
        }
        fn prepare_session(
            &self,
            _context: &ObserverSessionContext,
        ) -> Result<AdapterSessionPlan, ObserverAdapterError> {
            Ok(AdapterSessionPlan::default())
        }
        fn map_hook(&self, _raw: &RawObserverHook<'_>) -> Option<ObserverEvent> {
            None
        }
        fn restore_session_artifact(
            &self,
            path: &std::path::Path,
        ) -> Result<bool, ObserverAdapterError> {
            self.calls.lock().unwrap().push(path.to_path_buf());
            Ok(path
                .to_string_lossy()
                .ends_with(self.owns_suffix))
        }
    }

    #[test]
    fn restore_delegates_to_the_owning_adapter_and_skips_empty() {
        let claude = Arc::new(RestoreAdapter {
            provider: ObserverProvider::Claude,
            owns_suffix: ".settings.json",
            calls: std::sync::Mutex::new(vec![]),
        });
        let codex = Arc::new(RestoreAdapter {
            provider: ObserverProvider::Codex,
            owns_suffix: ".never",
            calls: std::sync::Mutex::new(vec![]),
        });
        let runtime = ObserverRuntime::new(test_hub(), vec![claude.clone(), codex.clone()]);

        // 빈 cleanup_paths(observer OFF 세션)는 no-op — 어떤 어댑터도 안 부른다.
        runtime.restore_session_artifacts("s0", &[]);
        assert!(claude.calls.lock().unwrap().is_empty());
        assert!(codex.calls.lock().unwrap().is_empty());

        // claude 소관 경로: claude가 Ok(true)를 주면 거기서 멈춘다(codex 미호출).
        let path = std::path::PathBuf::from("/data/observer/claude/ao-s1.settings.json");
        runtime.restore_session_artifacts("s1", std::slice::from_ref(&path));
        assert_eq!(claude.calls.lock().unwrap().as_slice(), std::slice::from_ref(&path));
        assert!(codex.calls.lock().unwrap().is_empty(), "stop at first owner");
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
                mapped: Some(ObserverEvent::Tool {
                    text: None,
                    assistant: None,
                }),
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
