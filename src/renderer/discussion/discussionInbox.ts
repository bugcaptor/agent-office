// src/renderer/discussion/discussionInbox.ts
//
// 에이전트·CLI(`office-task --discuss`)가 만든 토의를 받는다. 업무 수신함 주기마다 부른다.
// 백엔드가 요청을 검사해 준비 상태로 저장하고 새로 저장한 것만 돌려주므로, 여기서는 스토어에 붙이기만 한다.
// 시작은 사람이 카드에서 한다(CLI 시작은 'CLI 자동 진행 허용'이 켜졌을 때 `ctl discuss start`).
import { tauriApi } from "../ipc/tauriApi";
import { useDiscussionStore } from "./discussionStore";

export async function receiveDiscussionInbox(): Promise<void> {
  const store = useDiscussionStore.getState();
  // 읽는 중에 가져오면 로드 결과가 붙인 것을 덮을 수 있다. 다 읽은 뒤에 가져온다. 손상 파일이면 저장도 거부된다.
  if (store.status !== "ready") {
    await store.ensureLoaded();
    if (useDiscussionStore.getState().status !== "ready") return;
  }
  try {
    const imported = await tauriApi.scanDiscussionInbox();
    if (imported.length) useDiscussionStore.getState().adopt(imported);
  } catch (error) {
    console.warn("discussion-inbox: receive failed", error);
  }
}
