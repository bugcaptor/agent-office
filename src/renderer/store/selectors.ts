// src/renderer/store/selectors.ts
//
// Derived selector hooks. Selectors that build a fresh
// array/object each call are typically wrapped in `useShallow` to avoid
// infinite re-render loops from zustand's reference-equality check; selectors
// that build fresh *nested* objects (each element itself a fresh object, so
// `useShallow`'s one-level shallow-equal still fails per element) instead
// subscribe to raw slices and memoize with `useMemo` — see
// `useSessionTimeRows` below.
import { useMemo } from "react";
import { useShallow } from "zustand/react/shallow";
import { useAppStore } from "./appStore";
import { initialTurnState } from "../timeline/turnReducer";
import type { TurnPhase } from "../timeline/turnReducer";

export const useAgentList = () =>
  useAppStore(useShallow((s) => s.agentOrder.map((id) => s.agents[id])));

export const useAgentCount = () => useAppStore((s) => s.agentOrder.length);

export const useRunningCount = () =>
  useAppStore((s) => Object.values(s.sessions).filter((x) => x.status === "running").length);

export const usePendingCount = () =>
  useAppStore((s) => new Set(s.notifications.map((n) => n.agentId)).size);

export const useActiveAgentId = () => useAppStore((s) => s.activeTerminalAgentId);

export interface SessionTimeRow {
  agentId: string;
  name: string;
  phase: TurnPhase;
  /** 열린 턴 시작 백엔드 ms(없으면 null). 라이브 경과는 컴포넌트가 계산. */
  turnStartedAt: number | null;
  totalMs: number;
  workedMs: number;
  waitedMs: number;
  turns: number;
}

/**
 * 에이전트 생성 순서대로 시간 추적 행 목록(집계 없으면 idle 기본값).
 * 매 호출 새 행 객체를 만들기 때문에 useShallow로는 안정 참조를 얻을 수
 * 없다(원소별 Object.is 비교 실패). 대신 원시 슬라이스 3개를 구독하고
 * useMemo로 감싸 agentOrder/agents/timeTracking 참조가 바뀔 때만 재계산한다.
 */
export const useSessionTimeRows = (): SessionTimeRow[] => {
  const agentOrder = useAppStore((s) => s.agentOrder);
  const agents = useAppStore((s) => s.agents);
  const timeTracking = useAppStore((s) => s.timeTracking);
  return useMemo(
    () =>
      agentOrder.map((id) => {
        const t = timeTracking[id] ?? initialTurnState();
        return {
          agentId: id,
          name: agents[id]?.name ?? id,
          phase: t.phase,
          turnStartedAt: t.turnStartedAt,
          totalMs: t.totalMs,
          workedMs: t.workedMs,
          waitedMs: t.waitedMs,
          turns: t.turns,
        };
      }),
    [agentOrder, agents, timeTracking]
  );
};
