// @vitest-environment jsdom
//
// src/renderer/layout/__tests__/BottomBar.test.tsx
//
// Coverage:
// - "New Agent" opens the profile-create modal (already covered by
//   App.test.tsx too, kept minimal here).
// - "출근" (🏠) button shows the clocked-out count and is disabled at 0.
// - Clicking "출근" opens a menu listing clocked-out agents by name;
//   selecting one calls `clockInAgent(agent.id)`.
// - The "전체 출퇴근" bulk button opens a menu with "전체 출근" (calls
//   `clockInAll` directly) and "전체 퇴근" (opens the `confirm-clock-out-all`
//   modal; the actual clockOutAll call is ConfirmClockOutDialog's
//   responsibility). Items are disabled when their target set is empty.
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

const clockInAgent = vi.fn();
const clockInAll = vi.fn();
vi.mock("../../agent/clockOut", () => ({
  clockInAgent: (...args: unknown[]) => clockInAgent(...args),
  clockInAll: (...args: unknown[]) => clockInAll(...args),
}));

const { BottomBar } = await import("../BottomBar");

function mkProfile(id: string, overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id,
    name: `Agent ${id}`,
    role: "eng",
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
  clockInAll.mockClear();
});

afterEach(() => cleanup());

describe("New Agent", () => {
  it("opens the profile-create modal", () => {
    const { getByText } = render(<BottomBar />);
    fireEvent.click(getByText("＋ New Agent"));
    expect(useAppStore.getState().modal).toEqual({ kind: "profile-create" });
  });
});

describe("출근 버튼(🏠)", () => {
  it("퇴근한 에이전트가 없으면 카운트 0, 비활성", () => {
    const { getByRole } = render(<BottomBar />);
    const btn = getByRole("button", { name: /🏠/ }) as HTMLButtonElement;
    expect(btn.textContent).toContain("0");
    expect(btn.disabled).toBe(true);
  });

  it("퇴근한 에이전트 수를 배지로 표시하고 클릭하면 메뉴가 뜬다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.clockOut("a1");
    const { getByRole } = render(<BottomBar />);

    const btn = getByRole("button", { name: /🏠/ }) as HTMLButtonElement;
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

    fireEvent.click(getByRole("button", { name: /🏠/ }));
    fireEvent.click(getByRole("menuitem", { name: "Agent a1" }));

    expect(clockInAgent).toHaveBeenCalledWith("a1");
    expect(queryByRole("menu")).toBeNull();
  });
});

describe("테마 드롭다운", () => {
  it("클릭하면 전 테마가 메뉴로 뜨고, 고르면 그 테마로 바뀌며 메뉴가 닫힌다", async () => {
    const { THEMES, THEME_ORDER, DEFAULT_THEME_ID } = await import("../../theme/themes");
    const { getByRole, queryByRole } = render(<BottomBar />);

    const btn = getByRole("button", { name: "테마 선택" });
    expect(btn.textContent).toContain(THEMES[DEFAULT_THEME_ID].label);

    fireEvent.click(btn);
    for (const id of THEME_ORDER) {
      // 현재 테마는 체크 아이콘이 붙으므로 이름이 라벨과 정확히 같지 않다.
      expect(getByRole("menuitem", { name: new RegExp(THEMES[id].label) })).toBeTruthy();
    }

    fireEvent.click(getByRole("menuitem", { name: /핍보이/ }));
    expect(useAppStore.getState().theme).toBe("pipboy");
    expect(queryByRole("menu")).toBeNull();

    useAppStore.getState().setTheme(DEFAULT_THEME_ID); // 모듈 전역 DOM/영속 원복
  });
});

describe("정보 버튼(ℹ)", () => {
  it("클릭하면 about 모달을 연다", () => {
    const { getByRole } = render(<BottomBar />);
    fireEvent.click(getByRole("button", { name: "정보" }));
    expect(useAppStore.getState().modal).toEqual({ kind: "about" });
  });
});

describe("전체 출퇴근 버튼", () => {
  it("에이전트가 하나도 없으면 버튼이 비활성", () => {
    const { getByRole } = render(<BottomBar />);
    const btn = getByRole("button", { name: "전체 출퇴근" }) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("클릭하면 전체 출근/전체 퇴근 두 항목이 메뉴로 뜬다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.addAgent(mkProfile("a2"));
    s.clockOut("a2");
    const { getByRole } = render(<BottomBar />);

    fireEvent.click(getByRole("button", { name: "전체 출퇴근" }));
    expect(getByRole("menuitem", { name: /전체 출근/ })).toBeTruthy();
    expect(getByRole("menuitem", { name: /전체 퇴근/ })).toBeTruthy();
  });

  it("전체 퇴근을 고르면 confirm-clock-out-all 모달이 열리고 메뉴는 닫힌다", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    const { getByRole, queryByRole } = render(<BottomBar />);

    fireEvent.click(getByRole("button", { name: "전체 출퇴근" }));
    fireEvent.click(getByRole("menuitem", { name: /전체 퇴근/ }));

    expect(useAppStore.getState().modal).toEqual({ kind: "confirm-clock-out-all" });
    expect(queryByRole("menu")).toBeNull();
  });

  it("전체 출근을 고르면 clockInAll을 호출한다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.clockOut("a1");
    const { getByRole } = render(<BottomBar />);

    fireEvent.click(getByRole("button", { name: "전체 출퇴근" }));
    fireEvent.click(getByRole("menuitem", { name: /전체 출근/ }));

    expect(clockInAll).toHaveBeenCalledTimes(1);
  });

  it("근무 중이 0명이면 전체 퇴근 항목이, 퇴근자가 0명이면 전체 출근 항목이 비활성", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1")); // 근무 중 1, 퇴근 0
    const { getByRole } = render(<BottomBar />);

    fireEvent.click(getByRole("button", { name: "전체 출퇴근" }));
    const inItem = getByRole("menuitem", { name: /전체 출근/ }) as HTMLButtonElement;
    const outItem = getByRole("menuitem", { name: /전체 퇴근/ }) as HTMLButtonElement;
    expect(inItem.disabled).toBe(true);
    expect(outItem.disabled).toBe(false);
  });
});

describe("동료 대화 표시(TalkWidget)", () => {
  it("talkEnabled가 꺼져 있으면 하단바에 나타나지 않는다", () => {
    const { queryByLabelText } = render(<BottomBar />);
    expect(queryByLabelText("동료 대화")).toBeNull();
  });

  it("켜져 있으면 하단바에 나타난다", () => {
    useAppStore.setState({
      appSettings: { ...initialState.appSettings, talkEnabled: true },
    });
    const { getByLabelText } = render(<BottomBar />);
    expect(getByLabelText("동료 대화")).toBeTruthy();
  });
});
