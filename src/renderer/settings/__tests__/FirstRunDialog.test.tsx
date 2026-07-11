// @vitest-environment jsdom
//
// src/renderer/settings/__tests__/FirstRunDialog.test.tsx
//
// 첫 실행 온보딩 다이얼로그. settingsFirstRun=true일 때만 렌더하고,
// ConfirmQuitDialog와 달리 백드롭 클릭 핸들러가 없어(회피 불가) 그 점을
// 별도 확인한다. "시작하기"는 로컬에서 고른 선택값을 completeFirstRun에
// 그대로 넘겨야 한다.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useAppStore } from "../../store/appStore";
import { FirstRunDialog } from "../FirstRunDialog";

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
});

afterEach(() => cleanup());

describe("FirstRunDialog", () => {
  it("settingsFirstRun이 false면 아무것도 렌더하지 않는다", () => {
    useAppStore.getState().hydrateSettings(
      {
        version: 1,
        summarizerEnabled: false,
        summaryProvider: "claude",
        observerEnabled: false,
        soundEnabled: true,
        soundVolume: 0.5,
      },
      false,
    );
    const { container } = render(<FirstRunDialog />);
    expect(container.firstChild).toBeNull();
  });

  it("firstRun일 때 렌더되고 시작하기가 선택값으로 completeFirstRun을 부른다", () => {
    useAppStore.getState().hydrateSettings(
      {
        version: 1,
        summarizerEnabled: false,
        summaryProvider: "claude",
        observerEnabled: false,
        soundEnabled: true,
        soundVolume: 0.5,
      },
      true,
    );
    render(<FirstRunDialog />);
    fireEvent.click(screen.getByRole("radio", { name: "Codex" }));
    fireEvent.click(screen.getByRole("checkbox", { name: /에이전트 관찰/ }));
    fireEvent.click(screen.getByRole("button", { name: "시작하기" }));
    const s = useAppStore.getState();
    expect(s.settingsFirstRun).toBe(false);
    expect(s.appSettings).toEqual({
      version: 1,
      summarizerEnabled: false,
      summaryProvider: "codex",
      observerEnabled: true,
      soundEnabled: true,
      soundVolume: 0.5,
    });
  });

  it("백드롭 클릭으로 닫히지 않는다 (닫기 회피 불가)", () => {
    useAppStore.getState().hydrateSettings(
      {
        version: 1,
        summarizerEnabled: false,
        summaryProvider: "claude",
        observerEnabled: false,
        soundEnabled: true,
        soundVolume: 0.5,
      },
      true,
    );
    const { container } = render(<FirstRunDialog />);
    const backdrop = container.querySelector(".modal-backdrop");
    expect(backdrop).toBeTruthy();
    fireEvent.mouseDown(backdrop as Element, { button: 0 });
    expect(useAppStore.getState().settingsFirstRun).toBe(true);
  });
});
