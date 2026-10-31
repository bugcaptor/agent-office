// @vitest-environment jsdom
//
// src/renderer/settings/__tests__/ShortcutsTab.test.tsx
//
// 단축키 탭: 녹화 저장, 추천값은 저장되지 않은 placeholder, 충돌·금지 키는 저장 안 함.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile } from "@shared/types";

const { setAppSettings } = vi.hoisted(() => ({ setAppSettings: vi.fn().mockResolvedValue(undefined) }));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: { setAppSettings } }));
vi.mock("../../shared/platform", () => ({ IS_MAC: false, IS_WINDOWS: false }));

import { useAppStore } from "../../store/appStore";
import { ShortcutsTab } from "../ShortcutsTab";

const initial = useAppStore.getState();

function profile(id: string, name: string): AgentProfile {
  return { id, name, role: "", seed: id, createdAt: 0, deskIndex: 0 };
}

function record(name: string, init: KeyboardEventInit) {
  const btn = screen.getByRole("button", { name: `${name} 단축키 지정` });
  fireEvent.click(btn);
  fireEvent.keyDown(btn, init);
}

beforeEach(() => {
  useAppStore.setState(initial, true);
  setAppSettings.mockClear();
  useAppStore.setState({
    agents: { a: profile("a", "앨리스"), b: profile("b", "밥") },
    agentOrder: ["a", "b"],
  });
});

afterEach(() => cleanup());

describe("ShortcutsTab", () => {
  it("빈 칸은 추천값을 보여 주기만 하고 저장하지 않는다", () => {
    render(<ShortcutsTab />);
    expect(screen.getByRole("button", { name: "밥 단축키 지정" }).textContent).toBe("Ctrl+Alt+2");
    expect(useAppStore.getState().appSettings.agentShortcuts).toEqual({});
    expect(setAppSettings).not.toHaveBeenCalled();
  });

  it("추천값 버튼을 누르면 그 키로 저장한다", () => {
    render(<ShortcutsTab />);
    fireEvent.click(screen.getAllByRole("button", { name: "추천값" })[0]);
    expect(useAppStore.getState().appSettings.notifyJumpShortcut).toBe("Ctrl+Alt+KeyJ");
  });

  it("녹화한 키를 저장하고, 지워진 에이전트의 옛 키는 함께 정리한다", () => {
    useAppStore.setState({
      appSettings: { ...useAppStore.getState().appSettings, agentShortcuts: { gone: "Ctrl+Alt+Digit9" } },
    });
    render(<ShortcutsTab />);
    record("앨리스", { code: "KeyK", key: "k", ctrlKey: true, altKey: true });
    expect(useAppStore.getState().appSettings.agentShortcuts).toEqual({ a: "Ctrl+Alt+KeyK" });
    expect(screen.getByRole("button", { name: "앨리스 단축키 지정" }).textContent).toBe("Ctrl+Alt+K");
  });

  it("다른 칸과 겹치거나 셸 키면 저장하지 않고 이유를 보여 준다", () => {
    useAppStore.setState({
      appSettings: { ...useAppStore.getState().appSettings, agentShortcuts: { a: "Ctrl+Alt+KeyK" } },
    });
    render(<ShortcutsTab />);
    record("밥", { code: "KeyK", key: "k", ctrlKey: true, altKey: true });
    expect(screen.getByRole("alert").textContent).toContain("앨리스");
    record("밥", { code: "KeyR", key: "r", ctrlKey: true });
    expect(screen.getByRole("alert").textContent).toContain("셸");
    expect(useAppStore.getState().appSettings.agentShortcuts).toEqual({ a: "Ctrl+Alt+KeyK" });
  });

  it("Esc는 녹화를 취소하고 ×는 키를 지운다", () => {
    useAppStore.setState({
      appSettings: { ...useAppStore.getState().appSettings, agentShortcuts: { a: "Ctrl+Alt+KeyK" } },
    });
    render(<ShortcutsTab />);
    record("앨리스", { code: "Escape", key: "Escape" });
    expect(useAppStore.getState().appSettings.agentShortcuts).toEqual({ a: "Ctrl+Alt+KeyK" });
    fireEvent.click(screen.getByRole("button", { name: "앨리스 단축키 지우기" }));
    expect(useAppStore.getState().appSettings.agentShortcuts).toEqual({});
  });
});
