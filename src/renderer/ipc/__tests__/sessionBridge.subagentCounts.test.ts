// src/renderer/ipc/__tests__/sessionBridge.subagentCounts.test.ts
//
// Glue-seam integration test for `installSessionBridge()`'s subagent-count
// wiring. `subagentCounts` (a `SubagentCountTracker`) is module-private to
// `sessionBridge.ts` — the only external observation point is
// `officeBus.onSubagentCountChanged`. This test drives the three wire events
// that touch it (onActivity sub-start/sub-stop/sub-count, onSessionState
// exited/disposed) through captured callbacks
// and asserts on the counts relayed through that seam, so a future typo in
// any of the literal wire strings or a sign inversion in the bump delta
// fails this test.
//
// Mocking pattern follows sessionBridge.timeTracking.test.ts: `../tauriApi`
// is mocked so each `on*` registration captures its callback for the test to
// invoke manually; the other tauriApi methods used by installSessionBridge
// (setBadgeCount, appendSessionTurn, onNotificationCleared) are no-op stubs
// so the store subscriptions never throw.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ActivityEvent, NotificationEvent, SessionStateEvent } from "@shared/types";

const captured: {
  activity?: (e: ActivityEvent) => void;
  notif?: (e: NotificationEvent) => void;
  state?: (e: SessionStateEvent) => void;
} = {};

vi.mock("../tauriApi", () => ({
  tauriApi: {
    onSessionState: (cb: (e: SessionStateEvent) => void) => ((captured.state = cb), () => {}),
    onNotification: (cb: (e: NotificationEvent) => void) => ((captured.notif = cb), () => {}),
    onNotificationCleared: () => () => {},
    onActivity: (cb: (e: ActivityEvent) => void) => ((captured.activity = cb), () => {}),
    setBadgeCount: vi.fn(),
    appendSessionTurn: vi.fn(),
    clearNotifications: vi.fn(),
  },
}));

import { installSessionBridge, officeBus } from "../sessionBridge";
import { useAppStore } from "../../store/appStore";

const initial = useAppStore.getState();
let teardown: () => void;

function mkActivity(agentId: string, kind: ActivityEvent["kind"], count?: number): ActivityEvent {
  return { agentId, sessionId: "s1", kind, at: Date.now(), count };
}

function mkState(agentId: string, state: SessionStateEvent["state"]): SessionStateEvent {
  return { sessionId: "s1", agentId, state, at: Date.now() };
}

function mkNotif(agentId: string, source: NotificationEvent["source"]): NotificationEvent {
  return {
    id: `n-${agentId}-${source}-${Math.random()}`,
    sessionId: "s1",
    agentId,
    source,
    message: "x",
    dedupKey: `k-${agentId}-${source}`,
    at: Date.now(),
  };
}

beforeEach(() => {
  useAppStore.setState(initial, true);
  teardown = installSessionBridge();
});

afterEach(() => teardown());

describe("sessionBridge subagent-count glue seam", () => {
  it("sub-start bumps +1 per event, observed via officeBus.onSubagentCountChanged", () => {
    const spy = vi.fn();
    officeBus.onSubagentCountChanged(spy);

    captured.activity!(mkActivity("a1", "sub-start"));
    expect(spy).toHaveBeenLastCalledWith("a1", 1);

    captured.activity!(mkActivity("a1", "sub-start"));
    expect(spy).toHaveBeenLastCalledWith("a1", 2);
  });

  it("sub-stop bumps -1", () => {
    const spy = vi.fn();
    officeBus.onSubagentCountChanged(spy);

    captured.activity!(mkActivity("a2", "sub-start"));
    captured.activity!(mkActivity("a2", "sub-start"));
    spy.mockClear();

    captured.activity!(mkActivity("a2", "sub-stop"));
    expect(spy).toHaveBeenLastCalledWith("a2", 1);
  });

  it("clamps at 0 and never goes negative — stops notifying once at the floor", () => {
    const spy = vi.fn();
    officeBus.onSubagentCountChanged(spy);

    captured.activity!(mkActivity("a3", "sub-start")); // -> 1
    spy.mockClear();

    captured.activity!(mkActivity("a3", "sub-stop")); // 1 -> 0, notifies
    expect(spy).toHaveBeenLastCalledWith("a3", 0);
    spy.mockClear();

    // Further sub-stop at floor must NOT re-notify (no change).
    captured.activity!(mkActivity("a3", "sub-stop"));
    captured.activity!(mkActivity("a3", "sub-stop"));
    expect(spy).not.toHaveBeenCalled();
  });

  it("session state exited resets the count to 0", () => {
    const spy = vi.fn();
    officeBus.onSubagentCountChanged(spy);

    captured.activity!(mkActivity("a4", "sub-start"));
    captured.activity!(mkActivity("a4", "sub-start"));
    spy.mockClear();

    captured.state!(mkState("a4", "exited"));
    expect(spy).toHaveBeenLastCalledWith("a4", 0);
  });

  it("session state disposed also resets the count to 0", () => {
    const spy = vi.fn();
    officeBus.onSubagentCountChanged(spy);

    captured.activity!(mkActivity("a5", "sub-start"));
    spy.mockClear();

    captured.state!(mkState("a5", "disposed"));
    expect(spy).toHaveBeenLastCalledWith("a5", 0);
  });

  it("a notification with source=stop no longer touches the count", () => {
    const spy = vi.fn();
    officeBus.onSubagentCountChanged(spy);

    captured.activity!(mkActivity("a6", "sub-start"));
    captured.activity!(mkActivity("a6", "sub-start"));
    spy.mockClear();

    captured.notif!(mkNotif("a6", "stop"));
    expect(spy).not.toHaveBeenCalled();
  });

  it("sub-count sets the absolute count up and down", () => {
    const spy = vi.fn();
    officeBus.onSubagentCountChanged(spy);

    captured.activity!(mkActivity("a8", "sub-count", 4));
    expect(spy).toHaveBeenLastCalledWith("a8", 4);
    captured.activity!(mkActivity("a8", "sub-count", 1));
    expect(spy).toHaveBeenLastCalledWith("a8", 1);
  });

  it("sub-count without count defaults to 0", () => {
    const spy = vi.fn();
    officeBus.onSubagentCountChanged(spy);

    captured.activity!(mkActivity("a9", "sub-start"));
    spy.mockClear();
    captured.activity!(mkActivity("a9", "sub-count"));
    expect(spy).toHaveBeenLastCalledWith("a9", 0);
  });

  it("sub-start increments from the latest absolute count", () => {
    const spy = vi.fn();
    officeBus.onSubagentCountChanged(spy);

    captured.activity!(mkActivity("a10", "sub-count", 2));
    spy.mockClear();
    captured.activity!(mkActivity("a10", "sub-start"));
    expect(spy).toHaveBeenLastCalledWith("a10", 3);
  });

  it("prompt/tool activity events do not touch the subagent count", () => {
    const spy = vi.fn();
    officeBus.onSubagentCountChanged(spy);
    spy.mockClear();

    captured.activity!(mkActivity("a7", "prompt"));
    captured.activity!(mkActivity("a7", "tool"));

    expect(spy).not.toHaveBeenCalled();
  });
});
