// @vitest-environment jsdom
//
// useSessionTimeRows referential-stability contract (task 9 review fix):
// unrelated store updates must NOT produce a new rows array reference;
// relevant updates (agentOrder/agents/timeTracking) must recompute.
import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useAppStore } from "../appStore";
import { useSessionTimeRows } from "../selectors";
import type { AgentProfile } from "../types";

function mkProfile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: "a1",
    name: "Test Agent",
    role: "backend",
    note: "",
    seed: "seed-a1",
    createdAt: Date.now(),
    deskIndex: 0,
    ...overrides,
  };
}

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
});

afterEach(() => cleanup());

describe("useSessionTimeRows", () => {
  it("keeps the same array reference across unrelated store updates", () => {
    act(() => {
      useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    });
    const { result } = renderHook(() => useSessionTimeRows());
    const first = result.current;
    expect(first).toHaveLength(1);

    // Unrelated updates: neither agentOrder, agents, nor timeTracking change.
    act(() => {
      useAppStore.getState().toggleMuted();
      useAppStore.getState().openModal({ kind: "profile-create" });
      useAppStore.getState().closeModal();
    });

    expect(result.current).toBe(first);
  });

  it("recomputes when an activity event updates timeTracking", () => {
    act(() => {
      useAppStore.getState().addAgent(mkProfile({ id: "a1", name: "Alpha" }));
    });
    const { result } = renderHook(() => useSessionTimeRows());
    const before = result.current;
    expect(before[0]).toMatchObject({
      agentId: "a1",
      name: "Alpha",
      phase: "idle",
      turnStartedAt: null,
    });

    act(() => {
      useAppStore.getState().applyActivityEvent({
        agentId: "a1",
        sessionId: "s1",
        kind: "prompt",
        at: 1000,
      });
    });

    expect(result.current).not.toBe(before);
    expect(result.current[0].phase).toBe("working");
    expect(result.current[0].turnStartedAt).toBe(1000);
  });

  it("퇴근한(clockedOut) 에이전트는 행에서 제외한다", () => {
    act(() => {
      useAppStore.getState().addAgent(mkProfile({ id: "a1", name: "Alpha" }));
      useAppStore.getState().addAgent(mkProfile({ id: "a2", name: "Beta" }));
      useAppStore.getState().clockOut("a1");
    });
    const { result } = renderHook(() => useSessionTimeRows());
    expect(result.current.map((r) => r.agentId)).toEqual(["a2"]);
  });

  it("falls back to the id as name when no agent profile exists", () => {
    // Seed timeTracking for an id that has no profile (agents record empty),
    // and put the id in agentOrder directly to exercise the `?? id` fallback.
    act(() => {
      useAppStore.setState((s) => ({ agentOrder: [...s.agentOrder, "ghost"] }));
    });
    const { result } = renderHook(() => useSessionTimeRows());
    expect(result.current).toHaveLength(1);
    expect(result.current[0]).toMatchObject({
      agentId: "ghost",
      name: "ghost",
      phase: "idle",
      totalMs: 0,
      workedMs: 0,
      waitedMs: 0,
      turns: 0,
    });
  });
});
