// src/renderer/timeline/SessionTimePanel.tsx
//
// Always-visible, collapsible top-right panel showing per-agent turn time.
// One row per agent: name · status icon · live elapsed of the open
// turn (1s tick — renderer clock, DISPLAY ONLY) · cumulative "진행 … · 총 … ·
// N턴". All cumulative figures come from backend-timestamp settlement in the
// reducer; only the open-turn live elapsed uses the wall clock, and only for
// display.
import { useEffect, useState } from "react";
import { useSessionTimeRows, useTodayWorkedMs, type SessionTimeRow } from "../store/selectors";
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

export function SessionTimePanel() {
  const rows = useSessionTimeRows();
  const todayWorkedMs = useTodayWorkedMs();
  const [collapsed, setCollapsed] = useState(false);
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
      <div className="stp-today">오늘 {formatDuration(todayWorkedMs)}</div>
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
