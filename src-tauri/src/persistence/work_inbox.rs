//! 파일 수신함. scan은 원본을 보존하고 업무 스냅샷 저장 후 ack한다.
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fs, io::{Read, Write}, path::Path, time::{Duration, SystemTime}};

pub const MAX_BYTES: usize = 64 * 1024;
const MAX_BATCH: usize = 50;

#[derive(Debug, PartialEq)]
pub struct Parsed {
    pub title: String,
    pub body: String,
    pub assignee: Option<String>,
}
#[derive(Debug, PartialEq)]
pub enum RejectReason { InvalidUtf8, TooLarge, UnclosedFrontmatter, Empty }
impl RejectReason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidUtf8 => "invalid-utf8", Self::TooLarge => "too-large",
            Self::UnclosedFrontmatter => "unclosed-frontmatter", Self::Empty => "empty",
        }
    }
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
}

pub fn parse(_name: &str, bytes: &[u8]) -> Result<Parsed, RejectReason> {
    if bytes.len() > MAX_BYTES { return Err(RejectReason::TooLarge); }
    let text = std::str::from_utf8(bytes).map_err(|_| RejectReason::InvalidUtf8)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text).replace("\r\n", "\n");
    let mut lines = text.split('\n').peekable();
    let mut assignee = None;
    if lines.peek() == Some(&"---") {
        lines.next();
        let mut closed = false;
        for line in lines.by_ref() {
            if line == "---" { closed = true; break; }
            if let Some((key, value)) = line.split_once(':') {
                if key.trim() == "assignee" {
                    assignee = Some(value.trim().to_string()).filter(|v| !v.is_empty());
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
    if title.is_empty() && body.trim().is_empty() { return Err(RejectReason::Empty); }
    Ok(Parsed { title, body, assignee })
}
fn accepted_name(name: &str) -> bool {
    !name.starts_with('.') && name.ends_with(".md") && !name.contains(['/', '\\']) && !name.contains("..")
}
fn reject(root: &Path, name: &str, reason: RejectReason) -> std::io::Result<()> {
    let dir = root.join("rejected");
    fs::create_dir_all(&dir)?;
    // 같은 생산자가 이름을 재사용해도 이전 거부 파일을 보존한다.
    let target = if dir.join(name).exists() || dir.join(format!("{name}.error")).exists() {
        dir.join(format!("{}-{name}", uuid::Uuid::new_v4()))
    } else { dir.join(name) };
    fs::rename(root.join(name), &target)?;
    let mut error = fs::File::create(target.with_file_name(format!("{}.error", target.file_name().unwrap().to_string_lossy())))?;
    writeln!(error, "{}", reason.code())
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
                entries.push(InboxEntry { file_name: name, key, title: parsed.title, body: parsed.body, assignee: parsed.assignee });
            }
            Err(reason) => reject(root, &name, reason).map_err(|e| e.to_string())?,
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_plain_frontmatter_bom_crlf_and_preserves_body_indentation() {
        assert_eq!(parse("x.md", b"\n# Title\n\n  code  \n\n").unwrap(), Parsed { title: "Title".into(), body: "  code  ".into(), assignee: None });
        let p = parse("x.md", "\u{feff}---\r\nfuture: ignored\r\nassignee: 하나 \r\n---\r\n# 제목\r\n본문\r\n".as_bytes()).unwrap();
        assert_eq!(p, Parsed { title: "제목".into(), body: "본문".into(), assignee: Some("하나".into()) });
        assert_eq!(parse("x.md", b"Title").unwrap().body, "");
        assert_eq!(parse("x.md", b"# \nBody").unwrap(), Parsed { title: "".into(), body: "Body".into(), assignee: None });
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
    #[cfg(unix)]
    #[test]
    fn ignores_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("link.md")).unwrap();
        assert!(scan(root.path(), SystemTime::now()).unwrap().is_empty());
    }
}
