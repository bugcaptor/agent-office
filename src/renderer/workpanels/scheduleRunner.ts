// 예약 실행기. 메인 창에서만 돌며(bootstrap) 주기마다 때가 된 예약을 실행한다.
// 한 번 예약은 그 업무 자체를, 반복 예약은 템플릿을 복제해 실행한다. 선행 작업이 있으면
// 자동 진행으로, 없으면 보내기로 실행한다. 놓치거나 실행할 수 없는 회차는 건너뛰고 알린다.
// 다음 실행 시각은 보내기 전에 먼저 저장해 앱이 죽었다 켜져도 같은 회차를 두 번 실행하지 않는다.
import type { WorkItem, WorkSchedule, WorkScheduleSkipReason } from "@shared/types";
import { t } from "../i18n";
import { useAppStore } from "../store/appStore";
import { maybeSendOsNotification } from "../ipc/osNotify";
import { tauriApi } from "../ipc/tauriApi";
import { useWorkItemStore } from "./workItemStore";
import { useWorkMonitorStore } from "./workMonitor";
import { requestWorkSend, useWorkSendStore, workSendDisabledReason } from "./workSendStore";
import { pendingPrerequisites } from "./workDependencies";
import { workItemTitle } from "./workItemDraft";
import { planItems, startAutoRun } from "./autoRun";
import { codeText } from "./sendErrorText";
import { hasOpenRun, isRecurring, nextOccurrence } from "./workSchedule";
import { sameWorkValue } from "./workItemSync";

/** 예정 시각을 이만큼 넘겨서 알아챘으면(앱 꺼짐·잠자기) 놓친 회차로 보고 건너뛴다. */
export const MISSED_GRACE_MS = 5 * 60_000;
const CHECK_MS = 15_000;

const firing = new Set<string>();
const find = (id: string) => useWorkItemStore.getState().items.find((item) => item.id === id);

type Skip = { reason: WorkScheduleSkipReason; detail?: string };

/**
 * 담당 캐릭터의 세션에 보내도 되는가. CLI가 떠 있으면 보내기는 새 세션으로 재시작해 나누던 대화를
 * 끊으므로, 쉬는 중이어도 건너뛴다. 세션이 없거나 셸 프롬프트일 때만 보낸다.
 */
async function sessionCheck(agentId: string): Promise<Skip | undefined> {
  let target;
  try { target = await tauriApi.inspectWorkSendTarget(agentId); } catch (error) {
    return { reason: "blocked", detail: error instanceof Error ? error.message : String(error) };
  }
  if (!target.restartRequired || !target.sessionId) return undefined;
  if (target.hasUncommitted) return { reason: "blocked", detail: "work-send-human-input" };
  const phase = useWorkMonitorStore.getState().sessions[target.sessionId]?.phase;
  return { reason: phase === "working" || phase === "waiting" ? "agent-working" : "agent-session" };
}

/** 실행 전에 알 수 있는 막힘. 플랜의 나머지 조건은 `startAutoRun`이 복제본에서 다시 본다. */
async function precheck(item: WorkItem, items: WorkItem[], recurring: boolean, plan: boolean): Promise<Skip | undefined> {
  if (!recurring && item.status !== "todo") return { reason: "blocked", detail: "schedule-not-todo" };
  if (recurring && hasOpenRun(item.id, items)) return { reason: "previous-open" };
  const agentId = item.assigneeAgentId;
  const agent = agentId ? useAppStore.getState().agents[agentId] : undefined;
  if (!agent) return { reason: "blocked", detail: plan ? "auto-run-no-assignee" : "work-send-agent-missing" };
  const profile = workSendDisabledReason(agent);
  if (profile) return { reason: "blocked", detail: profile };
  if (Object.values(useWorkSendStore.getState().busy).includes(agent.id)) return { reason: "blocked", detail: "work-send-busy" };
  if (!plan && pendingPrerequisites(item, items).length) return { reason: "blocked", detail: "work-send-blocked" };
  // 턴 상태를 알아야 세션을 바꿀지 정할 수 있다. 자동 진행과 같은 조건이다.
  if (!useAppStore.getState().appSettings.observerEnabled) return { reason: "blocked", detail: "auto-run-observer-off" };
  return sessionCheck(agent.id);
}

export function skipText(skip: Skip): string {
  const wt = (key: string, options?: Record<string, unknown>) => t(`workpanels:${key}`, options);
  const reason = wt(`schedule.skip.${skip.reason}`);
  if (!skip.detail) return reason;
  const detail = skip.detail.startsWith("schedule-") ? wt(`schedule.detail.${skip.detail}`) : codeText(wt, skip.detail);
  return `${reason} ${detail}`;
}

/** 같은 이유로 거듭 건너뛰면(짧은 간격 예약) 처음 한 번만 알린다. 이유는 예약 영역에 계속 남는다. */
function notifySkip(item: WorkItem, skip: Skip, previous?: Skip): void {
  if (previous && previous.reason === skip.reason && previous.detail === skip.detail) return;
  void maybeSendOsNotification(t("workpanels:schedule.skippedTitle", { title: workItemTitle(item) }), skipText(skip));
}

/** 때가 된 예약 하나를 실행하거나 건너뛴다. */
async function fire(id: string, now: number): Promise<void> {
  const store = useWorkItemStore.getState();
  const item = find(id);
  const schedule = item?.schedule;
  if (!item || !schedule?.enabled || schedule.nextAt === undefined || schedule.nextAt > now) return;
  const due = schedule.nextAt;
  const recurring = isRecurring(item);
  const items = store.items;
  const plan = planItems(item, items).length > 0;
  const skip: Skip | undefined = now - due > MISSED_GRACE_MS ? { reason: "missed" } : await precheck(item, items, recurring, plan);
  // 점검하는 동안 사람이 예약을 바꾸거나 껐으면 이번 판단을 버린다.
  if (!sameWorkValue(find(id)?.schedule, schedule)) return;
  // 다음 회차를 먼저 저장한다. 한 번 예약은 실행하면 지우고, 건너뛰면 꺼서 이유를 남긴다.
  const lastSkip = skip ? { at: due, ...skip } : schedule.lastSkip;
  const advanced: WorkSchedule | undefined = recurring
    ? { ...schedule, nextAt: nextOccurrence(schedule.rule, now, due), lastRunAt: skip ? schedule.lastRunAt : due, lastSkip }
    : skip ? { rule: schedule.rule, enabled: false, lastRunAt: schedule.lastRunAt, lastSkip } : undefined;
  store.setSchedule(id, advanced);
  await useWorkItemStore.getState().flush();
  // 저장하지 못했으면 실행하지 않는다. 다시 켜졌을 때 같은 회차를 또 실행할 수 있어서다.
  if (useWorkItemStore.getState().error) return;
  if (skip) { notifySkip(item, skip, schedule.lastSkip); return; }

  const members = plan ? planItems(item, items).map((member) => member.id) : [];
  const clones = recurring ? useWorkItemStore.getState().cloneScheduled(id, members) : [id];
  const runId = clones?.[0];
  if (!runId) return;
  let failure: Skip | undefined;
  if (plan) {
    const blocked = startAutoRun(runId);
    if (blocked) failure = { reason: "blocked", detail: blocked };
  } else {
    await requestWorkSend(runId, { auto: true });
    const sent = find(runId);
    // 같은 캐릭터의 다른 보내기에 합류하면 오류 없이 끝난다. 세션이 없으면 보내지 못한 것이다.
    if (!sent?.sessionIds.length) failure = { reason: "failed", detail: sent?.lastError ?? "work-send-busy" };
  }
  if (!failure) return;
  // 시작도 못 한 복제본은 지운다. 남기면 다음 회차가 "지난 회차가 안 끝남"으로 계속 막힌다.
  if (recurring) for (const clone of clones) useWorkItemStore.getState().remove(clone);
  const latest = find(id)?.schedule;
  const failed = { at: due, ...failure };
  useWorkItemStore.getState().setSchedule(id, recurring && latest ? { ...latest, lastRunAt: schedule.lastRunAt, lastSkip: failed }
    : { rule: schedule.rule, enabled: false, lastRunAt: schedule.lastRunAt, lastSkip: failed });
  void useWorkItemStore.getState().flush();
  notifySkip(item, failure, schedule.lastSkip);
}

/** 때가 된 예약을 차례로 실행한다. 테스트가 직접 부른다. */
export async function checkSchedules(now = Date.now()): Promise<void> {
  const state = useWorkItemStore.getState();
  if (!state.loaded || state.error) return;
  // 업무 패널을 끈 동안이나 종료 확인 중에는 실행하지 않는다(수신함과 같은 수명). 그 사이 지난 회차는 놓친 것으로 건너뛴다.
  const app = useAppStore.getState();
  if (!app.appSettings.workPanelsEnabled || app.modal.kind === "confirm-quit") return;
  const due = state.items.filter((item) => item.schedule?.enabled && item.schedule.nextAt !== undefined && item.schedule.nextAt <= now && !firing.has(item.id));
  for (const item of due) {
    firing.add(item.id);
    try { await fire(item.id, now); } catch (error) { console.warn("schedule: fire failed", error); } finally { firing.delete(item.id); }
  }
}

export function installScheduleRunner(): () => void {
  let checking: Promise<void> | undefined;
  const check = () => { if (!checking) checking = checkSchedules().finally(() => { checking = undefined; }); };
  // 첫 검사는 한 주기 뒤에 한다. 부팅 중(세션 입양·관찰 상태 복원 전)에 보내지 않게 한다.
  const timer = setInterval(check, CHECK_MS);
  return () => clearInterval(timer);
}
