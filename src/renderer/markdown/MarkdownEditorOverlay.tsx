// src/renderer/markdown/MarkdownEditorOverlay.tsx
//
// 마크다운 편집기 오버레이(이슈 #10). 상단 바(relPath·더티 ●·소스/미리보기 토글·
// 닫기), 본문은 소스 모드=모노스페이스 textarea, 미리보기 모드=marked+DOMPurify
// 렌더(raw HTML sanitize, 링크 target 차단·클릭 무시, 로컬 이미지 미해석).
// Cmd/Ctrl+S 저장, Cmd/Ctrl+P 팔레트 재오픈, Esc 닫기(더티면 확인 다이얼로그).
// 저장 충돌(CONFLICT)은 다시 불러오기/덮어쓰기/취소 다이얼로그로 해결한다.
//
// self-gate 관례: 항상 마운트, 편집기 없으면 null 렌더. 키 이벤트는 오버레이에서
// stopPropagation해 터미널/전역 단축키로 새지 않게 한다.
import { useMemo } from "react";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { useMarkdownStore, isEditorDirty } from "./markdownStore";

// 링크는 새 창을 못 열게 target 속성을 제거한다(클릭 자체는 아래 onClick에서 무시).
DOMPurify.addHook("afterSanitizeAttributes", (node) => {
  if (node.tagName === "A") node.removeAttribute("target");
});

/** 마크다운 → 안전한 HTML. marked는 동기 파싱(async 확장 없음), DOMPurify로 sanitize. */
function renderMarkdown(src: string): string {
  const raw = marked.parse(src, { async: false, gfm: true, breaks: false }) as string;
  return DOMPurify.sanitize(raw, { FORBID_ATTR: ["target"] });
}

export function MarkdownEditorOverlay() {
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

  const content = editor?.content ?? "";
  const previewHtml = useMemo(
    () => (editor?.mode === "preview" ? renderMarkdown(content) : ""),
    [editor?.mode, content],
  );

  if (!editor) return null;
  const dirty = isEditorDirty(editor);

  // 더티 가드 다이얼로그에서 "저장 후 닫기".
  const saveThenClose = async () => {
    const res = await save();
    if (res.ok) closeEditor();
    // 충돌이면 save가 conflict 플래그를 세팅 → 아래 충돌 다이얼로그가 뜬다.
    // (discardConfirm은 cancelDiscard로 접어 충돌 다이얼로그만 남긴다.)
    else if (!res.ok && res.conflict) cancelDiscard();
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
      requestClose();
    }
  };

  return (
    <div className="md-overlay md-editor-overlay" onKeyDown={onKeyDown}>
      <div className="md-editor" role="dialog" aria-label={`마크다운 편집기: ${editor.relPath}`}>
        <div className="md-editor-bar">
          <span className="md-editor-path" title={editor.relPath}>
            {editor.relPath}
            {dirty && <span className="md-editor-dirty" aria-label="저장되지 않은 변경">●</span>}
          </span>
          <div className="md-editor-bar-actions">
            <button
              type="button"
              className={editor.mode === "source" ? "md-tab md-tab-active" : "md-tab"}
              aria-pressed={editor.mode === "source"}
              onClick={() => setMode("source")}
            >
              소스
            </button>
            <button
              type="button"
              className={editor.mode === "preview" ? "md-tab md-tab-active" : "md-tab"}
              aria-pressed={editor.mode === "preview"}
              onClick={() => setMode("preview")}
            >
              미리보기
            </button>
            <button
              type="button"
              className="md-editor-close"
              aria-label="편집기 닫기"
              onClick={requestClose}
            >
              ×
            </button>
          </div>
        </div>

        <div className="md-editor-body">
          {editor.loading ? (
            <div className="md-editor-status">불러오는 중…</div>
          ) : editor.loadError ? (
            <div className="md-editor-status md-editor-error">
              파일을 열 수 없습니다: {editor.loadError}
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
              // 링크 클릭은 무시(v1: 외부 링크 미탐색, 로컬 이미지 미해석).
              onClick={(e) => {
                if ((e.target as HTMLElement).closest("a")) e.preventDefault();
              }}
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
            if (e.button === 0 && e.target === e.currentTarget) cancelDiscard();
          }}
        >
          <div className="pixel-panel md-confirm">
            <h2 className="pixel-title">저장되지 않은 변경</h2>
            <p>
              <strong>{editor.relPath}</strong>에 저장하지 않은 변경이 있습니다.
            </p>
            <div className="dialog-actions">
              <button className="pixel-btn primary" onClick={() => void saveThenClose()}>
                저장 후 닫기
              </button>
              <button className="pixel-btn" onClick={closeEditor}>
                버리고 닫기
              </button>
              <button className="pixel-btn" onClick={cancelDiscard}>
                취소
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
            <h2 className="pixel-title">저장 충돌</h2>
            <p>다른 곳에서 파일이 변경되었습니다. 어떻게 할까요?</p>
            <div className="dialog-actions">
              <button className="pixel-btn" onClick={() => void reloadFromDisk()}>
                다시 불러오기(내 변경 버림)
              </button>
              <button className="pixel-btn primary" onClick={() => void overwrite()}>
                내 내용으로 덮어쓰기
              </button>
              <button className="pixel-btn" onClick={cancelConflict}>
                취소
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
