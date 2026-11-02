// 자동 진행 루프. 루트 업무가 기다리는 선행 작업을 진행자 한 명이
// 차례로 진행한다. 플랜 루트(`plan`)는 플랜 메모라 실행하지 않고, 아닌 루트는 마지막 단계로 진행한다.
// 태스크마다 새 세션으로 보내고, 턴이 끝나면 판단자(claude -p 이어지는
// 세션)에게 next·followup·stop 중 하나를 받아 적용한다. 루프와 상태는 여기만 쥔다.
// 태스크·후속 지시·판단자를 보내기 전마다 구독 사용량을 예측해(usage/usageForecast.ts),
// 모자랄 것 같으면 한도 창 리셋까지 기다렸다가 스스로 이어 간다.
import type { AgentProfile, AutoRun, AutoRunJudgeResult, AutoRunLogEntry, AutoRunPauseReason, AutoRunUsageWait, WorkItem } from "@shared/types";
import { t } from "../i18n";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import { normalizeWorkAgent } from "../profile/generate";
import { useWorkItemStore } from "./workItemStore";
import { useWorkMonitorStore } from "./workMonitor";
import { useSessionSummaryStore } from "./sessionSummaryStore";
import { pendingPrerequisites } from "./workDependencies";
import { workItemTitle } from "./workItemDraft";
import { requestWorkSend, workSendDisabledReason } from "./workSendStore";
import type { ForecastProvider, UsageBlock } from "../usage/usageForecast";
import {
  assessFollowup, assessJudge, assessTask, costMark, costSinceMark, recordFollowupSample, recordJudgeCost,
  recordTaskSample, refreshUsage, workerProvider,
} from "../usage/usageForecastStore";

export const MAX_FOLLOW_UPS = 3;
const MAX_LOG = 200;
const MAX_NOTES = 20;
const MAX_MEMO_CHARS = 16_000;

/** 루트가 직접·간접으로 기다리는 업무 전부(선행 작업의 전이 폐포), 진행 순서대로. */
export function planItems(root: WorkItem, items: WorkItem[]): WorkItem[] {
  const byId = new Map(items.map((item) => [item.id, item]));
  const members = new Map<string, WorkItem>();
  const stack = [...root.dependsOn ?? []];
  while (stack.length) {
    const id = stack.pop()!;
    const item = byId.get(id);
    if (!item || id === root.id || members.has(id)) continue;
    members.set(id, item);
    stack.push(...item.dependsOn ?? []);
  }
  // 선행 작업이 먼저 오고, 같이 준비되는 것끼리는 패널 순서를 따른다(완료된 것이 앞).
  const rank = (item: WorkItem) => (item.status === "done" ? 0 : 1) * 1e9 + item.order;
  const placed = new Set<string>();
  const ordered: WorkItem[] = [];
  let rest = [...members.values()];
  while (rest.length) {
    const ready = rest.filter((item) => (item.dependsOn ?? []).every((id) => !members.has(id) || placed.has(id)));
    // 순환은 저장 단계에서 막지만, 남으면 순서만 포기하고 나머지를 붙인다.
    const batch = (ready.length ? ready : rest).sort((a, b) => rank(a) - rank(b)).slice(0, 1);
    for (const item of batch) { placed.add(item.id); ordered.push(item); }
    rest = rest.filter((item) => !placed.has(item.id));
  }
  return ordered;
}

/**
 * 자동 진행이 차례로 보낼 태스크. 플랜 루트(메모 전용)는 선행 작업만, 아닌 루트는 선행 작업 뒤에
 * 루트 자신까지 넣는다. 루트에 적힌 작업이 실행되지 않은 채 검토 대기로 남지 않게 한다.
 */
export function runSteps(root: WorkItem, items: WorkItem[]): WorkItem[] {
  const plan = planItems(root, items);
  return root.plan || !plan.length ? plan : [...plan, root];
}

/** 진행자를 붙잡고 있는 다른 루트(끝나지 않은 자동 진행). 시작 차단 문구에서 이름과 열기로 보여 준다. */
export function busyRunRoot(root: WorkItem, items: WorkItem[]): WorkItem | undefined {
  return items.find((item) => item.id !== root.id && item.autoRun && item.autoRun.state !== "finished" && item.assigneeAgentId === root.assigneeAgentId);
}

/** 시작을 막는 이유. 없으면 undefined. 코드는 `sendErrorText`·`autoRun.blocked.*`로 표시한다. */
export function startBlockedReason(root: WorkItem, items: WorkItem[], agents: Readonly<Record<string, AgentProfile>>, observerEnabled: boolean): string | undefined {
  if (root.status === "done") return "auto-run-root-done";
  if (!root.assigneeAgentId || !agents[root.assigneeAgentId]) return "auto-run-no-assignee";
  const open = runSteps(root, items).filter((item) => item.status !== "done");
  if (!open.length) return "auto-run-no-plan";
  // 루트만 남았는데 진행 중·검토 대기면 보낼 수 없다. 다시 진행하려면 할 일로 되돌린다.
  if (open.length === 1 && open[0].id === root.id && root.status !== "todo") return "auto-run-root-busy";
  // 진행자 한 명은 끝나지 않은 실행을 하나만 가진다. 멈춘 실행의 현재 태스크와 섞이지 않게 한다.
  if (busyRunRoot(root, items)) return "auto-run-runner-busy";
  const profile = workSendDisabledReason(agents[root.assigneeAgentId]);
  if (profile) return profile;
  if (!observerEnabled) return "auto-run-observer-off";
  if (open.some((item) => item.assigneeAgentId && item.assigneeAgentId !== root.assigneeAgentId)) return "auto-run-foreign-task";
  return undefined;
}

function hash(text: string): string {
  let h = 0x811c9dc5;
  for (let n = 0; n < text.length; n++) { h ^= text.charCodeAt(n); h = Math.imul(h, 0x01000193); }
  return (h >>> 0).toString(16);
}
function clip(text: string, max: number): string {
  return text.length > max ? `…${text.slice(text.length - max)}` : text;
}

/** 다음 태스크 프롬프트 뒤에 붙이는 맥락. 새 세션이 앞 태스크를 모르므로 인계 메모를 넘긴다. */
export function workerContext(root: WorkItem, plan: WorkItem[], task: WorkItem, log: AutoRunLogEntry[]): string {
  const titles = new Map(plan.map((item) => [item.id, workItemTitle(item)]));
  const notes = log.filter((entry) => entry.kind === "next" && entry.text?.trim()).slice(-MAX_NOTES)
    .map((entry) => `- ${titles.get(entry.taskId ?? "") ?? "?"}: ${entry.text!.trim()}`);
  const parts = [
    "---",
    t("workpanels:autoRun.worker.intro", { plan: workItemTitle(root), index: plan.findIndex((item) => item.id === task.id) + 1, total: plan.length }),
  ];
  // 루트를 진행할 때는 루트 본문이 곧 태스크 본문이라 메모로 다시 붙이지 않는다.
  if (task.id === root.id) parts.push(t("workpanels:autoRun.worker.rootLast"));
  else if (root.body.trim()) parts.push(`${t("workpanels:autoRun.worker.memo")}\n${clip(root.body.trim(), MAX_MEMO_CHARS)}`);
  if (notes.length) parts.push(`${t("workpanels:autoRun.worker.notes")}\n${notes.join("\n")}`);
  parts.push(t("workpanels:autoRun.worker.report", { id: task.id }));
  return parts.join("\n\n");
}

const JUDGE_RULES = `You supervise an automatic task run in Agent Office. A coding agent (the worker) runs the tasks of a plan one at a time in a terminal. Nobody watches the worker: between tasks you stand in for the human. Every task starts in a fresh worker session. After each worker turn you get the worker's report, if it wrote one this turn (made with office-task --report; fields status, changed, verified, decisions, needs_human), and its final reply. You have no tools: judge only from what you are given.

Decisions:
- "next": the current task is done well enough. Accept it when the report or the reply says what changed and how it was verified (commands run and their results). For work with nothing to run (docs, notes, design), a clear account of what changed is enough. It is marked done and the next task starts in a fresh session. Put in "note" what later tasks must know (decisions, names of files or APIs, caveats); it is handed to them. Leave "message" empty.
- "followup": the worker should keep going in the same session. Put the exact instruction for the worker in "message". Use it for unfinished work, missing or failed verification, a missing report (ask for it), a question that the plan notes, the task description or the hand-off notes answer, and a reversible choice the worker left open: pick the option that fits the plan best and tell the worker to go ahead and record it under decisions. You can send at most ${MAX_FOLLOW_UPS} follow-ups per task.
- "stop": only when a human really must act: the next step is irreversible or leaves the machine (push, deploy, publish, delete data, spend money, use credentials) and neither the plan notes nor the task allows it; the requirements contradict each other and the plan notes do not settle it; the same failure repeats after follow-ups; or the worker needs something only a human has (access, secrets, an outside decision). In "message", ask the human one concrete question, with the options if there are any, and say briefly where the task stands.
Not reasons to stop: offers of optional extra work ("If you want, I can…"; ignore them), asking whether to continue, asking to confirm a reversible choice, or doubts the worker can check by itself. A report with status "blocked" is the worker's claim: stop only if its needs_human meets the rules above, and otherwise answer it with "followup". Prefer "followup" over "stop" when the worker can fix it alone. Do not accept a task when neither the report nor the reply shows that it was done. Write "message" and "note" in the language the tasks are written in.`;

export interface JudgeTurn {
  root: WorkItem;
  plan: WorkItem[];
  task: WorkItem;
  followUps: number;
  reply: string | null;
  /** 이번 턴에 작업자가 `office-task --report`로 남긴 보고. */
  report: string | null;
  /** 판단자 세션의 첫 호출인가. */
  first: boolean;
  /** 루트 본문이 지난 판단 뒤 바뀌었는가(첫 호출이면 무시). */
  memoChanged: boolean;
}
/** 판단자에게 보낼 메시지. 첫 호출에만 규칙·플랜을 넣고, 이후로는 이번 턴의 사건만 덧붙인다. */
export function judgePrompt(turn: JudgeTurn): string {
  const { root, plan, task } = turn;
  const parts: string[] = [];
  const memo = clip(root.body.trim(), MAX_MEMO_CHARS);
  if (turn.first) {
    parts.push(JUDGE_RULES);
    parts.push(`Plan: ${workItemTitle(root)}`);
    if (memo) parts.push(`Plan notes from the human (they may update them later):\n<<<\n${memo}\n>>>`);
    parts.push(`Tasks in order:\n${plan.map((item, n) => `${n + 1}. ${workItemTitle(item)}${item.status === "done" ? " (already done)" : ""}`).join("\n")}`);
  } else if (turn.memoChanged) {
    parts.push(`The human updated the plan notes:\n<<<\n${memo || "(empty)"}\n>>>`);
  }
  const index = plan.findIndex((item) => item.id === task.id) + 1;
  parts.push(`Task ${index}/${plan.length}: "${workItemTitle(task)}" — the worker's turn ended. Follow-ups sent for this task so far: ${turn.followUps}/${MAX_FOLLOW_UPS}.`);
  if (turn.followUps === 0 && task.body.trim()) parts.push(`Task description:\n<<<\n${task.body.trim()}\n>>>`);
  parts.push(turn.report?.trim() ? `Worker's report:\n<<<\n${turn.report.trim()}\n>>>` : "Worker's report: (none written this turn)");
  parts.push(`Worker's final reply:\n<<<\n${turn.reply?.trim() || "(the reply could not be read)"}\n>>>`);
  parts.push("Decide.");
  return parts.join("\n\n");
}

// ── 상태 쓰기 ──────────────────────────────────────────────────────────

function items(): WorkItem[] { return useWorkItemStore.getState().items; }
function find(id: string | undefined): WorkItem | undefined { return id ? items().find((item) => item.id === id) : undefined; }
function withLog(run: AutoRun, entry: Omit<AutoRunLogEntry, "at">): AutoRun {
  return { ...run, log: [...run.log, { at: Date.now(), ...entry }].slice(-MAX_LOG) };
}
/** 최신 상태를 읽어 고친다. 실행이 사라졌으면 아무것도 하지 않는다. */
function update(rootId: string, change: (run: AutoRun) => AutoRun): void {
  const run = find(rootId)?.autoRun;
  if (run) useWorkItemStore.getState().setAutoRun(rootId, change(run));
}
function pause(rootId: string, reason: AutoRunPauseReason, detail?: string): void {
  update(rootId, (run) => withLog({ ...run, state: "paused", pauseReason: reason, pauseDetail: detail }, { kind: "pause", text: reason }));
}
function running(rootId: string): AutoRun | undefined {
  const run = find(rootId)?.autoRun;
  return run?.state === "running" ? run : undefined;
}

/** 태스크를 맡을 CLI. 보내기(workSendStore)와 같은 규칙이다. */
function taskCli(item: WorkItem, profile: AgentProfile | undefined) {
  return normalizeWorkAgent(item.agent ?? profile?.defaultAgent ?? { cli: "claude" }).cli;
}

/** 다음에 보낼 태스크의 작업자 구독(화면의 사용량 예측용). 남은 단계가 없으면 루트 기준. */
export function nextTaskWorker(root: WorkItem, all: WorkItem[], agents: Readonly<Record<string, AgentProfile>>): ForecastProvider | null {
  const profile = root.assigneeAgentId ? agents[root.assigneeAgentId] : undefined;
  const run = root.autoRun;
  const next = (run?.currentId ? all.find((item) => item.id === run.currentId) : undefined)
    ?? runSteps(root, all).find((item) => item.status !== "done" && !pendingPrerequisites(item, all).length) ?? root;
  return workerProvider({ cli: taskCli(next, profile) });
}

/**
 * 사용량이 모자랄 것 같으면 사용량 대기로 멈추고 true. 사람이 대기를 건너뛰었으면 한 번
 * 통과시킨다. `patch`는 멈출 때 함께 남길 것(보내지 못한 후속 지시).
 */
function waitForUsage(rootId: string, stage: AutoRunUsageWait["stage"], worker: ForecastProvider | null, assess: () => UsageBlock | null, patch: Partial<AutoRun> = {}): boolean {
  const run = running(rootId);
  if (!run || run.usageGuard === false) return false;
  if (run.usageOverride) { update(rootId, (latest) => ({ ...latest, usageOverride: undefined })); return false; }
  const block = assess();
  if (!block) return false;
  const usageWait: AutoRunUsageWait = { ...block, stage, worker };
  update(rootId, (latest) => withLog({ ...latest, ...patch, state: "paused", pauseReason: "usage-wait", pauseDetail: undefined, usageWait }, { kind: "pause", text: "usage-wait" }));
  return true;
}

// ── 사람 조작 ──────────────────────────────────────────────────────────

export function startAutoRun(rootId: string, options: { usageGuard?: boolean } = {}): string | undefined {
  const root = find(rootId);
  if (!root) return "auto-run-no-plan";
  const state = useAppStore.getState();
  const blocked = startBlockedReason(root, items(), state.agents, state.appSettings.observerEnabled);
  if (blocked) return blocked;
  const run: AutoRun = { state: "running", judgeSessionId: crypto.randomUUID(), judgeStarted: false, followUps: 0, costUsd: 0, log: [], startedAt: Date.now(), usageGuard: options.usageGuard ?? true };
  useWorkItemStore.getState().setAutoRun(rootId, withLog(run, { kind: "start" }));
  kick(rootId);
  return undefined;
}
export function pauseAutoRun(rootId: string): void {
  // 사용량 대기도 사람이 멈추면 스스로 이어 가지 않는다.
  if (running(rootId) || find(rootId)?.autoRun?.pauseReason === "usage-wait") pause(rootId, "user");
}
/**
 * 멈춘 실행을 이어 간다. 사람이 사용량 대기를 이어 가면 다음 사용량 검사 한 번을 건너뛴다.
 * `auto`는 대기가 끝나 스스로 이어 가는 경우다(다시 검사한다).
 */
export function resumeAutoRun(rootId: string, options: { auto?: boolean } = {}): void {
  const run = find(rootId)?.autoRun;
  if (!run || run.state !== "paused") return;
  if (options.auto && run.pauseReason !== "usage-wait") return;
  // 판단 직후 멈췄다면 같은 턴을 다시 판단한다. 후속 지시 한도에 걸렸다면 한 번 더 기회를 준다.
  const followUps = run.pauseReason === "followup-limit" ? MAX_FOLLOW_UPS - 1 : run.followUps;
  const usageOverride = run.pauseReason === "usage-wait" && !options.auto ? true : undefined;
  useWorkItemStore.getState().setAutoRun(rootId, withLog({ ...run, state: "running", followUps, pauseReason: undefined, pauseDetail: undefined, usageWait: undefined, usageOverride }, { kind: "resume" }));
  kick(rootId);
}
/** 사용량 대기를 켜고 끈다. 기다리는 중에 끄면 바로 이어 간다. */
export function setAutoRunUsageGuard(rootId: string, on: boolean): void {
  update(rootId, (run) => ({ ...run, usageGuard: on }));
  if (!on) resumeAutoRun(rootId, { auto: true });
}
/** 실행 상태와 기록을 지운다. 업무 상태는 그대로 둔다. */
export function endAutoRun(rootId: string): void {
  useWorkItemStore.getState().setAutoRun(rootId, undefined);
}

// ── 루프 ──────────────────────────────────────────────────────────────

const inFlight = new Map<string, { again: boolean }>();
/** 루트 하나의 단계를 직렬로 돌린다. 도는 중에 다시 불리면 끝난 뒤 한 번 더 돈다. */
function kick(rootId: string): void {
  const flight = inFlight.get(rootId);
  if (flight) { flight.again = true; return; }
  const state = { again: false };
  inFlight.set(rootId, state);
  void (async () => {
    try {
      do { state.again = false; await stepAutoRun(rootId); } while (state.again && running(rootId));
    } catch (error) {
      console.warn("auto-run: step failed", error);
      pause(rootId, "send-failed", error instanceof Error ? error.message : String(error));
    } finally { inFlight.delete(rootId); }
  })();
}
export function isAutoRunStepping(rootId: string): boolean { return inFlight.has(rootId); }

function lastSession(item: WorkItem): string | undefined { return item.sessionIds[item.sessionIds.length - 1]; }

/** 한 단계. 테스트가 직접 부른다. */
export async function stepAutoRun(rootId: string): Promise<void> {
  const run = running(rootId);
  const root = find(rootId);
  if (!run || !root) return;
  const runner = root.assigneeAgentId;
  if (!runner) return pause(rootId, "task-changed");
  if (run.currentId) {
    const current = find(run.currentId);
    if (!current || current.status === "todo") return pause(rootId, "task-changed", current ? workItemTitle(current) : undefined);
    if (current.status === "in_progress") {
      const session = lastSession(current);
      if (session && useWorkMonitorStore.getState().sessions[session]?.phase === "ended") return pause(rootId, "session-ended", workItemTitle(current));
      return; // 턴이 끝나기를 기다린다.
    }
    if (current.status === "review") {
      // 후속 지시 전에 끝난 턴의 늦은 신호면 새 턴을 기다린다.
      const observed = useWorkMonitorStore.getState().sessions[lastSession(current) ?? ""];
      if (observed && run.sentAt && observed.at < run.sentAt) return;
      // 판단은 끝났고 사용량 때문에 후속 지시만 미뤄 뒀다. 다시 판단하지 않고 보낸다.
      if (run.pendingFollowUp) return sendFollowUp(rootId, current, run.pendingFollowUp);
      return judge(rootId, current);
    }
    // 사람이 완료로 바꿨다. 다음으로 넘어간다.
    update(rootId, (latest) => withLog({ ...latest, currentId: undefined, followUps: 0, sentAt: undefined, pendingFollowUp: undefined, taskMark: undefined, turnMark: undefined }, { kind: "human-done", taskId: current.id }));
  }
  return pickAndSend(rootId);
}

async function pickAndSend(rootId: string): Promise<void> {
  const root = find(rootId)!;
  const runner = root.assigneeAgentId!;
  const all = items();
  const plan = runSteps(root, all);
  const open = plan.filter((item) => item.status !== "done");
  // 플랜 루트가 아니면 이번 실행에서 루트까지 받아들여야 끝난다.
  const rootRan = !root.plan && find(rootId)!.autoRun!.log.some((entry) => entry.kind === "next" && entry.taskId === rootId);
  if (!open.length || rootRan) {
    update(rootId, (run) => withLog({ ...run, state: "finished", currentId: undefined, followUps: 0, sentAt: undefined }, { kind: "finish" }));
    if (root.status !== "done" && root.status !== "review") useWorkItemStore.getState().edit(rootId, { status: "review" });
    return;
  }
  const ready = open.filter((item) => !pendingPrerequisites(item, all).length);
  const busy = ready.find((item) => item.status !== "todo");
  if (busy) return pause(rootId, "task-busy", workItemTitle(busy));
  const next = ready[0];
  if (!next) return pause(rootId, "task-changed");
  if (next.assigneeAgentId && next.assigneeAgentId !== runner) return pause(rootId, "task-changed", workItemTitle(next));
  const worker = workerProvider({ cli: taskCli(next, useAppStore.getState().agents[runner]) });
  if (waitForUsage(rootId, "task", worker, () => assessTask(worker))) return;
  if (!next.assigneeAgentId) useWorkItemStore.getState().edit(next.id, { assigneeAgentId: runner });
  const run = running(rootId)!;
  const mark = { ...costMark(runner), judgeCost: run.costUsd };
  update(rootId, (latest) => withLog({ ...latest, currentId: next.id, followUps: 0, sentAt: Date.now(), pendingFollowUp: undefined, taskMark: mark, turnMark: mark }, { kind: "send", taskId: next.id }));
  const before = next.sessionIds.length;
  await requestWorkSend(next.id, { auto: true, appendBody: workerContext(root, plan, next, run.log) });
  const sent = find(next.id);
  if (!sent || sent.lastError || sent.sessionIds.length <= before) {
    // 사람이 그 사이 멈췄으면 사유를 덮지 않는다.
    if (!running(rootId)) return;
    const error = sent?.lastError;
    // 보내지 못했으면 태스크는 할 일로 남으므로 현재 태스크에서 뺀다. 계속을 누르면 다시 보낸다.
    update(rootId, (latest) => ({ ...latest, currentId: undefined }));
    return pause(rootId, error === "work-send-human-input" ? "human-input" : "send-failed", error);
  }
}

async function judge(rootId: string, current: WorkItem): Promise<void> {
  const root = find(rootId)!;
  const runner = root.assigneeAgentId!;
  const state = useAppStore.getState();
  const profile = state.agents[runner];
  const session = lastSession(current);
  const cli = taskCli(current, profile);
  const worker = workerProvider({ cli });
  // 판단자도 Claude 구독을 쓴다. 이미 한도에 닿았으면 부르지 않고 기다린다.
  if (waitForUsage(rootId, "judge", worker, () => assessJudge())) return;
  const run = running(rootId)!;
  recordTurnSample(rootId, runner, worker);
  const cwd = (session ? useSessionSummaryStore.getState().summaries[session]?.cwd : undefined) ?? state.taskLabels[runner]?.cwd ?? profile?.cwd ?? "";
  const reply = await tauriApi.workLastReply(runner, cli, cwd).catch((error: unknown) => { console.warn("auto-run: reply read failed", error); return null; });
  // 이번 턴을 보낸 뒤에 쓰인 보고만 읽는다. 지난 턴의 보고가 새 턴의 증거로 쓰이지 않게 한다.
  const report = await tauriApi.autoRunReport(current.id, run.sentAt ?? 0).catch((error: unknown) => { console.warn("auto-run: report read failed", error); return null; });
  const plan = runSteps(root, items());
  const memoHash = hash(root.body.trim());
  const prompt = judgePrompt({ root, plan, task: current, followUps: run.followUps, reply, report, first: !run.judgeStarted, memoChanged: run.judgedBodyHash !== memoHash });
  const model = root.agent?.cli === "claude" ? root.agent.model : undefined;
  let result: AutoRunJudgeResult;
  try {
    result = await tauriApi.autoRunJudge({ sessionId: run.judgeSessionId, resume: run.judgeStarted, prompt, model });
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    // 첫 호출이 실패했으면 그 id로 세션이 생겼는지 알 수 없다. 다음 시도는 새 id로 시작한다.
    update(rootId, (latest) => latest.judgeStarted ? latest : { ...latest, judgeSessionId: crypto.randomUUID() });
    if (running(rootId)) pause(rootId, "judge-failed", detail);
    return;
  }
  // 판단자 대화에는 이번 턴이 이미 들어갔다. 멈췄더라도 그 사실과 비용은 남긴다.
  recordJudgeCost(result.costUsd);
  update(rootId, (latest) => ({ ...latest, judgeStarted: true, judgedBodyHash: memoHash, costUsd: latest.costUsd + (result.costUsd ?? 0) }));
  const latest = running(rootId);
  if (!latest || latest.currentId !== current.id) return;
  if (result.decision === "next") {
    recordTaskUsage(latest, runner, worker);
    update(rootId, (r) => withLog({ ...r, currentId: undefined, followUps: 0, sentAt: undefined, pendingFollowUp: undefined, taskMark: undefined, turnMark: undefined }, { kind: "next", taskId: current.id, text: result.note }));
    // 루트 자신은 완료로 바꾸지 않는다(완료면 실행이 루트 완료로 끝난다). 검토 대기로 두고 실행을 마친다.
    if (current.id === rootId) return pickAndSend(rootId);
    useWorkItemStore.getState().edit(current.id, { status: "done" });
    return pickAndSend(rootId);
  }
  if (result.decision === "stop") return pause(rootId, "judge-stop", result.message);
  if (latest.followUps >= MAX_FOLLOW_UPS) return pause(rootId, "followup-limit", result.message);
  if (!session || !result.message.trim()) return pause(rootId, "judge-failed", "auto-run-judge-output");
  return sendFollowUp(rootId, current, result.message);
}

/** 판단자의 후속 지시를 같은 세션에 넣는다. 사용량이 모자라면 지시를 남겨 두고 기다린다. */
async function sendFollowUp(rootId: string, current: WorkItem, message: string): Promise<void> {
  const runner = find(rootId)!.assigneeAgentId!;
  const session = lastSession(current);
  const worker = workerProvider({ cli: taskCli(current, useAppStore.getState().agents[runner]) });
  if (waitForUsage(rootId, "followup", worker, () => assessFollowup(worker), { pendingFollowUp: message })) return;
  if (!session) return pause(rootId, "session-ended", workItemTitle(current));
  const mark = { ...costMark(runner), judgeCost: running(rootId)?.costUsd ?? 0 };
  let submittedAt: number;
  try {
    ({ submittedAt } = await tauriApi.sendWorkFollowUp({ agentId: runner, sessionId: session, text: message }));
  } catch (error) {
    const code = error instanceof Error ? error.message : String(error);
    // 계속을 누르면 같은 턴을 다시 판단한다.
    if (running(rootId)) {
      update(rootId, (r) => ({ ...r, pendingFollowUp: undefined }));
      pause(rootId, code === "work-send-human-input" ? "human-input" : "send-failed", code);
    }
    return;
  }
  update(rootId, (r) => withLog({ ...r, followUps: r.followUps + 1, sentAt: submittedAt, pendingFollowUp: undefined, turnMark: mark }, { kind: "followup", taskId: current.id, text: message }));
  // 새 턴의 prompt 신호보다 먼저 판단이 다시 돌지 않게 바로 진행 중으로 돌린다.
  if (find(current.id)?.status === "review") useWorkItemStore.getState().edit(current.id, { status: "in_progress" });
}

// ── 사용량 표본 ────────────────────────────────────────────────────────

/** 판단할 턴이 후속 지시였으면 그 한 턴의 비용을 남긴다. 같은 턴을 다시 판단해도 한 번만 센다. */
function recordTurnSample(rootId: string, runner: string, worker: ForecastProvider | null): void {
  const run = running(rootId);
  const mark = run?.turnMark;
  if (!run || !mark || run.followUps === 0) return;
  const cost = costSinceMark(runner, mark);
  if (cost !== null) recordFollowupSample({ provider: worker, cost, durationMs: Date.now() - mark.at, at: Date.now() });
  update(rootId, (latest) => ({ ...latest, turnMark: undefined }));
}

/** 받아들인 태스크 하나의 작업자·판단자 비용과 소요 시간을 남긴다. */
function recordTaskUsage(run: AutoRun, runner: string, worker: ForecastProvider | null): void {
  const mark = run.taskMark;
  const cost = costSinceMark(runner, mark);
  if (!mark || cost === null) return;
  recordTaskSample({ provider: worker, cost, judgeCost: Math.max(0, run.costUsd - mark.judgeCost), durationMs: Date.now() - mark.at, at: Date.now() });
}

// ── 사용량 대기 ────────────────────────────────────────────────────────

const USAGE_CHECK_MS = 30_000;
const waking = new Set<string>();

function stillShort(wait: AutoRunUsageWait): boolean {
  const worker = wait.worker ?? null;
  return !!(wait.stage === "task" ? assessTask(worker) : wait.stage === "followup" ? assessFollowup(worker) : assessJudge());
}

/**
 * 사용량 대기 중인 실행을 살핀다. 다시 볼 시각이 지났으면 사용량을 새로 받아 이어 가고(이어 간 단계가
 * 다시 검사한다), 그 전이라도 새 조회값으로 보니 모자라지 않으면 일찍 이어 간다.
 */
export async function checkUsageWaits(now = Date.now()): Promise<void> {
  for (const item of items()) {
    const run = item.autoRun;
    const wait = run?.state === "paused" && run.pauseReason === "usage-wait" ? run.usageWait : undefined;
    if (!wait || waking.has(item.id)) continue;
    if (now < wait.until && stillShort(wait)) continue;
    waking.add(item.id);
    try {
      if (now >= wait.until) await refreshUsage();
      // 그 사이 사람이 멈췄거나 끝냈으면 그대로 둔다.
      const latest = find(item.id)?.autoRun;
      if (latest?.state === "paused" && latest.pauseReason === "usage-wait") resumeAutoRun(item.id, { auto: true });
    } finally { waking.delete(item.id); }
  }
}

/** 업무·모니터 변화마다 돌고 있는 실행을 깨운다. 단계가 스스로 할 일이 없으면 바로 끝난다. */
export function installAutoRun(): () => void {
  const wake = () => { for (const item of items()) if (item.autoRun?.state === "running") kick(item.id); };
  const offItems = useWorkItemStore.subscribe((state, previous) => { if (state.items !== previous.items) wake(); });
  const offMonitor = useWorkMonitorStore.subscribe(wake);
  const check = () => { void checkUsageWaits(); };
  const offUsage = useAppStore.subscribe((state, previous) => { if (state.usage !== previous.usage) check(); });
  const timer = setInterval(check, USAGE_CHECK_MS);
  return () => { offItems(); offMonitor(); offUsage(); clearInterval(timer); };
}
