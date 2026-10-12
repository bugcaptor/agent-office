import type { SessionEventRecord, SessionSummary, WorkItem } from "@shared/types";
import { aggregate, reconstructTurns, type AgentSummary, type Turn } from "../analytics/aggregate";
import { estimateCostBreakdown } from "../analytics/pricing";

import { workItemTitle } from "./workItemDraft";

export interface SessionRow extends SessionSummary {
  live: boolean;
  /** 종료 이벤트 없이 앱 실행(run)이 끝난 세션. endedAt은 그 run의 마지막 이벤트 시각이다. */
  endedApprox?: boolean;
  /** 프롬프트·턴·사용량·연결 업무가 모두 없는 세션(앱 기동 때 열린 빈 셸 등). */
  empty: boolean;
  workTitle?: string;
  stats?: AgentSummary;
  turns: Turn[];
  usage: { model?: string; tokens: number; costUsd: number; unknown: boolean }[];
}
export function buildSessionRows(events: readonly SessionEventRecord[], summaries: Record<string, SessionSummary>, workItems: WorkItem[], liveIds: readonly string[], now = Date.now()): SessionRow[] {
  const groups = new Map<string, SessionEventRecord[]>();
  for (const event of events) {
    const group = groups.get(event.sessionId) ?? [];
    group.push(event); groups.set(event.sessionId, group);
  }
  // 앱 종료는 PTY를 kill만 하고 프로세스가 먼저 끝나 exited/disposed가 남지 않을 때가 많다.
  // 이전 run의 미종료 세션은 그 run이 마지막으로 기록한 시각에 끝난 것으로 본다.
  const runLastAt = new Map<string, number>();
  let currentRun: string | undefined; let latestAt = -Infinity;
  for (const event of events) {
    runLastAt.set(event.runId, Math.max(runLastAt.get(event.runId) ?? -Infinity, event.at));
    if (event.at > latestAt) { latestAt = event.at; currentRun = event.runId; }
  }
  const ids = new Set([...groups.keys(), ...Object.keys(summaries)]);
  const rows: SessionRow[] = [];
  for (const sessionId of ids) {
    const group = [...(groups.get(sessionId) ?? [])].sort((a, b) => a.at - b.at || a.runId.localeCompare(b.runId) || a.seq - b.seq);
    const started = group.find((e) => e.kind === "session_started");
    const ended = group.find((e) => e.kind === "session_state" && (e.state === "exited" || e.state === "disposed"));
    const stored = summaries[sessionId];
    const first = group[0];
    if (!stored && !first) continue;
    const agentId = started?.agentId ?? stored?.agentId ?? first.agentId;
    const live = liveIds.includes(sessionId) && !ended;
    const run = first?.runId;
    const endedApprox = !live && !ended && stored?.endedAt === undefined && run !== undefined && run !== currentRun;
    const summary: SessionSummary = {
      sessionId, agentId, agentName: started?.agentName ?? stored?.agentName ?? agentId,
      cwd: started?.cwd ?? stored?.cwd,
      startedAt: started?.at ?? stored?.startedAt ?? first.at,
      endedAt: live ? undefined : ended?.at ?? stored?.endedAt ?? (endedApprox ? runLastAt.get(run!) : undefined),
      goal: stored?.goal,
      prompts: stored?.prompts.map((p) => ({ ...p, origin: p.origin ?? group.find((e) => e.kind === "prompt" && e.at === p.at)?.origin })) ?? [],
      workItemId: stored?.workItemId,
    };
    if (summary.startedAt < now - 300 * 86_400_000) continue;
    const work = workItems.find((w) => w.id === summary.workItemId || w.sessionIds.includes(sessionId));
    // 분석 패널의 턴·토큰·비용 집계를 그대로 재사용한다. 열린 턴은 현재 시각까지 계산.
    const current = live ? [...group, { schemaVersion: 1, runId: "live", seq: 0, kind: "session_state" as const, state: "running" as const, sessionId, agentId, at: now }] : group;
    const stats = aggregate(current, {}).summary.find((s) => s.agentId === agentId);
    const usage = new Map<string, SessionRow["usage"][number]>();
    for (const e of group) {
      if (!e.tokens) continue;
      for (const tokens of e.tokens.byModel?.length ? e.tokens.byModel : [e.tokens]) {
        const cost = estimateCostBreakdown(tokens);
        const entry = usage.get(tokens.model ?? "") ?? { model: tokens.model, tokens: 0, costUsd: 0, unknown: false };
        entry.tokens += (tokens.input ?? 0) + (tokens.output ?? 0) + (tokens.cacheRead ?? 0) + (tokens.cacheWrite ?? 0);
        entry.costUsd += cost?.costUsd ?? 0; entry.unknown ||= cost?.hasUnknown ?? true;
        usage.set(tokens.model ?? "", entry);
      }
    }
    const turns = reconstructTurns(current);
    const empty = !summary.prompts.length && !turns.length && !usage.size && !work && !summary.workItemId;
    rows.push({ ...summary, live, endedApprox: endedApprox || undefined, empty, workItemId: work?.id ?? summary.workItemId, workTitle: work ? workItemTitle(work) : undefined, stats, turns, usage: [...usage.values()] });
  }
  return rows.sort((a, b) => Number(b.live) - Number(a.live) || b.startedAt - a.startedAt);
}
export function folderName(cwd?: string): string { return cwd?.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? ""; }
export interface SessionFilters { query: string; agentIds: string[]; cwds: string[]; fromAt: number }
export function filterSessionRows(rows: SessionRow[], filter: SessionFilters): SessionRow[] {
  const query = filter.query.trim().toLocaleLowerCase();
  return rows.filter((r) => (r.live || (r.endedAt ?? r.startedAt) >= filter.fromAt)
    && (!filter.agentIds.length || filter.agentIds.includes(r.agentId))
    && (!filter.cwds.length || filter.cwds.includes(r.cwd ?? ""))
    && (!query || [r.goal, ...r.prompts.map((p) => p.text), r.cwd, folderName(r.cwd), r.agentName, r.workTitle].some((s) => s?.toLocaleLowerCase().includes(query))));
}
