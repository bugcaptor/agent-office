// src/renderer/layout/TopBar.tsx
//
// Top bar: app title on the left, agent-count badge +
// running/pending counts on the right. The whole bar doubles as a "reopen
// terminal" button — a large, stationary click target, because clicking a
// wandering character sprite on the canvas is fiddly. A click routes through
// `officeBus.emitAgentClicked` (the same path as ticker cards and canvas
// sprites: session 재생성 + openTerminal + 백엔드 알림 클리어), targeting the
// most recently used terminal tab, falling back to the first created agent
// when nothing was ever opened.
import { useAppStore } from "../store/appStore";
import { useAgentCount, usePendingCount, useRunningCount } from "../store/selectors";
import { officeBus } from "../ipc/sessionBridge";

export function TopBar() {
  const agentCount = useAgentCount();
  const runningCount = useRunningCount();
  const pendingCount = usePendingCount();
  const targetAgentId = useAppStore((s) => s.recentAgentIds[0] ?? s.agentOrder[0] ?? null);

  return (
    <button
      type="button"
      className="top-bar pixel-panel"
      disabled={targetAgentId === null}
      title="터미널 열기"
      onClick={() => {
        if (targetAgentId) officeBus.emitAgentClicked(targetAgentId);
      }}
    >
      <span className="top-bar-title">Agent Office</span>
      <span className="top-bar-stats">
        <span className="top-bar-badge">{agentCount} agents</span>
        <span className="top-bar-counts">
          {runningCount} running · {pendingCount} pending
        </span>
      </span>
    </button>
  );
}
