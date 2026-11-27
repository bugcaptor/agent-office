// src/renderer/link/remoteWork.ts
//
// 머신 연동 호스트: 다른 머신 캐릭터(`기기id:캐릭터id`)에게 맡긴 업무. 호스트 보드가 정본이고 자동 진행도
// 여기(호스트 렌더러)가 돌린다. 그 머신에는 지시만 넘긴다 — 보내기(`work-send`, 그 머신이 저장소로 작업 폴더를
// 정한다)·후속 지시·마지막 응답·사용량. 그 머신이 받은 업무의 진행 상태는 `link-task` 이벤트로 올라오고,
// 원격 세션(`기기id:세션id`)의 신호로 업무 모니터에 넣어 이 머신 세션과 같은 상태 전이를 탄다.
import type { LinkTaskEvent, RemoteCharacter, RemoteMachine, UsageSnapshot, WorkItem, WorkRepo } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { useWorkItemStore } from "../workpanels/workItemStore";
import { observeWorkSession } from "../workpanels/workMonitor";
import { findRemote, useRemoteFloors } from "./remoteFloors";

/** 보내기 결과(세션이 열렸거나 실패)를 그 머신에서 기다리는 시간. 저장소 준비·CLI 기동까지 든다. */
export const REMOTE_SEND_TIMEOUT_MS = 150_000;

/** 원격 캐릭터 id면 그 머신과 캐릭터. 이 머신 캐릭터이거나 모르는 id면 null. */
export function remoteAssignee(id: string | undefined): { machine: RemoteMachine; character: RemoteCharacter } | null {
  if (!id || !id.includes(":")) return null;
  return findRemote(useRemoteFloors.getState().office, id);
}
export function isRemoteAgentId(id: string | undefined): id is string {
  return !!remoteAssignee(id);
}
/** 화면에 보일 원격 담당 이름(`이름 · 머신`). */
export function remoteName(id: string | undefined): string | undefined {
  const found = remoteAssignee(id);
  return found ? `${found.character.name} · ${found.machine.name}` : undefined;
}
export function remoteSessionId(deviceId: string, sessionId: string): string { return `${deviceId}:${sessionId}`; }
/** 호스트 보드에 적힌 원격 세션 id를 그 머신의 세션 id로. */
function machineSessionId(id: string): string { return id.slice(id.indexOf(":") + 1); }

/**
 * git 원격 주소 정규형(`호스트/경로`). Rust `link::repos::normalize_remote`와 같은 규칙이다 — 그 머신이 알린
 * 클론 목록과 비교한다.
 */
export function normalizeRepoUrl(raw: string): string | null {
  const s = raw.trim();
  // eslint-disable-next-line no-control-regex
  if (!s || /[\u0000-\u001f\u007f]/u.test(s)) return null;
  const trimPath = (p: string) => p.replace(/^\/+|\/+$/gu, "").replace(/\.git$/u, "").replace(/\/+$/u, "");
  if (s.startsWith("file://")) { const p = trimPath(s.slice(7)); return p ? `/${p}` : null; }
  if (/^[/.~]/u.test(s) || /^[A-Za-z]:/u.test(s)) { const p = trimPath(s); return p ? `${s.startsWith("/") ? "/" : ""}${p}` : null; }
  let host: string, path: string;
  const scheme = s.indexOf("://");
  if (scheme >= 0) {
    const rest = s.slice(scheme + 3);
    const slash = rest.indexOf("/");
    const authority = slash >= 0 ? rest.slice(0, slash) : rest;
    const tail = slash >= 0 ? rest.slice(slash + 1) : "";
    let h = authority.slice(authority.lastIndexOf("@") + 1);
    let extra = "";
    const colon = h.indexOf(":");
    if (colon >= 0) {
      const port = h.slice(colon + 1);
      if (!/^\d+$/u.test(port)) extra = `${port}/`;
      h = h.slice(0, colon);
    }
    host = h; path = `${extra}${tail}`;
  } else {
    const colon = s.indexOf(":");
    if (colon >= 0) {
      // scp 모양: [user@]host:path
      const left = s.slice(0, colon);
      host = left.slice(left.lastIndexOf("@") + 1); path = s.slice(colon + 1);
    } else {
      // 이미 정규형(`github.com/acme/app`)인 것.
      const slash = s.indexOf("/");
      if (slash < 0 || !s.slice(0, slash).includes(".")) return null;
      host = s.slice(0, slash); path = s.slice(slash + 1);
    }
  }
  host = host.trim().toLowerCase();
  path = trimPath(path);
  if (!host || !path || host.includes("/")) return null;
  return `${host}/${path}`;
}

/** 그 머신에 맡길 수 없는 이유 코드. 없으면 undefined. 맡기기 전(편집·보내기·자동 진행)에 본다. */
export function remoteSendBlocked(item: Pick<WorkItem, "assigneeAgentId" | "repo">): string | undefined {
  const found = remoteAssignee(item.assigneeAgentId);
  if (!found) return "work-send-agent-missing";
  if (!found.machine.online) return "link-device-offline";
  if (found.character.clockedOut) return "work-send-clocked-out";
  return repoIssue(item.repo, found.machine);
}
/** 저장소가 그 머신에 없거나 주소를 읽을 수 없으면 그 이유. */
export function repoIssue(repo: WorkRepo | undefined, machine: RemoteMachine): string | undefined {
  if (!repo?.url.trim()) return undefined;
  const url = normalizeRepoUrl(repo.url);
  if (!url) return "link-repo-invalid";
  return machine.repos.includes(url) ? undefined : "link-repo-missing";
}

interface Outcome { sessionId?: string; error?: string; event?: LinkTaskEvent }
const waiters = new Map<string, { seq: number; resolve(outcome: Outcome): void }>();

function find(id: string): WorkItem | undefined { return useWorkItemStore.getState().items.find((i) => i.id === id); }
function errorCode(error: unknown): string { return error instanceof Error ? error.message : String(error); }

/**
 * 원격 캐릭터에게 업무를 보낸다. 그 머신이 받아 보내기를 시작하면 결과(열린 세션 또는 실패)가 진행 상태로
 * 올라올 때까지 기다린다. 결과는 이 머신 보내기와 같이 업무에 남긴다(`recordSend`).
 */
export async function sendRemoteWork(item: WorkItem, options: { auto?: boolean; appendBody?: string } = {}): Promise<void> {
  const store = useWorkItemStore.getState();
  const blocked = remoteSendBlocked(item);
  if (blocked) { store.recordSend(item.id, { error: blocked }); return; }
  if (waiters.has(item.id)) return;
  const agentId = item.assigneeAgentId!;
  const deviceId = agentId.slice(0, agentId.indexOf(":"));
  const seq = Date.now();
  const body = options.appendBody ? (item.body.trim() ? `${item.body}\n\n${options.appendBody}` : options.appendBody) : item.body;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const outcome = new Promise<Outcome>((resolve) => {
    waiters.set(item.id, { seq, resolve });
    timer = setTimeout(() => resolve({ error: "link-send-timeout" }), REMOTE_SEND_TIMEOUT_MS);
  });
  try {
    await useWorkItemStore.getState().flush();
    await tauriApi.linkWork(agentId, "work-send", {
      taskId: item.id, title: item.title, body, sendSeq: seq, auto: !!options.auto,
      ...(item.agent ? { agent: item.agent } : {}), ...(item.repo ? { repo: item.repo } : {}),
    });
  } catch (error) {
    clearTimeout(timer); waiters.delete(item.id);
    useWorkItemStore.getState().recordSend(item.id, { error: errorCode(error) });
    return;
  }
  const result = await outcome;
  clearTimeout(timer); waiters.delete(item.id);
  if (!result.sessionId) { useWorkItemStore.getState().recordSend(item.id, { error: result.error ?? "link-send-timeout" }); return; }
  const sessionId = remoteSessionId(deviceId, result.sessionId);
  useWorkItemStore.getState().recordSend(item.id, { sessionId });
  const e = result.event;
  observeWorkSession(sessionId, agentId, e?.phase ?? "working", e?.receivedAt ?? Date.now(), e?.transition ?? false);
  await useWorkItemStore.getState().flush();
}

/** 붙은 머신이 올린 받은 업무의 진행 상태. 그 머신에 맡긴 업무에만 반영한다. */
export function applyLinkTask(e: LinkTaskEvent): void {
  const item = find(e.taskId);
  if (!item?.assigneeAgentId?.startsWith(`${e.deviceId}:`)) return;
  const waiter = waiters.get(e.taskId);
  if (waiter && e.sendSeq === waiter.seq && (e.sessionId || e.error)) {
    waiter.resolve({ sessionId: e.error ? undefined : e.sessionId, error: e.error, event: e });
    return;
  }
  if (!e.sessionId) return;
  const sessionId = remoteSessionId(e.deviceId, e.sessionId);
  if (!item.sessionIds.includes(sessionId)) {
    // 기다림이 끝난 뒤(시간 초과) 늦게 열린 세션이면 받아들인다. 그 밖의 모르는 세션은 버린다.
    if (waiter || item.lastError !== "link-send-timeout") return;
    useWorkItemStore.getState().recordSend(item.id, { sessionId });
  }
  // 최신 세션의 신호만 상태 전이에 쓴다(옛 세션의 늦은 신호는 모니터에만 남는다).
  if (e.phase) observeWorkSession(sessionId, item.assigneeAgentId, e.phase, e.receivedAt, !!e.transition);
}

/**
 * 원격 세션에 후속 지시를 넣는다. 보낸 시각은 이 머신 시계로, 묻기 시작한 때를 돌려준다 — 원격 신호의 시각도
 * 이 머신이 받은 때라, 새 턴의 신호가 답보다 먼저 와도 그 턴 것으로 친다.
 */
export async function remoteFollowUp(agentId: string, sessionId: string, text: string): Promise<{ submittedAt: number }> {
  const submittedAt = Date.now();
  await tauriApi.linkWork(agentId, "work-followup", { sessionId: machineSessionId(sessionId), text });
  return { submittedAt };
}
/** 원격 작업자의 마지막 응답. 읽지 못하면 null. */
export async function remoteLastReply(agentId: string, cli: string, taskId: string): Promise<string | null> {
  const answer = await tauriApi.linkWork(agentId, "work-reply", { cli, taskId }) as { reply?: string | null } | null;
  return answer?.reply ?? null;
}
/** 그 머신의 구독 사용량. 묻지 못하면 null. */
export async function remoteUsage(agentId: string): Promise<UsageSnapshot | null> {
  try { return await tauriApi.linkWork(agentId, "usage", {}) as UsageSnapshot; } catch { return null; }
}

/** 진행 상태를 받는다. 이 렌더러가 뜨기 전에 온 것(앱 시작·새로 고침)은 네이티브가 들고 있던 것으로 채운다. */
export function installRemoteWork(): () => void {
  const off = tauriApi.onLinkTask(applyLinkTask);
  let disposed = false;
  void (async () => {
    // 업무를 불러온 뒤에야 맞춰 볼 수 있다.
    await useWorkItemStore.getState().load();
    const states = await tauriApi.linkTaskStates().catch(() => []);
    if (!disposed) for (const e of states) applyLinkTask(e);
  })();
  return () => { disposed = true; off(); };
}
