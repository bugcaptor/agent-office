// 일반 에이전트(전용 탭). 캐릭터 없는 사무실 세션 한 칸(`office-agent`)을 곁 탭으로 연다.
// 웹 상태판이 지시를 넘기면 연다(`webDashControl`). 캐릭터 세션처럼 관찰 훅·사무실 안내문을 다 받으므로
// 이 세션은 `office-task`로 업무·토의를 만들며 앱을 다룰 수 있다. 탭·상태는 여기만 쥔다
// (appStore.sessions·recentAgentIds에 넣지 않는다).
import { create } from "zustand";
import { OFFICE_AGENT_ID, isOfficeAgentId } from "@shared/types";
import type { SessionStateEvent, SessionStatus, WorkAgent } from "@shared/types";

type WorkCli = WorkAgent["cli"];
import { t } from "../i18n";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import { normalizeWorkAgent } from "../profile/generate";
import { terminalRegistry } from "../terminal/TerminalRegistry";
import { closeSideTab, isLiveStatus as isLive, leaveSideTab, sideTabInitialSize, sideTabStatus } from "../terminal/sideTabs";

export interface OfficeAgentSession {
  agentId: string;
  sessionId: string;
  status: SessionStatus;
  cwd?: string;
  cli: WorkCli;
  openedAt: number;
}
interface OfficeAgentState {
  /** agentId → 세션. 한 칸이지만 다른 곁 탭 스토어와 같은 모양으로 둔다. */
  sessions: Record<string, OfficeAgentSession>;
  order: string[];
  opening: boolean;
}
export const useOfficeAgentStore = create<OfficeAgentState>(() => ({ sessions: {}, order: [], opening: false }));

export { isOfficeAgentId };

export function liveOfficeAgentCount(): number {
  const session = useOfficeAgentStore.getState().sessions[OFFICE_AGENT_ID];
  return session && isLive(session.status) ? 1 : 0;
}

/** 탭에 보일 제목. */
export function officeAgentTitle(): string {
  return t("terminal:tab.officeAgent");
}

function forget(agentId: string): void {
  terminalRegistry.destroy(agentId);
  latest.delete(agentId);
  useOfficeAgentStore.setState((s) => {
    const sessions = { ...s.sessions };
    delete sessions[agentId];
    return { sessions, order: s.order.filter((id) => id !== agentId) };
  });
}

export interface OpenOfficeAgentOptions {
  prompt: string;
  cwd?: string;
  cli: WorkCli;
  /** 살아 있는 세션을 버리고 새로 연다(웹에서 경고를 확인했다). */
  restart?: boolean;
}

/** 열기를 막는 이유. 없으면 undefined. */
export function officeAgentBlockedReason(restart: boolean): string | undefined {
  const { sessions, opening } = useOfficeAgentStore.getState();
  if (opening) return "office-agent-busy";
  if (!restart && isLive(sessions[OFFICE_AGENT_ID]?.status)) return "office-agent-running";
  return undefined;
}

/**
 * 일반 에이전트를 연다. 탭은 만들되 앱에서 보고 있는 탭은 바꾸지 않는다(멀리서 보낸 지시가 책상 앞 화면을 뺏지 않게).
 * 실패하면 오류 코드를 돌려준다(탭은 생기지 않는다).
 */
export async function openOfficeAgent(options: OpenOfficeAgentOptions): Promise<string | undefined> {
  const restart = options.restart === true;
  const blocked = officeAgentBlockedReason(restart);
  if (blocked) return blocked;
  const agentId = OFFICE_AGENT_ID;
  useOfficeAgentStore.setState({ opening: true });
  // 새 세션은 새 화면에서 시작한다. 출력을 먼저 구독해 첫 화면을 놓치지 않는다.
  const viewing = useAppStore.getState().activeTerminalAgentId === agentId;
  if (useOfficeAgentStore.getState().sessions[agentId]) forget(agentId);
  terminalRegistry.ensure(agentId);
  try {
    const result = await tauriApi.openOfficeAgent(
      { prompt: options.prompt, agent: normalizeWorkAgent({ cli: options.cli }), ...(options.cwd ? { cwd: options.cwd } : {}), ...sideTabInitialSize() },
      restart,
    );
    const status = latest.get(agentId)?.sessionId === result.sessionId ? latest.get(agentId)!.status : "running";
    const session: OfficeAgentSession = { agentId, sessionId: result.sessionId, status, cli: options.cli, openedAt: Date.now(), ...(options.cwd ? { cwd: options.cwd } : {}) };
    useOfficeAgentStore.setState((s) => ({ sessions: { ...s.sessions, [agentId]: session }, order: [...s.order.filter((id) => id !== agentId), agentId] }));
    // 보던 탭을 다시 연 것이면 새 화면으로 다시 붙인다.
    if (viewing) useAppStore.getState().openTerminal(agentId);
    return undefined;
  } catch (error) {
    terminalRegistry.destroy(agentId);
    leaveSideTab(agentId, []);
    return error instanceof Error ? error.message : String(error);
  } finally {
    useOfficeAgentStore.setState({ opening: false });
  }
}

/** 탭 닫기. CLI가 살아 있으면 확인창을 먼저 띄운다. */
export function requestCloseOfficeAgent(agentId: string): void {
  const session = useOfficeAgentStore.getState().sessions[agentId];
  if (!session) return;
  if (isLive(session.status)) useAppStore.getState().openModal({ kind: "confirm-close-task-chat", agentId });
  else void closeOfficeAgent(agentId);
}

/** 세션을 버리고 탭을 없앤다. */
export async function closeOfficeAgent(agentId: string = OFFICE_AGENT_ID): Promise<void> {
  const { sessions, order } = useOfficeAgentStore.getState();
  const session = sessions[agentId];
  if (!session) {
    // 탭은 없는데 세션이 남아 있으면(앱이 다시 뜬 뒤 등) 세션만 버린다.
    try { await tauriApi.disposeSession(agentId); } catch (err) { console.warn("office-agent: dispose failed", err); }
    return;
  }
  await closeSideTab(agentId, { order, live: isLive(session.status), forget, logTag: "office-agent" });
}

/** 마지막 세션 상태(탭이 생기기 전에 온 것도). 열기 결과의 sessionId와 맞으면 그 상태로 시작한다. */
const latest = new Map<string, { sessionId: string; status: SessionStatus }>();
export function applyOfficeAgentState(e: SessionStateEvent): void {
  const status = sideTabStatus(e);
  const { sessions, opening } = useOfficeAgentStore.getState();
  if (!latest.has(e.agentId) && !sessions[e.agentId] && !opening && status === "exited") return;
  latest.set(e.agentId, { sessionId: e.sessionId, status });
  useOfficeAgentStore.setState((s) => {
    const session = s.sessions[e.agentId];
    if (!session || session.sessionId !== e.sessionId || session.status === status) return s;
    return { sessions: { ...s.sessions, [e.agentId]: { ...session, status } } };
  });
}

export function installOfficeAgentTracking(): () => void {
  return tauriApi.onSessionState((e) => { if (isOfficeAgentId(e.agentId)) applyOfficeAgentState(e); });
}
