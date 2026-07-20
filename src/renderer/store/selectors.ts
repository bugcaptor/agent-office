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

/** 근무 중(퇴근하지 않은) 에이전트 목록, 생성 순서. 오피스 캔버스가 소비 —
 * 퇴근한 에이전트는 여기서 빠지므로 캔버스/탕비실에서 사라진다. */
export const useAgentList = () =>
  useAppStore(
    useShallow((s) => s.agentOrder.map((id) => s.agents[id]).filter((a) => !a.clockedOut))
  );

/** 퇴근한 에이전트 목록(소환 UI용), 생성 순서. */
export const useClockedOutAgents = () =>
  useAppStore(useShallow((s) => s.agentOrder.map((id) => s.agents[id]).filter((a) => a?.clockedOut)));

/** 퇴근한 에이전트 수(소환 버튼 배지용). */
export const useClockedOutCount = () =>
  useAppStore((s) => s.agentOrder.reduce((n, id) => n + (s.agents[id]?.clockedOut ? 1 : 0), 0));

export const useAgentCount = () => useAppStore((s) => s.agentOrder.length);

/** 사무실 소등 여부: 에이전트가 하나 이상 있으나 전원 퇴근했을 때 true.
 * (에이전트가 아예 없는 빈 새 사무실은 소등하지 않는다.) */
export const useLightsOff = () =>
  useAppStore(
    (s) => s.agentOrder.length > 0 && s.agentOrder.every((id) => s.agents[id]?.clockedOut)
  );

export const useRunningCount = () =>
  useAppStore((s) => Object.values(s.sessions).filter((x) => x.status === "running").length);

/** 알림 대기 중인 agentId 집합 — "pending" 정의의 단일 소스(배지·오피스 릴레이·replay 공용). */
export const pendingAgentIds = (notifications: ReadonlyArray<{ agentId: string }>): Set<string> =>
  new Set(notifications.map((n) => n.agentId));

export const usePendingCount = () => useAppStore((s) => pendingAgentIds(s.notifications).size);

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
      agentOrder.filter((id) => !agents[id]?.clockedOut).map((id) => {
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

/**
 * "오늘 일한 시간" 헤드라인. `todayWorkedBaseMs + (Σ메모리 workedMs -
 * memoryWorkedBaselineMs)` — 부팅/자정 리셋 베이스라인 위에 이번 실행에서
 * 새로 정산된 workedMs 델타만 더한다(이중 집계 없음). 계산 모델은
 * docs/superpowers/specs/2026-07-11-today-worked-total-design.md 참고.
 */
export const useTodayWorkedMs = (): number => {
  const base = useAppStore((s) => s.todayWorkedBaseMs);
  const baseline = useAppStore((s) => s.memoryWorkedBaselineMs);
  const timeTracking = useAppStore((s) => s.timeTracking);
  return useMemo(
    () =>
      base +
      Object.values(timeTracking).reduce((a, t) => a + t.workedMs, 0) -
      baseline,
    [base, baseline, timeTracking]
  );
};
