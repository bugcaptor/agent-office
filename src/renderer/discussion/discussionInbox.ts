// src/renderer/discussion/discussionInbox.ts
//
// 에이전트·CLI(`office-task --discuss`)가 만든 토의와 `--role-add` 등이 남긴 역할 변경을 받는다. 업무 수신함 주기마다 부른다.
// 백엔드가 요청을 검사해 준비 상태로 저장하고 새로 저장한 것만 돌려주므로, 여기서는 스토어에 붙이기만 한다.
// 시작은 사람이 카드에서 한다(CLI 시작은 'CLI 자동 진행 허용'이 켜졌을 때 `ctl discuss start`).
import { tauriApi } from "../ipc/tauriApi";
import { useDiscussionStore } from "./discussionStore";
import { useRoleStore } from "./roleStore";

export async function receiveDiscussionInbox(): Promise<void> {
  const store = useDiscussionStore.getState();
  // 읽는 중에 가져오면 로드 결과가 붙인 것을 덮을 수 있다. 다 읽은 뒤에 가져온다. 손상 파일이면 저장도 거부된다.
  // 손상 파일이면 사람이 다시 읽기를 누를 때까지 두드리지 않는다(주기마다 읽으면 오류 알림이 깜빡인다).
  if (store.status === "error") return;
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

/** 에이전트·CLI(`office-task --role-add` 등)가 남긴 역할 변경을 받는다. 토의 수신함과 같은 주기다.
 *  백엔드는 `roles.json`이 있어야 적용하므로, 먼저 불러온다(파일이 없으면 이때 프리셋을 시드한다). */
export async function receiveRoleInbox(): Promise<void> {
  const store = useRoleStore.getState();
  if (store.status === "error" || store.editing) return;
  if (store.status !== "ready") {
    await store.ensureLoaded();
    if (useRoleStore.getState().status !== "ready") return;
  }
  try {
    // 불러오는 사이 창이 열렸으면 다음 주기로 미룬다.
    if (useRoleStore.getState().editing) return;
    const file = await tauriApi.scanRoleInbox();
    if (file) useRoleStore.setState({ roles: file.roles });
  } catch (error) {
    console.warn("role-inbox: receive failed", error);
  }
}
