// src/renderer/agent/__tests__/resumeAgentSession.test.ts
//
// TDD for Claude 세션 이어하기 오케스트레이터: sessionId 형식 검증,
// disposeSession → registry.destroy → bumpTerminalEpoch → starting →
// createSession(startupCommand override) 순서, 실행 중 세션이 없어도 동작.
// restartAgentSession.test.ts의 목 패턴을 따른다.
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

const { resumeAgentSession, buildResumeStartupCommand } = await import("../resumeAgentSession");

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

describe("buildResumeStartupCommand (sessionId 형식 검증)", () => {
  it("UUID류 형식이면 claude --resume 명령을 만든다", () => {
    expect(buildResumeStartupCommand("2f8c1a20-0000-4b0e-9c3a-abcdef012345")).toBe(
      "claude --resume 2f8c1a20-0000-4b0e-9c3a-abcdef012345",
    );
  });

  it("16진수+하이픈만이면 허용한다(느슨한 UUID)", () => {
    expect(buildResumeStartupCommand("abc-123-DEF")).toBe("claude --resume abc-123-DEF");
  });

  it("허용되지 않는 문자가 섞이면 null을 반환한다(셸 주입 방지)", () => {
    expect(buildResumeStartupCommand("abc; rm -rf /")).toBeNull();
    expect(buildResumeStartupCommand("$(whoami)")).toBeNull();
    expect(buildResumeStartupCommand("ghijk")).toBeNull(); // 16진수 아님
    expect(buildResumeStartupCommand("")).toBeNull();
  });
});

describe("resumeAgentSession 오케스트레이션", () => {
  it("dispose → destroy → bumpTerminalEpoch → starting → createSession(resume override) 순서로 진행한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", { cwd: "/work/a1", startupCommand: "claude" }));

    const order: string[] = [];
    disposeSession.mockImplementationOnce(async (id: string) => {
      order.push(`dispose:${id}`);
    });
    destroy.mockImplementationOnce((id: string) => order.push(`destroy:${id}`));
    createSession.mockImplementationOnce(async (id: string, opts: unknown) => {
      order.push(`create:${id}:${JSON.stringify(opts)}`);
      order.push(`status-at-create:${useAppStore.getState().sessions.a1.status}`);
      order.push(`epoch-at-create:${useAppStore.getState().terminalEpochs.a1}`);
    });

    await resumeAgentSession("a1", "abc-123");

    expect(order).toEqual([
      "dispose:a1",
      "destroy:a1",
      'create:a1:{"agentName":"Agent a1","agentRole":"eng","cwd":"/work/a1","startupCommand":"claude --resume abc-123"}',
      "status-at-create:starting",
      "epoch-at-create:1",
    ]);
    expect(useAppStore.getState().sessions.a1.status).toBe("starting");
  });

  it("프로필의 startupCommand를 resume 명령으로 대체한다(cwd/shell 유지)", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", { cwd: "/work/a1", shell: "zsh", startupCommand: "claude" }));

    await resumeAgentSession("a1", "abc-123");

    expect(createSession).toHaveBeenCalledWith("a1", {
      agentName: "Agent a1",
      agentRole: "eng",
      cwd: "/work/a1",
      shell: "zsh",
      startupCommand: "claude --resume abc-123",
    });
  });

  it("실행 중 세션이 없어도(exited) 이어하기가 동작한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.setSessionState({ agentId: "a1", status: "exited" });

    await resumeAgentSession("a1", "abc-123");

    expect(disposeSession).toHaveBeenCalledWith("a1");
    expect(destroy).toHaveBeenCalledWith("a1");
    expect(createSession).toHaveBeenCalledWith("a1", {
      agentName: "Agent a1",
      agentRole: "eng",
      startupCommand: "claude --resume abc-123",
    });
  });

  it("sessionId 형식이 유효하지 않으면 기존 세션을 건드리지 않고 거부한다", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));

    await resumeAgentSession("a1", "abc; rm -rf /");

    expect(disposeSession).not.toHaveBeenCalled();
    expect(destroy).not.toHaveBeenCalled();
    expect(createSession).not.toHaveBeenCalled();
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  it("disposeSession이 실패해도 이어하기는 계속 진행된다", async () => {
    disposeSession.mockRejectedValueOnce(new Error("no such session"));
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));

    await resumeAgentSession("a1", "abc-123");

    expect(destroy).toHaveBeenCalledWith("a1");
    expect(useAppStore.getState().terminalEpochs.a1).toBe(1);
    expect(createSession).toHaveBeenCalledWith("a1", {
      agentName: "Agent a1",
      agentRole: "eng",
      startupCommand: "claude --resume abc-123",
    });
  });

  it("createSession이 실패하면 상태가 exited로 바뀐다", async () => {
    createSession.mockRejectedValueOnce(new Error("spawn failed"));
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));

    await resumeAgentSession("a1", "abc-123");

    expect(useAppStore.getState().sessions.a1.status).toBe("exited");
  });
});
