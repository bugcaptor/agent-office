// @vitest-environment jsdom
//
// src/renderer/agent/__tests__/ConfirmResumeDialog.test.tsx
//
// Claude 세션 이어하기 확인 다이얼로그 (ConfirmRestartDialog 테스트와 동일 패턴).
// resumeAgentSession은 모듈 목으로 대체해 다이얼로그의 배선만 검증한다.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

const resumeAgentSession = vi.fn().mockResolvedValue(undefined);
vi.mock("../resumeAgentSession", () => ({
  resumeAgentSession: (...args: unknown[]) => resumeAgentSession(...args),
}));

const { ConfirmResumeDialog } = await import("../ConfirmResumeDialog");

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
  resumeAgentSession.mockClear();
});

afterEach(() => cleanup());

describe("ConfirmResumeDialog", () => {
  it("modal이 confirm-resume이 아니면 아무것도 렌더하지 않는다", () => {
    const { container } = render(<ConfirmResumeDialog />);
    expect(container.firstChild).toBeNull();
  });

  it("에이전트 이름을 표시한다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.openModal({ kind: "confirm-resume", agentId: "a1", sessionId: "abc-123" });

    render(<ConfirmResumeDialog />);

    expect(screen.getByText("코난")).toBeTruthy();
  });

  it("세션이 실행 중이면 종료/스크롤백 경고를 표시한다 (running)", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.setSessionState({ agentId: "a1", status: "running" });
    s.openModal({ kind: "confirm-resume", agentId: "a1", sessionId: "abc-123" });

    render(<ConfirmResumeDialog />);

    expect(
      screen.getByText(/실행 중인 세션이 종료되고 스크롤백이 지워집니다/),
    ).toBeTruthy();
  });

  it("이어하기 확인 시 resumeAgentSession(agentId, sessionId)을 호출하고 모달을 닫는다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.openModal({ kind: "confirm-resume", agentId: "a1", sessionId: "abc-123" });

    render(<ConfirmResumeDialog />);
    fireEvent.click(screen.getByRole("button", { name: "이어하기" }));

    expect(resumeAgentSession).toHaveBeenCalledWith("a1", "abc-123");
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });

  it("취소 시 resumeAgentSession을 호출하지 않고 모달만 닫는다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.openModal({ kind: "confirm-resume", agentId: "a1", sessionId: "abc-123" });

    render(<ConfirmResumeDialog />);
    fireEvent.click(screen.getByRole("button", { name: "취소" }));

    expect(resumeAgentSession).not.toHaveBeenCalled();
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });
});
