// src/renderer/agent/__tests__/restartAgentSession.test.ts
//
// TDD for 터미널 재시작 오케스트레이터: disposeSession → registry.destroy →
// bumpTerminalEpoch → setSessionState(starting) → createSession 순서 보장,
// 그리고 각 단계 실패 시의 폴백 동작(deleteAgent.test.ts의 목 패턴 참고).
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import { ensureSession } from "../../ipc/sessionBridge";
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

const flushAgent = vi.fn().mockResolvedValue(undefined);
vi.mock("../../diary/diaryFlusher", () => ({
  sharedDiaryFlusher: () => ({ flushAgent: (...args: unknown[]) => flushAgent(...args) }),
}));

const { restartAgentSession } = await import("../restartAgentSession");

function mkProfile(id: string, overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id,
    name: `Agent ${id}`,
    role: "eng",
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
  flushAgent.mockClear();
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
      'create:a1:{"agentName":"Agent a1","agentRole":"eng","cwd":"/work/a1"}',
      "status-at-create:starting",
      "epoch-at-create:1",
    ]);
    expect(useAppStore.getState().terminalEpochs.a1).toBe(1);
    expect(useAppStore.getState().sessions.a1.status).toBe("starting");
  });

  it("cwd 없는 에이전트도 프로필 스냅샷으로 createSession을 호출한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));

    await restartAgentSession("a1");

    expect(createSession).toHaveBeenCalledWith("a1", {
      agentName: "Agent a1",
      agentRole: "eng",
    });
  });

  it("shell이 설정된 에이전트는 createSession opts에 shell을 포함한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", { shell: "wsl" }));

    await restartAgentSession("a1");

    expect(createSession).toHaveBeenCalledWith("a1", {
      agentName: "Agent a1",
      agentRole: "eng",
      shell: "wsl",
    });
  });

  it("cwd와 shell이 모두 설정된 에이전트는 createSession opts에 둘 다 포함한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", { cwd: "/work/a1", shell: "wsl" }));

    await restartAgentSession("a1");

    expect(createSession).toHaveBeenCalledWith("a1", {
      agentName: "Agent a1",
      agentRole: "eng",
      cwd: "/work/a1",
      shell: "wsl",
    });
  });

  it("현재 폴더 override는 이번 재시작 createSession에만 프로필 cwd보다 우선한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", { cwd: "/profile/start" }));

    await restartAgentSession("a1", { cwd: "/observed/current" });

    expect(createSession).toHaveBeenCalledWith("a1", {
      agentName: "Agent a1",
      agentRole: "eng",
      cwd: "/observed/current",
    });
    expect(useAppStore.getState().agents.a1.cwd).toBe("/profile/start");
  });

  it("재시작 직후에는 이전 세션 라벨의 stale cwd를 실제 새 시작 cwd로 보정한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", { cwd: "/profile/start" }));
    useAppStore.setState({
      taskLabels: { a1: { sessionId: "old", cwd: "/old/session", goal: "작업 중" } },
    });

    await restartAgentSession("a1", { cwd: "/observed/current" });

    expect(useAppStore.getState().taskLabels.a1).toEqual({
      sessionId: "old",
      cwd: "/observed/current",
      goal: "작업 중",
    });
  });

  it("override 없는 재시작은 이전 세션 라벨 cwd를 프로필 시작 폴더로 보정한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", { cwd: "/profile/start" }));
    useAppStore.setState({ taskLabels: { a1: { sessionId: "old", cwd: "/old/session" } } });

    await restartAgentSession("a1");

    expect(useAppStore.getState().taskLabels.a1?.cwd).toBe("/profile/start");
  });

  it("disposeSession이 실패해도 재시작은 계속 진행된다", async () => {
    disposeSession.mockRejectedValueOnce(new Error("no such session"));
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));

    await restartAgentSession("a1");

    expect(destroy).toHaveBeenCalledWith("a1");
    expect(useAppStore.getState().terminalEpochs.a1).toBe(1);
    expect(createSession).toHaveBeenCalledWith("a1", {
      agentName: "Agent a1",
      agentRole: "eng",
    });
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

  it("옛 세션 일기 catch-up을 명시 flush로 트리거한다(#75, create 전·starting 상태)", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));

    const order: string[] = [];
    flushAgent.mockImplementationOnce((id: string) => {
      order.push(`flush:${id}:status=${useAppStore.getState().sessions.a1.status}`);
      return Promise.resolve();
    });
    createSession.mockImplementationOnce(async (id: string) => {
      order.push(`create:${id}`);
    });

    await restartAgentSession("a1");

    // flush가 createSession 전에, 그리고 status가 아직 running으로 안 바뀐
    // starting 상태에서 옛 세션을 대상으로(includeLive:false) 트리거된다.
    expect(flushAgent).toHaveBeenCalledWith("a1", { includeLive: false, source: "session-end" });
    expect(order).toEqual(["flush:a1:status=starting", "create:a1"]);
  });

  it("폐기 이벤트 뒤 클릭과 중복 재시작은 새 세션을 앞질러 만들지 않는다", async () => {
    useAppStore.getState().addAgent(mkProfile("a1", { cwd: "/profile" }));
    let finishDispose!: () => void;
    disposeSession.mockReturnValueOnce(new Promise<void>((resolve) => { finishDispose = resolve; }));
    const restarting = restartAgentSession("a1", { cwd: "/chosen" });
    useAppStore.getState().setSessionState({ agentId: "a1", status: "exited" });
    ensureSession("a1");
    await restartAgentSession("a1");
    expect(createSession).not.toHaveBeenCalled();
    expect(disposeSession).toHaveBeenCalledTimes(1);
    finishDispose();
    await restarting;
    expect(createSession).toHaveBeenCalledTimes(1);
    expect(createSession).toHaveBeenCalledWith("a1", expect.objectContaining({ cwd: "/chosen" }));
  });

  it("재시작은 중단된 자동화를 지우고 이전 상태 응답의 재유입을 막는다", async () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    const old = { running: false, phase: "cancelled" as const, cli: "claude" as const, filePath: "", startedAtMs: 1 };
    useAppStore.setState({ automation: { a1: old } });
    const epochs = useAppStore.getState().automationEpochs;
    disposeSession.mockImplementationOnce(async () => {
      expect(useAppStore.getState().automation.a1).toBeUndefined();
      useAppStore.getState().seedAutomationStatus({ agents: { a1: old } }, epochs);
    });
    await restartAgentSession("a1");
    expect(useAppStore.getState().automation.a1).toBeUndefined();
    expect(useAppStore.getState().automationEpochs.a1).toBe(1);
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
