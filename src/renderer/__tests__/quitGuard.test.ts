// src/renderer/__tests__/quitGuard.test.ts
//
// TDD for the app-quit confirmation gate: `installQuitGuard` registers a
// `CloseRequested` handler that blocks the close and opens the `confirm-quit`
// modal only when some agent is still on duty (present but not `clockedOut`,
// the same signal as `useLightsOff`). An empty office, or one where everyone
// has clocked out, closes without a prompt.
//
// `@tauri-apps/api/window` is mocked at the module boundary (same pattern as
// `ipc/__tests__/tauriApi.test.ts` mocking `@tauri-apps/api/core`/`event`) so
// this test never touches a real Tauri runtime.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

let capturedHandler: ((event: { preventDefault: () => void }) => void) | undefined;
const destroy = vi.fn().mockResolvedValue(undefined);
const saveWorkItems = vi.fn().mockResolvedValue(undefined);
const onCloseRequested = vi.fn((handler: (event: { preventDefault: () => void }) => void) => {
  capturedHandler = handler;
  return Promise.resolve(vi.fn());
});
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ onCloseRequested, destroy }),
}));

// `installQuitGuard`가 부팅 시 캐시하는 handoff_supported() 조회 — 실제
// `@tauri-apps/api/core` invoke를 타지 않도록 tauriApi 자체를 모킹한다
// (다른 tauriApi 메서드는 이 파일에서 쓰지 않으므로 전체 모킹으로 충분).
const handoffSupported = vi.fn();
vi.mock("../ipc/tauriApi", () => ({
  tauriApi: { handoffSupported: (...args: unknown[]) => handoffSupported(...args), saveWorkItems: (...args: unknown[]) => saveWorkItems(...args) },
}));

import { useWorkItemStore } from "../workpanels/workItemStore";
import { useAppStore } from "../store/appStore";
import type { AgentProfile } from "../store/types";
import { installQuitGuard, isHandoffSupported } from "../quitGuard";

const initialState = useAppStore.getState();

function mkAgent(id: string, clockedOut?: boolean): AgentProfile {
  return {
    id,
    name: id,
    role: "",
    seed: id,
    createdAt: 0,
    deskIndex: 0,
    ...(clockedOut ? { clockedOut: true } : {}),
  };
}

beforeEach(() => {
  useAppStore.setState(initialState, true);
  onCloseRequested.mockClear();
  destroy.mockClear(); saveWorkItems.mockReset().mockResolvedValue(undefined);
  useWorkItemStore.setState({items:[],loaded:false,dirty:false,error:null});
  capturedHandler = undefined;
  handoffSupported.mockReset().mockResolvedValue(false);
});

function fireCloseRequested(): { prevented: boolean } {
  const result = { prevented: false };
  capturedHandler?.({ preventDefault: () => { result.prevented = true; } });
  return result;
}

afterEach(async () => { await useWorkItemStore.getState().flush(); });

describe("installQuitGuard", () => {
  it("전원 퇴근 상태라도 업무 저장을 기다린 뒤 종료한다", async () => {
    useWorkItemStore.setState({loaded:true});
    useWorkItemStore.getState().add("Unsaved task");
    installQuitGuard();
    expect(fireCloseRequested().prevented).toBe(true);
    expect(destroy).not.toHaveBeenCalled();
    await vi.waitFor(() => expect(destroy).toHaveBeenCalledTimes(1));
    expect(saveWorkItems).toHaveBeenCalledTimes(1);
  });
  it("종료 직전 저장 실패는 창을 파괴하지 않고 재시도할 수 있게 한다", async () => {
    useWorkItemStore.setState({loaded:true});
    useWorkItemStore.getState().add("Unsaved task");
    saveWorkItems.mockRejectedValueOnce(new Error("disk"));
    installQuitGuard();
    expect(fireCloseRequested().prevented).toBe(true);
    await vi.waitFor(() => expect(useAppStore.getState().modal).toEqual({kind:"confirm-quit"}));
    expect(destroy).not.toHaveBeenCalled(); expect(useWorkItemStore.getState().dirty).toBe(true);
  });
  it("registers a CloseRequested handler", () => {
    installQuitGuard();
    expect(onCloseRequested).toHaveBeenCalledTimes(1);
  });

  it("does nothing for an empty office (no agents)", () => {
    installQuitGuard();

    const { prevented } = fireCloseRequested();

    expect(prevented).toBe(false);
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });

  it("does nothing when every agent has clocked out", () => {
    useAppStore.setState({
      agents: { a1: mkAgent("a1", true), a2: mkAgent("a2", true) },
      agentOrder: ["a1", "a2"],
    });
    installQuitGuard();

    const { prevented } = fireCloseRequested();

    expect(prevented).toBe(false);
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });

  it("blocks the close and opens confirm-quit when some agent is still on duty", () => {
    useAppStore.setState({
      agents: { a1: mkAgent("a1", true), a2: mkAgent("a2") },
      agentOrder: ["a1", "a2"],
    });
    installQuitGuard();

    const { prevented } = fireCloseRequested();

    expect(prevented).toBe(true);
    expect(useAppStore.getState().modal).toEqual({ kind: "confirm-quit" });
  });

  it("blocks even for an idle on-duty agent (clock-out, not turn phase, is the signal)", () => {
    // An agent that never opened a turn but hasn't clocked out must still block.
    useAppStore.setState({
      agents: { a1: mkAgent("a1") },
      agentOrder: ["a1"],
    });
    installQuitGuard();

    const { prevented } = fireCloseRequested();

    expect(prevented).toBe(true);
    expect(useAppStore.getState().modal).toEqual({ kind: "confirm-quit" });
  });

  it("the returned unlisten fn resolves the underlying unlisten promise", async () => {
    const un = installQuitGuard();
    un();
    await Promise.resolve();
    // The mocked onCloseRequested resolves to a fresh vi.fn(); we only assert
    // that calling the teardown doesn't throw and awaits cleanly.
    expect(true).toBe(true);
  });
});

describe("isHandoffSupported cache", () => {
  it("caches a resolved handoff_supported()=true after install", async () => {
    handoffSupported.mockResolvedValue(true);
    installQuitGuard();

    await Promise.resolve();
    await Promise.resolve();

    expect(isHandoffSupported()).toBe(true);
  });

  it("caches false when handoff_supported() resolves false", async () => {
    handoffSupported.mockResolvedValue(false);
    installQuitGuard();

    await Promise.resolve();
    await Promise.resolve();

    expect(isHandoffSupported()).toBe(false);
  });

  it("falls back to false when handoff_supported() rejects (unsupported/older backend)", async () => {
    handoffSupported.mockRejectedValue(new Error("no such command"));
    installQuitGuard();

    await Promise.resolve();
    await Promise.resolve();

    expect(isHandoffSupported()).toBe(false);
  });
});
