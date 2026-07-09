// @vitest-environment jsdom
//
// src/renderer/agent/__tests__/ConfirmClockOutDialog.test.tsx
//
// 퇴근 확인 다이얼로그 TDD (ConfirmDeleteDialog.test.tsx와 동일한 접근).
// clockOutAgent/clockOutAll은 모듈 목으로 대체해 다이얼로그의 오케스트레이션
// 배선만 검증한다(실제 PTY/스토어/xterm 정리는 clockOut.test.ts가 담당).
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

const clockOutAgent = vi.fn().mockResolvedValue(undefined);
const clockOutAll = vi.fn().mockResolvedValue(undefined);
vi.mock("../clockOut", () => ({
  clockOutAgent: (...args: unknown[]) => clockOutAgent(...args),
  clockOutAll: (...args: unknown[]) => clockOutAll(...args),
}));

const { ConfirmClockOutDialog } = await import("../ConfirmClockOutDialog");

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
  clockOutAgent.mockClear();
  clockOutAll.mockClear();
});

afterEach(() => cleanup());

describe("ConfirmClockOutDialog", () => {
  it("modal이 confirm-clock-out(-all)이 아니면 아무것도 렌더하지 않는다", () => {
    const { container } = render(<ConfirmClockOutDialog />);
    expect(container.firstChild).toBeNull();
  });

  describe("개별 퇴근 (confirm-clock-out)", () => {
    it("에이전트 이름을 표시한다", () => {
      const s = useAppStore.getState();
      s.addAgent(mkProfile("a1", "코난")); // 세션 status: starting
      s.openModal({ kind: "confirm-clock-out", agentId: "a1" });

      render(<ConfirmClockOutDialog />);

      expect(screen.getByText("코난")).toBeTruthy();
    });

    it("세션이 실행 중이면 종료 경고를 표시한다 (running)", () => {
      const s = useAppStore.getState();
      s.addAgent(mkProfile("a1", "코난"));
      s.setSessionState({ agentId: "a1", status: "running" });
      s.openModal({ kind: "confirm-clock-out", agentId: "a1" });

      render(<ConfirmClockOutDialog />);

      expect(screen.getByText(/진행 중인 세션이 종료됩니다/)).toBeTruthy();
    });

    it("세션이 종료(exited) 상태면 경고를 표시하지 않는다", () => {
      const s = useAppStore.getState();
      s.addAgent(mkProfile("a1", "코난"));
      s.setSessionState({ agentId: "a1", status: "exited" });
      s.openModal({ kind: "confirm-clock-out", agentId: "a1" });

      render(<ConfirmClockOutDialog />);

      expect(screen.queryByText(/진행 중인 세션이 종료됩니다/)).toBeNull();
    });

    it("퇴근 확인 시 clockOutAgent를 호출하고 모달을 닫는다", () => {
      const s = useAppStore.getState();
      s.addAgent(mkProfile("a1", "코난"));
      s.openModal({ kind: "confirm-clock-out", agentId: "a1" });

      render(<ConfirmClockOutDialog />);
      fireEvent.click(screen.getByRole("button", { name: "퇴근" }));

      expect(clockOutAgent).toHaveBeenCalledWith("a1");
      expect(clockOutAll).not.toHaveBeenCalled();
      expect(useAppStore.getState().modal).toEqual({ kind: "none" });
    });

    it("취소 시 clockOutAgent를 호출하지 않고 모달만 닫는다", () => {
      const s = useAppStore.getState();
      s.addAgent(mkProfile("a1", "코난"));
      s.openModal({ kind: "confirm-clock-out", agentId: "a1" });

      render(<ConfirmClockOutDialog />);
      fireEvent.click(screen.getByRole("button", { name: "취소" }));

      expect(clockOutAgent).not.toHaveBeenCalled();
      expect(useAppStore.getState().modal).toEqual({ kind: "none" });
    });
  });

  describe("전체 퇴근 (confirm-clock-out-all)", () => {
    it("근무 중인 인원 수를 표시하고 항상 경고를 띄운다", () => {
      const s = useAppStore.getState();
      s.addAgent(mkProfile("a1", "코난"));
      s.addAgent(mkProfile("a2", "김전일"));
      s.addAgent(mkProfile("a3", "소년"));
      s.clockOut("a3"); // 이미 퇴근 -> 근무 중 카운트에서 제외
      s.openModal({ kind: "confirm-clock-out-all" });

      render(<ConfirmClockOutDialog />);

      expect(screen.getByText(/근무 중인 캐릭터 2명을 모두 퇴근시킬까요\?/)).toBeTruthy();
      expect(screen.getByText(/진행 중인 세션이 모두 종료됩니다/)).toBeTruthy();
    });

    it("퇴근 확인 시 clockOutAll을 호출하고 모달을 닫는다", () => {
      const s = useAppStore.getState();
      s.addAgent(mkProfile("a1", "코난"));
      s.openModal({ kind: "confirm-clock-out-all" });

      render(<ConfirmClockOutDialog />);
      fireEvent.click(screen.getByRole("button", { name: "퇴근" }));

      expect(clockOutAll).toHaveBeenCalledTimes(1);
      expect(clockOutAgent).not.toHaveBeenCalled();
      expect(useAppStore.getState().modal).toEqual({ kind: "none" });
    });

    it("취소 시 clockOutAll을 호출하지 않고 모달만 닫는다", () => {
      const s = useAppStore.getState();
      s.addAgent(mkProfile("a1", "코난"));
      s.openModal({ kind: "confirm-clock-out-all" });

      render(<ConfirmClockOutDialog />);
      fireEvent.click(screen.getByRole("button", { name: "취소" }));

      expect(clockOutAll).not.toHaveBeenCalled();
      expect(useAppStore.getState().modal).toEqual({ kind: "none" });
    });
  });
});
