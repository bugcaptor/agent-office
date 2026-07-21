// @vitest-environment jsdom
//
// src/renderer/terminal/__tests__/TerminalOverlay.test.tsx
//
// Tests for `TerminalOverlay`.
//
// The load-bearing assertion (README keep-alive guarantee): closing the
// overlay is a `display:none` toggle on the overlay root, never a
// conditional-render unmount of its children (`AgentTabStrip`,
// `TerminalHost`) — both must stay mounted (no cleanup-effect firing) across
// open/close cycles so xterm instances underneath never see a remount.
//
// `AgentTabStrip` and `TerminalHost` are mocked with instrumented stand-ins
// (mount/unmount effect spies) so this test is only about the overlay
// shell's own display-toggle behavior, not their internals.
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { useEffect } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

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

const tabStripMount = vi.fn();
const tabStripUnmount = vi.fn();
const hostMount = vi.fn();
const hostUnmount = vi.fn();

vi.mock("../AgentTabStrip", () => ({
  AgentTabStrip: () => {
    useEffect(() => {
      tabStripMount();
      return tabStripUnmount;
    }, []);
    return <div data-testid="tab-strip-stub" />;
  },
}));

vi.mock("../TerminalHost", () => ({
  TerminalHost: () => {
    useEffect(() => {
      hostMount();
      return hostUnmount;
    }, []);
    return <div data-testid="terminal-host-stub" />;
  },
}));

const { TerminalOverlay } = await import("../TerminalOverlay");

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
  tabStripMount.mockReset();
  tabStripUnmount.mockReset();
  hostMount.mockReset();
  hostUnmount.mockReset();
});

afterEach(() => cleanup());

describe("TerminalOverlay display toggle (keep-alive)", () => {
  it("is display:none when no terminal is active", () => {
    const { container } = render(<TerminalOverlay />);
    const root = container.firstElementChild as HTMLElement;
    expect(root.style.display).toBe("none");
  });

  it("becomes visible (non-'none' display) once a terminal is opened, without remounting children", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    const { container } = render(<TerminalOverlay />);
    expect(tabStripMount).toHaveBeenCalledTimes(1);
    expect(hostMount).toHaveBeenCalledTimes(1);

    act(() => useAppStore.getState().openTerminal("a1"));

    const root = container.firstElementChild as HTMLElement;
    expect(root.style.display).not.toBe("none");
    expect(tabStripMount).toHaveBeenCalledTimes(1); // no re-mount
    expect(hostMount).toHaveBeenCalledTimes(1);
    expect(tabStripUnmount).not.toHaveBeenCalled();
    expect(hostUnmount).not.toHaveBeenCalled();
  });

  it("closing again only flips display back to none — children remain mounted", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    const { container } = render(<TerminalOverlay />);
    act(() => useAppStore.getState().openTerminal("a1"));
    act(() => useAppStore.getState().closeTerminal());

    const root = container.firstElementChild as HTMLElement;
    expect(root.style.display).toBe("none");
    expect(tabStripUnmount).not.toHaveBeenCalled();
    expect(hostUnmount).not.toHaveBeenCalled();
    // Children are still present in the tree (not conditionally rendered away).
    expect(container.querySelector('[data-testid="tab-strip-stub"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="terminal-host-stub"]')).not.toBeNull();
  });

  it("survives repeated open/close cycles with exactly one mount/unmount each", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    render(<TerminalOverlay />);
    for (let i = 0; i < 5; i += 1) {
      act(() => useAppStore.getState().openTerminal("a1"));
      act(() => useAppStore.getState().closeTerminal());
    }
    expect(tabStripMount).toHaveBeenCalledTimes(1);
    expect(hostMount).toHaveBeenCalledTimes(1);
    expect(tabStripUnmount).not.toHaveBeenCalled();
    expect(hostUnmount).not.toHaveBeenCalled();
  });

  it("always renders both AgentTabStrip and TerminalHost as children", () => {
    const { container } = render(<TerminalOverlay />);
    expect(container.querySelector('[data-testid="tab-strip-stub"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="terminal-host-stub"]')).not.toBeNull();
  });
});

describe("뷰 모드 클래스(이슈 #69)", () => {
  it("루트에 현재 terminalViewMode에 대응하는 mode-* 클래스를 붙인다", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    const { container } = render(<TerminalOverlay />);
    const root = container.firstElementChild as HTMLElement;
    expect(root.className).toContain("mode-windowed");

    act(() => useAppStore.getState().setTerminalViewMode("filled"));
    expect(root.className).toContain("mode-filled");
    expect(root.className).not.toContain("mode-windowed");
  });

  it("모드 전환은 자식(TerminalHost/AgentTabStrip)을 리마운트하지 않는다(keep-alive)", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    render(<TerminalOverlay />);
    act(() => useAppStore.getState().openTerminal("a1"));
    expect(tabStripMount).toHaveBeenCalledTimes(1);
    expect(hostMount).toHaveBeenCalledTimes(1);

    act(() => useAppStore.getState().setTerminalViewMode("filled"));
    act(() => useAppStore.getState().setTerminalViewMode("windowed"));

    expect(tabStripMount).toHaveBeenCalledTimes(1);
    expect(hostMount).toHaveBeenCalledTimes(1);
    expect(tabStripUnmount).not.toHaveBeenCalled();
    expect(hostUnmount).not.toHaveBeenCalled();
  });
});

describe("backdrop mousedown close (third escape path, alongside X button / Cmd+W)", () => {
  it("mousedown directly on the overlay root (backdrop) closes the overlay", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    const { container } = render(<TerminalOverlay />);
    act(() => useAppStore.getState().openTerminal("a1"));

    const root = container.firstElementChild as HTMLElement;
    fireEvent.mouseDown(root);

    expect(useAppStore.getState().activeTerminalAgentId).toBeNull();
    expect(root.style.display).toBe("none");
  });

  it("mousedown inside the panel does not close the overlay", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    const { container } = render(<TerminalOverlay />);
    act(() => useAppStore.getState().openTerminal("a1"));

    const panel = container.querySelector(".terminal-overlay-panel") as HTMLElement;
    fireEvent.mouseDown(panel);

    expect(useAppStore.getState().activeTerminalAgentId).toBe("a1");
    const root = container.firstElementChild as HTMLElement;
    expect(root.style.display).not.toBe("none");
  });
});
