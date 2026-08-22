// @vitest-environment jsdom
//
// src/renderer/terminal/__tests__/TerminalSummaryBar.test.tsx
//
// 활성 탭 요약 바(이슈 #44 T1): activeTerminalAgentId의 라벨을 머리 위 라벨과
// 같은 파생 규칙으로 한 줄 표시한다. 세션이 starting/running이 아니면 실황
// (line2)은 stale이므로 억제하고 line1만 흐리게. 라벨이 없으면 미표시.
import { cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useAppStore } from "../../store/appStore";
import { initialTurnState } from "../../timeline/turnReducer";
import { TerminalSummaryBar } from "../TerminalSummaryBar";
import type { AgentProfile } from "../../store/types";

function agent(id: string, cwd?: string): AgentProfile {
  return { id, name: id, role: "", seed: "s", createdAt: 0, deskIndex: 0, cwd };
}

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
});

afterEach(() => cleanup());

function seed(opts: {
  activeId?: string | null;
  status?: "idle" | "starting" | "running" | "exited" | "disposed";
  label?: Partial<import("../../store/types").AgentTaskLabel>;
  phase?: "idle" | "working" | "waiting";
}) {
  useAppStore.setState({
    activeTerminalAgentId: opts.activeId === undefined ? "a1" : opts.activeId,
    agents: { a1: agent("a1", "/Users/me/dev/agent-office") },
    sessions: {
      a1: { agentId: "a1", status: opts.status ?? "running", cols: 80, rows: 24, lastActivityAt: 0 },
    },
    taskLabels: opts.label ? { a1: { sessionId: "s1", ...opts.label } } : {},
    timeTracking: { a1: { ...initialTurnState(), phase: opts.phase ?? "working" } },
  });
}

describe("TerminalSummaryBar", () => {
  it("활성 에이전트의 라벨을 '프로젝트명 · 목표 — 실황'으로 한 줄 표시한다", () => {
    seed({ label: { goal: "버그 수정", latestAssistantText: "원인 좁히는 중" } });
    const { container } = render(<TerminalSummaryBar />);
    const bar = container.querySelector(".terminal-summary-bar")!;
    expect(bar.querySelector(".terminal-summary-line1")!.textContent).toBe("agent-office · 버그 수정");
    expect(bar.querySelector(".terminal-summary-line2")!.textContent).toBe("원인 좁히는 중");
    expect(bar.className).toContain("phase-working");
  });

  it("세션이 running/starting이 아니면 실황(line2)을 억제하고 line1만 흐리게 남긴다", () => {
    seed({ status: "exited", label: { goal: "버그 수정", latestAssistantText: "원인 좁히는 중" } });
    const { container } = render(<TerminalSummaryBar />);
    const bar = container.querySelector(".terminal-summary-bar")!;
    expect(bar.querySelector(".terminal-summary-line1")!.textContent).toBe("agent-office · 버그 수정");
    expect(bar.querySelector(".terminal-summary-line2")).toBeNull();
    expect(bar.className).toContain("terminal-summary-stale");
  });

  // 표시할 게 없어도 바는 자리를 지킨다. 라벨이 생길 때 패널 높이가 변하면
  // xterm rows가 줄어 PTY resize가 나가고, pi 기본 TUI는 resize마다 스크롤백을
  // 지운다 — 그래서 조건부 렌더 대신 visibility 토글이다.
  it("라벨이 없으면(표시할 것 없음) 내용 없이 자리만 남긴 바를 렌더한다", () => {
    seed({ label: undefined, status: "running" });
    // cwd만 있는 라벨 없는 상태 — line1은 프로젝트명이 나오므로 라벨 없음 검증을
    // 위해 agents에서도 cwd를 비운다.
    useAppStore.setState({ agents: { a1: agent("a1") } });
    const { container } = render(<TerminalSummaryBar />);
    const bar = container.querySelector(".terminal-summary-bar");
    expect(bar).not.toBeNull();
    expect(bar!.className).toContain("terminal-summary-hidden");
    expect(bar!.textContent).toBe("");
  });

  it("라벨이 생겨도 바가 새로 나타나지 않는다(레이아웃/터미널 rows 불변)", () => {
    seed({ label: undefined, status: "running" });
    useAppStore.setState({ agents: { a1: agent("a1") } });
    const { container, rerender } = render(<TerminalSummaryBar />);
    expect(container.querySelectorAll(".terminal-summary-bar")).toHaveLength(1);

    seed({ label: { goal: "버그 수정" }, status: "running" });
    rerender(<TerminalSummaryBar />);
    const bars = container.querySelectorAll(".terminal-summary-bar");
    expect(bars).toHaveLength(1);
    expect(bars[0].className).not.toContain("terminal-summary-hidden");
  });

  it("활성 터미널이 없으면 아무것도 렌더하지 않는다", () => {
    seed({ activeId: null, label: { goal: "버그 수정" } });
    const { container } = render(<TerminalSummaryBar />);
    expect(container.querySelector(".terminal-summary-bar")).toBeNull();
  });
});
