// @vitest-environment jsdom
//
// src/renderer/layout/__tests__/BottomBar.test.tsx
//
// Coverage:
// - "New Agent" opens the profile-create modal (already covered by
//   App.test.tsx too, kept minimal here).
// - "소환" button shows the clocked-out count and is disabled at 0.
// - Clicking "소환" opens a menu listing clocked-out agents by name;
//   selecting one calls `clockInAgent(agent.id)`.
// - "전체 퇴근" is disabled when there are no on-duty agents, and opens the
//   `confirm-clock-out-all` modal when clicked (actual clockOutAll call is
//   ConfirmClockOutDialog's responsibility, covered in its own test file).
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

const clockInAgent = vi.fn();
vi.mock("../../agent/clockOut", () => ({
  clockInAgent: (...args: unknown[]) => clockInAgent(...args),
}));

const { BottomBar } = await import("../BottomBar");

function mkProfile(id: string, overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id,
    name: `Agent ${id}`,
    role: "eng",
    note: "",
    seed: id,
    createdAt: Date.now(),
    deskIndex: 0,
    ...overrides,
  };
}

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
  clockInAgent.mockClear();
});

afterEach(() => cleanup());

describe("New Agent", () => {
  it("opens the profile-create modal", () => {
    const { getByText } = render(<BottomBar />);
    fireEvent.click(getByText("＋ New Agent"));
    expect(useAppStore.getState().modal).toEqual({ kind: "profile-create" });
  });
});

describe("소환 버튼", () => {
  it("퇴근한 에이전트가 없으면 카운트 0, 비활성", () => {
    const { getByRole } = render(<BottomBar />);
    const btn = getByRole("button", { name: /소환/ }) as HTMLButtonElement;
    expect(btn.textContent).toContain("0");
    expect(btn.disabled).toBe(true);
  });

  it("퇴근한 에이전트 수를 배지로 표시하고 클릭하면 메뉴가 뜬다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.clockOut("a1");
    const { getByRole } = render(<BottomBar />);

    const btn = getByRole("button", { name: /소환/ }) as HTMLButtonElement;
    expect(btn.textContent).toContain("1");
    expect(btn.disabled).toBe(false);

    fireEvent.click(btn);
    expect(getByRole("menuitem", { name: "Agent a1" })).toBeTruthy();
  });

  it("메뉴에서 에이전트를 선택하면 clockInAgent(agentId)가 호출되고 메뉴는 닫힌다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.clockOut("a1");
    const { getByRole, queryByRole } = render(<BottomBar />);

    fireEvent.click(getByRole("button", { name: /소환/ }));
    fireEvent.click(getByRole("menuitem", { name: "Agent a1" }));

    expect(clockInAgent).toHaveBeenCalledWith("a1");
    expect(queryByRole("menu")).toBeNull();
  });
});

describe("전체 퇴근 버튼", () => {
  it("근무 중인 에이전트가 없으면 비활성", () => {
    const { getByRole } = render(<BottomBar />);
    const btn = getByRole("button", { name: "전체 퇴근" }) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("근무 중인 에이전트가 있으면 활성화되고 클릭하면 confirm-clock-out-all 모달을 연다", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    const { getByRole } = render(<BottomBar />);

    const btn = getByRole("button", { name: "전체 퇴근" }) as HTMLButtonElement;
    expect(btn.disabled).toBe(false);

    fireEvent.click(btn);
    expect(useAppStore.getState().modal).toEqual({ kind: "confirm-clock-out-all" });
  });

  it("전부 퇴근한 상태(근무 중 0명)면 비활성", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.clockOut("a1");
    const { getByRole } = render(<BottomBar />);

    const btn = getByRole("button", { name: "전체 퇴근" }) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });
});
