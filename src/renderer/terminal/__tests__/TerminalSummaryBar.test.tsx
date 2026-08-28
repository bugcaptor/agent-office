// @vitest-environment jsdom
//
// src/renderer/terminal/__tests__/TerminalSummaryBar.test.tsx
//
// 활성 탭 요약 바(이슈 #44 T1): activeTerminalAgentId의 라벨을 머리 위 라벨과
// 같은 파생 규칙으로 한 줄 표시한다. 세션이 starting/running이 아니면 실황
// (line2)은 stale이므로 억제하고 line1만 흐리게. 라벨이 없으면 미표시.
//
// 이슈 #44 2단계: 오른쪽 끝 사용량(토큰·비용) 스팬(docs/session-analytics-design.md
// §11). useSessionUsageSeed가 매 렌더 호출되므로 tauriApi.loadSessionEvents를
// 목업해 실제 invoke를 타지 않게 한다 — 아래 사용량 테스트는 시드 자체를
// setSessionUsageSeed로 직접 심어 검증하므로 훅의 fetch 결과에는 의존하지 않는다.
import { cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile } from "../../store/types";

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: { loadSessionEvents: vi.fn().mockResolvedValue([]) },
}));

import { useAppStore } from "../../store/appStore";
import { initialTurnState } from "../../timeline/turnReducer";
import { formatTokens, formatUsd } from "../../analytics/pricing";
import type { SessionUsageTotals } from "../../usage/sessionCost";
import { TerminalSummaryBar } from "../TerminalSummaryBar";

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
  sessionCostEnabled?: boolean;
}) {
  const s = useAppStore.getState();
  useAppStore.setState({
    activeTerminalAgentId: opts.activeId === undefined ? "a1" : opts.activeId,
    agents: { a1: agent("a1", "/Users/me/dev/agent-office") },
    sessions: {
      a1: { agentId: "a1", status: opts.status ?? "running", cols: 80, rows: 24, lastActivityAt: 0 },
    },
    taskLabels: opts.label ? { a1: { sessionId: "s1", ...opts.label } } : {},
    timeTracking: { a1: { ...initialTurnState(), phase: opts.phase ?? "working" } },
    appSettings: { ...s.appSettings, sessionCostEnabled: opts.sessionCostEnabled ?? s.appSettings.sessionCostEnabled },
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

describe("TerminalSummaryBar 사용량 스팬 (이슈 #44 2단계)", () => {
  function mkTotals(overrides: Partial<SessionUsageTotals> = {}): SessionUsageTotals {
    return {
      input: 0,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      costUsd: 0,
      costUnknownTurns: 0,
      turns: 0,
      ...overrides,
    };
  }

  it("설정(sessionCostEnabled)이 꺼져 있으면 사용량이 있어도 스팬을 렌더하지 않는다", () => {
    seed({ label: { goal: "버그 수정" }, sessionCostEnabled: false });
    useAppStore.setState({
      sessionUsage: {
        a1: { sessionId: "s1", totals: mkTotals({ input: 1000, output: 500, turns: 2, model: "claude-opus-5" }) },
      },
    });
    const { container } = render(<TerminalSummaryBar />);
    expect(container.querySelector(".terminal-summary-usage")).toBeNull();
  });

  it("turns===0이면 사용량 스팬을 렌더하지 않는다", () => {
    seed({ label: { goal: "버그 수정" }, sessionCostEnabled: true });
    useAppStore.setState({
      sessionUsage: { a1: { sessionId: "s1", totals: mkTotals() } },
    });
    const { container } = render(<TerminalSummaryBar />);
    expect(container.querySelector(".terminal-summary-usage")).toBeNull();
  });

  it("라벨이 없어도 사용량만 있으면 바가 보인다(hidden 클래스 없음)", () => {
    seed({ label: undefined, status: "running", sessionCostEnabled: true });
    useAppStore.setState({
      agents: { a1: agent("a1") }, // cwd도 비워 라벨 부재를 확실히 한다.
      sessionUsage: {
        a1: { sessionId: "s1", totals: mkTotals({ input: 1000, output: 500, costUsd: 0.01, turns: 2, model: "claude-opus-5" }) },
      },
    });
    const { container } = render(<TerminalSummaryBar />);
    const bar = container.querySelector(".terminal-summary-bar")!;
    expect(bar.className).not.toContain("terminal-summary-hidden");
    const usage = bar.querySelector(".terminal-summary-usage");
    expect(usage).not.toBeNull();
    expect(usage!.textContent).toBe(`${formatTokens(1500)} · ${formatUsd(0.01)}`);
  });

  it("시드와 실시간 누계를 합쳐서 그린다", () => {
    seed({ label: { goal: "버그 수정" }, sessionCostEnabled: true });
    useAppStore.setState({
      sessionUsage: {
        a1: { sessionId: "s1", totals: mkTotals({ input: 100, output: 50, costUsd: 0.1, turns: 1, model: "claude-opus-5" }) },
      },
      sessionUsageSeed: {
        at: 500,
        bySession: {
          s1: mkTotals({ input: 900, output: 450, costUsd: 0.4, turns: 4, model: "claude-opus-5" }),
        },
      },
    });
    const { container } = render(<TerminalSummaryBar />);
    const usage = container.querySelector(".terminal-summary-usage")!;
    expect(usage).not.toBeNull();
    // 입력 100+900=1000, 출력 50+450=500 → 합계 1500 토큰, 비용 0.1+0.4=0.5, 턴 1+4=5.
    expect(usage.textContent).toBe(`${formatTokens(1500)} · ${formatUsd(0.5)}`);
    expect(usage.getAttribute("title")).toContain("5턴");
  });

  it("costUnknownTurns가 있으면 비용 앞에 물결(~)을 붙이고 툴팁에 뜻을 남긴다", () => {
    seed({ label: { goal: "버그 수정" }, sessionCostEnabled: true });
    useAppStore.setState({
      sessionUsage: {
        a1: {
          sessionId: "s1",
          totals: mkTotals({ input: 100, costUsd: 0.1, costUnknownTurns: 1, turns: 2, model: "llama-3" }),
        },
      },
    });
    const { container } = render(<TerminalSummaryBar />);
    const usage = container.querySelector(".terminal-summary-usage")!;
    expect(usage.textContent).toBe(`${formatTokens(100)} · ~${formatUsd(0.1)}`);
    // D-2: "~"의 뜻이 툴팁에 한 줄 더 있어야 한다(summary.usage.costUnknownHint).
    expect(usage.getAttribute("title")).toContain("1턴");
    expect(usage.getAttribute("title")).toContain("제외");
  });

  it("D-1: 비용을 하나도 모르면(costUnknownTurns===turns) 비용 자리를 —로 떨구고 토큰은 그대로 보여준다", () => {
    seed({ label: { goal: "버그 수정" }, sessionCostEnabled: true });
    useAppStore.setState({
      sessionUsage: {
        a1: {
          sessionId: "s1",
          totals: mkTotals({ input: 1_000_000, output: 200_000, costUsd: 0, costUnknownTurns: 2, turns: 2, model: "llama-3" }),
        },
      },
    });
    const { container } = render(<TerminalSummaryBar />);
    const usage = container.querySelector(".terminal-summary-usage")!;
    expect(usage.textContent).toBe(`${formatTokens(1_200_000)} · —`);
  });

  it("D-3: 캐시만 있는 턴(input+output===0, cacheRead>0)이면 토큰 자리를 0이 아니라 —로 보여준다", () => {
    seed({ label: { goal: "버그 수정" }, sessionCostEnabled: true });
    useAppStore.setState({
      sessionUsage: {
        a1: {
          sessionId: "s1",
          totals: mkTotals({ input: 0, output: 0, cacheRead: 500, costUsd: 0.0012, turns: 1, model: "claude-opus-5" }),
        },
      },
    });
    const { container } = render(<TerminalSummaryBar />);
    const usage = container.querySelector(".terminal-summary-usage")!;
    expect(usage.textContent).toBe(`— · ${formatUsd(0.0012)}`);
  });
});
