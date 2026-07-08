// src/renderer/agent/__tests__/deleteAgent.test.ts
//
// deleteAgent TDD: PTY 종료 → 활성 탭 전환 → removeAgent 캐스케이드 →
// xterm 정리의 순서 보장.
//
// tauriApi / TerminalRegistry는 모듈 목으로 대체한다: 전자는 실제 IPC
// (invoke) 호출을, 후자는 @xterm/xterm CSS/DOM 로드를 피하기 위함. 스토어는
// 실제 useAppStore를 사용해 removeAgent 캐스케이드와 활성 탭 전환을 검증.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

const disposeSession = vi.fn().mockResolvedValue(undefined);
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    disposeSession: (...args: unknown[]) => disposeSession(...args),
  },
}));

const destroy = vi.fn();
vi.mock("../../terminal/TerminalRegistry", () => ({
  terminalRegistry: {
    destroy: (...args: unknown[]) => destroy(...args),
  },
}));

const { deleteAgent } = await import("../deleteAgent");

function mkProfile(id: string): AgentProfile {
  return {
    id,
    name: `Agent ${id}`,
    role: "eng",
    note: "",
    seed: id,
    createdAt: Date.now(),
    deskIndex: 0,
  };
}

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
  disposeSession.mockClear();
  destroy.mockClear();
  disposeSession.mockResolvedValue(undefined);
});

describe("deleteAgent 오케스트레이션", () => {
  it("disposeSession 호출 + removeAgent 캐스케이드 + TerminalRegistry.destroy 호출", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.openTerminal("a1");

    await deleteAgent("a1");

    expect(disposeSession).toHaveBeenCalledWith("a1");
    const st = useAppStore.getState();
    expect(st.agents.a1).toBeUndefined();
    expect(st.sessions.a1).toBeUndefined();
    expect(st.agentOrder).not.toContain("a1");
    expect(st.recentAgentIds).not.toContain("a1");
    expect(destroy).toHaveBeenCalledWith("a1");
  });

  it("활성 탭 삭제 시 다음(인접) 에이전트로 전환한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.addAgent(mkProfile("a2"));
    s.addAgent(mkProfile("a3"));
    s.openTerminal("a3");
    s.openTerminal("a2");
    s.openTerminal("a1"); // recentAgentIds = [a1, a2, a3], active = a1

    await deleteAgent("a1");

    expect(useAppStore.getState().activeTerminalAgentId).toBe("a2");
  });

  it("다음 이웃이 없으면 이전 이웃으로 전환한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.addAgent(mkProfile("a2"));
    s.addAgent(mkProfile("a3"));
    // active가 탭 목록의 마지막이라 next(idx+1)가 없는 상황을 직접 구성.
    useAppStore.setState({ recentAgentIds: ["a1", "a2", "a3"], activeTerminalAgentId: "a3" });

    await deleteAgent("a3");

    expect(useAppStore.getState().activeTerminalAgentId).toBe("a2");
  });

  it("마지막 남은 탭을 삭제하면 활성 탭은 null", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.openTerminal("a1");

    await deleteAgent("a1");

    expect(useAppStore.getState().activeTerminalAgentId).toBeNull();
  });

  it("비활성 탭을 삭제해도 활성 탭은 그대로 유지된다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.addAgent(mkProfile("a2"));
    s.openTerminal("a2");
    s.openTerminal("a1"); // recentAgentIds = [a1, a2], active = a1

    await deleteAgent("a2");

    expect(useAppStore.getState().activeTerminalAgentId).toBe("a1");
    expect(useAppStore.getState().agents.a2).toBeUndefined();
  });

  it("disposeSession 대기 중 스토어가 변해도 최신 상태 기준으로 탭을 전환한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.addAgent(mkProfile("a2"));
    s.addAgent(mkProfile("a3"));
    s.openTerminal("a3");
    s.openTerminal("a2");
    s.openTerminal("a1"); // recentAgentIds = [a1, a2, a3], active = a1

    let resolveDispose!: () => void;
    disposeSession.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          resolveDispose = resolve;
        }),
    );

    const deletePromise = deleteAgent("a1");

    // disposeSession 대기 중, 겹치는 동작(예: 사용자의 다른 탭 전환 또는
    // 다른 삭제 호출)으로 스토어가 바뀐다: 활성 탭이 a1이 아닌 a3로 바뀜.
    useAppStore.getState().openTerminal("a3");
    expect(useAppStore.getState().activeTerminalAgentId).toBe("a3");

    resolveDispose();
    await deletePromise;

    // 활성 탭 판정은 disposeSession 완료 후의 최신 상태를 기준으로 해야
    // 한다. await 이전 스냅샷을 썼다면 활성 탭이 여전히 a1이라고 오판해
    // 이웃 a2로 잘못 전환했을 것이다 — 이미 a3로 옮겨간 활성 탭을 덮어쓰지
    // 않아야 한다.
    const st = useAppStore.getState();
    expect(st.activeTerminalAgentId).toBe("a3");
    expect(st.agents.a1).toBeUndefined();
  });

  it("disposeSession이 실패해도 삭제는 계속 진행된다 (이미 죽은 세션)", async () => {
    disposeSession.mockRejectedValueOnce(new Error("no such session"));
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.openTerminal("a1");

    await deleteAgent("a1");

    expect(useAppStore.getState().agents.a1).toBeUndefined();
    expect(destroy).toHaveBeenCalledWith("a1");
  });
});
