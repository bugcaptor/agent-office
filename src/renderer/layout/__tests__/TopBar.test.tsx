// @vitest-environment jsdom
//
// src/renderer/layout/__tests__/TopBar.test.tsx
//
// TopBar는 카운트 표시 + "터미널 다시 열기" 진입점이다. 클릭 라우팅은
// NotificationTicker와 동일하게 `officeBus.emitAgentClicked`를 통과해야
// 하므로(세션 재생성 + openTerminal + 백엔드 알림 클리어 묶음 —
// sessionBridge.test.ts에서 검증됨) 브리지를 모킹해 라우팅만 확인한다.
//
// Coverage:
// - 카운트 표시(agents / running / pending).
// - 클릭 → emitAgentClicked(최근 탭 recentAgentIds[0]).
// - 최근 탭이 없으면 agentOrder[0]으로 폴백.
// - 에이전트가 없으면 비활성(disabled) + 클릭 no-op.
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

const emitAgentClicked = vi.fn();

vi.mock("../../ipc/sessionBridge", () => ({
  officeBus: {
    emitAgentClicked: (...args: unknown[]) => emitAgentClicked(...args),
  },
}));

const { TopBar } = await import("../TopBar");

function mkProfile(id: string, name?: string): AgentProfile {
  return {
    id,
    name: name ?? `Agent ${id}`,
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
  emitAgentClicked.mockClear();
});

afterEach(() => cleanup());

describe("rendering", () => {
  it("shows agent/running/pending counts", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.getState().addAgent(mkProfile("a2"));
    useAppStore.getState().setSessionState({ agentId: "a1", status: "running" });

    const { getByRole } = render(<TopBar />);
    const bar = getByRole("button");

    expect(bar.textContent).toContain("2 agents");
    expect(bar.textContent).toContain("1 running");
    expect(bar.textContent).toContain("0 pending");
  });
});

describe("click routing", () => {
  it("clicking the bar reopens the most recent terminal via officeBus", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.getState().addAgent(mkProfile("a2"));
    useAppStore.getState().openTerminal("a2"); // a2가 최근 탭
    useAppStore.getState().closeTerminal();

    const { getByRole } = render(<TopBar />);
    fireEvent.click(getByRole("button"));

    expect(emitAgentClicked).toHaveBeenCalledTimes(1);
    expect(emitAgentClicked).toHaveBeenCalledWith("a2");
  });

  it("falls back to the first created agent when no terminal was ever opened", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.getState().addAgent(mkProfile("a2"));

    const { getByRole } = render(<TopBar />);
    fireEvent.click(getByRole("button"));

    expect(emitAgentClicked).toHaveBeenCalledWith("a1");
  });

  it("is disabled and a no-op when there are no agents", () => {
    const { getByRole } = render(<TopBar />);
    const bar = getByRole("button") as HTMLButtonElement;

    expect(bar.disabled).toBe(true);
    fireEvent.click(bar);
    expect(emitAgentClicked).not.toHaveBeenCalled();
  });
});
