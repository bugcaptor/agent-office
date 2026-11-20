// src/renderer/markdown/renderMarkdown.ts
//
// 마크다운 → 안전한 HTML(marked + DOMPurify). 편집기 미리보기와 토의 결정 메모 보기가 함께 쓴다.
// 메모는 에이전트가 쓴 글이라 원문 HTML을 반드시 거른다.
import { marked } from "marked";
import DOMPurify from "dompurify";

// 링크가 웹뷰에서 새 창을 열지 못하게 target 속성을 제거한다. 클릭은 쓰는 쪽에서
// worktree 링크/OS 기본 앱으로 명시적으로 라우팅한다.
DOMPurify.addHook("afterSanitizeAttributes", (node) => {
  if (node.tagName === "A") node.removeAttribute("target");
});

/** marked는 동기 파싱(async 확장 없음), DOMPurify로 sanitize. */
export function renderMarkdown(src: string): string {
  const raw = marked.parse(src, { async: false, gfm: true, breaks: false }) as string;
  return DOMPurify.sanitize(raw, { FORBID_ATTR: ["target"] });
}
