// src/renderer/markdown/MarkdownEditorOverlay.tsx
//
// 마크다운 편집기 오버레이(이슈 #10). 상단 바(relPath·더티 ●·소스/미리보기 토글·
// 닫기), 본문은 소스 모드=모노스페이스 textarea, 미리보기 모드=marked+DOMPurify
// 렌더(raw HTML sanitize, 내부 Markdown 링크 탐색, 외부 URL·로컬 파일 OS 위임).
// Cmd/Ctrl+S 저장, Cmd/Ctrl+P 팔레트 재오픈, Esc 닫기(더티면 확인 다이얼로그).
// 저장 충돌(CONFLICT)은 다시 불러오기/덮어쓰기/취소 다이얼로그로 해결한다.
//
// self-gate 관례: 항상 마운트, 편집기 없으면 null 렌더. 키 이벤트는 오버레이에서
// stopPropagation해 터미널/전역 단축키로 새지 않게 한다.
import { useMemo, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { openUrl } from "@tauri-apps/plugin-opener";
import { tauriApi } from "../ipc/tauriApi";
import { useMarkdownStore, isEditorDirty } from "./markdownStore";
import { resolveMarkdownLink } from "./markdownLinks";

// 링크가 웹뷰에서 새 창을 열지 못하게 target 속성을 제거한다. 클릭은 아래에서
// worktree 링크/OS 기본 앱으로 명시적으로 라우팅한다.
DOMPurify.addHook("afterSanitizeAttributes", (node) => {
  if (node.tagName === "A") node.removeAttribute("target");
});

/** 마크다운 → 안전한 HTML. marked는 동기 파싱(async 확장 없음), DOMPurify로 sanitize. */
function renderMarkdown(src: string): string {
  const raw = marked.parse(src, { async: false, gfm: true, breaks: false }) as string;
  return DOMPurify.sanitize(raw, { FORBID_ATTR: ["target"] });
}

function headingSlug(text: string): string {
  return text
    .trim()
    .toLocaleLowerCase()
    .replace(/[^\p{Letter}\p{Number}\s_-]/gu, "")
    .replace(/\s+/g, "-");
}

/** marked 18은 heading id를 만들지 않으므로 텍스트에서 GitHub식 slug를 계산한다. */
function scrollToFragment(fragment: string): void {
  const direct = document.getElementById(fragment);
  const preview = document.querySelector(".md-editor-preview");
  const target =
    direct ??
    [...(preview?.querySelectorAll<HTMLElement>("h1, h2, h3, h4, h5, h6") ?? [])].find(
      (heading) => headingSlug(heading.textContent ?? "") === fragment.toLocaleLowerCase(),
    );
  target?.scrollIntoView({ block: "start" });
}

export function MarkdownEditorOverlay() {
  const { t } = useTranslation("workdir");
  const editor = useMarkdownStore((s) => s.editor);
  const discardConfirm = useMarkdownStore((s) => s.discardConfirm);
  const setContent = useMarkdownStore((s) => s.setContent);
  const setMode = useMarkdownStore((s) => s.setMode);
  const save = useMarkdownStore((s) => s.save);
  const requestClose = useMarkdownStore((s) => s.requestClose);
  const cancelDiscard = useMarkdownStore((s) => s.cancelDiscard);
  const closeEditor = useMarkdownStore((s) => s.closeEditor);
  const openPalette = useMarkdownStore((s) => s.openPalette);
  const reloadFromDisk = useMarkdownStore((s) => s.reloadFromDisk);
  const overwrite = useMarkdownStore((s) => s.overwrite);
  const cancelConflict = useMarkdownStore((s) => s.cancelConflict);
  const [pendingNavigation, setPendingNavigation] = useState<
    { kind: "link"; relPath: string; fragment: string } | { kind: "back" } | null
  >(null);
  const openLinkedFile = useMarkdownStore((s) => s.openLinkedFile);
  const goBack = useMarkdownStore((s) => s.goBack);

  const content = editor?.content ?? "";
  const previewHtml = useMemo(
    () => (editor?.mode === "preview" ? renderMarkdown(content) : ""),
    [editor?.mode, content],
  );

  if (!editor) return null;
  const dirty = isEditorDirty(editor);
  const canGoBack = (editor.history?.length ?? 0) > 0;

  const performNavigation = (
    action: { kind: "link"; relPath: string; fragment: string } | { kind: "back" },
  ) => {
    if (action.kind === "back") {
      goBack();
      return;
    }
    void openLinkedFile(action.relPath).then(() => {
      if (action.fragment) {
        window.setTimeout(() => scrollToFragment(action.fragment), 0);
      }
    });
  };

  const requestNavigation = (
    action: { kind: "link"; relPath: string; fragment: string } | { kind: "back" },
  ) => {
    if (dirty) {
      setPendingNavigation(action);
      requestClose();
      return;
    }
    performNavigation(action);
  };

  const openPreviewLink = (event: React.MouseEvent<HTMLDivElement>) => {
    const link = (event.target as HTMLElement).closest("a");
    if (!link) return;
    event.preventDefault();
    const href = link.getAttribute("href");
    if (!href) return;
    const target = resolveMarkdownLink(href, editor.relPath);
    if (target.kind === "anchor") {
      scrollToFragment(target.fragment);
    } else if (target.kind === "markdown") {
      requestNavigation({ kind: "link", relPath: target.relPath, fragment: target.fragment });
    } else if (target.kind === "local") {
      void tauriApi
        .markdownOpenLocalLink(editor.root, target.relPath)
        .catch((err) => console.warn("markdown: local link open failed", err));
    } else if (target.kind === "external") {
      void openUrl(target.url).catch((err) =>
        console.warn("markdown: external link open failed", err),
      );
    }
  };

  // 더티 가드 다이얼로그에서 저장한 뒤 닫기 또는 요청한 문서로 이동한다.
  const saveThenContinue = async () => {
    const res = await save();
    if (res.ok) {
      if (pendingNavigation) {
        const action = pendingNavigation;
        setPendingNavigation(null);
        cancelDiscard();
        performNavigation(action);
      } else {
        closeEditor();
      }
    }
    // 충돌이면 save가 conflict 플래그를 세팅 → 아래 충돌 다이얼로그가 뜬다.
    // (discardConfirm은 cancelDiscard로 접어 충돌 다이얼로그만 남긴다.)
    else if (!res.ok && res.conflict) cancelDiscard();
  };

  const discardThenContinue = () => {
    if (!pendingNavigation) {
      closeEditor();
      return;
    }
    const action = pendingNavigation;
    setPendingNavigation(null);
    // 링크를 따라가기 전 문서를 history에 담더라도 버리기로 한 내용이 다시
    // 나타나지 않도록 기준선으로 되돌린 뒤 이동한다.
    setContent(editor.baseline);
    cancelDiscard();
    performNavigation(action);
  };

  const cancelPendingAction = () => {
    setPendingNavigation(null);
    cancelDiscard();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    // 편집기 내부 키가 터미널/전역 단축키로 새지 않게 막는다.
    e.stopPropagation();
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key.toLowerCase() === "s") {
      e.preventDefault();
      void save();
      return;
    }
    if (mod && e.key.toLowerCase() === "p") {
      e.preventDefault();
      openPalette(editor.root, editor.agentId);
      return;
    }
    if (e.key === "Escape") {
      // 충돌/더티 다이얼로그가 떠 있으면 Esc는 그 다이얼로그가 처리하도록 둔다.
      if (editor.conflict || discardConfirm) return;
      e.preventDefault();
      setPendingNavigation(null);
      requestClose();
    }
  };

  return (
    <div className="md-overlay md-editor-overlay" onKeyDown={onKeyDown}>
      <div
        className="md-editor"
        role="dialog"
        aria-label={t("markdown.editorAria", { path: editor.relPath })}
      >
        <div className="md-editor-bar">
          <span className="md-editor-path" title={editor.relPath}>
            {editor.relPath}
            {dirty && (
              <span className="md-editor-dirty" aria-label={t("markdown.unsaved")}>
                ●
              </span>
            )}
          </span>
          <div className="md-editor-bar-actions">
            {canGoBack && (
              <button
                type="button"
                className="md-tab"
                aria-label={t("markdown.backDocument")}
                title={t("markdown.backDocument")}
                onClick={() => requestNavigation({ kind: "back" })}
              >
                ←
              </button>
            )}
            <button
              type="button"
              className={editor.mode === "source" ? "md-tab md-tab-active" : "md-tab"}
              aria-pressed={editor.mode === "source"}
              onClick={() => setMode("source")}
            >
              {t("markdown.tabSource")}
            </button>
            <button
              type="button"
              className={editor.mode === "preview" ? "md-tab md-tab-active" : "md-tab"}
              aria-pressed={editor.mode === "preview"}
              onClick={() => setMode("preview")}
            >
              {t("markdown.tabPreview")}
            </button>
            <button
              type="button"
              className="md-editor-close"
              aria-label={t("markdown.closeEditor")}
              onClick={() => {
                setPendingNavigation(null);
                requestClose();
              }}
            >
              ×
            </button>
          </div>
        </div>

        <div className="md-editor-body">
          {editor.loading ? (
            <div className="md-editor-status">{t("markdown.loading")}</div>
          ) : editor.loadError ? (
            <div className="md-editor-status md-editor-error">
              {t("markdown.loadError", { error: editor.loadError })}
            </div>
          ) : editor.mode === "source" ? (
            <textarea
              className="md-editor-textarea"
              value={content}
              spellCheck={false}
              onChange={(e) => setContent(e.target.value)}
              // eslint 접근성: 편집기는 열릴 때 본문에 포커스가 가는 게 자연스럽다.
              autoFocus
            />
          ) : (
            <div
              className="md-editor-preview"
              onClick={openPreviewLink}
              // marked+DOMPurify로 sanitize한 HTML만 주입한다.
              dangerouslySetInnerHTML={{ __html: previewHtml }}
            />
          )}
        </div>
      </div>

      {/* 더티 가드: 저장 후 닫기 / 버리고 닫기 / 취소 */}
      {discardConfirm && (
        <div
          className="md-inner-backdrop"
          onMouseDown={(e) => {
            if (e.button === 0 && e.target === e.currentTarget) cancelPendingAction();
          }}
        >
          <div className="pixel-panel md-confirm">
            <h2 className="pixel-title">{t("markdown.unsaved")}</h2>
            <p>
              <Trans
                t={t}
                i18nKey="markdown.discardBody"
                values={{ path: editor.relPath }}
                components={{ b: <strong /> }}
              />
            </p>
            <div className="dialog-actions">
              <button className="pixel-btn primary" onClick={() => void saveThenContinue()}>
                {t(pendingNavigation ? "markdown.saveAndNavigate" : "markdown.saveAndClose")}
              </button>
              <button className="pixel-btn" onClick={discardThenContinue}>
                {t(pendingNavigation ? "markdown.discardAndNavigate" : "markdown.discardAndClose")}
              </button>
              <button className="pixel-btn" onClick={cancelPendingAction}>
                {t("markdown.cancel")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 저장 충돌 해결: 다시 불러오기 / 덮어쓰기 / 취소 */}
      {editor.conflict && (
        <div
          className="md-inner-backdrop"
          onMouseDown={(e) => {
            if (e.button === 0 && e.target === e.currentTarget) cancelConflict();
          }}
        >
          <div className="pixel-panel md-confirm">
            <h2 className="pixel-title">{t("markdown.conflictTitle")}</h2>
            <p>{t("markdown.conflictBody")}</p>
            <div className="dialog-actions">
              <button className="pixel-btn" onClick={() => void reloadFromDisk()}>
                {t("markdown.conflictReload")}
              </button>
              <button className="pixel-btn primary" onClick={() => void overwrite()}>
                {t("markdown.conflictOverwrite")}
              </button>
              <button className="pixel-btn" onClick={cancelConflict}>
                {t("markdown.cancel")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
