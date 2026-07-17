// @vitest-environment jsdom
//
// src/renderer/agent/__tests__/ConfirmQuitDialog.test.tsx
//
// 앱 종료 확인 다이얼로그 (ConfirmRestartDialog 테스트와 동일 패턴).
// `@tauri-apps/api/window`는 모듈 목으로 대체해 `destroy()` 호출 배선만
// 검증한다(실제 윈도우 종료/Rust ExitRequested 정리는 백엔드 책임).
//
// 세션 핸드오프 3버튼 분기는 `quitGuard.isHandoffSupported()`(부팅 시 캐시)와
// 스토어의 Running 세션 유무 조합으로 결정된다 — `isHandoffSupported`는
// 모듈 경계에서 목으로 대체해 기본 false(미지원 = 기존 2버튼)로 두고, 개별
// 테스트에서만 true로 바꾼다.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";

const destroy = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ destroy: (...args: unknown[]) => destroy(...args) }),
}));

const handoffSessions = vi.fn().mockResolvedValue(0);
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: { handoffSessions: (...args: unknown[]) => handoffSessions(...args) },
}));

const isHandoffSupportedMock = vi.fn(() => false);
vi.mock("../../quitGuard", () => ({
  isHandoffSupported: () => isHandoffSupportedMock(),
}));

const serializeAllMock = vi.fn(() => ({}) as Record<string, string>);
vi.mock("../../terminal/TerminalRegistry", () => ({
  terminalRegistry: { serializeAll: () => serializeAllMock() },
}));

const { ConfirmQuitDialog } = await import("../ConfirmQuitDialog");

const initialState = useAppStore.getState();

function mkRunningSession(agentId: string) {
  return { agentId, status: "running" as const, cols: 80, rows: 24, lastActivityAt: Date.now() };
}

beforeEach(() => {
  useAppStore.setState(initialState, true);
  destroy.mockClear();
  handoffSessions.mockReset().mockResolvedValue(0);
  isHandoffSupportedMock.mockReset().mockReturnValue(false);
  serializeAllMock.mockReset().mockReturnValue({});
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
        "아직 퇴근하지 않은 에이전트가 있습니다. 지금 종료하면 실행 중인 세션이 모두 중단됩니다."
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

describe("3버튼 분기 (핸드오프 지원 + Running 세션)", () => {
  it("미지원이면 Running 세션이 있어도 기존 2버튼 그대로", () => {
    isHandoffSupportedMock.mockReturnValue(false);
    useAppStore.setState({
      agentOrder: ["a1"],
      sessions: { a1: mkRunningSession("a1") },
    });
    const s = useAppStore.getState();
    s.openModal({ kind: "confirm-quit" });

    render(<ConfirmQuitDialog />);

    expect(screen.getByRole("button", { name: "종료" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "터미널 유지하고 종료" })).toBeNull();
  });

  it("지원되어도 Running 세션이 없으면 기존 2버튼 그대로", () => {
    isHandoffSupportedMock.mockReturnValue(true);
    const s = useAppStore.getState();
    s.openModal({ kind: "confirm-quit" });

    render(<ConfirmQuitDialog />);

    expect(screen.getByRole("button", { name: "종료" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "터미널 유지하고 종료" })).toBeNull();
  });

  it("지원 + Running 세션이 있으면 3버튼(유지/모두 종료/취소)을 표시한다", () => {
    isHandoffSupportedMock.mockReturnValue(true);
    useAppStore.setState({
      agentOrder: ["a1"],
      sessions: { a1: mkRunningSession("a1") },
    });
    const s = useAppStore.getState();
    s.openModal({ kind: "confirm-quit" });

    render(<ConfirmQuitDialog />);

    expect(screen.getByRole("button", { name: "터미널 유지하고 종료" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "모두 종료하고 종료" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "취소" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "종료" })).toBeNull();
  });

  it("'터미널 유지하고 종료' 클릭 시 serializeAll() 결과를 실어 handoffSessions()를 기다린 뒤 destroy()를 호출한다", async () => {
    isHandoffSupportedMock.mockReturnValue(true);
    serializeAllMock.mockReturnValue({ a1: "SCREEN-BEFORE-QUIT" });
    let resolveHandoff!: (n: number) => void;
    handoffSessions.mockReturnValue(new Promise<number>((resolve) => (resolveHandoff = resolve)));
    useAppStore.setState({
      agentOrder: ["a1"],
      sessions: { a1: mkRunningSession("a1") },
    });
    const s = useAppStore.getState();
    s.openModal({ kind: "confirm-quit" });

    render(<ConfirmQuitDialog />);
    fireEvent.click(screen.getByRole("button", { name: "터미널 유지하고 종료" }));

    expect(handoffSessions).toHaveBeenCalledTimes(1);
    expect(handoffSessions).toHaveBeenCalledWith({ a1: "SCREEN-BEFORE-QUIT" });
    expect(destroy).not.toHaveBeenCalled(); // handoffSessions()가 아직 settle 전

    resolveHandoff(2);
    await Promise.resolve();
    await Promise.resolve();

    expect(destroy).toHaveBeenCalledTimes(1);
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });

  it("handoffSessions()가 실패해도(구버전 데몬 등) 종료는 그대로 진행한다", async () => {
    isHandoffSupportedMock.mockReturnValue(true);
    handoffSessions.mockRejectedValue(new Error("daemon spawn failed"));
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    useAppStore.setState({
      agentOrder: ["a1"],
      sessions: { a1: mkRunningSession("a1") },
    });
    const s = useAppStore.getState();
    s.openModal({ kind: "confirm-quit" });

    render(<ConfirmQuitDialog />);
    fireEvent.click(screen.getByRole("button", { name: "터미널 유지하고 종료" }));

    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    expect(destroy).toHaveBeenCalledTimes(1);
    warn.mockRestore();
  });

  it("'모두 종료하고 종료' 클릭 시 handoffSessions() 없이 바로 destroy()를 호출한다", () => {
    isHandoffSupportedMock.mockReturnValue(true);
    useAppStore.setState({
      agentOrder: ["a1"],
      sessions: { a1: mkRunningSession("a1") },
    });
    const s = useAppStore.getState();
    s.openModal({ kind: "confirm-quit" });

    render(<ConfirmQuitDialog />);
    fireEvent.click(screen.getByRole("button", { name: "모두 종료하고 종료" }));

    expect(handoffSessions).not.toHaveBeenCalled();
    expect(destroy).toHaveBeenCalledTimes(1);
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });

  it("3버튼 모달에서도 취소는 destroy()를 호출하지 않는다", () => {
    isHandoffSupportedMock.mockReturnValue(true);
    useAppStore.setState({
      agentOrder: ["a1"],
      sessions: { a1: mkRunningSession("a1") },
    });
    const s = useAppStore.getState();
    s.openModal({ kind: "confirm-quit" });

    render(<ConfirmQuitDialog />);
    fireEvent.click(screen.getByRole("button", { name: "취소" }));

    expect(destroy).not.toHaveBeenCalled();
    expect(handoffSessions).not.toHaveBeenCalled();
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });
});
