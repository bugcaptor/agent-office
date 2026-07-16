// src/renderer/terminal/AgentTabStrip.tsx
//
// Terminal overlay header: tab strip over `recentAgentIds` — the store's own
// tab-strip-order field (LRU, most-recent-first).
// Clicking a tab keeps the overlay mounted and only switches
// `activeTerminalAgentId` (no remount, handled by TerminalHost).
//
// Keyboard routing, active only while a terminal is open:
// - Cmd/Ctrl+1..9  -> jump to that tab index.
// - Cmd/Ctrl+W     -> close the overlay (`closeTerminal`).
// - Escape         -> deliberately NOT handled here. Claiming Escape would
//   break TUI apps (vim etc.) that need a real Escape keystroke delivered to
//   the shell; overlay close is header-X-button/Cmd+W only.
import { useEffect, useMemo, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { useAppStore } from "../store/appStore";
import { generateSpritePreview } from "../office/gen/characterFactory";
import { resolveArchetype } from "../office/gen/archetypes";
import { ContextMenu } from "../ui/ContextMenu";
import { tauriApi } from "../ipc/tauriApi";

export function AgentTabStrip() {
  const isOpen = useAppStore((s) => s.activeTerminalAgentId !== null);
  const activeId = useAppStore((s) => s.activeTerminalAgentId);
  // `recentAgentIds` (string[]) is used directly rather than mapped to
  // `{id, name}` objects here: mapping to fresh object literals inside the
  // selector would make every snapshot referentially new even when nothing
  // changed, defeating useShallow's equality check and causing an infinite
  // render loop. `agents` is looked up separately — its reference is stable
  // across renders unless a profile actually changes.
  const tabIds = useAppStore(useShallow((s) => s.recentAgentIds));
  const agents = useAppStore((s) => s.agents);
  const sessions = useAppStore((s) => s.sessions);
  const portraits = useAppStore((s) => s.portraits);
  const spritePreviews = useAppStore((s) => s.spritePreviews);
  const tabs = useMemo(
    () =>
      tabIds.map((id) => {
        const agent = agents[id];
        const thumb =
          portraits[id] ??
          spritePreviews[id] ??
          (agent
            ? generateSpritePreview(
                agent.seed || agent.id,
                6,
                undefined,
                undefined,
                // 월드(createCharacterAssets)와 동일한 아키타입 해석 —
                // 누락 시 폴백 썸네일이 항상 human으로 렌더되는 버그.
                resolveArchetype(agent.archetype, agent.seed || agent.id)
              )
            : undefined);
        return { id, name: agent?.name ?? id, thumb };
      }),
    [tabIds, agents, portraits, spritePreviews]
  );
  const openTerminal = useAppStore((s) => s.openTerminal);
  const closeTerminal = useAppStore((s) => s.closeTerminal);
  const openModal = useAppStore((s) => s.openModal);
  const [menu, setMenu] = useState<{ agentId: string; x: number; y: number } | null>(null);

  useEffect(() => {
    if (!isOpen) return;

    function onKeyDown(e: KeyboardEvent) {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) return;

      if (e.key.toLowerCase() === "w") {
        e.preventDefault();
        closeTerminal();
        return;
      }

      if (/^[1-9]$/.test(e.key)) {
        const tab = tabs[Number(e.key) - 1];
        if (tab) {
          e.preventDefault();
          openTerminal(tab.id);
        }
      }
      // No `default:`/Escape case on purpose — see file header.
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [isOpen, tabs, openTerminal, closeTerminal]);

  return (
    <div className="agent-tab-strip" role="tablist">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          role="tab"
          aria-selected={tab.id === activeId}
          className={tab.id === activeId ? "agent-tab agent-tab-active" : "agent-tab"}
          onClick={() => openTerminal(tab.id)}
          onContextMenu={(e) => {
            e.preventDefault();
            setMenu({ agentId: tab.id, x: e.clientX, y: e.clientY });
          }}
        >
          {tab.thumb && (
            <img className="agent-tab-thumb" src={tab.thumb} alt="" aria-hidden="true" />
          )}
          {tab.name}
        </button>
      ))}
      <button
        type="button"
        className="agent-tab-strip-close"
        aria-label="Close terminal overlay"
        onClick={closeTerminal}
      >
        ×
      </button>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={[
            {
              label: "터미널 재시작",
              onSelect: () =>
                openModal({ kind: "confirm-restart", agentId: menu.agentId }),
            },
            {
              label: "터미널 종료",
              // PTY가 살아있을 때만 의미가 있다 — 이미 exited/idle이면 캐릭터는
              // 탕비실(또는 재소환 대기)이므로 비활성화.
              disabled: !["starting", "running"].includes(
                sessions[menu.agentId]?.status ?? "idle"
              ),
              onSelect: () =>
                openModal({ kind: "confirm-terminate", agentId: menu.agentId }),
            },
            {
              label: "VS Code로 열기",
              // 작업 폴더(cwd) 미설정 프로필은 비활성화 — 홈 디렉터리 폴백 없음.
              disabled: !agents[menu.agentId]?.cwd,
              onSelect: () => {
                const cwd = agents[menu.agentId]?.cwd;
                if (!cwd) return;
                void tauriApi
                  .openInVscode(cwd)
                  .catch((err) => console.warn("VS Code 열기 실패", err));
              },
            },
            {
              // 인앱 PTY(터미널 재시작/종료)와 구분되는 외부 OS 터미널 앱.
              label: "OS 터미널로 열기",
              disabled: !agents[menu.agentId]?.cwd,
              onSelect: () => {
                const cwd = agents[menu.agentId]?.cwd;
                if (!cwd) return;
                void tauriApi
                  .openInTerminal(cwd)
                  .catch((err) => console.warn("OS 터미널 열기 실패", err));
              },
            },
            {
              label: "프로필 편집",
              onSelect: () =>
                openModal({ kind: "profile-edit", agentId: menu.agentId }),
            },
            {
              label: "퇴근",
              onSelect: () =>
                openModal({ kind: "confirm-clock-out", agentId: menu.agentId }),
            },
            {
              label: "캐릭터 삭제",
              onSelect: () =>
                openModal({ kind: "confirm-delete", agentId: menu.agentId }),
            },
          ]}
        />
      )}
    </div>
  );
}
