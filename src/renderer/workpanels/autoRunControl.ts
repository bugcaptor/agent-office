// CLI(`agent-office ctl autorun …`)의 자동 진행 쓰기 요청을 받는다. 백엔드 control 서버가
// `autorun-control` 이벤트로 넘기고, 여기서 GUI 버튼과 같은 함수를 부른 뒤 결과를 돌려준다.
// 허용 설정(cliAutoRunEnabled)은 백엔드가 이미 확인했다.
import { listen } from "@tauri-apps/api/event";
import { Events } from "@shared/ipc";
import { tauriApi } from "../ipc/tauriApi";
import { useWorkItemStore } from "./workItemStore";
import { endAutoRun, escalateAutoRun, pauseAutoRun, resumeAutoRunFromCtl, startAutoRun } from "./autoRun";

export interface AutoRunControlRequest {
  requestId: string;
  op: "start" | "resume" | "pause" | "end" | "escalate";
  rootId: string;
  options?: { message?: string | null; usageGuard?: boolean | null };
}

/** 요청 하나를 처리한다. 거절이면 이유 코드를 돌려준다. */
export function handleAutoRunControl(request: AutoRunControlRequest): string | undefined {
  const root = useWorkItemStore.getState().items.find((item) => item.id === request.rootId);
  if (!root) return "auto-run-no-plan";
  const run = root.autoRun;
  switch (request.op) {
    case "start":
      // 끝난 실행은 GUI처럼 기록을 지우고 새로 시작한다.
      if (run && run.state !== "finished") return "auto-run-already-running";
      if (run) endAutoRun(root.id);
      return startAutoRun(root.id, { usageGuard: request.options?.usageGuard ?? true, viaCtl: true });
    case "resume":
      return resumeAutoRunFromCtl(root.id);
    case "pause":
      if (!run || run.state === "finished") return "auto-run-no-run";
      pauseAutoRun(root.id, { viaCtl: true });
      return undefined;
    case "end":
      if (!run) return "auto-run-no-run";
      endAutoRun(root.id);
      return undefined;
    case "escalate":
      return escalateAutoRun(root.id, request.options?.message?.trim() ?? "");
    default:
      return "auto-run-unknown-op";
  }
}

export function installAutoRunControl(): () => void {
  let unlisten: (() => void) | null = null;
  let disposed = false;
  void listen<AutoRunControlRequest>(Events.autoRunControl, (event) => {
    const request = event.payload;
    if (!request || typeof request.requestId !== "string") return;
    let error: string | undefined;
    try {
      error = handleAutoRunControl(request);
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    }
    void tauriApi.autoRunControlReply(request.requestId, error).catch((err: unknown) => console.warn("auto-run control: reply failed", err));
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
