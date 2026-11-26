// src-tauri/src/webdash/markdown.rs
//
// 웹 상태판 대화 로그의 에이전트 말을 Markdown(GFM 일부)으로 그려 보낸다. 페이지는 외부 스크립트를
// 싣지 않으므로(CSP) 서버에서 HTML로 바꾸고 거른다. 앱의 대화 보기는 렌더러의 `renderMarkdown`을 쓴다.
//
// - 글에 섞인 원문 HTML은 태그로 살리지 않고 글자로 보인다(채팅에서 `<div>`를 말한 것일 때가 많다).
// - 링크는 http(s)·mailto만 남기고 새 탭으로 연다. 상대 경로(작업 폴더 파일)는 이 페이지 주소로 풀려
//   의미가 없으므로 링크를 떼고 글자만 남긴다 — 링크한 파일은 따로 다운로드 버튼이 붙는다.
// - 그림은 싣지 않는다(CSP가 바깥 그림을 막는다).
use pulldown_cmark::{html, Event, Options, Parser};

pub fn render(text: &str) -> String {
    let options = Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
    let events = Parser::new_ext(text, options).map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        other => other,
    });
    let mut out = String::with_capacity(text.len() * 3 / 2);
    html::push_html(&mut out, events);
    ammonia::Builder::default()
        .add_tags(["input"])
        .add_tag_attributes("input", ["type", "checked", "disabled"])
        .rm_tags(["img"])
        .url_schemes(["http", "https", "mailto"].into())
        .url_relative(ammonia::UrlRelative::Deny)
        .link_rel(Some("noopener noreferrer"))
        .set_tag_attribute_value("a", "target", "_blank")
        .clean(&out)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_common_markdown() {
        let html = render("# 제목\n\n**굵게** `code`\n\n- 하나\n- 둘\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn x() {}\n```");
        for want in ["<h1>제목</h1>", "<strong>굵게</strong>", "<code>code</code>", "<li>하나</li>", "<table>", "<pre><code>fn x() {}"] {
            assert!(html.contains(want), "{want} in {html}");
        }
    }

    #[test]
    fn raw_html_shows_as_text_and_scripts_never_run() {
        let html = render("<script>alert(1)</script> 그리고 <b onclick=\"x()\">b</b>");
        assert!(!html.contains("<script"));
        assert!(!html.contains("<b"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn keeps_only_web_links_and_drops_images() {
        let html = render("[웹](https://example.com) [파일](src/main.rs) [js](javascript:alert(1)) ![그림](https://example.com/a.png)");
        assert!(html.contains("href=\"https://example.com\""));
        assert!(html.contains("target=\"_blank\""));
        assert!(!html.contains("src/main.rs\""));
        assert!(!html.contains("javascript:"));
        assert!(!html.contains("<img"));
        assert!(html.contains("파일"));
    }
}
