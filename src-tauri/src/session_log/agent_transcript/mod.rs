// src-tauri/src/session_log/agent_transcript/mod.rs
//
// 에이전트 CLI가 스스로 남기는 전사(JSONL)를 세션 로그로 끌어오는 경로.
// docs/session-log-design.md §3.4 / §3.8 이 정본.
//
// **왜 필요한가**: Claude Code(v2.1.x)와 Codex는 시작 즉시 대체 화면
// (`CSI ?1049h`)으로 들어간다. PTY 전사(transcript.rs)는 대체 화면 안을
// 기록하지 않으므로 -- 전면 재그리기 덩어리라 기록해도 못 읽는다 -- 에이전트
// 대화가 통째로 마커 두 줄로 남았다. 그런데 두 CLI 모두 자기 대화를 완전한
// JSONL로 이미 남긴다. 화면을 긁을 이유가 없다: 그 파일을 tail 해서 읽을 수
// 있는 줄로 바꿔 같은 세션 로그에 끼워 넣는다.
//
//   PTY 전사      → 사람이 친 셸 명령, 도구 출력, 전체 화면 앱 마커
//   JSONL 전사    → 사용자 프롬프트, 에이전트 응답, 도구 호출/결과  ← 이 모듈
//
// 어떤 실패도 세션이나 로그를 막지 않는다. 파일이 없으면 아무 일도 없고,
// 파싱이 깨진 줄은 조용히 건너뛴다.

pub mod claude;
pub mod codex;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 한 번에 읽어 들이는 최대 바이트. 에이전트가 거대한 도구 결과를 한 번에
/// 쏟아도 기록 스레드가 한 틱에 묶이지 않게 한다(다음 틱에 이어 읽는다).
const MAX_READ_PER_TICK: u64 = 2 * 1024 * 1024;
/// 렌더한 값 하나의 문자 상한. 도구 결과 전문을 그대로 옮기면 로그가 원본
/// JSONL보다 커진다 -- 회고에 필요한 만큼만 남기고 자른다.
pub(crate) const MAX_VALUE_CHARS: usize = 1200;
/// 도구 결과에서 남길 줄 수 상한.
pub(crate) const MAX_VALUE_LINES: usize = 24;

/// 웹 채팅 프레임의 문자 상한. 로그와 달리 "펼치면 전문이 보인다"가 목적이라
/// 훨씬 넉넉하다 — 클라이언트가 접기/펼치기로 화면 점유를 관리한다.
/// 상한 산정: 폰이 들고 있는 항목 수(웹 `MAX_ITEMS` 400) × 이 한도가 최악의
/// 메모리이므로, 16k자면 UTF-16 기준 대략 12MB대 — 400개가 전부 상한을 채우는
/// 일은 실제로 없고(도구 결과 대부분은 수백 자) 현실적 상한으로 충분하다.
pub const MAX_WEB_VALUE_CHARS: usize = 16_000;
/// 웹 채팅 프레임의 줄 수 상한.
pub const MAX_WEB_VALUE_LINES: usize = 240;

/// 백필(웹 채팅 진입)에서 파일 끝에서부터 거슬러 읽는 최대 바이트.
pub const BACKFILL_MAX_BYTES: u64 = 256 * 1024;
/// 백필로 돌려주는 최대 항목 수(가장 최근 것부터 남긴다).
pub const BACKFILL_MAX_ITEMS: usize = 100;

// ── 구조화된 채팅 항목 ────────────────────────────────────────────────
//
// 세션 로그(문자열)와 웹 채팅 뷰(버블)가 **같은 파싱 결과**를 쓴다:
// `TranscriptSource::parse`가 JSONL 한 줄을 이 항목들로 바꾸고,
// `format_items`가 그것을 로그 줄로 되돌린다. 포맷 결합(claude/codex JSONL
// 스키마)이 한 곳에만 남는다는 것이 이 분리의 목적이다
// (docs/web-remote-design.md §5 M2).

/// 항목의 화자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemRole {
    User,
    Assistant,
}

/// 항목의 종류. thinking·이미지·서명 블롭은 애초에 항목이 되지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Text,
    ToolUse,
    ToolResult,
}

/// 전사 한 조각(= 채팅 버블 하나 / 로그 블록 하나).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptItem {
    pub role: ItemRole,
    pub kind: ItemKind,
    /// 본문. **자르지 않은 원문**이다 — 로그 포매터가 `block`에서 한 번만
    /// 자르고(이중 클램프는 "… (이하 생략)"를 두 번 붙인다), 와이어로 나갈
    /// 때는 호출자가 `clamped()`로 자른다.
    pub text: String,
    /// 도구 이름. `ToolUse`인데 이름이 없으면 "이름 없는 활동 줄"이다
    /// (codex의 서브에이전트 활동 — 로그에서 `⤷ …` 한 줄로 나간다).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub is_error: bool,
    /// 서브에이전트(sidechain) 대화인가. 로그에서는 `⤷` 표식이 붙는다.
    #[serde(default)]
    pub sidechain: bool,
}

impl TranscriptItem {
    pub fn speech(role: ItemRole, text: impl Into<String>) -> Self {
        Self {
            role,
            kind: ItemKind::Text,
            text: text.into(),
            tool_name: None,
            is_error: false,
            sidechain: false,
        }
    }

    pub fn tool_use(name: Option<String>, text: impl Into<String>) -> Self {
        Self {
            role: ItemRole::Assistant,
            kind: ItemKind::ToolUse,
            text: text.into(),
            tool_name: name,
            is_error: false,
            sidechain: false,
        }
    }

    pub fn tool_result(text: impl Into<String>, is_error: bool) -> Self {
        Self {
            role: ItemRole::User,
            kind: ItemKind::ToolResult,
            text: text.into(),
            tool_name: None,
            is_error,
            sidechain: false,
        }
    }

    pub fn with_sidechain(mut self, side: bool) -> Self {
        self.sidechain = side;
        self
    }

    /// 와이어로 내보내기 전 본문을 자른다(로그의 `block`과 같은 규칙).
    pub fn clamped(mut self) -> Self {
        self.text = clamp_value(&self.text).join("\n");
        self
    }

    /// 웹 채팅 프레임용 클램프. 로그 한도(1200자/24줄)로 자르면 브라우저에서
    /// 펼쳐도 원문이 없다 — 잘린 뒤라 되살릴 곳이 없기 때문이다. 그래서 웹은
    /// 별도의 넉넉한 한도를 쓰고, 접기/펼치기는 클라이언트가 한다.
    pub fn clamped_for_web(mut self) -> Self {
        self.text = clamp_value_with(&self.text, MAX_WEB_VALUE_CHARS, MAX_WEB_VALUE_LINES)
            .join("\n");
        self
    }
}

/// 항목들 → 세션 로그 줄들. **문자열 수준에서 예전 `render`와 동일해야 한다**
/// (기존 세션 로그 픽스처 테스트가 그 계약을 핀으로 박아 둔다).
pub fn format_items(items: &[TranscriptItem]) -> Vec<String> {
    items.iter().flat_map(format_item).collect()
}

fn format_item(item: &TranscriptItem) -> Vec<String> {
    let mark = |glyph: &str| {
        if item.sidechain {
            format!("⤷ {glyph}")
        } else {
            glyph.to_string()
        }
    };
    match item.kind {
        ItemKind::Text => {
            let glyph = match item.role {
                ItemRole::User => "▶ 사용자:",
                ItemRole::Assistant => "⏺ 에이전트:",
            };
            block(&mark(glyph), &item.text)
        }
        ItemKind::ToolUse => match &item.tool_name {
            Some(name) => block(&mark(&format!("⚒ {name}:")), &item.text),
            // 이름 없는 활동 줄 — 표식만 앞에 두고 본문을 그대로 흘린다.
            None => {
                let prefix = mark("");
                let prefix = prefix.trim_end();
                if prefix.is_empty() {
                    clamp_value(&item.text)
                } else {
                    block(prefix, &item.text)
                }
            }
        },
        ItemKind::ToolResult => {
            let glyph = if item.is_error {
                "⇤ 결과(오류):"
            } else {
                "⇤ 결과:"
            };
            block(&mark(glyph), &item.text)
        }
    }
}

/// 훅이 알려 준 "지금 이 캐릭터가 쓰고 있는 네이티브 세션" 스냅샷.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionSnapshot {
    pub session_id: String,
    /// 훅 body의 cwd(세션 도중 폴더가 바뀌었을 수 있다).
    pub cwd: Option<String>,
    /// 훅 body의 `transcript_path` — CLI가 **직접 알려 준** JSONL 절대 경로.
    /// 있으면 이게 정답이다: 사용자가 `CLAUDE_CONFIG_DIR`을 어디로 옮겼든
    /// 실제로 쓰고 있는 파일이라 경로를 추측할 필요가 없다.
    pub transcript_path: Option<String>,
}

/// 에이전트별 "지금 이 캐릭터가 쓰고 있는 네이티브 세션"을 알려주는 조회기.
/// 프로덕션 구현은 `ClaudeResumeStore`(훅이 채운다).
pub trait AgentSessionLookup: Send + Sync {
    /// 모르면 None.
    fn latest_session(&self, agent_id: &str) -> Option<AgentSessionSnapshot>;
}

/// 전사 파일 한 종류(= CLI 한 종류)를 다루는 소스.
pub trait TranscriptSource: Send + Sync {
    /// 사람이 읽을 소스 이름(마커에 쓴다).
    fn label(&self) -> &'static str;
    /// 지금 이 세션에 붙은 전사 파일. 없거나 모르면 None.
    fn locate(&mut self, agent_id: &str, cwd: &str) -> Option<PathBuf>;
    /// JSONL 한 줄 → 구조화된 항목들. 남길 것이 없으면 빈 벡터.
    /// **포맷 결합(claude/codex 스키마)은 이 함수 하나에만 있다.**
    fn parse(&self, raw: &str) -> Vec<TranscriptItem>;
    /// JSONL 한 줄 → 로그 줄들. 기본 구현이 `parse` + 포매터라 로그와 채팅이
    /// 절대 어긋나지 않는다(테스트용 가짜 소스만 이걸 덮어쓴다).
    fn render(&self, raw: &str) -> Vec<String> {
        format_items(&self.parse(raw))
    }
}

/// 파일 하나를 어디까지 읽었는지.
struct Tail {
    offset: u64,
    /// 아직 개행이 오지 않은 마지막 조각.
    partial: String,
}

/// 소스들을 주기적으로 tail 해서 새 줄을 뽑아내는 수집기. 시계·파일시스템
/// 접근만 하고 쓰기는 호출자(기록 스레드)가 한다 -- 테스트가 틱을 직접 돌린다.
pub struct TranscriptTailer {
    agent_id: String,
    cwd: String,
    sources: Vec<Box<dyn TranscriptSource>>,
    tails: HashMap<PathBuf, Tail>,
    /// 소스별로 지금 붙어 있는 파일(바뀌면 마커를 남긴다).
    current: HashMap<&'static str, PathBuf>,
}

impl TranscriptTailer {
    pub fn new(agent_id: &str, cwd: &str, sources: Vec<Box<dyn TranscriptSource>>) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            cwd: cwd.to_string(),
            sources,
            tails: HashMap::new(),
            current: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// 한 틱 분량을 읽어 새로 확정된 줄들을 돌려준다.
    ///
    /// 처음 만난 파일은 **끝에서부터** 읽는다. 앱이 이미 한참 돌던 세션에
    /// 붙었을 때 과거 대화 전체를 로그에 다시 쏟지 않기 위함이다 -- 대신
    /// 원본 경로를 한 줄 남겨 그쪽을 찾아갈 수 있게 한다.
    pub fn tick(&mut self) -> Vec<String> {
        self.tick_with(
            |source, raw| source.render(raw),
            |label, path| Some(format!("[{label} 전사 연결: {}]", path.to_string_lossy())),
        )
    }

    /// 같은 틱을 **구조화된 항목**으로 받는다(웹 채팅 뷰). 연결 마커는 로그
    /// 전용이라 여기서는 나오지 않는다.
    pub fn tick_items(&mut self) -> Vec<TranscriptItem> {
        self.tick_with(|source, raw| source.parse(raw), |_, _| None)
    }

    /// 지금 어떤 전사 파일에라도 붙어 있는가(없으면 채팅화 불가 = 터미널 폴백).
    pub fn has_target(&self) -> bool {
        !self.current.is_empty()
    }

    /// 지금 붙어 있는 전사 파일들(정렬 — 호출자가 "바뀌었는가"를 비교한다).
    pub fn targets(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.current.values().cloned().collect();
        v.sort();
        v
    }

    /// 붙어 있는 파일들의 **최근 항목**. 파일 끝에서 `max_bytes`만큼 거슬러
    /// 읽어 파싱하고 마지막 `max_items`개만 남긴다. tick 계열이 파일을 찾은
    /// 뒤에 부른다(그 전에는 붙은 파일이 없어 빈 결과다).
    pub fn backfill(&mut self, max_bytes: u64, max_items: usize) -> Vec<TranscriptItem> {
        let mut out = Vec::new();
        let sources = std::mem::take(&mut self.sources);
        for source in sources.iter() {
            let Some(path) = self.current.get(source.label()) else {
                continue;
            };
            let Some((chunk, from_start)) = read_tail(path, max_bytes) else {
                continue;
            };
            for line in complete_lines(&chunk, from_start) {
                out.extend(source.parse(line));
            }
        }
        self.sources = sources;
        if out.len() > max_items {
            out.drain(..out.len() - max_items);
        }
        out
    }

    fn tick_with<T>(
        &mut self,
        mut convert: impl FnMut(&dyn TranscriptSource, &str) -> Vec<T>,
        mut marker: impl FnMut(&str, &Path) -> Option<T>,
    ) -> Vec<T> {
        let mut out = Vec::new();
        // sources를 &mut로 쓰면서 self의 다른 필드도 건드려야 해서 잠시 꺼낸다.
        let mut sources = std::mem::take(&mut self.sources);
        for source in sources.iter_mut() {
            let Some(path) = source.locate(&self.agent_id, &self.cwd) else {
                continue;
            };
            let label = source.label();
            let known = self.current.get(label) == Some(&path);
            if !known {
                let start = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                self.current.insert(label, path.clone());
                self.tails.entry(path.clone()).or_insert(Tail {
                    offset: start,
                    partial: String::new(),
                });
                out.extend(marker(label, &path));
            }
            self.read_new(&path, source.as_ref(), &mut convert, &mut out);
        }
        self.sources = std::mem::take(&mut sources);
        out
    }

    fn read_new<T>(
        &mut self,
        path: &Path,
        source: &dyn TranscriptSource,
        convert: &mut impl FnMut(&dyn TranscriptSource, &str) -> Vec<T>,
        out: &mut Vec<T>,
    ) {
        let Some(tail) = self.tails.get_mut(path) else {
            return;
        };
        let Ok(meta) = std::fs::metadata(path) else {
            return;
        };
        let len = meta.len();
        if len < tail.offset {
            // 파일이 줄었다(교체·회전). 처음부터 다시 읽는 대신 끝으로 옮긴다.
            tail.offset = len;
            tail.partial.clear();
            return;
        }
        if len == tail.offset {
            return;
        }
        let Ok(file) = std::fs::File::open(path) else {
            return;
        };
        let mut reader = BufReader::new(file);
        if reader.seek(SeekFrom::Start(tail.offset)).is_err() {
            return;
        }
        let budget = (len - tail.offset).min(MAX_READ_PER_TICK);
        let mut read = 0u64;
        let mut buf = Vec::new();
        while read < budget {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    read += n as u64;
                    let chunk = String::from_utf8_lossy(&buf);
                    if buf.last() != Some(&b'\n') {
                        // 아직 쓰이는 중인 마지막 줄 -- 다음 틱에 이어 붙인다.
                        tail.partial.push_str(&chunk);
                        break;
                    }
                    let line = if tail.partial.is_empty() {
                        chunk.into_owned()
                    } else {
                        let mut whole = std::mem::take(&mut tail.partial);
                        whole.push_str(&chunk);
                        whole
                    };
                    let line = line.trim_end_matches(['\n', '\r']);
                    if !line.is_empty() {
                        out.extend(convert(source, line));
                    }
                }
                Err(_) => break,
            }
        }
        tail.offset += read;
    }
}

/// 파일 끝에서 최대 `max_bytes`를 거슬러 읽는다. 두 번째 값은 **파일 전체를
/// 읽었는가** — false면 첫 줄이 중간에서 잘렸을 수 있다는 뜻이다.
fn read_tail(path: &Path, max_bytes: u64) -> Option<(String, bool)> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    file.read_to_end(&mut buf).ok()?;
    Some((String::from_utf8_lossy(&buf).into_owned(), start == 0))
}

/// 꼬리 조각에서 **완전한 줄만** 골라낸다(순수 함수).
///
/// - `from_start`가 false면 첫 줄은 중간에서 잘렸을 수 있으므로 버린다.
///   (JSONL이라 잘린 줄은 어차피 파싱에 실패하지만, 운 나쁘게 파싱되는
///   조각으로 가짜 항목을 만들지 않도록 규칙으로 못 박는다.)
/// - 마지막 줄은 개행으로 끝나지 않았으면 "아직 쓰이는 중"이라 버린다.
pub fn complete_lines(chunk: &str, from_start: bool) -> Vec<&str> {
    let mut lines: Vec<&str> = chunk.split('\n').collect();
    // split('\n')의 마지막 조각은 개행 뒤의 나머지다 — 비어 있으면 파일이
    // 개행으로 끝난 것이고, 아니면 미완성 줄이다. 어느 쪽이든 버린다.
    lines.pop();
    if !from_start && !lines.is_empty() {
        lines.remove(0);
    }
    lines
        .into_iter()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.is_empty())
        .collect()
}

/// JSON 값 안의 긴 문자열은 자리표시로 바꾼 뒤 압축 JSON으로 만든다.
///
/// 도구 인자에는 암호화 블롭(Codex의 서브에이전트 `message`가 대표적이다)이나
/// 파일 전문이 그대로 들어 있다. 자르기(clamp) 전에 걸러야 한다 -- 안 그러면
/// 상한이 블롭으로 다 차서 정작 알아야 할 뒷 키들이 잘려 나간다.
pub(crate) fn compact_json_brief(v: &serde_json::Value) -> String {
    fn redact(v: &serde_json::Value) -> serde_json::Value {
        use serde_json::Value;
        const MAX_INLINE: usize = 200;
        match v {
            Value::String(s) if s.chars().count() > MAX_INLINE => {
                Value::String(format!("(생략 {}자)", s.chars().count()))
            }
            Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
            Value::Object(map) => {
                Value::Object(map.iter().map(|(k, v)| (k.clone(), redact(v))).collect())
            }
            other => other.clone(),
        }
    }
    serde_json::to_string(&redact(v)).unwrap_or_default()
}

/// 여러 줄 값을 로그에 넣을 수 있게 자른다(줄 수·문자 수 둘 다).
/// **세션 로그 전용 한도**다 — 픽스처 테스트가 문자열 수준으로 핀을 박고 있어
/// 여기서 나오는 바이트는 바뀌면 안 된다.
pub(crate) fn clamp_value(text: &str) -> Vec<String> {
    clamp_value_with(text, MAX_VALUE_CHARS, MAX_VALUE_LINES)
}

/// 한도를 주입받는 클램프. 소비처마다 상한이 다르다(로그는 회고용 요약,
/// 웹 채팅은 "펼치면 전문이 보인다"가 목적).
pub(crate) fn clamp_value_with(text: &str, max_chars: usize, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut chars = 0usize;
    let mut truncated = false;
    for raw in text.lines() {
        if lines.len() >= max_lines {
            truncated = true;
            break;
        }
        let line = raw.trim_end();
        let remaining = max_chars.saturating_sub(chars);
        if remaining == 0 {
            truncated = true;
            break;
        }
        if line.chars().count() > remaining {
            let cut: String = line.chars().take(remaining).collect();
            lines.push(cut);
            truncated = true;
            break;
        }
        chars += line.chars().count();
        lines.push(line.to_string());
    }
    if truncated {
        lines.push("… (이하 생략)".to_string());
    }
    lines
}

/// `"  들여쓴 본문"` 꼴로 접두어를 붙인다. 첫 줄에만 화살표를 두고 이어지는
/// 줄은 같은 폭으로 들여써 사람이 블록 경계를 알아볼 수 있게 한다.
pub(crate) fn block(prefix: &str, body: &str) -> Vec<String> {
    let lines = clamp_value(body);
    if lines.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(lines.len());
    out.push(format!("{prefix} {}", lines[0]));
    let indent = " ".repeat(prefix.chars().count() + 1);
    for line in &lines[1..] {
        if line.is_empty() {
            out.push(String::new());
        } else {
            out.push(format!("{indent}{line}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct EchoSource {
        path: PathBuf,
    }

    impl TranscriptSource for EchoSource {
        fn label(&self) -> &'static str {
            "echo"
        }
        fn locate(&mut self, _agent_id: &str, _cwd: &str) -> Option<PathBuf> {
            self.path.exists().then(|| self.path.clone())
        }
        fn parse(&self, raw: &str) -> Vec<TranscriptItem> {
            if raw.starts_with("skip") {
                Vec::new()
            } else {
                vec![TranscriptItem::speech(ItemRole::User, raw)]
            }
        }
        /// 이 가짜 소스만 포매터를 우회한다 — 틱 배관(오프셋·부분 줄·회전)을
        /// 글리프 없이 그대로 확인하기 위해서다.
        fn render(&self, raw: &str) -> Vec<String> {
            self.parse(raw).into_iter().map(|i| i.text).collect()
        }
    }

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agent-office-tailer-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn append(path: &Path, text: &str) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    fn tailer(path: &Path) -> TranscriptTailer {
        TranscriptTailer::new(
            "a1",
            "/tmp",
            vec![Box::new(EchoSource {
                path: path.to_path_buf(),
            })],
        )
    }

    #[test]
    fn existing_content_is_skipped_and_new_lines_are_tailed() {
        let dir = scratch();
        let path = dir.join("t.jsonl");
        append(&path, "old-1\nold-2\n");

        let mut t = tailer(&path);
        let first = t.tick();
        // 연결 마커만. 과거 내용은 다시 쏟지 않는다.
        assert_eq!(first.len(), 1, "{first:?}");
        assert!(first[0].contains("전사 연결"), "{first:?}");

        append(&path, "new-1\nskip-me\nnew-2\n");
        assert_eq!(t.tick(), vec!["new-1", "new-2"]);
        // 새 내용이 없으면 아무것도 내지 않는다.
        assert!(t.tick().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn partial_line_is_held_until_newline_arrives() {
        let dir = scratch();
        let path = dir.join("t.jsonl");
        append(&path, "");
        let mut t = tailer(&path);
        t.tick();

        append(&path, "half");
        assert!(t.tick().is_empty(), "개행 전에는 내보내지 않는다");
        append(&path, "-and-half\n");
        assert_eq!(t.tick(), vec!["half-and-half"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let dir = scratch();
        let mut t = tailer(&dir.join("nope.jsonl"));
        assert!(t.tick().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn truncated_file_resets_without_replaying() {
        let dir = scratch();
        let path = dir.join("t.jsonl");
        append(&path, "a\nb\n");
        let mut t = tailer(&path);
        t.tick();
        append(&path, "c\n");
        assert_eq!(t.tick(), vec!["c"]);

        std::fs::write(&path, b"").unwrap(); // 파일이 줄어듦
        assert!(t.tick().is_empty());
        append(&path, "d\n");
        assert_eq!(t.tick(), vec!["d"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clamp_value_caps_lines_and_chars() {
        let many: String = (0..100).map(|i| format!("line{i}\n")).collect();
        let out = clamp_value(&many);
        assert!(out.len() <= MAX_VALUE_LINES + 1, "{}", out.len());
        assert_eq!(out.last().unwrap(), "… (이하 생략)");

        let long = "가".repeat(5000);
        let out = clamp_value(&long);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].chars().count(), MAX_VALUE_CHARS);
    }

    #[test]
    fn compact_json_brief_redacts_long_blobs_but_keeps_short_keys() {
        let blob = "g".repeat(500);
        let v: serde_json::Value = serde_json::json!({
            "task_name": "inspect_gfx",
            "message": blob,
            "nested": {"deep": "x".repeat(300)},
        });
        let out = compact_json_brief(&v);
        assert!(out.contains("\"task_name\":\"inspect_gfx\""), "{out}");
        assert!(out.contains("(생략 500자)"), "{out}");
        assert!(out.contains("(생략 300자)"), "{out}");
        assert!(!out.contains("gggg"), "{out}");
    }

    #[test]
    fn block_indents_continuation_lines() {
        let out = block("⏺", "first\nsecond");
        assert_eq!(out, vec!["⏺ first", "  second"]);
    }

    // ── 구조화 항목 ↔ 로그 줄 ─────────────────────────────────────────

    #[test]
    fn formatter_reproduces_the_log_glyphs() {
        assert_eq!(
            format_items(&[TranscriptItem::speech(ItemRole::User, "고쳐줘")]),
            vec!["▶ 사용자: 고쳐줘"]
        );
        assert_eq!(
            format_items(&[TranscriptItem::speech(ItemRole::Assistant, "네")]),
            vec!["⏺ 에이전트: 네"]
        );
        assert_eq!(
            format_items(&[TranscriptItem::tool_use(
                Some("Bash".into()),
                "git status"
            )]),
            vec!["⚒ Bash: git status"]
        );
        assert_eq!(
            format_items(&[TranscriptItem::tool_result("clean", false)]),
            vec!["⇤ 결과: clean"]
        );
        assert_eq!(
            format_items(&[TranscriptItem::tool_result("boom", true)]),
            vec!["⇤ 결과(오류): boom"]
        );
        // sidechain은 표식만 앞에 붙는다.
        assert_eq!(
            format_items(&[
                TranscriptItem::speech(ItemRole::Assistant, "서브에이전트 응답").with_sidechain(true)
            ]),
            vec!["⤷ ⏺ 에이전트: 서브에이전트 응답"]
        );
        // 이름 없는 활동 줄(codex 서브에이전트) — 표식 + 본문.
        assert_eq!(
            format_items(&[
                TranscriptItem::tool_use(None, "서브에이전트 /a/b: spawn").with_sidechain(true)
            ]),
            vec!["⤷ 서브에이전트 /a/b: spawn"]
        );
    }

    /// 항목의 `text`는 자르지 않은 원문이고, 자르기는 포매터(또는 `clamped`)가
    /// **한 번만** 한다 — 두 번 자르면 생략 표시가 두 줄 붙는다.
    #[test]
    fn clamping_happens_once() {
        let long = "가".repeat(5000);
        let item = TranscriptItem::speech(ItemRole::User, long.clone());
        assert_eq!(item.text.chars().count(), 5000, "항목은 원문을 들고 있다");

        let lines = format_items(&[item.clone()]);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].trim(), "… (이하 생략)");

        let clamped = item.clamped();
        assert!(clamped.text.ends_with("… (이하 생략)"));
        assert_eq!(clamped.text.lines().count(), 2);
    }

    /// 웹 채팅은 별도 한도다 — 로그가 자르는 길이도 여기서는 그대로 살아남아야
    /// 클라이언트가 "펼치기"로 전문을 보여줄 수 있다.
    #[test]
    fn web_clamp_keeps_what_the_log_would_cut() {
        let long = "가".repeat(5000);
        let item = TranscriptItem::speech(ItemRole::User, long.clone());

        // 로그 한도로는 잘린다.
        let logged = item.clone().clamped();
        assert_eq!(
            logged.text.lines().next().unwrap().chars().count(),
            MAX_VALUE_CHARS
        );
        assert!(logged.text.ends_with("… (이하 생략)"));
        // 웹 한도로는 원문 그대로.
        let web = item.clone().clamped_for_web();
        assert_eq!(web.text, long);
        assert!(!web.text.contains("… (이하 생략)"));

        // 웹 한도도 상한은 있다 — 넘으면 같은 마커가 붙는다.
        let huge = "나".repeat(MAX_WEB_VALUE_CHARS + 100);
        let cut = TranscriptItem::speech(ItemRole::User, huge).clamped_for_web();
        assert!(cut.text.ends_with("… (이하 생략)"));
        assert_eq!(cut.text.lines().next().unwrap().chars().count(), MAX_WEB_VALUE_CHARS);

        // 줄 수 상한도 웹 쪽이 넉넉하다.
        let many: String = (0..300).map(|i| format!("line{i}\n")).collect();
        let lines = clamp_value_with(&many, MAX_WEB_VALUE_CHARS, MAX_WEB_VALUE_LINES);
        assert_eq!(lines.len(), MAX_WEB_VALUE_LINES + 1);
        assert_eq!(lines.last().unwrap(), "… (이하 생략)");
    }

    /// 한도 파라미터화가 **세션 로그 출력을 바꾸지 않았다**는 핀.
    #[test]
    fn parameterized_clamp_preserves_the_log_defaults() {
        let cases: Vec<String> = vec![
            "짧은 줄".to_string(),
            "가".repeat(5000),
            (0..100).map(|i| format!("line{i}\n")).collect(),
            String::new(),
        ];
        for (i, text) in cases.iter().enumerate() {
            assert_eq!(
                clamp_value(text),
                clamp_value_with(text, MAX_VALUE_CHARS, MAX_VALUE_LINES),
                "case {i}"
            );
        }
    }

    #[test]
    fn complete_lines_drops_partial_head_and_tail() {
        // 중간부터 읽은 조각 — 첫 줄은 잘렸을 수 있으니 버린다.
        let out = complete_lines("f-line\"}\nsecond\nthird\npart", false);
        assert_eq!(out, vec!["second", "third"]);

        // 파일 전체를 읽었으면 첫 줄도 온전하다.
        let whole = complete_lines("first\nsecond\n", true);
        assert_eq!(whole, vec!["first", "second"]);

        // 개행이 하나도 없는 조각은 확정된 줄이 없다.
        assert!(complete_lines("no-newline-yet", false).is_empty());
        assert!(complete_lines("", true).is_empty());
        // CRLF와 빈 줄은 걸러진다.
        assert_eq!(complete_lines("a\r\n\r\nb\r\n", true), vec!["a", "b"]);
    }

    #[test]
    fn backfill_returns_the_most_recent_items_from_the_file_tail() {
        let dir = scratch();
        let path = dir.join("t.jsonl");
        for i in 0..10 {
            append(&path, &format!("line-{i}\n"));
        }
        let mut t = tailer(&path);
        // tick이 파일을 찾아 붙는다(끝에서 시작하므로 항목은 없다).
        assert!(t.tick_items().is_empty());
        assert!(t.has_target());

        let items = t.backfill(BACKFILL_MAX_BYTES, 3);
        assert_eq!(items.len(), 3, "가장 최근 3개만");
        assert_eq!(items[0].text, "line-7");
        assert_eq!(items[2].text, "line-9");

        // 상한이 넉넉하면 전부.
        let all = t.backfill(BACKFILL_MAX_BYTES, 100);
        assert_eq!(all.len(), 10);

        // 바이트 상한이 걸리면 잘린 첫 줄을 버린다.
        let small = t.backfill(20, 100);
        assert!(small.len() < 10, "{small:?}");
        assert_eq!(small.last().unwrap().text, "line-9");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn backfill_without_a_located_file_is_empty() {
        let dir = scratch();
        let mut t = tailer(&dir.join("nope.jsonl"));
        assert!(!t.has_target());
        assert!(t.backfill(BACKFILL_MAX_BYTES, 100).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
