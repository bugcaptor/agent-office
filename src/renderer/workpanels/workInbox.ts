import type { AgentProfile } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import { useWorkItemStore } from "./workItemStore";

export function resolveAssignee(raw: string | undefined, profiles: Pick<AgentProfile, "id" | "name">[]): string | undefined {
  if (!raw) return;
  const idMatch = profiles.find((profile) => profile.id === raw);
  if (idMatch) return idMatch.id;
  const matches = profiles.filter((profile) => profile.name.trim() === raw.trim());
  return matches.length === 1 ? matches[0].id : undefined;
}

let timer: ReturnType<typeof setInterval> | undefined;
let inFlight: Promise<void> | undefined;
let generation = 0;
export function hasPendingWorkInbox(): boolean { return timer !== undefined || inFlight !== undefined; }
async function receive(epoch: number): Promise<void> {
  if (!useWorkItemStore.getState().loaded || useWorkItemStore.getState().error === "load") return;
  try {
    const entries = await tauriApi.scanWorkInbox();
    // 패널을 끄거나 종료한 뒤 도착한 scan 응답은 그대로 수신함에 둔다.
    if (epoch !== generation || !timer) return;
    // 지난 저장 실패가 남아 있으면 먼저 재시도해 오류를 풀고 같은 주기에 가져온다.
    if (useWorkItemStore.getState().error === "save") await useWorkItemStore.getState().flush();
    const profiles = Object.values(useAppStore.getState().agents);
    const accepted = new Set(useWorkItemStore.getState().importInbox(entries.map((entry) => ({
      id: `inbox-${entry.key}`, title: entry.title, body: entry.body,
      assigneeAgentId: resolveAssignee(entry.assignee, profiles),
    }))));
    await useWorkItemStore.getState().flush();
    const state = useWorkItemStore.getState();
    if (!state.loaded || state.dirty || state.error) return;
    const names = entries.filter((entry) => accepted.has(`inbox-${entry.key}`)).map((entry) => entry.fileName);
    if (names.length) await tauriApi.ackWorkInbox(names);
  } catch (error) { console.warn("work-inbox: receive failed", error); }
}
export function startWorkInbox(): void {
  if (timer) return;
  const epoch = ++generation;
  timer = setInterval(() => {
    if (inFlight) return;
    inFlight = receive(epoch).finally(() => { inFlight = undefined; });
  }, 3000);
}
/** 이미 import한 주기는 flush/ack까지 마친 뒤 반환한다. */
export async function stopWorkInbox(): Promise<void> {
  clearInterval(timer); timer = undefined; generation++;
  await inFlight;
}
