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
    handoffSupported: vi.fn(),
    handoffSessions: vi.fn(),
    adoptDetachedSessions: vi.fn(),
    sessionBrokerMode: vi.fn(),
    uploadSessionSnapshots: vi.fn(),
  },
}));

vi.mock("../ipc/tauriApi", () => ({ tauriApi: mockApi }));

// `TerminalRegistry`는 실제 xterm(DOM 필요)을 구성한다 — node 환경인 이 테스트
// 파일에서는 `installSoundManager`와 같은 이유로 목으로 대체하고, 입양 시드가
// `markAdopted`를 호출하는지만 배선으로 검증한다.
const markAdopted = vi.fn();
const serializeAll = vi.fn(() => ({}) as Record<string, string>);
const getRenderedBytes = vi.fn(() => ({}) as Record<string, number>);
vi.mock("../terminal/TerminalRegistry", () => ({
  terminalRegistry: {
    markAdopted: (...args: unknown[]) => markAdopted(...args),
    serializeAll: () => serializeAll(),
    // 30초 업로더는 flush 후 직렬화하는 async 경로를 쓴다(§P1) — 목은 동일한
    // serializeAll 스텁을 Promise로 감싸 돌려준다.
    flushAndSerializeAll: () => Promise.resolve(serializeAll()),
    // §#49: 업로더는 스냅샷과 함께 렌더러 누적 바이트를 실어 보낸다.
    getRenderedBytes: () => getRenderedBytes(),
  },
}));

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
import { bootApp, SNAPSHOT_UPLOAD_INTERVAL_MS } from "../bootstrap";

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
      diaryEnabled: false,
      observerEnabled: false,
      soundEnabled: true,
      soundVolume: 0.5,
    },
    firstRun: false,
  });
  mockApi.loadSessionTurns.mockResolvedValue([]);
  mockApi.handoffSupported.mockResolvedValue(false);
  mockApi.handoffSessions.mockResolvedValue(0);
  mockApi.adoptDetachedSessions.mockResolvedValue([]);
  mockApi.sessionBrokerMode.mockResolvedValue(false);
  mockApi.uploadSessionSnapshots.mockResolvedValue(undefined);
  markAdopted.mockClear();
  serializeAll.mockClear();
  serializeAll.mockReturnValue({});
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
        diaryEnabled: false,
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
      diaryEnabled: false,
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
      diaryEnabled: false,
      observerEnabled: false,
      soundEnabled: true,
      soundVolume: 0.5,
      externalTerminal: "terminal",
      externalEditor: "system",
      attentionHoldMs: 5000,
      gitStatusEnabled: true,
      cliEnabled: false,
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

describe("session-handoff adoption (bootApp)", () => {
  it("seeds an adopted session's status/size and marks it for a terminal redraw nudge", async () => {
    mockApi.adoptDetachedSessions.mockResolvedValue([
      { agentId: "a1", sessionId: "s-old", rows: 40, cols: 120 },
    ]);

    teardown = await bootApp();

    const session = useAppStore.getState().sessions.a1;
    expect(session.status).toBe("running");
    expect(session.cols).toBe(120);
    expect(session.rows).toBe(40);
    expect(markAdopted).toHaveBeenCalledWith(["a1"]);
  });

  it("does not touch the store or mark anything when adoptDetachedSessions resolves empty (default/unsupported)", async () => {
    teardown = await bootApp();

    // mkPersisted's a1 keeps its post-hydrate idle status untouched.
    expect(useAppStore.getState().sessions.a1.status).toBe("idle");
    expect(markAdopted).not.toHaveBeenCalled();
  });

  it("seeds every returned session when multiple sessions are adopted", async () => {
    mockApi.loadState.mockResolvedValue({
      agents: [
        { id: "a1", name: "A1", role: "", note: "", seed: "s1", createdAt: 0, deskIndex: 0 },
        { id: "a2", name: "A2", role: "", note: "", seed: "s2", createdAt: 0, deskIndex: 1 },
      ],
      version: 1,
    });
    mockApi.adoptDetachedSessions.mockResolvedValue([
      { agentId: "a1", sessionId: "s-old-1", rows: 24, cols: 80 },
      { agentId: "a2", sessionId: "s-old-2", rows: 30, cols: 100 },
    ]);

    teardown = await bootApp();

    expect(useAppStore.getState().sessions.a1.status).toBe("running");
    expect(useAppStore.getState().sessions.a2.status).toBe("running");
    expect(markAdopted).toHaveBeenCalledWith(["a1", "a2"]);
  });

  it("continues booting (no half-boot) when adoptDetachedSessions rejects", async () => {
    mockApi.adoptDetachedSessions.mockRejectedValue(new Error("no sessiond socket"));
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});

    teardown = await bootApp();

    expect(useAppStore.getState().agentOrder).toEqual(["a1"]); // hydrate still ran
    expect(markAdopted).not.toHaveBeenCalled();
    expect(warn).toHaveBeenCalledWith(
      "bootstrap: 세션 입양 실패 — 이전 세션 없이 진행",
      expect.any(Error)
    );

    // Rest of boot still live (bridge + persistence).
    capturedOnNotification?.(mkNotifEvent({ agentId: "a1" }));
    expect(useAppStore.getState().notifications).toHaveLength(1);

    warn.mockRestore();
  });
});

describe("session-broker snapshot uploader (bootApp)", () => {
  it("브로커 모드가 아니면 주기 스냅샷 업로드 타이머를 켜지 않는다", async () => {
    mockApi.sessionBrokerMode.mockResolvedValue(false);

    teardown = await bootApp();
    await vi.advanceTimersByTimeAsync(60_000);

    expect(serializeAll).not.toHaveBeenCalled();
    expect(mockApi.uploadSessionSnapshots).not.toHaveBeenCalled();
  });

  it("브로커 모드면 30초마다 직렬화 화면을 업로드한다", async () => {
    mockApi.sessionBrokerMode.mockResolvedValue(true);
    serializeAll.mockReturnValue({ a1: "SCREEN-A1", a2: "SCREEN-A2" });
    getRenderedBytes.mockReturnValue({ a1: 11, a2: 22 });

    teardown = await bootApp();
    // 인터벌 첫 발화(30s).
    await vi.advanceTimersByTimeAsync(SNAPSHOT_UPLOAD_INTERVAL_MS);

    expect(mockApi.uploadSessionSnapshots).toHaveBeenCalledWith(
      {
        a1: "SCREEN-A1",
        a2: "SCREEN-A2",
      },
      { a1: 11, a2: 22 }
    );

    // 두 번째 주기에도 다시 발화.
    await vi.advanceTimersByTimeAsync(SNAPSHOT_UPLOAD_INTERVAL_MS);
    expect(mockApi.uploadSessionSnapshots).toHaveBeenCalledTimes(2);
  });

  it("브로커 모드라도 직렬화 결과가 비면 업로드하지 않는다", async () => {
    mockApi.sessionBrokerMode.mockResolvedValue(true);
    serializeAll.mockReturnValue({});

    teardown = await bootApp();
    await vi.advanceTimersByTimeAsync(SNAPSHOT_UPLOAD_INTERVAL_MS);

    expect(mockApi.uploadSessionSnapshots).not.toHaveBeenCalled();
  });

  it("teardown이 스냅샷 타이머를 멈춘다", async () => {
    mockApi.sessionBrokerMode.mockResolvedValue(true);
    serializeAll.mockReturnValue({ a1: "SCREEN" });

    teardown = await bootApp();
    await vi.advanceTimersByTimeAsync(SNAPSHOT_UPLOAD_INTERVAL_MS);
    expect(mockApi.uploadSessionSnapshots).toHaveBeenCalledTimes(1);

    teardown();
    teardown = () => {}; // afterEach 중복 호출 방지
    await vi.advanceTimersByTimeAsync(SNAPSHOT_UPLOAD_INTERVAL_MS * 3);
    expect(mockApi.uploadSessionSnapshots).toHaveBeenCalledTimes(1);
  });
});
