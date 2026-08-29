// src/renderer/ipc/__tests__/sessionBridge.usage.test.ts
//
// 세션 사용량(터미널 요약 바, docs/session-analytics-design.md §11) 배선
// 검증: onSessionState→noteUsageSession, onTurnUsage→applyTurnUsage.
// turn-usage는 notification-new와 분리된 채널이다(결정 A) — 알림이 억제돼도
// 사용량 배선은 별도로 동작해야 하므로, 알림(notif) 콜백과 사용량(usage)
// 콜백을 각각 따로 캡처해 구분해서 검증한다.
// sessionBridge.timeTracking.test.ts와 같은 결(콜백 캡처형 tauriApi 목업).
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const captured: {
  notif?: (e: any) => void;
  state?: (e: any) => void;
  usage?: (e: any) => void;
} = {};

vi.mock("../tauriApi", () => ({
  tauriApi: {
    onSessionState: (cb: any) => ((captured.state = cb), () => {}),
    onNotification: (cb: any) => ((captured.notif = cb), () => {}),
    onNotificationCleared: () => () => {},
    onActivity: () => () => {},
    onTurnUsage: (cb: any) => ((captured.usage = cb), () => {}),
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
    captured.usage!({
      agentId: "a1", sessionId: "s1", at: 100,
      tokens: { input: 100, model: "claude-opus-5" },
    });
    expect(useAppStore.getState().sessionUsage["a1"].totals.turns).toBe(1);

    captured.state!({ agentId: "a1", sessionId: "s2", state: "running", at: 200 });
    expect(useAppStore.getState().sessionUsage["a1"]).toEqual({
      sessionId: "s2",
      totals: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, costUsd: 0, costUnknownTurns: 0, turns: 0 },
    });
  });

  it("turn-usage events feed applyTurnUsage", () => {
    captured.state!({ agentId: "a1", sessionId: "s1", state: "running", at: 0 });
    captured.usage!({
      agentId: "a1", sessionId: "s1", at: 100,
      tokens: { input: 100, output: 50, model: "claude-opus-5" },
    });
    const entry = useAppStore.getState().sessionUsage["a1"];
    expect(entry.sessionId).toBe("s1");
    expect(entry.totals.input).toBe(100);
    expect(entry.totals.output).toBe(50);
    expect(entry.totals.turns).toBe(1);
  });

  it("notification events alone (동반 turn-usage 없음) do not touch sessionUsage", () => {
    // 결정 A: 알림과 사용량은 분리된 채널이다 — notification-new만 와서는
    // sessionUsage가 전혀 바뀌지 않는다(turn-usage가 와야 바뀐다).
    captured.state!({ agentId: "a1", sessionId: "s1", state: "running", at: 0 });
    const before = useAppStore.getState().sessionUsage;
    captured.notif!({
      id: "n1", sessionId: "s1", agentId: "a1", source: "stop",
      message: "done", dedupKey: "k1", at: 100,
    });
    expect(useAppStore.getState().sessionUsage).toBe(before);
  });
});
