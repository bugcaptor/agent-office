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
import { useTranslation } from "react-i18next";
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
  const { t } = useTranslation("activity");
  const { rows, loading, error, retry } = useAgentStats(true);
  if (error) {
    return (
      <div className="stp-stats-msg">
        {t("timeline.statsError")}{" "}
        <button type="button" className="stp-stats-retry" onClick={retry}>
          {t("timeline.retry")}
        </button>
      </div>
    );
  }
  if (loading && rows.length === 0) {
    return <div className="stp-stats-msg">{t("timeline.loading")}</div>;
  }
  if (rows.length === 0) {
    return <div className="stp-stats-msg">{t("timeline.noRecords")}</div>;
  }
  return (
    <ul className="stp-stats">
      {rows.map((r) => (
        <li key={r.agentId} className={`stp-stat-row${r.departed ? " departed" : ""}`}>
          <span className="stp-stat-name">
            {r.departed ? t("timeline.departed", { short: r.label }) : r.label}
          </span>
          <span className="stp-stat-vals">
            {t("timeline.statVals", {
              today: formatDuration(r.todayWorkedMs),
              total: formatDuration(r.totalWorkedMs),
            })}
          </span>
        </li>
      ))}
    </ul>
  );
}

export function SessionTimePanel() {
  const { t } = useTranslation("activity");
  const rows = useSessionTimeRows();
  const todayWorkedMs = useTodayWorkedMs();
  const [collapsed, setCollapsed] = useState(false);
  const [statsOpen, setStatsOpen] = useState(false);
  const anyOpen = rows.some((r) => r.phase !== "idle");
  const now = useOneSecondTick(anyOpen && !collapsed);

  return (
    <div className="session-time-panel pixel-panel">
      <div className="stp-head">
        <span className="stp-title">{t("timeline.title")}</span>
        <button
          type="button"
          className="stp-toggle"
          aria-label={collapsed ? t("timeline.expand") : t("timeline.collapse")}
          onClick={() => setCollapsed((c) => !c)}
        >
          {collapsed ? "▸" : "▾"}
        </button>
      </div>
      <div className="stp-today">
        <span>{t("timeline.todayTotal", { total: formatDuration(todayWorkedMs) })}</span>
        <button
          type="button"
          className="stp-stats-toggle"
          aria-expanded={statsOpen}
          aria-label={statsOpen ? t("timeline.statsCollapse") : t("timeline.statsExpand")}
          onClick={() => setStatsOpen((s) => !s)}
        >
          {t("timeline.statsToggle")} {statsOpen ? "▾" : "▸"}
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
                  {t("timeline.rowCum", {
                    worked: formatDuration(r.workedMs),
                    total: formatDuration(r.totalMs),
                    turns: t("timeline.turns", { count: r.turns }),
                  })}
                </span>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
