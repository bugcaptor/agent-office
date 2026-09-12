// src/renderer/store/__tests__/persist.test.ts
//
// Tests for `installPersistence`.
//
// Coverage:
// - Multiple agent-profile changes within the 500ms debounce window collapse
//   into a single `tauriApi.saveState` call, carrying the *latest* state.
// - Purely-runtime noise (notifications, session state) never triggers a
//   save, even across the debounce window.
// - `installPersistence()`'s cleanup unsubscribes and cancels any still-
//   pending debounced save.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { NotificationEvent } from "@shared/types";

const { mockApi } = vi.hoisted(() => ({
  mockApi: {
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
    onSessionState: vi.fn(),
    onNotification: vi.fn(),
    onNotificationCleared: vi.fn(),
  },
}));

vi.mock("../../ipc/tauriApi", () => ({ tauriApi: mockApi }));

import { useAppStore } from "../appStore";
import { flushPersistence, installPersistence } from "../persist";
import type { AgentProfile } from "../types";

const initialState = useAppStore.getState();

function mkProfile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: "a1",
    name: "Test Agent",
    role: "backend",
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
  vi.useFakeTimers();
  useAppStore.setState(initialState, true);
  Object.values(mockApi).forEach((fn) => fn.mockClear());
  cleanup = installPersistence();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("installPersistence / debounce", () => {
  it("collapses multiple agent changes within 500ms into a single saveState call", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    vi.advanceTimersByTime(200);
    useAppStore.getState().addAgent(mkProfile({ id: "a2", name: "Second" }));
    vi.advanceTimersByTime(200);
    useAppStore.getState().updateAgent("a2", { name: "Renamed" });

    expect(mockApi.saveState).not.toHaveBeenCalled();

    vi.advanceTimersByTime(500);

    expect(mockApi.saveState).toHaveBeenCalledTimes(1);
    expect(mockApi.saveState).toHaveBeenCalledWith({
      agents: [
        expect.objectContaining({ id: "a1" }),
        expect.objectContaining({ id: "a2", name: "Renamed" }),
      ],
      version: 1,
      vacationMode: false,
    });
  });

  it("saves again after a quiet period, debouncing each burst separately", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    vi.advanceTimersByTime(500);
    expect(mockApi.saveState).toHaveBeenCalledTimes(1);

    useAppStore.getState().addAgent(mkProfile({ id: "a2" }));
    vi.advanceTimersByTime(500);
    expect(mockApi.saveState).toHaveBeenCalledTimes(2);
  });
});

describe("installPersistence / vacationMode", () => {
  it("includes vacationMode in the saveState payload after a debounced toggle", () => {
    useAppStore.getState().toggleVacationMode();

    expect(mockApi.saveState).not.toHaveBeenCalled();

    vi.advanceTimersByTime(500);

    expect(mockApi.saveState).toHaveBeenCalledTimes(1);
    expect(mockApi.saveState).toHaveBeenCalledWith(
      expect.objectContaining({ vacationMode: true })
    );
  });
});

describe("installPersistence / noise filtering", () => {
  it("does not save on notification-only changes", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    vi.advanceTimersByTime(500);
    mockApi.saveState.mockClear();

    useAppStore.getState().pushNotification(mkNotifEvent({ agentId: "a1" }));
    vi.advanceTimersByTime(500);

    expect(mockApi.saveState).not.toHaveBeenCalled();
  });

  it("does not save on session-state-only changes", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    vi.advanceTimersByTime(500);
    mockApi.saveState.mockClear();

    useAppStore.getState().setSessionState({ agentId: "a1", status: "running" });
    vi.advanceTimersByTime(500);

    expect(mockApi.saveState).not.toHaveBeenCalled();
  });
});

describe("installPersistence cleanup", () => {
  it("cancels a still-pending debounced save", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    cleanup();

    vi.advanceTimersByTime(1000);

    expect(mockApi.saveState).not.toHaveBeenCalled();
  });

  it("stops reacting to further agent changes", () => {
    cleanup();
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));

    vi.advanceTimersByTime(1000);

    expect(mockApi.saveState).not.toHaveBeenCalled();
  });
});

describe("installPersistence / connection flush", () => {
  it("serializes a newer explicit flush behind an in-flight older write", async () => {
    let finishFirst!: () => void;
    mockApi.saveState
      .mockImplementationOnce(() => new Promise<void>((resolve) => { finishFirst = resolve; }))
      .mockResolvedValueOnce(undefined);
    useAppStore.getState().addAgent(mkProfile({ name: "old" }));
    vi.advanceTimersByTime(500);
    useAppStore.getState().updateAgent("a1", { name: "new" });
    const flush = flushPersistence();
    expect(mockApi.saveState).toHaveBeenCalledTimes(1);
    finishFirst();
    await flush;
    expect(mockApi.saveState).toHaveBeenCalledTimes(2);
    expect(mockApi.saveState.mock.calls[1][0]).toEqual(expect.objectContaining({
      agents: [expect.objectContaining({ name: "new" })],
    }));
  });
});
