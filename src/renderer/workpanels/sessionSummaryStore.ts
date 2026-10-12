import { create } from "zustand";
import type { SessionSummary, SessionStateEvent } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";

export const SUMMARY_RETENTION_MS = 300 * 86_400_000;
export const useSessionSummaryStore = create<{ summaries: Record<string, SessionSummary>; liveIds: string[]; error: boolean }>(() => ({ summaries: {}, liveIds: [], error: false }));
let active: { flush(): Promise<void>; pending(): boolean } | undefined;
export function hasPendingSessionSummaries(): boolean { return active?.pending() ?? false; }
export async function flushSessionSummaries(): Promise<void> { await active?.flush(); }
/** 설정과 무관한 상시 수집. 종료 flush와 실패 재시도 모두 같은 직렬 저장 큐를 쓴다. */
export async function installSessionSummaryRecorder(): Promise<() => void> {
  try {
    const summaries = await tauriApi.loadSessionSummaries();
    useSessionSummaryStore.setState({ summaries: Object.fromEntries(summaries.map((s) => [s.sessionId, s])), error: false });
  } catch (err) {
    console.warn("session-summaries: restore failed", err);
    useSessionSummaryStore.setState({ error: true });
  }
  const dirty = new Map<string, SessionSummary>();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let saving: Promise<void> | undefined;
  let disposed = false;
  const seenPrompts = new Map<string, number>();
  function put(summary: SessionSummary) {
    if (summary.startedAt < Date.now() - SUMMARY_RETENTION_MS) return;
    const summaries = useSessionSummaryStore.getState().summaries;
    useSessionSummaryStore.setState({ summaries: { ...summaries, [summary.sessionId]: summary } });
    dirty.set(summary.sessionId, summary);
    clearTimeout(timer);
    timer = setTimeout(() => void flush(), 1000);
  }
  async function flush(): Promise<void> {
    clearTimeout(timer); timer = undefined;
    if (saving) { await saving; return flush(); }
    if (dirty.size === 0 || disposed) return;
    const batch = [...dirty.values()];
    saving = (async () => {
      try {
        await tauriApi.appendSessionSummaries(batch);
        for (const summary of batch) if (dirty.get(summary.sessionId) === summary) dirty.delete(summary.sessionId);
        useSessionSummaryStore.setState({ error: false });
      } catch (err) {
        console.warn("session-summaries: save failed", err);
        useSessionSummaryStore.setState({ error: true });
        if (!disposed) timer = setTimeout(() => void flush(), 5000);
      }
    })();
    await saving; saving = undefined;
    if (dirty.size && !useSessionSummaryStore.getState().error) await flush();
  }
  function onState(e: SessionStateEvent) {
    const { agents } = useAppStore.getState();
    const { summaries, liveIds } = useSessionSummaryStore.getState();
    const previous = summaries[e.sessionId];
    const live = e.state === "starting" || e.state === "running";
    useSessionSummaryStore.setState({ liveIds: live ? [...new Set([...liveIds, e.sessionId])] : liveIds.filter((id) => id !== e.sessionId) });
    const base = previous ?? { sessionId: e.sessionId, agentId: e.agentId, agentName: agents[e.agentId]?.name ?? e.agentId, cwd: agents[e.agentId]?.cwd, startedAt: e.at, prompts: [] };
    // exited→disposed에서 실제 종료 시각을 덮어쓰지 않는다.
    if (!previous || (!live && previous.endedAt === undefined)) put({ ...base, ...(!live ? { endedAt: e.at } : {}) });
  }
  function observe() {
    const { taskLabels, agents, sessions, sessionUsage } = useAppStore.getState();
    for (const [agentId, usage] of Object.entries(sessionUsage)) {
      if (sessions[agentId]?.status === "running" || sessions[agentId]?.status === "starting") {
        const state = useSessionSummaryStore.getState();
        if (!state.liveIds.includes(usage.sessionId)) useSessionSummaryStore.setState({ liveIds: [...state.liveIds, usage.sessionId] });
      }
    }
    for (const [agentId, label] of Object.entries(taskLabels)) {
      const previous = useSessionSummaryStore.getState().summaries[label.sessionId];
      const base: SessionSummary = previous ?? { sessionId: label.sessionId, agentId, agentName: agents[agentId]?.name ?? agentId, cwd: label.cwd ?? agents[agentId]?.cwd, startedAt: label.latestPromptAt ?? Date.now(), prompts: [] };
      let next = base;
      if (label.latestPromptText && label.latestPromptAt !== undefined && seenPrompts.get(label.sessionId) !== label.latestPromptAt) {
        seenPrompts.set(label.sessionId, label.latestPromptAt);
        if (!base.prompts.some((p) => p.at === label.latestPromptAt) && base.prompts.length < 50) {
          next = { ...next, prompts: [...next.prompts, { at: label.latestPromptAt, text: Array.from(label.latestPromptText).slice(0, 300).join("") }] };
        }
      }
      if (label.goal && label.goal !== base.goal) next = { ...next, goal: label.goal };
      if (next !== base) put(next);
    }
  }
  const offState = tauriApi.onSessionState(onState);
  const offStore = useAppStore.subscribe((s) => [s.taskLabels, s.sessionUsage], observe);
  observe();
  const handle = { flush, pending: () => dirty.size > 0 || saving !== undefined };
  active = handle;
  return () => {
    // bootstrap teardown는 동기 계약. 정상 종료는 별도 flush를 기다린다.
    void flush().finally(() => { disposed = true; clearTimeout(timer); });
    offState(); offStore();
    if (active === handle) active = undefined;
  };
}
