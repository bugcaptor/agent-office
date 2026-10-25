//! 문서 뷰어 변환기(docs/doc-view-design.md §구성 1). Markdown 파일 하나를 브라우저로
//! 바로 열 수 있는 완성된 HTML 문서로 바꾼다. `agent-office md`(doc_view_cli.rs)와 앱
//! 미리보기 버튼이 같은 경로를 써서 결과가 하나로 맞는다.
//!
//! - GFM은 `pulldown-cmark`, 원문 HTML 거르기는 `ammonia`. 우리가 넣는 스크립트는 거른
//!   뒤에 덧붙인다.
//! - 상대 링크·이미지는 거르기 단계에서 원본 폴더 기준 `file://` 절대 URL로 바꾼다.
//! - highlight.js·mermaid는 CDN에서 버전 고정으로 불러온다. 오프라인이면 하이라이트 없는
//!   코드·원문 mermaid 블록이 그대로 보인다.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 앱 데이터 아래 캐시 폴더. 작업 트리에는 파일을 만들지 않는다.
pub const CACHE_DIR: &str = "doc-view";
/// `doc-view/` 아래 열기 요청 폴더(§구성 2). 앱이 감시해 요청 파일을 지운다.
pub const REQUEST_DIR: &str = "open";
/// `--latest`와 CLI가 받는 확장자(소문자, 점 없이).
pub const MARKDOWN_EXTENSIONS: [&str; 2] = ["md", "markdown"];

const HTML_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// CLI는 요청을 2초 기다린 뒤 스스로 지운다. 이보다 오래 남은 요청은 처리 못 한 것이다.
const REQUEST_TTL: Duration = Duration::from_secs(60);

const HLJS: &str = "https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.12.0";
const MERMAID: &str = "https://cdn.jsdelivr.net/npm/mermaid@11.17.2/dist/mermaid.esm.min.mjs";

/// `source`(절대 경로)의 `markdown`을 완성된 HTML 문서로 바꾼다.
pub fn render_document(source: &Path, markdown: &str) -> String {
    render_document_at(source, markdown, &chrono::Local::now().format("%Y-%m-%d %H:%M").to_string())
}

fn render_document_at(source: &Path, markdown: &str, converted_at: &str) -> String {
    let body = render_body(source, markdown);
    let title = body.title.unwrap_or_else(|| {
        source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    });
    let source_display = source.to_string_lossy();
    let mut head_links = String::new();
    let mut scripts = String::new();
    if body.has_code {
        head_links.push_str(&format!(
            "<link rel=\"stylesheet\" href=\"{HLJS}/styles/github.min.css\" media=\"(prefers-color-scheme: light)\">\n\
             <link rel=\"stylesheet\" href=\"{HLJS}/styles/github-dark.min.css\" media=\"(prefers-color-scheme: dark)\">\n"
        ));
        scripts.push_str(&format!(
            "<script src=\"{HLJS}/highlight.min.js\"></script>\n\
             <script>if (window.hljs) document.querySelectorAll('pre code[class*=\"language-\"]').forEach((el) => hljs.highlightElement(el));</script>\n"
        ));
    }
    if body.has_mermaid {
        scripts.push_str(&format!(
            "<script type=\"module\">\n\
             import mermaid from \"{MERMAID}\";\n\
             const dark = matchMedia(\"(prefers-color-scheme: dark)\").matches;\n\
             mermaid.initialize({{ startOnLoad: false, theme: dark ? \"dark\" : \"default\" }});\n\
             await mermaid.run({{ querySelector: \"pre.mermaid\" }});\n\
             </script>\n"
        ));
    }
    format!(
        "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"color-scheme\" content=\"light dark\">\n\
         <title>{title}</title>\n{head_links}<style>{STYLE}</style>\n</head>\n<body>\n\
         <header class=\"doc-meta\"><a href=\"{source_url}\">{source}</a> · {converted_at}</header>\n\
         <main>\n{html}</main>\n{scripts}</body>\n</html>\n",
        title = escape_html(&title),
        source_url = escape_html(&file_url(source)),
        source = escape_html(&source_display),
        converted_at = escape_html(converted_at),
        html = body.html,
    )
}

struct Body {
    html: String,
    title: Option<String>,
    has_code: bool,
    has_mermaid: bool,
}

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_HEADING_ATTRIBUTES
}

fn render_body(source: &Path, markdown: &str) -> Body {
    let events: Vec<Event> = Parser::new_ext(markdown, options()).collect();
    let mut out: Vec<Event> = Vec::with_capacity(events.len());
    let mut title = None;
    let mut has_code = false;
    let mut has_mermaid = false;
    let mut mermaid: Option<String> = None;
    for (index, event) in events.iter().enumerate() {
        if let Some(buffer) = mermaid.as_mut() {
            match event {
                Event::Text(text) => buffer.push_str(text),
                Event::End(TagEnd::CodeBlock) => {
                    out.push(Event::Html(format!("<pre class=\"mermaid\">{}</pre>\n", escape_html(buffer)).into()));
                    mermaid = None;
                }
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(Tag::Heading { level, id, classes, attrs }) => {
                let text = heading_text(&events[index + 1..]);
                if *level == HeadingLevel::H1 && title.is_none() && !text.trim().is_empty() {
                    title = Some(text.trim().to_string());
                }
                let id = id.clone().or_else(|| Some(heading_slug(&text).into()));
                out.push(Event::Start(Tag::Heading { level: *level, id, classes: classes.clone(), attrs: attrs.clone() }));
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                match info.split_whitespace().next() {
                    Some("mermaid") => {
                        has_mermaid = true;
                        mermaid = Some(String::new());
                        continue;
                    }
                    Some(_) => has_code = true,
                    None => {}
                }
                out.push(event.clone());
            }
            _ => out.push(event.clone()),
        }
    }
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, out.into_iter());
    Body { html: sanitize(source.parent().unwrap_or(Path::new("/")), &html), title, has_code, has_mermaid }
}

/// 제목 시작 바로 뒤부터 제목 끝까지의 글자(인라인 코드 포함) — DOM의 textContent와 같다.
fn heading_text(events: &[Event]) -> String {
    let mut text = String::new();
    for event in events {
        match event {
            Event::End(TagEnd::Heading(_)) => break,
            Event::Text(t) | Event::Code(t) => text.push_str(t),
            _ => {}
        }
    }
    text
}

/// GitHub식 제목 slug. `MarkdownEditorOverlay.tsx`의 `headingSlug`와 같은 규칙이다
/// (글자·숫자·공백·`_`·`-`만 남기고 공백 덩어리를 `-`로). JS의 `\p{Letter}`와 Rust의
/// `is_alphabetic`은 결합 모음 일부에서만 다르다.
pub fn heading_slug(text: &str) -> String {
    let kept: String = text
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphabetic() || c.is_numeric() || c.is_whitespace() || *c == '_' || *c == '-')
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join("-")
}

/// 변환한 HTML(원문 HTML 포함)을 거른다. script·style·이벤트 속성은 빠지고, 상대
/// URL은 `base`(원본 폴더) 기준 `file://` URL이 된다.
fn sanitize(base: &Path, html: &str) -> String {
    ammonia::Builder::default()
        .add_generic_attributes(["id", "class"])
        .add_tags(["input"])
        .add_tag_attributes("input", ["type", "checked", "disabled"])
        // pulldown-cmark가 표 정렬을 style로 내보낸다.
        .add_tag_attributes("th", ["style"])
        .add_tag_attributes("td", ["style"])
        .add_url_schemes(["file"])
        .url_relative(ammonia::UrlRelative::Custom(Box::new(RelativeToSource(base.to_path_buf()))))
        .clean(html)
        .to_string()
}

struct RelativeToSource(PathBuf);
impl ammonia::UrlRelativeEvaluate<'static> for RelativeToSource {
    fn evaluate<'u>(&self, url: &'u str) -> Option<Cow<'u, str>> {
        rewrite_relative(&self.0, url)
    }
}

/// 상대 URL 하나를 바꾼다. `#앵커`와 `//host` 형태는 그대로 둔다.
fn rewrite_relative<'u>(base: &Path, url: &'u str) -> Option<Cow<'u, str>> {
    if url.is_empty() || url.starts_with('#') || url.starts_with("//") {
        return Some(Cow::Borrowed(url));
    }
    let split = url.find(['?', '#']).unwrap_or(url.len());
    let (path, suffix) = url.split_at(split);
    let decoded = percent_decode(path);
    let path = Path::new(&decoded);
    let absolute = if decoded.starts_with('/') || path.is_absolute() { path.to_path_buf() } else { base.join(path) };
    Some(Cow::Owned(format!("{}{suffix}", file_url(&absolute))))
}

/// 절대 경로 → `file://` URL. 경로 구분자 밖의 예약 문자는 퍼센트 인코딩한다.
pub fn file_url(path: &Path) -> String {
    let mut text = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        text = text.replace('\\', "/");
    }
    if !text.starts_with('/') {
        text.insert(0, '/');
    }
    let mut url = String::from("file://");
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => url.push(byte as char),
            _ => url.push_str(&format!("%{byte:02X}")),
        }
    }
    url
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((high * 16 + low) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// `doc-view/<원본 절대 경로 sha1 앞 12자>-<파일 stem>.html`. 같은 문서는 같은 경로라
/// 다시 열면 덮어써서 브라우저 새로고침으로 최신이 된다.
pub fn cache_path(data_dir: &Path, source: &Path) -> PathBuf {
    let hash = sha1_smol::Sha1::from(source.to_string_lossy().as_bytes()).digest().to_string();
    let stem: String = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' })
        .collect();
    data_dir.join(CACHE_DIR).join(format!("{}-{stem}.html", &hash[..12]))
}

pub fn has_markdown_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| MARKDOWN_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// `dir` 아래에서 수정 시각이 가장 늦은 Markdown 파일. `.gitignore`·숨김을 존중한다
/// (`file_scan::walk_files`와 같은 WalkBuilder 설정). 같은 시각이면 경로가 앞선 것.
pub fn latest_markdown(dir: &Path) -> Option<PathBuf> {
    let root = std::fs::canonicalize(dir).ok()?;
    let mut builder = ignore::WalkBuilder::new(&root);
    builder
        .follow_links(false)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true)
        .parents(true)
        .require_git(false)
        .filter_entry(|entry| entry.depth() == 0 || entry.file_name() != ".git");
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for entry in builder.build().flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) || !has_markdown_extension(entry.path()) {
            continue;
        }
        let Some(modified) = entry.metadata().ok().and_then(|m| m.modified().ok()) else { continue };
        let newer = match &best {
            None => true,
            Some((time, path)) => modified > *time || (modified == *time && entry.path() < path.as_path()),
        };
        if newer {
            best = Some((modified, entry.into_path()));
        }
    }
    best.map(|(_, path)| path)
}

/// 부팅 때 캐시를 치운다: 7일 지난 HTML·남은 임시 파일과 처리 못 한 열기 요청.
/// 지운 파일 수를 돌려준다.
pub fn gc(data_dir: &Path) -> usize {
    gc_at(data_dir, SystemTime::now())
}

fn gc_at(data_dir: &Path, now: SystemTime) -> usize {
    let cache = data_dir.join(CACHE_DIR);
    remove_older(&cache, now, HTML_TTL, |name| name.ends_with(".html") || name.contains(".tmp-"))
        + remove_older(&cache.join(REQUEST_DIR), now, REQUEST_TTL, |_| true)
}

/// 캐시 폴더 `doc-view/`. Codex 세션의 쓰기 허용 경로이기도 하다(§구성 4).
pub fn cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(CACHE_DIR)
}

/// 열기 요청 폴더 `doc-view/open/`.
pub fn request_dir(data_dir: &Path) -> PathBuf {
    cache_dir(data_dir).join(REQUEST_DIR)
}

/// CLI가 앱에 "이 HTML을 열어 달라"는 요청 `open/<유닉스 밀리초>-<난수 8자>.json`
/// (`{ "html": "<절대 경로>" }`)을 원자적으로 쓰고 그 경로를 돌려준다. 쓰는 중의 임시
/// 파일은 `.json`으로 끝나지 않아 감시가 집지 않는다.
pub fn write_open_request(data_dir: &Path, html: &Path) -> std::io::Result<PathBuf> {
    let millis = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis();
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let path = request_dir(data_dir).join(format!("{millis}-{}.json", &nonce[..8]));
    let body = serde_json::json!({ "html": html.to_string_lossy() }).to_string();
    crate::persistence::durable::write_atomic(&path, body.as_bytes())?;
    Ok(path)
}

/// 요청 본문이 가리키는 HTML. `doc-view/` 바로 아래 실재하는 `.html` 파일일 때만
/// 정규화한 경로를 돌려준다. 그 밖(폴더 밖·`..`·심볼릭 링크로 빠지는 경로, URL, 다른
/// 확장자, 깨진 JSON)은 None — 앱이 임의 파일·URL을 열지 않게 한다.
pub fn resolve_open_request(data_dir: &Path, body: &[u8]) -> Option<PathBuf> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let html = Path::new(value.get("html")?.as_str()?);
    if !html.is_absolute() {
        return None;
    }
    let target = std::fs::canonicalize(html).ok()?;
    let cache = std::fs::canonicalize(data_dir.join(CACHE_DIR)).ok()?;
    let is_html = target.extension().is_some_and(|e| e.eq_ignore_ascii_case("html"));
    (target.parent() == Some(cache.as_path()) && is_html && target.is_file()).then_some(target)
}

/// 앱 쪽 한 번의 훑기: `open/`의 `*.json` 요청을 읽어 지우고, 지우기에 성공한 것 중 유효한
/// HTML만 돌려준다. 지우기 성공을 "내가 맡았다"로 삼아 CLI 폴백과 둘 다 여는 일을 막는다.
pub fn take_open_requests(data_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(request_dir(data_dir)) else { return vec![] };
    let mut names: Vec<_> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
        .map(|e| e.path())
        .collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|path| {
            let body = std::fs::read(&path).ok();
            std::fs::remove_file(&path).ok()?;
            resolve_open_request(data_dir, &body?)
        })
        .collect()
}

/// 앱 부팅 때 띄우는 요청 감시 스레드(§구성 3). 업무 패널 설정과 무관하게 돈다.
pub fn spawn_request_watcher(
    data_dir: PathBuf,
    interval: Duration,
    open: impl Fn(&Path) -> Result<(), String> + Send + 'static,
) {
    let _ = std::thread::Builder::new().name("doc-view-open".into()).spawn(move || loop {
        for html in take_open_requests(&data_dir) {
            if let Err(error) = open(&html) {
                eprintln!("agent-office: failed to open {}: {error}", html.display());
            }
        }
        std::thread::sleep(interval);
    });
}

fn remove_older(dir: &Path, now: SystemTime, ttl: Duration, matches: impl Fn(&str) -> bool) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() || !matches(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let expired = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age > ttl);
        if expired && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

const STYLE: &str = r#"
:root { --fg: #1f2328; --muted: #59636e; --bg: #ffffff; --line: #d1d9e0; --code-bg: #f6f8fa; --link: #0969da; }
@media (prefers-color-scheme: dark) {
  :root { --fg: #e6edf3; --muted: #9198a1; --bg: #0d1117; --line: #3d444d; --code-bg: #151b23; --link: #4493f8; }
}
* { box-sizing: border-box; }
html { background: var(--bg); color: var(--fg); }
body { margin: 0 auto; max-width: 860px; padding: 24px 32px 64px; font: 16px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", "Noto Sans", "Apple SD Gothic Neo", "Malgun Gothic", sans-serif; overflow-wrap: break-word; }
a { color: var(--link); text-decoration: none; }
a:hover { text-decoration: underline; }
.doc-meta { color: var(--muted); font-size: 12px; padding-bottom: 8px; margin-bottom: 24px; border-bottom: 1px solid var(--line); word-break: break-all; }
.doc-meta a { color: inherit; }
h1, h2, h3, h4, h5, h6 { margin: 24px 0 16px; line-height: 1.25; font-weight: 600; }
h1 { font-size: 2em; padding-bottom: .3em; border-bottom: 1px solid var(--line); }
h2 { font-size: 1.5em; padding-bottom: .3em; border-bottom: 1px solid var(--line); }
h3 { font-size: 1.25em; }
p, ul, ol, table, pre, blockquote, details { margin: 0 0 16px; }
ul, ol { padding-left: 2em; }
li + li { margin-top: .25em; }
li > input[type="checkbox"] { margin: 0 .4em 0 -1.3em; vertical-align: middle; }
ul:has(> li > input[type="checkbox"]) { list-style: none; }
blockquote { padding: 0 1em; color: var(--muted); border-left: .25em solid var(--line); }
code, pre { font: 85%/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
code { padding: .2em .4em; background: var(--code-bg); border-radius: 6px; }
pre { padding: 16px; overflow: auto; background: var(--code-bg); border-radius: 6px; }
pre code, pre code.hljs { padding: 0; background: transparent; font-size: 100%; }
pre.mermaid[data-processed] { background: transparent; text-align: center; }
table { border-collapse: collapse; display: block; max-width: 100%; overflow: auto; }
th, td { padding: 6px 13px; border: 1px solid var(--line); }
th { font-weight: 600; }
tr:nth-child(2n) { background: var(--code-bg); }
img { max-width: 100%; }
hr { height: 2px; border: 0; background: var(--line); margin: 24px 0; }
.footnote-definition { font-size: 85%; color: var(--muted); display: flex; gap: .5em; }
.footnote-definition p { margin: 0; }
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn render(markdown: &str) -> String {
        render_document_at(Path::new("/docs/plan.md"), markdown, "2026-10-25 12:00")
    }

    #[test]
    fn renders_gfm_tables_tasklists_footnotes_strikethrough() {
        let html = render("| a | b |\n|:--|--:|\n| 1 | 2 |\n\n- [x] done\n- [ ] todo\n\n~~old~~ note[^1]\n\n[^1]: footnote body\n");
        assert!(html.contains("<table>"), "{html}");
        assert!(html.contains("<td style=\"text-align: left\">1</td>"), "{html}");
        assert!(html.contains("type=\"checkbox\""), "{html}");
        assert!(html.contains("checked"), "{html}");
        assert!(html.contains("<del>old</del>"), "{html}");
        assert!(html.contains("class=\"footnote-definition\""), "{html}");
    }

    #[test]
    fn heading_ids_follow_editor_slug_rule() {
        assert_eq!(heading_slug("  Hello, World!  "), "hello-world");
        assert_eq!(heading_slug("문서 뷰어 (`office-md`) 설계"), "문서-뷰어-office-md-설계");
        assert_eq!(heading_slug("a_b - c"), "a_b---c");
        let html = render("# 문서 뷰어 `office-md`\n\n## Second {#custom}\n");
        assert!(html.contains("<h1 id=\"문서-뷰어-office-md\">"), "{html}");
        assert!(html.contains("<h2 id=\"custom\">"), "{html}");
    }

    #[test]
    fn title_is_first_h1_or_file_name() {
        assert!(render("intro\n\n## Sub\n\n# Main & `<Title>`\n\n# Later\n").contains("<title>Main &amp; &lt;Title&gt;</title>"));
        assert!(render("no heading\n").contains("<title>plan.md</title>"));
    }

    #[test]
    fn mermaid_block_becomes_pre_and_loads_pinned_script() {
        let html = render("```mermaid\ngraph TD\n  A-->B\n```\n");
        assert!(html.contains("<pre class=\"mermaid\">graph TD\n  A--&gt;B\n</pre>"), "{html}");
        assert!(html.contains(MERMAID), "{html}");
        assert!(html.contains("theme: dark ? \"dark\" : \"default\""), "{html}");
        assert!(!html.contains("highlight.min.js"), "no code block, no highlight.js: {html}");
    }

    #[test]
    fn code_blocks_keep_language_class_and_load_highlight_js() {
        let html = render("```rust\nfn main() {}\n```\n");
        assert!(html.contains("<code class=\"language-rust\">"), "{html}");
        assert!(html.contains(&format!("{HLJS}/highlight.min.js")), "{html}");
        assert!(html.contains("github-dark.min.css\" media=\"(prefers-color-scheme: dark)\""), "{html}");
        assert!(!html.contains(MERMAID), "{html}");
        assert!(!render("plain\n").contains("<script"));
    }

    #[test]
    fn raw_html_is_sanitized_but_our_scripts_survive() {
        let html = render("<script>alert(1)</script>\n\n<img src=\"x.png\" onerror=\"alert(2)\">\n\n<a href=\"javascript:alert(3)\">x</a>\n\n<details><summary>more</summary>hidden</details>\n\n```mermaid\nA\n```\n");
        assert!(!html.contains("alert(1)"), "{html}");
        assert!(!html.contains("onerror"), "{html}");
        assert!(!html.contains("javascript:"), "{html}");
        assert!(html.contains("<details><summary>more</summary>"), "{html}");
        assert!(html.contains("<script type=\"module\">"), "{html}");
    }

    #[test]
    fn relative_links_and_images_become_file_urls() {
        let html = render("![img](images/a%20b.png) [doc](other.md#sec) [up](../x.md) [abs](/etc/hosts) [web](https://example.com/a) [mail](mailto:a@b.c) [anchor](#top) <img src=\"raw.png\">\n");
        assert!(html.contains("src=\"file:///docs/images/a%20b.png\""), "{html}");
        assert!(html.contains("href=\"file:///docs/other.md#sec\""), "{html}");
        assert!(html.contains("href=\"file:///docs/../x.md\""), "{html}");
        assert!(html.contains("href=\"file:///etc/hosts\""), "{html}");
        assert!(html.contains("href=\"https://example.com/a\""), "{html}");
        assert!(html.contains("href=\"mailto:a@b.c\""), "{html}");
        assert!(html.contains("href=\"#top\""), "{html}");
        assert!(html.contains("src=\"file:///docs/raw.png\""), "{html}");
    }

    #[test]
    fn header_shows_source_and_time_escaped() {
        let html = render_document_at(Path::new("/tmp/a & b/한글.md"), "x", "2026-10-25 12:00");
        assert!(html.contains("<a href=\"file:///tmp/a%20%26%20b/%ED%95%9C%EA%B8%80.md\">/tmp/a &amp; b/한글.md</a> · 2026-10-25 12:00"), "{html}");
        assert!(html.contains("prefers-color-scheme: dark"), "{html}");
    }

    #[test]
    fn percent_decode_handles_bad_and_trailing_escapes() {
        assert_eq!(percent_decode("a%20b%ED%95%9C"), "a b한");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%2"), "%zz%2");
        assert_eq!(percent_decode("%한글"), "%한글");
    }

    #[test]
    fn cache_path_is_stable_per_source() {
        let data = Path::new("/data");
        let a = cache_path(data, Path::new("/work/docs/plan.md"));
        assert_eq!(a, cache_path(data, Path::new("/work/docs/plan.md")));
        assert_ne!(a, cache_path(data, Path::new("/other/docs/plan.md")));
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(a.parent().unwrap(), data.join("doc-view"));
        assert_eq!(name.len(), 12 + "-plan.html".len(), "{name}");
        assert!(name.ends_with("-plan.html"), "{name}");
        let odd = cache_path(data, Path::new("/w/my doc?.md"));
        assert!(odd.to_string_lossy().ends_with("-my_doc_.html"), "{odd:?}");
    }

    fn touch(path: &Path, age: Duration) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    #[test]
    fn latest_markdown_picks_newest_and_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        touch(&root.join("old.md"), Duration::from_secs(300));
        touch(&root.join("sub/new.markdown"), Duration::from_secs(100));
        touch(&root.join("ignored/newest.md"), Duration::from_secs(1));
        touch(&root.join(".hidden/newest.md"), Duration::from_secs(1));
        touch(&root.join("notes.txt"), Duration::from_secs(0));
        let found = latest_markdown(root).unwrap();
        assert_eq!(found, fs::canonicalize(root).unwrap().join("sub/new.markdown"));
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(latest_markdown(empty.path()), None);
        assert_eq!(latest_markdown(&root.join("missing")), None);
    }

    #[test]
    fn gc_removes_expired_html_and_stale_requests() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join(CACHE_DIR);
        touch(&cache.join("old.html"), Duration::from_secs(8 * 24 * 3600));
        touch(&cache.join("fresh.html"), Duration::from_secs(3600));
        touch(&cache.join("keep.txt"), Duration::from_secs(8 * 24 * 3600));
        touch(&cache.join("open/stale.json"), Duration::from_secs(120));
        touch(&cache.join("open/pending.json"), Duration::from_secs(1));
        assert_eq!(gc(dir.path()), 2);
        assert!(!cache.join("old.html").exists());
        assert!(cache.join("fresh.html").exists());
        assert!(cache.join("keep.txt").exists());
        assert!(!cache.join("open/stale.json").exists());
        assert!(cache.join("open/pending.json").exists());
        assert_eq!(gc(&dir.path().join("missing")), 0);
    }

    fn request(html: &str) -> Vec<u8> {
        serde_json::json!({ "html": html }).to_string().into_bytes()
    }

    #[test]
    fn open_request_accepts_only_existing_html_directly_in_cache() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        let cache = data.join(CACHE_DIR);
        let good = cache.join("abc-plan.html");
        touch(&good, Duration::ZERO);
        touch(&cache.join("notes.txt"), Duration::ZERO);
        touch(&request_dir(data).join("nested.html"), Duration::ZERO);
        touch(&data.join("outside.html"), Duration::ZERO);
        let canonical = fs::canonicalize(&good).unwrap();
        assert_eq!(resolve_open_request(data, &request(&good.to_string_lossy())), Some(canonical.clone()));
        let dotted = cache.join("../doc-view/abc-plan.html");
        assert_eq!(resolve_open_request(data, &request(&dotted.to_string_lossy())), Some(canonical));
        let rejected = [
            request(&data.join("outside.html").to_string_lossy()),
            request(&cache.join("../outside.html").to_string_lossy()),
            request(&cache.join("notes.txt").to_string_lossy()),
            request(&request_dir(data).join("nested.html").to_string_lossy()),
            request(&cache.join("missing.html").to_string_lossy()),
            request("abc-plan.html"),
            request("https://example.com/a.html"),
            request(&format!("file://{}", good.display())),
            br#"{"html": 1}"#.to_vec(),
            b"{broken".to_vec(),
        ];
        for body in rejected {
            assert_eq!(resolve_open_request(data, &body), None, "{}", String::from_utf8_lossy(&body));
        }
    }

    #[cfg(unix)]
    #[test]
    fn open_request_rejects_symlink_escaping_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join(CACHE_DIR);
        touch(&dir.path().join("secret.html"), Duration::ZERO);
        fs::create_dir_all(&cache).unwrap();
        std::os::unix::fs::symlink(dir.path().join("secret.html"), cache.join("link.html")).unwrap();
        assert_eq!(resolve_open_request(dir.path(), &request(&cache.join("link.html").to_string_lossy())), None);
    }

    #[test]
    fn take_open_requests_consumes_json_and_skips_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        let html = data.join(CACHE_DIR).join("abc-plan.html");
        touch(&html, Duration::ZERO);
        let first = write_open_request(data, &html).unwrap();
        let name = first.file_name().unwrap().to_string_lossy().into_owned();
        let (millis, nonce) = name.strip_suffix(".json").unwrap().split_once('-').unwrap();
        assert!(millis.parse::<u128>().is_ok() && nonce.len() == 8, "{name}");
        fs::write(request_dir(data).join("0-bad.json"), request("https://example.com")).unwrap();
        fs::write(request_dir(data).join("x.json.tmp-123"), request(&html.to_string_lossy())).unwrap();
        assert_eq!(take_open_requests(data), vec![fs::canonicalize(&html).unwrap()]);
        let left: Vec<String> =
            fs::read_dir(request_dir(data)).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert_eq!(left, vec!["x.json.tmp-123".to_string()], "invalid requests are removed too");
        assert!(take_open_requests(data).is_empty());
        assert!(take_open_requests(&data.join("missing")).is_empty());
    }
}
