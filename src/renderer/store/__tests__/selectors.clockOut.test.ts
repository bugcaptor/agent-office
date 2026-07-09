// @vitest-environment jsdom
//
// src/renderer/store/__tests__/selectors.clockOut.test.ts
//
// Coverage:
// - useAgentList excludes clocked-out agents (office canvas consumer).
// - useClockedOutAgents returns only clocked-out agents, creation order.
// - useClockedOutCount counts clocked-out agents.
// - useLightsOff: false with no agents, false with a mix, true when every
//   agent is clocked out.
import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useAppStore } from "../appStore";
import {
  useAgentList,
  useClockedOutAgents,
  useClockedOutCount,
  useLightsOff,
} from "../selectors";
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

describe("useAgentList", () => {
  it("퇴근한 에이전트를 제외한다", () => {
    act(() => {
      useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
      useAppStore.getState().addAgent(mkProfile({ id: "a2" }));
      useAppStore.getState().clockOut("a1");
    });
    const { result } = renderHook(() => useAgentList());
    expect(result.current.map((a) => a.id)).toEqual(["a2"]);
  });
});

describe("useClockedOutAgents", () => {
  it("퇴근한 에이전트만, 생성 순서로 반환한다", () => {
    act(() => {
      useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
      useAppStore.getState().addAgent(mkProfile({ id: "a2" }));
      useAppStore.getState().clockOut("a1");
    });
    const { result } = renderHook(() => useClockedOutAgents());
    expect(result.current.map((a) => a.id)).toEqual(["a1"]);
  });

  it("아무도 퇴근하지 않았으면 빈 배열", () => {
    act(() => {
      useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    });
    const { result } = renderHook(() => useClockedOutAgents());
    expect(result.current).toEqual([]);
  });
});

describe("useClockedOutCount", () => {
  it("퇴근한 에이전트 수를 센다", () => {
    act(() => {
      useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
      useAppStore.getState().addAgent(mkProfile({ id: "a2" }));
      useAppStore.getState().addAgent(mkProfile({ id: "a3" }));
      useAppStore.getState().clockOut("a1");
      useAppStore.getState().clockOut("a2");
    });
    const { result } = renderHook(() => useClockedOutCount());
    expect(result.current).toBe(2);
  });

  it("아무도 퇴근하지 않았으면 0", () => {
    act(() => {
      useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    });
    const { result } = renderHook(() => useClockedOutCount());
    expect(result.current).toBe(0);
  });
});

describe("useLightsOff", () => {
  it("에이전트가 하나도 없으면 false (빈 새 사무실은 소등하지 않는다)", () => {
    const { result } = renderHook(() => useLightsOff());
    expect(result.current).toBe(false);
  });

  it("일부만 퇴근했으면 false", () => {
    act(() => {
      useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
      useAppStore.getState().addAgent(mkProfile({ id: "a2" }));
      useAppStore.getState().clockOut("a1");
    });
    const { result } = renderHook(() => useLightsOff());
    expect(result.current).toBe(false);
  });

  it("전원 퇴근했으면 true", () => {
    act(() => {
      useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
      useAppStore.getState().addAgent(mkProfile({ id: "a2" }));
      useAppStore.getState().clockOut("a1");
      useAppStore.getState().clockOut("a2");
    });
    const { result } = renderHook(() => useLightsOff());
    expect(result.current).toBe(true);
  });
});
