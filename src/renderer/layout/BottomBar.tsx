// src/renderer/layout/BottomBar.tsx
//
// Bottom bar: the primary "+ New Agent" entry point on the
// left (opens `ProfileDialog` in create mode), next to it a "출근"
// (clock-in) button showing the clocked-out count that opens a `ContextMenu`
// listing clocked-out agents by name (selecting one calls `clockInAgent`),
// and a bulk clock button that toggles by state: when anyone is on duty it is
// "전체 퇴근" (opens a `confirm-clock-out-all` modal → `clockOutAll`); when
// everyone is clocked out it becomes "전체 출근" (calls `clockInAll` directly,
// no confirm — clock-in is non-destructive). Then a running/pending status summary in
// the center, a settings (⚙) button that
// opens `SettingsDialog` (Claude Code 연동 opt-in 2종), and the mute toggle
// on the right (flips `store.muted`; the actual badge resync on toggle lives
// in `ipc/sessionBridge.ts`'s `installSessionBridge`, not here).
import { useState } from "react";
import { useAppStore } from "../store/appStore";
import {
  useAgentList,
  useClockedOutAgents,
  useClockedOutCount,
  usePendingCount,
  useRunningCount,
} from "../store/selectors";
import { THEMES, nextThemeId } from "../theme/themes";
import { ContextMenu } from "../ui/ContextMenu";
import { clockInAgent, clockInAll } from "../agent/clockOut";

export function BottomBar() {
  const openModal = useAppStore((s) => s.openModal);
  const muted = useAppStore((s) => s.muted);
  const toggleMuted = useAppStore((s) => s.toggleMuted);
  const theme = useAppStore((s) => s.theme);
  const setTheme = useAppStore((s) => s.setTheme);
  const runningCount = useRunningCount();
  const pendingCount = usePendingCount();
  const onDutyCount = useAgentList().length;
  const clockedOutAgents = useClockedOutAgents();
  const clockedOutCount = useClockedOutCount();
  const [summonMenu, setSummonMenu] = useState<{ x: number; y: number } | null>(null);

  return (
    <footer className="bottom-bar pixel-panel">
      <button
        type="button"
        className="pixel-btn primary new-agent-btn"
        onClick={() => openModal({ kind: "profile-create" })}
      >
        ＋ New Agent
      </button>
      <button
        type="button"
        className="pixel-btn summon-btn"
        disabled={clockedOutCount === 0}
        onClick={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          setSummonMenu({ x: rect.left, y: rect.top });
        }}
      >
        🏠 출근 ({clockedOutCount})
      </button>
      {onDutyCount === 0 ? (
        <button
          type="button"
          className="pixel-btn clock-in-all-btn"
          disabled={clockedOutCount === 0}
          onClick={() => clockInAll()}
        >
          전체 출근
        </button>
      ) : (
        <button
          type="button"
          className="pixel-btn clock-out-all-btn"
          onClick={() => openModal({ kind: "confirm-clock-out-all" })}
        >
          전체 퇴근
        </button>
      )}
      {summonMenu && (
        <ContextMenu
          x={summonMenu.x}
          y={summonMenu.y}
          onClose={() => setSummonMenu(null)}
          items={clockedOutAgents.map((agent) => ({
            label: agent.name,
            onSelect: () => clockInAgent(agent.id),
          }))}
        />
      )}
      <span className="bottom-bar-status">
        {runningCount} running · {pendingCount} needs input
      </span>
      <button
        type="button"
        className="pixel-btn settings-btn"
        aria-label="설정"
        title="설정 (Claude Code 연동 opt-in)"
        onClick={() => openModal({ kind: "settings" })}
      >
        ⚙
      </button>
      <button
        type="button"
        className="pixel-btn theme-btn"
        aria-label="테마 전환"
        title="클릭할 때마다 다음 테마로 전환"
        onClick={() => setTheme(nextThemeId(theme))}
      >
        테마: {THEMES[theme].label}
      </button>
      <button
        type="button"
        className="pixel-btn mute-btn"
        aria-pressed={muted}
        aria-label={muted ? "Unmute notifications" : "Mute notifications"}
        onClick={toggleMuted}
      >
        {muted ? "🔇" : "🔔"}
      </button>
    </footer>
  );
}
