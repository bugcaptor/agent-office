// src-tauri/tests/contract_fixtures.rs
//
// R-9 옵션 A: Rust<->TS 타입 계약 테스트, Rust 쪽 절반.
//
// `src/shared/contract-fixtures/*.json`은 렌더러(TS)와 백엔드(Rust)가 함께
// 검증하는 공유 픽스처다. 여기서는 각 픽스처가 실제 serde 출력/입력과
// 정확히 일치하는지를 확인한다. 문자열 비교가 아니라 `serde_json::Value`
// 동등 비교를 쓴다 — 키 순서 무관, casing·null-vs-생략까지 검증된다.
//
// Serialize + Deserialize 둘 다 구현한 타입(AgentProfile/PersistedState,
// SessionEventRecord, BotStatus류, AppSettings)은 완전 왕복
// (from_str::<T> -> to_value -> 픽스처와 비교)을 한다.
//
// 이벤트/응답 타입 다수는 backend->renderer 단방향이라 `Serialize`만
// derive돼 있다(`Deserialize`가 없다) -- 이런 타입은 픽스처와 동등한 Rust
// 값을 직접 만들어 `to_value`로 직렬화한 뒤 픽스처를 일반 `Value`로 파싱한
// 것과 비교한다(수신 방향 대신 "우리가 이 모양으로 내보낸다"를 검증).
//
// `CreateSessionRequest`는 반대로 `Deserialize`만 있다(renderer->backend
// 전용) -- 이 경우 `from_str::<T>`로 역직렬화 성공 + 필드값 검증만 한다.

use agent_office_lib::ipc::commands::settings::GetAppSettingsResult;
use agent_office_lib::persistence::settings_store::{
    AppSettings, ExternalEditor, ExternalTerminal, SummaryProvider,
};
use agent_office_lib::session_events::types::SessionEventRecord;
use agent_office_lib::types::{
    ActivityEvent, ActivityKind, AdoptedSessionInfo, AgentProfile, BotAgentStatus, BotPhase,
    BotStatus, CreateSessionRequest, CreateSessionResult, NotificationEvent, NotificationSource,
    OutputChunk, PersistedState, SessionExitInfo, SessionState, SessionStateEvent,
};
use agent_office_lib::usage::{Provider, ProviderUsage, UsageSnapshot, UsageWindow, UsageWindowKind};
use agent_office_lib::workdir::{GitCommitEntry, GitFileHistoryResult, GitFileStatus, GitStatusResult};

use serde_json::Value;

/// 픽스처 파일을 컴파일타임에 문자열로 포함한다.
macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../src/shared/contract-fixtures/",
            $name
        ))
    };
}

/// 픽스처 JSON 문자열과 임의의 `serde_json::Value`가 동등한지 검증한다
/// (키 순서 무관, 파싱된 구조 비교).
fn assert_value_eq(fixture_json: &str, actual: Value) {
    let expected: Value = serde_json::from_str(fixture_json).expect("fixture must be valid JSON");
    assert_eq!(actual, expected, "fixture JSON:\n{fixture_json}");
}

/// Serialize + Deserialize 둘 다 갖춘 타입의 완전 왕복 검증:
/// 픽스처 -> T -> Value, 그리고 픽스처를 그대로 Value로 파싱한 것과 비교.
fn assert_roundtrip<T>(fixture_json: &str)
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let parsed: T = serde_json::from_str(fixture_json)
        .unwrap_or_else(|e| panic!("deserialize failed: {e}\nfixture:\n{fixture_json}"));
    let actual = serde_json::to_value(&parsed).expect("serialize back to Value");
    assert_value_eq(fixture_json, actual);
}

// ---------------------------------------------------------------------
// Serialize-only 타입: 픽스처와 동등한 값을 직접 구성해 to_value 비교.
// ---------------------------------------------------------------------

#[test]
fn session_state_event_exit_matches_fixture() {
    let fixture_json = fixture!("session-state-event.exit.json");
    let value = SessionStateEvent {
        session_id: "s1".into(),
        agent_id: "a1".into(),
        state: SessionState::Exited,
        exit: Some(SessionExitInfo {
            session_id: "s1".into(),
            exit_code: Some(1),
            signal: None,
            intentional: false,
        }),
        at: 1720000000001,
    };
    assert_value_eq(fixture_json, serde_json::to_value(&value).unwrap());
}

#[test]
fn notification_event_matches_fixture() {
    let fixture_json = fixture!("notification-event.json");
    let value = NotificationEvent {
        id: "n1".into(),
        session_id: "s1".into(),
        agent_id: "a1".into(),
        source: NotificationSource::Hook,
        message: "needs input".into(),
        dedup_key: "hook:s1".into(),
        at: 1720000000002,
    };
    assert_value_eq(fixture_json, serde_json::to_value(&value).unwrap());
}

#[test]
fn output_chunk_matches_fixture() {
    let fixture_json = fixture!("output-chunk.json");
    let value = OutputChunk {
        session_id: "s1".into(),
        agent_id: "a1".into(),
        data: "hello".into(),
        frames: 3,
        seq: 42,
        bytes: 5,
    };
    assert_value_eq(fixture_json, serde_json::to_value(&value).unwrap());
}

#[test]
fn create_session_result_matches_fixture() {
    let fixture_json = fixture!("create-session-result.json");
    let value = CreateSessionResult {
        session_id: "s1".into(),
        state: SessionState::Starting,
    };
    assert_value_eq(fixture_json, serde_json::to_value(&value).unwrap());
}

#[test]
fn adopted_session_info_matches_fixture() {
    let fixture_json = fixture!("adopted-session-info.json");
    let value = AdoptedSessionInfo {
        agent_id: "a1".into(),
        session_id: "s1".into(),
        rows: 24,
        cols: 80,
    };
    assert_value_eq(fixture_json, serde_json::to_value(&value).unwrap());
}

#[test]
fn activity_event_prompt_matches_fixture() {
    let fixture_json = fixture!("activity-event.prompt.json");
    let value = ActivityEvent {
        agent_id: "a1".into(),
        session_id: "s1".into(),
        kind: ActivityKind::Prompt,
        at: 1720000000005,
        text: Some("버그 고쳐줘".into()),
        assistant_text: None,
        cwd: None,
        count: None,
    };
    assert_value_eq(fixture_json, serde_json::to_value(&value).unwrap());
}

#[test]
fn usage_snapshot_matches_fixture() {
    let fixture_json = fixture!("usage-snapshot.json");
    let value = UsageSnapshot {
        claude: Some(ProviderUsage {
            provider: Provider::Claude,
            fetched_at_ms: 1784281391475,
            plan_label: Some("max_20x".into()),
            windows: vec![
                UsageWindow {
                    kind: UsageWindowKind::Session,
                    label: None,
                    used_percent: 61.0,
                    resets_at_ms: Some(1784281800243),
                    window_minutes: None,
                    is_active: Some(true),
                },
                UsageWindow {
                    kind: UsageWindowKind::WeeklyModel,
                    label: Some("Fable".into()),
                    used_percent: 24.0,
                    resets_at_ms: Some(1784606400000),
                    window_minutes: None,
                    is_active: Some(false),
                },
            ],
        }),
        codex: Some(ProviderUsage {
            provider: Provider::Codex,
            fetched_at_ms: 1784287217595,
            plan_label: Some("prolite".into()),
            windows: vec![UsageWindow {
                kind: UsageWindowKind::Weekly,
                label: None,
                used_percent: 11.0,
                resets_at_ms: Some(1784786662000),
                window_minutes: Some(10080),
                is_active: None,
            }],
        }),
    };
    assert_value_eq(fixture_json, serde_json::to_value(&value).unwrap());
}

#[test]
fn get_app_settings_result_matches_fixture() {
    let fixture_json = fixture!("get-app-settings-result.json");
    let value = GetAppSettingsResult {
        settings: AppSettings {
            version: 1,
            summarizer_enabled: false,
            summary_provider: SummaryProvider::Claude,
            diary_enabled: false,
            observer_enabled: false,
            sound_enabled: true,
            sound_volume: 0.5,
            external_terminal: ExternalTerminal::Terminal,
            external_editor: ExternalEditor::System,
            attention_hold_ms: 5000,
            git_status_enabled: true,
            cli_enabled: false,
        },
        first_run: true,
    };
    assert_value_eq(fixture_json, serde_json::to_value(&value).unwrap());
}

#[test]
fn git_status_result_matches_fixture() {
    let fixture_json = fixture!("git-status-result.json");
    let value = GitStatusResult {
        is_repo: true,
        branch: Some("main".into()),
        ahead: 2,
        behind: 0,
        entries: vec![GitFileStatus {
            path: "src/lib.rs".into(),
            status: "M".into(),
            xy: " M".into(),
        }],
        timed_out: false,
    };
    assert_value_eq(fixture_json, serde_json::to_value(&value).unwrap());
}

#[test]
fn git_file_history_result_matches_fixture() {
    let fixture_json = fixture!("git-file-history-result.json");
    let value = GitFileHistoryResult {
        commits: vec![GitCommitEntry {
            hash: "abcdef0123456789abcdef0123456789abcdef01".into(),
            short_hash: "abcdef0".into(),
            author: "bugcaptor".into(),
            date: "2026-07-19".into(),
            subject: "fix: something".into(),
        }],
        has_more: false,
        timed_out: false,
    };
    assert_value_eq(fixture_json, serde_json::to_value(&value).unwrap());
}

// ---------------------------------------------------------------------
// Deserialize-only 타입: 역직렬화 성공 + 필드값 검증(재직렬화 불가).
// ---------------------------------------------------------------------

#[test]
fn create_session_request_deserializes_from_fixture() {
    let fixture_json = fixture!("create-session-request.json");
    let value: CreateSessionRequest = serde_json::from_str(fixture_json)
        .unwrap_or_else(|e| panic!("deserialize failed: {e}\nfixture:\n{fixture_json}"));
    assert_eq!(value.agent_id, "a1");
    assert_eq!(value.cols, Some(80));
    assert_eq!(value.rows, Some(24));
    assert_eq!(value.cwd.as_deref(), Some("/tmp/proj"));
    assert_eq!(value.shell.as_deref(), Some("pwsh"));
    assert_eq!(value.startup_command.as_deref(), Some("echo hi"));
    assert_eq!(value.personality_prompt.as_deref(), Some("친절하게 대답해"));
    assert_eq!(value.autostart_claude, Some(false));
}

// ---------------------------------------------------------------------
// Serialize + Deserialize 둘 다 있는 타입: 완전 왕복.
// ---------------------------------------------------------------------

#[test]
fn persisted_state_full_roundtrips() {
    assert_roundtrip::<PersistedState>(fixture!("persisted-state.full.json"));
}

#[test]
fn persisted_state_minimal_roundtrips() {
    assert_roundtrip::<PersistedState>(fixture!("persisted-state.minimal.json"));
}

#[test]
fn persisted_state_minimal_agent_profile_has_no_cwd_or_startup_command() {
    // skip_serializing_if 필드가 minimal 픽스처에서 실제로 부재함을 재확인
    // (왕복 동등 비교와 별개로, "부재"가 아니라 "null"로 새는 회귀를 잡는다).
    let parsed: PersistedState =
        serde_json::from_str(fixture!("persisted-state.minimal.json")).unwrap();
    let profile: &AgentProfile = &parsed.agents[0];
    assert!(profile.cwd.is_none());
    assert!(profile.startup_command.is_none());
}

#[test]
fn session_event_record_started_roundtrips() {
    assert_roundtrip::<SessionEventRecord>(fixture!("session-event-record.started.json"));
}

#[test]
fn session_event_record_tool_roundtrips() {
    assert_roundtrip::<SessionEventRecord>(fixture!("session-event-record.tool.json"));
}

#[test]
fn bot_status_roundtrips() {
    assert_roundtrip::<BotStatus>(fixture!("bot-status.json"));
}

#[test]
fn bot_status_agent_fields_match_fixture() {
    // BTreeMap 순서/필드값을 직접도 확인 -- to_value 비교가 구조를 보장하지만
    // 의미 있는 값(phase enum casing 등)도 사람이 읽을 수 있게 재확인.
    let parsed: BotStatus = serde_json::from_str(fixture!("bot-status.json")).unwrap();
    let a1: &BotAgentStatus = parsed.agents.get("a1").expect("a1 present");
    assert!(a1.running);
    assert_eq!(a1.phase, BotPhase::Working);
    assert_eq!(a1.issue, Some(42));
    assert_eq!(a1.slug.as_deref(), Some("nova"));
    assert_eq!(a1.poll_interval_sec, 60);
    assert_eq!(a1.last_poll_at_ms, Some(1720000000000));
    assert!(a1.error.is_none());
}
