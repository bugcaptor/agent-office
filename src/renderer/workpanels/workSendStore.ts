import { create } from "zustand";
import type { AgentProfile, WorkItem, WorkSendTarget } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { flushPersistence } from "../store/persist";
import { reserveSessionReplacement, runGuardedCreateSession } from "../ipc/sessionBridge";
import { normalizeWorkAgent } from "../profile/generate";
import { useWorkItemStore } from "./workItemStore";
import { workItemTitle } from "./workItemDraft";
import { reconcileWorkMonitor } from "./workMonitor";
import { linkWorkSession } from "./sessionSummaryStore";
import { pendingPrerequisites } from "./workDependencies";

interface Confirmation { itemId: string; title: string; agentName: string; target: WorkSendTarget }
export const useWorkSendStore = create<{ busy: Record<string, string>; confirmation: Confirmation | null }>(() => ({ busy: {}, confirmation: null }));
const active = new Map<string, { itemId: string; promise: Promise<void> }>();
let decide: ((accepted: boolean) => void) | undefined;
export function decideWorkSend(accepted: boolean): void { const resolve = decide; decide = undefined; useWorkSendStore.setState({ confirmation: null }); resolve?.(accepted); }
export function hasPendingWorkSends(): boolean { return active.size > 0; }
/** 종료 시 확인 대기는 취소하고 이미 시작한 보내기는 결과/저장을 기다린다. */
export async function settleWorkSends(): Promise<void> {
  decideWorkSend(false);
  await Promise.all([...active.values()].map((job) => job.promise));
}
function snapshot(item: WorkItem, agent: AgentProfile): string {
  return JSON.stringify([item.title, item.body, item.assigneeAgentId, item.agent, agent.id, agent.cwd, agent.shell, agent.defaultAgent, agent.startupCommand, agent.clockedOut, agent.personalityPrompt]);
}
/** 자동 진행이 이 캐릭터를 진행자로 쓰는 중인가(docs/auto-run-design.md). */
export function autoRunRunner(agentId: string): WorkItem | undefined {
  return useWorkItemStore.getState().items.find((i) => i.autoRun?.state === "running" && i.assigneeAgentId === agentId);
}
function profileError(agent: AgentProfile | undefined, auto = false): string | undefined {
  if (!agent) return "work-send-agent-missing";
  if (!auto && autoRunRunner(agent.id)) return "work-send-auto-run";
  if (useAppStore.getState().sessions[agent.id]?.status === "starting") return "work-send-busy";
  if (agent.clockedOut) return "work-send-clocked-out";
  if (agent.startupCommand?.trim()) return "work-send-profile-unsupported";
  return undefined;
}
export function workSendDisabledReason(agent: AgentProfile | undefined): string | undefined { return profileError(agent); }
/** 자동 진행이 쓰는 보내기 옵션. 사람의 클릭에는 쓰지 않는다. */
export interface WorkSendOptions {
  /** 재시작이 필요하면 확인창 없이 재시작한다. 미제출 사람 입력이 있으면 재시작하지 않고 실패한다. 터미널을 열지 않는다. */
  auto?: boolean;
  /** 프롬프트 본문 뒤에 붙일 자동 진행 맥락. 업무에는 저장하지 않는다. */
  appendBody?: string;
}
/** 명시적 클릭만 입력을 발생시킨다. 확인/10초 대기 중 같은 캐릭터는 한 건만 보낸다. */
export function requestWorkSend(itemId: string, options: WorkSendOptions = {}): Promise<void> {
  const item = useWorkItemStore.getState().items.find((i) => i.id === itemId);
  if (!item) return Promise.resolve();
  const agent = item.assigneeAgentId ? useAppStore.getState().agents[item.assigneeAgentId] : undefined;
  const error = pendingPrerequisites(item, useWorkItemStore.getState().items).length ? "work-send-blocked" : profileError(agent, options.auto);
  if (error || !agent) { useWorkItemStore.getState().recordSend(itemId, { error }); return Promise.resolve(); }
  if (active.has(agent.id)) return active.get(agent.id)!.promise;
  const captured = snapshot(item, agent);
  const unchanged = () => {
    const current = useWorkItemStore.getState().items.find((i) => i.id === itemId);
    const profile = useAppStore.getState().agents[agent.id];
    if (!current || !profile || snapshot(current, profile) !== captured) throw new Error("work-send-session-changed");
  };
  const job = { itemId, promise: Promise.resolve() };
  active.set(agent.id, job);
  useWorkSendStore.setState((s) => ({ busy: { ...s.busy, [itemId]: agent.id } }));
  job.promise = (async () => {
    try {
      // native reads the saved profile. Flush older snapshots before inspection.
      await useWorkItemStore.getState().flush();
      if (useWorkItemStore.getState().dirty) throw new Error("work-send-save-failed");
      try { await flushPersistence(); } catch { throw new Error("work-send-save-failed"); }
      unchanged();
      let target = await tauriApi.inspectWorkSendTarget(agent.id);
      unchanged();
      let freshSession = false;
      if (target.restartRequired) {
        if (options.auto) {
          // 자동 진행은 확인 없이 새 세션으로 바꾸되, 사람이 쓰다 만 입력은 지우지 않는다.
          if (target.hasUncommitted) throw new Error("work-send-human-input");
        } else {
          // 다른 캐릭터의 확인창도 하나씩만 연다. 사용자가 다시 눌러 진행할 수 있다.
          if (decide) throw new Error("work-send-busy");
          const accepted = await new Promise<boolean>((resolve) => {
            decide = resolve;
            useWorkSendStore.setState({ confirmation: { itemId, title: workItemTitle(item), agentName: agent.name, target } });
          });
          if (!accepted) return;
        }
        unchanged();
        const confirmed = target;
        const current = await tauriApi.inspectWorkSendTarget(agent.id);
        unchanged();
        if (current.sessionId !== confirmed.sessionId || current.humanInputRevision !== confirmed.humanInputRevision) throw new Error("work-send-human-input");
        const { restartAgentSession } = await import("../agent/restartAgentSession");
        unchanged();
        const result = await restartAgentSession(agent.id, { cwd: useAppStore.getState().taskLabels[agent.id]?.cwd, expected: { sessionId: confirmed.sessionId!, humanInputRevision: confirmed.humanInputRevision } });
        if (!result || result.state !== "running") throw new Error("work-send-create-failed");
        target = await tauriApi.inspectWorkSendTarget(agent.id);
        if (target.sessionId !== result.sessionId) throw new Error("work-send-session-changed");
        freshSession = true;
      } else if (!target.sessionId) {
        const release = reserveSessionReplacement(agent.id);
        if (!release) throw new Error("work-send-busy");
        let result;
        try {
          useAppStore.getState().setSessionState({ agentId: agent.id, status: "starting" });
          result = await runGuardedCreateSession(agent.id);
        } finally { release(); }
        if (!result || result.state !== "running") throw new Error("work-send-create-failed");
        target = await tauriApi.inspectWorkSendTarget(agent.id);
        if (target.sessionId !== result.sessionId) throw new Error("work-send-session-changed");
        freshSession = true;
      }
      unchanged();
      if (!target.sessionId) throw new Error("work-send-session-changed");
      if (!options.auto) useAppStore.getState().openTerminal(agent.id);
      const body = options.appendBody ? (item.body.trim() ? `${item.body}\n\n${options.appendBody}` : options.appendBody) : item.body;
      const result = await tauriApi.sendWorkItem({ agentId: agent.id, sessionId: target.sessionId, humanInputRevision: target.humanInputRevision, freshSession, agent: normalizeWorkAgent(item.agent ?? agent.defaultAgent ?? { cli: "claude" }), title: item.title, body });
      if (result.sessionId !== target.sessionId) throw new Error("work-send-session-changed");
      useWorkItemStore.getState().recordSend(itemId, { sessionId: result.sessionId });
      linkWorkSession(itemId, agent.id, result.sessionId, result.submittedAt);
      reconcileWorkMonitor(result.sessionId, result.submittedAt);
      await useWorkItemStore.getState().flush();
    } catch (error) {
      useWorkItemStore.getState().recordSend(itemId, { error: error instanceof Error ? error.message : String(error) });
    } finally {
      if (useWorkSendStore.getState().confirmation?.itemId === itemId) decideWorkSend(false);
      active.delete(agent.id);
      useWorkSendStore.setState((s) => { const busy = { ...s.busy }; delete busy[itemId]; return { busy }; });
    }
  })();
  return job.promise;
}
