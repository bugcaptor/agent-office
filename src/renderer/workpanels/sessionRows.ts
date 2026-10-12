import type { SessionEventRecord, SessionSummary, WorkItem } from "@shared/types";
import { aggregate, reconstructTurns, type AgentSummary, type Turn } from "../analytics/aggregate";
import { estimateCostBreakdown } from "../analytics/pricing";

import { workItemTitle } from "./workItemDraft";

export interface SessionRow extends SessionSummary {
  live: boolean;
  workTitle?: string;
  stats?: AgentSummary;
  turns: Turn[];
  usage: { at: number; model?: string; tokens: number; costUsd: number; unknown: boolean }[];
}
export function buildSessionRows(events: readonly SessionEventRecord[], summaries: Record<string, SessionSummary>, workItems: WorkItem[], liveIds: readonly string[], now = Date.now()): SessionRow[] {
  const groups = new Map<string, SessionEventRecord[]>();
  for (const event of events) {
    const group = groups.get(event.sessionId) ?? [];
    group.push(event); groups.set(event.sessionId, group);
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
    const summary: SessionSummary = {
      sessionId, agentId, agentName: started?.agentName ?? stored?.agentName ?? agentId,
      cwd: started?.cwd ?? stored?.cwd,
      startedAt: started?.at ?? stored?.startedAt ?? first.at,
      endedAt: live ? undefined : ended?.at ?? stored?.endedAt,
      goal: stored?.goal,
      prompts: stored?.prompts.map((p) => ({ ...p, origin: p.origin ?? group.find((e) => e.kind === "prompt" && e.at === p.at)?.origin })) ?? [],
      workItemId: stored?.workItemId,
    };
    if (summary.startedAt < now - 300 * 86_400_000) continue;
    const work = workItems.find((w) => w.id === summary.workItemId || w.sessionIds.includes(sessionId));
    // 분석 패널의 턴·토큰·비용 집계를 그대로 재사용한다. 열린 턴은 현재 시각까지 계산.
    const current = live ? [...group, { schemaVersion: 1, runId: "live", seq: 0, kind: "session_state" as const, state: "running" as const, sessionId, agentId, at: now }] : group;
    const stats = aggregate(current, {}).summary.find((s) => s.agentId === agentId);
    rows.push({ ...summary, live, workItemId: work?.id ?? summary.workItemId, workTitle: work ? workItemTitle(work) : undefined, stats, turns: reconstructTurns(current), usage: group.filter((e) => e.tokens).flatMap((e) => {
      const components = e.tokens!.byModel?.length ? e.tokens!.byModel : [e.tokens!];
      return components.map((tokens) => {
        const cost = estimateCostBreakdown(tokens);
        return { at: e.at, model: tokens.model, tokens: (tokens.input ?? 0) + (tokens.output ?? 0) + (tokens.cacheRead ?? 0) + (tokens.cacheWrite ?? 0), costUsd: cost?.costUsd ?? 0, unknown: cost?.hasUnknown ?? true };
      });
    }) });
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
