// 곁 세션 탭(태스크 대화·토의 참가자)의 공통 동작. 캐릭터 없는 앱 안 터미널이라 appStore.sessions에 넣지 않고
// 각 스토어가 탭·상태를 쥔다. 여기는 두 스토어가 같은 규칙으로 쓰는 판정·크기·닫기만 둔다.
import type { SessionStateEvent, SessionStatus } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import { terminalRegistry } from "./TerminalRegistry";

/** CLI가 살아 있는 세션 상태. */
export function isLiveStatus(status: SessionStatus | undefined): boolean {
  return status === "starting" || status === "running";
}

/** 세션 상태 이벤트 → 탭 상태. 버려진 세션은 끝난 것으로 본다. */
export function sideTabStatus(e: Pick<SessionStateEvent, "state">): SessionStatus {
  return e.state === "disposed" ? "exited" : e.state;
}

/** 새 곁 세션의 첫 크기. 지금 보이는 터미널, 없으면 보던(또는 최근) 캐릭터 세션 크기를 빌린다. 탭을 열면 다시 맞춘다. */
export function sideTabInitialSize(): { cols?: number; rows?: number } {
  const { activeTerminalAgentId, recentAgentIds, sessions } = useAppStore.getState();
  const fitted = activeTerminalAgentId && terminalRegistry.get(activeTerminalAgentId)?.term;
  if (fitted && fitted.cols > 0 && fitted.rows > 0) return { cols: fitted.cols, rows: fitted.rows };
  const source = sessions[activeTerminalAgentId ?? ""] ?? sessions[recentAgentIds[0] ?? ""];
  return source ? { cols: source.cols, rows: source.rows } : {};
}

/** 보던 탭이 곁 세션 탭이면 이웃 탭(같은 종류), 없으면 최근 캐릭터 탭으로 옮긴다. 다른 탭을 보고 있으면 그대로 둔다. */
export function leaveSideTab(agentId: string, order: readonly string[]): void {
  const app = useAppStore.getState();
  if (app.activeTerminalAgentId !== agentId) return;
  const idx = order.indexOf(agentId);
  const next = (idx >= 0 ? order[idx + 1] ?? order[idx - 1] : undefined) ?? app.recentAgentIds[0];
  if (next) app.openTerminal(next);
  else app.closeTerminal();
}

/** 곁 세션 탭을 닫는다. 보던 탭이면 옮기고(`leaveSideTab`), 탭을 잊은 뒤 CLI가 살아 있었으면 세션을 버린다. */
export async function closeSideTab(agentId: string, tab: {
  order: readonly string[];
  live: boolean;
  forget: (agentId: string) => void;
  logTag: string;
}): Promise<void> {
  leaveSideTab(agentId, tab.order);
  tab.forget(agentId);
  if (tab.live) {
    try { await tauriApi.disposeSession(agentId); } catch (err) { console.warn(`${tab.logTag}: dispose failed for ${agentId}`, err); }
  }
}
