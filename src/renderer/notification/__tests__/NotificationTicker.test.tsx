// @vitest-environment jsdom
//
// src/renderer/notification/__tests__/NotificationTicker.test.tsx
//
// TDD for `NotificationTicker`.
//
// `../../ipc/sessionBridge` is mocked so a click's routing
// (`officeBus.emitAgentClicked`) can be asserted without a real Tauri
// runtime or the rest of the bridge's IPC wiring.
//
// Coverage:
// - Renders newest-first, one card per agent (dedupe), capped at 5 visible
//   cards with a "+N more" overflow card for the rest.
// - Card click routes through `officeBus.emitAgentClicked(agentId)` (which
//   itself opens the terminal + clears backend notifications — see
//   `sessionBridge.test.ts`; not re-tested here).
// - Card content: agent name (falls back to id when the profile is gone),
//   excerpt text, and type icon.
// - Renders nothing but the (empty) container when there are no
//   notifications.
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile, Notification } from "../../store/types";

const emitAgentClicked = vi.fn();

vi.mock("../../ipc/sessionBridge", () => ({
  officeBus: {
    emitAgentClicked: (...args: unknown[]) => emitAgentClicked(...args),
    onNotificationChanged: vi.fn(() => () => {}),
    onSessionStateChanged: vi.fn(() => () => {}),
  },
}));

const { NotificationTicker } = await import("../NotificationTicker");

function mkProfile(id: string, name?: string): AgentProfile {
  return {
    id,
    name: name ?? `Agent ${id}`,
    role: "eng",
    note: "",
    seed: id,
    createdAt: Date.now(),
    deskIndex: 0,
  };
}

let seq = 0;
function mkNotif(overrides: Partial<Notification> = {}): Notification {
  seq += 1;
  return {
    id: `n${seq}`,
    agentId: "a1",
    type: "info",
    message: "hello",
    excerpt: "hello",
    createdAt: Date.now(),
    ...overrides,
  };
}

const initialState = useAppStore.getState();

beforeEach(() => {
  seq = 0;
  useAppStore.setState(initialState, true);
  emitAgentClicked.mockClear();
});

afterEach(() => cleanup());

describe("rendering", () => {
  it("renders nothing but an empty container when there are no notifications", () => {
    const { container } = render(<NotificationTicker />);
    expect(container.querySelectorAll(".ticker-card")).toHaveLength(0);
    expect(container.querySelectorAll(".ticker-overflow")).toHaveLength(0);
  });

  it("dedupes to one card per agent and shows newest first", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.getState().addAgent(mkProfile("a2"));
    // Store keeps `notifications` newest-first (see appStore.test.ts) —
    // seed it directly in that shape rather than via pushNotification, which
    // would suppress a1 as soon as a2 becomes active elsewhere.
    useAppStore.setState({
      notifications: [
        mkNotif({ id: "n-a1-new", agentId: "a1", createdAt: 300, excerpt: "a1 latest" }),
        mkNotif({ id: "n-a2", agentId: "a2", createdAt: 250, excerpt: "a2 only" }),
        mkNotif({ id: "n-a1-old", agentId: "a1", createdAt: 100, excerpt: "a1 stale" }),
      ],
    });

    const { getAllByRole } = render(<NotificationTicker />);
    const cards = getAllByRole("button");

    expect(cards).toHaveLength(2);
    expect(cards[0].textContent).toContain("Agent a1");
    expect(cards[0].textContent).toContain("a1 latest");
    expect(cards[0].textContent).not.toContain("a1 stale");
    expect(cards[1].textContent).toContain("Agent a2");
  });

  it("caps visible cards at 5 and shows a +N more overflow card for the rest", () => {
    const notifications: Notification[] = [];
    for (let i = 0; i < 7; i += 1) {
      const id = `a${i}`;
      useAppStore.getState().addAgent(mkProfile(id));
      notifications.push(mkNotif({ id: `n${i}`, agentId: id, createdAt: 1000 - i }));
    }
    useAppStore.setState({ notifications });

    const { getAllByRole, getByText } = render(<NotificationTicker />);

    expect(getAllByRole("button")).toHaveLength(5);
    expect(getByText("+2 more")).toBeTruthy();
  });

  it("falls back to the raw agentId when the profile is missing", () => {
    useAppStore.setState({ notifications: [mkNotif({ agentId: "ghost" })] });

    const { getByRole } = render(<NotificationTicker />);

    expect(getByRole("button").textContent).toContain("ghost");
  });

  it("shows the type icon for each notification type", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.getState().addAgent(mkProfile("a2"));
    useAppStore.getState().addAgent(mkProfile("a3"));
    useAppStore.setState({
      notifications: [
        mkNotif({ agentId: "a1", type: "question", createdAt: 3 }),
        mkNotif({ agentId: "a2", type: "done", createdAt: 2 }),
        mkNotif({ agentId: "a3", type: "info", createdAt: 1 }),
      ],
    });

    const { getAllByRole } = render(<NotificationTicker />);
    const cards = getAllByRole("button");

    expect(cards[0].textContent).toContain("❓");
    expect(cards[1].textContent).toContain("✅");
    expect(cards[2].textContent).toContain("ℹ️");
  });

  it("renders provider-neutral observer fallback copy exactly", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.getState().addAgent(mkProfile("a2"));
    useAppStore.setState({
      notifications: [
        mkNotif({
          agentId: "a1",
          type: "question",
          excerpt: "확인이 필요합니다",
          createdAt: 2,
        }),
        mkNotif({
          agentId: "a2",
          type: "done",
          excerpt: "작업이 완료되었습니다.",
          createdAt: 1,
        }),
      ],
    });

    const { getAllByRole } = render(<NotificationTicker />);
    const rendered = getAllByRole("button")
      .map((card) => card.textContent ?? "")
      .join("\n");

    expect(rendered).toContain("확인이 필요합니다");
    expect(rendered).toContain("작업이 완료되었습니다.");
    expect(rendered).not.toContain("Claude");
    expect(rendered).not.toContain("Codex");
  });
});

describe("click routing", () => {
  it("clicking a card routes through officeBus.emitAgentClicked(agentId)", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.setState({ notifications: [mkNotif({ agentId: "a1" })] });

    const { getByRole } = render(<NotificationTicker />);
    fireEvent.click(getByRole("button"));

    expect(emitAgentClicked).toHaveBeenCalledTimes(1);
    expect(emitAgentClicked).toHaveBeenCalledWith("a1");
  });

  it("clicking the overflow card is a no-op (it has no click handler)", () => {
    const notifications: Notification[] = [];
    for (let i = 0; i < 6; i += 1) {
      const id = `a${i}`;
      useAppStore.getState().addAgent(mkProfile(id));
      notifications.push(mkNotif({ id: `n${i}`, agentId: id, createdAt: 1000 - i }));
    }
    useAppStore.setState({ notifications });

    const { getByText } = render(<NotificationTicker />);
    fireEvent.click(getByText("+1 more"));

    expect(emitAgentClicked).not.toHaveBeenCalled();
  });
});

describe("relative time", () => {
  it("shows 방금 for a just-now notification", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.setState({ notifications: [mkNotif({ agentId: "a1", createdAt: Date.now() })] });

    const { getByRole } = render(<NotificationTicker />);
    expect(getByRole("button").textContent).toContain("방금");
  });

  it("shows a minute-scale relative time for an older notification", () => {
    vi.useFakeTimers();
    try {
      const now = new Date("2026-01-01T00:10:00Z").getTime();
      vi.setSystemTime(now);
      useAppStore.getState().addAgent(mkProfile("a1"));
      useAppStore.setState({
        notifications: [mkNotif({ agentId: "a1", createdAt: now - 5 * 60 * 1000 })],
      });

      let result: ReturnType<typeof render>;
      act(() => {
        result = render(<NotificationTicker />);
      });

      expect(result!.getByRole("button").textContent).toContain("5분 전");
    } finally {
      vi.useRealTimers();
    }
  });
});
