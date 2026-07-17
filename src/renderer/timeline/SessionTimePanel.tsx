// src/renderer/timeline/SessionTimePanel.tsx
//
// Always-visible, collapsible top-right panel showing per-agent turn time.
// One row per agent: name · status icon · live elapsed of the open
// turn (1s tick — renderer clock, DISPLAY ONLY) · cumulative "진행 … · 총 … ·
// N턴". All cumulative figures come from backend-timestamp settlement in the
// reducer; only the open-turn live elapsed uses the wall clock, and only for
// display. A 통계 toggle reveals per-agent cumulative worked totals (오늘/총)
// aggregated from the disk log via useAgentStats.
import { useEffect, useState } from "react";
import { useSessionTimeRows, useTodayWorkedMs, type SessionTimeRow } from "../store/selectors";
import { useAgentStats } from "./useAgentStats";
import { formatDuration } from "./format";

const PHASE_ICON: Record<SessionTimeRow["phase"], string> = {
  working: "●",
  waiting: "⚠",
  idle: "○",
};

/** Live elapsed of the open turn, ticking once a second (display only). */
function useOneSecondTick(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now()); // avoid showing a stale value for up to 1s after (re-)activating
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [active]);
  return now;
}

/** 통계 뷰 본문 — statsOpen && !collapsed일 때만 마운트(그래야 훅 로드가 발화). */
function AgentStatsSection() {
  const { rows, loading, error, retry } = useAgentStats(true);
  if (error) {
    return (
      <div className="stp-stats-msg">
        통계를 불러오지 못했습니다{" "}
        <button type="button" className="stp-stats-retry" onClick={retry}>
          다시 시도
        </button>
      </div>
    );
  }
  if (loading && rows.length === 0) {
    return <div className="stp-stats-msg">불러오는 중…</div>;
  }
  if (rows.length === 0) {
    return <div className="stp-stats-msg">기록 없음</div>;
  }
  return (
    <ul className="stp-stats">
      {rows.map((r) => (
        <li key={r.agentId} className={`stp-stat-row${r.departed ? " departed" : ""}`}>
          <span className="stp-stat-name">{r.label}</span>
          <span className="stp-stat-vals">
            오늘 {formatDuration(r.todayWorkedMs)} · 총 {formatDuration(r.totalWorkedMs)}
          </span>
        </li>
      ))}
    </ul>
  );
}

export function SessionTimePanel() {
  const rows = useSessionTimeRows();
  const todayWorkedMs = useTodayWorkedMs();
  const [collapsed, setCollapsed] = useState(false);
  const [statsOpen, setStatsOpen] = useState(false);
  const anyOpen = rows.some((r) => r.phase !== "idle");
  const now = useOneSecondTick(anyOpen && !collapsed);

  return (
    <div className="session-time-panel pixel-panel">
      <div className="stp-head">
        <span className="stp-title">세션 시간</span>
        <button
          type="button"
          className="stp-toggle"
          aria-label={collapsed ? "펼치기" : "접기"}
          onClick={() => setCollapsed((c) => !c)}
        >
          {collapsed ? "▸" : "▾"}
        </button>
      </div>
      <div className="stp-today">
        <span>오늘 {formatDuration(todayWorkedMs)}</span>
        <button
          type="button"
          className="stp-stats-toggle"
          aria-expanded={statsOpen}
          aria-label={statsOpen ? "통계 접기" : "통계 펼치기"}
          onClick={() => setStatsOpen((s) => !s)}
        >
          통계 {statsOpen ? "▾" : "▸"}
        </button>
      </div>
      {!collapsed && statsOpen && <AgentStatsSection />}
      {!collapsed && (
        <ul className="stp-rows">
          {rows.map((r) => {
            const live =
              r.phase !== "idle" && r.turnStartedAt !== null
                ? Math.max(0, now - r.turnStartedAt)
                : 0;
            return (
              <li key={r.agentId} className={`stp-row phase-${r.phase}`}>
                <span className="stp-name">{r.name}</span>
                <span className="stp-icon" aria-hidden="true">
                  {PHASE_ICON[r.phase]}
                </span>
                <span className="stp-live">
                  {r.phase !== "idle" ? formatDuration(live) : "—"}
                </span>
                <span className="stp-cum">
                  진행 {formatDuration(r.workedMs)} · 총 {formatDuration(r.totalMs)} · {r.turns}턴
                </span>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
