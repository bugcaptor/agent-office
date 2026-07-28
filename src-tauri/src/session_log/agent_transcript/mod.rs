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
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// 한 번에 읽어 들이는 최대 바이트. 에이전트가 거대한 도구 결과를 한 번에
/// 쏟아도 기록 스레드가 한 틱에 묶이지 않게 한다(다음 틱에 이어 읽는다).
const MAX_READ_PER_TICK: u64 = 2 * 1024 * 1024;
/// 렌더한 값 하나의 문자 상한. 도구 결과 전문을 그대로 옮기면 로그가 원본
/// JSONL보다 커진다 -- 회고에 필요한 만큼만 남기고 자른다.
pub(crate) const MAX_VALUE_CHARS: usize = 1200;
/// 도구 결과에서 남길 줄 수 상한.
pub(crate) const MAX_VALUE_LINES: usize = 24;

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
    /// JSONL 한 줄 → 로그 줄들. 기록할 것이 없으면 빈 벡터.
    fn render(&self, raw: &str) -> Vec<String>;
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
                out.push(format!(
                    "[{label} 전사 연결: {}]",
                    path.to_string_lossy()
                ));
            }
            self.read_new(&path, source.as_ref(), &mut out);
        }
        self.sources = std::mem::take(&mut sources);
        out
    }

    fn read_new(&mut self, path: &Path, source: &dyn TranscriptSource, out: &mut Vec<String>) {
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
                        out.extend(source.render(line));
                    }
                }
                Err(_) => break,
            }
        }
        tail.offset += read;
    }
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
pub(crate) fn clamp_value(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut chars = 0usize;
    let mut truncated = false;
    for raw in text.lines() {
        if lines.len() >= MAX_VALUE_LINES {
            truncated = true;
            break;
        }
        let line = raw.trim_end();
        let remaining = MAX_VALUE_CHARS.saturating_sub(chars);
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
        fn render(&self, raw: &str) -> Vec<String> {
            if raw.starts_with("skip") {
                Vec::new()
            } else {
                vec![raw.to_string()]
            }
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
}
