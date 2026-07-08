// @vitest-environment jsdom
//
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { SessionTimePanel } from "../SessionTimePanel";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

const initial = useAppStore.getState();

function mkProfile(id: string, name: string): AgentProfile {
  return { id, name, role: "backend", note: "", seed: id, createdAt: 0, deskIndex: 0 };
}

beforeEach(() => useAppStore.setState(initial, true));
afterEach(cleanup);

describe("SessionTimePanel", () => {
  it("renders one row per agent with name and cumulative summary", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "Ada"));
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 0 });
    s.applyNotificationTiming({
      id: "n1", sessionId: "s1", agentId: "a1", source: "stop",
      message: "done", dedupKey: "k", at: 90_000,
    });

    render(<SessionTimePanel />);
    expect(screen.getByText("Ada")).toBeTruthy();
    // 총 90s = "1m 30s", 1턴
    expect(screen.getByText(/총 1m 30s/)).toBeTruthy();
    expect(screen.getByText(/1턴/)).toBeTruthy();
  });

  it("shows the working status icon for an open turn", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "Ada"));
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 0 });

    render(<SessionTimePanel />);
    // 작업중 아이콘 ●
    expect(screen.getByText("●")).toBeTruthy();
  });

  it("collapses and expands when the toggle is clicked", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "Ada"));
    render(<SessionTimePanel />);

    expect(screen.queryByText("Ada")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /접기|펼치기/ }));
    expect(screen.queryByText("Ada")).toBeNull(); // 접힘: 행 숨김
  });
});
