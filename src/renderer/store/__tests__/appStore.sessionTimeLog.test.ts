// src/renderer/store/__tests__/appStore.sessionTimeLog.test.ts
//
// 턴 정산(settle) 시점에 tauriApi.appendSessionTurn이 정확한 기록으로
// 정확히 1번 호출되는지 검증한다. 턴이 열리기만 할 때(단일 prompt)나
// idle no-op에서는 호출되지 않아야 한다.
import { beforeEach, describe, expect, it, vi } from "vitest";

const { appendSessionTurnMock } = vi.hoisted(() => ({
  appendSessionTurnMock: vi.fn(),
}));
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: { appendSessionTurn: appendSessionTurnMock },
}));

import { useAppStore } from "../appStore";

const initialState = useAppStore.getState();

beforeEach(() => {
  appendSessionTurnMock.mockClear();
  useAppStore.setState(initialState, true);
});

describe("appStore session-time logging", () => {
  it("prompt → stop settles one turn and appends exactly one record", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 1000 });
    s.applyNotificationTiming({
      id: "n1", sessionId: "s1", agentId: "a1", source: "stop",
      message: "done", dedupKey: "k", at: 4000,
    });

    expect(appendSessionTurnMock).toHaveBeenCalledTimes(1);
    const record = appendSessionTurnMock.mock.calls[0][0];
    expect(record.agentId).toBe("a1");
    expect(record.startedAt).toBe(1000);
    expect(record.endedAt).toBe(4000);
    expect(record.totalMs).toBeGreaterThanOrEqual(0);
    expect(record.workedMs).toBeGreaterThanOrEqual(0);
    expect(record.waitedMs).toBeGreaterThanOrEqual(0);
    expect(record.totalMs).toBe(3000);
  });

  it("consecutive prompts settle the previous turn and append exactly one record", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 0 });
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 2500 });

    expect(appendSessionTurnMock).toHaveBeenCalledTimes(1);
    const record = appendSessionTurnMock.mock.calls[0][0];
    expect(record.agentId).toBe("a1");
    expect(record.startedAt).toBe(0);
    expect(record.endedAt).toBe(2500);
    expect(record.totalMs).toBe(2500);
  });

  it("does NOT log when a turn merely opens (single prompt from idle)", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 1000 });

    expect(appendSessionTurnMock).not.toHaveBeenCalled();
  });

  it("does NOT log on an idle no-op (tool event with no open turn)", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "tool", at: 1000 });

    expect(appendSessionTurnMock).not.toHaveBeenCalled();
  });

  it("applySessionTiming(exited) force-settles and logs; running is ignored", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 0 });
    s.applySessionTiming("a1", "running", 2000);
    expect(appendSessionTurnMock).not.toHaveBeenCalled();

    s.applySessionTiming("a1", "exited", 5000);
    expect(appendSessionTurnMock).toHaveBeenCalledTimes(1);
    const record = appendSessionTurnMock.mock.calls[0][0];
    expect(record.agentId).toBe("a1");
    expect(record.startedAt).toBe(0);
    expect(record.endedAt).toBe(5000);
  });
});
