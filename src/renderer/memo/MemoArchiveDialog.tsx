// src/renderer/memo/MemoArchiveDialog.tsx
//
// 포스트잇 메모(#79) 아카이브 열람 다이얼로그. DiaryDialog와 같은 관례:
// 항상 마운트되고 스토어 타깃이 없으면 null 렌더(self-gate), backdrop 클릭으로
// 닫히며, Esc는 **캡처 단계에서 stopPropagation** 해 터미널/전역으로 새지
// 않게 한다.
//
// 좌: 넘긴 장 목록(최신순, 생성·수정·아카이브 시각). 우: 고른 장의 본문 —
// 읽기 전용이고 복사만 된다(과거 장을 되살리는 기능은 범위 밖).
import { useEffect } from "react";
import { useMemoStore } from "./memoStore";
import { formatMemoWhen } from "./memoFormat";
import "./memo.css";

/** 본문 미리보기 한 줄 — 목록에서 어떤 장인지 알아보게 돕는다. */
function previewOf(content: string): string {
  const line = content.split("\n").find((l) => l.trim() !== "");
  if (line === undefined) return "";
  return line.length > 40 ? `${line.slice(0, 40)}…` : line;
}

export function MemoArchiveDialog() {
  const target = useMemoStore((s) => s.archive);
  const items = useMemoStore((s) => s.archiveItems);
  const loading = useMemoStore((s) => s.archiveLoading);
  const selected = useMemoStore((s) => s.archiveSelected);
  const notice = useMemoStore((s) => s.archiveNotice);
  const closeArchive = useMemoStore((s) => s.closeArchive);
  const selectSheet = useMemoStore((s) => s.selectSheet);
  const copySelected = useMemoStore((s) => s.copySelected);

  // Esc 닫기(전역/터미널로 새지 않게 캡처 단계에서 멈춘다).
  const open = target !== null;
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        closeArchive();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open, closeArchive]);

  if (!target) return null;

  return (
    <div
      className="memo-archive-overlay"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeArchive();
      }}
    >
      <div
        className="pixel-panel memo-archive-dialog"
        role="dialog"
        aria-label={`${target.agentName}의 메모 아카이브`}
      >
        <div className="memo-archive-header">
          <h2 className="memo-archive-title">🗒 {target.agentName}의 메모 아카이브</h2>
          <div className="memo-archive-actions">
            <button
              type="button"
              className="pixel-btn"
              disabled={!selected}
              title={selected ? "이 장의 본문을 복사" : "먼저 장을 고르세요"}
              onClick={() => void copySelected()}
            >
              복사
            </button>
            <button type="button" className="pixel-btn" onClick={closeArchive}>
              닫기
            </button>
          </div>
        </div>

        {notice && <div className="memo-archive-notice">{notice}</div>}

        {loading ? (
          <div className="memo-archive-empty">불러오는 중…</div>
        ) : items.length === 0 ? (
          <div className="memo-archive-empty">
            아직 넘긴 장이 없습니다. 포스트잇에서 ‘한 장 넘기기’를 하면 여기 쌓입니다.
          </div>
        ) : (
          <div className="memo-archive-body">
            <ul className="memo-archive-list">
              {items.map((item) => (
                <li key={item.sheetId}>
                  <button
                    type="button"
                    className={
                      selected?.sheetId === item.sheetId
                        ? "memo-archive-item memo-archive-item-active"
                        : "memo-archive-item"
                    }
                    onClick={() => void selectSheet(item.sheetId)}
                  >
                    <span className="memo-archive-item-when">
                      {formatMemoWhen(item.archived)} 넘김
                    </span>
                    <span className="memo-archive-item-sub">
                      {formatMemoWhen(item.created)} 시작 · {formatMemoWhen(item.updated)} 수정
                    </span>
                  </button>
                </li>
              ))}
            </ul>
            <div className="memo-archive-view">
              {selected ? (
                <>
                  <div className="memo-archive-view-head">
                    {previewOf(selected.content) || "(빈 장)"}
                  </div>
                  <pre className="memo-archive-view-body">{selected.content}</pre>
                </>
              ) : (
                <div className="memo-archive-empty">왼쪽에서 장을 고르세요.</div>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
