// 예약 실행기. 메인 창에서만 돌며(bootstrap) 주기마다 때가 된 예약을 실행한다.
// 한 번 예약은 그 업무 자체를, 반복 예약은 템플릿을 복제해 실행한다. 선행 작업이 있으면
// 자동 진행으로, 없으면 보내기로 실행한다. 놓치거나 실행할 수 없는 회차는 건너뛰고 알린다.
// 다음 실행 시각은 보내기 전에 먼저 저장해 앱이 죽었다 켜져도 같은 회차를 두 번 실행하지 않는다.
import type { WorkItem, WorkSchedule, WorkScheduleSkipReason } from "@shared/types";
import { t } from "../i18n";
import { useAppStore } from "../store/appStore";
import { maybeSendOsNotification } from "../ipc/osNotify";
import { useWorkItemStore } from "./workItemStore";
import { useWorkMonitorStore } from "./workMonitor";
import { requestWorkSend, useWorkSendStore, workSendDisabledReason } from "./workSendStore";
import { pendingPrerequisites } from "./workDependencies";
import { workItemTitle } from "./workItemDraft";
import { planItems, startAutoRun } from "./autoRun";
import { codeText } from "./sendErrorText";
import { hasOpenRun, isRecurring, nextOccurrence } from "./workSchedule";

/** 예정 시각을 이만큼 넘겨서 알아챘으면(앱 꺼짐·잠자기) 놓친 회차로 보고 건너뛴다. */
export const MISSED_GRACE_MS = 5 * 60_000;
const CHECK_MS = 15_000;

const firing = new Set<string>();
const find = (id: string) => useWorkItemStore.getState().items.find((item) => item.id === id);

/** 담당 캐릭터의 가장 최근 세션이 턴 중이거나 사람 응답을 기다리는가. 사람과 대화 중인 세션을 갈아 끼우지 않는다. */
function agentWorking(agentId: string): boolean {
  let latest: { phase: string; at: number } | undefined;
  for (const observed of Object.values(useWorkMonitorStore.getState().sessions)) {
    if (observed.agentId === agentId && (!latest || observed.at > latest.at)) latest = observed;
  }
  return latest?.phase === "working" || latest?.phase === "waiting";
}

type Skip = { reason: WorkScheduleSkipReason; detail?: string };

/** 실행 전에 알 수 있는 막힘. 플랜의 나머지 조건은 `startAutoRun`이 복제본에서 다시 본다. */
function precheck(item: WorkItem, items: WorkItem[], recurring: boolean, plan: boolean): Skip | undefined {
  if (!recurring && item.status !== "todo") return { reason: "blocked", detail: "schedule-not-todo" };
  if (recurring && hasOpenRun(item.id, items)) return { reason: "previous-open" };
  const agentId = item.assigneeAgentId;
  const agent = agentId ? useAppStore.getState().agents[agentId] : undefined;
  if (!agent) return { reason: "blocked", detail: plan ? "auto-run-no-assignee" : "work-send-agent-missing" };
  const profile = workSendDisabledReason(agent);
  if (profile) return { reason: "blocked", detail: profile };
  if (Object.values(useWorkSendStore.getState().busy).includes(agent.id)) return { reason: "blocked", detail: "work-send-busy" };
  if (agentWorking(agent.id)) return { reason: "agent-working" };
  if (!plan && pendingPrerequisites(item, items).length) return { reason: "blocked", detail: "work-send-blocked" };
  return undefined;
}

export function skipText(skip: Skip): string {
  const wt = (key: string, options?: Record<string, unknown>) => t(`workpanels:${key}`, options);
  const reason = wt(`schedule.skip.${skip.reason}`);
  if (!skip.detail) return reason;
  const detail = skip.detail.startsWith("schedule-") ? wt(`schedule.detail.${skip.detail}`) : codeText(wt, skip.detail);
  return `${reason} ${detail}`;
}

function notifySkip(item: WorkItem, skip: Skip): void {
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
  const skip: Skip | undefined = now - due > MISSED_GRACE_MS ? { reason: "missed" } : precheck(item, items, recurring, plan);
  // 다음 회차를 먼저 저장한다. 한 번 예약은 실행하면 지우고, 건너뛰면 꺼서 이유를 남긴다.
  const lastSkip = skip ? { at: due, ...skip } : schedule.lastSkip;
  const advanced: WorkSchedule | undefined = recurring
    ? { ...schedule, nextAt: nextOccurrence(schedule.rule, now, due), lastRunAt: skip ? schedule.lastRunAt : due, lastSkip }
    : skip ? { rule: schedule.rule, enabled: false, lastRunAt: schedule.lastRunAt, lastSkip } : undefined;
  store.setSchedule(id, advanced);
  await useWorkItemStore.getState().flush();
  // 저장하지 못했으면 실행하지 않는다. 다시 켜졌을 때 같은 회차를 또 실행할 수 있어서다.
  if (useWorkItemStore.getState().error) return;
  if (skip) { notifySkip(item, skip); return; }

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
    if (sent?.lastError && !sent.sessionIds.length) failure = { reason: "failed", detail: sent.lastError };
  }
  if (!failure) return;
  // 시작도 못 한 복제본은 지운다. 남기면 다음 회차가 "지난 회차가 안 끝남"으로 계속 막힌다.
  if (recurring) for (const clone of clones) useWorkItemStore.getState().remove(clone);
  const latest = find(id)?.schedule;
  const failed = { at: due, ...failure };
  useWorkItemStore.getState().setSchedule(id, recurring && latest ? { ...latest, lastRunAt: schedule.lastRunAt, lastSkip: failed }
    : { rule: schedule.rule, enabled: false, lastRunAt: schedule.lastRunAt, lastSkip: failed });
  void useWorkItemStore.getState().flush();
  notifySkip(item, failure);
}

/** 때가 된 예약을 차례로 실행한다. 테스트가 직접 부른다. */
export async function checkSchedules(now = Date.now()): Promise<void> {
  const state = useWorkItemStore.getState();
  if (!state.loaded || state.error) return;
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
