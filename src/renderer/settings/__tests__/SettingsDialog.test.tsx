// @vitest-environment jsdom
//
// src/renderer/settings/__tests__/SettingsDialog.test.tsx
//
// 상시 설정 다이얼로그(ConfirmQuitDialog와 동일 패턴). FirstRunDialog와
// 달리 스토어 값을 직접 바인딩한다 — 토글 클릭이 즉시 updateAppSettings로
// 반영되는지, 닫기 버튼/백드롭이 closeModal을 부르는지 확인한다.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useAppStore } from "../../store/appStore";
import { SettingsDialog } from "../SettingsDialog";

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
});

afterEach(() => cleanup());

describe("SettingsDialog", () => {
  it("modal이 settings가 아니면 아무것도 렌더하지 않는다", () => {
    const { container } = render(<SettingsDialog />);
    expect(container.firstChild).toBeNull();
  });

  it("settings 모달일 때 렌더되고 공통 설정 변경을 즉시 반영한다", () => {
    useAppStore.getState().hydrateSettings(
      {
        version: 1,
        summarizerEnabled: false,
        summaryProvider: "claude",
        observerEnabled: false,
        soundEnabled: true,
        soundVolume: 0.5,
        externalTerminal: "terminal",
        externalEditor: "system",
        attentionHoldMs: 5000,
      },
      false,
    );
    useAppStore.getState().openModal({ kind: "settings" });

    render(<SettingsDialog />);
    expect(screen.getByText("설정")).toBeTruthy();

    fireEvent.click(screen.getByRole("radio", { name: "Codex" }));
    fireEvent.click(screen.getByRole("checkbox", { name: /에이전트 관찰/ }));

    expect(useAppStore.getState().appSettings).toEqual({
      version: 1,
      summarizerEnabled: false,
      summaryProvider: "codex",
      observerEnabled: true,
      soundEnabled: true,
      soundVolume: 0.5,
      externalTerminal: "terminal",
      externalEditor: "system",
      attentionHoldMs: 5000,
    });
  });

  it("외부 터미널 앱 셀렉터가 iTerm2 선택을 즉시 반영한다", () => {
    useAppStore.getState().hydrateSettings(
      {
        version: 1,
        summarizerEnabled: false,
        summaryProvider: "claude",
        observerEnabled: false,
        soundEnabled: true,
        soundVolume: 0.5,
        externalTerminal: "terminal",
        externalEditor: "system",
        attentionHoldMs: 5000,
      },
      false,
    );
    useAppStore.getState().openModal({ kind: "settings" });

    render(<SettingsDialog />);
    // 이제 셀렉터가 둘(외부 터미널/셸 출력 에디터)이므로 이름으로 특정한다.
    fireEvent.change(screen.getByRole("combobox", { name: /외부 터미널/ }), {
      target: { value: "iterm" },
    });

    expect(useAppStore.getState().appSettings.externalTerminal).toBe("iterm");
  });

  it("셸 출력 에디터 셀렉터가 VS Code 선택을 즉시 반영한다", () => {
    useAppStore.getState().hydrateSettings(
      {
        version: 1,
        summarizerEnabled: false,
        summaryProvider: "claude",
        observerEnabled: false,
        soundEnabled: true,
        soundVolume: 0.5,
        externalTerminal: "terminal",
        externalEditor: "system",
        attentionHoldMs: 5000,
      },
      false,
    );
    useAppStore.getState().openModal({ kind: "settings" });

    render(<SettingsDialog />);
    fireEvent.change(screen.getByRole("combobox", { name: /셸 출력 에디터/ }), {
      target: { value: "vscode" },
    });

    expect(useAppStore.getState().appSettings.externalEditor).toBe("vscode");
  });

  it("닫기 버튼 클릭 시 closeModal을 부른다", () => {
    useAppStore.getState().openModal({ kind: "settings" });

    render(<SettingsDialog />);
    fireEvent.click(screen.getByRole("button", { name: "닫기" }));

    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });

  it("백드롭 클릭 시 closeModal을 부른다", () => {
    useAppStore.getState().openModal({ kind: "settings" });

    const { container } = render(<SettingsDialog />);
    const backdrop = container.querySelector(".modal-backdrop");
    expect(backdrop).toBeTruthy();
    fireEvent.mouseDown(backdrop as Element, { button: 0 });

    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });
});
