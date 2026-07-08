// @vitest-environment jsdom
//
// src/renderer/agent/__tests__/ConfirmRestartDialog.test.tsx
//
// 터미널 재시작 확인 다이얼로그 (ConfirmDeleteDialog 테스트와 동일 패턴).
// restartAgentSession은 모듈 목으로 대체해 다이얼로그의 오케스트레이션 배선만
// 검증한다(실제 PTY/스토어/xterm 재시작은 restartAgentSession 테스트가 담당).
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

const restartAgentSession = vi.fn().mockResolvedValue(undefined);
vi.mock("../restartAgentSession", () => ({
  restartAgentSession: (...args: unknown[]) => restartAgentSession(...args),
}));

const { ConfirmRestartDialog } = await import("../ConfirmRestartDialog");

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
  restartAgentSession.mockClear();
});

afterEach(() => cleanup());

describe("ConfirmRestartDialog", () => {
  it("modal이 confirm-restart가 아니면 아무것도 렌더하지 않는다", () => {
    const { container } = render(<ConfirmRestartDialog />);
    expect(container.firstChild).toBeNull();
  });

  it("confirm-delete 모달에서도 렌더하지 않는다 (다른 modal kind)", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.openModal({ kind: "confirm-delete", agentId: "a1" });

    const { container } = render(<ConfirmRestartDialog />);
    expect(container.firstChild).toBeNull();
  });

  it("에이전트 이름을 표시한다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난")); // 세션 status: starting
    s.openModal({ kind: "confirm-restart", agentId: "a1" });

    render(<ConfirmRestartDialog />);

    expect(screen.getByText("코난")).toBeTruthy();
  });

  it("세션이 실행 중이면 종료/스크롤백 경고를 표시한다 (running)", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.setSessionState({ agentId: "a1", status: "running" });
    s.openModal({ kind: "confirm-restart", agentId: "a1" });

    render(<ConfirmRestartDialog />);

    expect(
      screen.getByText(/실행 중인 세션이 종료되고 스크롤백이 지워집니다/)
    ).toBeTruthy();
  });

  it("세션이 종료(exited) 상태면 경고를 표시하지 않는다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.setSessionState({ agentId: "a1", status: "exited" });
    s.openModal({ kind: "confirm-restart", agentId: "a1" });

    render(<ConfirmRestartDialog />);

    expect(
      screen.queryByText(/실행 중인 세션이 종료되고 스크롤백이 지워집니다/)
    ).toBeNull();
  });

  it("재시작 확인 시 restartAgentSession을 호출하고 모달을 닫는다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.openModal({ kind: "confirm-restart", agentId: "a1" });

    render(<ConfirmRestartDialog />);
    fireEvent.click(screen.getByRole("button", { name: "재시작" }));

    expect(restartAgentSession).toHaveBeenCalledWith("a1");
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });

  it("취소 시 restartAgentSession을 호출하지 않고 모달만 닫는다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.openModal({ kind: "confirm-restart", agentId: "a1" });

    render(<ConfirmRestartDialog />);
    fireEvent.click(screen.getByRole("button", { name: "취소" }));

    expect(restartAgentSession).not.toHaveBeenCalled();
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });
});
