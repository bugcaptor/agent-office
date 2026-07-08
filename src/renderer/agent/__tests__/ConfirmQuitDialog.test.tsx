// @vitest-environment jsdom
//
// src/renderer/agent/__tests__/ConfirmQuitDialog.test.tsx
//
// 앱 종료 확인 다이얼로그 (ConfirmRestartDialog 테스트와 동일 패턴).
// `@tauri-apps/api/window`는 모듈 목으로 대체해 `destroy()` 호출 배선만
// 검증한다(실제 윈도우 종료/Rust ExitRequested 정리는 백엔드 책임).
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";

const destroy = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ destroy: (...args: unknown[]) => destroy(...args) }),
}));

const { ConfirmQuitDialog } = await import("../ConfirmQuitDialog");

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
  destroy.mockClear();
});

afterEach(() => cleanup());

describe("ConfirmQuitDialog", () => {
  it("modal이 confirm-quit이 아니면 아무것도 렌더하지 않는다", () => {
    const { container } = render(<ConfirmQuitDialog />);
    expect(container.firstChild).toBeNull();
  });

  it("confirm-restart 모달에서도 렌더하지 않는다 (다른 modal kind)", () => {
    const s = useAppStore.getState();
    s.openModal({ kind: "confirm-restart", agentId: "a1" });

    const { container } = render(<ConfirmQuitDialog />);
    expect(container.firstChild).toBeNull();
  });

  it("confirm-quit 모달에서 제목과 본문을 표시한다", () => {
    const s = useAppStore.getState();
    s.openModal({ kind: "confirm-quit" });

    render(<ConfirmQuitDialog />);

    expect(screen.getByText("종료 확인")).toBeTruthy();
    expect(
      screen.getByText(
        "진행 중인 작업이 있습니다. 지금 종료하면 실행 중인 세션이 모두 중단됩니다."
      )
    ).toBeTruthy();
  });

  it("종료 확인 시 모달을 닫고 destroy()를 호출한다", () => {
    const s = useAppStore.getState();
    s.openModal({ kind: "confirm-quit" });

    render(<ConfirmQuitDialog />);
    fireEvent.click(screen.getByRole("button", { name: "종료" }));

    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
    expect(destroy).toHaveBeenCalledTimes(1);
  });

  it("취소 시 destroy()를 호출하지 않고 모달만 닫는다", () => {
    const s = useAppStore.getState();
    s.openModal({ kind: "confirm-quit" });

    render(<ConfirmQuitDialog />);
    fireEvent.click(screen.getByRole("button", { name: "취소" }));

    expect(destroy).not.toHaveBeenCalled();
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });

  it("배경 클릭 시 모달을 닫는다", () => {
    const s = useAppStore.getState();
    s.openModal({ kind: "confirm-quit" });

    const { container } = render(<ConfirmQuitDialog />);
    const backdrop = container.querySelector(".modal-backdrop");
    expect(backdrop).toBeTruthy();
    fireEvent.mouseDown(backdrop as Element, { button: 0 });

    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });
});
