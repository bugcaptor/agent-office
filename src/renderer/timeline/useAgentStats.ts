// src/renderer/timeline/useAgentStats.ts
//
// Hook backing the per-agent stats view. On open it snapshots the current
// in-memory workedMs baseline (synchronously) THEN reloads the disk log, so
// turns settled after open are counted exactly once (as a live delta) and
// never double-counted against the freshly-read file. Live-updates via a
// memoized buildAgentStatsRows over the timeTracking/agents store slices, and
// re-runs when the day rolls over (todayWorkedBaseMs OR memoryWorkedBaselineMs
// flips — either one changing at midnight triggers re-aggregation). See
// docs/superpowers/specs/2026-07-11-per-agent-stats-design.md.
import { useEffect, useMemo, useState } from "react";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { startOfLocalDay } from "./todayTotal";
import {
  aggregateByAgent,
  buildAgentStatsRows,
  type AgentStatsAgg,
  type AgentStatsRow,
} from "./agentStats";

export interface UseAgentStatsResult {
  rows: AgentStatsRow[];
  loading: boolean;
  error: boolean;
  retry(): void;
}

/** 현재 timeTracking에서 agentId -> workedMs만 뽑는다(기준선 스냅샷/라이브 델타용). */
function workedByAgent(tt: Record<string, { workedMs: number }>): Record<string, number> {
  const out: Record<string, number> = {};
  for (const [id, t] of Object.entries(tt)) out[id] = t.workedMs;
  return out;
}

export function useAgentStats(open: boolean): UseAgentStatsResult {
  const timeTracking = useAppStore((s) => s.timeTracking);
  const agents = useAppStore((s) => s.agents);
  const todayWorkedBaseMs = useAppStore((s) => s.todayWorkedBaseMs);
  const memoryWorkedBaselineMs = useAppStore((s) => s.memoryWorkedBaselineMs);

  const [diskAgg, setDiskAgg] = useState<Record<string, AgentStatsAgg> | null>(null);
  const [baseline, setBaseline] = useState<Record<string, number>>({});
  const [inFlight, setInFlight] = useState(false);
  const [error, setError] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    // 1) 기준선 스냅샷(동기) → 2) 파일 재읽기. 순서 필수(역순이면 사이 정산분 누락).
    setBaseline(workedByAgent(useAppStore.getState().timeTracking));
    setInFlight(true);
    setError(false);
    tauriApi
      .loadSessionTurns()
      .then((records) => {
        if (cancelled) return;
        setDiskAgg(aggregateByAgent(records, startOfLocalDay(Date.now())));
        setInFlight(false);
      })
      .catch((err) => {
        if (cancelled) return;
        console.warn("useAgentStats: loadSessionTurns 실패", err);
        setError(true);
        setInFlight(false);
      });
    return () => {
      cancelled = true;
    };
    // todayWorkedBaseMs, memoryWorkedBaselineMs: 자정 롤오버가 둘 중 하나라도
    // 값을 바꾸면 오늘 재집계(둘 다 바뀌는 통상 케이스뿐 아니라, base=0으로
    // 부팅해 base가 그대로 0인 날에도 baseline은 0→Σ메모리로 바뀌어 재로드가
    // 보장됨). reloadKey: retry.
  }, [open, todayWorkedBaseMs, memoryWorkedBaselineMs, reloadKey]);

  const rows = useMemo(
    () => (diskAgg ? buildAgentStatsRows(diskAgg, workedByAgent(timeTracking), baseline, agents) : []),
    [diskAgg, timeTracking, baseline, agents]
  );

  // 첫 로드 전(diskAgg null, 에러 아님)에도 loading으로 취급 — effect 실행 전
  // 한 프레임 "기록 없음" 깜빡임 방지.
  const loading = inFlight || (open && diskAgg === null && !error);

  return { rows, loading, error, retry: () => setReloadKey((k) => k + 1) };
}
