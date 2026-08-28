// src/renderer/ipc/__tests__/sessionBridge.usage.test.ts
//
// 세션 사용량(터미널 요약 바, docs/session-analytics-design.md §11) 배선
// 검증: onSessionState→noteUsageSession, onNotification→applyNotificationUsage.
// sessionBridge.timeTracking.test.ts와 같은 결(콜백 캡처형 tauriApi 목업).
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const captured: {
  notif?: (e: any) => void;
  state?: (e: any) => void;
} = {};

vi.mock("../tauriApi", () => ({
  tauriApi: {
    onSessionState: (cb: any) => ((captured.state = cb), () => {}),
    onNotification: (cb: any) => ((captured.notif = cb), () => {}),
    onNotificationCleared: () => () => {},
    onActivity: () => () => {},
    onTalkMessage: () => () => {},
    setBadgeCount: vi.fn(),
    appendSessionTurn: vi.fn(),
  },
}));

import { installSessionBridge } from "../sessionBridge";
import { useAppStore } from "../../store/appStore";

const initial = useAppStore.getState();
let teardown: () => void;

beforeEach(() => {
  useAppStore.setState(initial, true);
  teardown = installSessionBridge();
});
afterEach(() => teardown());

describe("sessionBridge session-usage wiring", () => {
  it("session-state events feed noteUsageSession (sessionId 잡아두기)", () => {
    captured.state!({ agentId: "a1", sessionId: "s1", state: "running", at: 0 });
    expect(useAppStore.getState().sessionUsage["a1"]).toEqual({
      sessionId: "s1",
      totals: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, costUsd: 0, costUnknownTurns: 0, turns: 0 },
    });
  });

  it("session-state events reset totals when the sessionId changes", () => {
    captured.state!({ agentId: "a1", sessionId: "s1", state: "running", at: 0 });
    captured.notif!({
      id: "n1", sessionId: "s1", agentId: "a1", source: "stop",
      message: "done", dedupKey: "k1", at: 100,
      tokens: { input: 100, model: "claude-opus-5" },
    });
    expect(useAppStore.getState().sessionUsage["a1"].totals.turns).toBe(1);

    captured.state!({ agentId: "a1", sessionId: "s2", state: "running", at: 200 });
    expect(useAppStore.getState().sessionUsage["a1"]).toEqual({
      sessionId: "s2",
      totals: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, costUsd: 0, costUnknownTurns: 0, turns: 0 },
    });
  });

  it("notification events with tokens feed applyNotificationUsage", () => {
    captured.state!({ agentId: "a1", sessionId: "s1", state: "running", at: 0 });
    captured.notif!({
      id: "n1", sessionId: "s1", agentId: "a1", source: "stop",
      message: "done", dedupKey: "k1", at: 100,
      tokens: { input: 100, output: 50, model: "claude-opus-5" },
    });
    const entry = useAppStore.getState().sessionUsage["a1"];
    expect(entry.sessionId).toBe("s1");
    expect(entry.totals.input).toBe(100);
    expect(entry.totals.output).toBe(50);
    expect(entry.totals.turns).toBe(1);
  });

  it("notification events without tokens do not touch sessionUsage", () => {
    captured.state!({ agentId: "a1", sessionId: "s1", state: "running", at: 0 });
    const before = useAppStore.getState().sessionUsage;
    captured.notif!({
      id: "n1", sessionId: "s1", agentId: "a1", source: "hook",
      message: "?", dedupKey: "k1", at: 100,
    });
    expect(useAppStore.getState().sessionUsage).toBe(before);
  });
});
