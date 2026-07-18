//! src-tauri/src/types.rs
//!
//! Serde mirror of `src/shared/types.ts` (the source of truth). Field-mapping
//! rules: structs use `#[serde(rename_all = "camelCase")]`, enums use
//! `#[serde(rename_all = "lowercase")]`, and `Option<T>` fields use
//! `skip_serializing_if = "Option::is_none"` to mirror `T | undefined`.

use serde::{Deserialize, Serialize};

pub type AgentId = String;
pub type SessionId = String;

/// 세션 라이프사이클 상태. TS SessionState('starting'|'running'|'exited'|'disposed')와 동일.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Starting,
    Running,
    Exited,
    Disposed,
}

/// 세션 종료 사유. Exited/Disposed 전이 시 동반.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionExitInfo {
    pub session_id: SessionId,
    /// portable-pty ExitStatus.exit_code()를 i32로.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// portable-pty는 크로스플랫폼 ExitStatus에서 시그널을 분리 노출하지 않는다 → 항상 None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
    /// true=앱이 의도적으로 kill(dispose/quit), false=예기치 않은 종료.
    pub intentional: bool,
}

/// 세션 상태 전이 브로드캐스트. 이벤트 "session-state".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStateEvent {
    pub session_id: SessionId,
    pub agent_id: AgentId,
    pub state: SessionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit: Option<SessionExitInfo>,
    pub at: u64,
}

/// 알림 출처. TS NotificationSource와 동일.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationSource {
    Hook,
    Stop,
    Bell,
}

impl NotificationSource {
    /// dedupKey 계산용 안정 문자열.
    pub fn as_key(self) -> &'static str {
        match self {
            NotificationSource::Hook => "hook",
            NotificationSource::Stop => "stop",
            NotificationSource::Bell => "bell",
        }
    }
}

/// 정규화된 알림 이벤트. hook POST/BEL 모두 이 형태로 수렴. 이벤트 "notification-new".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationEvent {
    pub id: String, // uuid v4, NotificationHub가 발급 (렌더러 재발급 금지)
    pub session_id: SessionId,
    pub agent_id: AgentId,
    pub source: NotificationSource,
    pub message: String,
    pub dedup_key: String,
    pub at: u64,
}

/// activity 신호 종류. TS ActivityKind와 동일.
/// prompt = UserPromptSubmit(턴 시작), tool = PostToolUse(하트비트).
/// sub-start = PreToolUse:Task(서브에이전트 소환), sub-stop = SubagentStop(종료),
/// sub-count = 현재 실행 중 서브에이전트 절대 수.
/// resume = 완료 알림 이후 출력이 계속 쏟아져 "아직 작업중"으로 복귀시키는 신호
/// (NotificationHub의 출력 휴리스틱이 방출, 이슈 #39). 세 sub-* 는 카운트 기반
/// 미니 캐릭터 전용이라 시간 추적/시계열엔 기록하지 않지만, resume 은 렌더러의
/// 턴 상태를 working 으로 되돌리는 신호로 쓰인다(tool 과 동일하게 취급).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityKind {
    Prompt,
    Tool,
    #[serde(rename = "sub-start")]
    SubStart,
    #[serde(rename = "sub-stop")]
    SubStop,
    #[serde(rename = "sub-count")]
    SubCount,
    Resume,
}

/// 세션 시간 추적용 활동 이벤트. NotificationHub의 dedup/큐를 우회해
/// "activity-event"로 렌더러 직행. TS ActivityEvent와 1:1.
/// at은 백엔드 now_ms() epoch ms — 렌더러 정산의 유일한 시계.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub kind: ActivityKind,
    pub at: u64,
    /// kind=Prompt일 때 사용자 프롬프트 원문(최대 2,000자, chars 기준 절단),
    /// kind=Tool일 때 도구 요약("Bash: npm test" 등, 최대 60자). 부재 시 None —
    /// None이면 wire에서 필드 생략.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// kind=Tool일 때 턴 중간 assistant 내레이션(claude transcript 꼬리, 스로틀
    /// 적용). 그 외 kind/codex/부재는 None — None이면 wire에서 필드 생략.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assistant_text: Option<String>,
    /// kind=Prompt일 때 훅 body top-level cwd(세션 실제 작업 디렉터리, 라벨
    /// 프로젝트명 표시용, 이슈 #44 작업 D). 그 외 kind/부재는 None — None이면 wire에서 필드 생략.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// kind=SubCount일 때 현재 실행 중 서브에이전트 절대 수.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
}

/// 완료된 턴 1건의 시계열 기록. TS `SessionTurnRecord` 미러.
/// 모든 시각은 백엔드 epoch ms. append-only 로그(session-times.jsonl)의 한 줄.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTurnRecord {
    pub agent_id: AgentId,
    pub started_at: u64,
    pub ended_at: u64,
    pub total_ms: u64,
    pub worked_ms: u64,
    pub waited_ms: u64,
}

/// renderer→backend 세션 생성 옵션. 프런트 AgentOfficeApi.createSession(agentId, opts?).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionRequest {
    pub agent_id: AgentId,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub cwd: Option<String>,
    /// 프로필의 셸 선택 id("powershell" | "pwsh" | "git-bash" | "wsl").
    /// None이면 자동 선택(`session::shells::resolve_observed`가 pwsh > powershell
    /// 순으로 고른다). Windows 전용 기능 -- 다른 플랫폼에서는 무시된다.
    pub shell: Option<String>,
    /// 세션이 Running으로 전이한 뒤 셸 stdin에 `{command}\n`으로 주입할 시작 명령어.
    /// None/공백이면 미주입. 셸 문법(bat/sh/pwsh 등)은 사용자가 선택 셸에 맞게 작성.
    pub startup_command: Option<String>,
    /// Claude Code에 `--append-system-prompt`로 전달할 캐릭터 성격 프롬프트.
    pub personality_prompt: Option<String>,
    /// 동결 API opts에는 없음 → 프런트 어댑터는 항상 미지정(=false). 기본 false:
    /// 세션은 자동 실행 없이 셸만 띄운다. Observation이 켜진 세션은 adapter가
    /// 제공한 command wrapper specs를 PowerShell 함수, Git Bash `--rcfile`,
    /// 또는 zsh ZDOTDIR shim으로 렌더링한다.
    pub autostart_claude: Option<bool>,
}

/// createSession 응답.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionResult {
    pub session_id: SessionId,
    pub state: SessionState,
}

/// 세션 핸드오프(docs/session-handoff-design.md): 부트스트랩 시
/// `adopt_detached_sessions` 커맨드가 되찾은 세션 하나. 프론트는 이 목록으로
/// 상태를 Running 시드하고, 터미널을 재부착할 때 rows/cols로 redraw nudge를
/// 수행한다(§프론트).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptedSessionInfo {
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub rows: u16,
    pub cols: u16,
}

/// PTY 출력 청크(배치). backend→webview, tauri::ipc::Channel로 전송.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputChunk {
    pub session_id: SessionId,
    pub agent_id: AgentId, // 렌더러 필터링용
    pub data: String,      // UTF-8. OutputBatcher가 이어붙인 결과(경계 캐리 처리됨)
    pub frames: u32,       // 담은 원본 read 이벤트 수(진단용)
    pub seq: u64,          // 세션별 단조 증가
}

/// 알림 클리어됨 브로드캐스트. 이벤트 "notification-cleared".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationClearedEvent {
    pub agent_id: AgentId,
    pub ids: Vec<String>,
}

/// 프로필 스키마(단일 정의).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub role: String,
    pub note: String,
    pub seed: String,
    pub created_at: u64,
    pub desk_index: u32,
    /// 사용자가 책상 클릭으로 수동 지정한 책상 인덱스. 없으면 자동(해시) 배정.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub assigned_desk_index: Option<u32>,
    /// 세션 작업 디렉터리. 미지정 시 백엔드가 홈 디렉터리로 폴백(manager.rs).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cwd: Option<String>,
    /// 외모 묘사 힌트(자유 텍스트). 이미지 프롬프트에 반영. 없으면 프롬프트에서 생략.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub appearance: Option<String>,
    /// 초상 존재 표시 + 프론트 캐시 무효화 키(epoch ms). 없으면 초상 없음.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub portrait_updated_at: Option<u64>,
    /// 픽셀아트 프롬프트 의뢰 문구(자유 텍스트). 비면 appearance로 폴백.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sprite_request: Option<String>,
    /// 커스텀 스프라이트 존재 표시 + 프론트 캐시 무효화 키(epoch ms). 없으면 절차 생성 사용.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sprite_updated_at: Option<u64>,
    /// 캐릭터 아키타입(종족) id. 부재 = 레거시(로드 시 "human" 백필), 알 수 없음 = "human" 폴백.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub archetype: Option<String>,
    /// 세션 셸 선택 id("powershell" | "pwsh" | "git-bash" | "wsl"). 없으면
    /// 자동 선택(session::shells::resolve_observed). Windows 전용 기능.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub shell: Option<String>,
    /// 새 세션이 뜰 때마다 셸 stdin에 주입할 시작 명령어. 없으면 미주입.
    /// 예: "source ./init.sh", "mysetup.bat". 셸 문법은 사용자 책임.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub startup_command: Option<String>,
    /// Claude Code 세션에 추가 시스템 프롬프트로 주입할 캐릭터 성격(멀티라인 가능).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub personality_prompt: Option<String>,
    /// 퇴근(clock-out) 상태. Some(true)면 오피스/터미널에서 숨기고 소환 목록에만
    /// 남긴다. 부재/false = 근무 중. TS `clockedOut?: boolean` 미러.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub clocked_out: Option<bool>,
    /// 키보드 사운드 팩 id (렌더러 sound/packs.ts 참고). 없음/무효 = 기본 팩.
    /// TS `keyboardSound?: string` 미러.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub keyboard_sound: Option<String>,
}

/// 영속 상태. version은 리터럴 1.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedState {
    pub agents: Vec<AgentProfile>,
    pub version: u32,
}

impl PersistedState {
    pub fn empty() -> Self {
        Self {
            agents: Vec::new(),
            version: 1,
        }
    }
}

/// epoch ms 헬퍼.
pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Step 3: enum roundtrip snapshots (lowercase) ----

    #[test]
    fn session_state_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&SessionState::Starting).unwrap(),
            "\"starting\""
        );
        assert_eq!(
            serde_json::to_string(&SessionState::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&SessionState::Exited).unwrap(),
            "\"exited\""
        );
        assert_eq!(
            serde_json::to_string(&SessionState::Disposed).unwrap(),
            "\"disposed\""
        );
    }

    #[test]
    fn session_state_roundtrips_from_ts_literal() {
        // Must deserialize the exact literal a TS `SessionState` union would send.
        let s: SessionState = serde_json::from_str("\"running\"").unwrap();
        assert_eq!(s, SessionState::Running);
    }

    #[test]
    fn notification_source_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&NotificationSource::Hook).unwrap(),
            "\"hook\""
        );
        assert_eq!(
            serde_json::to_string(&NotificationSource::Stop).unwrap(),
            "\"stop\""
        );
        assert_eq!(
            serde_json::to_string(&NotificationSource::Bell).unwrap(),
            "\"bell\""
        );
    }

    #[test]
    fn activity_kind_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&ActivityKind::Prompt).unwrap(), "\"prompt\"");
        assert_eq!(serde_json::to_string(&ActivityKind::Tool).unwrap(), "\"tool\"");
        assert_eq!(serde_json::to_string(&ActivityKind::Resume).unwrap(), "\"resume\"");
        let r: ActivityKind = serde_json::from_str("\"resume\"").unwrap();
        assert_eq!(r, ActivityKind::Resume);
    }

    #[test]
    fn activity_kind_serializes_subagent_variants_as_kebab() {
        assert_eq!(serde_json::to_string(&ActivityKind::SubStart).unwrap(), "\"sub-start\"");
        assert_eq!(serde_json::to_string(&ActivityKind::SubStop).unwrap(), "\"sub-stop\"");
    }

    #[test]
    fn activity_kind_deserializes_subagent_variants_from_ts_literal() {
        let a: ActivityKind = serde_json::from_str("\"sub-start\"").unwrap();
        let b: ActivityKind = serde_json::from_str("\"sub-stop\"").unwrap();
        let c: ActivityKind = serde_json::from_str("\"sub-count\"").unwrap();
        assert_eq!(a, ActivityKind::SubStart);
        assert_eq!(b, ActivityKind::SubStop);
        assert_eq!(c, ActivityKind::SubCount);
        assert_eq!(serde_json::to_string(&c).unwrap(), "\"sub-count\"");
    }

    #[test]
    fn activity_event_keys_are_camel_case() {
        let ev = ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: ActivityKind::Prompt,
            at: 1_720_000_000_000,
            text: None,
            assistant_text: None,
            cwd: None,
            count: None,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            json,
            "{\"agentId\":\"a1\",\"sessionId\":\"s1\",\"kind\":\"prompt\",\"at\":1720000000000}"
        );
    }

    #[test]
    fn activity_event_omits_cwd_when_none_and_serializes_when_some() {
        // 이슈 #44 작업 D: cwd는 None이면 wire에서 생략, Some이면 camelCase로 실린다.
        let ev = ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: ActivityKind::Prompt,
            at: 1,
            text: None,
            assistant_text: None,
            cwd: None,
            count: None,
        };
        let j = serde_json::to_string(&ev).unwrap();
        assert!(!j.contains("\"cwd\""), "None이면 필드 자체가 생략돼야 한다: {j}");

        let ev2 = ActivityEvent {
            cwd: Some("/w/project".into()),
            ..ev
        };
        let j2 = serde_json::to_string(&ev2).unwrap();
        assert!(j2.contains(r#""cwd":"/w/project""#), "{j2}");
    }

    #[test]
    fn activity_event_omits_text_when_none_and_serializes_when_some() {
        let ev = ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: ActivityKind::Prompt,
            at: 1,
            text: None,
            assistant_text: None,
            cwd: None,
            count: None,
        };
        let j = serde_json::to_string(&ev).unwrap();
        assert!(!j.contains("\"text\""), "None이면 필드 자체가 생략돼야 한다: {j}");

        let ev2 = ActivityEvent { text: Some("고쳐줘".into()), ..ev };
        let j2 = serde_json::to_string(&ev2).unwrap();
        assert!(j2.contains(r#""text":"고쳐줘""#), "{j2}");
    }

    #[test]
    fn activity_event_omits_count_when_none_and_serializes_when_some() {
        let ev = ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: ActivityKind::SubCount,
            at: 1,
            text: None,
            assistant_text: None,
            cwd: None,
            count: None,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(!json.contains("\"count\""), "{json}");

        let counted = ActivityEvent {
            count: Some(2),
            ..ev
        };
        let counted_json = serde_json::to_string(&counted).unwrap();
        assert!(counted_json.contains(r#""count":2"#), "{counted_json}");
    }

    // ---- Step 3: struct roundtrip snapshots (camelCase keys) ----

    #[test]
    fn notification_event_keys_are_camel_case() {
        let ev = NotificationEvent {
            id: "n1".into(),
            session_id: "s1".into(),
            agent_id: "a1".into(),
            source: NotificationSource::Hook,
            message: "needs input".into(),
            dedup_key: "hook:s1".into(),
            at: 1_720_000_000_000,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            json,
            "{\"id\":\"n1\",\"sessionId\":\"s1\",\"agentId\":\"a1\",\"source\":\"hook\",\
             \"message\":\"needs input\",\"dedupKey\":\"hook:s1\",\"at\":1720000000000}"
        );
    }

    #[test]
    fn session_state_event_omits_absent_exit() {
        let ev = SessionStateEvent {
            session_id: "s1".into(),
            agent_id: "a1".into(),
            state: SessionState::Running,
            exit: None,
            at: 1,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            json,
            "{\"sessionId\":\"s1\",\"agentId\":\"a1\",\"state\":\"running\",\"at\":1}"
        );
        assert!(!json.contains("exit"));
    }

    #[test]
    fn session_state_event_includes_exit_when_present() {
        let ev = SessionStateEvent {
            session_id: "s1".into(),
            agent_id: "a1".into(),
            state: SessionState::Exited,
            exit: Some(SessionExitInfo {
                session_id: "s1".into(),
                exit_code: Some(1),
                signal: None,
                intentional: false,
            }),
            at: 2,
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            json,
            "{\"sessionId\":\"s1\",\"agentId\":\"a1\",\"state\":\"exited\",\
             \"exit\":{\"sessionId\":\"s1\",\"exitCode\":1,\"intentional\":false},\"at\":2}"
        );
        // signal is None -> must be omitted (Option <-> T | undefined mapping).
        assert!(!json.contains("signal"));
    }

    #[test]
    fn output_chunk_camel_case() {
        let chunk = OutputChunk {
            session_id: "s1".into(),
            agent_id: "a1".into(),
            data: "hello".into(),
            frames: 3,
            seq: 42,
        };
        let json = serde_json::to_string(&chunk).unwrap();
        assert_eq!(
            json,
            "{\"sessionId\":\"s1\",\"agentId\":\"a1\",\"data\":\"hello\",\"frames\":3,\"seq\":42}"
        );
    }

    #[test]
    fn create_session_request_deserializes_camel_case_from_ts() {
        // Matches the wire payload a TS `CreateSessionRequest` would produce.
        let json = "{\"agentId\":\"a1\",\"cols\":80,\"rows\":24,\"cwd\":null,\"autostartClaude\":null}";
        let req: CreateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.agent_id, "a1");
        assert_eq!(req.cols, Some(80));
        assert_eq!(req.rows, Some(24));
        assert_eq!(req.cwd, None);
        assert_eq!(req.autostart_claude, None);
    }

    #[test]
    fn create_session_request_allows_omitted_optionals() {
        let json = "{\"agentId\":\"a1\"}";
        let req: CreateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.agent_id, "a1");
        assert_eq!(req.cols, None);
        assert_eq!(req.rows, None);
        assert_eq!(req.cwd, None);
        assert_eq!(req.autostart_claude, None);
    }

    #[test]
    fn create_session_result_camel_case() {
        let res = CreateSessionResult {
            session_id: "s1".into(),
            state: SessionState::Starting,
        };
        assert_eq!(
            serde_json::to_string(&res).unwrap(),
            "{\"sessionId\":\"s1\",\"state\":\"starting\"}"
        );
    }

    #[test]
    fn notification_cleared_event_camel_case() {
        let ev = NotificationClearedEvent {
            agent_id: "a1".into(),
            ids: vec!["n1".into(), "n2".into()],
        };
        assert_eq!(
            serde_json::to_string(&ev).unwrap(),
            "{\"agentId\":\"a1\",\"ids\":[\"n1\",\"n2\"]}"
        );
    }

    #[test]
    fn agent_profile_and_persisted_state_roundtrip() {
        let json = "{\"agents\":[{\"id\":\"p1\",\"name\":\"Ada\",\"role\":\"backend\",\
                     \"note\":\"\",\"seed\":\"abc123\",\"createdAt\":1720000000003,\
                     \"deskIndex\":0}],\"version\":1}";
        let parsed: PersistedState = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.agents.len(), 1);
        assert_eq!(parsed.agents[0].id, "p1");
        assert_eq!(parsed.agents[0].desk_index, 0);

        // Roundtrip back out matches the same camelCase shape.
        let out = serde_json::to_string(&parsed).unwrap();
        let reparsed: PersistedState = serde_json::from_str(&out).unwrap();
        assert_eq!(reparsed.agents[0].name, "Ada");
    }

    #[test]
    fn agent_profile_deserializes_without_cwd() {
        // Backward compat: profiles.json files predating the `cwd` field have no
        // `cwd` key at all -> must still deserialize, with cwd == None.
        let json = "{\"id\":\"p1\",\"name\":\"Ada\",\"role\":\"backend\",\"note\":\"\",\
                     \"seed\":\"abc123\",\"createdAt\":1720000000003,\"deskIndex\":0}";
        let profile: AgentProfile = serde_json::from_str(json).unwrap();
        assert_eq!(profile.cwd, None);
    }

    #[test]
    fn agent_profile_roundtrips_assigned_desk_index_and_defaults_to_none() {
        // 수동 책상 지정: 키 부재(레거시) -> None, None은 직렬화에서 생략.
        let json = "{\"id\":\"p1\",\"name\":\"Ada\",\"role\":\"backend\",\"note\":\"\",\
                     \"seed\":\"abc123\",\"createdAt\":1720000000003,\"deskIndex\":0}";
        let profile: AgentProfile = serde_json::from_str(json).unwrap();
        assert_eq!(profile.assigned_desk_index, None);
        assert!(!serde_json::to_string(&profile)
            .unwrap()
            .contains("assignedDeskIndex"));

        let json2 = "{\"id\":\"p1\",\"name\":\"Ada\",\"role\":\"backend\",\"note\":\"\",\
                     \"seed\":\"abc123\",\"createdAt\":1720000000003,\"deskIndex\":0,\
                     \"assignedDeskIndex\":5}";
        let profile2: AgentProfile = serde_json::from_str(json2).unwrap();
        assert_eq!(profile2.assigned_desk_index, Some(5));
        assert!(serde_json::to_string(&profile2)
            .unwrap()
            .contains("\"assignedDeskIndex\":5"));
    }

    #[test]
    fn agent_profile_serializes_cwd_camel_case_when_present() {
        let profile = AgentProfile {
            id: "p1".into(),
            name: "Ada".into(),
            role: "backend".into(),
            note: "".into(),
            seed: "abc123".into(),
            created_at: 1_720_000_000_003,
            desk_index: 0,
            assigned_desk_index: None,
            cwd: Some("/tmp/proj".into()),
            appearance: None,
            portrait_updated_at: None,
            sprite_request: None,
            sprite_updated_at: None,
            archetype: None,
            shell: None,
            startup_command: None,
            clocked_out: None,
            personality_prompt: None,
        keyboard_sound: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("\"cwd\":\"/tmp/proj\""));
    }

    #[test]
    fn agent_profile_omits_cwd_when_none() {
        let profile = AgentProfile {
            id: "p1".into(),
            name: "Ada".into(),
            role: "backend".into(),
            note: "".into(),
            seed: "abc123".into(),
            created_at: 1,
            desk_index: 0,
            assigned_desk_index: None,
            cwd: None,
            appearance: None,
            portrait_updated_at: None,
            sprite_request: None,
            sprite_updated_at: None,
            archetype: None,
            shell: None,
            startup_command: None,
            clocked_out: None,
            personality_prompt: None,
        keyboard_sound: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(!json.contains("cwd"));
    }

    #[test]
    fn agent_profile_deserializes_without_portrait_fields() {
        // 기존(프리-이번기능) profiles.json에는 appearance/portraitUpdatedAt 키가
        // 아예 없다 -> 여전히 파싱되고 둘 다 None 이어야 한다.
        let json = "{\"id\":\"p1\",\"name\":\"Ada\",\"role\":\"backend\",\"note\":\"\",\
                     \"seed\":\"abc123\",\"createdAt\":1720000000003,\"deskIndex\":0}";
        let profile: AgentProfile = serde_json::from_str(json).unwrap();
        assert_eq!(profile.appearance, None);
        assert_eq!(profile.portrait_updated_at, None);
    }

    #[test]
    fn agent_profile_serializes_portrait_fields_camel_case_when_present() {
        let profile = AgentProfile {
            id: "p1".into(),
            name: "Ada".into(),
            role: "backend".into(),
            note: "".into(),
            seed: "abc123".into(),
            created_at: 1,
            desk_index: 0,
            assigned_desk_index: None,
            cwd: None,
            appearance: Some("short black hair, glasses".into()),
            portrait_updated_at: Some(1_720_000_000_777),
            sprite_request: None,
            sprite_updated_at: None,
            archetype: None,
            shell: None,
            startup_command: None,
            clocked_out: None,
            personality_prompt: None,
        keyboard_sound: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("\"appearance\":\"short black hair, glasses\""));
        assert!(json.contains("\"portraitUpdatedAt\":1720000000777"));
    }

    #[test]
    fn agent_profile_omits_portrait_fields_when_none() {
        let profile = AgentProfile {
            id: "p1".into(),
            name: "Ada".into(),
            role: "backend".into(),
            note: "".into(),
            seed: "abc123".into(),
            created_at: 1,
            desk_index: 0,
            assigned_desk_index: None,
            cwd: None,
            appearance: None,
            portrait_updated_at: None,
            sprite_request: None,
            sprite_updated_at: None,
            archetype: None,
            shell: None,
            startup_command: None,
            clocked_out: None,
            personality_prompt: None,
        keyboard_sound: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(!json.contains("appearance"));
        assert!(!json.contains("portraitUpdatedAt"));
    }

    #[test]
    fn agent_profile_deserializes_without_sprite_fields() {
        // 기존 profiles.json에는 spriteRequest/spriteUpdatedAt 키가 없다 ->
        // 여전히 파싱되고 둘 다 None 이어야 한다.
        let json = "{\"id\":\"p1\",\"name\":\"Ada\",\"role\":\"backend\",\"note\":\"\",\
                     \"seed\":\"abc123\",\"createdAt\":1720000000003,\"deskIndex\":0}";
        let profile: AgentProfile = serde_json::from_str(json).unwrap();
        assert_eq!(profile.sprite_request, None);
        assert_eq!(profile.sprite_updated_at, None);
    }

    #[test]
    fn agent_profile_serializes_sprite_fields_camel_case_when_present() {
        let profile = AgentProfile {
            id: "p1".into(),
            name: "Ada".into(),
            role: "backend".into(),
            note: "".into(),
            seed: "abc123".into(),
            created_at: 1,
            desk_index: 0,
            assigned_desk_index: None,
            cwd: None,
            appearance: None,
            portrait_updated_at: None,
            sprite_request: Some("red cloak wizard".into()),
            sprite_updated_at: Some(1_720_000_000_888),
            archetype: None,
            shell: None,
            startup_command: None,
            clocked_out: None,
            personality_prompt: None,
        keyboard_sound: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("\"spriteRequest\":\"red cloak wizard\""));
        assert!(json.contains("\"spriteUpdatedAt\":1720000000888"));
    }

    #[test]
    fn agent_profile_omits_sprite_fields_when_none() {
        let profile = AgentProfile {
            id: "p1".into(),
            name: "Ada".into(),
            role: "backend".into(),
            note: "".into(),
            seed: "abc123".into(),
            created_at: 1,
            desk_index: 0,
            assigned_desk_index: None,
            cwd: None,
            appearance: None,
            portrait_updated_at: None,
            sprite_request: None,
            sprite_updated_at: None,
            archetype: None,
            shell: None,
            startup_command: None,
            clocked_out: None,
            personality_prompt: None,
        keyboard_sound: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(!json.contains("spriteRequest"));
        assert!(!json.contains("spriteUpdatedAt"));
    }

    #[test]
    fn agent_profile_deserializes_without_archetype() {
        // 레거시 profiles.json엔 archetype 키가 없다 -> 파싱되고 None.
        let json = "{\"id\":\"p1\",\"name\":\"Ada\",\"role\":\"backend\",\"note\":\"\",\
                     \"seed\":\"abc123\",\"createdAt\":1720000000003,\"deskIndex\":0}";
        let profile: AgentProfile = serde_json::from_str(json).unwrap();
        assert_eq!(profile.archetype, None);
    }

    #[test]
    fn agent_profile_serializes_archetype_camel_case_when_present() {
        let profile = AgentProfile {
            id: "p1".into(), name: "Ada".into(), role: "backend".into(), note: "".into(),
            seed: "abc123".into(), created_at: 1, desk_index: 0, assigned_desk_index: None, cwd: None, appearance: None,
            portrait_updated_at: None, sprite_request: None, sprite_updated_at: None,
            archetype: Some("orc".into()),
            shell: None,
            startup_command: None,
            clocked_out: None,
            personality_prompt: None,
        keyboard_sound: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("\"archetype\":\"orc\""));
    }

    #[test]
    fn agent_profile_omits_archetype_when_none() {
        let profile = AgentProfile {
            id: "p1".into(), name: "Ada".into(), role: "backend".into(), note: "".into(),
            seed: "abc123".into(), created_at: 1, desk_index: 0, assigned_desk_index: None, cwd: None, appearance: None,
            portrait_updated_at: None, sprite_request: None, sprite_updated_at: None,
            archetype: None,
            shell: None,
            startup_command: None,
            clocked_out: None,
            personality_prompt: None,
        keyboard_sound: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(!json.contains("archetype"));
    }

    #[test]
    fn agent_profile_deserializes_without_keyboard_sound() {
        // 레거시 profiles.json엔 keyboardSound 키가 없다 -> 파싱되고 None.
        let json = "{\"id\":\"p1\",\"name\":\"Ada\",\"role\":\"backend\",\"note\":\"\",\
                     \"seed\":\"abc123\",\"createdAt\":1720000000003,\"deskIndex\":0}";
        let profile: AgentProfile = serde_json::from_str(json).unwrap();
        assert_eq!(profile.keyboard_sound, None);
    }

    #[test]
    fn agent_profile_serializes_keyboard_sound_camel_case_when_present() {
        let profile = AgentProfile {
            id: "p1".into(), name: "Ada".into(), role: "backend".into(), note: "".into(),
            seed: "abc123".into(), created_at: 1, desk_index: 0, assigned_desk_index: None, cwd: None, appearance: None,
            portrait_updated_at: None, sprite_request: None, sprite_updated_at: None,
            archetype: None,
            shell: None,
            startup_command: None,
            clocked_out: None,
            personality_prompt: None,
                       keyboard_sound: Some("topre-hhkb".into()),
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("\"keyboardSound\":\"topre-hhkb\""));
    }

    #[test]
    fn agent_profile_omits_keyboard_sound_when_none() {
        let profile = AgentProfile {
            id: "p1".into(), name: "Ada".into(), role: "backend".into(), note: "".into(),
            seed: "abc123".into(), created_at: 1, desk_index: 0, assigned_desk_index: None, cwd: None, appearance: None,
            portrait_updated_at: None, sprite_request: None, sprite_updated_at: None,
            archetype: None,
            shell: None,
            startup_command: None,
            clocked_out: None,
            personality_prompt: None,
                       keyboard_sound: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(!json.contains("keyboardSound"));
    }

    #[test]
    fn agent_profile_deserializes_without_shell() {
        // 레거시 profiles.json엔 shell 키가 없다 -> 파싱되고 None.
        let json = "{\"id\":\"p1\",\"name\":\"Ada\",\"role\":\"backend\",\"note\":\"\",\
                     \"seed\":\"abc123\",\"createdAt\":1720000000003,\"deskIndex\":0}";
        let profile: AgentProfile = serde_json::from_str(json).unwrap();
        assert_eq!(profile.shell, None);
    }

    #[test]
    fn agent_profile_serializes_shell_when_present() {
        let profile = AgentProfile {
            id: "p1".into(), name: "Ada".into(), role: "backend".into(), note: "".into(),
            seed: "abc123".into(), created_at: 1, desk_index: 0, assigned_desk_index: None, cwd: None, appearance: None,
            portrait_updated_at: None, sprite_request: None, sprite_updated_at: None,
            archetype: None, shell: Some("git-bash".into()), startup_command: None,
            clocked_out: None,
            personality_prompt: None,
        keyboard_sound: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("\"shell\":\"git-bash\""));
    }

    #[test]
    fn agent_profile_omits_shell_when_none() {
        let profile = AgentProfile {
            id: "p1".into(), name: "Ada".into(), role: "backend".into(), note: "".into(),
            seed: "abc123".into(), created_at: 1, desk_index: 0, assigned_desk_index: None, cwd: None, appearance: None,
            portrait_updated_at: None, sprite_request: None, sprite_updated_at: None,
            archetype: None, shell: None, startup_command: None,
            clocked_out: None,
            personality_prompt: None,
        keyboard_sound: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(!json.contains("shell"));
    }

    #[test]
    fn agent_profile_deserializes_without_clocked_out() {
        // 레거시 profiles.json엔 clockedOut 키가 없다 -> 파싱되고 None(=근무 중).
        let json = "{\"id\":\"p1\",\"name\":\"Ada\",\"role\":\"backend\",\"note\":\"\",\
                     \"seed\":\"abc123\",\"createdAt\":1720000000003,\"deskIndex\":0}";
        let profile: AgentProfile = serde_json::from_str(json).unwrap();
        assert_eq!(profile.clocked_out, None);
    }

    #[test]
    fn agent_profile_serializes_clocked_out_when_present() {
        let profile = AgentProfile {
            id: "p1".into(), name: "Ada".into(), role: "backend".into(), note: "".into(),
            seed: "abc123".into(), created_at: 1, desk_index: 0, assigned_desk_index: None, cwd: None, appearance: None,
            portrait_updated_at: None, sprite_request: None, sprite_updated_at: None,
            archetype: None, shell: None, startup_command: None,
            clocked_out: Some(true),
            personality_prompt: None,
        keyboard_sound: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("\"clockedOut\":true"));
    }

    #[test]
    fn agent_profile_omits_clocked_out_when_none() {
        let profile = AgentProfile {
            id: "p1".into(), name: "Ada".into(), role: "backend".into(), note: "".into(),
            seed: "abc123".into(), created_at: 1, desk_index: 0, assigned_desk_index: None, cwd: None, appearance: None,
            portrait_updated_at: None, sprite_request: None, sprite_updated_at: None,
            archetype: None, shell: None, startup_command: None,
            clocked_out: None,
            personality_prompt: None,
        keyboard_sound: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(!json.contains("clockedOut"));
    }

    #[test]
    fn persisted_state_empty_has_version_one() {
        let empty = PersistedState::empty();
        assert_eq!(empty.version, 1);
        assert!(empty.agents.is_empty());
    }

    #[test]
    fn notification_source_as_key_matches_serde_value() {
        assert_eq!(NotificationSource::Hook.as_key(), "hook");
        assert_eq!(NotificationSource::Stop.as_key(), "stop");
        assert_eq!(NotificationSource::Bell.as_key(), "bell");
    }

    #[test]
    fn now_ms_is_plausible_epoch_millis() {
        // Sanity bound: must be after 2020-01-01 and not absurdly far in the future.
        let ms = now_ms();
        assert!(ms > 1_577_836_800_000);
    }

    #[test]
    fn session_turn_record_serializes_camel_case() {
        let rec = SessionTurnRecord {
            agent_id: "a1".into(),
            started_at: 1_000,
            ended_at: 4_000,
            total_ms: 3_000,
            worked_ms: 2_000,
            waited_ms: 1_000,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"agentId\":\"a1\""), "{json}");
        assert!(json.contains("\"startedAt\":1000"), "{json}");
        assert!(json.contains("\"endedAt\":4000"), "{json}");
        assert!(json.contains("\"totalMs\":3000"), "{json}");
        assert!(json.contains("\"workedMs\":2000"), "{json}");
        assert!(json.contains("\"waitedMs\":1000"), "{json}");
    }

    #[test]
    fn session_turn_record_roundtrips() {
        let rec = SessionTurnRecord {
            agent_id: "a1".into(),
            started_at: 1_000,
            ended_at: 4_000,
            total_ms: 3_000,
            worked_ms: 2_000,
            waited_ms: 1_000,
        };
        let json = serde_json::to_string(&rec).unwrap();
        let parsed: SessionTurnRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, rec);
    }
}
