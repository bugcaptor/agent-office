// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Discussion } from "@shared/types";

vi.mock("../../ipc/tauriApi", () => ({ tauriApi: {} }));

import { createMockOfficeBus } from "../../office/bus";
import { useAppStore } from "../../store/appStore";
import { resetDiscussionStoreForTest, useDiscussionStore } from "../discussionStore";
import { MeetingBoard } from "../MeetingBoard";

const now = 1_800_000_000_000;
const agent = (id: string, name: string) => ({ id, name, role: "", seed: id, createdAt: 0, deskIndex: 0 });
const running: Discussion = {
  id: "d1", topic: "점심 메뉴", context: "", moderatorAgentId: "m",
  participants: [
    { agentId: "a", agent: { cli: "pi" } },
    { agentId: "m", agent: { cli: "claude" } },
    { agentId: "b", agent: { cli: "codex" } },
    { agentId: "c", agent: { cli: "claude" } },
  ],
  limits: { maxRounds: 3, maxMinutes: 15, maxCostUsd: 2, usageGuard: true }, state: "running", round: 2,
  transcript: [
    { at: now - 7 * 60_000, round: 1, kind: "opening", agentId: "a", text: "국밥이 무난하다" },
    { at: now - 30_000, round: 2, kind: "question", agentId: "m", text: "예산이 얼마인가요?" },
  ],
  run: { phase: "speakers", seen: {}, turns: [{ agentId: "b", sentAt: now - 3 * 60_000 }] },
  costUsd: 0, createdAt: now, updatedAt: now,
};

const rowText = (name: string) => screen.getByText(name).closest("li")!.textContent;

beforeEach(() => {
  vi.useFakeTimers({ now, toFake: ["Date", "setInterval", "clearInterval"] });
  // 이 jsdom 환경에는 localStorage가 없다 — 메모리 저장소로 대신한다.
  const store = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
  });
  resetDiscussionStoreForTest();
  useAppStore.setState({ agents: { a: agent("a", "타락천사"), m: agent("m", "시니어캣"), b: agent("b", "오버플로우") } });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("MeetingBoard", () => {
  it("진행 중인 토의가 없으면 그리지 않는다", () => {
    useDiscussionStore.setState({ discussions: [{ ...running, state: "review" }] });
    const { container } = render(<MeetingBoard bus={createMockOfficeBus()} />);
    expect(container.firstChild).toBeNull();
  });

  it("참가자마다 최근 발언·경과·말하는 중을 보이고, 30초마다 경과를 다시 센다", () => {
    useDiscussionStore.setState({ discussions: [running] });
    render(<MeetingBoard bus={createMockOfficeBus()} />);
    expect(screen.getByText("점심 메뉴")).toBeTruthy();
    expect(rowText("시니어캣")).toBe("시니어캣사회질문: 예산이 얼마인가요?방금");
    expect(rowText("타락천사")).toBe("타락천사국밥이 무난하다7분 전");
    expect(rowText("오버플로우")).toBe("오버플로우말하는 중 · 3분째");
    expect(rowText("삭제된 팀원")).toBe("삭제된 팀원아직 발언 없음");
    act(() => vi.advanceTimersByTime(60_000));
    expect(rowText("시니어캣")).toContain("1분 전");
    expect(rowText("오버플로우")).toContain("4분째");
  });

  it("회의실 옆에 붙고, 회의실이 없으면 화면 구석에 붙는다", () => {
    useDiscussionStore.setState({ discussions: [running] });
    const bus = createMockOfficeBus();
    bus.emitMeetingRoomRectChanged({ x: 100, y: 50, w: 240, h: 144 });
    render(<MeetingBoard bus={bus} />);
    const board = screen.getByRole("region", { name: "회의록" });
    expect([board.style.left, board.style.top, board.classList.contains("docked")]).toEqual(["348px", "50px", false]);
    act(() => bus.emitMeetingRoomRectChanged(null));
    expect([board.style.left, board.classList.contains("docked")]).toEqual(["", true]);
  });

  it("머리를 누르면 접히고, 접힘을 기억한다", () => {
    useDiscussionStore.setState({ discussions: [running] });
    const first = render(<MeetingBoard bus={createMockOfficeBus()} />);
    fireEvent.click(screen.getByRole("button", { expanded: true }));
    expect(screen.queryByText("국밥이 무난하다")).toBeNull();
    first.unmount();
    render(<MeetingBoard bus={createMockOfficeBus()} />);
    expect(screen.queryByText("국밥이 무난하다")).toBeNull();
    fireEvent.click(screen.getByRole("button", { expanded: false }));
    expect(screen.getByText("국밥이 무난하다")).toBeTruthy();
  });
});
