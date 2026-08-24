// src/renderer/memo/PostItWidget.tsx
//
// 에이전트별 포스트잇 메모(#79) 위젯. 터미널 오버레이 패널 **우상단**에
// absolute로 얹히는 작은 쪽지 한 장이다. 본문은 plain text `<textarea>` —
// 마크다운이 아니고 렌더링도 하지 않는다(사용자가 쓰는 대로 보이는 게 요점).
//
// 왜 조건부 렌더인가: 터미널의 keep-alive 불변식은 `AgentTabStrip`/
// `TerminalHost`(그 아래 xterm 인스턴스)에 대한 것이고, 이 위젯은 그 트리와
// 무관한 형제다. 닫으면 언마운트해도 잃는 게 없다(본문은 디스크에 있다) —
// 대신 언마운트 직전에 반드시 flush 한다.
//
// 포커스/키 격리: 터미널 오버레이는 window에 Cmd/Ctrl+1..9·Cmd+W 등을 걸어
// 두므로, 위젯 안에서 누른 키가 거기까지 올라가면 메모를 타이핑하다 탭이
// 바뀐다. React는 루트 컨테이너에 리스너를 붙이므로 여기서
// stopPropagation 하면 window 리스너까지 도달하지 않는다.
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";

import { useAppStore } from "../store/appStore";
import { useMemoStore } from "./memoStore";
import { formatMemoWhen } from "./memoFormat";
import "./memo.css";

export function PostItWidget() {
  const { t } = useTranslation("journal");
  const visible = useMemoStore((s) => s.visible);
  const activeId = useAppStore((s) => s.activeTerminalAgentId);
  const agentName = useAppStore((s) => (activeId ? s.agents[activeId]?.name : undefined));

  const agentId = useMemoStore((s) => s.agentId);
  const sheet = useMemoStore((s) => s.sheet);
  const draft = useMemoStore((s) => s.draft);
  const loading = useMemoStore((s) => s.loading);
  const dirty = useMemoStore((s) => s.dirty);
  const archiving = useMemoStore((s) => s.archiving);
  const notice = useMemoStore((s) => s.notice);
  const setVisible = useMemoStore((s) => s.setVisible);
  const focusAgent = useMemoStore((s) => s.focusAgent);
  const edit = useMemoStore((s) => s.edit);
  const flush = useMemoStore((s) => s.flush);
  const archiveNow = useMemoStore((s) => s.archiveNow);
  const copyAll = useMemoStore((s) => s.copyAll);
  const openArchive = useMemoStore((s) => s.openArchive);

  // 활성 탭이 바뀌면 그 캐릭터의 장으로 갈아탄다(이전 캐릭터 편집은 store가 flush).
  const open = visible && activeId !== null;
  useEffect(() => {
    if (!open || activeId === null) return;
    void focusAgent(activeId);
  }, [open, activeId, focusAgent]);

  // 언마운트(닫힘/오버레이 종료) 직전 마지막 타이핑 확정. 저장 버튼이 없으므로
  // 이 경로가 빠지면 마지막 1초가 유실된다.
  const flushRef = useRef(flush);
  flushRef.current = flush;
  useEffect(() => {
    if (!open) return;
    return () => {
      void flushRef.current();
    };
  }, [open]);

  if (!open) return null;

  // 로드 전에는 편집을 막는다 — 빈 draft로 덮어써 저장하는 사고를 방지.
  const ready = sheet !== null && agentId === activeId;
  const canArchive = ready && !archiving && draft.trim() !== "";

  return (
    <div
      className="pixel-panel postit"
      role="group"
      aria-label={t("memo.widgetAria", { name: agentName ?? t("memo.fallbackName") })}
      // 위젯 내부 키 입력이 터미널 오버레이의 전역 단축키로 새지 않게 한다.
      onKeyDown={(e) => e.stopPropagation()}
      onKeyUp={(e) => e.stopPropagation()}
      // 패널 위 클릭이 오버레이 backdrop 닫기로 오해되지 않게(형제 경로 차단).
      onMouseDown={(e) => e.stopPropagation()}
    >
      <div className="postit-header">
        <span className="postit-title" title={agentName ?? undefined}>
          🗒 {agentName ?? t("memo.headerFallback")}
        </span>
        <button
          type="button"
          className="postit-x"
          title={t("memo.close")}
          aria-label={t("memo.close")}
          onClick={() => setVisible(false)}
        >
          ×
        </button>
      </div>

      <textarea
        className="postit-text"
        aria-label={t("memo.bodyAria")}
        placeholder={loading ? t("memo.loading") : t("memo.placeholder")}
        spellCheck={false}
        disabled={!ready}
        value={draft}
        onChange={(e) => edit(e.target.value)}
        onBlur={() => void flush()}
      />

      <div className="postit-actions">
        <button
          type="button"
          className="pixel-btn postit-btn"
          title={t("memo.copyAllTitle")}
          disabled={!ready || draft === ""}
          onClick={() => void copyAll()}
        >
          {t("memo.copyAll")}
        </button>
        <button
          type="button"
          className="pixel-btn postit-btn"
          title={canArchive ? t("memo.flipTitle") : t("memo.flipEmptyTitle")}
          disabled={!canArchive}
          onClick={() => void archiveNow()}
        >
          {archiving ? t("memo.flipping") : t("memo.flip")}
        </button>
        <button
          type="button"
          className="pixel-btn postit-btn"
          title={t("memo.archiveTitle")}
          disabled={activeId === null}
          onClick={() => {
            if (activeId) void openArchive(activeId, agentName ?? t("memo.fallbackName"));
          }}
        >
          {t("memo.archive")}
        </button>
      </div>

      {notice && <div className="postit-notice">{notice}</div>}
      {ready && sheet && (
        // 저장 시각을 렌더러가 추측해 표시하지 않는다(진실은 디스크의 헤더다) —
        // 대신 "지금 미저장 편집이 있는가"와 이 장이 시작된 시각만 보여준다.
        <div className="postit-stamp">
          {dirty ? t("memo.saving") : t("memo.saved")} ·{" "}
          {t("memo.startedAt", { when: formatMemoWhen(sheet.created) })}
        </div>
      )}
    </div>
  );
}
