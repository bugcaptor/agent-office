// src/renderer/terminal/AgentTabStrip.tsx
//
// Terminal overlay header: tab strip over `recentAgentIds` — the store's own
// tab-strip-order field (LRU, most-recent-first).
// Clicking a tab keeps the overlay mounted and only switches
// `activeTerminalAgentId` (no remount, handled by TerminalHost).
//
// Keyboard routing, active only while a terminal is open:
// - Cmd/Ctrl+1..9      -> jump to that tab index.
// - Cmd/Ctrl+W         -> close the overlay (`closeTerminal`).
// - Cmd/Ctrl+Shift+E   -> 셸 출력을 에디터로 내보내기(이슈 #42, activeId 대상).
// - Escape         -> deliberately NOT handled here. Claiming Escape would
//   break TUI apps (vim etc.) that need a real Escape keystroke delivered to
//   the shell; overlay close is header-X-button/Cmd+W only.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { useAppStore } from "../store/appStore";
import { generateSpritePreview } from "../office/gen/characterFactory";
import { resolveArchetype } from "../office/gen/archetypes";
import { ContextMenu } from "../ui/ContextMenu";
import { deriveTaskLabelLines } from "../labels/labelText";
import { tauriApi } from "../ipc/tauriApi";
import { useMarkdownStore } from "../markdown/markdownStore";
import { useWorkdirStore } from "../workdir/workdirStore";
import { terminalRegistry } from "./TerminalRegistry";
import type { ClaudeResumeEntry } from "@shared/types";

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
  // 탭 툴팁(이슈 #44 T2)용 라벨 소스. 원본 참조 그대로 구독하고 파생은
  // 렌더(useMemo)에서 — 셀렉터에서 새 객체를 만들면 useShallow가 무력화된다.
  const taskLabels = useAppStore((s) => s.taskLabels);
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
        // 탭 툴팁(이슈 #44 T2): 머리 위 라벨과 같은 파생 규칙으로 2줄을 만들어
        // native title로 붙인다(폭 넉넉히). 세션이 starting/running이 아니면
        // 실황(line2)은 stale이므로 line1만. 라벨이 없으면 title 생략.
        const { line1, line2 } = deriveTaskLabelLines(taskLabels[id], agent?.cwd, {
          goalMax: 80,
          currentMax: 120,
        });
        const status = sessions[id]?.status;
        const live = status === "starting" || status === "running";
        const titleLines = [line1, live ? line2 : undefined].filter(Boolean);
        const title = titleLines.length > 0 ? titleLines.join("\n") : undefined;
        return { id, name: agent?.name ?? id, thumb, title };
      }),
    [tabIds, agents, sessions, taskLabels, portraits, spritePreviews]
  );
  const openTerminal = useAppStore((s) => s.openTerminal);
  const closeTerminal = useAppStore((s) => s.closeTerminal);
  const openModal = useAppStore((s) => s.openModal);
  // 이슈 #10: 활성 에이전트 cwd를 root로 마크다운 문서 팔레트를 연다.
  const openMarkdownPalette = useMarkdownStore((s) => s.openPalette);
  // 이슈 #11: 작업 폴더 보기(파일 목록 + git 상태) 오버레이를 연다.
  const openWorkdirPalette = useWorkdirStore((s) => s.openPalette);
  // 활성 에이전트의 cwd(문서 버튼 활성 조건). 없으면 버튼 비활성.
  const activeCwd = activeId ? agents[activeId]?.cwd : undefined;
  const [menu, setMenu] = useState<{ agentId: string; x: number; y: number } | null>(null);
  // 메뉴를 열 때 조회한 Claude 이어하기 후보(agentId → 최신 1건). 엔트리가
  // 있는 에이전트만 "이전 세션 이어하기"가 활성화된다. 열 때마다 비우고
  // 응답 도착까지는 비활성 — 이전 조회의 낡은 ID(/clear 후 등)가 잠깐이라도
  // 활성으로 노출되면 엉뚱한 대화를 이어버린다(Codex 리뷰 지적).
  const [resumeEntries, setResumeEntries] = useState<Record<string, ClaudeResumeEntry>>({});
  // 조회 세대 — 메뉴를 연달아 열 때 늦게 도착한 옛 응답이 최신 상태를
  // 덮지 않게 최신 세대의 응답만 반영한다.
  const resumeFetchSeq = useRef(0);

  // 이슈 #42: 해당 에이전트 터미널의 현재 버퍼를 뽑아 외부 에디터로 내보낸다.
  // 미생성 터미널(getPlainText === undefined)은 무시. 파일명은 프로필 이름 폴백.
  const exportShellOutput = useCallback(
    (agentId: string) => {
      const text = terminalRegistry.getPlainText(agentId);
      if (text === undefined) return;
      void tauriApi
        .exportTerminalOutput(agents[agentId]?.name ?? agentId, text)
        .catch((err) => console.warn("셸 출력 내보내기 실패", err));
    },
    [agents]
  );

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

      // Cmd/Ctrl+Shift+E: 현재 활성 터미널의 셸 출력을 에디터로 내보내기.
      if (e.shiftKey && e.key.toLowerCase() === "e") {
        if (activeId === null) return;
        e.preventDefault();
        exportShellOutput(activeId);
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
  }, [isOpen, tabs, activeId, openTerminal, closeTerminal, exportShellOutput]);

  return (
    <div className="agent-tab-strip" role="tablist">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          role="tab"
          aria-selected={tab.id === activeId}
          className={tab.id === activeId ? "agent-tab agent-tab-active" : "agent-tab"}
          title={tab.title}
          onClick={() => openTerminal(tab.id)}
          onContextMenu={(e) => {
            e.preventDefault();
            setMenu({ agentId: tab.id, x: e.clientX, y: e.clientY });
            // 메뉴가 열리는 동안 이어하기 후보를 조회한다. 응답이 오면
            // 리렌더되어 해당 항목의 활성 여부가 갱신된다(약간의 지연 허용).
            // 조회 전엔 항상 비운다 — 실패하면 비활성인 채로 남는다.
            setResumeEntries({});
            const seq = ++resumeFetchSeq.current;
            void tauriApi
              .listClaudeResumeSessions()
              .then((entries) => {
                if (resumeFetchSeq.current === seq) setResumeEntries(entries);
              })
              .catch((err) => console.warn("이어하기 후보 조회 실패", err));
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
        className="agent-tab-strip-docs"
        // 활성 에이전트 cwd를 root로 마크다운 문서 팔레트 오픈. cwd 없으면 비활성.
        title="마크다운 문서 열기"
        disabled={!activeCwd}
        onClick={() => {
          if (activeId && activeCwd) openMarkdownPalette(activeCwd, activeId);
        }}
      >
        문서
      </button>
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
              label: "이전 세션 이어하기",
              // 캡처된 Claude native 세션이 있을 때만 활성 — 없으면 비활성.
              disabled: !resumeEntries[menu.agentId],
              onSelect: () => {
                const entry = resumeEntries[menu.agentId];
                if (!entry) return;
                openModal({
                  kind: "confirm-resume",
                  agentId: menu.agentId,
                  sessionId: entry.sessionId,
                });
              },
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
              label: "작업 폴더 보기",
              // 작업 폴더(cwd) 미설정 프로필은 비활성화 — 홈 디렉터리 폴백 없음.
              disabled: !agents[menu.agentId]?.cwd,
              onSelect: () => {
                const cwd = agents[menu.agentId]?.cwd;
                if (cwd) openWorkdirPalette(cwd, menu.agentId);
              },
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
              // 이슈 #42: 현재 터미널 버퍼(스크롤백 포함)를 .txt로 내보내 에디터로 연다.
              label: "셸 출력을 에디터로 보기",
              // 터미널이 아직 만들어지지 않았으면(has === false) 뽑을 버퍼가 없다.
              disabled: !terminalRegistry.has(menu.agentId),
              onSelect: () => exportShellOutput(menu.agentId),
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
