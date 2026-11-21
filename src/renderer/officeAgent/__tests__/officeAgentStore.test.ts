import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionStateEvent } from "@shared/types";
const { api, registry, stateCbs } = vi.hoisted(() => ({
  api: { openOfficeAgent: vi.fn(), disposeSession: vi.fn() },
  registry: { ensure: vi.fn(), destroy: vi.fn(), get: vi.fn(), has: vi.fn() },
  stateCbs: new Set<(e: SessionStateEvent) => void>(),
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: {
  ...api,
  onSessionState: (cb: (e: SessionStateEvent) => void) => { stateCbs.add(cb); return () => stateCbs.delete(cb); },
} }));
vi.mock("../../terminal/TerminalRegistry", () => ({ terminalRegistry: registry }));
import { OFFICE_AGENT_ID, isSideSessionId } from "@shared/types";
import { useAppStore } from "../../store/appStore";
import {
  closeOfficeAgent, installOfficeAgentTracking, liveOfficeAgentCount, officeAgentBlockedReason, openOfficeAgent, useOfficeAgentStore,
} from "../officeAgentStore";

const initialApp = useAppStore.getState();
let off: () => void;
function emit(e: Partial<SessionStateEvent>) { stateCbs.forEach((cb) => cb({ agentId: OFFICE_AGENT_ID, sessionId: "", state: "running", at: 1, ...e } as SessionStateEvent)); }

beforeEach(() => {
  Object.values(api).forEach((f) => f.mockReset());
  Object.values(registry).forEach((f) => f.mockReset());
  let n = 0;
  api.openOfficeAgent.mockImplementation(async () => ({ agentId: OFFICE_AGENT_ID, sessionId: `sid-${++n}` }));
  api.disposeSession.mockResolvedValue(undefined);
  useAppStore.setState({ ...initialApp, activeTerminalAgentId: "a", recentAgentIds: ["a"] }, true);
  useOfficeAgentStore.setState({ sessions: {}, order: [], opening: false }, true);
  off = installOfficeAgentTracking();
});
afterEach(() => off());

describe("일반 에이전트 탭", () => {
  it("곁 세션이고, 열면 탭을 만들되 보던 탭은 바꾸지 않는다", async () => {
    expect(isSideSessionId(OFFICE_AGENT_ID)).toBe(true);
    expect(await openOfficeAgent({ prompt: "정리해 줘", cwd: "/work", cli: "codex" })).toBeUndefined();
    const [request, restart] = api.openOfficeAgent.mock.calls[0];
    expect(request).toMatchObject({ prompt: "정리해 줘", cwd: "/work", agent: { cli: "codex" } });
    expect(restart).toBe(false);
    expect(registry.ensure).toHaveBeenCalledWith(OFFICE_AGENT_ID);
    expect(useOfficeAgentStore.getState().order).toEqual([OFFICE_AGENT_ID]);
    expect(useAppStore.getState().activeTerminalAgentId).toBe("a");
    expect(liveOfficeAgentCount()).toBe(1);
  });

  it("살아 있으면 새로 열기를 막고, 재시작 확인이면 새 세션으로 바꾼다", async () => {
    await openOfficeAgent({ prompt: "one", cli: "claude" });
    expect(officeAgentBlockedReason(false)).toBe("office-agent-running");
    expect(await openOfficeAgent({ prompt: "two", cli: "claude" })).toBe("office-agent-running");
    expect(await openOfficeAgent({ prompt: "two", cli: "claude", restart: true })).toBeUndefined();
    expect(api.openOfficeAgent.mock.calls[1][1]).toBe(true);
    // 버린 옛 세션의 끝 신호는 새 탭 상태를 바꾸지 않는다.
    emit({ sessionId: "sid-1", state: "exited" });
    expect(useOfficeAgentStore.getState().sessions[OFFICE_AGENT_ID]).toMatchObject({ sessionId: "sid-2", status: "running" });
    emit({ sessionId: "sid-2", state: "exited" });
    expect(liveOfficeAgentCount()).toBe(0);
    expect(officeAgentBlockedReason(false)).toBeUndefined();
  });

  it("열기에 실패하면 탭을 남기지 않는다", async () => {
    api.openOfficeAgent.mockRejectedValueOnce(new Error("work-send-launch-timeout"));
    expect(await openOfficeAgent({ prompt: "x", cli: "claude" })).toBe("work-send-launch-timeout");
    expect(useOfficeAgentStore.getState().order).toEqual([]);
    expect(registry.destroy).toHaveBeenCalledWith(OFFICE_AGENT_ID);
  });

  it("끝내면 세션을 버리고 탭을 없앤다", async () => {
    await openOfficeAgent({ prompt: "x", cli: "claude" });
    await closeOfficeAgent();
    expect(api.disposeSession).toHaveBeenCalledWith(OFFICE_AGENT_ID);
    expect(useOfficeAgentStore.getState().order).toEqual([]);
  });
});
