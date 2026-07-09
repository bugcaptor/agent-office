// src/renderer/agent/__tests__/clockOut.test.ts
//
// clockOutAgent/clockOutAll/clockInAgent TDD, mirroring
// `deleteAgent.test.ts`'s mocking approach: `tauriApi`/`TerminalRegistry`
// are module-mocked (real IPC / real xterm DOM are out of scope here), and
// `officeBus` (sessionBridge) is mocked so `clockInAgent`'s "bring the
// terminal up" delegation can be asserted without pulling in the real
// session bridge wiring. The store itself is the real `useAppStore`, so the
// clockOut/clockIn store-action cascade (covered separately in
// appStore.test.ts) is exercised end-to-end here too.
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

const emitAgentClicked = vi.fn();
vi.mock("../../ipc/sessionBridge", () => ({
  officeBus: {
    emitAgentClicked: (...args: unknown[]) => emitAgentClicked(...args),
  },
}));

const { clockOutAgent, clockOutAll, clockInAgent, clockInAll } = await import("../clockOut");

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
  disposeSession.mockResolvedValue(undefined);
  createSession.mockClear();
  createSession.mockResolvedValue(undefined);
  destroy.mockClear();
  emitAgentClicked.mockClear();
});

describe("clockOutAgent", () => {
  it("disposeSession → store.clockOut → terminalRegistry.destroy 순서로 실행한다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.openTerminal("a1");

    await clockOutAgent("a1");

    expect(disposeSession).toHaveBeenCalledWith("a1");
    const st = useAppStore.getState();
    expect(st.agents.a1.clockedOut).toBe(true);
    expect(st.agents.a1).toBeDefined(); // 프로필은 삭제되지 않는다
    expect(st.sessions.a1).toBeUndefined();
    expect(st.activeTerminalAgentId).toBeNull();
    expect(destroy).toHaveBeenCalledWith("a1");
  });

  it("disposeSession이 실패해도 퇴근 처리는 계속 진행된다", async () => {
    disposeSession.mockRejectedValueOnce(new Error("no such session"));
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));

    await clockOutAgent("a1");

    expect(useAppStore.getState().agents.a1.clockedOut).toBe(true);
    expect(destroy).toHaveBeenCalledWith("a1");
  });
});

describe("clockOutAll", () => {
  it("근무 중인 에이전트를 전부 퇴근시키고, 이미 퇴근한 에이전트는 건너뛴다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.addAgent(mkProfile("a2"));
    s.addAgent(mkProfile("a3"));
    s.clockOut("a2"); // 이미 퇴근 상태

    await clockOutAll();

    const st = useAppStore.getState();
    expect(st.agents.a1.clockedOut).toBe(true);
    expect(st.agents.a2.clockedOut).toBe(true);
    expect(st.agents.a3.clockedOut).toBe(true);
    // a2는 애초에 근무 중이 아니었으므로 disposeSession이 다시 호출되지 않는다.
    expect(disposeSession).toHaveBeenCalledWith("a1");
    expect(disposeSession).toHaveBeenCalledWith("a3");
    expect(disposeSession).not.toHaveBeenCalledWith("a2");
  });

  it("근무 중인 에이전트가 없으면 아무 것도 하지 않는다", async () => {
    await clockOutAll();
    expect(disposeSession).not.toHaveBeenCalled();
  });
});

describe("clockInAgent", () => {
  it("clockedOut 플래그를 해제한 뒤 officeBus.emitAgentClicked를 호출한다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.clockOut("a1");
    expect(useAppStore.getState().agents.a1.clockedOut).toBe(true);

    clockInAgent("a1");

    expect(useAppStore.getState().agents.a1.clockedOut).toBeUndefined();
    expect(emitAgentClicked).toHaveBeenCalledWith("a1");
  });

  it("PTY를 직접 생성한다(createSession) — clockIn이 starting을 선점해 ensureSession이 스킵되므로", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.clockOut("a1");

    clockInAgent("a1");

    expect(createSession.mock.calls.map((c) => c[0])).toContain("a1");
  });

  it("터미널 에폭을 올려 TerminalMount를 강제 리마운트시킨다(재소환 시 빈 화면 방지)", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.clockOut("a1");
    const before = useAppStore.getState().terminalEpochs.a1 ?? 0;

    clockInAgent("a1");

    const after = useAppStore.getState().terminalEpochs.a1 ?? 0;
    expect(after).toBeGreaterThan(before);
    expect(useAppStore.getState().agents.a1.clockedOut).toBeUndefined();
  });
});

describe("clockInAll", () => {
  it("퇴근한 에이전트만 전부 출근시키고, 근무 중인 에이전트는 건드리지 않는다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.addAgent(mkProfile("a2"));
    s.addAgent(mkProfile("a3"));
    s.clockOut("a1");
    s.clockOut("a3"); // a2만 근무 중

    clockInAll();

    const st = useAppStore.getState();
    expect(st.agents.a1.clockedOut).toBeUndefined();
    expect(st.agents.a3.clockedOut).toBeUndefined();
    expect(st.agents.a2.clockedOut).toBeUndefined(); // 원래 근무 중이었음
    const started = createSession.mock.calls.map((c) => c[0]);
    expect(started).toContain("a1");
    expect(started).toContain("a3");
    expect(started).not.toContain("a2");
  });

  it("퇴근한 에이전트가 없으면 아무 것도 하지 않는다", () => {
    useAppStore.getState().addAgent(mkProfile("a1")); // 근무 중
    clockInAll();
    expect(createSession).not.toHaveBeenCalled();
  });
});
