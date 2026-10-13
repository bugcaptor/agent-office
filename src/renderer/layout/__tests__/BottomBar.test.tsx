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
//   `clockInAll` directly), "전체 자리로" (calls `summonAllToDesk` for
//   on-duty agents with no live session) and "전체 퇴근" (opens the
//   `confirm-clock-out-all` modal; the actual clockOutAll call is
//   ConfirmClockOutDialog's responsibility). Items are disabled when their
//   target set is empty.
// - "📊 기록" merges 분석/우수사원.
// - "🎨 {풍경}·{테마}"는 풍경/테마 두 값을 한 버튼에 보여주고, 메뉴는
//   "풍경"/"테마" 섹션 헤더로 나뉜다.
import { t } from "@renderer/i18n";
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

const summonAllToDesk = vi.fn();
vi.mock("../../agent/summonToDesk", () => ({
  summonAllToDesk: (...args: unknown[]) => summonAllToDesk(...args),
}));

const setAppSettings = vi.fn((_settings: unknown) => Promise.resolve());
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    setAppSettings: (settings: unknown) => setAppSettings(settings),
  },
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
  summonAllToDesk.mockClear();
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

describe("풍경·테마 통합 버튼", () => {
  it("라벨에 현재 풍경·테마 값을 둘 다 보여준다", async () => {
    const { THEMES, DEFAULT_THEME_ID } = await import("../../theme/themes");
    const { SCENES, DEFAULT_SCENE_ID } = await import("../../office/scenes/scenes");
    const { getByRole } = render(<BottomBar />);

    const btn = getByRole("button", { name: "풍경·테마 선택" });
    expect(btn.textContent).toContain(t(SCENES[DEFAULT_SCENE_ID].labelKey));
    expect(btn.textContent).toContain(t(THEMES[DEFAULT_THEME_ID].labelKey));
  });

  it("메뉴가 '풍경'/'테마' 섹션 헤더로 나뉘고, 각 섹션에 값이 모두 나온다", async () => {
    const { THEME_ORDER } = await import("../../theme/themes");
    const { SCENES, SCENE_ORDER } = await import("../../office/scenes/scenes");
    const { THEMES } = await import("../../theme/themes");
    const { getByRole, getByText } = render(<BottomBar />);

    fireEvent.click(getByRole("button", { name: "풍경·테마 선택" }));

    expect(getByText("풍경")).toBeTruthy();
    expect(getByText("테마")).toBeTruthy();
    for (const id of SCENE_ORDER) {
      expect(getByRole("menuitem", { name: new RegExp(t(SCENES[id].labelKey)) })).toBeTruthy();
    }
    for (const id of THEME_ORDER) {
      // 현재 테마는 체크 아이콘이 붙으므로 이름이 라벨과 정확히 같지 않다.
      expect(getByRole("menuitem", { name: new RegExp(t(THEMES[id].labelKey)) })).toBeTruthy();
    }
  });

  it("테마를 고르면 setTheme이 불리고 메뉴가 닫힌다", async () => {
    const { DEFAULT_THEME_ID } = await import("../../theme/themes");
    const { getByRole, queryByRole } = render(<BottomBar />);

    fireEvent.click(getByRole("button", { name: "풍경·테마 선택" }));
    fireEvent.click(getByRole("menuitem", { name: /핍보이/ }));

    expect(useAppStore.getState().theme).toBe("pipboy");
    expect(queryByRole("menu")).toBeNull();

    useAppStore.getState().setTheme(DEFAULT_THEME_ID); // 모듈 전역 DOM/영속 원복
  });

  it("풍경을 고르면 setScene이 불리고 메뉴가 닫힌다", async () => {
    const { SCENES, SCENE_ORDER, DEFAULT_SCENE_ID } = await import("../../office/scenes/scenes");
    const other = SCENE_ORDER.find((id) => id !== DEFAULT_SCENE_ID);
    if (!other) throw new Error("테스트용 대체 풍경이 없다");
    const { getByRole, queryByRole } = render(<BottomBar />);

    fireEvent.click(getByRole("button", { name: "풍경·테마 선택" }));
    fireEvent.click(getByRole("menuitem", { name: new RegExp(t(SCENES[other].labelKey)) }));

    expect(useAppStore.getState().scene).toBe(other);
    expect(queryByRole("menu")).toBeNull();

    useAppStore.getState().setScene(DEFAULT_SCENE_ID); // 원복
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

  it("클릭하면 전체 출근/전체 자리로/전체 퇴근 세 항목이 메뉴로 뜬다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.addAgent(mkProfile("a2"));
    s.clockOut("a2");
    const { getByRole } = render(<BottomBar />);

    fireEvent.click(getByRole("button", { name: "전체 출퇴근" }));
    expect(getByRole("menuitem", { name: /전체 출근/ })).toBeTruthy();
    expect(getByRole("menuitem", { name: /전체 자리로/ })).toBeTruthy();
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

  it("전체 자리로는 세션 없는 근무자 수를 배지로 보여주고 summonAllToDesk를 부른다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.setSessionState({ agentId: "a1", status: "exited" }); // 세션 없음 → 탕비실
    s.addAgent(mkProfile("a2"));
    s.setSessionState({ agentId: "a2", status: "running" }); // 자리에 앉아 있음
    s.addAgent(mkProfile("a3"));
    s.clockOut("a3"); // 퇴근자는 대상이 아니다
    const { getByRole } = render(<BottomBar />);

    fireEvent.click(getByRole("button", { name: "전체 출퇴근" }));
    const item = getByRole("menuitem", { name: /전체 자리로/ }) as HTMLButtonElement;
    expect(item.textContent).toContain("1");
    expect(item.disabled).toBe(false);

    fireEvent.click(item);
    expect(summonAllToDesk).toHaveBeenCalledTimes(1);
  });

  it("탕비실에 아무도 없으면 전체 자리로 항목이 비활성", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.setSessionState({ agentId: "a1", status: "running" });
    const { getByRole } = render(<BottomBar />);

    fireEvent.click(getByRole("button", { name: "전체 출퇴근" }));
    expect((getByRole("menuitem", { name: /전체 자리로/ }) as HTMLButtonElement).disabled).toBe(
      true,
    );
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

describe("기록 버튼(📊, 분석·우수사원 통합)", () => {
  it("분석/우수사원을 고르면 각각 모달을 열고 메뉴가 닫힌다", () => {
    const { getByRole, getByText, queryByRole } = render(<BottomBar />);

    fireEvent.click(getByRole("button", { name: "기록" }));
    fireEvent.click(getByText("세션 활동 분석"));
    expect(useAppStore.getState().modal).toEqual({ kind: "analytics" });
    expect(queryByRole("menu")).toBeNull();

    fireEvent.click(getByRole("button", { name: "기록" }));
    fireEvent.click(getByText("이 달의 우수사원"));
    expect(useAppStore.getState().modal).toEqual({ kind: "awards" });
  });

});
