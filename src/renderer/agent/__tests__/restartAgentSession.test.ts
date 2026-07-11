// src/renderer/agent/__tests__/restartAgentSession.test.ts
//
// TDD for 터미널 재시작 오케스트레이터: disposeSession → registry.destroy →
// bumpTerminalEpoch → setSessionState(starting) → createSession 순서 보장,
// 그리고 각 단계 실패 시의 폴백 동작(deleteAgent.test.ts의 목 패턴 참고).
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

const disposeSession = vi.fn().mockResolvedValue(undefined);
const createSession = vi.fn().mockResolvedValue(undefined);
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    disposeSession: (...args: unknown[]) => disposeSession(...args),
    createSession: (...args: unknown[]) => createSession(...args),
  },
}));

const destroy = vi.fn();
vi.mock("../../terminal/TerminalRegistry", () => ({
  terminalRegistry: {
    destroy: (...args: unknown[]) => destroy(...args),
  },
}));

const { restartAgentSession } = await import("../restartAgentSession");

function mkProfile(id: string, overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id,
    name: `Agent ${id}`,
    role: "eng",
    note: "",
    seed: id,
    createdAt: Date.now(),
    deskIndex: 0,
    ...overrides,
  };
}

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
  disposeSession.mockClear();
  createSession.mockClear();
  destroy.mockClear();
  disposeSession.mockResolvedValue(undefined);
  createSession.mockResolvedValue(undefined);
});

describe("restartAgentSession 오케스트레이션", () => {
  it("disposeSession → destroy → bumpTerminalEpoch → starting → createSession(cwd) 순서로 진행한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", { cwd: "/work/a1" }));

    const order: string[] = [];
    disposeSession.mockImplementationOnce(async (id: string) => {
      order.push(`dispose:${id}`);
    });
    destroy.mockImplementationOnce((id: string) => order.push(`destroy:${id}`));
    createSession.mockImplementationOnce(async (id: string, opts: unknown) => {
      order.push(`create:${id}:${JSON.stringify(opts)}`);
      // 이 시점에는 이미 starting/에폭 증가가 반영돼 있어야 한다.
      order.push(`status-at-create:${useAppStore.getState().sessions.a1.status}`);
      order.push(`epoch-at-create:${useAppStore.getState().terminalEpochs.a1}`);
    });

    await restartAgentSession("a1");

    expect(order).toEqual([
      "dispose:a1",
      "destroy:a1",
      'create:a1:{"cwd":"/work/a1"}',
      "status-at-create:starting",
      "epoch-at-create:1",
    ]);
    expect(useAppStore.getState().terminalEpochs.a1).toBe(1);
    expect(useAppStore.getState().sessions.a1.status).toBe("starting");
  });

  it("cwd 없는 에이전트는 createSession을 opts 없이 호출한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));

    await restartAgentSession("a1");

    expect(createSession).toHaveBeenCalledWith("a1", undefined);
  });

  it("shell이 설정된 에이전트는 createSession opts에 shell을 포함한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", { shell: "wsl" }));

    await restartAgentSession("a1");

    expect(createSession).toHaveBeenCalledWith("a1", { shell: "wsl" });
  });

  it("cwd와 shell이 모두 설정된 에이전트는 createSession opts에 둘 다 포함한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", { cwd: "/work/a1", shell: "wsl" }));

    await restartAgentSession("a1");

    expect(createSession).toHaveBeenCalledWith("a1", { cwd: "/work/a1", shell: "wsl" });
  });

  it("disposeSession이 실패해도 재시작은 계속 진행된다", async () => {
    disposeSession.mockRejectedValueOnce(new Error("no such session"));
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));

    await restartAgentSession("a1");

    expect(destroy).toHaveBeenCalledWith("a1");
    expect(useAppStore.getState().terminalEpochs.a1).toBe(1);
    expect(createSession).toHaveBeenCalledWith("a1", undefined);
    expect(useAppStore.getState().sessions.a1.status).toBe("starting");
  });

  it("createSession이 실패하면 상태가 exited로 바뀐다", async () => {
    createSession.mockRejectedValueOnce(new Error("spawn failed"));
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));

    await restartAgentSession("a1");

    expect(useAppStore.getState().sessions.a1.status).toBe("exited");
  });

  it("createSession 결과가 running이면 상태를 running으로 반영한다(재사용 경로 — 상태 이벤트 없음)", async () => {
    createSession.mockResolvedValueOnce({ sessionId: "s1", state: "running" });
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));

    await restartAgentSession("a1");

    expect(useAppStore.getState().sessions.a1.status).toBe("running");
  });

  it("createSession invoke가 영원히 settle되지 않으면 타임아웃 후 exited로 복구된다", async () => {
    vi.useFakeTimers();
    try {
      const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
      createSession.mockReturnValueOnce(new Promise(() => {})); // 백엔드 패닉 = 영구 미해결
      const s = useAppStore.getState();
      s.addAgent(mkProfile("a1"));

      const done = restartAgentSession("a1");
      expect(useAppStore.getState().sessions.a1.status).toBe("starting");

      await vi.advanceTimersByTimeAsync(15_001);
      await done;

      expect(useAppStore.getState().sessions.a1.status).toBe("exited");
      warn.mockRestore();
    } finally {
      vi.useRealTimers();
    }
  });

  it("여러 번 재시작하면 에폭이 매번 증가한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));

    await restartAgentSession("a1");
    await restartAgentSession("a1");

    expect(useAppStore.getState().terminalEpochs.a1).toBe(2);
    expect(destroy).toHaveBeenCalledTimes(2);
  });
});
