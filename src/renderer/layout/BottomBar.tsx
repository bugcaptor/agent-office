// src/renderer/layout/BottomBar.tsx
//
// Bottom bar: the primary "+ New Agent" entry point on the
// left (opens `ProfileDialog` in create mode), next to it a "출근"
// (clock-in) button showing the clocked-out count that opens a `ContextMenu`
// listing clocked-out agents by name (selecting one calls `clockInAgent`),
// and a "전체 출퇴근" bulk button that opens a `ContextMenu` with two choices:
// "전체 출근" (calls `clockInAll` directly, no confirm — clock-in is
// non-destructive) and "전체 퇴근" (opens a `confirm-clock-out-all` modal →
// `clockOutAll`). Each item is disabled when its target set is empty, and the
// button itself is disabled only when there are no agents at all.
// Then a running/pending status summary in
// the center, an analytics (📊) button that opens `AnalyticsDialog`, a
// trophy (🏆) button right after it that opens `AwardsDialog`("이 달의
// 우수사원" — docs/employee-of-the-month-design.md §6), a settings (⚙)
// button that opens `SettingsDialog` (선택적 에이전트 연동 2종), an info (ℹ)
// button right after it that opens `AboutDialog` (앱 이름/버전/라이선스), and the
// mute toggle on the right (flips `store.muted`; the actual badge resync on
// toggle lives in `ipc/sessionBridge.ts`'s `installSessionBridge`, not
// here).
import { useState } from "react";
import { useAppStore } from "../store/appStore";
import {
  useAgentList,
  useClockedOutAgents,
  useClockedOutCount,
  usePendingCount,
  useRunningCount,
} from "../store/selectors";
import { THEMES, THEME_ORDER } from "../theme/themes";
import { SCENES, SCENE_ORDER } from "../office/scenes/scenes";
import { ContextMenu } from "../ui/ContextMenu";
import { clockInAgent, clockInAll } from "../agent/clockOut";
import { UsageWidget } from "../usage/UsageWidget";
import { TalkWidget } from "../talk/TalkWidget";

export function BottomBar() {
  const openModal = useAppStore((s) => s.openModal);
  const muted = useAppStore((s) => s.muted);
  const toggleMuted = useAppStore((s) => s.toggleMuted);
  const theme = useAppStore((s) => s.theme);
  const setTheme = useAppStore((s) => s.setTheme);
  const scene = useAppStore((s) => s.scene);
  const setScene = useAppStore((s) => s.setScene);
  const runningCount = useRunningCount();
  const pendingCount = usePendingCount();
  const onDutyCount = useAgentList().length;
  const clockedOutAgents = useClockedOutAgents();
  const clockedOutCount = useClockedOutCount();
  const [summonMenu, setSummonMenu] = useState<{ x: number; y: number } | null>(null);
  const [themeMenu, setThemeMenu] = useState<{ x: number; y: number } | null>(null);
  const [sceneMenu, setSceneMenu] = useState<{ x: number; y: number } | null>(null);
  const [clockAllMenu, setClockAllMenu] = useState<{ x: number; y: number } | null>(null);

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
      <button
        type="button"
        className="pixel-btn clock-all-btn"
        aria-haspopup="menu"
        disabled={onDutyCount === 0 && clockedOutCount === 0}
        onClick={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          setClockAllMenu({ x: rect.left, y: rect.top });
        }}
      >
        전체 출퇴근
      </button>
      {clockAllMenu && (
        <ContextMenu
          x={clockAllMenu.x}
          y={clockAllMenu.y}
          onClose={() => setClockAllMenu(null)}
          items={[
            {
              icon: "🏠",
              label: `전체 출근 (터미널 켜기, ${clockedOutCount}명)`,
              disabled: clockedOutCount === 0,
              onSelect: () => clockInAll(),
            },
            {
              icon: "🌙",
              label: `전체 퇴근 (${onDutyCount}명)`,
              danger: true,
              disabled: onDutyCount === 0,
              onSelect: () => openModal({ kind: "confirm-clock-out-all" }),
            },
          ]}
        />
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
      <UsageWidget />
      {/* 동료 대화 표시 + 킬스위치. talkEnabled가 꺼져 있으면 스스로 null 렌더. */}
      <TalkWidget />
      <button
        type="button"
        className="pixel-btn analytics-btn"
        aria-label="분석"
        title="세션 활동 분석"
        onClick={() => openModal({ kind: "analytics" })}
      >
        📊 분석
      </button>
      <button
        type="button"
        className="pixel-btn awards-btn"
        aria-label="이 달의 우수사원"
        title="이 달의 우수사원"
        onClick={() => openModal({ kind: "awards" })}
      >
        🏆 우수사원
      </button>
      <button
        type="button"
        className="pixel-btn settings-btn"
        aria-label="설정"
        title="설정 (선택적 에이전트 연동)"
        onClick={() => openModal({ kind: "settings" })}
      >
        ⚙
      </button>
      <button
        type="button"
        className="pixel-btn about-btn"
        aria-label="정보"
        title="Agent Office 정보"
        onClick={() => openModal({ kind: "about" })}
      >
        ℹ
      </button>
      <button
        type="button"
        className="pixel-btn scene-btn"
        aria-label="풍경 선택"
        aria-haspopup="menu"
        title="오피스 풍경 목록에서 고르기"
        onClick={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          setSceneMenu({ x: rect.left, y: rect.top });
        }}
      >
        풍경: {SCENES[scene].label}
      </button>
      {sceneMenu && (
        <ContextMenu
          x={sceneMenu.x}
          y={sceneMenu.y}
          onClose={() => setSceneMenu(null)}
          items={SCENE_ORDER.map((id) => ({
            // 테마 드롭다운과 같은 관례: 현재 항목만 체크 아이콘.
            icon: id === scene ? "✔" : undefined,
            label: SCENES[id].label,
            onSelect: () => setScene(id),
          }))}
        />
      )}
      <button
        type="button"
        className="pixel-btn theme-btn"
        aria-label="테마 선택"
        aria-haspopup="menu"
        title="테마 목록에서 고르기"
        onClick={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          setThemeMenu({ x: rect.left, y: rect.top });
        }}
      >
        테마: {THEMES[theme].label}
      </button>
      {themeMenu && (
        <ContextMenu
          x={themeMenu.x}
          y={themeMenu.y}
          onClose={() => setThemeMenu(null)}
          items={THEME_ORDER.map((id) => ({
            // 현재 테마는 체크로 표시(아이콘 슬롯 폭은 나머지 항목도 유지한다).
            icon: id === theme ? "✔" : undefined,
            label: THEMES[id].label,
            onSelect: () => setTheme(id),
          }))}
        />
      )}
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
