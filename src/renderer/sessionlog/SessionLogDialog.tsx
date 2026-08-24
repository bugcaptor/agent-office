// src/renderer/sessionlog/SessionLogDialog.tsx
//
// 세션 로그 보기 오버레이(docs/session-log-design.md §7). DiaryDialog와 같은
// self-gate 관례 — 항상 마운트되고 오버레이 타깃이 없으면 null 렌더.
//
// 목록은 10개씩 페이징하고, 한 개를 고르면 그때 동작 영역(편집기로 열기 /
// 학습자료 만들기)이 열린다. 고르기 전에는 동작을 보여주지 않는다 — 무엇에
// 적용될지 모르는 버튼을 띄우지 않기 위해서다.
import { useTranslation } from "react-i18next";
import { useSessionLogStore, PAGE_SIZE } from "./sessionLogStore";
import { useEscapeToClose } from "../shared/useEscapeToClose";
import { renderText } from "../shared/textKey";
import { formatBytes, formatDuration, formatWhen, shortenPath } from "./format";
import "./sessionLog.css";

export function SessionLogDialog() {
  const { t } = useTranslation("activity");
  const overlay = useSessionLogStore((s) => s.overlay);
  const items = useSessionLogStore((s) => s.items);
  const total = useSessionLogStore((s) => s.total);
  const page = useSessionLogStore((s) => s.page);
  const selected = useSessionLogStore((s) => s.selected);
  const loading = useSessionLogStore((s) => s.loading);
  const generating = useSessionLogStore((s) => s.generating);
  const notice = useSessionLogStore((s) => s.notice);
  const close = useSessionLogStore((s) => s.close);
  const setPage = useSessionLogStore((s) => s.setPage);
  const select = useSessionLogStore((s) => s.select);
  const openInEditor = useSessionLogStore((s) => s.openInEditor);
  const makeStudyMaterial = useSessionLogStore((s) => s.makeStudyMaterial);

  // Esc 닫기(전역/터미널로 새지 않게 캡처 단계에서 멈춘다).
  useEscapeToClose(overlay !== null, close);

  if (!overlay) return null;

  const pageCount = Math.max(1, Math.ceil(total / PAGE_SIZE));

  return (
    <div
      className="slog-overlay"
      onMouseDown={(e) => {
        // 생성 중에는 배경 클릭으로 닫지 않는다 — 진행 중인 작업을 잃어버린
        // 것처럼 보이지 않게.
        if (e.button === 0 && e.target === e.currentTarget && !generating) close();
      }}
    >
      <div
        className="pixel-panel slog-dialog"
        role="dialog"
        aria-label={t("sessionLog.dialogAria", { name: overlay.agentName })}
      >
        <div className="slog-header">
          <h2 className="slog-title">{t("sessionLog.title", { name: overlay.agentName })}</h2>
          <button type="button" className="pixel-btn" onClick={close}>
            {t("sessionLog.close")}
          </button>
        </div>

        {notice && <div className="slog-notice">{notice}</div>}

        {loading ? (
          <div className="slog-empty">{t("sessionLog.loading")}</div>
        ) : items.length === 0 ? (
          <div className="slog-empty">
            {t("sessionLog.emptyTitle")}
            <br />
            {t("sessionLog.emptyHint")}
          </div>
        ) : (
          <ul className="slog-list">
            {items.map((item) => {
              const active = selected === item.path;
              const when = formatWhen(item.startedAt);
              const duration = formatDuration(item.startedAt, item.modifiedAt);
              return (
                <li key={item.path}>
                  <button
                    type="button"
                    className={`slog-row${active ? " active" : ""}`}
                    aria-pressed={active}
                    onClick={() => select(active ? null : item.path)}
                  >
                    <span className="slog-when">{when ?? t("sessionLog.whenUnknown")}</span>
                    <span className="slog-meta">
                      {duration && renderText(duration, t)}
                    </span>
                    <span className="slog-meta">{formatBytes(item.bytes)}</span>
                    <span className="slog-cwd" title={item.cwd}>
                      {shortenPath(item.cwd)}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}

        {total > PAGE_SIZE && (
          <div className="slog-pager">
            <button
              type="button"
              className="pixel-btn"
              disabled={page === 0 || loading}
              onClick={() => void setPage(page - 1)}
            >
              ‹
            </button>
            <span className="slog-pager-label">
              {page + 1} / {pageCount}
            </span>
            <button
              type="button"
              className="pixel-btn"
              disabled={page + 1 >= pageCount || loading}
              onClick={() => void setPage(page + 1)}
            >
              ›
            </button>
          </div>
        )}

        {selected && (
          <div className="slog-actions">
            <button type="button" className="pixel-btn" onClick={() => void openInEditor()}>
              {t("sessionLog.openInEditor")}
            </button>
            <button
              type="button"
              className="pixel-btn primary"
              disabled={generating}
              title={t("sessionLog.makeStudyTitle")}
              onClick={() => void makeStudyMaterial()}
            >
              {generating ? t("sessionLog.makingStudy") : t("sessionLog.makeStudy")}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
