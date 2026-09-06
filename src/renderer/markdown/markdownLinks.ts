// Markdown preview links are intentionally resolved without Node's path module:
// the renderer also runs in a browser context.

export type MarkdownLinkTarget =
  | { kind: "anchor"; fragment: string }
  | { kind: "markdown"; relPath: string; fragment: string }
  | { kind: "local"; relPath: string }
  | { kind: "external"; url: string }
  | { kind: "invalid" };

const MARKDOWN_EXTENSION = /\.(md|mdx|markdown)$/i;

function decode(value: string): string | null {
  try {
    return decodeURIComponent(value);
  } catch {
    return null;
  }
}

/** Resolves a relative markdown href and rejects paths that leave its worktree. */
export function resolveMarkdownLink(href: string, currentRelPath: string): MarkdownLinkTarget {
  const value = href.trim();
  if (!value) return { kind: "invalid" };

  // Tauri opener의 기본 URL 권한과 같은 웹/통신 scheme만 OS에 넘긴다.
  // file:/javascript:/data:와 알 수 없는 scheme은 로컬 경계나 웹뷰 보안을
  // 우회하지 못하게 거부한다.
  if (value.startsWith("//")) return { kind: "external", url: `https:${value}` };
  if (/^(https?|mailto|tel):/i.test(value)) return { kind: "external", url: value };
  if (/^[a-z][a-z\d+.-]*:/i.test(value)) return { kind: "invalid" };

  const hashIndex = value.indexOf("#");
  const queryIndex = value.indexOf("?");
  const end = Math.min(
    hashIndex === -1 ? value.length : hashIndex,
    queryIndex === -1 ? value.length : queryIndex,
  );
  const pathPart = value.slice(0, end);
  const fragment = hashIndex === -1 ? "" : decode(value.slice(hashIndex + 1).split("?", 1)[0]) ?? "";

  // `#heading` stays in the currently rendered document.
  if (!pathPart) return fragment ? { kind: "anchor", fragment } : { kind: "invalid" };

  const decodedPath = decode(pathPart);
  if (decodedPath === null || decodedPath.includes("\\") || decodedPath.includes("\0")) {
    return { kind: "invalid" };
  }

  const parts = decodedPath.startsWith("/")
    ? []
    : currentRelPath.split("/").slice(0, -1).filter(Boolean);
  for (const part of decodedPath.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (parts.length === 0) return { kind: "invalid" };
      parts.pop();
    } else {
      parts.push(part);
    }
  }
  if (parts.length === 0) return { kind: "invalid" };
  const relPath = parts.join("/");
  return MARKDOWN_EXTENSION.test(relPath)
    ? { kind: "markdown", relPath, fragment }
    : { kind: "local", relPath };
}
