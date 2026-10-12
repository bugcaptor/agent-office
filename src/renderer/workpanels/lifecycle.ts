import { useAppStore } from "../store/appStore";
import { startWorkInbox, stopWorkInbox, hasPendingWorkInbox } from "./workInbox";
import { useWorkItemStore, hasPendingWorkItems } from "./workItemStore";
import { flushSessionSummaries, hasPendingSessionSummaries } from "./sessionSummaryStore";
import { hasPendingWorkSends, settleWorkSends } from "./workSendStore";
export function hasPendingWorkPanelWrites(): boolean {
  return hasPendingWorkInbox() || hasPendingWorkSends() || hasPendingWorkItems() || hasPendingSessionSummaries();
}
/** 저장 실패 시 false: 미저장 업무를 남겨둔 채 창을 파괴하지 않는다. */
export async function flushWorkPanelWrites(): Promise<boolean> {
  // 루프는 즉시 멈추되 진행 중인 주기는 저장과 나란히 기다린다. flush 시작을 늦추면
  // 저장 중 취소 경합의 순서가 바뀐다.
  const inbox = stopWorkInbox();
  if (hasPendingWorkSends()) await settleWorkSends();
  await Promise.all([inbox, useWorkItemStore.getState().flush(), flushSessionSummaries()]);
  const saved = !hasPendingWorkSends() && !hasPendingWorkItems() && !hasPendingSessionSummaries();
  if (!saved && useAppStore.getState().appSettings.workPanelsEnabled) startWorkInbox();
  return saved;
}

/** 설정 변경·종료 확인창 취소에도 수신 루프의 수명을 맞춘다. 로컬 부팅(bootApp)에서만 설치한다. */
export function installWorkInboxLifecycle(): () => void {
  const refresh = () => {
    const state = useAppStore.getState();
    if (state.appSettings.workPanelsEnabled && state.modal.kind !== "confirm-quit") startWorkInbox();
    else void stopWorkInbox();
  };
  refresh();
  const off = useAppStore.subscribe((state, previous) => {
    if (state.appSettings.workPanelsEnabled !== previous.appSettings.workPanelsEnabled || state.modal.kind !== previous.modal.kind) refresh();
  });
  return () => { off(); void stopWorkInbox(); };
}
