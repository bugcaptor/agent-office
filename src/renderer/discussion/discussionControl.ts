// src/renderer/discussion/discussionControl.ts
//
// CLI(`agent-office ctl discuss start|answer`)의 토의 쓰기 요청을 받는다. 백엔드 control 서버가
// `discussion-control` 이벤트로 넘기고, 여기서 카드 버튼과 같은 함수를 부른 뒤 결과를 돌려준다.
// 허용 설정(cliAutoRunEnabled)과 수신함에서 가져왔는지는 백엔드가 이미 확인했다. 결론 확인은 사람의 일이라 받지 않는다.
import { listen } from "@tauri-apps/api/event";
import { Events } from "@shared/ipc";
import { tauriApi } from "../ipc/tauriApi";
import { useDiscussionStore } from "./discussionStore";
import { answerDiscussion, startDiscussion } from "./discussionRun";

export interface DiscussionControlRequest {
  requestId: string;
  op: "start" | "answer";
  discussionId: string;
  options?: { text?: string | null };
}

/** 백엔드가 막 저장한 토의가 스토어에 붙을 때까지(수신함 주기의 응답) 잠깐 기다린다. */
const ADOPT_WAIT_MS = 2000;

async function findDiscussion(id: string, waitMs: number): Promise<boolean> {
  const has = () => useDiscussionStore.getState().discussions.some((d) => d.id === id);
  const deadline = Date.now() + waitMs;
  while (!has() && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 100));
  return has();
}

/** 요청 하나를 처리한다. 거절이면 이유 코드를 돌려준다. */
export async function handleDiscussionControl(request: DiscussionControlRequest, waitMs = ADOPT_WAIT_MS): Promise<string | undefined> {
  if (!(await findDiscussion(request.discussionId, waitMs))) return "discussion-not-found";
  switch (request.op) {
    case "start":
      return startDiscussion(request.discussionId);
    case "answer": {
      const text = request.options?.text?.trim() ?? "";
      if (!text) return "discussion-answer-empty";
      return answerDiscussion(request.discussionId, text) ? undefined : "discussion-not-waiting";
    }
    default:
      return "discussion-unknown-op";
  }
}

export function installDiscussionControl(): () => void {
  let unlisten: (() => void) | null = null;
  let disposed = false;
  void listen<DiscussionControlRequest>(Events.discussionControl, (event) => {
    const request = event.payload;
    if (!request || typeof request.requestId !== "string") return;
    void handleDiscussionControl(request)
      .catch((err: unknown) => (err instanceof Error ? err.message : String(err)))
      .then((error) => tauriApi.discussionControlReply(request.requestId, error))
      .catch((err: unknown) => console.warn("discussion control: reply failed", err));
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
