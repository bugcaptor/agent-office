//! 파일 수신함. scan은 원본을 보존하고 업무 스냅샷 저장 후 ack한다.
//! 형식 오류는 scan이, 의미 오류(id 충돌·없는 완료 대상)는 렌더러가 판정해 `rejected/`로
//! 옮기고, 고쳐 받은 업무(버린 선행·모르는 담당)는 `warnings.log`에 남긴다.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::{Read, Write}, path::Path, time::{Duration, SystemTime}};

pub const MAX_BYTES: usize = 64 * 1024;
const MAX_BATCH: usize = 50;
pub const WARNINGS_LOG: &str = "warnings.log";
/// 이보다 커지면 `warnings.log.1` 하나만 남기고 새로 쓴다.
const MAX_WARNINGS_BYTES: u64 = 256 * 1024;

#[derive(Debug, PartialEq)]
pub struct Parsed {
    pub title: String,
    pub body: String,
    pub assignee: Option<String>,
    /// 생산자가 정한 업무 id. 형식이 틀리면 버리고 해시 key로 만든다.
    pub id: Option<String>,
    /// 선행 업무 id. 앱이 가져올 때 목록에 있는 것만 남기고 나머지는 경고로 남긴다.
    pub after: Vec<String>,
    /// 완료 요청. 비어 있지 않으면 새 업무가 아니라 이 id들을 완료로 바꾸는 파일이다.
    pub done: Vec<String>,
}
#[derive(Debug, PartialEq)]
pub enum RejectReason { InvalidUtf8, TooLarge, UnclosedFrontmatter, Empty, ControlChar(char) }
impl RejectReason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidUtf8 => "invalid-utf8", Self::TooLarge => "too-large",
            Self::UnclosedFrontmatter => "unclosed-frontmatter", Self::Empty => "empty",
            Self::ControlChar(_) => "control-char",
        }
    }
    /// `.error` 둘째 줄. 제어문자 거부는 첫 문자를 `U+001B`처럼 적는다.
    pub fn detail(&self) -> Option<String> {
        match self { Self::ControlChar(c) => Some(format!("U+{:04X}", *c as u32)), _ => None }
    }
}
/// 업무 제목·본문에 둘 수 없는 문자. 업무 보내기(`work_send`)와 같은 기준으로 탭·줄바꿈만 허용한다.
pub fn forbidden_control(text: &str) -> Option<char> {
    text.chars().find(|c| c.is_control() && !matches!(c, '\n' | '\t'))
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxEntry {
    pub file_name: String,
    pub key: String,
    pub title: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub done: Vec<String>,
}

/// 수신함으로 지정할 수 있는 업무 id. 앱이 만든 id(nanoid)도 이 형식이다.
pub fn valid_task_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
/// `after:` 값은 쉼표나 공백으로 나눈다. 형식이 틀린 id는 버리고 중복은 합친다.
pub fn parse_after(value: &str) -> Vec<String> {
    let mut ids: Vec<String> = vec![];
    for id in value.split(|c: char| c == ',' || c.is_whitespace()).filter(|id| valid_task_id(id)) {
        if !ids.iter().any(|known| known == id) { ids.push(id.to_string()); }
    }
    ids
}

pub fn parse(_name: &str, bytes: &[u8]) -> Result<Parsed, RejectReason> {
    if bytes.len() > MAX_BYTES { return Err(RejectReason::TooLarge); }
    let text = std::str::from_utf8(bytes).map_err(|_| RejectReason::InvalidUtf8)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text).replace("\r\n", "\n");
    let mut lines = text.split('\n').peekable();
    let mut assignee = None;
    let mut id = None;
    let mut after = vec![];
    let mut done = vec![];
    if lines.peek() == Some(&"---") {
        lines.next();
        let mut closed = false;
        for line in lines.by_ref() {
            if line == "---" { closed = true; break; }
            if let Some((key, value)) = line.split_once(':') {
                match key.trim() {
                    "assignee" => assignee = Some(value.trim().to_string()).filter(|v| !v.is_empty()),
                    "id" => id = Some(value.trim().to_string()).filter(|v| valid_task_id(v)),
                    "after" => after = parse_after(value),
                    "done" => done = parse_after(value),
                    _ => {}
                }
            }
        }
        if !closed { return Err(RejectReason::UnclosedFrontmatter); }
    }
    let first = lines.by_ref().find(|line| !line.trim().is_empty()).unwrap_or("").trim();
    // 빈 제목은 `# `로 쓴다. trim 뒤 `#`만 남으므로 그것도 빈 제목으로 본다.
    let title = if first == "#" { "" } else { first.strip_prefix("# ").unwrap_or(first) }.trim().to_string();
    let body = lines.collect::<Vec<_>>().join("\n");
    // 본문의 들여쓰기와 trailing spaces는 보존하고 빈 줄만 다듬는다.
    let body = body.split('\n').collect::<Vec<_>>();
    let start = body.iter().position(|s| !s.trim().is_empty()).unwrap_or(body.len());
    let end = body.iter().rposition(|s| !s.trim().is_empty()).map_or(start, |i| i + 1);
    let body = body[start..end].join("\n");
    if done.is_empty() && title.is_empty() && body.trim().is_empty() { return Err(RejectReason::Empty); }
    // ESC 등이 남으면 `office-task --list`·터미널에서 해석되고 업무 보내기도 거절하므로 받지 않는다.
    // 완료 요청은 제목·본문을 쓰지 않으므로 보지 않는다.
    let control = if done.is_empty() { forbidden_control(&title).or_else(|| forbidden_control(&body)) } else { None };
    if let Some(c) = control { return Err(RejectReason::ControlChar(c)); }
    Ok(Parsed { title, body, assignee, id, after, done })
}
fn accepted_name(name: &str) -> bool {
    !name.starts_with('.') && name.ends_with(".md") && !name.contains(['/', '\\']) && !name.contains("..")
}
/// 렌더러가 판정한 의미 오류 거부. `reason`은 `.error` 첫 줄, `detail`은 둘째 줄이다.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rejection {
    pub file_name: String,
    pub reason: String,
    #[serde(default)]
    pub detail: Option<String>,
}
/// 업무는 만들었지만 입력 일부를 버리거나 바꾼 기록.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    pub file_name: String,
    pub task_id: String,
    pub code: String,
    pub detail: String,
}
fn valid_code(code: &str) -> bool {
    (1..=32).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
}
/// 기록 파일 한 칸에 들어갈 값. 제어 문자는 공백으로 바꾸고 길이를 자른다.
fn field(value: &str) -> String {
    value.chars().map(|c| if c.is_control() { ' ' } else { c }).take(512).collect()
}
fn reject(root: &Path, name: &str, code: &str, detail: Option<&str>) -> std::io::Result<()> {
    let dir = root.join("rejected");
    fs::create_dir_all(&dir)?;
    // 같은 생산자가 이름을 재사용해도 이전 거부 파일을 보존한다.
    let target = if dir.join(name).exists() || dir.join(format!("{name}.error")).exists() {
        dir.join(format!("{}-{name}", uuid::Uuid::new_v4()))
    } else { dir.join(name) };
    fs::rename(root.join(name), &target)?;
    let mut error = fs::File::create(target.with_file_name(format!("{}.error", target.file_name().unwrap().to_string_lossy())))?;
    writeln!(error, "{code}")?;
    if let Some(detail) = detail { writeln!(error, "{}", field(detail))?; }
    Ok(())
}
pub fn scan(root: &Path, now: SystemTime) -> Result<Vec<InboxEntry>, String> {
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let mut names = vec![];
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        // 디렉터리와 심볼릭 링크는 수신함 파일로 취급하지 않는다.
        if !entry.file_type().map_err(|e| e.to_string())?.is_file() { continue; }
        let Ok(name) = entry.file_name().into_string() else { continue; };
        if name.starts_with('.') && name.ends_with(".tmp") {
            let old = entry.metadata().and_then(|m| m.modified()).ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > Duration::from_secs(24 * 3600));
            if old { fs::remove_file(entry.path()).map_err(|e| e.to_string())?; }
        } else if accepted_name(&name) { names.push(name); }
    }
    names.sort();
    let mut entries = vec![];
    for name in names.into_iter().take(MAX_BATCH) {
        let file = match fs::File::open(root.join(&name)) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
        };
        let mut bytes = vec![];
        file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        match parse(&name, &bytes) {
            Ok(parsed) => {
                let mut hash = Sha256::new();
                hash.update(name.as_bytes()); hash.update([0]); hash.update(&bytes);
                let key = format!("{:x}", hash.finalize())[..16].to_string();
                entries.push(InboxEntry { file_name: name, key, title: parsed.title, body: parsed.body, assignee: parsed.assignee, id: parsed.id, after: parsed.after, done: parsed.done });
            }
            Err(reason) => reject(root, &name, reason.code(), reason.detail().as_deref()).map_err(|e| e.to_string())?,
        }
    }
    Ok(entries)
}
pub fn ack(root: &Path, file_names: &[String]) -> Result<(), String> {
    for name in file_names.iter().filter(|name| accepted_name(name)) {
        match fs::remove_file(root.join(name)) {
            Ok(()) => {}, Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}
/// 의미 오류 파일을 `rejected/`로 옮긴다. 안전하지 않은 이름·사유 코드는 무시하고 이미 없으면 건너뛴다.
pub fn reject_files(root: &Path, rejections: &[Rejection]) -> Result<(), String> {
    for rejection in rejections.iter().filter(|r| accepted_name(&r.file_name) && valid_code(&r.reason)) {
        if !root.join(&rejection.file_name).is_file() { continue; }
        reject(root, &rejection.file_name, &rejection.reason, rejection.detail.as_deref()).map_err(|e| e.to_string())?;
    }
    Ok(())
}
/// `warnings.log`에 시각·파일·업무 id·코드·값을 탭으로 나눠 한 줄씩 덧붙인다.
pub fn append_warnings(root: &Path, warnings: &[Warning], now: chrono::DateTime<chrono::Utc>) -> Result<(), String> {
    let lines: String = warnings.iter().filter(|w| valid_code(&w.code)).map(|w| format!(
        "{}\t{}\t{}\t{}\t{}\n", now.format("%Y-%m-%dT%H:%M:%SZ"), field(&w.file_name), field(&w.task_id), w.code, field(&w.detail),
    )).collect();
    if lines.is_empty() { return Ok(()); }
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let path = root.join(WARNINGS_LOG);
    if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_WARNINGS_BYTES) {
        fs::rename(&path, root.join(format!("{WARNINGS_LOG}.1"))).map_err(|e| e.to_string())?;
    }
    let mut file = fs::OpenOptions::new().create(true).append(true).open(&path).map_err(|e| e.to_string())?;
    file.write_all(lines.as_bytes()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_plain_frontmatter_bom_crlf_and_preserves_body_indentation() {
        assert_eq!(parse("x.md", b"\n# Title\n\n  code  \n\n").unwrap(), Parsed { title: "Title".into(), body: "  code  ".into(), assignee: None, id: None, after: vec![], done: vec![] });
        let p = parse("x.md", "\u{feff}---\r\nfuture: ignored\r\nassignee: 하나 \r\n---\r\n# 제목\r\n본문\r\n".as_bytes()).unwrap();
        assert_eq!(p, Parsed { title: "제목".into(), body: "본문".into(), assignee: Some("하나".into()), id: None, after: vec![], done: vec![] });
        assert_eq!(parse("x.md", b"Title").unwrap().body, "");
        assert_eq!(parse("x.md", b"# \nBody").unwrap(), Parsed { title: "".into(), body: "Body".into(), assignee: None, id: None, after: vec![], done: vec![] });
    }
    #[test]
    fn parses_task_id_and_prerequisites() {
        // 자기 자신은 앱이 가져올 때 경고와 함께 뺀다.
        let p = parse("x.md", b"---\nid: t-ab12\nafter: t-1, t-2 t-1,bad/id,t-ab12\n---\n# Step\n").unwrap();
        assert_eq!((p.id.as_deref(), p.after), (Some("t-ab12"), vec!["t-1".to_string(), "t-2".to_string(), "t-ab12".to_string()]));
        let p = parse("x.md", b"---\nid: not valid\nafter:\n---\n# Step\n").unwrap();
        assert_eq!((p.id, p.after), (None, vec![]));
        assert!(valid_task_id("vLdkeylcT4Nb9W4IQVT4g") && valid_task_id("inbox-0123abcd"));
        assert!(!valid_task_id("") && !valid_task_id(&"a".repeat(65)) && !valid_task_id("a.b"));
    }
    #[test]
    fn parses_completion_requests_without_title() {
        let p = parse("x.md", b"---\ndone: t-1, t-2\n---\n").unwrap();
        assert_eq!((p.done, p.title.as_str()), (vec!["t-1".to_string(), "t-2".to_string()], ""));
        assert_eq!(parse("x.md", b"---\ndone: a.b\n---\n"), Err(RejectReason::Empty));
    }
    #[test]
    fn rejects_malformed_content_and_accepts_exact_size_limit() {
        assert_eq!(parse("x.md", b"---\nassignee: a"), Err(RejectReason::UnclosedFrontmatter));
        assert_eq!(parse("x.md", b"\n \n"), Err(RejectReason::Empty));
        assert_eq!(parse("x.md", &[255]), Err(RejectReason::InvalidUtf8));
        assert_eq!(parse("x.md", &vec![b'a'; MAX_BYTES + 1]), Err(RejectReason::TooLarge));
        assert!(parse("x.md", &vec![b'a'; MAX_BYTES]).is_ok());
    }
    #[test]
    fn rejects_control_characters_except_tab_and_newline() {
        assert_eq!(parse("x.md", b"# a\x1b]52;c;aGk=\x07"), Err(RejectReason::ControlChar('\u{1b}')));
        assert_eq!(parse("x.md", b"# a\nline\rover"), Err(RejectReason::ControlChar('\r')));
        assert_eq!(parse("x.md", "# a\n\u{9b}31m".as_bytes()), Err(RejectReason::ControlChar('\u{9b}')));
        assert_eq!(parse("x.md", b"# a\tb\r\n\tindented\r\n").unwrap().body, "\tindented");
        assert!(parse("x.md", b"---\ndone: t-1\n---\n\x1b").is_ok());
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("esc.md"), "# \x1b[8mhidden").unwrap();
        assert!(scan(root.path(), SystemTime::now()).unwrap().is_empty());
        assert_eq!(fs::read_to_string(root.path().join("rejected/esc.md.error")).unwrap(), "control-char\nU+001B\n");
    }
    #[test]
    fn scans_sorted_limited_and_deterministic_then_acks_safe_names() {
        let root = tempfile::tempdir().unwrap();
        for i in (0..55).rev() { fs::write(root.path().join(format!("{i:03}.md")), "Task").unwrap(); }
        fs::write(root.path().join(".hidden.md"), "Ignore").unwrap();
        fs::write(root.path().join("partial.tmp"), "Ignore").unwrap();
        let first = scan(root.path(), SystemTime::now()).unwrap();
        assert_eq!(first.len(), 50); assert_eq!(first[0].file_name, "000.md");
        assert_eq!(first[49].file_name, "049.md");
        assert_eq!(first[0].key, scan(root.path(), SystemTime::now()).unwrap()[0].key);
        assert_eq!(first[0].key.len(), 16);
        ack(root.path(), &["000.md".into(), "missing.md".into(), "../escape.md".into(), "a\\x.md".into(), ".hidden.md".into(), "partial.tmp".into()]).unwrap();
        assert!(!root.path().join("000.md").exists()); assert!(root.path().join(".hidden.md").exists());
    }
    #[test]
    fn rejects_and_cleans_only_stale_hidden_temps() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("bad.md"), [255]).unwrap();
        fs::write(root.path().join(".stale.tmp"), "partial").unwrap();
        fs::write(root.path().join("visible.tmp"), "partial").unwrap();
        assert!(scan(root.path(), SystemTime::now() + Duration::from_secs(24 * 3600 + 1)).unwrap().is_empty());
        assert!(!root.path().join(".stale.tmp").exists()); assert!(root.path().join("visible.tmp").exists());
        assert_eq!(fs::read(root.path().join("rejected/bad.md")).unwrap(), [255]);
        assert_eq!(fs::read_to_string(root.path().join("rejected/bad.md.error")).unwrap(), "invalid-utf8\n");
        fs::write(root.path().join("bad.md"), "---").unwrap();
        scan(root.path(), SystemTime::now()).unwrap();
        assert_eq!(fs::read_dir(root.path().join("rejected")).unwrap().count(), 4);
    }
    #[test]
    fn rejects_semantic_errors_with_detail_and_skips_unsafe_or_missing() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("dup.md"), "---\nid: t-1\n---\n# New").unwrap();
        fs::write(root.path().join("keep.md"), "Keep").unwrap();
        let rejection = |name: &str, reason: &str, detail: Option<&str>| Rejection { file_name: name.into(), reason: reason.into(), detail: detail.map(String::from) };
        reject_files(root.path(), &[
            rejection("dup.md", "duplicate-id", Some("t-1\nx")),
            rejection("gone.md", "unknown-task", None),
            rejection("../keep.md", "unknown-task", None),
            rejection("keep.md", "Bad Code", None),
        ]).unwrap();
        assert!(!root.path().join("dup.md").exists()); assert!(root.path().join("keep.md").exists());
        assert_eq!(fs::read_to_string(root.path().join("rejected/dup.md")).unwrap(), "---\nid: t-1\n---\n# New");
        assert_eq!(fs::read_to_string(root.path().join("rejected/dup.md.error")).unwrap(), "duplicate-id\nt-1 x\n");
        assert_eq!(fs::read_dir(root.path().join("rejected")).unwrap().count(), 2);
    }
    #[test]
    fn appends_warnings_and_rotates_large_log() {
        let root = tempfile::tempdir().unwrap();
        let now: chrono::DateTime<chrono::Utc> = "2026-10-14T01:02:03Z".parse().unwrap();
        let warning = |code: &str, detail: &str| Warning { file_name: "a.md".into(), task_id: "t-1".into(), code: code.into(), detail: detail.into() };
        append_warnings(root.path(), &[warning("unknown-assignee", "없는\t사람"), warning("BAD", "x")], now).unwrap();
        append_warnings(root.path(), &[warning("unknown-prerequisite", "t-ghost")], now).unwrap();
        append_warnings(root.path(), &[], now).unwrap();
        assert_eq!(fs::read_to_string(root.path().join(WARNINGS_LOG)).unwrap(),
            "2026-10-14T01:02:03Z\ta.md\tt-1\tunknown-assignee\t없는 사람\n2026-10-14T01:02:03Z\ta.md\tt-1\tunknown-prerequisite\tt-ghost\n");
        fs::write(root.path().join(WARNINGS_LOG), vec![b'x'; MAX_WARNINGS_BYTES as usize + 1]).unwrap();
        append_warnings(root.path(), &[warning("self-prerequisite", "t-1")], now).unwrap();
        assert_eq!(fs::metadata(root.path().join(format!("{WARNINGS_LOG}.1"))).unwrap().len(), MAX_WARNINGS_BYTES + 1);
        assert!(fs::read_to_string(root.path().join(WARNINGS_LOG)).unwrap().ends_with("\tself-prerequisite\tt-1\n"));
        // scan은 기록 파일을 업무로 읽지 않는다.
        assert!(scan(root.path(), SystemTime::now()).unwrap().is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn ignores_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("link.md")).unwrap();
        assert!(scan(root.path(), SystemTime::now()).unwrap().is_empty());
    }
}
