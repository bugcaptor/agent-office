// src/renderer/layout/BottomBar.tsx
//
// Bottom bar: the primary "+ New Agent" entry point on the
// left (opens `ProfileDialog` in create mode), a running/pending status
// summary in the center, a settings (⚙) button that opens `SettingsDialog`
// (Claude Code 연동 opt-in 2종), and the mute toggle on the right (flips
// `store.muted`; the actual badge resync on toggle lives in
// `ipc/sessionBridge.ts`'s `installSessionBridge`, not here).
import { useAppStore } from "../store/appStore";
import { usePendingCount, useRunningCount } from "../store/selectors";
import { THEMES, nextThemeId } from "../theme/themes";

export function BottomBar() {
  const openModal = useAppStore((s) => s.openModal);
  const muted = useAppStore((s) => s.muted);
  const toggleMuted = useAppStore((s) => s.toggleMuted);
  const theme = useAppStore((s) => s.theme);
  const setTheme = useAppStore((s) => s.setTheme);
  const runningCount = useRunningCount();
  const pendingCount = usePendingCount();

  return (
    <footer className="bottom-bar pixel-panel">
      <button
        type="button"
        className="pixel-btn primary new-agent-btn"
        onClick={() => openModal({ kind: "profile-create" })}
      >
        ＋ New Agent
      </button>
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
