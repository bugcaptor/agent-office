// src/renderer/timeline/agentStats.ts
//
// Pure helpers backing the per-agent "누적 통계" view in SessionTimePanel.
// No React/zustand/IPC dependency — records/time/store slices are passed in
// so these stay deterministically testable (sibling of todayTotal.ts).
// See docs/superpowers/specs/2026-07-11-per-agent-stats-design.md.
import type { SessionTurnRecord } from "@shared/types";

/** 에이전트 1명의 디스크 집계: 누적/오늘 worked(대기 제외). */
export interface AgentStatsAgg {
  totalWorkedMs: number;
  todayWorkedMs: number;
}

/** 통계 뷰 렌더 행(정렬 완료). */
export interface AgentStatsRow {
  agentId: string;
  /** 표시 이름(명부에 있으면 name, 없으면 축약 id + "(퇴사)"). */
  label: string;
  /** 명부에서 사라진 과거 에이전트면 true(dim 표시용). */
  departed: boolean;
  totalWorkedMs: number;
  todayWorkedMs: number;
}

/**
 * JSONL 레코드를 agentId별로 집계한다. 오늘 = `endedAt >= sinceMs`(경계 포함,
 * todayTotal.sumWorkedSince와 동일 규칙).
 */
export function aggregateByAgent(
  records: SessionTurnRecord[],
  sinceMs: number
): Record<string, AgentStatsAgg> {
  const out: Record<string, AgentStatsAgg> = {};
  for (const r of records) {
    const agg = (out[r.agentId] ??= { totalWorkedMs: 0, todayWorkedMs: 0 });
    agg.totalWorkedMs += r.workedMs;
    if (r.endedAt >= sinceMs) agg.todayWorkedMs += r.workedMs;
  }
  return out;
}

/** 명부에 없는 과거 에이전트용 축약 라벨. */
function departedLabel(agentId: string): string {
  return `${agentId.slice(0, 8)}… (퇴사)`;
}

/**
 * 디스크 집계 + 이번 실행 라이브 델타를 합쳐 정렬된 렌더 행을 만든다.
 * 델타 = max(0, memory − baseline)를 총/오늘 양쪽에 더한다(열람 후 정산분은
 * endedAt이 항상 오늘 자정 이후이므로 오늘에 귀속). removeAgent로 memory가
 * 사라져 음수가 되는 경우를 클램프로 방어. 정렬: 총 desc → 오늘 desc → 라벨.
 */
export function buildAgentStatsRows(
  diskAgg: Record<string, AgentStatsAgg>,
  memoryWorkedByAgent: Record<string, number>,
  baselineByAgent: Record<string, number>,
  agents: Record<string, { name: string; clockedOut?: boolean }>
): AgentStatsRow[] {
  const ids = new Set<string>([...Object.keys(diskAgg), ...Object.keys(memoryWorkedByAgent)]);
  const rows: AgentStatsRow[] = [];
  for (const id of ids) {
    const disk = diskAgg[id] ?? { totalWorkedMs: 0, todayWorkedMs: 0 };
    const delta = Math.max(0, (memoryWorkedByAgent[id] ?? 0) - (baselineByAgent[id] ?? 0));
    const profile = agents[id];
    rows.push({
      agentId: id,
      label: profile ? profile.name : departedLabel(id),
      departed: !profile,
      totalWorkedMs: disk.totalWorkedMs + delta,
      todayWorkedMs: disk.todayWorkedMs + delta,
    });
  }
  rows.sort(
    (a, b) =>
      b.totalWorkedMs - a.totalWorkedMs ||
      b.todayWorkedMs - a.todayWorkedMs ||
      a.label.localeCompare(b.label)
  );
  return rows;
}
