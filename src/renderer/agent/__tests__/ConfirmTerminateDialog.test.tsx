// @vitest-environment jsdom
//
// src/renderer/agent/__tests__/ConfirmTerminateDialog.test.tsx
//
// 터미널 종료 확인 다이얼로그 (ConfirmRestartDialog 테스트와 동일 패턴).
// terminateAgentSession은 모듈 목으로 대체해 다이얼로그의 배선만 검증한다
// (실제 PTY 종료/알림 클리어는 terminateSession 테스트가 담당).
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

const terminateAgentSession = vi.fn().mockResolvedValue(undefined);
vi.mock("../terminateSession", () => ({
  terminateAgentSession: (...args: unknown[]) => terminateAgentSession(...args),
}));

const { ConfirmTerminateDialog } = await import("../ConfirmTerminateDialog");

function mkProfile(id: string, name: string): AgentProfile {
  return {
    id,
    name,
    role: "eng",
    note: "",
    seed: id,
    createdAt: Date.now(),
    deskIndex: 0,
  };
}

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
  terminateAgentSession.mockClear();
});

afterEach(() => cleanup());

describe("ConfirmTerminateDialog", () => {
  it("modal이 confirm-terminate가 아니면 아무것도 렌더하지 않는다", () => {
    const { container } = render(<ConfirmTerminateDialog />);
    expect(container.firstChild).toBeNull();
  });

  it("에이전트 이름을 표시한다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.openModal({ kind: "confirm-terminate", agentId: "a1" });

    render(<ConfirmTerminateDialog />);

    expect(screen.getByText("코난")).toBeTruthy();
  });

  it("세션이 실행 중이면 종료/탕비실 안내를 표시한다 (running)", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.setSessionState({ agentId: "a1", status: "running" });
    s.openModal({ kind: "confirm-terminate", agentId: "a1" });

    render(<ConfirmTerminateDialog />);

    expect(screen.getByText(/실행 중인 세션이 종료됩니다/)).toBeTruthy();
    expect(screen.getByText(/탕비실에서 대기/)).toBeTruthy();
  });

  it("세션이 종료(exited) 상태면 경고를 표시하지 않는다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.setSessionState({ agentId: "a1", status: "exited" });
    s.openModal({ kind: "confirm-terminate", agentId: "a1" });

    render(<ConfirmTerminateDialog />);

    expect(screen.queryByText(/실행 중인 세션이 종료됩니다/)).toBeNull();
  });

  it("종료 확인 시 terminateAgentSession을 호출하고 모달을 닫는다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.openModal({ kind: "confirm-terminate", agentId: "a1" });

    render(<ConfirmTerminateDialog />);
    fireEvent.click(screen.getByRole("button", { name: "종료" }));

    expect(terminateAgentSession).toHaveBeenCalledWith("a1");
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });

  it("취소 시 terminateAgentSession을 호출하지 않고 모달만 닫는다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.openModal({ kind: "confirm-terminate", agentId: "a1" });

    render(<ConfirmTerminateDialog />);
    fireEvent.click(screen.getByRole("button", { name: "취소" }));

    expect(terminateAgentSession).not.toHaveBeenCalled();
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });
});
