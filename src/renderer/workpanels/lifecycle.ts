import { useWorkItemStore, hasPendingWorkItems } from "./workItemStore";
import { flushSessionSummaries, hasPendingSessionSummaries } from "./sessionSummaryStore";
import { hasPendingWorkSends, settleWorkSends } from "./workSendStore";
export function hasPendingWorkPanelWrites(): boolean {
  return hasPendingWorkSends() || hasPendingWorkItems() || hasPendingSessionSummaries();
}
/** 저장 실패 시 false: 미저장 업무를 남겨둔 채 창을 파괴하지 않는다. */
export async function flushWorkPanelWrites(): Promise<boolean> {
  if (hasPendingWorkSends()) await settleWorkSends();
  await Promise.all([useWorkItemStore.getState().flush(), flushSessionSummaries()]);
  return !hasPendingWorkPanelWrites();
}
