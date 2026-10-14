// 자동 진행 루프(docs/auto-run-design.md). 루트 업무가 기다리는 선행 작업을 진행자 한 명이
// 차례로 진행한다. 태스크마다 새 세션으로 보내고, 턴이 끝나면 판단자(claude -p 이어지는
// 세션)에게 next·followup·stop 중 하나를 받아 적용한다. 루프와 상태는 여기만 쥔다.
import type { AgentProfile, AutoRun, AutoRunJudgeResult, AutoRunLogEntry, AutoRunPauseReason, WorkItem } from "@shared/types";
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

/** 시작을 막는 이유. 없으면 undefined. 코드는 `sendErrorText`·`autoRun.blocked.*`로 표시한다. */
export function startBlockedReason(root: WorkItem, items: WorkItem[], agents: Readonly<Record<string, AgentProfile>>, observerEnabled: boolean): string | undefined {
  if (root.status === "done") return "auto-run-root-done";
  if (!root.assigneeAgentId || !agents[root.assigneeAgentId]) return "auto-run-no-assignee";
  const open = planItems(root, items).filter((item) => item.status !== "done");
  if (!open.length) return "auto-run-no-plan";
  // 진행자 한 명은 끝나지 않은 실행을 하나만 가진다. 멈춘 실행의 현재 태스크와 섞이지 않게 한다.
  if (items.some((item) => item.id !== root.id && item.autoRun && item.autoRun.state !== "finished" && item.assigneeAgentId === root.assigneeAgentId)) return "auto-run-runner-busy";
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
  if (root.body.trim()) parts.push(`${t("workpanels:autoRun.worker.memo")}\n${clip(root.body.trim(), MAX_MEMO_CHARS)}`);
  if (notes.length) parts.push(`${t("workpanels:autoRun.worker.notes")}\n${notes.join("\n")}`);
  parts.push(t("workpanels:autoRun.worker.report"));
  return parts.join("\n\n");
}

const JUDGE_RULES = `You supervise an automatic task run in Agent Office. A coding agent (the worker) runs the tasks of a plan one at a time in a terminal. Every task starts in a fresh worker session. After each worker turn you get the worker's final reply and decide what happens next. You have no tools: judge only from what you are given.

Decisions:
- "next": the current task is done well enough. It is marked done and the next task starts in a fresh session. Put in "note" what later tasks must know (decisions, names of files or APIs, caveats); it is handed to them. Leave "message" empty.
- "followup": the worker should keep going in the same session. Put the exact instruction for the worker in "message". Use it for unfinished work, missing or failed verification, or to answer a question the worker asked. You can send at most ${MAX_FOLLOW_UPS} follow-ups per task.
- "stop": a human must step in (unclear requirements, a risky or destructive step, repeated failure, a choice you should not make alone). Put the reason for the human in "message".
Prefer "followup" over "stop" when the worker can fix it alone. Do not accept a task whose reply shows no evidence that it was done. Write "message" and "note" in the language the tasks are written in.`;

export interface JudgeTurn {
  root: WorkItem;
  plan: WorkItem[];
  task: WorkItem;
  followUps: number;
  reply: string | null;
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

// ── 사람 조작 ──────────────────────────────────────────────────────────

export function startAutoRun(rootId: string): string | undefined {
  const root = find(rootId);
  if (!root) return "auto-run-no-plan";
  const state = useAppStore.getState();
  const blocked = startBlockedReason(root, items(), state.agents, state.appSettings.observerEnabled);
  if (blocked) return blocked;
  const run: AutoRun = { state: "running", judgeSessionId: crypto.randomUUID(), judgeStarted: false, followUps: 0, costUsd: 0, log: [], startedAt: Date.now() };
  useWorkItemStore.getState().setAutoRun(rootId, withLog(run, { kind: "start" }));
  kick(rootId);
  return undefined;
}
export function pauseAutoRun(rootId: string): void {
  if (running(rootId)) pause(rootId, "user");
}
export function resumeAutoRun(rootId: string): void {
  const run = find(rootId)?.autoRun;
  if (!run || run.state !== "paused") return;
  // 판단 직후 멈췄다면 같은 턴을 다시 판단한다. 후속 지시 한도에 걸렸다면 한 번 더 기회를 준다.
  const followUps = run.pauseReason === "followup-limit" ? MAX_FOLLOW_UPS - 1 : run.followUps;
  useWorkItemStore.getState().setAutoRun(rootId, withLog({ ...run, state: "running", followUps, pauseReason: undefined, pauseDetail: undefined }, { kind: "resume" }));
  kick(rootId);
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
      return judge(rootId, current);
    }
    // 사람이 완료로 바꿨다. 다음으로 넘어간다.
    update(rootId, (latest) => withLog({ ...latest, currentId: undefined, followUps: 0, sentAt: undefined }, { kind: "human-done", taskId: current.id }));
  }
  return pickAndSend(rootId);
}

async function pickAndSend(rootId: string): Promise<void> {
  const root = find(rootId)!;
  const runner = root.assigneeAgentId!;
  const all = items();
  const plan = planItems(root, all);
  const open = plan.filter((item) => item.status !== "done");
  if (!open.length) {
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
  if (!next.assigneeAgentId) useWorkItemStore.getState().edit(next.id, { assigneeAgentId: runner });
  const run = running(rootId)!;
  update(rootId, (latest) => withLog({ ...latest, currentId: next.id, followUps: 0, sentAt: Date.now() }, { kind: "send", taskId: next.id }));
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
  const run = running(rootId)!;
  const runner = root.assigneeAgentId!;
  const state = useAppStore.getState();
  const profile = state.agents[runner];
  const session = lastSession(current);
  const cli = normalizeWorkAgent(current.agent ?? profile?.defaultAgent ?? { cli: "claude" }).cli;
  const cwd = (session ? useSessionSummaryStore.getState().summaries[session]?.cwd : undefined) ?? state.taskLabels[runner]?.cwd ?? profile?.cwd ?? "";
  const reply = await tauriApi.workLastReply(runner, cli, cwd).catch((error: unknown) => { console.warn("auto-run: reply read failed", error); return null; });
  const plan = planItems(root, items());
  const memoHash = hash(root.body.trim());
  const prompt = judgePrompt({ root, plan, task: current, followUps: run.followUps, reply, first: !run.judgeStarted, memoChanged: run.judgedBodyHash !== memoHash });
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
  update(rootId, (latest) => ({ ...latest, judgeStarted: true, judgedBodyHash: memoHash, costUsd: latest.costUsd + (result.costUsd ?? 0) }));
  const latest = running(rootId);
  if (!latest || latest.currentId !== current.id) return;
  if (result.decision === "next") {
    update(rootId, (r) => withLog({ ...r, currentId: undefined, followUps: 0, sentAt: undefined }, { kind: "next", taskId: current.id, text: result.note }));
    useWorkItemStore.getState().edit(current.id, { status: "done" });
    return pickAndSend(rootId);
  }
  if (result.decision === "stop") return pause(rootId, "judge-stop", result.message);
  if (latest.followUps >= MAX_FOLLOW_UPS) return pause(rootId, "followup-limit", result.message);
  if (!session || !result.message.trim()) return pause(rootId, "judge-failed", "auto-run-judge-output");
  let submittedAt: number;
  try {
    ({ submittedAt } = await tauriApi.sendWorkFollowUp({ agentId: runner, sessionId: session, text: result.message }));
  } catch (error) {
    const code = error instanceof Error ? error.message : String(error);
    if (running(rootId)) pause(rootId, code === "work-send-human-input" ? "human-input" : "send-failed", code);
    return;
  }
  update(rootId, (r) => withLog({ ...r, followUps: r.followUps + 1, sentAt: submittedAt }, { kind: "followup", taskId: current.id, text: result.message }));
  // 새 턴의 prompt 신호보다 먼저 판단이 다시 돌지 않게 바로 진행 중으로 돌린다.
  if (find(current.id)?.status === "review") useWorkItemStore.getState().edit(current.id, { status: "in_progress" });
}

/** 업무·모니터 변화마다 돌고 있는 실행을 깨운다. 단계가 스스로 할 일이 없으면 바로 끝난다. */
export function installAutoRun(): () => void {
  const wake = () => { for (const item of items()) if (item.autoRun?.state === "running") kick(item.id); };
  const offItems = useWorkItemStore.subscribe((state, previous) => { if (state.items !== previous.items) wake(); });
  const offMonitor = useWorkMonitorStore.subscribe(wake);
  return () => { offItems(); offMonitor(); };
}
