// src/renderer/ipc/__tests__/sessionBridge.test.ts
//
// TDD for the IPC <-> store bridge and the
// store-backed `OfficeBus` implementation. `../tauriApi` is mocked in full
// (satisfying `AgentOfficeApi`) so the bridge's event routing can be driven
// without any real Tauri runtime.
//
// Coverage:
// - onNotification -> store gains the notification; officeBus relays
//   hasPending=true to onNotificationChanged listeners; badge count synced.
// - onNotificationCleared -> store drops it; hasPending flips back to false;
//   badge count synced back down.
// - onSessionState -> store's session status is set from the wire `state`
//   (SessionState, translated into SessionStatus with no lossy cast); the
//   same `state` is relayed verbatim to officeBus's onSessionStateChanged
//   listeners.
// - Badge sync respects `muted`.
// - emitAgentClicked -> store.openTerminal(agentId) + tauriApi.clearNotifications(agentId).
// - installSessionBridge()'s cleanup detaches the store subscription (no
//   more badge syncing after teardown).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  NotificationClearedEvent,
  NotificationEvent,
  SessionStateEvent,
} from "@shared/types";

const { mockApi, capture } = vi.hoisted(() => {
  const capture: {
    onSessionState?: (e: SessionStateEvent) => void;
    onNotification?: (e: NotificationEvent) => void;
    onNotificationCleared?: (e: NotificationClearedEvent) => void;
    onActivity?: (e: unknown) => void;
  } = {};
  const mockApi = {
    createSession: vi.fn(),
    disposeSession: vi.fn(),
    writeInput: vi.fn(),
    resize: vi.fn(),
    clearNotifications: vi.fn(),
    listNotifications: vi.fn(),
    loadState: vi.fn(),
    saveState: vi.fn(),
    setBadgeCount: vi.fn(),
    onData: vi.fn(),
    onSessionState: vi.fn((cb: (e: SessionStateEvent) => void) => {
      capture.onSessionState = cb;
      return vi.fn();
    }),
    onNotification: vi.fn((cb: (e: NotificationEvent) => void) => {
      capture.onNotification = cb;
      return vi.fn();
    }),
    onNotificationCleared: vi.fn((cb: (e: NotificationClearedEvent) => void) => {
      capture.onNotificationCleared = cb;
      return vi.fn();
    }),
    onActivity: vi.fn((cb: (e: unknown) => void) => {
      capture.onActivity = cb;
      return vi.fn();
    }),
  };
  return { mockApi, capture };
});

vi.mock("../tauriApi", () => ({ tauriApi: mockApi }));

import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";
import { installSessionBridge, officeBus } from "../sessionBridge";

const initialState = useAppStore.getState();

function mkProfile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: "a1",
    name: "Test Agent",
    role: "backend",
    note: "",
    seed: "seed-a1",
    createdAt: Date.now(),
    deskIndex: 0,
    ...overrides,
  };
}

function mkNotifEvent(overrides: Partial<NotificationEvent> = {}): NotificationEvent {
  return {
    id: "n1",
    sessionId: "s1",
    agentId: "a1",
    source: "bell",
    message: "hello",
    dedupKey: "dedup-1",
    at: Date.now(),
    ...overrides,
  };
}

let cleanup: () => void;

beforeEach(() => {
  useAppStore.setState(initialState, true);
  Object.values(mockApi).forEach((fn) => fn.mockClear());
  capture.onSessionState = undefined;
  capture.onNotification = undefined;
  capture.onNotificationCleared = undefined;
  capture.onActivity = undefined;
  cleanup = installSessionBridge();
});

afterEach(() => {
  cleanup();
});

describe("installSessionBridge / onNotification", () => {
  it("adds the notification to the store", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));

    capture.onNotification?.(mkNotifEvent({ agentId: "a1" }));

    const notifications = useAppStore.getState().notifications;
    expect(notifications).toHaveLength(1);
    expect(notifications[0].id).toBe("n1");
    expect(notifications[0].agentId).toBe("a1");
  });

  it("relays hasPending=true to officeBus.onNotificationChanged listeners", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    const listener = vi.fn();
    officeBus.onNotificationChanged(listener);

    capture.onNotification?.(mkNotifEvent({ agentId: "a1" }));

    expect(listener).toHaveBeenCalledWith("a1", true);
  });

  it("syncs the dock badge count", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));

    capture.onNotification?.(mkNotifEvent({ agentId: "a1" }));

    expect(mockApi.setBadgeCount).toHaveBeenCalledWith(1);
  });

  it("does not touch the badge while muted", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    useAppStore.setState({ muted: true });
    mockApi.setBadgeCount.mockClear();

    capture.onNotification?.(mkNotifEvent({ agentId: "a1" }));

    expect(mockApi.setBadgeCount).not.toHaveBeenCalled();
  });
});

describe("installSessionBridge / onNotificationCleared", () => {
  it("removes the notification from the store and flips hasPending back to false", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    capture.onNotification?.(mkNotifEvent({ agentId: "a1", id: "n1" }));
    const listener = vi.fn();
    officeBus.onNotificationChanged(listener);

    capture.onNotificationCleared?.({ agentId: "a1", ids: ["n1"] });

    expect(useAppStore.getState().notifications).toHaveLength(0);
    expect(listener).toHaveBeenCalledWith("a1", false);
    expect(mockApi.setBadgeCount).toHaveBeenLastCalledWith(0);
  });
});

describe("installSessionBridge / onSessionState", () => {
  it("translates the wire event into the store's session status", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));

    capture.onSessionState?.({
      sessionId: "s1",
      agentId: "a1",
      state: "running",
      at: Date.now(),
    });

    expect(useAppStore.getState().sessions.a1.status).toBe("running");
  });

  it("relays the raw wire state to officeBus.onSessionStateChanged listeners", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    const listener = vi.fn();
    officeBus.onSessionStateChanged(listener);

    capture.onSessionState?.({
      sessionId: "s1",
      agentId: "a1",
      state: "exited",
      at: Date.now(),
    });

    expect(listener).toHaveBeenCalledWith("a1", "exited");
  });
});

describe("officeBus.emitAgentClicked", () => {
  it("opens the terminal in the store and clears backend notifications for that agent", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));

    officeBus.emitAgentClicked("a1");

    expect(useAppStore.getState().activeTerminalAgentId).toBe("a1");
    expect(mockApi.clearNotifications).toHaveBeenCalledWith("a1");
  });
});

// Clicking a character with no live PTY (idle after hydrate, or exited)
// must recreate its session exactly once, and surface start failures.
describe("officeBus.emitAgentClicked / ensureSession (Fix 1b)", () => {
  const flush = () => new Promise((r) => setTimeout(r, 0));

  it("starts a session for an idle (persisted) agent and marks it starting", async () => {
    useAppStore.getState().hydrate({ agents: [mkProfile({ id: "a1" })], version: 1 });
    expect(useAppStore.getState().sessions.a1.status).toBe("idle");
    mockApi.createSession.mockResolvedValueOnce({ sessionId: "s1", state: "starting" });

    officeBus.emitAgentClicked("a1");

    expect(mockApi.createSession).toHaveBeenCalledTimes(1);
    expect(mockApi.createSession).toHaveBeenCalledWith("a1", undefined);
    expect(useAppStore.getState().sessions.a1.status).toBe("starting");
    await flush();
  });

  it("passes the profile's cwd from the store when starting a session (Task 3)", async () => {
    useAppStore
      .getState()
      .hydrate({ agents: [mkProfile({ id: "a1", cwd: "/tmp/proj" })], version: 1 });
    mockApi.createSession.mockResolvedValueOnce({ sessionId: "s1", state: "starting" });

    officeBus.emitAgentClicked("a1");

    expect(mockApi.createSession).toHaveBeenCalledWith("a1", { cwd: "/tmp/proj" });
    await flush();
  });

  it("does not start a session for an already-running agent", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    useAppStore.getState().setSessionState({ agentId: "a1", status: "running" });

    officeBus.emitAgentClicked("a1");

    expect(mockApi.createSession).not.toHaveBeenCalled();
  });

  it("creates only ONE session even on a rapid double click", async () => {
    useAppStore.getState().hydrate({ agents: [mkProfile({ id: "a1" })], version: 1 });
    mockApi.createSession.mockResolvedValue({ sessionId: "s1", state: "starting" });

    officeBus.emitAgentClicked("a1");
    officeBus.emitAgentClicked("a1");

    expect(mockApi.createSession).toHaveBeenCalledTimes(1);
    await flush();
  });

  it("recreates a session for an exited agent", async () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    useAppStore.getState().setSessionState({ agentId: "a1", status: "exited" });
    mockApi.createSession.mockResolvedValueOnce({ sessionId: "s2", state: "starting" });

    officeBus.emitAgentClicked("a1");

    expect(mockApi.createSession).toHaveBeenCalledTimes(1);
    expect(useAppStore.getState().sessions.a1.status).toBe("starting");
    await flush();
  });

  it("flips status back to exited and warns when createSession rejects", async () => {
    useAppStore.getState().hydrate({ agents: [mkProfile({ id: "a1" })], version: 1 });
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    mockApi.createSession.mockRejectedValueOnce(new Error("boom"));

    officeBus.emitAgentClicked("a1");

    await vi.waitFor(() =>
      expect(useAppStore.getState().sessions.a1.status).toBe("exited")
    );
    expect(warn).toHaveBeenCalled();

    // The failed start must be retryable: a subsequent click starts again.
    mockApi.createSession.mockResolvedValueOnce({ sessionId: "s3", state: "starting" });
    officeBus.emitAgentClicked("a1");
    expect(mockApi.createSession).toHaveBeenCalledTimes(2);

    warn.mockRestore();
    await flush();
  });
});

describe("installSessionBridge / mute toggle (Task 4G)", () => {
  it("forces the badge to 0 the instant muted flips on, even with pending notifications", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    capture.onNotification?.(mkNotifEvent({ agentId: "a1" }));
    mockApi.setBadgeCount.mockClear();

    useAppStore.getState().toggleMuted();

    expect(useAppStore.getState().muted).toBe(true);
    expect(mockApi.setBadgeCount).toHaveBeenCalledWith(0);
  });

  it("resyncs the badge to the current pending count the instant muted flips off", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    useAppStore.getState().addAgent(mkProfile({ id: "a2" }));
    useAppStore.setState({ muted: true });
    // Arrives while muted: store gains it, but the badge stays untouched (existing behavior).
    capture.onNotification?.(mkNotifEvent({ agentId: "a1", id: "n1" }));
    mockApi.setBadgeCount.mockClear();

    useAppStore.getState().toggleMuted();

    expect(useAppStore.getState().muted).toBe(false);
    expect(mockApi.setBadgeCount).toHaveBeenCalledWith(1);
  });
});

describe("installSessionBridge cleanup", () => {
  it("detaches the store subscription so the badge no longer syncs", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    cleanup();
    mockApi.setBadgeCount.mockClear();

    useAppStore.getState().pushNotification(mkNotifEvent({ agentId: "a1" }));

    expect(mockApi.setBadgeCount).not.toHaveBeenCalled();
  });

  it("also detaches the mute-toggle subscription", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    cleanup();
    mockApi.setBadgeCount.mockClear();

    useAppStore.getState().toggleMuted();

    expect(mockApi.setBadgeCount).not.toHaveBeenCalled();
  });
});
