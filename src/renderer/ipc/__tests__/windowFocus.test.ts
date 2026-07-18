// src/renderer/ipc/__tests__/windowFocus.test.ts
//
// 창 포커스 추적(이슈 #39): isFocused()로 초기값을 시딩하고 onFocusChanged로
// 이후 변화를 store.windowFocused에 반영하는지, 런타임 부재에도 안전한지 본다.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { getCurrentWindow, isFocused, onFocusChanged } = vi.hoisted(() => {
  const isFocused = vi.fn();
  const onFocusChanged = vi.fn();
  return {
    getCurrentWindow: vi.fn(() => ({ isFocused, onFocusChanged })),
    isFocused,
    onFocusChanged,
  };
});

vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow }));

import { useAppStore } from "../../store/appStore";
import { installWindowFocusTracking } from "../windowFocus";

const initialState = useAppStore.getState();
const flush = () => new Promise((r) => setTimeout(r, 0));
let cleanup: () => void = () => {};

beforeEach(() => {
  useAppStore.setState(initialState, true);
  vi.clearAllMocks();
  getCurrentWindow.mockImplementation(() => ({ isFocused, onFocusChanged }));
});

afterEach(() => {
  cleanup();
  cleanup = () => {};
});

describe("installWindowFocusTracking", () => {
  it("seeds initial focus from isFocused() and reflects onFocusChanged updates", async () => {
    isFocused.mockResolvedValue(false);
    let handler: (e: { payload: boolean }) => void = () => {};
    const unlisten = vi.fn();
    onFocusChanged.mockImplementation((cb: (e: { payload: boolean }) => void) => {
      handler = cb;
      return Promise.resolve(unlisten);
    });

    cleanup = installWindowFocusTracking();
    await flush();
    expect(useAppStore.getState().windowFocused).toBe(false);

    handler({ payload: true });
    expect(useAppStore.getState().windowFocused).toBe(true);
    handler({ payload: false });
    expect(useAppStore.getState().windowFocused).toBe(false);

    cleanup();
    cleanup = () => {};
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("stays focused (default) and does not throw when the window API is unavailable", () => {
    getCurrentWindow.mockImplementation(() => {
      throw new Error("no runtime");
    });
    expect(() => {
      cleanup = installWindowFocusTracking();
    }).not.toThrow();
    expect(useAppStore.getState().windowFocused).toBe(true);
  });
});
