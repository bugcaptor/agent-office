import { create } from "zustand";
import { notificationType } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { useWorkItemStore } from "./workItemStore";
export type WorkPhase = "working" | "waiting" | "review" | "ended";
interface Observation { phase: WorkPhase; at: number; agentId: string; transition: boolean }
export const useWorkMonitorStore = create<{ sessions: Record<string, Observation> }>(() => ({ sessions: {} }));
function applyStatus(sessionId: string, phase: WorkPhase) {
  // 최신 연결 세션만 따라간다. 사람이 todo/done으로 바꾼 업무는 자동으로 덮지 않는다.
  for (const item of useWorkItemStore.getState().items) {
    if (item.sessionIds[item.sessionIds.length - 1] !== sessionId) continue;
    if (phase === "review" && item.status === "in_progress") useWorkItemStore.getState().edit(item.id, { status: "review" });
    if (phase === "working" && item.status === "review") useWorkItemStore.getState().edit(item.id, { status: "in_progress" });
  }
}
/** CR 직후 훅이 IPC 응답보다 먼저 도착해도 연결 확정 후 반영한다. */
export function reconcileWorkMonitor(sessionId: string, submittedAt: number): void {
  const event = useWorkMonitorStore.getState().sessions[sessionId];
  if (event?.transition && event.at >= submittedAt) applyStatus(sessionId, event.phase);
}
export function installWorkMonitor(): () => void {
  function observe(sessionId: string, agentId: string, phase: WorkPhase, at: number, transition: boolean) {
    const old = useWorkMonitorStore.getState().sessions[sessionId];
    if (old && old.at > at) return;
    // 턴 종료 뒤의 질문 신호는 Claude idle_prompt(입력 대기 리마인더)다. 다음 prompt 전까지 턴 종료를 유지한다.
    if (phase === "waiting" && (old?.phase === "review" || old?.phase === "ended")) return;
    useWorkMonitorStore.setState((s) => ({ sessions: { ...s.sessions, [sessionId]: { phase, at, agentId, transition } } }));
    if (transition) applyStatus(sessionId, phase);
  }
  const offActivity = tauriApi.onActivity((e) => {
    if (e.kind === "prompt") observe(e.sessionId, e.agentId, "working", e.at, true);
    else if (e.kind === "tool" || e.kind === "resume") observe(e.sessionId, e.agentId, "working", e.at, false);
  });
  const offNotification = tauriApi.onNotification((e) => {
    const type = notificationType(e.source);
    if (type === "question") observe(e.sessionId, e.agentId, "waiting", e.at, false);
    else if (type === "done") observe(e.sessionId, e.agentId, "review", e.at, true);
  });
  const offUsage = tauriApi.onTurnUsage((e) => {
    if (!e.partial) observe(e.sessionId, e.agentId, "review", e.at, true);
  });
  const offSession = tauriApi.onSessionState((e) => {
    if (e.state === "exited" || e.state === "disposed") observe(e.sessionId, e.agentId, "ended", e.at, false);
  });
  return () => { offActivity(); offNotification(); offUsage(); offSession(); };
}
