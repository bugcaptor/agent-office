// src/renderer/link/receivedWork.ts
//
// 머신 연동 클라이언트: 호스트에서 받은 업무. 호스트가 `work-send`로 보내면 네이티브(`link/work.rs`)가 저장소로
// 작업 폴더를 정하고 `link-work-receive`로 넘긴다. 여기서 같은 id의 받은 업무(`received`)로 로컬 보드에 넣고
// 그 폴더의 새 세션으로 보낸다. 그 뒤 진행 상태(상태·최신 세션·단계)가 바뀔 때마다 호스트로 올린다 —
// 판단·후속 지시는 호스트가 한다. 끊겨 있으면 네이티브가 쌓아 두었다가 다시 붙으면 최신 하나를 올린다.
import type { LinkTaskReport, WebDashControlRequest, WorkItem } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import { useWorkItemStore } from "../workpanels/workItemStore";
import { useWorkMonitorStore } from "../workpanels/workMonitor";
import { requestWorkSend } from "../workpanels/workSendStore";

/** 업무별로 마지막에 올린 상태(같으면 다시 올리지 않는다)와 그 세션을 연 보내기 번호. */
const sent = new Map<string, string>();
const outcomeSeq = new Map<string, number>();

function lastSession(item: WorkItem): string | undefined { return item.sessionIds[item.sessionIds.length - 1]; }

/** 받은 업무 하나의 지금 진행 상태. */
export function taskReport(item: WorkItem): LinkTaskReport {
  const sessionId = lastSession(item);
  const observed = sessionId ? useWorkMonitorStore.getState().sessions[sessionId] : undefined;
  const seq = outcomeSeq.get(item.id);
  return {
    taskId: item.id, status: item.status,
    ...(sessionId ? { sessionId } : {}),
    ...(observed ? { phase: observed.phase, transition: observed.transition } : {}),
    ...(item.lastError ? { error: item.lastError } : {}),
    ...(seq !== undefined ? { sendSeq: seq } : {}),
  };
}

function report(item: WorkItem, force = false): void {
  const next = taskReport(item);
  const key = JSON.stringify(next);
  if (!force && sent.get(item.id) === key) return;
  sent.set(item.id, key);
  void tauriApi.linkTaskReport(next).catch((error: unknown) => {
    // 클라이언트가 아니게 된 머신의 남은 받은 업무는 올릴 곳이 없다.
    if (String(error) !== "link-not-client") console.warn("link: task report failed", error);
  });
}

/** 호스트가 보낸 업무를 받는다. 넣고 보내기를 시작한 뒤 바로 답한다(결과는 진행 상태로 올라간다). */
export async function receiveHostWork(request: WebDashControlRequest): Promise<{ error?: string }> {
  const work = useWorkItemStore.getState();
  if (!work.loaded) await work.load();
  const id = request.itemId ?? "";
  const agentId = request.agentId ?? "";
  if (!id || !request.cwd) return { error: "bad-request" };
  if (!useAppStore.getState().agents[agentId]) return { error: "work-send-agent-missing" };
  const ok = useWorkItemStore.getState().upsertReceived({
    id, title: request.title ?? "", body: request.body ?? "", assigneeAgentId: agentId,
    agent: request.agent ?? undefined,
    received: { hostName: request.hostName ?? "", cwd: request.cwd, receivedAt: Date.now() },
  });
  if (!ok) return { error: "work-not-loaded" };
  await useWorkItemStore.getState().flush();
  const auto = request.auto === true;
  const seq = typeof request.sendSeq === "number" ? request.sendSeq : undefined;
  void requestWorkSend(id, { auto, overHumanInput: auto, restartConfirmed: !auto, cwd: request.cwd })
    .catch((error: unknown) => console.warn("link: received work send failed", error))
    .finally(() => {
      const item = useWorkItemStore.getState().items.find((i) => i.id === id);
      if (!item) return;
      // 이번 보내기의 결과(열린 세션이나 실패)를 그 번호와 함께 올린다. 그 뒤의 진행 상태도 이 번호를 단다.
      if (seq !== undefined) outcomeSeq.set(id, seq);
      report(item, true);
    });
  return {};
}

/** 받은 업무가 바뀔 때마다(상태·세션·단계) 호스트에 올린다. 호스트에 붙은 클라이언트일 때만 뜻이 있다. */
export function installReceivedWork(): () => void {
  const scan = () => {
    for (const item of useWorkItemStore.getState().items) if (item.received) report(item);
  };
  const offItems = useWorkItemStore.subscribe((state, previous) => { if (state.items !== previous.items) scan(); });
  const offMonitor = useWorkMonitorStore.subscribe(scan);
  return () => { offItems(); offMonitor(); };
}
