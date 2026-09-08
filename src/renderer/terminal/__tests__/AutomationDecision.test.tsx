// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import { AutomationDecision } from "../AutomationDecision";
import type { AutomationAgentStatus } from "@shared/types";

const decideAutomationMock = vi.fn().mockResolvedValue(true);

describe("AutomationDecision", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useAppStore.setState({
      decideAutomation: decideAutomationMock,
      automation: {},
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("timeoutDecision phase가 아니면 패널을 렌더하지 않는다", () => {
    useAppStore.setState({
      automation: {
        a1: {
          running: true,
          phase: "watching",
          cli: "claude",
          filePath: "/tmp/result.json",
          startedAtMs: Date.now(),
        },
      },
    });

    const { container } = render(<AutomationDecision agentId="a1" />);
    expect(container.firstChild).toBeNull();
  });

  it("timeoutDecision phase일 때 비차단 패널과 선택 버튼을 렌더한다", () => {
    const status: AutomationAgentStatus = {
      running: true,
      phase: "timeoutDecision",
      cli: "claude",
      filePath: "/tmp/result.json",
      startedAtMs: Date.now(),
      runId: "run-1",
      stepExecutionId: "step-1",
      decisionId: "dec-1",
    };
    useAppStore.setState({
      automation: { a1: status },
    });

    render(<AutomationDecision agentId="a1" />);

    expect(screen.getByText("작업 대기 시간 초과")).toBeDefined();
    expect(screen.getByText("더 기다리기")).toBeDefined();
    expect(screen.getByText("자동화 중단")).toBeDefined();
    expect(screen.queryByText("완료 신호가 감지되었습니다.")).toBeNull();
  });

  it("마커가 감지되었으면 완료 신호 안내를 함께 표시한다", () => {
    const status: AutomationAgentStatus = {
      running: true,
      phase: "timeoutDecision",
      cli: "claude",
      filePath: "/tmp/result.json",
      startedAtMs: Date.now(),
      runId: "run-1",
      stepExecutionId: "step-1",
      decisionId: "dec-1",
      markerObservedAtMs: Date.now(),
    };
    useAppStore.setState({
      automation: { a1: status },
    });

    render(<AutomationDecision agentId="a1" />);

    expect(screen.getByText("완료 신호가 감지되었습니다.")).toBeDefined();
  });

  it("더 기다리기 클릭 시 decideAutomation(extend)을 호출한다", () => {
    const status: AutomationAgentStatus = {
      running: true,
      phase: "timeoutDecision",
      cli: "claude",
      filePath: "/tmp/result.json",
      startedAtMs: Date.now(),
      runId: "run-1",
      stepExecutionId: "step-1",
      decisionId: "dec-1",
    };
    useAppStore.setState({
      automation: { a1: status },
    });

    render(<AutomationDecision agentId="a1" />);
    const extendBtn = screen.getByText("더 기다리기");
    fireEvent.click(extendBtn);

    expect(decideAutomationMock).toHaveBeenCalledWith("a1", "extend");
  });

  it("자동화 중단 클릭 시 decideAutomation(stop)을 호출한다", () => {
    const status: AutomationAgentStatus = {
      running: true,
      phase: "timeoutDecision",
      cli: "claude",
      filePath: "/tmp/result.json",
      startedAtMs: Date.now(),
      runId: "run-1",
      stepExecutionId: "step-1",
      decisionId: "dec-1",
    };
    useAppStore.setState({
      automation: { a1: status },
    });

    render(<AutomationDecision agentId="a1" />);
    const stopBtn = screen.getByText("자동화 중단");
    fireEvent.click(stopBtn);

    expect(decideAutomationMock).toHaveBeenCalledWith("a1", "stop");
  });

  it("CLI 반환 receipt가 도착해도 패널을 유지하고 사용자의 선택을 기다린다", () => {
    useAppStore.setState({ automation: { a1: {
      running: true, phase: "timeoutDecision", cli: "claude", filePath: "", startedAtMs: Date.now(),
      decisionReason: "cliExitTimeout", cliReturnObservedAtMs: Date.now(),
    } } });
    render(<AutomationDecision agentId="a1" />);
    expect(screen.getByText("CLI 반환 기록이 도착했습니다. 계속할 방법을 선택하세요.")).toBeDefined();
    expect(screen.getByText("더 기다리기")).toBeDefined();
    expect(screen.getByText("자동화 중단")).toBeDefined();
  });
});
