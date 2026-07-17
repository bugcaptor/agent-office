// @vitest-environment jsdom
//
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// 턴 정산(prompt → stop)이 appendSessionTurn을 호출하므로 실 tauriApi(invoke)를
// 타지 않도록 모킹(다른 시간추적 테스트와 동일 컨벤션). 통계 뷰 테스트는
// loadSessionTurns도 모킹해 useAgentStats가 이 모의 API를 사용하도록 한다.
const { mockApi } = vi.hoisted(() => ({
  mockApi: { appendSessionTurn: vi.fn(), loadSessionTurns: vi.fn() },
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: mockApi }));

import { SessionTimePanel } from "../SessionTimePanel";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

const initial = useAppStore.getState();

function mkProfile(id: string, name: string): AgentProfile {
  return { id, name, role: "backend", note: "", seed: id, createdAt: 0, deskIndex: 0 };
}

beforeEach(() => useAppStore.setState(initial, true));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

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
    // 통계 토글("통계 펼치기")도 /펼치기/에 매치되므로 패널 접기/펼치기 토글만 정확히 지정.
    fireEvent.click(screen.getByRole("button", { name: "접기" }));
    expect(screen.queryByText("Ada")).toBeNull(); // 접힘: 행 숨김
  });
});

describe("SessionTimePanel stats view", () => {
  it("hides the stats list until the 통계 toggle is clicked, then shows rows", async () => {
    mockApi.loadSessionTurns.mockResolvedValue([
      { agentId: "a", startedAt: 0, endedAt: Date.now(), totalMs: 100, workedMs: 500, waitedMs: 0 },
    ]);
    useAppStore.setState({ agents: { a: { name: "Alice" } } } as never);
    render(<SessionTimePanel />);
    expect(screen.queryByText("Alice")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /통계 펼치기/ }));
    await waitFor(() => expect(screen.getByText("Alice")).toBeTruthy());
  });

  it("shows an error message with a retry button when loading fails", async () => {
    mockApi.loadSessionTurns.mockRejectedValue(new Error("boom"));
    render(<SessionTimePanel />);
    fireEvent.click(screen.getByRole("button", { name: /통계 펼치기/ }));
    await waitFor(() => expect(screen.getByText(/불러오지 못했습니다/)).toBeTruthy());
    expect(screen.getByRole("button", { name: "다시 시도" })).toBeTruthy();
  });
});
