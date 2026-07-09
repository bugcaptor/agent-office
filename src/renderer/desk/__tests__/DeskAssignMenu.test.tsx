// @vitest-environment jsdom
//
// 책상 클릭 → 주인 지정 컨텍스트 메뉴 TDD.
//
// officeBus(스토어 기반 실물)의 emitDeskClicked로 메뉴를 열고, 항목 선택이
// appStore.assignDesk를 통해 agents[..].assignedDeskIndex에 반영되는지 검증.
// appStore/sessionBridge가 import하는 tauriApi만 최소 mock.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile } from "../../store/types";

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    setAppSettings: vi.fn().mockResolvedValue(undefined),
    clearNotifications: vi.fn(),
    createSession: vi.fn().mockResolvedValue({ sessionId: "s", state: "starting" }),
    setBadgeCount: vi.fn(),
  },
}));

import { useAppStore } from "../../store/appStore";
import { officeBus } from "../../ipc/sessionBridge";
import { DeskAssignMenu } from "../DeskAssignMenu";

const initialState = useAppStore.getState();

const mkProfile = (id: string, name: string): AgentProfile => ({
  id,
  name,
  role: "eng",
  note: "",
  seed: id,
  createdAt: 1,
  deskIndex: 0,
});

beforeEach(() => {
  useAppStore.setState(initialState, true);
});
afterEach(() => cleanup());

describe("DeskAssignMenu", () => {
  it("책상 클릭 이벤트로 메뉴가 열리고, 에이전트 선택 시 그 책상이 지정된다", () => {
    useAppStore.getState().addAgent(mkProfile("a1", "김철수"));
    useAppStore.getState().addAgent(mkProfile("a2", "이영희"));
    render(<DeskAssignMenu />);

    act(() => officeBus.emitDeskClicked(2, 40, 50));
    fireEvent.click(screen.getByRole("menuitem", { name: "김철수" }));

    expect(useAppStore.getState().agents.a1.assignedDeskIndex).toBe(2);
    expect(screen.queryByRole("menu")).toBeNull(); // 선택 후 닫힘
  });

  it("현재 주인은 ✓ 표시되고, 다른 에이전트를 고르면 주인이 교체된다", () => {
    useAppStore.getState().addAgent(mkProfile("a1", "김철수"));
    useAppStore.getState().addAgent(mkProfile("a2", "이영희"));
    useAppStore.getState().assignDesk(3, "a1");
    render(<DeskAssignMenu />);

    act(() => officeBus.emitDeskClicked(3, 0, 0));
    expect(screen.getByRole("menuitem", { name: "✓ 김철수" })).toBeTruthy();

    fireEvent.click(screen.getByRole("menuitem", { name: "이영희" }));
    const agents = useAppStore.getState().agents;
    expect(agents.a2.assignedDeskIndex).toBe(3);
    expect(agents.a1.assignedDeskIndex).toBeUndefined();
  });

  it("주인이 있는 책상은 '지정 해제'로 해제할 수 있다", () => {
    useAppStore.getState().addAgent(mkProfile("a1", "김철수"));
    useAppStore.getState().assignDesk(1, "a1");
    render(<DeskAssignMenu />);

    act(() => officeBus.emitDeskClicked(1, 40, 50));
    fireEvent.click(screen.getByRole("menuitem", { name: "지정 해제" }));

    expect(useAppStore.getState().agents.a1.assignedDeskIndex).toBeUndefined();
  });

  it("주인이 없으면 '지정 해제'가 비활성화된다", () => {
    useAppStore.getState().addAgent(mkProfile("a1", "김철수"));
    render(<DeskAssignMenu />);

    act(() => officeBus.emitDeskClicked(0, 0, 0));

    const item = screen.getByRole("menuitem", { name: "지정 해제" }) as HTMLButtonElement;
    expect(item.disabled).toBe(true);
  });

  it("메뉴가 닫혀 있으면 아무것도 렌더하지 않는다", () => {
    render(<DeskAssignMenu />);
    expect(screen.queryByRole("menu")).toBeNull();
  });
});
