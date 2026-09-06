import { describe, expect, it } from "vitest";
import { resolveMarkdownLink } from "../markdownLinks";

describe("resolveMarkdownLink", () => {
  it("현재 문서 기준으로 percent-decoded Markdown 경로를 정규화한다", () => {
    expect(resolveMarkdownLink("../Guide%20One.mdx?view=1#intro", "docs/setup/readme.md")).toEqual({
      kind: "markdown",
      relPath: "docs/Guide One.mdx",
      fragment: "intro",
    });
  });

  it("fragment와 query는 확장자 판별에서 제외한다", () => {
    expect(resolveMarkdownLink("movie.mp4?download=1#top", "docs/readme.md")).toEqual({
      kind: "local",
      relPath: "docs/movie.mp4",
    });
  });

  it("anchor-only 링크는 현재 문서 안에서 처리한다", () => {
    expect(resolveMarkdownLink("#%EC%A0%9C%EB%AA%A9", "docs/readme.md")).toEqual({
      kind: "anchor",
      fragment: "제목",
    });
  });

  it("외부 URL은 브라우저 대상으로 남긴다", () => {
    expect(resolveMarkdownLink("https://example.com/a.md", "docs/readme.md")).toEqual({
      kind: "external",
      url: "https://example.com/a.md",
    });
    expect(resolveMarkdownLink("//example.com/manual", "docs/readme.md")).toEqual({
      kind: "external",
      url: "https://example.com/manual",
    });
  });

  it("실행 가능한 scheme과 임의 file URL은 거부한다", () => {
    expect(resolveMarkdownLink("javascript:alert(1)", "docs/readme.md")).toEqual({ kind: "invalid" });
    expect(resolveMarkdownLink("file:///etc/passwd", "docs/readme.md")).toEqual({ kind: "invalid" });
  });

  it("worktree 밖으로 나가는 경로와 잘못된 인코딩을 거부한다", () => {
    expect(resolveMarkdownLink("../../secret.md", "docs/readme.md")).toEqual({ kind: "invalid" });
    expect(resolveMarkdownLink("bad%ZZ.md", "docs/readme.md")).toEqual({ kind: "invalid" });
  });

});
