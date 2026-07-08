// @vitest-environment jsdom
//
// src/renderer/terminal/__tests__/AgentTabStrip.test.tsx
//
// Tests for `AgentTabStrip`.
//
// Coverage:
// - Renders one tab per `recentAgentIds` entry (LRU, most-recent-first —
//   the store's own tab-strip-order field), marking the active one.
// - Clicking a tab switches the active terminal (store.openTerminal), the
//   overlay/host stay keep-alive (that's TerminalOverlay's job, not tested
//   here).
// - Keyboard routing: Cmd/Ctrl+1..9 jumps to that tab index, Cmd/Ctrl+W
//   closes the overlay (both `preventDefault`ed so the browser/OS doesn't
//   also act on them), and Escape is deliberately left alone — no handler
//   claims it, so it can reach the shell/xterm underneath (vim etc. needs
//   real Escape).
// - Shortcuts are inert while the overlay is closed (no active terminal).
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

// `../../office/gen/characterFactory` is mocked because `generateSpritePreview`
// composes a real sprite sheet on a `document.createElement("canvas")` context
// that jsdom does not implement (same rationale as ProfileDialog's tests).
// Arg-recording spy: 폴백 프리뷰가 프로필 archetype으로 생성되는지 검증한다.
const generateSpritePreview = vi.fn(
  (..._args: unknown[]) => "data:image/png;base64,stub"
);
vi.mock("../../office/gen/characterFactory", () => ({
  generateSpritePreview: (...args: unknown[]) => generateSpritePreview(...args),
}));

const { AgentTabStrip } = await import("../AgentTabStrip");

function mkProfile(id: string): AgentProfile {
  return {
    id,
    name: `Agent ${id}`,
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
  generateSpritePreview.mockClear();
});

afterEach(() => cleanup());

/** Seeds 3 agents and opens them in an order that makes recentAgentIds = [a1, a2, a3]. */
function seedThreeTabs() {
  const s = useAppStore.getState();
  s.addAgent(mkProfile("a1"));
  s.addAgent(mkProfile("a2"));
  s.addAgent(mkProfile("a3"));
  s.openTerminal("a3");
  s.openTerminal("a2");
  s.openTerminal("a1"); // most-recent-first -> [a1, a2, a3]; active = a1
}

describe("tab rendering", () => {
  it("renders one tab per recentAgentIds entry, in LRU order, marking the active tab", () => {
    seedThreeTabs();
    const { getAllByRole } = render(<AgentTabStrip />);

    const tabs = getAllByRole("tab");
    expect(tabs.map((t) => t.textContent)).toEqual(["Agent a1", "Agent a2", "Agent a3"]);
    expect(tabs[0].getAttribute("aria-selected")).toBe("true");
    expect(tabs[1].getAttribute("aria-selected")).toBe("false");
  });

  it("renders no tabs when nothing has ever been opened", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    const { queryAllByRole } = render(<AgentTabStrip />);
    expect(queryAllByRole("tab")).toHaveLength(0);
  });
});

describe("썸네일 폴백의 archetype 반영", () => {
  it("non-human archetype 프로필의 폴백 프리뷰는 해당 archetype으로 생성된다", () => {
    const s = useAppStore.getState();
    s.addAgent({ ...mkProfile("a1"), archetype: "orc" });
    s.openTerminal("a1");

    render(<AgentTabStrip />);

    // 월드(characterFactory.createCharacterAssets)와 동일하게
    // resolveArchetype(profile.archetype, seed) 결과가 전달되어야 한다.
    expect(generateSpritePreview).toHaveBeenCalledWith(
      "a1",
      6,
      undefined,
      undefined,
      "orc"
    );
  });

  it("archetype 미지정 프로필은 human으로 폴백된다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.openTerminal("a1");

    render(<AgentTabStrip />);

    expect(generateSpritePreview).toHaveBeenCalledWith(
      "a1",
      6,
      undefined,
      undefined,
      "human"
    );
  });
});

describe("click switching", () => {
  it("clicking a tab makes it active without touching the others", () => {
    seedThreeTabs();
    const { getAllByRole } = render(<AgentTabStrip />);

    fireEvent.click(getAllByRole("tab")[2]); // "Agent a3"

    expect(useAppStore.getState().activeTerminalAgentId).toBe("a3");
  });

  it("the close (X) button closes the overlay", () => {
    seedThreeTabs();
    const { getByRole } = render(<AgentTabStrip />);

    fireEvent.click(getByRole("button", { name: /close/i }));

    expect(useAppStore.getState().activeTerminalAgentId).toBeNull();
  });
});

describe("keyboard routing while the overlay is open", () => {
  it("Cmd+2 / Ctrl+2 switches to the 2nd tab", () => {
    seedThreeTabs();
    render(<AgentTabStrip />);

    fireEvent.keyDown(window, { key: "2", metaKey: true });
    expect(useAppStore.getState().activeTerminalAgentId).toBe("a2");

    act(() => useAppStore.getState().openTerminal("a1")); // reset
    fireEvent.keyDown(window, { key: "2", ctrlKey: true });
    expect(useAppStore.getState().activeTerminalAgentId).toBe("a2");
  });

  it("a digit beyond the tab count is a no-op (no crash, no state change)", () => {
    seedThreeTabs();
    render(<AgentTabStrip />);

    fireEvent.keyDown(window, { key: "9", metaKey: true });
    expect(useAppStore.getState().activeTerminalAgentId).toBe("a1");
  });

  it("Cmd+W / Ctrl+W closes the overlay and prevents the default browser action", () => {
    seedThreeTabs();
    render(<AgentTabStrip />);

    const event = new KeyboardEvent("keydown", { key: "w", metaKey: true, cancelable: true });
    act(() => {
      window.dispatchEvent(event);
    });

    expect(event.defaultPrevented).toBe(true);
    expect(useAppStore.getState().activeTerminalAgentId).toBeNull();
  });

  it("digit shortcuts also preventDefault (so the OS/browser doesn't switch its own tabs)", () => {
    seedThreeTabs();
    render(<AgentTabStrip />);

    const event = new KeyboardEvent("keydown", { key: "2", metaKey: true, cancelable: true });
    act(() => {
      window.dispatchEvent(event);
    });

    expect(event.defaultPrevented).toBe(true);
  });

  it("Escape is left untouched: no preventDefault, and it does not close the overlay", () => {
    seedThreeTabs();
    render(<AgentTabStrip />);

    const event = new KeyboardEvent("keydown", { key: "Escape", cancelable: true });
    act(() => {
      window.dispatchEvent(event);
    });

    expect(event.defaultPrevented).toBe(false);
    expect(useAppStore.getState().activeTerminalAgentId).toBe("a1"); // unchanged, overlay still open
  });

  it("a bare digit (no Cmd/Ctrl) does nothing — it's plain terminal input", () => {
    seedThreeTabs();
    render(<AgentTabStrip />);

    fireEvent.keyDown(window, { key: "2" });
    expect(useAppStore.getState().activeTerminalAgentId).toBe("a1");
  });
});

describe("keyboard routing while the overlay is closed", () => {
  it("shortcuts are inert when there is no active terminal", () => {
    seedThreeTabs();
    useAppStore.getState().closeTerminal();
    render(<AgentTabStrip />);

    fireEvent.keyDown(window, { key: "2", metaKey: true });
    fireEvent.keyDown(window, { key: "w", metaKey: true });

    expect(useAppStore.getState().activeTerminalAgentId).toBeNull();
  });
});

describe("cleanup", () => {
  it("removes its keydown listener on unmount (no leaks across mounts)", () => {
    seedThreeTabs();
    const { unmount } = render(<AgentTabStrip />);
    unmount();

    // After unmount, shortcuts must not fire (would throw/act-warn on state
    // updates outside React if the listener leaked, and more importantly
    // must not flip state).
    fireEvent.keyDown(window, { key: "w", metaKey: true });
    expect(useAppStore.getState().activeTerminalAgentId).toBe("a1");
  });
});

describe("탭 우클릭 컨텍스트 메뉴", () => {
  it("탭을 우클릭하면 메뉴가 뜨고 '프로필 편집' 선택 시 편집 모달을 열고 메뉴는 닫힌다", () => {
    seedThreeTabs();
    const { getAllByRole, getByRole, queryByRole } = render(<AgentTabStrip />);

    fireEvent.contextMenu(getAllByRole("tab")[0]); // "Agent a1"
    fireEvent.click(getByRole("menuitem", { name: "프로필 편집" }));

    expect(useAppStore.getState().modal).toEqual({
      kind: "profile-edit",
      agentId: "a1",
    });
    // 선택 후 메뉴는 닫힌다.
    expect(queryByRole("menu")).toBeNull();
  });

  it("메뉴 밖 mousedown으로 메뉴가 닫힌다", () => {
    seedThreeTabs();
    const { getAllByRole, getByRole, queryByRole } = render(<AgentTabStrip />);

    fireEvent.contextMenu(getAllByRole("tab")[0]);
    expect(getByRole("menu")).toBeTruthy();

    fireEvent.mouseDown(document.body);
    expect(queryByRole("menu")).toBeNull();
  });

  it("'캐릭터 삭제' 선택 시 confirm-delete 모달을 열고 메뉴는 닫힌다", () => {
    seedThreeTabs();
    const { getAllByRole, getByRole, queryByRole } = render(<AgentTabStrip />);

    fireEvent.contextMenu(getAllByRole("tab")[0]); // "Agent a1"
    fireEvent.click(getByRole("menuitem", { name: "캐릭터 삭제" }));

    expect(useAppStore.getState().modal).toEqual({
      kind: "confirm-delete",
      agentId: "a1",
    });
    expect(queryByRole("menu")).toBeNull();
  });

  it("'터미널 재시작'이 첫 항목으로 보이고 선택 시 confirm-restart 모달을 열고 메뉴는 닫힌다", () => {
    seedThreeTabs();
    const { getAllByRole, getByRole, queryByRole } = render(<AgentTabStrip />);

    fireEvent.contextMenu(getAllByRole("tab")[0]); // "Agent a1"
    const items = getAllByRole("menuitem");
    expect(items[0].textContent).toBe("터미널 재시작");

    fireEvent.click(getByRole("menuitem", { name: "터미널 재시작" }));

    expect(useAppStore.getState().modal).toEqual({
      kind: "confirm-restart",
      agentId: "a1",
    });
    expect(queryByRole("menu")).toBeNull();
  });
});
