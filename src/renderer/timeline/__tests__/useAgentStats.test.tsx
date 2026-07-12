// @vitest-environment jsdom
// src/renderer/timeline/__tests__/useAgentStats.test.tsx
//
// Hook tests for useAgentStats: baseline-snapshot-then-reload ordering, error
// + retry, day-rollover reload, and no-load-while-closed. Only tauriApi is
// mocked (bootstrap.test convention). See
// docs/superpowers/specs/2026-07-11-per-agent-stats-design.md.
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import type { SessionTurnRecord } from "@shared/types";

const { mockApi } = vi.hoisted(() => ({ mockApi: { loadSessionTurns: vi.fn() } }));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: mockApi }));

import { useAppStore } from "../../store/appStore";
import { useAgentStats } from "../useAgentStats";

function rec(o: Partial<SessionTurnRecord> = {}): SessionTurnRecord {
  return { agentId: "a", startedAt: 0, endedAt: Date.now(), totalMs: 100, workedMs: 100, waitedMs: 0, ...o };
}

// RTL auto-cleanup needs vitest globals (not enabled in this project's
// vitest.config.ts — see its "environment" comment); without an explicit
// unmount, prior tests' renderHook instances stay subscribed to the shared
// zustand store and re-fire their effects on later store updates, inflating
// call counts. Same explicit-cleanup convention as
// SessionTimePanel.test.tsx / ConfirmRestartDialog.test.tsx.
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  useAppStore.setState({ timeTracking: {}, agents: {}, todayWorkedBaseMs: 0 } as never);
});

describe("useAgentStats", () => {
  it("does not load while closed", () => {
    mockApi.loadSessionTurns.mockResolvedValue([]);
    renderHook(() => useAgentStats(false));
    expect(mockApi.loadSessionTurns).not.toHaveBeenCalled();
  });

  it("loads once on open and produces rows", async () => {
    mockApi.loadSessionTurns.mockResolvedValue([rec({ agentId: "a", workedMs: 500 })]);
    useAppStore.setState({ agents: { a: { name: "Alice" } } } as never);
    const { result } = renderHook(() => useAgentStats(true));
    await waitFor(() => expect(result.current.rows.length).toBe(1));
    expect(mockApi.loadSessionTurns).toHaveBeenCalledTimes(1);
    expect(result.current.rows[0]).toMatchObject({ label: "Alice", totalWorkedMs: 500 });
  });

  it("surfaces error and reloads on retry", async () => {
    mockApi.loadSessionTurns.mockRejectedValueOnce(new Error("boom")).mockResolvedValueOnce([]);
    const { result } = renderHook(() => useAgentStats(true));
    await waitFor(() => expect(result.current.error).toBe(true));
    act(() => result.current.retry());
    await waitFor(() => expect(result.current.error).toBe(false));
    expect(mockApi.loadSessionTurns).toHaveBeenCalledTimes(2);
  });

  it("reloads when todayWorkedBaseMs changes (day rollover)", async () => {
    mockApi.loadSessionTurns.mockResolvedValue([]);
    renderHook(() => useAgentStats(true));
    await waitFor(() => expect(mockApi.loadSessionTurns).toHaveBeenCalledTimes(1));
    // baseMs unchanged (0->0) here, but memoryWorkedBaselineMs flips 0->999 —
    // that alone reloads (call #2), proving the OR-dependency.
    act(() => useAppStore.setState({ todayWorkedBaseMs: 0, memoryWorkedBaselineMs: 999 } as never));
    await waitFor(() => expect(mockApi.loadSessionTurns).toHaveBeenCalledTimes(2));
    // Then baseMs itself flips 0->1 -> reloads again (call #3).
    act(() => useAppStore.setState({ todayWorkedBaseMs: 1 } as never));
    await waitFor(() => expect(mockApi.loadSessionTurns).toHaveBeenCalledTimes(3));
  });

  it("reloads on the real midnight rollover even when todayWorkedBaseMs was already 0 (regression)", async () => {
    // 부팅 시 당일 디스크 기록이 없어 base=0으로 시작한 흔한 케이스를 재현.
    // 이 상태에서 자정 롤오버가 오면 setTodayWorkedBase(0, memorySum)이
    // 호출되는데, todayWorkedBaseMs는 0->0으로 안 바뀌므로 그것에만 의존하면
    // 재로드가 누락된다. memoryWorkedBaselineMs는 0->memorySum으로 바뀌므로
    // 이를 통해 재로드가 보장돼야 한다.
    mockApi.loadSessionTurns.mockResolvedValue([]);
    useAppStore.setState({
      todayWorkedBaseMs: 0,
      memoryWorkedBaselineMs: 0,
      timeTracking: { a: { workedMs: 1234 } },
    } as never);
    renderHook(() => useAgentStats(true));
    await waitFor(() => expect(mockApi.loadSessionTurns).toHaveBeenCalledTimes(1));

    const memorySum = Object.values(useAppStore.getState().timeTracking).reduce(
      (sum, t) => sum + (t as { workedMs: number }).workedMs,
      0
    );
    act(() => useAppStore.getState().setTodayWorkedBase(0, memorySum));

    await waitFor(() => expect(mockApi.loadSessionTurns).toHaveBeenCalledTimes(2));
  });
});
