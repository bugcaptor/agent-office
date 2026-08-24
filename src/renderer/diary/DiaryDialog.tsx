// src/renderer/diary/DiaryDialog.tsx
//
// 캐릭터 일기(#56) 열람/생성 오버레이. self-gate 관례(다이얼로그와 동일):
// 항상 마운트되며 오버레이 타깃이 없으면 null 렌더. 날짜 역순으로 일기를
// 보여주고, "일기 쓰기" 버튼으로 지금까지의 작업 로그를 한 편으로 남긴다
// (수동 트리거 — 비용·기대 UX상 사용자 요청 기반). "내보내기"(#65)는 일기
// 전체를 Markdown/JSON 파일로 저장한다. 일기 자체는 읽기 전용 뷰.
//
// i18n: 번역 대상은 껍데기(제목·버튼·빈 상태·안내)뿐이다. `entry.body`는 생성
// 당시 언어로 쓰인 사료라 그대로 보여준다(재번역하지 않는다).
import { useTranslation } from "react-i18next";

import { useDiaryStore } from "./diaryStore";
import { useEscapeToClose } from "../shared/useEscapeToClose";
import { formatWhen } from "./diaryExport";
import "./diary.css";

export function DiaryDialog() {
  const { t } = useTranslation("journal");
  const overlay = useDiaryStore((s) => s.overlay);
  const entries = useDiaryStore((s) => s.entries);
  const loading = useDiaryStore((s) => s.loading);
  const generating = useDiaryStore((s) => s.generating);
  const backfilling = useDiaryStore((s) => s.backfilling);
  const notice = useDiaryStore((s) => s.notice);
  const exporting = useDiaryStore((s) => s.exporting);
  const closeDiary = useDiaryStore((s) => s.closeDiary);
  const writeNow = useDiaryStore((s) => s.writeNow);
  const exportNow = useDiaryStore((s) => s.exportNow);

  // Esc 닫기(전역/터미널로 새지 않게 캡처 단계에서 멈춘다).
  useEscapeToClose(overlay !== null, closeDiary);

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
      <div
        className="pixel-panel diary-dialog"
        role="dialog"
        aria-label={t("diary.dialogAria", { name: overlay.agentName })}
      >
        <div className="diary-header">
          <h2 className="diary-title">{t("diary.title", { name: overlay.agentName })}</h2>
          <div className="diary-actions">
            <button
              type="button"
              className="pixel-btn primary"
              disabled={generating}
              onClick={() => void writeNow(overlay.agentId)}
            >
              {generating ? t("diary.writing") : t("diary.write")}
            </button>
            <button
              type="button"
              className="pixel-btn"
              disabled={exporting || entries.length === 0}
              title={entries.length === 0 ? t("diary.exportNoneTitle") : t("diary.exportTitle")}
              onClick={() => void exportNow(overlay.agentId)}
            >
              {exporting ? t("diary.exporting") : t("diary.export")}
            </button>
            <button type="button" className="pixel-btn" onClick={closeDiary}>
              {t("diary.close")}
            </button>
          </div>
        </div>

        {backfilling && <div className="diary-notice">{t("diary.backfilling")}</div>}
        {notice && <div className="diary-notice">{notice}</div>}

        {loading ? (
          <div className="diary-empty">{t("diary.loading")}</div>
        ) : ordered.length === 0 ? (
          <div className="diary-empty">{t("diary.empty")}</div>
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
