// @vitest-environment jsdom
//
// src/renderer/terminal/__tests__/TerminalHost.test.tsx
//
// Tests for `TerminalHost` + `TerminalMount`.
//
// `TerminalRegistry` and `tauriApi` are both mocked — this is a pure
// orchestration test (does the component call the registry with the right
// arguments at the right times?), not a real-xterm test (that's 4C's job).
//
// Coverage:
// - Mounts exactly one `TerminalMount` per non-idle agent, and attaches its
//   container to the registry.
// - The active agent's mount is `display:block`; every other mount is
//   `display:none` — never removed from the DOM (keep-alive).
// - Becoming active triggers `terminalRegistry.activate()`, whose resize
//   callback updates both the store's session size and `tauriApi.resize`.
// - A `ResizeObserver` is installed only for the active mount, debounced
//   120ms, calling `terminalRegistry.refit()` (not `activate()` — no
//   refocus/re-scroll on plain resize).
// - The `ResizeObserver` is disconnected when the mount stops being active
//   or unmounts (no leaks).
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

const attach = vi.fn();
const activate = vi.fn();
const refit = vi.fn();

vi.mock("../TerminalRegistry", () => ({
  terminalRegistry: {
    attach: (...args: unknown[]) => attach(...args),
    activate: (...args: unknown[]) => activate(...args),
    refit: (...args: unknown[]) => refit(...args),
  },
}));

const resize = vi.fn();
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: { resize: (...args: unknown[]) => resize(...args) },
}));

const { TerminalHost } = await import("../TerminalHost");

class FakeResizeObserver {
  static instances: FakeResizeObserver[] = [];
  cb: ResizeObserverCallback;
  disconnect = vi.fn();
  observe = vi.fn();
  unobserve = vi.fn();
  constructor(cb: ResizeObserverCallback) {
    this.cb = cb;
    FakeResizeObserver.instances.push(this);
  }
  trigger() {
    this.cb([] as unknown as ResizeObserverEntry[], this as unknown as ResizeObserver);
  }
}

function mkProfile(id: string): AgentProfile {
  return {
    id,
    name: id,
    role: "eng",
    note: "",
    seed: id,
    createdAt: Date.now(),
    deskIndex: 0,
  };
}

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
  attach.mockReset();
  activate.mockReset();
  refit.mockReset();
  resize.mockReset();
  FakeResizeObserver.instances = [];
  vi.stubGlobal("ResizeObserver", FakeResizeObserver);
  vi.useFakeTimers();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("TerminalHost mount set", () => {
  it("renders one mount per non-idle agent and attaches it to the registry", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.getState().addAgent(mkProfile("a2"));

    const { container } = render(<TerminalHost />);

    const mounts = container.querySelectorAll("[data-agent-id]");
    expect(mounts).toHaveLength(2);
    expect(attach).toHaveBeenCalledTimes(2);
    expect(attach).toHaveBeenCalledWith("a1", expect.any(HTMLElement));
    expect(attach).toHaveBeenCalledWith("a2", expect.any(HTMLElement));
  });

  it("excludes idle agents (hydrated profiles with no session yet)", () => {
    useAppStore.getState().hydrate({
      agents: [{ ...mkProfile("a1") }],
      version: 1,
    });

    const { container } = render(<TerminalHost />);

    expect(container.querySelectorAll("[data-agent-id]")).toHaveLength(0);
    expect(attach).not.toHaveBeenCalled();
  });

  it("shows only the active agent's mount (display:block), others display:none", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.getState().addAgent(mkProfile("a2"));
    useAppStore.getState().openTerminal("a1");

    const { container } = render(<TerminalHost />);

    const a1 = container.querySelector('[data-agent-id="a1"]') as HTMLElement;
    const a2 = container.querySelector('[data-agent-id="a2"]') as HTMLElement;
    expect(a1.style.display).toBe("block");
    expect(a2.style.display).toBe("none");
  });

  it("switching the active agent toggles display without removing any mount from the DOM", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.getState().addAgent(mkProfile("a2"));
    useAppStore.getState().openTerminal("a1");

    const { container } = render(<TerminalHost />);
    const a1 = container.querySelector('[data-agent-id="a1"]') as HTMLElement;
    const a2 = container.querySelector('[data-agent-id="a2"]') as HTMLElement;

    act(() => useAppStore.getState().openTerminal("a2"));

    expect(container.querySelectorAll("[data-agent-id]")).toHaveLength(2);
    expect(a1.style.display).toBe("none");
    expect(a2.style.display).toBe("block");
  });
});

describe("activation: fit + resize + focus wiring", () => {
  it("calls terminalRegistry.activate() when an agent becomes active, whose resize callback updates session size and calls tauriApi.resize", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));

    render(<TerminalHost />);
    act(() => useAppStore.getState().openTerminal("a1"));

    expect(activate).toHaveBeenCalledTimes(1);
    const [agentId, onResize] = activate.mock.calls[0] as [string, (c: number, r: number) => void];
    expect(agentId).toBe("a1");

    act(() => onResize(100, 40));

    expect(useAppStore.getState().sessions["a1"].cols).toBe(100);
    expect(useAppStore.getState().sessions["a1"].rows).toBe(40);
    expect(resize).toHaveBeenCalledWith("a1", 100, 40);
  });

  it("does not activate mounts that never become active", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.getState().addAgent(mkProfile("a2"));
    useAppStore.getState().openTerminal("a1");

    render(<TerminalHost />);

    expect(activate).toHaveBeenCalledTimes(1);
    expect(activate).toHaveBeenCalledWith("a1", expect.any(Function));
  });
});

describe("ResizeObserver: active-only, 120ms debounce, refit (not activate)", () => {
  it("observes only the active mount's host element", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.getState().addAgent(mkProfile("a2"));
    useAppStore.getState().openTerminal("a1");

    render(<TerminalHost />);

    expect(FakeResizeObserver.instances).toHaveLength(1);
  });

  it("debounces bursts of resize callbacks into a single refit() after 120ms", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.getState().openTerminal("a1");

    render(<TerminalHost />);
    const ro = FakeResizeObserver.instances[0];

    act(() => {
      ro.trigger();
      vi.advanceTimersByTime(50);
      ro.trigger();
      vi.advanceTimersByTime(50);
      ro.trigger();
    });
    expect(refit).not.toHaveBeenCalled();

    act(() => vi.advanceTimersByTime(120));

    expect(refit).toHaveBeenCalledTimes(1);
    expect(refit).toHaveBeenCalledWith("a1", expect.any(Function));
  });

  it("refit's resize callback updates session size and calls tauriApi.resize (same contract as activate)", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.getState().openTerminal("a1");

    render(<TerminalHost />);
    const ro = FakeResizeObserver.instances[0];

    act(() => {
      ro.trigger();
      vi.advanceTimersByTime(120);
    });

    const [, onResize] = refit.mock.calls[0] as [string, (c: number, r: number) => void];
    act(() => onResize(90, 30));

    expect(useAppStore.getState().sessions["a1"].cols).toBe(90);
    expect(resize).toHaveBeenCalledWith("a1", 90, 30);
  });

  it("disconnects the observer when the mount stops being active", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.getState().addAgent(mkProfile("a2"));
    useAppStore.getState().openTerminal("a1");

    render(<TerminalHost />);
    const ro = FakeResizeObserver.instances[0];

    act(() => useAppStore.getState().openTerminal("a2"));

    expect(ro.disconnect).toHaveBeenCalledTimes(1);
    // The newly-active mount gets its own observer.
    expect(FakeResizeObserver.instances).toHaveLength(2);
  });

  it("disconnects the observer and clears any pending debounce timer on unmount", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.getState().openTerminal("a1");

    const { unmount } = render(<TerminalHost />);
    const ro = FakeResizeObserver.instances[0];
    act(() => ro.trigger()); // schedule a debounced refit

    unmount();
    act(() => vi.advanceTimersByTime(200));

    expect(ro.disconnect).toHaveBeenCalledTimes(1);
    expect(refit).not.toHaveBeenCalled(); // timer must have been cleared, not left to fire post-unmount
  });
});
