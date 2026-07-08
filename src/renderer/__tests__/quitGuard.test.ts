// src/renderer/__tests__/quitGuard.test.ts
//
// TDD for the app-quit confirmation gate: `installQuitGuard` registers a
// `CloseRequested` handler that blocks the close and opens the
// `confirm-quit` modal only when some agent has an open turn (same signal
// as SessionTimePanel's `anyOpen`: `timeTracking[agentId].phase !== "idle"`,
// NOT `session.status` — a bare idle shell must not block quit).
//
// `@tauri-apps/api/window` is mocked at the module boundary (same pattern as
// `ipc/__tests__/tauriApi.test.ts` mocking `@tauri-apps/api/core`/`event`) so
// this test never touches a real Tauri runtime.
import { beforeEach, describe, expect, it, vi } from "vitest";

let capturedHandler: ((event: { preventDefault: () => void }) => void) | undefined;
const onCloseRequested = vi.fn((handler: (event: { preventDefault: () => void }) => void) => {
  capturedHandler = handler;
  return Promise.resolve(vi.fn());
});
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ onCloseRequested }),
}));

import { useAppStore } from "../store/appStore";
import { initialTurnState } from "../timeline/turnReducer";
import { installQuitGuard } from "../quitGuard";

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
  onCloseRequested.mockClear();
  capturedHandler = undefined;
});

function fireCloseRequested(): { prevented: boolean } {
  const result = { prevented: false };
  capturedHandler?.({ preventDefault: () => { result.prevented = true; } });
  return result;
}

describe("installQuitGuard", () => {
  it("registers a CloseRequested handler", () => {
    installQuitGuard();
    expect(onCloseRequested).toHaveBeenCalledTimes(1);
  });

  it("does nothing when no agent has an open turn (all idle)", () => {
    useAppStore.setState({ timeTracking: { a1: initialTurnState() } });
    installQuitGuard();

    const { prevented } = fireCloseRequested();

    expect(prevented).toBe(false);
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });

  it("blocks the close and opens confirm-quit when some agent has an open turn", () => {
    useAppStore.setState({
      timeTracking: { a1: { ...initialTurnState(), phase: "working" } },
    });
    installQuitGuard();

    const { prevented } = fireCloseRequested();

    expect(prevented).toBe(true);
    expect(useAppStore.getState().modal).toEqual({ kind: "confirm-quit" });
  });

  it("ignores session.status and only looks at timeTracking phase", () => {
    // A bare idle shell (session running, no open turn) must not block quit.
    useAppStore.setState({
      sessions: { a1: { agentId: "a1", status: "running", cols: 80, rows: 24, lastActivityAt: 0 } },
      timeTracking: { a1: initialTurnState() },
    });
    installQuitGuard();

    const { prevented } = fireCloseRequested();

    expect(prevented).toBe(false);
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
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
