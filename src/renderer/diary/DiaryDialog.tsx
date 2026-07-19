// src/renderer/diary/DiaryDialog.tsx
//
// 캐릭터 일기(#56) 열람/생성 오버레이. self-gate 관례(다이얼로그와 동일):
// 항상 마운트되며 오버레이 타깃이 없으면 null 렌더. 날짜 역순으로 일기를
// 보여주고, "일기 쓰기" 버튼으로 지금까지의 작업 로그를 한 편으로 남긴다
// (수동 트리거 — 비용·기대 UX상 사용자 요청 기반). 읽기 전용 뷰.
import { useEffect } from "react";
import { useDiaryStore } from "./diaryStore";
import "./diary.css";

/** epoch ms → 사람이 읽는 로컬 날짜·시각. */
function formatWhen(at: number): string {
  const d = new Date(at);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function DiaryDialog() {
  const overlay = useDiaryStore((s) => s.overlay);
  const entries = useDiaryStore((s) => s.entries);
  const loading = useDiaryStore((s) => s.loading);
  const generating = useDiaryStore((s) => s.generating);
  const backfilling = useDiaryStore((s) => s.backfilling);
  const notice = useDiaryStore((s) => s.notice);
  const closeDiary = useDiaryStore((s) => s.closeDiary);
  const writeNow = useDiaryStore((s) => s.writeNow);

  // Esc 닫기(전역/터미널로 새지 않게 캡처 단계에서 멈춘다).
  const open = overlay !== null;
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        closeDiary();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open, closeDiary]);

  if (!overlay) return null;

  // 날짜 역순(최신 먼저)으로 표시.
  const ordered = [...entries].reverse();

  return (
    <div
      className="diary-overlay"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeDiary();
      }}
    >
      <div className="pixel-panel diary-dialog" role="dialog" aria-label={`${overlay.agentName}의 일기`}>
        <div className="diary-header">
          <h2 className="diary-title">📔 {overlay.agentName}의 일기</h2>
          <div className="diary-actions">
            <button
              type="button"
              className="pixel-btn primary"
              disabled={generating}
              onClick={() => void writeNow(overlay.agentId)}
            >
              {generating ? "쓰는 중…" : "일기 쓰기"}
            </button>
            <button type="button" className="pixel-btn" onClick={closeDiary}>
              닫기
            </button>
          </div>
        </div>

        {backfilling && <div className="diary-notice">밀린 일기 쓰는 중…</div>}
        {notice && <div className="diary-notice">{notice}</div>}

        {loading ? (
          <div className="diary-empty">불러오는 중…</div>
        ) : ordered.length === 0 ? (
          <div className="diary-empty">아직 일기가 없습니다. ‘일기 쓰기’로 첫 일기를 남겨 보세요.</div>
        ) : (
          <ul className="diary-list">
            {ordered.map((entry) => (
              <li key={`${entry.at}-${entry.sessionId}`} className="diary-entry">
                <div className="diary-entry-date">{formatWhen(entry.at)}</div>
                <div className="diary-entry-body">{entry.body}</div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
