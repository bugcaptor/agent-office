import type { AgentProfile, InboxEntry, InboxRejection, InboxWarning } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import { useWorkItemStore, type InboxRequest } from "./workItemStore";

export function resolveAssignee(raw: string | undefined, profiles: Pick<AgentProfile, "id" | "name">[]): string | undefined {
  if (!raw) return;
  const idMatch = profiles.find((profile) => profile.id === raw);
  if (idMatch) return idMatch.id;
  const matches = profiles.filter((profile) => profile.name.trim() === raw.trim());
  return matches.length === 1 ? matches[0].id : undefined;
}

/** 담당 값이 있는데 배정하지 못한 이유. 미배정으로 받되 경고를 남긴다. */
export function assigneeIssue(raw: string | undefined, profiles: Pick<AgentProfile, "id" | "name">[]): "unknown-assignee" | "ambiguous-assignee" | undefined {
  if (!raw?.trim() || resolveAssignee(raw, profiles)) return;
  return profiles.some((profile) => profile.name.trim() === raw.trim()) ? "ambiguous-assignee" : "unknown-assignee";
}

const DROPPED_CODE = { self: "self-prerequisite", unknown: "unknown-prerequisite", cycle: "cyclic-prerequisite" } as const;

/** `office-task`가 정해 출력한 id를 그대로 쓴다. 그래야 `--after`로 이을 수 있다. */
export function inboxItemId(entry: Pick<InboxEntry, "id" | "key">): string { return entry.id ?? `inbox-${entry.key}`; }

/** 수신함 변경 요청 파일을 저장소 요청으로. 담당을 찾지 못한 수정은 담당을 그대로 둔다(경고). */
function toRequest(entry: InboxEntry, profiles: Pick<AgentProfile, "id" | "name">[]): InboxRequest {
  if (entry.done?.length) return { kind: "done", ids: entry.done };
  if (entry.delete?.length) return { kind: "delete", ids: entry.delete };
  const edit = entry.edit!;
  const assigneeAgentId = edit.assignee === undefined ? undefined : edit.assignee.trim() ? resolveAssignee(edit.assignee, profiles) : null;
  return { kind: "edit", id: edit.id, patch: {
    title: edit.title, body: edit.body, status: edit.status, assigneeAgentId,
    dependsOn: edit.after, addDependsOn: edit.addAfter, removeDependsOn: edit.removeAfter,
  } };
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
    // 완료·삭제·수정 요청은 같은 주기의 추가를 먼저 반영한 뒤 파일 이름(시각) 순으로 적용해
    // 방금 남긴 업무도 대상으로 삼을 수 있다.
    const isRequest = (entry: InboxEntry) => !!(entry.done?.length || entry.delete?.length || entry.edit);
    const requests = entries.filter(isRequest);
    const additions = entries.filter((entry) => !isRequest(entry));
    const imported = useWorkItemStore.getState().importInbox(additions.map((entry) => ({
      id: inboxItemId(entry), title: entry.title, body: entry.body,
      assigneeAgentId: resolveAssignee(entry.assignee, profiles), dependsOn: entry.after,
    })));
    const applied = requests.length ? useWorkItemStore.getState().applyInbox(requests.map((entry) => toRequest(entry, profiles))) : [];
    await useWorkItemStore.getState().flush();
    const state = useWorkItemStore.getState();
    if (!state.loaded || state.dirty || state.error) return;
    // 의미 오류는 버리지 않는다: 반영할 수 없는 파일은 rejected/로, 고쳐 받은 업무는 경고 기록으로.
    const ack: string[] = [];
    const rejections: InboxRejection[] = [];
    const warnings: InboxWarning[] = [];
    additions.forEach((entry, n) => {
      const result = imported[n];
      const taskId = inboxItemId(entry);
      if (!result || result.kind === "skipped") return;
      if (result.kind === "duplicate") { rejections.push({ fileName: entry.fileName, reason: "duplicate-id", detail: taskId }); return; }
      ack.push(entry.fileName);
      if (result.kind !== "added") return;
      const issue = assigneeIssue(entry.assignee, profiles);
      if (issue) warnings.push({ fileName: entry.fileName, taskId, code: issue, detail: entry.assignee!.trim() });
      for (const dropped of result.dropped) warnings.push({ fileName: entry.fileName, taskId, code: DROPPED_CODE[dropped.reason], detail: dropped.id });
    });
    requests.forEach((entry, n) => {
      const result = applied[n];
      if (!result) return;
      if (result.unknown.length) { rejections.push({ fileName: entry.fileName, reason: "unknown-task", detail: result.unknown.join(", ") }); return; }
      if (result.empty) { rejections.push({ fileName: entry.fileName, reason: "empty-task", detail: entry.edit!.id }); return; }
      ack.push(entry.fileName);
      const edit = entry.edit;
      if (!edit) return;
      const issue = assigneeIssue(edit.assignee, profiles);
      if (issue) warnings.push({ fileName: entry.fileName, taskId: edit.id, code: issue, detail: edit.assignee!.trim() });
      for (const dropped of result.dropped) warnings.push({ fileName: entry.fileName, taskId: edit.id, code: DROPPED_CODE[dropped.reason], detail: dropped.id });
    });
    // 경고 기록 실패로 업무를 다시 받지는 않는다(다시 받으면 replayed라 경고도 사라진다).
    if (warnings.length) await tauriApi.warnWorkInbox(warnings).catch((error) => console.warn("work-inbox: warning log failed", error));
    if (ack.length) await tauriApi.ackWorkInbox(ack);
    if (rejections.length) await tauriApi.rejectWorkInbox(rejections);
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
