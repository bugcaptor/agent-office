// src/renderer/webdash/webDashControl.ts
//
// 웹 상태판(브라우저)의 쓰기 요청을 받는다. 백엔드가 `web-dash-control` 이벤트로 넘기고, 여기서
// 앱 메뉴·버튼과 같은 함수를 부른 뒤 결과를 돌려준다. 백엔드 다리는 5초만 기다리므로 오래 걸리는 일
// (재시작·이어하기·보내기)은 시작만 확인하고 바로 답한다 — 결과는 웹이 상태 갱신으로 본다.
import { listen } from "@tauri-apps/api/event";
import { Events } from "@shared/ipc";
import type { WebDashControlRequest } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import { useWorkItemStore } from "../workpanels/workItemStore";
import { requestWorkSend, workSendDisabledReason } from "../workpanels/workSendStore";
import { endAutoRun, startAutoRun } from "../workpanels/autoRun";
import { handleDiscussionControl } from "../discussion/discussionControl";

function fireAndForget(label: string, work: Promise<unknown>): void {
  work.catch((err: unknown) => console.warn(`web dash: ${label} failed`, err));
}

/** 요청 하나를 처리한다. 거절이면 이유 코드를, 성공이면 돌려줄 값을 준다. */
export async function handleWebDashControl(request: WebDashControlRequest): Promise<{ error?: string; data?: unknown }> {
  const app = useAppStore.getState();
  switch (request.op) {
    case "restart":
    case "terminate":
    case "resume": {
      const agentId = request.agentId ?? "";
      if (!app.agents[agentId]) return { error: "unknown-agent" };
      if (request.op === "restart") {
        const { restartAgentSession } = await import("../agent/restartAgentSession");
        fireAndForget("restart", restartAgentSession(agentId));
      } else if (request.op === "terminate") {
        const status = app.sessions[agentId]?.status ?? "idle";
        if (!["starting", "running"].includes(status)) return { error: "not-running" };
        const { terminateAgentSession } = await import("../agent/terminateSession");
        fireAndForget("terminate", terminateAgentSession(agentId));
      } else {
        const entries = await tauriApi.listClaudeResumeSessions();
        const entry = entries[agentId];
        if (!entry) return { error: "no-resume-session" };
        const { resumeAgentSession } = await import("../agent/resumeAgentSession");
        fireAndForget("resume", resumeAgentSession(agentId, entry.sessionId));
      }
      return {};
    }
    case "work-create": {
      const work = useWorkItemStore.getState();
      if (!work.loaded) return { error: "work-not-loaded" };
      const assignee = request.assigneeAgentId ?? undefined;
      if (assignee && !app.agents[assignee]) return { error: "unknown-agent" };
      // `add`는 새 업무를 골라 앱에 업무 팝업을 띄운다. 웹에서 만든 업무는 앱 화면을 바꾸지 않게 고르던 것을 되돌린다.
      const selected = work.selectedId;
      const id = work.add({
        title: request.title ?? "",
        body: request.body ?? "",
        status: "todo",
        assigneeAgentId: assignee,
        agent: request.cli ? { cli: request.cli } : undefined,
      });
      if (!id) return { error: "empty" };
      if (useWorkItemStore.getState().selectedId === id) useWorkItemStore.getState().select(selected);
      await useWorkItemStore.getState().flush();
      if (request.send && assignee) {
        const blocked = workSendDisabledReason(app.agents[assignee]);
        if (blocked) return { error: blocked, data: { itemId: id } };
        fireAndForget("work send", requestWorkSend(id, { restartConfirmed: request.restartConfirmed === true }));
      }
      return { data: { itemId: id } };
    }
    case "work-send": {
      const item = useWorkItemStore.getState().items.find((i) => i.id === request.itemId);
      if (!item) return { error: "task-not-found" };
      if (!item.assigneeAgentId) return { error: "work-send-agent-missing" };
      const blocked = workSendDisabledReason(app.agents[item.assigneeAgentId]);
      if (blocked) return { error: blocked };
      fireAndForget("work send", requestWorkSend(item.id));
      return {};
    }
    case "work-done": {
      const work = useWorkItemStore.getState();
      const item = work.items.find((i) => i.id === request.itemId);
      if (!item) return { error: "task-not-found" };
      if (item.status !== "done") {
        work.edit(item.id, { status: "done" });
        await useWorkItemStore.getState().flush();
      }
      // 앱에 떠 있는 것도 내린다: 이 업무 팝업, 담당 캐릭터의 알림 카드·배지.
      if (useWorkItemStore.getState().selectedId === item.id) useWorkItemStore.getState().select(null);
      const agentId = item.assigneeAgentId;
      if (agentId) {
        useAppStore.getState().clearNotificationsFor(agentId);
        tauriApi.clearNotifications(agentId);
      }
      return {};
    }
    case "autorun-start": {
      const root = useWorkItemStore.getState().items.find((i) => i.id === request.itemId);
      if (!root) return { error: "auto-run-no-plan" };
      if (root.autoRun && root.autoRun.state !== "finished") return { error: "auto-run-already-running" };
      // 끝난 실행은 앱 버튼처럼 기록을 지우고 새로 시작한다.
      if (root.autoRun) endAutoRun(root.id);
      const error = startAutoRun(root.id, { usageGuard: true });
      return error ? { error } : {};
    }
    case "discuss-start":
    case "discuss-answer": {
      const error = await handleDiscussionControl({
        requestId: request.requestId,
        op: request.op === "discuss-start" ? "start" : "answer",
        discussionId: request.discussionId ?? "",
        options: { text: request.text ?? null },
      });
      return error ? { error } : {};
    }
    case "office-agent-open": {
      // 일반 에이전트를 연다. 기동·첫 프롬프트까지 5초를 넘기므로 막히는 이유만 보고 바로 답한다.
      // 실패는 네이티브가 기억해 웹 상태에 실린다(`office_agent::last_error`).
      const prompt = request.prompt ?? "";
      if (!prompt.trim()) return { error: "empty" };
      const restart = request.restartConfirmed === true;
      // 터미널(xterm)을 끌어오므로 쓸 때만 읽는다.
      const { officeAgentBlockedReason, openOfficeAgent } = await import("../officeAgent/officeAgentStore");
      const blocked = officeAgentBlockedReason(restart);
      if (blocked) return { error: blocked };
      fireAndForget("office agent open", openOfficeAgent({ prompt, cli: request.cli ?? "claude", restart, ...(request.cwd ? { cwd: request.cwd } : {}) })
        .then((error) => { if (error) console.warn("web dash: office agent open failed", error); }));
      return {};
    }
    case "office-agent-close": {
      const { closeOfficeAgent } = await import("../officeAgent/officeAgentStore");
      fireAndForget("office agent close", closeOfficeAgent());
      return {};
    }
    default:
      return { error: "unknown-op" };
  }
}

export function installWebDashControl(): () => void {
  let unlisten: (() => void) | null = null;
  let disposed = false;
  void listen<WebDashControlRequest>(Events.webDashControl, (event) => {
    const request = event.payload;
    if (!request || typeof request.requestId !== "string") return;
    void handleWebDashControl(request)
      .catch((err: unknown) => ({ error: err instanceof Error ? err.message : String(err), data: undefined }))
      .then(({ error, data }) => tauriApi.webDashControlReply(request.requestId, error, data))
      .catch((err: unknown) => console.warn("web dash control: reply failed", err));
  }).then((off) => {
    if (disposed) off();
    else unlisten = off;
  });
  return () => {
    if (disposed) return;
    disposed = true;
    unlisten?.();
    unlisten = null;
  };
}
