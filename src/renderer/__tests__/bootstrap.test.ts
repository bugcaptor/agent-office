// src/renderer/__tests__/bootstrap.test.ts
//
// TDD for `bootApp`'s boot sequence,
// integration-level over the real store/`installSessionBridge`/
// `installPersistence` (only `tauriApi` is mocked, same convention as
// `ipc/__tests__/sessionBridge.test.ts` and `store/__tests__/persist.test.ts`).
//
// Coverage:
// - `loadState`'s result reaches the store via `hydrate` before `bootApp`
//   resolves.
// - The session bridge is live afterwards: an incoming notification updates
//   the store and syncs the badge.
// - Persistence is live afterwards: an agent-profile change (post-boot)
//   queues a debounced `saveState`, and it is NOT fired by the hydrate call
//   itself (persistence installed after hydrate, per the ordering comment).
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { NotificationEvent, PersistedState } from "@shared/types";

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
    getAppSettings: vi.fn(),
    setAppSettings: vi.fn(),
    onData: vi.fn(),
    onSessionState: vi.fn(() => vi.fn()),
    onNotification: vi.fn(),
    onNotificationCleared: vi.fn(() => vi.fn()),
    onActivity: vi.fn(() => vi.fn()),
    appendSessionTurn: vi.fn(),
    loadSessionTurns: vi.fn(),
  },
}));

vi.mock("../ipc/tauriApi", () => ({ tauriApi: mockApi }));

// `installQuitGuard` (Task: quit confirmation gate) talks to
// `@tauri-apps/api/window` directly, not through `tauriApi` — mock it at
// this boundary the same way `tauriApi.test.ts` mocks `@tauri-apps/api/core`
// and `@tauri-apps/api/event`, so bootApp doesn't hit the real (absent)
// Tauri runtime in tests.
const mockOnCloseRequested = vi.fn(() => Promise.resolve(vi.fn()));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ onCloseRequested: mockOnCloseRequested }),
}));

// `installSoundManager`는 장식 기능 — node 테스트 환경엔 AudioContext가
// 없어 목 없이도 통과하지만(backend가 null → no-op teardown), 의도를
// 명시하기 위해 목을 건다.
vi.mock("../sound/soundManager", () => ({ installSoundManager: () => () => {} }));

import { useAppStore } from "../store/appStore";
import { bootApp } from "../bootstrap";

const initialState = useAppStore.getState();

function mkPersisted(): PersistedState {
  return {
    agents: [
      {
        id: "a1",
        name: "Loaded Agent",
        role: "backend",
        note: "",
        seed: "seed-a1",
        createdAt: Date.now(),
        deskIndex: 0,
      },
    ],
    version: 1,
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

let capturedOnNotification: ((e: NotificationEvent) => void) | undefined;
let teardown: () => void;

beforeEach(() => {
  vi.useFakeTimers();
  useAppStore.setState(initialState, true);
  Object.values(mockApi).forEach((fn) => fn.mockClear());
  mockApi.loadState.mockResolvedValue(mkPersisted());
  mockApi.getAppSettings.mockResolvedValue({
    settings: {
      version: 1,
      summarizerEnabled: false,
      summaryProvider: "claude",
      observerEnabled: false,
      soundEnabled: true,
      soundVolume: 0.5,
    },
    firstRun: false,
  });
  mockApi.loadSessionTurns.mockResolvedValue([]);
  mockApi.onNotification.mockImplementation((cb: (e: NotificationEvent) => void) => {
    capturedOnNotification = cb;
    return vi.fn();
  });
});

afterEach(() => {
  teardown?.();
  vi.useRealTimers();
});

describe("bootApp", () => {
  it("hydrates the store from loadState before resolving", async () => {
    teardown = await bootApp();

    const st = useAppStore.getState();
    expect(st.agentOrder).toEqual(["a1"]);
    expect(st.agents.a1.name).toBe("Loaded Agent");
  });

  it("does not queue a save just from the initial hydrate", async () => {
    teardown = await bootApp();

    await vi.advanceTimersByTimeAsync(1000);

    expect(mockApi.saveState).not.toHaveBeenCalled();
  });

  it("getAppSettings 결과를 부팅 전에 스토어에 반영한다", async () => {
    mockApi.getAppSettings.mockResolvedValue({
      settings: {
        version: 1,
        summarizerEnabled: true,
        summaryProvider: "codex",
        observerEnabled: false,
        soundEnabled: true,
        soundVolume: 0.5,
      },
      firstRun: true,
    });

    teardown = await bootApp();

    const s = useAppStore.getState();
    expect(s.appSettings).toEqual({
      version: 1,
      summarizerEnabled: true,
      summaryProvider: "codex",
      observerEnabled: false,
      soundEnabled: true,
      soundVolume: 0.5,
    });
    expect(s.settingsFirstRun).toBe(true);
  });

  it("getAppSettings가 실패해도 부팅은 계속되고 기본값(전부 OFF)이 유지된다", async () => {
    mockApi.getAppSettings.mockRejectedValue(new Error("backend not ready"));
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});

    teardown = await bootApp();

    const s = useAppStore.getState();
    expect(s.appSettings).toEqual({
      version: 1,
      summarizerEnabled: false,
      summaryProvider: "claude",
      observerEnabled: false,
      soundEnabled: true,
      soundVolume: 0.5,
      externalTerminal: "terminal",
    });
    expect(s.settingsFirstRun).toBe(false);
    expect(warn).toHaveBeenCalledWith(
      "bootstrap: 앱 설정 로드 실패 — 기본값(전부 OFF)으로 진행",
      expect.any(Error)
    );

    warn.mockRestore();
  });

  it("installs a live session bridge (notification -> store + badge sync)", async () => {
    teardown = await bootApp();

    capturedOnNotification?.(mkNotifEvent({ agentId: "a1" }));

    expect(useAppStore.getState().notifications).toHaveLength(1);
    expect(mockApi.setBadgeCount).toHaveBeenCalledWith(1);
  });

  it("installs live persistence (post-boot agent change debounces a save)", async () => {
    teardown = await bootApp();

    useAppStore.getState().addAgent({
      id: "a2",
      name: "New Agent",
      role: "eng",
      note: "",
      seed: "seed-a2",
      createdAt: Date.now(),
      deskIndex: 1,
    });

    expect(mockApi.saveState).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(500);
    expect(mockApi.saveState).toHaveBeenCalledTimes(1);
  });

  it("continues booting with empty state when loadState rejects (no half-boot)", async () => {
    mockApi.loadState.mockRejectedValue(new Error("backend not ready"));
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});

    // Must resolve, not reject — main.tsx calls `void bootApp()`, so a throw
    // here would be an unhandled rejection and would skip bridge/persistence.
    teardown = await bootApp();

    // Hydrated with the empty fallback.
    const st = useAppStore.getState();
    expect(st.agentOrder).toEqual([]);
    expect(st.agents).toEqual({});
    expect(warn).toHaveBeenCalledWith(
      "bootApp: loadState failed, continuing with empty state",
      expect.any(Error)
    );

    // Session bridge still installed: notification -> store + badge.
    useAppStore.getState().addAgent({
      id: "a1",
      name: "Agent",
      role: "eng",
      note: "",
      seed: "seed-a1",
      createdAt: Date.now(),
      deskIndex: 0,
    });
    capturedOnNotification?.(mkNotifEvent({ agentId: "a1" }));
    expect(useAppStore.getState().notifications).toHaveLength(1);
    expect(mockApi.setBadgeCount).toHaveBeenCalledWith(1);

    // Persistence still installed: the addAgent above debounces a save.
    await vi.advanceTimersByTimeAsync(500);
    expect(mockApi.saveState).toHaveBeenCalledTimes(1);

    warn.mockRestore();
  });
});
