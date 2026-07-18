// src/renderer/ipc/__tests__/tauriApi.test.ts
//
// TDD for the frontend adapter over
// @tauri-apps/api. Everything at the `@tauri-apps/api/core` and
// `@tauri-apps/api/event` boundary is mocked; nothing else.
//
// Coverage required by the task brief:
// - onData fanout: 2 callbacks registered for the same agentId both receive
//   the same chunk.
// - unsubscribe refcount: `unsubscribe_output` is only invoked once the last
//   onData callback for an agentId has unsubscribed.
// - wrapListen pre-resolution unsubscribe: calling the returned unsubscribe
//   function before the `listen()` promise settles must not leak the
//   underlying Tauri listener (it must still be torn down once resolved).

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { Commands, Events } from "@shared/ipc";
import type { OutputChunk } from "@shared/types";

// Minimal stand-in for @tauri-apps/api/core's `Channel<T>`: the adapter only
// relies on `onmessage` being assignable, which is exactly what the real
// class supports.
class FakeChannel<T> {
  onmessage: (payload: T) => void = () => {};
}

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invoke(...args),
  Channel: FakeChannel,
}));

const listen = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listen(...args),
}));

beforeEach(() => {
  vi.resetModules();
  invoke.mockReset();
  listen.mockReset();
  invoke.mockResolvedValue(undefined);
});

function makeChunk(overrides: Partial<OutputChunk> = {}): OutputChunk {
  return {
    sessionId: "s1",
    agentId: "a1",
    data: "hello",
    frames: 1,
    seq: 1,
    ...overrides,
  };
}

/** Grabs the `Channel` instance passed to the `subscribe_output` invoke call. */
function capturedChannel(agentId: string): FakeChannel<OutputChunk> {
  const call = invoke.mock.calls.find(
    ([cmd, args]) => cmd === Commands.subscribeOutput && (args as { agentId: string }).agentId === agentId,
  );
  if (!call) throw new Error(`no subscribe_output invoke recorded for ${agentId}`);
  return (call[1] as { channel: FakeChannel<OutputChunk> }).channel;
}

async function importTauriApi() {
  const mod = await import("../tauriApi");
  return mod.tauriApi;
}

describe("onData fanout", () => {
  it("delivers a single chunk to every registered callback for the agent", async () => {
    const tauriApi = await importTauriApi();
    const cb1 = vi.fn();
    const cb2 = vi.fn();

    tauriApi.onData("a1", cb1);
    tauriApi.onData("a1", cb2);

    const channel = capturedChannel("a1");
    channel.onmessage(makeChunk({ data: "hi" }));

    expect(cb1).toHaveBeenCalledWith("hi");
    expect(cb2).toHaveBeenCalledWith("hi");
  });

  it("only subscribes once (single Channel/invoke) for multiple callbacks on the same agent", async () => {
    const tauriApi = await importTauriApi();
    tauriApi.onData("a1", vi.fn());
    tauriApi.onData("a1", vi.fn());

    const subscribeCalls = invoke.mock.calls.filter(([cmd]) => cmd === Commands.subscribeOutput);
    expect(subscribeCalls).toHaveLength(1);
  });

  it("keeps agents independent (separate channels, no cross-talk)", async () => {
    const tauriApi = await importTauriApi();
    const cbA = vi.fn();
    const cbB = vi.fn();
    tauriApi.onData("a1", cbA);
    tauriApi.onData("a2", cbB);

    capturedChannel("a1").onmessage(makeChunk({ agentId: "a1", data: "for-a1" }));

    expect(cbA).toHaveBeenCalledWith("for-a1");
    expect(cbB).not.toHaveBeenCalled();
  });

  it("does not let a throwing callback stop delivery to sibling callbacks", async () => {
    const tauriApi = await importTauriApi();
    const boom = vi.fn(() => {
      throw new Error("boom");
    });
    const fine = vi.fn();
    tauriApi.onData("a1", boom);
    tauriApi.onData("a1", fine);

    const channel = capturedChannel("a1");
    expect(() => channel.onmessage(makeChunk({ data: "x" }))).not.toThrow();
    expect(fine).toHaveBeenCalledWith("x");
  });
});

describe("onData unsubscribe refcount", () => {
  it("does not call unsubscribe_output while other callbacks remain", async () => {
    const tauriApi = await importTauriApi();
    const unsub1 = tauriApi.onData("a1", vi.fn());
    tauriApi.onData("a1", vi.fn());

    unsub1();

    expect(invoke).not.toHaveBeenCalledWith(Commands.unsubscribeOutput, expect.anything());
  });

  it("calls unsubscribe_output exactly once when the last callback unsubscribes", async () => {
    const tauriApi = await importTauriApi();
    const unsub1 = tauriApi.onData("a1", vi.fn());
    const unsub2 = tauriApi.onData("a1", vi.fn());

    unsub1();
    unsub2();

    const calls = invoke.mock.calls.filter(([cmd]) => cmd === Commands.unsubscribeOutput);
    expect(calls).toHaveLength(1);
    expect(calls[0][1]).toEqual({ agentId: "a1" });
  });

  it("unsubscribe is idempotent (calling it twice does not double-invoke unsubscribe_output)", async () => {
    const tauriApi = await importTauriApi();
    const unsub = tauriApi.onData("a1", vi.fn());

    unsub();
    unsub();

    const calls = invoke.mock.calls.filter(([cmd]) => cmd === Commands.unsubscribeOutput);
    expect(calls).toHaveLength(1);
  });

  it("re-registering onData after full unsubscribe subscribes again", async () => {
    const tauriApi = await importTauriApi();
    const unsub = tauriApi.onData("a1", vi.fn());
    unsub();

    tauriApi.onData("a1", vi.fn());

    const subscribeCalls = invoke.mock.calls.filter(([cmd]) => cmd === Commands.subscribeOutput);
    expect(subscribeCalls).toHaveLength(2);
  });
});

describe("wrapListen (session-state / notification-new / notification-cleared)", () => {
  it("subscribes to the exact event name and forwards the payload", async () => {
    let handler: ((e: { payload: unknown }) => void) | undefined;
    listen.mockImplementation((_event: string, h: (e: { payload: unknown }) => void) => {
      handler = h;
      return Promise.resolve(vi.fn());
    });

    const tauriApi = await importTauriApi();
    const cb = vi.fn();
    tauriApi.onSessionState(cb);
    await Promise.resolve();
    await Promise.resolve();

    expect(listen).toHaveBeenCalledWith(Events.sessionState, expect.any(Function));
    const payload = { sessionId: "s1", agentId: "a1", state: "running", at: 1 };
    handler?.({ payload });
    expect(cb).toHaveBeenCalledWith(payload);
  });

  it("unsubscribing before listen() resolves still tears down the listener once resolved (no leak)", async () => {
    let resolveListen!: (fn: UnlistenFn) => void;
    listen.mockImplementation(
      () =>
        new Promise<UnlistenFn>((resolve) => {
          resolveListen = resolve;
        }),
    );

    const tauriApi = await importTauriApi();
    const unsub = tauriApi.onSessionState(vi.fn());

    // Unsubscribe fires synchronously, before the listen() promise settles.
    unsub();

    const unlistenSpy = vi.fn();
    resolveListen(unlistenSpy);
    await Promise.resolve();
    await Promise.resolve();

    expect(unlistenSpy).toHaveBeenCalledTimes(1);
  });

  it("unsubscribing after resolution calls the real unlisten function exactly once, even if called twice", async () => {
    const unlistenSpy = vi.fn();
    listen.mockResolvedValue(unlistenSpy);

    const tauriApi = await importTauriApi();
    const unsub = tauriApi.onNotification(vi.fn());
    await Promise.resolve();
    await Promise.resolve();

    unsub();
    unsub();

    expect(unlistenSpy).toHaveBeenCalledTimes(1);
  });

  it("a throwing callback does not prevent future events (onNotificationCleared)", async () => {
    let handler: ((e: { payload: unknown }) => void) | undefined;
    listen.mockImplementation((_event: string, h: (e: { payload: unknown }) => void) => {
      handler = h;
      return Promise.resolve(vi.fn());
    });

    const tauriApi = await importTauriApi();
    const boom = vi.fn(() => {
      throw new Error("boom");
    });
    tauriApi.onNotificationCleared(boom);
    await Promise.resolve();
    await Promise.resolve();

    const payload = { agentId: "a1", ids: ["n1"] };
    expect(() => handler?.({ payload })).not.toThrow();
    expect(boom).toHaveBeenCalledWith(payload);
  });
});

describe("command invocations", () => {
  it("createSession maps missing opts to null and uses the exact command name", async () => {
    invoke.mockResolvedValueOnce({ sessionId: "s1", state: "starting" });
    const tauriApi = await importTauriApi();

    const result = await tauriApi.createSession("a1");

    expect(invoke).toHaveBeenCalledWith(Commands.createSession, { agentId: "a1", opts: null });
    expect(result).toEqual({ sessionId: "s1", state: "starting" });
  });

  it("createSession forwards provided opts as-is", async () => {
    invoke.mockResolvedValueOnce({ sessionId: "s1", state: "starting" });
    const tauriApi = await importTauriApi();

    await tauriApi.createSession("a1", { cols: 80, rows: 24, cwd: "/tmp" });

    expect(invoke).toHaveBeenCalledWith(Commands.createSession, {
      agentId: "a1",
      opts: { cols: 80, rows: 24, cwd: "/tmp" },
    });
  });

  it("createSession forwards a shell opt as-is", async () => {
    invoke.mockResolvedValueOnce({ sessionId: "s1", state: "starting" });
    const tauriApi = await importTauriApi();

    await tauriApi.createSession("a1", { cwd: "/tmp", shell: "wsl" });

    expect(invoke).toHaveBeenCalledWith(Commands.createSession, {
      agentId: "a1",
      opts: { cwd: "/tmp", shell: "wsl" },
    });
  });

  it("disposeSession invokes dispose_session with agentId", async () => {
    const tauriApi = await importTauriApi();
    await tauriApi.disposeSession("a1");
    expect(invoke).toHaveBeenCalledWith(Commands.disposeSession, { agentId: "a1" });
  });

  it("writeInput is fire-and-forget and does not return a promise", async () => {
    const tauriApi = await importTauriApi();
    const result = tauriApi.writeInput("a1", "ls\n");
    expect(result).toBeUndefined();
    expect(invoke).toHaveBeenCalledWith(Commands.writeInput, { agentId: "a1", data: "ls\n" });
  });

  it("resize invokes resize_session with cols/rows", async () => {
    const tauriApi = await importTauriApi();
    tauriApi.resize("a1", 100, 40);
    expect(invoke).toHaveBeenCalledWith(Commands.resize, { agentId: "a1", cols: 100, rows: 40 });
  });

  it("clearNotifications maps missing ids to null", async () => {
    const tauriApi = await importTauriApi();
    tauriApi.clearNotifications("a1");
    expect(invoke).toHaveBeenCalledWith(Commands.clearNotifications, { agentId: "a1", ids: null });
  });

  it("clearNotifications forwards provided ids", async () => {
    const tauriApi = await importTauriApi();
    tauriApi.clearNotifications("a1", ["n1", "n2"]);
    expect(invoke).toHaveBeenCalledWith(Commands.clearNotifications, { agentId: "a1", ids: ["n1", "n2"] });
  });

  it("listNotifications returns the array resolved by invoke", async () => {
    const notifications = [
      { id: "n1", sessionId: "s1", agentId: "a1", source: "hook", message: "?", dedupKey: "k", at: 1 },
    ];
    invoke.mockResolvedValueOnce(notifications);
    const tauriApi = await importTauriApi();

    const result = await tauriApi.listNotifications("a1");

    expect(invoke).toHaveBeenCalledWith(Commands.listNotifications, { agentId: "a1" });
    expect(Array.isArray(result)).toBe(true);
    expect(result).toEqual(notifications);
  });

  it("loadState invokes load_state with no args and returns the resolved state", async () => {
    const state = { agents: [], version: 1 as const };
    invoke.mockResolvedValueOnce(state);
    const tauriApi = await importTauriApi();

    const result = await tauriApi.loadState();

    expect(invoke).toHaveBeenCalledWith(Commands.loadState);
    expect(result).toEqual(state);
  });

  it("saveState invokes save_state with the state payload", async () => {
    const state = { agents: [], version: 1 as const };
    const tauriApi = await importTauriApi();

    await tauriApi.saveState(state);

    expect(invoke).toHaveBeenCalledWith(Commands.saveState, { state });
  });

  it("setBadgeCount invokes set_badge_count with count", async () => {
    const tauriApi = await importTauriApi();
    tauriApi.setBadgeCount(3);
    expect(invoke).toHaveBeenCalledWith(Commands.setBadgeCount, { count: 3 });
  });

  it("summarizeText는 provider snapshot을 함께 전달한다", async () => {
    const tauriApi = await importTauriApi();

    await tauriApi.summarizeText("codex", "요약 지시", "원문");

    expect(invoke).toHaveBeenCalledWith(Commands.summarizeText, {
      provider: "codex",
      instruction: "요약 지시",
      text: "원문",
    });
  });

  it("generateSpriteImage는 generate_sprite_image를 description과 함께 invoke하고 결과를 반환한다", async () => {
    invoke.mockResolvedValueOnce({ pngBase64: "AAAA", costUsd: 0.02 });
    const { tauriApi } = await import("../tauriApi");
    const res = await tauriApi.generateSpriteImage("a knight");
    expect(invoke).toHaveBeenCalledWith(Commands.generateSpriteImage, {
      description: "a knight",
    });
    expect(res).toEqual({ pngBase64: "AAAA", costUsd: 0.02 });
  });
});

describe("portrait commands", () => {
  it("savePortrait invokes save_portrait with agentId + pngBase64", async () => {
    const tauriApi = await importTauriApi();
    await tauriApi.savePortrait("a1", "BASE64DATA");
    expect(invoke).toHaveBeenCalledWith(Commands.savePortrait, {
      agentId: "a1",
      pngBase64: "BASE64DATA",
    });
  });

  it("loadPortrait returns the backend base64 string", async () => {
    const tauriApi = await importTauriApi();
    invoke.mockResolvedValueOnce("PNGBASE64");
    const result = await tauriApi.loadPortrait("a1");
    expect(invoke).toHaveBeenCalledWith(Commands.loadPortrait, { agentId: "a1" });
    expect(result).toBe("PNGBASE64");
  });

  it("loadPortrait returns null when the backend has no file", async () => {
    const tauriApi = await importTauriApi();
    invoke.mockResolvedValueOnce(null);
    expect(await tauriApi.loadPortrait("a1")).toBeNull();
  });

  it("deletePortrait invokes delete_portrait", async () => {
    const tauriApi = await importTauriApi();
    await tauriApi.deletePortrait("a1");
    expect(invoke).toHaveBeenCalledWith(Commands.deletePortrait, { agentId: "a1" });
  });
});

describe("app settings commands", () => {
  it("getAppSettings는 get_app_settings를 인자 없이 invoke한다", async () => {
    invoke.mockResolvedValueOnce({
      settings: {
        version: 1,
        summarizerEnabled: false,
        summaryProvider: "claude",
        observerEnabled: false,
      },
      firstRun: true,
    });
    const tauriApi = await importTauriApi();

    const r = await tauriApi.getAppSettings();

    expect(invoke).toHaveBeenCalledWith(Commands.getAppSettings);
    expect(r.firstRun).toBe(true);
  });

  it("setAppSettings는 set_app_settings에 { settings }를 전달한다", async () => {
    const s = {
      version: 1,
      summarizerEnabled: true,
      summaryProvider: "codex" as const,
      observerEnabled: false,
      soundEnabled: true,
      soundVolume: 0.5,
      externalTerminal: "terminal" as const,
      attentionHoldMs: 5000,
    };
    const tauriApi = await importTauriApi();

    await tauriApi.setAppSettings(s);

    expect(invoke).toHaveBeenCalledWith(Commands.setAppSettings, { settings: s });
  });

  it("listAvailableShells invokes list_available_shells with no args and returns the resolved list", async () => {
    const shells = [
      { id: "pwsh", label: "PowerShell 7", path: "C:\\pwsh.exe", hooksSupported: true },
      { id: "wsl", label: "WSL", path: "C:\\wsl.exe", hooksSupported: false },
    ];
    invoke.mockResolvedValueOnce(shells);
    const tauriApi = await importTauriApi();

    const result = await tauriApi.listAvailableShells();

    expect(invoke).toHaveBeenCalledWith(Commands.listAvailableShells);
    expect(result).toEqual(shells);
  });
});

describe("session handoff commands", () => {
  it("handoffSupported invokes handoff_supported with no args and returns the resolved boolean", async () => {
    invoke.mockResolvedValueOnce(true);
    const tauriApi = await importTauriApi();

    const result = await tauriApi.handoffSupported();

    expect(invoke).toHaveBeenCalledWith(Commands.handoffSupported);
    expect(result).toBe(true);
  });

  it("handoffSessions invokes handoff_sessions with the snapshots map and returns the handed-off count", async () => {
    invoke.mockResolvedValueOnce(3);
    const tauriApi = await importTauriApi();
    const snapshots = { a1: "SCREEN-A1", a2: "SCREEN-A2" };

    const result = await tauriApi.handoffSessions(snapshots);

    expect(invoke).toHaveBeenCalledWith(Commands.handoffSessions, { snapshots });
    expect(result).toBe(3);
  });

  it("adoptDetachedSessions invokes adopt_detached_sessions with no args and returns the resolved list", async () => {
    const sessions = [{ agentId: "a1", sessionId: "s1", rows: 24, cols: 80 }];
    invoke.mockResolvedValueOnce(sessions);
    const tauriApi = await importTauriApi();

    const result = await tauriApi.adoptDetachedSessions();

    expect(invoke).toHaveBeenCalledWith(Commands.adoptDetachedSessions);
    expect(result).toEqual(sessions);
  });
});
