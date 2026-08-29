use std::path::PathBuf;

use crate::types::SessionEventTokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObserverProvider {
    Claude,
    Codex,
}

impl ObserverProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObserverEvent {
    Prompt {
        text: Option<String>,
        /// 훅 body top-level cwd — 라벨 프로젝트명 표시용(이슈 #44 작업 D). codex 등
        /// body에 cwd가 없으면 None.
        cwd: Option<String>,
    },
    Tool {
        /// 도구 요약("Bash: npm test" 등). 파싱 실패/서브에이전트 이벤트는 None.
        text: Option<String>,
        /// 턴 중간 assistant 내레이션(claude transcript 꼬리, 스로틀 적용). codex는 항상 None.
        assistant: Option<String>,
    },
    SubStart,
    SubStop,
    SubCount {
        running: u32,
    },
    Attention {
        message: Option<String>,
    },
    Stop {
        message: Option<String>,
        running: Option<u32>,
        /// 이 턴이 쓴 토큰(전사/rollout에서 뽑아 실음). 추출 실패·미지원
        /// 제공자는 None — 시계열에 tokens 필드 자체가 생기지 않는다.
        tokens: Option<SessionEventTokens>,
    },
}

pub struct RawObserverHook<'a> {
    pub event_name: &'a str,
    pub body: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WrapperArg {
    Literal(String),
    Env(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandWrapperSpec {
    pub command: String,
    pub prefix_args: Vec<WrapperArg>,
    pub skip_if_present: Vec<String>,
    /// Some(env_name)이면, 렌더된 래퍼는 prefix를 붙이기 전에 그 env가 가리키는
    /// 파일의 존재를 확인한다 — 없으면 경고 후 prefix 없이 원본 명령을 실행한다
    /// (이슈 #40). observer 설정 파일이 사라진 셸에서 claude가 `--settings <없는
    /// 파일>`로 하드 실패하는 대신 비관찰로 강등해 실행을 보장한다. prefix가
    /// 비어 있으면 의미가 없으므로 렌더러가 무시한다.
    pub skip_prefix_if_env_file_missing: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserverSessionContext {
    pub session_id: String,
    pub hook_url: String,
}

impl ObserverSessionContext {
    pub fn new(session_id: impl Into<String>, hook_url: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            hook_url: hook_url.into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdapterSessionPlan {
    pub env: Vec<(String, String)>,
    pub wrappers: Vec<CommandWrapperSpec>,
    pub cleanup_paths: Vec<PathBuf>,
}

impl AdapterSessionPlan {
    pub fn merge(&mut self, mut other: Self) {
        self.env.append(&mut other.env);
        self.wrappers.append(&mut other.wrappers);
        self.cleanup_paths.append(&mut other.cleanup_paths);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserverAdapterError(String);

impl ObserverAdapterError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for ObserverAdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub const MAX_PROMPT_TEXT_CHARS: usize = 2_000;

fn is_command_prompt(value: &str) -> bool {
    value.starts_with('!') || value.starts_with('/') || value.starts_with('#')
}

pub fn prompt_text(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let prompt = value.get("prompt")?.as_str()?.trim();
    if prompt.is_empty() || is_command_prompt(prompt) {
        return None;
    }
    Some(prompt.chars().take(MAX_PROMPT_TEXT_CHARS).collect())
}

pub fn message(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let message = value.get("message")?.as_str()?;
    (!message.trim().is_empty()).then(|| message.to_string())
}

pub fn agent_id(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let agent_id = value.get("agent_id")?.as_str()?;
    (!agent_id.trim().is_empty()).then(|| agent_id.to_string())
}

/// Claude 훅 body의 top-level `session_id`(= native 리줌 ID). 모든 이벤트마다
/// 실려 오므로 종료 전에도 캡처할 수 있다(docs/claude-session-resume-design.md §2).
/// 공백/빈 값은 None.
pub fn native_session_id(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let session_id = value.get("session_id")?.as_str()?;
    (!session_id.trim().is_empty()).then(|| session_id.to_string())
}

/// Claude 훅 body의 top-level `cwd`(리줌은 같은 프로젝트 디렉터리에서만 가능해
/// 함께 저장해 둔다). 공백/빈 값은 None.
pub fn hook_cwd(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let cwd = value.get("cwd")?.as_str()?;
    (!cwd.trim().is_empty()).then(|| cwd.to_string())
}

/// Claude Stop/SubagentStop body의 background_tasks에서 실행 중 서브에이전트 수를 센다.
/// SubagentStop 스냅샷에는 정지 중인 자기 자신이 아직 "running"으로 포함되므로
/// top-level agent_id와 id가 일치하는 엔트리는 제외한다. 배열 부재/파싱 실패 = None.
pub fn running_subagents(body: &[u8]) -> Option<u32> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let tasks = value.get("background_tasks")?.as_array()?;
    let self_id = value.get("agent_id").and_then(|v| v.as_str());
    let count = tasks
        .iter()
        .filter(|t| {
            t.get("type").and_then(|v| v.as_str()) == Some("subagent")
                && t.get("status").and_then(|v| v.as_str()) == Some("running")
                && (self_id.is_none() || t.get("id").and_then(|v| v.as_str()) != self_id)
        })
        .count();
    Some(count as u32)
}

pub fn tool_description(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let description = value.get("tool_input")?.get("description")?.as_str()?;
    (!description.trim().is_empty()).then(|| description.to_string())
}

/// 캐릭터 머리 위 도구 요약 최대 길이(chars). 라벨 2줄에 실리므로 짧게 자른다(이슈 #43).
pub const MAX_TOOL_TEXT_CHARS: usize = 60;

/// PostToolUse 훅 body에서 top-level `tool_name`(str)과 `tool_input`(object)을
/// 읽어 라벨용 도구 요약을 만든다(이슈 #43). 도구별로 가장 의미 있는 detail 한
/// 조각을 뽑아 `"{tool_name}: {detail}"`(detail 없으면 tool_name만)로 만들고,
/// chars 기준 MAX_TOOL_TEXT_CHARS로 절단한다(멀티바이트 안전, `…` 부착).
/// tool_name 부재/공백/비문자열이면 None.
pub fn tool_activity_text(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let tool_name = value.get("tool_name")?.as_str()?.trim();
    if tool_name.is_empty() {
        return None;
    }
    let detail = value
        .get("tool_input")
        .and_then(|input| tool_activity_detail(tool_name, input));
    let summary = match detail {
        Some(detail) => format!("{tool_name}: {detail}"),
        None => tool_name.to_string(),
    };
    Some(truncate_tool_text(&summary))
}

/// tool_name 기준으로 tool_input에서 detail 한 조각을 뽑는다. 필드 부재/비문자열/
/// 공백뿐 또는 미지 도구면 None(→ tool_name만 표시).
fn tool_activity_detail(tool_name: &str, input: &serde_json::Value) -> Option<String> {
    let is_sep = |c: char| c == '/' || c == '\\';
    let raw = match tool_name {
        "Bash" => input
            .get("command")?
            .as_str()?
            .lines()
            .next()?
            .trim()
            .to_string(),
        "Edit" | "Write" | "Read" | "NotebookEdit" => {
            let path = input.get("file_path")?.as_str()?.trim();
            let trimmed = path.trim_end_matches(is_sep);
            match trimmed.rsplit(is_sep).next() {
                Some(name) if !name.is_empty() => name.to_string(),
                _ => trimmed.to_string(),
            }
        }
        "Grep" | "Glob" => input.get("pattern")?.as_str()?.trim().to_string(),
        "Task" => input.get("description")?.as_str()?.trim().to_string(),
        "WebFetch" => input.get("url")?.as_str()?.trim().to_string(),
        "WebSearch" => input.get("query")?.as_str()?.trim().to_string(),
        _ => return None,
    };
    (!raw.is_empty()).then_some(raw)
}

/// chars 기준 MAX_TOOL_TEXT_CHARS 절단 + 잘렸으면 `…`(truncate_stop_message 패턴).
fn truncate_tool_text(text: &str) -> String {
    if text.chars().count() > MAX_TOOL_TEXT_CHARS {
        let head: String = text.chars().take(MAX_TOOL_TEXT_CHARS).collect();
        format!("{head}…")
    } else {
        text.to_string()
    }
}

/// Pi 확장이 `tool_execution_start`에서 실어 보낸 `{tool_name, tool_input}`을
/// 라벨용 도구 요약으로 만든다(Claude의 `tool_activity_text` 대응). 도구 이름이
/// Claude와 다르고(소문자) 인자 필드명도 달라서(`path`/`command`/`pattern`)
/// 별도 매핑이 필요하다 — 실측 근거는 pi v0.84.2의
/// `dist/core/tools/{bash,read,write,edit,ls,find,grep}.js` 파라미터 스키마.
/// tool_name 부재/공백/비문자열이면 None.
pub fn pi_tool_activity_text(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let tool_name = value.get("tool_name")?.as_str()?.trim();
    if tool_name.is_empty() {
        return None;
    }
    let detail = value
        .get("tool_input")
        .and_then(|input| pi_tool_activity_detail(tool_name, input));
    let summary = match detail {
        Some(detail) => format!("{tool_name}: {detail}"),
        None => tool_name.to_string(),
    };
    Some(truncate_tool_text(&summary))
}

/// pi 도구 이름 기준으로 인자에서 detail 한 조각을 뽑는다. 미지 도구(확장이 등록한
/// 커스텀 툴 포함)는 None → 도구 이름만 표시한다.
fn pi_tool_activity_detail(tool_name: &str, input: &serde_json::Value) -> Option<String> {
    let is_sep = |c: char| c == '/' || c == '\\';
    let raw = match tool_name {
        "bash" => input
            .get("command")?
            .as_str()?
            .lines()
            .next()?
            .trim()
            .to_string(),
        "read" | "write" | "edit" | "ls" => {
            let path = input.get("path")?.as_str()?.trim();
            let trimmed = path.trim_end_matches(is_sep);
            match trimmed.rsplit(is_sep).next() {
                Some(name) if !name.is_empty() => name.to_string(),
                _ => trimmed.to_string(),
            }
        }
        "find" | "grep" => input.get("pattern")?.as_str()?.trim().to_string(),
        _ => return None,
    };
    (!raw.is_empty()).then_some(raw)
}

/// Pi 확장이 `message_end`(assistant)에서 뽑아 실어 보낸 턴 중간 내레이션.
/// Claude는 전사 파일 tail에서 같은 것을 뽑지만(claude_transcript_progress_message),
/// pi 확장은 프로세스 안에서 메시지를 직접 보므로 body에 담아 온다. 절단 규칙은
/// 완료 메시지와 동일.
pub fn pi_assistant_text(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    truncate_stop_message(value.get("assistant")?.as_str()?)
}

/// Claude 훅 body의 top-level `transcript_path`. 공백/부재/비문자열은 None.
pub fn transcript_path(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let path = value.get("transcript_path")?.as_str()?;
    (!path.trim().is_empty()).then(|| path.to_string())
}

/// 완료(Stop) 알림 본문 최대 길이(chars). 티커/OS 알림에 실리므로 백엔드에서
/// 적당히 잘라 hub로 넘긴다(프런트 excerpt는 이 위에서 더 줄인다). 이슈 #39.
pub const MAX_STOP_MESSAGE_CHARS: usize = 300;

/// 완료 메시지 정규화: trim 후 공백뿐이면 None(→ hub의 STOP_FALLBACK 유지),
/// MAX_STOP_MESSAGE_CHARS 초과면 chars 기준 절단 + "…". 멀티바이트 안전.
pub fn truncate_stop_message(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() > MAX_STOP_MESSAGE_CHARS {
        let head: String = trimmed.chars().take(MAX_STOP_MESSAGE_CHARS).collect();
        Some(format!("{head}…"))
    } else {
        Some(trimmed.to_string())
    }
}

/// Codex Stop 훅 body의 `last_assistant_message`를 완료 메시지로 추출·절단한다.
/// 부재/비문자열/공백뿐이면 None(이슈 #39: 예전엔 의도적으로 버렸으나 이제 노출).
pub fn codex_stop_message(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let message = value.get("last_assistant_message")?.as_str()?;
    truncate_stop_message(message)
}

/// transcript 파일 끝에서 이만큼만 읽어 뒤에서부터 줄을 스캔한다(전체 로드 회피).
const TRANSCRIPT_TAIL_BYTES: u64 = 64 * 1024;

/// Claude Stop 훅 body의 `transcript_path`(JSONL)를 열어 마지막 assistant
/// 메시지 텍스트를 완료 메시지로 추출·절단한다(이슈 #39). 견고성 우선:
/// 파일 부재/포맷 이상/빈 텍스트는 모두 None으로 폴백한다. 파일이 클 수 있으니
/// 끝에서 TRANSCRIPT_TAIL_BYTES 만큼만 읽어 뒤에서부터 줄을 스캔하고, 가장
/// 마지막의 `type=="assistant"` 라인에서 `message.content[]` 중 `type=="text"`
/// 조각들을 이어붙인 값을 쓴다.
pub fn claude_transcript_message(body: &[u8]) -> Option<String> {
    let tail = read_transcript_tail(body)?;
    let last = tail.lines().rev().find_map(assistant_line_text)?;
    truncate_stop_message(&last)
}

/// 턴 "중간"의 최신 assistant 내레이션을 transcript 꼬리에서 뽑는다(이슈 #43).
/// `claude_transcript_message`와 같은 방식으로 transcript_path 꼬리
/// TRANSCRIPT_TAIL_BYTES를 읽어 뒤에서부터 스캔하되, **"진짜 사용자 프롬프트" 줄을
/// 만나면 즉시 None을 반환**한다 — 그 줄 이후 assistant 텍스트가 없다는 뜻(현재
/// 턴은 아직 도구 단계)이라, 직전 턴의 마지막 답변을 현재 실황으로 오인 표시하지
/// 않기 위해서다. tool_result 조각만 있는 user 줄은 턴 내부 도구 결과이므로 경계가
/// 아니며(스캔 계속), isSidechain 줄은 assistant/user 판정 모두에서 스킵한다.
pub fn claude_transcript_progress_message(body: &[u8]) -> Option<String> {
    let tail = read_transcript_tail(body)?;
    for line in tail.lines().rev() {
        match transcript_scan_line(line) {
            TranscriptScan::Assistant(text) => return truncate_stop_message(&text),
            TranscriptScan::UserPrompt => return None,
            TranscriptScan::Other => continue,
        }
    }
    None
}

/// body의 transcript_path를 열어 끝에서 TRANSCRIPT_TAIL_BYTES만큼 읽는다.
/// 부재/공백/파일 없음/비JSON은 모두 None.
fn read_transcript_tail(body: &[u8]) -> Option<String> {
    let path = transcript_path(body)?;
    read_file_tail(std::path::Path::new(&path), TRANSCRIPT_TAIL_BYTES)
}

/// transcript 한 줄의 스캔 결과. 뒤에서부터 스캔하며 경계/실황을 판정한다.
enum TranscriptScan {
    /// 턴 중간 실황으로 쓸 assistant 텍스트.
    Assistant(String),
    /// 진짜 사용자 프롬프트 줄(스캔 중단 경계).
    UserPrompt,
    /// 판정 무관(스캔 계속): tool_result-only user, sidechain, 파싱 실패 등.
    Other,
}

fn transcript_scan_line(line: &str) -> TranscriptScan {
    let value: serde_json::Value = match serde_json::from_str(line.trim()) {
        Ok(value) => value,
        Err(_) => return TranscriptScan::Other,
    };
    // 서브에이전트 사이드체인 줄은 assistant/user 판정 모두에서 스킵.
    if value.get("isSidechain").and_then(|v| v.as_bool()) == Some(true) {
        return TranscriptScan::Other;
    }
    match value.get("type").and_then(|t| t.as_str()) {
        Some("assistant") => match assistant_content_text(&value) {
            Some(text) => TranscriptScan::Assistant(text),
            None => TranscriptScan::Other,
        },
        Some("user") if is_real_user_prompt(&value) => TranscriptScan::UserPrompt,
        _ => TranscriptScan::Other,
    }
}

/// transcript의 user 줄이 "진짜 사용자 프롬프트"인지 — `message.content`가 문자열
/// 이거나, 배열에 `type=="text"` 조각이 있으면 true. tool_result 조각만 있는 줄은
/// 턴 내부 도구 결과이므로 false(경계가 아님).
fn is_real_user_prompt(value: &serde_json::Value) -> bool {
    let Some(content) = value.get("message").and_then(|m| m.get("content")) else {
        return false;
    };
    if content.is_string() {
        return true;
    }
    content
        .as_array()
        .is_some_and(|arr| {
            arr.iter()
                .any(|part| part.get("type").and_then(|t| t.as_str()) == Some("text"))
        })
}

/// 턴 사용량 합산을 위해 전사 꼬리에서 읽는 최대 바이트. 완료 메시지용
/// `TRANSCRIPT_TAIL_BYTES`(64KB)보다 훨씬 크다 — 한 턴이 도구 호출 수십 개로
/// 길어지면 그 턴의 첫 assistant 응답이 64KB 밖으로 밀려나 사용량이 통째로
/// 누락되기 때문이다. Stop 훅은 턴당 1회뿐이라 이 비용을 감당할 수 있다.
/// 이 상한 안에서 턴 경계(직전 사용자 프롬프트)를 못 찾으면 찾은 데까지만
/// 합산한다(과소 집계로 강등 — 조용한 폴백 원칙).
const TRANSCRIPT_USAGE_TAIL_BYTES: u64 = 2 * 1024 * 1024;

/// Claude Stop 훅의 `transcript_path`(JSONL) 꼬리를 읽어 **이번 턴**이 쓴 토큰을
/// 합산한다. 실패(경로 부재/파일 없음/유효 사용량 없음)는 모두 None. 성공하면
/// 합산값과 함께 이번에 센 것 중 **가장 최근** `message.id`(다음 호출의 워터마크
/// 후보)를 돌려준다.
///
/// 스캔 종료 조건은 `watermark` 유무로 갈린다.
///
/// - `watermark == None`(앱 재시작 직후 등 직전 합산 지점을 모를 때): 뒤에서부터
///   스캔하다 만나는 첫 진짜 사용자 프롬프트(`claude_transcript_progress_message`가
///   쓰는 판정 `is_real_user_prompt`, tool_result-only user 줄은 경계가 아님)에서
///   break한다. 세션 누계의 델타를 기억하는 방식 대신 이 방식을 택한 이유는
///   (a) 전사에 누계 필드가 없어 어차피 전 구간을 합산해야 하고, (b) 앱 재시작·
///   세션 입양 후에도 상태 없이 정확하기 때문이다.
/// - `watermark == Some(id)`: 프롬프트 경계에서 멈추지 않고 **워터마크 id를 만날
///   때까지** 계속 스캔한다 — 백그라운드 서브에이전트가 Stop 이후에도 사이드체인
///   줄을 계속 append하고, 그 완료가 `task-notification`을 새 user 프롬프트로
///   주입하는 경로에서는 프롬프트 경계가 워터마크보다 먼저 걸려 그 사이드체인
///   몫(대개 이 턴 비용의 대부분)이 통째로 누락되기 때문이다. 다만 워터마크
///   줄이 꼬리 상한(2MB) 밖으로 밀려나 무제한 과대 집계가 되는 걸 막기 위해,
///   **처음 만난(=가장 최근) 진짜 사용자 프롬프트 지점의 합계를 스냅샷**해
///   두었다가 워터마크를 끝내 못 만나면(꼬리 소진) 그 스냅샷으로 강등한다.
///   워터마크를 만나면 스냅샷은 버리고 그 지점까지의 **전체 합계**를 쓴다 —
///   이게 정상 경로다. **앱을 재시작하면 워터마크가 없어져** 그 세션의 다음
///   턴 창이 실제보다 한 턴만큼 과대 집계될 수 있다(수용된 한계, 결정 C와 별개).
///
/// **같은 응답이 여러 줄로 쪼개져 기록된다** — Claude는 assistant 응답 하나를
/// content 블록별(thinking/text/tool_use)로 나눠 여러 줄에 쓰면서 `message.usage`를
/// 매 줄에 그대로 복사한다. 줄 단위로 더하면 2~3배 과대 집계되므로 `message.id`로
/// 중복을 제거한다.
///
/// 서브에이전트(`isSidechain`) 줄의 사용량도 합산한다 — 실제로 청구되는 비용이고
/// 이 턴에 속한다. 다만 경계 판정에서는 스킵한다(서브에이전트의 프롬프트 줄이
/// 메인 턴 경계로 오인되면 스캔이 조기 종료된다).
pub fn claude_transcript_usage(
    body: &[u8],
    watermark: Option<&str>,
) -> Option<(SessionEventTokens, Option<String>)> {
    let path = transcript_path(body)?;
    let tail = read_file_tail(std::path::Path::new(&path), TRANSCRIPT_USAGE_TAIL_BYTES)?;
    let (totals, newest_id) = sum_claude_turn_usage(&tail, watermark);
    // 못 셌으면 None — 호출부가 기존 워터마크를 그대로 유지한다(전사에 애초에
    // 유효 사용량 줄이 없는 경우 등, 조용한 폴백 원칙).
    totals.non_empty().map(|t| (t, newest_id))
}

/// 전사 꼬리 문자열에서 마지막 턴의 사용량을 합산한다(위 규칙). 순수 함수라
/// 테스트가 파일 없이 직접 부른다.
fn sum_claude_turn_usage(
    tail: &str,
    watermark: Option<&str>,
) -> (SessionEventTokens, Option<String>) {
    let mut totals = SessionEventTokens::default();
    let mut seen_messages: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut newest_id: Option<String> = None;
    let mut input = 0u64;
    let mut output = 0u64;
    let mut cache_read = 0u64;
    let mut cache_write = 0u64;
    // watermark가 있을 때만 쓰는 폴백: 처음(=가장 최근) 만난 진짜 사용자 프롬프트
    // 지점까지의 합계 스냅샷. 워터마크를 끝내 못 만나면(꼬리 밖으로 밀려남) 이걸로
    // 강등한다.
    let mut prompt_boundary_snapshot: Option<(u64, u64, u64, u64)> = None;
    let mut watermark_hit = false;

    for line in tail.lines().rev() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue; // 손상/절단 라인(꼬리 앞머리 포함)은 조용히 스킵
        };
        let sidechain = value.get("isSidechain").and_then(|v| v.as_bool()) == Some(true);
        match value.get("type").and_then(|t| t.as_str()) {
            // 메인 세션의 진짜 사용자 프롬프트 = 턴 시작.
            // watermark가 없으면 지금까지가 전부다 → 스캔 종료.
            // watermark가 있으면 그 뒤(더 과거)에 워터마크가 있을 수 있으니
            // 스냅샷만 남기고 계속 스캔한다(필수 1).
            Some("user") if !sidechain && is_real_user_prompt(&value) => {
                if watermark.is_none() {
                    break;
                }
                prompt_boundary_snapshot
                    .get_or_insert((input, output, cache_read, cache_write));
            }
            Some("assistant") => {
                let Some(message) = value.get("message") else {
                    continue;
                };
                let msg_id = message.get("id").and_then(|v| v.as_str());
                // 결정 E: 워터마크 비교는 dedup 삽입보다 **먼저** — 꼬리부터
                // 스캔하므로 같은 id의 더 새 줄이 먼저 나온다. dedup을 먼저
                // 태우면 그 줄이 카운트된 뒤 break해 이중 계산이 된다.
                if msg_id == watermark && msg_id.is_some() {
                    watermark_hit = true;
                    break;
                }
                if let Some(id) = msg_id {
                    newest_id.get_or_insert_with(|| id.to_string());
                }
                // 같은 응답이 블록별로 쪼개져 usage를 반복 기록한다 → id로 1회만.
                if let Some(id) = msg_id {
                    if !seen_messages.insert(id.to_string()) {
                        continue;
                    }
                }
                // 대표 모델 = 뒤에서부터 처음 만난 메인 세션 응답의 모델
                // (서브에이전트는 다른 모델을 쓸 수 있어 대표로 삼지 않는다).
                if totals.model.is_none() && !sidechain {
                    totals.model = message
                        .get("model")
                        .and_then(|v| v.as_str())
                        .filter(|m| !m.trim().is_empty())
                        .map(str::to_string);
                }
                let Some(usage) = message.get("usage") else {
                    continue;
                };
                let field = |name: &str| usage.get(name).and_then(serde_json::Value::as_u64);
                input = input.saturating_add(field("input_tokens").unwrap_or(0));
                output = output.saturating_add(field("output_tokens").unwrap_or(0));
                cache_read =
                    cache_read.saturating_add(field("cache_read_input_tokens").unwrap_or(0));
                cache_write =
                    cache_write.saturating_add(field("cache_creation_input_tokens").unwrap_or(0));
            }
            _ => continue,
        }
    }

    // 워터마크가 있었는데 꼬리를 다 훑고도 못 만났다면(꼬리 밖으로 밀려남) 전체
    // 합계 대신 프롬프트 경계 스냅샷으로 강등한다. 프롬프트 경계조차 못 만났으면
    // (전사가 온통 한 턴뿐이라 스냅샷이 없으면) 전체 합계를 그대로 쓴다.
    if watermark.is_some() && !watermark_hit {
        if let Some((si, so, sr, sw)) = prompt_boundary_snapshot {
            input = si;
            output = so;
            cache_read = sr;
            cache_write = sw;
        }
    }

    totals.input = Some(input);
    totals.output = Some(output);
    totals.cache_read = Some(cache_read);
    totals.cache_write = Some(cache_write);
    (totals, newest_id)
}

/// 파일 끝에서 최대 `max` 바이트를 읽어 String으로. 앞머리에서 잘린 멀티바이트는
/// lossy 변환으로 흡수한다(뒤에서부터 스캔하므로 앞머리 손상은 무해).
fn read_file_tail(path: &std::path::Path, max: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(max);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// JSONL 한 줄이 assistant 메시지면 그 안의 text 조각들을 이어붙여 돌려준다.
/// assistant 아님/파싱 실패/텍스트 없음은 None.
fn assistant_line_text(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if value.get("type").and_then(|t| t.as_str()) != Some("assistant") {
        return None;
    }
    assistant_content_text(&value)
}

/// assistant 메시지 값의 `message.content[]`에서 `type=="text"` 조각들을 이어붙인다.
/// 텍스트 없음/구조 이상은 None.
fn assistant_content_text(value: &serde_json::Value) -> Option<String> {
    let content = value.get("message")?.get("content")?.as_array()?;
    let mut out = String::new();
    for part in content {
        if part.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                out.push_str(text);
            }
        }
    }
    (!out.trim().is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::{
        agent_id, hook_cwd, is_command_prompt, message, native_session_id, prompt_text,
        running_subagents, tool_description, AdapterSessionPlan, CommandWrapperSpec,
        ObserverProvider, ObserverSessionContext,
    };

    fn wrapper(command: &str) -> CommandWrapperSpec {
        CommandWrapperSpec {
            command: command.into(),
            prefix_args: vec![],
            skip_if_present: vec![],
            ..Default::default()
        }
    }

    #[test]
    fn observer_json_helpers_preserve_prompt_and_message_rules() {
        assert_eq!(prompt_text(b"not-json"), None);
        assert_eq!(prompt_text(br#"{"prompt":"   "}"#), None);
        assert_eq!(prompt_text(br#"{"prompt":"/clear"}"#), None);
        let long = serde_json::json!({ "prompt": "가".repeat(2_500) }).to_string();
        assert_eq!(prompt_text(long.as_bytes()).unwrap().chars().count(), 2_000);
        assert_eq!(
            message(br#"{"message":"attention"}"#).as_deref(),
            Some("attention")
        );
        assert_eq!(message(br#"{"message":" "}"#), None);
        assert_eq!(
            tool_description(br#"{"tool_input":{"description":"approval detail"}}"#).as_deref(),
            Some("approval detail"),
        );
    }

    #[test]
    fn agent_id_reads_only_non_empty_top_level_strings() {
        assert_eq!(
            agent_id(br#"{"agent_id":"uuid-123"}"#).as_deref(),
            Some("uuid-123")
        );
        assert_eq!(agent_id(br#"{}"#), None);
        assert_eq!(agent_id(b"not json"), None);
        assert_eq!(agent_id(br#"{"agent_id":""}"#), None);
        assert_eq!(agent_id(br#"{"agent_id":"   "}"#), None);
        assert_eq!(agent_id(br#"{"agent_id":42}"#), None);
        assert_eq!(agent_id(br#"{"agent_id":null}"#), None);
    }

    #[test]
    fn native_session_id_reads_only_non_empty_top_level_strings() {
        assert_eq!(
            native_session_id(br#"{"session_id":"native-abc"}"#).as_deref(),
            Some("native-abc")
        );
        assert_eq!(native_session_id(br#"{}"#), None);
        assert_eq!(native_session_id(b"not json"), None);
        assert_eq!(native_session_id(br#"{"session_id":""}"#), None);
        assert_eq!(native_session_id(br#"{"session_id":"   "}"#), None);
        assert_eq!(native_session_id(br#"{"session_id":42}"#), None);
        assert_eq!(native_session_id(br#"{"session_id":null}"#), None);
    }

    #[test]
    fn hook_cwd_reads_only_non_empty_top_level_strings() {
        assert_eq!(
            hook_cwd(br#"{"cwd":"/home/x/project"}"#).as_deref(),
            Some("/home/x/project")
        );
        assert_eq!(hook_cwd(br#"{}"#), None);
        assert_eq!(hook_cwd(b"not json"), None);
        assert_eq!(hook_cwd(br#"{"cwd":""}"#), None);
        assert_eq!(hook_cwd(br#"{"cwd":"  "}"#), None);
        assert_eq!(hook_cwd(br#"{"cwd":5}"#), None);
    }

    #[test]
    fn running_subagents_excludes_matching_self_id() {
        let body = br#"{
            "agent_id":"self",
            "background_tasks":[
                {"id":"self","type":"subagent","status":"running"},
                {"id":"other","type":"subagent","status":"running"}
            ]
        }"#;
        assert_eq!(running_subagents(body), Some(1));
    }

    #[test]
    fn running_subagents_stop_shape_without_agent_id_counts_all() {
        let body = br#"{"background_tasks":[
            {"id":"one","type":"subagent","status":"running"},
            {"id":"two","type":"subagent","status":"running"}
        ]}"#;
        assert_eq!(running_subagents(body), Some(2));
    }

    #[test]
    fn running_subagents_does_not_subtract_when_self_is_absent() {
        let body = br#"{"agent_id":"missing","background_tasks":[
            {"id":"other","type":"subagent","status":"running"}
        ]}"#;
        assert_eq!(running_subagents(body), Some(1));
    }

    #[test]
    fn running_subagents_filters_status_and_type() {
        let body = br#"{"background_tasks":[
            {"id":"running-sub","type":"subagent","status":"running"},
            {"id":"stopped-sub","type":"subagent","status":"stopped"},
            {"id":"running-shell","type":"shell","status":"running"}
        ]}"#;
        assert_eq!(running_subagents(body), Some(1));
    }

    #[test]
    fn running_subagents_distinguishes_missing_invalid_and_empty_arrays() {
        assert_eq!(running_subagents(br#"{}"#), None);
        assert_eq!(running_subagents(b"not-json"), None);
        assert_eq!(running_subagents(br#"{"background_tasks":[]}"#), Some(0));
    }

    #[test]
    fn prompt_text_preserves_plain_text_and_filters_command_prefixes() {
        assert_eq!(
            prompt_text(r#"{"prompt":"  버그 고쳐줘  "}"#.as_bytes()).as_deref(),
            Some("버그 고쳐줘"),
        );
        assert_eq!(prompt_text(br#"{"prompt":"!git status"}"#), None);
        assert_eq!(prompt_text(br##"{"prompt":"#remember"}"##), None);
        assert_eq!(prompt_text(br#"{"session_id":"s1"}"#), None);
    }

    #[test]
    fn truncate_stop_message_trims_blanks_and_caps_length() {
        use super::{truncate_stop_message, MAX_STOP_MESSAGE_CHARS};
        assert_eq!(truncate_stop_message("   "), None);
        assert_eq!(truncate_stop_message(""), None);
        assert_eq!(truncate_stop_message("  done  ").as_deref(), Some("done"));
        // 멀티바이트 안전 + "…" 부착
        let long: String = "가".repeat(MAX_STOP_MESSAGE_CHARS + 50);
        let out = truncate_stop_message(&long).unwrap();
        assert_eq!(out.chars().count(), MAX_STOP_MESSAGE_CHARS + 1); // +1 은 ellipsis
        assert!(out.ends_with('…'));
    }

    #[test]
    fn codex_stop_message_extracts_last_assistant_message() {
        use super::codex_stop_message;
        assert_eq!(
            codex_stop_message(r#"{"last_assistant_message":"작업 완료"}"#.as_bytes()).as_deref(),
            Some("작업 완료"),
        );
        assert_eq!(codex_stop_message(br#"{"last_assistant_message":"   "}"#), None);
        assert_eq!(codex_stop_message(br#"{}"#), None);
        assert_eq!(codex_stop_message(b"not json"), None);
    }

    #[test]
    fn claude_transcript_message_reads_last_assistant_text_from_tail() {
        use super::claude_transcript_message;
        let dir = std::env::temp_dir().join(format!(
            "agent-office-transcript-test-{}",
            uuid::Uuid::new_v4(),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");
        // user 줄 + 두 개의 assistant 줄. 마지막 assistant 의 text 조각이 이어붙어야 한다.
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"안녕"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"첫 응답"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"두 번째 "},{"type":"tool_use","name":"bash"},{"type":"text","text":"응답"}]}}"#,
        ];
        std::fs::write(&path, lines.join("\n")).unwrap();

        let body = serde_json::json!({ "transcript_path": path.to_string_lossy() })
            .to_string()
            .into_bytes();
        assert_eq!(claude_transcript_message(&body).as_deref(), Some("두 번째 응답"));

        // 파일 부재/필드 부재/포맷 이상은 None 폴백.
        let missing = serde_json::json!({ "transcript_path": dir.join("nope.jsonl").to_string_lossy() })
            .to_string()
            .into_bytes();
        assert_eq!(claude_transcript_message(&missing), None);
        assert_eq!(claude_transcript_message(br#"{}"#), None);
        assert_eq!(claude_transcript_message(b"not json"), None);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tool_activity_text_summarizes_by_tool_name() {
        use super::{tool_activity_text, MAX_TOOL_TEXT_CHARS};

        // Bash → command 첫 줄(trim).
        assert_eq!(
            tool_activity_text(br#"{"tool_name":"Bash","tool_input":{"command":"  npm test  \nsecond"}}"#)
                .as_deref(),
            Some("Bash: npm test"),
        );
        // Edit/Write/Read/NotebookEdit → file_path basename(양쪽 구분자).
        assert_eq!(
            tool_activity_text(br#"{"tool_name":"Edit","tool_input":{"file_path":"/a/b/c.rs"}}"#)
                .as_deref(),
            Some("Edit: c.rs"),
        );
        assert_eq!(
            tool_activity_text(br#"{"tool_name":"Write","tool_input":{"file_path":"C:\\x\\y.rs"}}"#)
                .as_deref(),
            Some("Write: y.rs"),
        );
        assert_eq!(
            tool_activity_text(br#"{"tool_name":"Read","tool_input":{"file_path":"/only/dir/"}}"#)
                .as_deref(),
            Some("Read: dir"),
        );
        // Grep/Glob → pattern.
        assert_eq!(
            tool_activity_text(br#"{"tool_name":"Grep","tool_input":{"pattern":"TODO"}}"#).as_deref(),
            Some("Grep: TODO"),
        );
        // Task → description(멀티바이트 안전).
        assert_eq!(
            tool_activity_text(
                r#"{"tool_name":"Task","tool_input":{"description":"조사하기"}}"#.as_bytes()
            )
            .as_deref(),
            Some("Task: 조사하기"),
        );
        // WebFetch → url, WebSearch → query.
        assert_eq!(
            tool_activity_text(br#"{"tool_name":"WebFetch","tool_input":{"url":"https://x.dev"}}"#)
                .as_deref(),
            Some("WebFetch: https://x.dev"),
        );
        // 미지 도구/필드 부재 → 도구 이름만.
        assert_eq!(
            tool_activity_text(br#"{"tool_name":"MysteryTool","tool_input":{"foo":"bar"}}"#)
                .as_deref(),
            Some("MysteryTool"),
        );
        assert_eq!(
            tool_activity_text(br#"{"tool_name":"Bash","tool_input":{}}"#).as_deref(),
            Some("Bash"),
        );
        // tool_name 부재/공백 → None. not-json → None.
        assert_eq!(tool_activity_text(br#"{"tool_input":{"command":"x"}}"#), None);
        assert_eq!(tool_activity_text(br#"{"tool_name":"   "}"#), None);
        assert_eq!(tool_activity_text(b"not json"), None);

        // 멀티바이트 절단 + "…" 부착: 긴 Bash 명령.
        let long_cmd = "가".repeat(200);
        let body = serde_json::json!({
            "tool_name": "Bash",
            "tool_input": { "command": long_cmd },
        })
        .to_string()
        .into_bytes();
        let out = tool_activity_text(&body).unwrap();
        assert_eq!(out.chars().count(), MAX_TOOL_TEXT_CHARS + 1); // +1 은 ellipsis
        assert!(out.ends_with('…'));
    }

    // pi v0.84.2 `tool_execution_start` 실측 페이로드(spy 확장으로 덤프)를 픽스처로
    // 쓴다: {"toolName":"read","args":{"path":"..."}} / {"toolName":"bash",
    // "args":{"command":"echo done"}}. 확장이 이를 {tool_name, tool_input}로
    // 감싸 POST한다.
    #[test]
    fn pi_tool_activity_text_summarizes_pi_tool_names_and_arguments() {
        use super::{pi_tool_activity_text, MAX_TOOL_TEXT_CHARS};

        assert_eq!(
            pi_tool_activity_text(
                br#"{"tool_name":"bash","tool_input":{"command":"  echo done  \nsecond"}}"#
            )
            .as_deref(),
            Some("bash: echo done"),
        );
        assert_eq!(
            pi_tool_activity_text(br#"{"tool_name":"read","tool_input":{"path":"/a/b/spy2.ts"}}"#)
                .as_deref(),
            Some("read: spy2.ts"),
        );
        assert_eq!(
            pi_tool_activity_text(br#"{"tool_name":"write","tool_input":{"path":"C:\\x\\y.rs"}}"#)
                .as_deref(),
            Some("write: y.rs"),
        );
        assert_eq!(
            pi_tool_activity_text(br#"{"tool_name":"edit","tool_input":{"path":"src/main.rs"}}"#)
                .as_deref(),
            Some("edit: main.rs"),
        );
        assert_eq!(
            pi_tool_activity_text(br#"{"tool_name":"ls","tool_input":{"path":"/only/dir/"}}"#)
                .as_deref(),
            Some("ls: dir"),
        );
        assert_eq!(
            pi_tool_activity_text(br#"{"tool_name":"grep","tool_input":{"pattern":"TODO"}}"#)
                .as_deref(),
            Some("grep: TODO"),
        );
        assert_eq!(
            pi_tool_activity_text(br#"{"tool_name":"find","tool_input":{"pattern":"**/*.ts"}}"#)
                .as_deref(),
            Some("find: **/*.ts"),
        );
        // 확장이 등록한 커스텀 툴 등 미지 도구 / 인자 부재 → 도구 이름만.
        assert_eq!(
            pi_tool_activity_text(br#"{"tool_name":"my_tool","tool_input":{"foo":"bar"}}"#)
                .as_deref(),
            Some("my_tool"),
        );
        assert_eq!(
            pi_tool_activity_text(br#"{"tool_name":"bash","tool_input":{}}"#).as_deref(),
            Some("bash"),
        );
        // Claude 대문자 이름은 pi 매핑에 없다 → 이름만(교차 오염 방지).
        assert_eq!(
            pi_tool_activity_text(br#"{"tool_name":"Bash","tool_input":{"command":"x"}}"#)
                .as_deref(),
            Some("Bash"),
        );
        // tool_name 부재/공백/비JSON → None.
        assert_eq!(
            pi_tool_activity_text(br#"{"tool_input":{"command":"x"}}"#),
            None
        );
        assert_eq!(pi_tool_activity_text(br#"{"tool_name":"  "}"#), None);
        assert_eq!(pi_tool_activity_text(b"not json"), None);

        // 멀티바이트 절단 + "…" 부착.
        let long_cmd = "가".repeat(200);
        let body = serde_json::json!({ "tool_name": "bash", "tool_input": { "command": long_cmd } })
            .to_string()
            .into_bytes();
        let out = pi_tool_activity_text(&body).unwrap();
        assert_eq!(out.chars().count(), MAX_TOOL_TEXT_CHARS + 1);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn pi_assistant_text_trims_and_truncates_like_stop_messages() {
        use super::{pi_assistant_text, MAX_STOP_MESSAGE_CHARS};
        assert_eq!(
            pi_assistant_text(r#"{"assistant":"  파일을 읽는 중  "}"#.as_bytes()).as_deref(),
            Some("파일을 읽는 중"),
        );
        assert_eq!(pi_assistant_text(br#"{"assistant":"   "}"#), None);
        assert_eq!(pi_assistant_text(br#"{"tool_name":"bash"}"#), None);
        assert_eq!(pi_assistant_text(b"not json"), None);

        let long = "가".repeat(MAX_STOP_MESSAGE_CHARS + 50);
        let body = serde_json::json!({ "assistant": long }).to_string().into_bytes();
        let out = pi_assistant_text(&body).unwrap();
        assert_eq!(out.chars().count(), MAX_STOP_MESSAGE_CHARS + 1);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn claude_transcript_progress_message_returns_mid_turn_narration() {
        use super::claude_transcript_progress_message;
        let dir = std::env::temp_dir().join(format!(
            "agent-office-progress-test-{}",
            uuid::Uuid::new_v4(),
        ));
        std::fs::create_dir_all(&dir).unwrap();

        // 턴 중간: 진짜 프롬프트 → assistant 내레이션 → tool_result-only user.
        // 마지막 user는 도구 결과이므로 경계가 아니고, 그 앞 assistant를 실황으로 쓴다.
        let mid_turn = [
            r#"{"type":"user","message":{"role":"user","content":"작업 시작"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"파일을 살펴보는 중"}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}"#,
        ];
        let path = dir.join("mid.jsonl");
        std::fs::write(&path, mid_turn.join("\n")).unwrap();
        let body = serde_json::json!({ "transcript_path": path.to_string_lossy() })
            .to_string()
            .into_bytes();
        assert_eq!(
            claude_transcript_progress_message(&body).as_deref(),
            Some("파일을 살펴보는 중"),
        );

        // 마지막 진짜 프롬프트 이후 assistant 텍스트가 없으면(직전 턴 답변만 있음) None.
        let new_turn = [
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"이전 턴 답변"}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":"다음 지시"}}"#,
        ];
        let path2 = dir.join("new-turn.jsonl");
        std::fs::write(&path2, new_turn.join("\n")).unwrap();
        let body2 = serde_json::json!({ "transcript_path": path2.to_string_lossy() })
            .to_string()
            .into_bytes();
        assert_eq!(claude_transcript_progress_message(&body2), None);

        // isSidechain assistant는 스킵하고 그 앞의 메인 assistant를 쓴다.
        let sidechain = [
            r#"{"type":"user","message":{"role":"user","content":"메인 지시"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"메인 실황"}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant","content":[{"type":"text","text":"서브 내레이션"}]}}"#,
        ];
        let path3 = dir.join("sidechain.jsonl");
        std::fs::write(&path3, sidechain.join("\n")).unwrap();
        let body3 = serde_json::json!({ "transcript_path": path3.to_string_lossy() })
            .to_string()
            .into_bytes();
        assert_eq!(
            claude_transcript_progress_message(&body3).as_deref(),
            Some("메인 실황"),
        );

        // 파일 부재/필드 부재/비JSON → None.
        let missing = serde_json::json!({ "transcript_path": dir.join("nope.jsonl").to_string_lossy() })
            .to_string()
            .into_bytes();
        assert_eq!(claude_transcript_progress_message(&missing), None);
        assert_eq!(claude_transcript_progress_message(br#"{}"#), None);
        assert_eq!(claude_transcript_progress_message(b"not json"), None);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn claude_transcript_usage_sums_one_turn_and_deduplicates_split_lines() {
        use super::claude_transcript_usage;
        let dir = std::env::temp_dir().join(format!(
            "agent-office-usage-test-{}",
            uuid::Uuid::new_v4(),
        ));
        std::fs::create_dir_all(&dir).unwrap();

        // 직전 턴(경계 밖) → 사용자 프롬프트 → 이번 턴 응답들.
        // msg-2 는 thinking/tool_use 로 쪼개져 usage 가 두 줄에 복제돼 있다(1회만 세야 한다).
        let usage = |input, out, read, write| {
            format!(
                r#""usage":{{"input_tokens":{input},"output_tokens":{out},"cache_read_input_tokens":{read},"cache_creation_input_tokens":{write}}}"#
            )
        };
        let lines = [
            format!(
                r#"{{"type":"assistant","message":{{"id":"msg-old","model":"claude-opus-5","content":[{{"type":"text","text":"직전 턴"}}],{}}}}}"#,
                usage(9_999, 9_999, 9_999, 9_999)
            ),
            r#"{"type":"user","message":{"role":"user","content":"이번 턴 지시"}}"#.into(),
            format!(
                r#"{{"type":"assistant","message":{{"id":"msg-1","model":"claude-opus-5","content":[{{"type":"text","text":"첫 응답"}}],{}}}}}"#,
                usage(10, 100, 1_000, 50)
            ),
            // 도구 결과 user 줄은 턴 경계가 아니다(스캔 계속).
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}"#.into(),
            format!(
                r#"{{"type":"assistant","message":{{"id":"msg-2","model":"claude-opus-5","content":[{{"type":"thinking"}}],{}}}}}"#,
                usage(20, 200, 2_000, 60)
            ),
            format!(
                r#"{{"type":"assistant","message":{{"id":"msg-2","model":"claude-opus-5","content":[{{"type":"tool_use"}}],{}}}}}"#,
                usage(20, 200, 2_000, 60)
            ),
            // 서브에이전트 줄: 사용량은 합산하되 프롬프트 줄은 경계로 보지 않는다.
            r#"{"type":"user","isSidechain":true,"message":{"role":"user","content":"서브 지시"}}"#.into(),
            format!(
                r#"{{"type":"assistant","isSidechain":true,"message":{{"id":"msg-sub","model":"claude-haiku-4-5","content":[{{"type":"text","text":"서브 응답"}}],{}}}}}"#,
                usage(5, 50, 500, 5)
            ),
        ];
        let path = dir.join("transcript.jsonl");
        std::fs::write(&path, lines.join("\n")).unwrap();
        let body = serde_json::json!({ "transcript_path": path.to_string_lossy() })
            .to_string()
            .into_bytes();

        let (tokens, newest_id) = claude_transcript_usage(&body, None).unwrap();
        assert_eq!(tokens.input, Some(10 + 20 + 5));
        assert_eq!(tokens.output, Some(100 + 200 + 50));
        assert_eq!(tokens.cache_read, Some(1_000 + 2_000 + 500));
        assert_eq!(tokens.cache_write, Some(50 + 60 + 5));
        // 대표 모델은 가장 최근 **메인 세션** 응답의 모델(서브에이전트 모델 아님).
        assert_eq!(tokens.model.as_deref(), Some("claude-opus-5"));
        // 워터마크 후보 = 뒤에서부터 처음 만난(=가장 최근) message.id.
        assert_eq!(newest_id.as_deref(), Some("msg-sub"));

        // 워터마크가 msg-1이면 msg-1과 그 이전(직전 턴 msg-old 포함)은 스캔에서
        // 제외되고, msg-2/msg-sub 몫만 합산된다.
        let (tokens, newest_id) = claude_transcript_usage(&body, Some("msg-1")).unwrap();
        assert_eq!(tokens.input, Some(20 + 5));
        assert_eq!(tokens.output, Some(200 + 50));
        assert_eq!(newest_id.as_deref(), Some("msg-sub"));

        // 워터마크가 이번 스캔에서 가장 최근 id(msg-sub)와 같으면 합산할 게
        // 없어 합계가 전부 0 → non_empty()가 걸러내 None.
        assert_eq!(claude_transcript_usage(&body, Some("msg-sub")), None);

        // 사용량 라인이 하나도 없으면(프롬프트만 있는 파일) None.
        let empty = dir.join("empty-turn.jsonl");
        std::fs::write(
            &empty,
            r#"{"type":"user","message":{"role":"user","content":"방금 보낸 지시"}}"#,
        )
        .unwrap();
        let empty_body = serde_json::json!({ "transcript_path": empty.to_string_lossy() })
            .to_string()
            .into_bytes();
        assert_eq!(claude_transcript_usage(&empty_body, None), None);

        // 파일 부재/필드 부재/비JSON → None 폴백.
        let missing =
            serde_json::json!({ "transcript_path": dir.join("nope.jsonl").to_string_lossy() })
                .to_string()
                .into_bytes();
        assert_eq!(claude_transcript_usage(&missing, None), None);
        assert_eq!(claude_transcript_usage(br#"{}"#, None), None);
        assert_eq!(claude_transcript_usage(b"not json", None), None);

        let _ = std::fs::remove_dir_all(dir);
    }

    /// 필수 1 회귀 테스트: 서브에이전트가 Stop 이후에도 사이드체인 줄을 계속
    /// append하고, 그 완료가 `task-notification`을 새 user 프롬프트로 주입하는
    /// 경로에서 워터마크가 프롬프트 경계보다 우선해야 사이드체인 몫이 안 새어
    /// 나간다. 워터마크 뒤에 사이드체인 줄 → user 프롬프트 주입 줄 → 새 메인
    /// 응답 순서로 쌓는다.
    #[test]
    fn watermark_scan_reaches_past_a_prompt_boundary_to_collect_trailing_sidechain_usage() {
        use super::claude_transcript_usage;
        let dir = std::env::temp_dir().join(format!(
            "agent-office-usage-watermark-test-{}",
            uuid::Uuid::new_v4(),
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let usage = |input, out, read, write| {
            format!(
                r#""usage":{{"input_tokens":{input},"output_tokens":{out},"cache_read_input_tokens":{read},"cache_creation_input_tokens":{write}}}"#
            )
        };
        let lines = [
            // 직전 Stop이 이미 합산한 지점(워터마크) — 다시 세면 안 된다.
            format!(
                r#"{{"type":"assistant","message":{{"id":"msg-wm","model":"claude-opus-5","content":[{{"type":"text","text":"워터마크 응답"}}],{}}}}}"#,
                usage(1, 1, 1, 1)
            ),
            // 백그라운드 서브에이전트가 Stop#1 이후에도 계속 append한 사이드체인.
            r#"{"type":"user","isSidechain":true,"message":{"role":"user","content":"서브 지시"}}"#.into(),
            format!(
                r#"{{"type":"assistant","isSidechain":true,"message":{{"id":"msg-sub","model":"claude-haiku-4-5","content":[{{"type":"text","text":"서브 응답"}}],{}}}}}"#,
                usage(5, 50, 500, 5)
            ),
            // 서브 완료 후 claude가 task-notification을 새 user 프롬프트로 주입.
            r#"{"type":"user","message":{"role":"user","content":"[task-notification] 서브 완료"}}"#.into(),
            // Stop#2 직전 새 메인 응답.
            format!(
                r#"{{"type":"assistant","message":{{"id":"msg-new","model":"claude-opus-5","content":[{{"type":"text","text":"새 응답"}}],{}}}}}"#,
                usage(10, 100, 1_000, 50)
            ),
        ];
        let path = dir.join("transcript.jsonl");
        std::fs::write(&path, lines.join("\n")).unwrap();
        let body = serde_json::json!({ "transcript_path": path.to_string_lossy() })
            .to_string()
            .into_bytes();

        let (tokens, newest_id) = claude_transcript_usage(&body, Some("msg-wm")).unwrap();
        // 사이드체인(msg-sub) 몫과 메인(msg-new) 몫이 프롬프트 경계를 넘어 모두 합산된다.
        assert_eq!(tokens.input, Some(5 + 10));
        assert_eq!(tokens.output, Some(50 + 100));
        assert_eq!(tokens.cache_read, Some(500 + 1_000));
        assert_eq!(tokens.cache_write, Some(5 + 50));
        assert_eq!(newest_id.as_deref(), Some("msg-new"));

        let _ = std::fs::remove_dir_all(dir);
    }

    /// 필수 1 회귀 테스트: 워터마크 id가 2MB 꼬리 안에 없으면(꼬리 밖으로
    /// 밀려남) 무제한 과대 집계 대신 프롬프트 경계 스냅샷으로 강등한다 —
    /// 그 앞(더 과거) 구간의 사용량은 섞이지 않는다. 결과가 워터마크 없이
    /// 프롬프트 경계에서 멈춘 스캔과 같아야 한다.
    #[test]
    fn watermark_not_found_in_tail_demotes_to_the_prompt_boundary_snapshot() {
        use super::claude_transcript_usage;
        let dir = std::env::temp_dir().join(format!(
            "agent-office-usage-watermark-demote-test-{}",
            uuid::Uuid::new_v4(),
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let usage = |input, out, read, write| {
            format!(
                r#""usage":{{"input_tokens":{input},"output_tokens":{out},"cache_read_input_tokens":{read},"cache_creation_input_tokens":{write}}}"#
            )
        };
        let lines = [
            // 프롬프트 경계보다 앞(더 과거) 구간 — 섞이면 안 된다.
            format!(
                r#"{{"type":"assistant","message":{{"id":"msg-old","model":"claude-opus-5","content":[{{"type":"text","text":"직전 턴"}}],{}}}}}"#,
                usage(999, 999, 999, 999)
            ),
            r#"{"type":"user","message":{"role":"user","content":"이번 턴 지시"}}"#.into(),
            format!(
                r#"{{"type":"assistant","message":{{"id":"msg-new","model":"claude-opus-5","content":[{{"type":"text","text":"응답"}}],{}}}}}"#,
                usage(10, 100, 1_000, 50)
            ),
            format!(
                r#"{{"type":"assistant","isSidechain":true,"message":{{"id":"msg-sub","model":"claude-haiku-4-5","content":[{{"type":"text","text":"서브 응답"}}],{}}}}}"#,
                usage(5, 50, 500, 5)
            ),
        ];
        let path = dir.join("transcript.jsonl");
        std::fs::write(&path, lines.join("\n")).unwrap();
        let body = serde_json::json!({ "transcript_path": path.to_string_lossy() })
            .to_string()
            .into_bytes();

        let (no_watermark, _) = claude_transcript_usage(&body, None).unwrap();
        let (demoted, newest_id) =
            claude_transcript_usage(&body, Some("msg-does-not-exist")).unwrap();

        // 워터마크를 못 찾았을 때의 결과는 워터마크 없이 프롬프트 경계에서
        // 멈춘 스캔과 정확히 같다 — 그 앞 msg-old의 999는 안 섞인다.
        assert_eq!(demoted.input, no_watermark.input);
        assert_eq!(demoted.output, no_watermark.output);
        assert_eq!(demoted.cache_read, no_watermark.cache_read);
        assert_eq!(demoted.cache_write, no_watermark.cache_write);
        assert_eq!(demoted.input, Some(10 + 5));
        assert_eq!(demoted.output, Some(100 + 50));
        // newest_id는 스냅샷/폴백과 무관하게 "뒤에서부터 처음 만난 것" 그대로.
        assert_eq!(newest_id.as_deref(), Some("msg-sub"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn is_command_prompt_flags_bash_slash_and_memory_prefixes() {
        assert!(is_command_prompt("!git status"));
        assert!(is_command_prompt("/clear"));
        assert!(is_command_prompt("#remember"));
        assert!(!is_command_prompt("버그 고쳐줘"));
        assert!(!is_command_prompt("git status"));
        // 절대경로 텍스트도 '/'로 시작하면 명령으로 취급된다 — 감수하는 트레이드오프.
        assert!(is_command_prompt("/home/x"));
    }

    #[test]
    fn observer_contract_constructors_preserve_exact_values() {
        assert_eq!(ObserverProvider::Claude.as_str(), "claude");
        assert_eq!(ObserverProvider::Codex.as_str(), "codex");
        assert_eq!(
            ObserverProvider::parse("claude"),
            Some(ObserverProvider::Claude)
        );
        assert_eq!(
            ObserverProvider::parse("codex"),
            Some(ObserverProvider::Codex)
        );
        assert_eq!(ObserverProvider::parse("other"), None);

        let codex_wrapper = wrapper("codex");
        assert_eq!(codex_wrapper.command, "codex");
        assert!(codex_wrapper.prefix_args.is_empty());
        assert!(codex_wrapper.skip_if_present.is_empty());

        let context = ObserverSessionContext::new("s1", "http://127.0.0.1/hook");
        assert_eq!(context.session_id, "s1");
        assert_eq!(context.hook_url, "http://127.0.0.1/hook");

        let mut merged = AdapterSessionPlan {
            env: vec![("FIRST".into(), "1".into())],
            wrappers: vec![wrapper("claude")],
            cleanup_paths: vec!["first.json".into()],
        };
        merged.merge(AdapterSessionPlan {
            env: vec![("SECOND".into(), "2".into())],
            wrappers: vec![wrapper("codex")],
            cleanup_paths: vec!["second.json".into()],
        });
        assert_eq!(merged.env[1], ("SECOND".into(), "2".into()));
        assert_eq!(merged.wrappers[1].command, "codex");
        assert_eq!(
            merged.cleanup_paths[1],
            std::path::PathBuf::from("second.json")
        );
    }
}
