// src/renderer/notification/NotificationTicker.tsx
//
// Right-column notification ticker. Renders at most `MAX_VISIBLE` cards,
// newest agent-deduped notification first, plus a "+N more" summary card
// for the rest.
//
// Deviation from the original design sketch: it called
// `openTerminal(agentId)` (store action) and then a backend
// `clearNotifications(agentId)` call as two separate steps.
// `officeBus.emitAgentClicked` (`ipc/sessionBridge.ts`) already does exactly
// that pair — it's the same "click an agent -> focus its terminal + clear
// its backend notifications" behavior the office canvas uses for clicking a
// character sprite. Calling it here instead of re-deriving both steps keeps
// the two entry points (ticker card, office sprite) from drifting apart.
import { useMemo } from "react";
import { useShallow } from "zustand/react/shallow";
import { useAppStore } from "../store/appStore";
import { officeBus } from "../ipc/sessionBridge";
import { dedupeLatestPerAgent } from "./dedupe";
import type { Notification } from "../store/types";

const MAX_VISIBLE = 5;
const rtf = new Intl.RelativeTimeFormat("ko", { numeric: "auto" });

/** "방금" under 45s, otherwise the coarsest unit (minute, then hour) that keeps the number small. */
function relTime(ts: number): string {
  const diffSec = Math.round((ts - Date.now()) / 1000);
  if (Math.abs(diffSec) < 45) return "방금";
  const diffMin = Math.round(diffSec / 60);
  if (Math.abs(diffMin) < 60) return rtf.format(diffMin, "minute");
  return rtf.format(Math.round(diffMin / 60), "hour");
}

const TYPE_ICON: Record<Notification["type"], string> = {
  question: "❓",
  done: "✅",
  info: "ℹ️",
};

export function NotificationTicker() {
  const notifications = useAppStore(useShallow((s) => s.notifications));
  const agents = useAppStore((s) => s.agents);

  const deduped = useMemo(() => dedupeLatestPerAgent(notifications), [notifications]);
  const visible = deduped.slice(0, MAX_VISIBLE);
  const overflow = deduped.length - visible.length;

  const onClick = (agentId: string) => {
    officeBus.emitAgentClicked(agentId);
  };

  return (
    <div className="notification-ticker" aria-live="polite">
      {visible.map((n) => {
        const agent = agents[n.agentId];
        return (
          <button
            key={n.id}
            type="button"
            className={`ticker-card pixel-panel type-${n.type}`}
            onClick={() => onClick(n.agentId)}
          >
            <span className="ticker-icon">{TYPE_ICON[n.type]}</span>
            <span className="ticker-body">
              <span className="ticker-name">{agent?.name ?? n.agentId}</span>
              <span className="ticker-msg">{n.excerpt}</span>
              <span className="ticker-time">{relTime(n.createdAt)}</span>
            </span>
          </button>
        );
      })}
      {overflow > 0 && (
        <div className="ticker-overflow pixel-panel">+{overflow} more</div>
      )}
    </div>
  );
}
