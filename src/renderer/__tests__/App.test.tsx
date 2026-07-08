// @vitest-environment jsdom
//
// src/renderer/__tests__/App.test.tsx
//
// TDD for `App`'s 4-layer z-stack assembly.
// Pure orchestration test — do the four layers exist in the DOM with the
// right structure, and does the BottomBar's "New Agent" button reach
// `ProfileDialog` through the real store (no store mocking here, same
// convention as the other 4x component tests)?
//
// `tauriApi` and `generateSpritePreview` are mocked for the same reasons
// `ProfileDialog.test.tsx` mocks them (no real Tauri runtime / no canvas in
// jsdom). `TerminalRegistry` is mocked for the same reason
// `TerminalHost.test.tsx` mocks it: once a test creates an agent, its
// session becomes non-`idle` and `TerminalHost` really attaches a mount to
// the registry, which for the real `@xterm/xterm` `Terminal.open()` needs
// canvas/`matchMedia` APIs jsdom doesn't implement.
//
// `../office/OfficeCanvas` is mocked out (no real Pixi
// canvas/WebGL context in jsdom -- that's `OfficeCanvas.test.tsx`'s job).
// This test only cares that `App` wires the real `officeBus`
// (`sessionBridge.ts`) and the store's live agent list (`useAgentList`)
// through to it as props.
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../store/appStore";

const generateSpritePreview = vi.fn((seed: string) => `data:image/png;base64,PREVIEW-${seed}`);
vi.mock("../office/gen/characterFactory", () => ({
  generateSpritePreview: (seed: string) => generateSpritePreview(seed),
}));

const createSession = vi.fn().mockResolvedValue({ sessionId: "s1", state: "starting" });
const clearNotifications = vi.fn();
vi.mock("../ipc/tauriApi", () => ({
  tauriApi: {
    createSession: (...args: unknown[]) => createSession(...args),
    resize: vi.fn(),
    clearNotifications: (...args: unknown[]) => clearNotifications(...args),
    onData: vi.fn(() => vi.fn()),
  },
}));

vi.mock("../terminal/TerminalRegistry", () => ({
  terminalRegistry: {
    attach: vi.fn(),
    activate: vi.fn(),
    refit: vi.fn(),
  },
}));

const officeCanvasProps = vi.fn();
vi.mock("../office/OfficeCanvas", () => ({
  OfficeCanvas: (props: { bus: unknown; profiles: unknown }) => {
    officeCanvasProps(props);
    return <div className="office-canvas-stub" />;
  },
}));

// The active mount installs a `ResizeObserver` (`TerminalHost.tsx`); jsdom
// doesn't implement it.
class FakeResizeObserver {
  observe = vi.fn();
  unobserve = vi.fn();
  disconnect = vi.fn();
}
vi.stubGlobal("ResizeObserver", FakeResizeObserver);

const { default: App } = await import("../App");

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
  officeCanvasProps.mockClear();
  clearNotifications.mockClear();
});

afterEach(() => cleanup());

describe("App shell", () => {
  it("renders the four z-stack layers", () => {
    const { container } = render(<App />);

    expect(container.querySelector(".app-root")).not.toBeNull();
    expect(container.querySelector(".office-canvas-stub")).not.toBeNull();
    expect(container.querySelector(".ui-chrome")).not.toBeNull();
    expect(container.querySelector(".terminal-overlay")).not.toBeNull();
    expect(container.querySelector(".modal-root")).not.toBeNull();
  });

  it("wires the real officeBus and the store's live agent list into OfficeCanvas", async () => {
    const { officeBus } = await import("../ipc/sessionBridge");
    render(<App />);

    expect(officeCanvasProps).toHaveBeenCalledTimes(1);
    const props = officeCanvasProps.mock.calls[0][0] as { bus: unknown; profiles: unknown[] };
    expect(props.bus).toBe(officeBus);
    expect(props.profiles).toEqual([]);
  });

  it("reflects a newly-added agent in the profiles passed to OfficeCanvas without re-mounting it", () => {
    render(<App />);
    expect(officeCanvasProps).toHaveBeenCalledTimes(1);

    act(() => {
      useAppStore.getState().addAgent({
        id: "a1",
        name: "Agent",
        role: "eng",
        note: "",
        seed: "seed-a1",
        createdAt: Date.now(),
        deskIndex: 0,
      });
    });

    const calls = officeCanvasProps.mock.calls;
    const lastProps = calls[calls.length - 1][0] as {
      profiles: Array<{ id: string }>;
    };
    expect(lastProps.profiles.map((p) => p.id)).toEqual(["a1"]);
  });

  it("does not re-render OfficeCanvas on unrelated store changes (stable useAgentList reference, no render loop)", () => {
    render(<App />);
    expect(officeCanvasProps).toHaveBeenCalledTimes(1);

    act(() => {
      useAppStore.getState().toggleMuted();
    });

    // `useAgentList`'s useShallow-wrapped selector doesn't include `muted`,
    // so this must not trigger another OfficeCanvas render -- if it fired
    // on every store update (or produced a fresh array each time an
    // unrelated slice changed) that would be the infinite-render trap this
    // task's brief calls out.
    expect(officeCanvasProps).toHaveBeenCalledTimes(1);
  });

  it("routes officeBus.emitAgentClicked through to openTerminal + tauriApi.clearNotifications", async () => {
    const { officeBus } = await import("../ipc/sessionBridge");
    render(<App />);

    act(() => {
      useAppStore.getState().addAgent({
        id: "a1",
        name: "Agent",
        role: "eng",
        note: "",
        seed: "seed-a1",
        createdAt: Date.now(),
        deskIndex: 0,
      });
    });

    act(() => {
      officeBus.emitAgentClicked("a1");
    });

    expect(useAppStore.getState().activeTerminalAgentId).toBe("a1");
    expect(clearNotifications).toHaveBeenCalledWith("a1");
  });

  it("keeps the terminal overlay mounted (display:none, not unmounted) when no terminal is open", () => {
    const { container } = render(<App />);

    const overlay = container.querySelector(".terminal-overlay") as HTMLElement;
    expect(overlay.style.display).toBe("none");
  });

  it("renders no modal by default, and opens ProfileDialog from BottomBar's New Agent button", () => {
    const { container, getByText } = render(<App />);

    expect(container.querySelector(".modal-backdrop")).toBeNull();

    fireEvent.click(getByText("＋ New Agent"));

    expect(container.querySelector(".modal-backdrop")).not.toBeNull();
    expect(useAppStore.getState().modal).toEqual({ kind: "profile-create" });
  });

  it("opens the terminal overlay when an agent is created (activeTerminalAgentId set on addAgent+openTerminal flow)", () => {
    // addAgent alone doesn't open the overlay (that's openTerminal's job,
    // e.g. via an office-canvas click) -- confirm the overlay reacts to the
    // store field App wires through TerminalOverlay, not to agent count.
    const { container } = render(<App />);
    const overlay = container.querySelector(".terminal-overlay") as HTMLElement;

    act(() => {
      useAppStore.getState().addAgent({
        id: "a1",
        name: "Agent",
        role: "eng",
        note: "",
        seed: "seed-a1",
        createdAt: Date.now(),
        deskIndex: 0,
      });
    });
    expect(overlay.style.display).toBe("none");

    act(() => {
      useAppStore.getState().openTerminal("a1");
    });
    expect(overlay.style.display).toBe("flex");
  });
});
