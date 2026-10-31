// src/renderer/shortcuts/agentShortcuts.ts
//
// 설정에서 지정한 단축키로 에이전트 터미널을 바로 연다(앱 창 안에서만).
// - 에이전트별 키: 그 에이전트 터미널을 연다. 이미 앞에 열려 있으면 아무 일 없음.
// - 알림 키: 남은 알림 중 가장 오래된 것의 에이전트로 간다. 열면 그 에이전트
//   알림이 지워지므로 다시 누르면 그다음 오래된 알림으로 간다.
// 여는 동작은 캐릭터 클릭과 같은 `officeBus.emitAgentClicked`(세션 보장 + 터미널
// 열기 + 백엔드 알림 지우기)다.
//
// window 캡처 단계에서 받는다 — 터미널(xterm)에 포커스가 있어도 셸보다 먼저
// 가로채야 하기 때문이다. 맞으면 preventDefault + stopPropagation.
import { useAppStore } from "../store/appStore";
import { officeBus } from "../ipc/sessionBridge";
import { IS_MAC } from "../shared/platform";
import { isTaskChatId } from "@shared/types";
import { matchesAccelerator } from "./accelerator";
import type { AgentProfile } from "@shared/types";
import type { Notification } from "../store/types";

export interface ShortcutContext {
  agentShortcuts: Record<string, string>;
  notifyJumpShortcut: string;
  agents: Record<string, AgentProfile>;
  /** 최신순(appStore.notifications 그대로). */
  notifications: Notification[];
  activeTerminalAgentId: string | null;
}

type KeyLike = Parameters<typeof matchesAccelerator>[0];

/**
 * 이 키가 가리키는 에이전트. 단축키가 아니면 null, 단축키지만 할 일이 없으면
 * (이미 열려 있음, 퇴근, 남은 알림 없음) "consume" — 키는 삼키되 아무것도 하지 않는다.
 */
export function resolveShortcut(
  e: KeyLike,
  ctx: ShortcutContext,
  isMac: boolean,
): { agentId: string } | "consume" | null {
  if (matchesAccelerator(e, ctx.notifyJumpShortcut, isMac)) {
    // 최신순 배열이므로 뒤에서부터 = 오래된 것부터. 열 수 없는 대상(지워진
    // 에이전트, 퇴근)은 건너뛴다 — 안 그러면 그 알림에 막혀 영영 못 넘어간다.
    for (let i = ctx.notifications.length - 1; i >= 0; i--) {
      const id = ctx.notifications[i].agentId;
      if (isTaskChatId(id)) return { agentId: id };
      const agent = ctx.agents[id];
      if (agent && !agent.clockedOut) return { agentId: id };
    }
    return "consume";
  }
  for (const [agentId, acc] of Object.entries(ctx.agentShortcuts)) {
    if (!matchesAccelerator(e, acc, isMac)) continue;
    const agent = ctx.agents[agentId];
    if (!agent || agent.clockedOut) return "consume";
    if (ctx.activeTerminalAgentId === agentId) return "consume";
    return { agentId };
  }
  return null;
}

function onKeyDown(e: KeyboardEvent): void {
  if (e.isComposing || e.repeat) return;
  const s = useAppStore.getState();
  // 모달 위에서 누르면 터미널이 모달 뒤에 열린다 — 모달이 닫힌 뒤에만.
  // 설정 창(단축키 녹화)도 모달이라 녹화 중에는 여기서 걸러진다.
  if (s.modal.kind !== "none") return;
  const target = resolveShortcut(
    e,
    {
      agentShortcuts: s.appSettings.agentShortcuts,
      notifyJumpShortcut: s.appSettings.notifyJumpShortcut,
      agents: s.agents,
      notifications: s.notifications,
      activeTerminalAgentId: s.activeTerminalAgentId,
    },
    IS_MAC,
  );
  if (target === null) return;
  e.preventDefault();
  e.stopPropagation();
  if (target !== "consume") officeBus.emitAgentClicked(target.agentId);
}

/** 부팅 때 한 번. 해제 함수를 돌려준다. */
export function installAgentShortcuts(): () => void {
  if (typeof window === "undefined") return () => {};
  window.addEventListener("keydown", onKeyDown, true);
  return () => window.removeEventListener("keydown", onKeyDown, true);
}
