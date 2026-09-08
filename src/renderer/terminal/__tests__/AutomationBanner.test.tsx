// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import { AutomationBanner } from "../AutomationBanner";
import type { AutomationAgentStatus } from "@shared/types";

const stopAutomationMock = vi.fn().mockResolvedValue(undefined);
const continueAutomationMock = vi.fn().mockResolvedValue(undefined);

describe("AutomationBanner", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useAppStore.setState({
      stopAutomation: stopAutomationMock,
      continueAutomation: continueAutomationMock,
      automation: {},
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("자동화 상태가 없으면 아무것도 렌더하지 않는다", () => {
    const { container } = render(<AutomationBanner agentId="a1" />);
    expect(container.firstChild).toBeNull();
  });

  it("running 상태일 때 CLI, 상태, 남은 시간, 중단 버튼을 표시한다", () => {
    const status: AutomationAgentStatus = {
      running: true,
      phase: "watching",
      cli: "claude",
      filePath: "/tmp/result.json",
      startedAtMs: Date.now(),
      deadlineMs: Date.now() + 60000, // 1분 뒤
      extensionCount: 2,
    };
    useAppStore.setState({
      automation: { a1: status },
    });

    render(<AutomationBanner agentId="a1" />);

    expect(screen.getByText("claude")).toBeDefined();
    expect(screen.getByText("결과 파일 기다리는 중…")).toBeDefined();
    expect(screen.getByText(/남은 시간/)).toBeDefined();
    expect(screen.getByText(/연장 2회/)).toBeDefined();

    const stopBtn = screen.getByText("자동화 중단");
    fireEvent.click(stopBtn);
    expect(stopAutomationMock).toHaveBeenCalledWith("a1");
  });

  it("failed 상태일 때 실패 메시지를 표시한다", () => {
    const status: AutomationAgentStatus = {
      running: false,
      phase: "failed",
      cli: "claude",
      filePath: "/tmp/result.json",
      startedAtMs: Date.now(),
      error: "automation-session-lost",
    };
    useAppStore.setState({
      automation: { a1: status },
    });

    render(<AutomationBanner agentId="a1" />);
    expect(screen.getByText("자동화 실패")).toBeDefined();
    expect(screen.queryByText("자동화 중단")).toBeNull();
  });

  it("사용자 중단으로 끝난 실행도 결말을 보여준다", () => {
    const status: AutomationAgentStatus = {
      running: false,
      phase: "cancelled",
      cli: "claude",
      filePath: "/tmp/result.json",
      startedAtMs: Date.now(),
    };
    useAppStore.setState({ automation: { a1: status } });

    render(<AutomationBanner agentId="a1" />);
    expect(screen.getByText("자동화 중단됨")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "터미널 재시작" }));
    expect(useAppStore.getState().modal).toEqual({ kind: "confirm-restart", agentId: "a1" });
    // 이미 끝났으니 중단 버튼은 없다.
    expect(screen.queryByText("자동화 중단")).toBeNull();
  });

  it("humanTyping 보류 시 보류 문구와 계속 버튼을 표시하고 클릭 시 continueAutomation을 호출한다", () => {
    const status: AutomationAgentStatus = {
      running: true,
      phase: "injecting",
      cli: "claude",
      filePath: "/tmp/result.json",
      startedAtMs: Date.now(),
      pendingReason: "humanTyping",
    };
    useAppStore.setState({ automation: { a1: status } });

    render(<AutomationBanner agentId="a1" />);
    expect(screen.getByText("자동 입력 대기 — 입력창에 안 보낸 글이 남아 있습니다")).toBeDefined();
    const contBtn = screen.getByText("계속");
    expect(contBtn).toBeDefined();

    fireEvent.click(contBtn);
    expect(continueAutomationMock).toHaveBeenCalledWith("a1");
  });

  it("anotherProducer 보류 시 안내를 표시하고 계속 버튼은 표시하지 않는다", () => {
    const status: AutomationAgentStatus = {
      running: true,
      phase: "injecting",
      cli: "claude",
      filePath: "/tmp/result.json",
      startedAtMs: Date.now(),
      pendingReason: "anotherProducer",
    };
    useAppStore.setState({ automation: { a1: status } });

    render(<AutomationBanner agentId="a1" />);
    expect(screen.getByText("자동 입력 대기 — 다른 작업이 입력 중입니다")).toBeDefined();
    expect(screen.queryByText("계속")).toBeNull();
  });
});
