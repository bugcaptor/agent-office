// 토의 실행기. 사회자 주도로 한 토의를 진행한다. 루프와 진행 상태(`Discussion.run`)는 여기만 쥔다.
//
// - 첫 라운드(opening)는 토론자 전원에게 동시에 차례를 주고, 전원 발언을 받은 뒤 한꺼번에 공개한다.
// - 그다음은 사회자 차례(moderator)다. 사회자는 구조화된 지시(JSON 하나)를 낸다:
//   next = 토론자를 순서대로 지목(speakers 단계, 지목 한 번이 한 라운드), ask_user = 사람에게 질문(waiting_user),
//   wrap = 정리(wrap_up). 지시도 발언과 같은 제출 경로(`office-discuss say`·마지막 응답)로 받는다.
// - 참가자 세션은 이어지므로 차례마다 그 세션이 아직 못 본 기록(델타)만 전한다. 세션을 다시 열면 처음부터 전한다.
// - 차례를 받을 참가자의 세션은 그때 연다(첫 프롬프트에 차례를 싣는다). 사회자는 첫 라운드가 끝난 뒤 열린다.
// - 턴 종료는 관찰 훅의 턴 단계(idle, 보낸 뒤 신호)로 안다. 발언이 없거나 지시를 못 읽으면 한 번 재촉하고, 또 그러면 멈춘다.
// - 쓰다 만 사람 입력이 있으면 차례를 넣지 않고 멈춘다(자동 진행과 같은 `work-send-human-input`).
// - 토의 세션은 앱 재시작 때 버려진다. 재시작하면 진행 중이던 토의는 멈춤(restarted)이 되고, 계속하면 세션을 다시 연다.
import type {
  AgentProfile, Discussion, DiscussionPauseReason, DiscussionRun, DiscussionTurn, DiscussionUtterance,
} from "@shared/types";
import { discussionProblem, discussionSessionAgentId } from "@shared/types";
import { t } from "../i18n";
import { tauriApi } from "../ipc/tauriApi";
import { maybeSendOsNotification } from "../ipc/osNotify";
import { useAppStore } from "../store/appStore";
import { useDiscussionStore } from "./discussionStore";
import {
  closeDiscussionSessions, collectDiscussionUtterance, liveDiscussionSessions, openDiscussionSession, sendDiscussionTurn,
  useDiscussionSessionStore,
} from "./discussionSessionStore";
import { useRoleStore } from "./roleStore";

/** 사회자 지시 형식. 키는 언어와 무관하게 고정이다. */
export const MODERATOR_FORMAT = [
  '{"action":"next","speakers":[{"id":"<debater id>","question":"<question for them>"}]}',
  '{"action":"ask_user","question":"<question for the person>"}',
  '{"action":"wrap","summary":"<conclusion, reasons, rejected alternatives, open issues>"}',
].join("\n");

/** 끝나지 않은 토의(동시 토의는 하나만). 준비·끝남이 아니면 진행 중으로 본다. */
export function isActiveDiscussion(d: Pick<Discussion, "state">): boolean {
  return d.state !== "ready" && d.state !== "finished";
}

// ── 사회자 지시 ────────────────────────────────────────────────────────

export type ModeratorDirective =
  | { action: "next"; speakers: { agentId: string; question: string }[] }
  | { action: "ask_user"; question: string }
  | { action: "wrap"; summary: string };
/** 지시를 못 읽은 이유. 사회자에게 다시 낼 때 알려 준다(`discussion:run.problems.*`). */
export type DirectiveProblem =
  | "no-json" | "bad-action" | "no-speakers" | "unknown-speaker" | "moderator-speaker" | "duplicate-speaker"
  | "empty-question" | "empty-summary";

function jsonObjects(text: string): unknown[] {
  const candidates = [...text.matchAll(/```(?:json)?\s*([\s\S]*?)```/g)].map((m) => m[1]);
  candidates.push(text);
  const out: unknown[] = [];
  for (const raw of candidates) {
    const start = raw.indexOf("{"), end = raw.lastIndexOf("}");
    if (start < 0 || end <= start) continue;
    try { out.push(JSON.parse(raw.slice(start, end + 1))); } catch { /* 다음 후보 */ }
  }
  return out;
}
const str = (v: unknown) => (typeof v === "string" ? v.trim() : "");

/**
 * 사회자 발언에서 지시를 뽑는다. 산문 안이나 코드 블록 안의 JSON 객체 하나를 읽는다(마지막 응답 경로는
 * 답변 전체가 오므로). 지목은 토론자 id로 받고, id가 아니면 팀원 이름이 하나로 맞을 때만 받는다.
 */
export function parseModeratorDirective(
  text: string,
  d: Pick<Discussion, "participants" | "moderatorAgentId">,
  names: Readonly<Record<string, string>> = {},
): { directive: ModeratorDirective } | { problem: DirectiveProblem } {
  const object = jsonObjects(text).find((v): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v) && "action" in v);
  if (!object) return { problem: "no-json" };
  const action = str(object.action).toLowerCase();
  if (action === "ask_user") {
    const question = str(object.question);
    return question ? { directive: { action, question } } : { problem: "empty-question" };
  }
  if (action === "wrap") {
    const summary = str(object.summary);
    return summary ? { directive: { action, summary } } : { problem: "empty-summary" };
  }
  if (action !== "next") return { problem: "bad-action" };
  const list = Array.isArray(object.speakers) ? object.speakers : [];
  if (!list.length) return { problem: "no-speakers" };
  const debaters = d.participants.map((p) => p.agentId).filter((id) => id !== d.moderatorAgentId);
  const byName = (name: string) => {
    const key = name.replace(/^@/, "").trim().toLowerCase();
    const hits = d.participants.filter((p) => (names[p.agentId] ?? "").trim().toLowerCase() === key);
    return hits.length === 1 ? hits[0].agentId : undefined;
  };
  const speakers: { agentId: string; question: string }[] = [];
  for (const item of list) {
    const entry = item && typeof item === "object" ? item as Record<string, unknown> : {};
    const ref = str(entry.id) || str(entry.agentId) || str(entry.name) || str(item);
    const agentId = d.participants.some((p) => p.agentId === ref) ? ref : byName(ref);
    if (!agentId) return { problem: "unknown-speaker" };
    if (agentId === d.moderatorAgentId || !debaters.includes(agentId)) return { problem: "moderator-speaker" };
    if (speakers.some((s) => s.agentId === agentId)) return { problem: "duplicate-speaker" };
    const question = str(entry.question);
    if (!question) return { problem: "empty-question" };
    speakers.push({ agentId, question });
  }
  return { directive: { action: "next", speakers } };
}

// ── 차례 지시 ──────────────────────────────────────────────────────────

function memberName(agentId: string | undefined): string {
  return (agentId && useAppStore.getState().agents[agentId]?.name.trim()) || t("discussion:board.missingAgent");
}
function roleName(d: Discussion, agentId: string): string | undefined {
  const roleId = d.participants.find((p) => p.agentId === agentId)?.roleId;
  return roleId ? useRoleStore.getState().roles.find((r) => r.id === roleId)?.name.trim() || undefined : undefined;
}
function names(d: Discussion): Record<string, string> {
  return Object.fromEntries(d.participants.map((p) => [p.agentId, memberName(p.agentId)]));
}

function entryLabel(d: Discussion, e: DiscussionUtterance): string {
  if (e.kind === "answer") return t("discussion:run.label.answer");
  if (e.kind === "notice") return t("discussion:run.label.notice");
  const name = memberName(e.agentId);
  if (e.kind === "question") return t("discussion:run.label.question", { name });
  if (e.kind === "moderator") return t("discussion:run.label.moderator", { name });
  const role = e.agentId ? roleName(d, e.agentId) : undefined;
  return role ? `${name} · ${role}` : name;
}

/** 이 세션이 아직 못 본 기록. 같은 세션이면 자기 발언은 뺀다. 새 세션이면 처음부터 다 전한다. */
export function deltaEntries(d: Discussion, agentId: string, sessionId: string | undefined): DiscussionUtterance[] {
  const seen = d.run?.seen[agentId];
  const same = !!sessionId && seen?.sessionId === sessionId;
  return d.transcript.slice(same ? seen.count : 0).filter((e) => !same || e.agentId !== agentId);
}

export function deltaText(d: Discussion, agentId: string, sessionId: string | undefined): string {
  const entries = deltaEntries(d, agentId, sessionId);
  const fresh = !sessionId || d.run?.seen[agentId]?.sessionId !== sessionId;
  if (!entries.length) return fresh ? "" : t("discussion:run.prompt.noNew");
  const head = t(fresh ? "discussion:run.prompt.allHeader" : "discussion:run.prompt.newHeader");
  const body = entries.map((e) => `#### ${entryLabel(d, e)} (R${e.round})\n\n${e.text.trim()}`).join("\n\n");
  return `${head}\n\n${body}`;
}

function roster(d: Discussion): string {
  return d.participants.filter((p) => p.agentId !== d.moderatorAgentId).map((p) => {
    const role = roleName(d, p.agentId);
    return `- ${memberName(p.agentId)}${role ? ` (${role})` : ""}: \`${p.agentId}\``;
  }).join("\n");
}

/** 차례 하나의 지시. 앞에 델타, 뒤에 이번 차례에 할 일. */
export function turnText(d: Discussion, turn: DiscussionTurn, sessionId: string | undefined): string {
  const run = d.run!;
  let instruction: string;
  if (run.phase === "opening") instruction = t("discussion:run.prompt.opening");
  else if (run.phase === "speakers") instruction = t("discussion:run.prompt.speaker", { question: turn.question ?? "" });
  else instruction = t("discussion:run.prompt.moderator", { round: d.round, max: d.limits.maxRounds, format: MODERATOR_FORMAT, roster: roster(d) });
  return [deltaText(d, turn.agentId, sessionId), instruction].filter(Boolean).join("\n\n");
}

// ── 상태 쓰기 ──────────────────────────────────────────────────────────

function find(id: string): Discussion | undefined { return useDiscussionStore.getState().discussions.find((d) => d.id === id); }
function running(id: string): Discussion | undefined {
  const d = find(id);
  return d?.run && (d.state === "running" || d.state === "wrapping_up") ? d : undefined;
}
/** 최신 토의를 읽어 고친다. 없으면 아무것도 하지 않는다. */
function update(id: string, change: (d: Discussion) => Discussion): Discussion | undefined {
  const d = find(id);
  if (!d) return undefined;
  const next = { ...change(d), updatedAt: Date.now() };
  void useDiscussionStore.getState().commit(next);
  return next;
}
function updateRun(id: string, change: (run: DiscussionRun, d: Discussion) => DiscussionRun): void {
  update(id, (d) => (d.run ? { ...d, run: change(d.run, d) } : d));
}
function patchTurn(id: string, agentId: string, patch: Partial<DiscussionTurn>): void {
  updateRun(id, (run) => ({ ...run, turns: run.turns.map((turn) => (turn.agentId === agentId ? { ...turn, ...patch } : turn)) }));
}
function utterance(d: Discussion, kind: DiscussionUtterance["kind"], text: string, agentId?: string): DiscussionUtterance {
  return { at: Date.now(), round: d.round, kind, ...(agentId ? { agentId } : {}), text };
}
function pause(id: string, reason: DiscussionPauseReason, detail?: string): void {
  update(id, (d) => (d.run ? { ...d, state: "paused", run: { ...d.run, pauseReason: reason, ...(detail ? { pauseDetail: detail } : { pauseDetail: undefined }) } } : d));
}
function errorCode(error: unknown): string { return error instanceof Error ? error.message : String(error); }

// ── 사람 조작 ──────────────────────────────────────────────────────────

/** 시작을 막는 이유. 없으면 undefined. 문구는 `discussion:run.blocked.*`. */
export function discussionStartBlockedReason(
  d: Discussion,
  all: readonly Discussion[],
  agents: Readonly<Record<string, AgentProfile>>,
  observerEnabled: boolean,
): string | undefined {
  if (d.state !== "ready") return "discussion-not-ready";
  if (discussionProblem(d)) return "discussion-invalid";
  if (all.some((other) => other.id !== d.id && isActiveDiscussion(other))) return "discussion-busy";
  if (liveDiscussionSessions().some((s) => s.discussionId !== d.id)) return "discussion-busy";
  if (d.participants.some((p) => !agents[p.agentId])) return "discussion-missing-member";
  if (d.participants.some((p) => !discussionSessionAgentId(d.id, p.agentId))) return "discussion-invalid";
  if (!observerEnabled) return "discussion-observer-off";
  return undefined;
}

export function startDiscussion(id: string): string | undefined {
  const d = find(id);
  if (!d) return "discussion-not-ready";
  const app = useAppStore.getState();
  const blocked = discussionStartBlockedReason(d, useDiscussionStore.getState().discussions, app.agents, app.appSettings.observerEnabled);
  if (blocked) return blocked;
  const debaters = d.participants.filter((p) => p.agentId !== d.moderatorAgentId);
  const run: DiscussionRun = { phase: "opening", turns: debaters.map((p) => ({ agentId: p.agentId })), seen: {} };
  update(id, (latest) => ({ ...latest, state: "running", round: 1, transcript: [], startedAt: Date.now(), endedAt: undefined, run }));
  kick(id);
  return undefined;
}

/** 멈춤. 돌고 있는 턴은 끝까지 두고, 다음 차례를 넣지 않는다. */
export function pauseDiscussion(id: string): void {
  if (running(id)) pause(id, "user");
}

/**
 * 계속. 세션이 사라진 차례는 다시 보내고(세션을 새로 연다), 재촉도 다시 한 번 할 수 있게 한다.
 * 관찰 훅이 꺼져 있으면 턴 종료를 알 수 없어 이어 가지 않는다.
 */
export function resumeDiscussion(id: string): string | undefined {
  const d = find(id);
  if (!d?.run || d.state !== "paused") return undefined;
  if (!useAppStore.getState().appSettings.observerEnabled) return "discussion-observer-off";
  const live = new Map(liveDiscussionSessions().filter((s) => s.discussionId === id).map((s) => [s.participantAgentId, s.sessionId]));
  update(id, (latest) => {
    const run = latest.run!;
    const turns = run.turns.map((turn) => {
      if (turn.text !== undefined) return turn;
      const alive = turn.sessionId && live.get(turn.agentId) === turn.sessionId;
      return alive ? { ...turn, nudged: undefined } : { agentId: turn.agentId, ...(turn.question ? { question: turn.question } : {}) };
    });
    return { ...latest, state: run.phase === "user" ? "waiting_user" : run.phase === "wrap_up" ? "wrapping_up" : "running", run: { ...run, turns, pauseReason: undefined, pauseDetail: undefined } };
  });
  kick(id);
  return undefined;
}

/** 사회자 질문에 사람이 답한다. 답은 기록에 남고 다시 사회자 차례가 된다. */
export function answerDiscussion(id: string, text: string): boolean {
  const d = find(id);
  const answer = text.trim();
  if (!d?.run || d.state !== "waiting_user" || d.run.phase !== "user" || !answer) return false;
  update(id, (latest) => ({
    ...latest, state: "running", transcript: [...latest.transcript, utterance(latest, "answer", answer)],
    run: { ...latest.run!, phase: "moderator", question: undefined, turns: [{ agentId: latest.moderatorAgentId }] },
  }));
  kick(id);
  return true;
}

/** 끝내기. 어느 단계에서든 토의를 끝남으로 바꾸고 참가자 세션을 모두 닫는다. */
export async function endDiscussion(id: string): Promise<void> {
  const d = find(id);
  if (!d || !isActiveDiscussion(d)) return;
  await finish(id, "ended-by-user");
}

async function finish(id: string, notice?: string): Promise<void> {
  update(id, (d) => ({
    ...d, state: "finished", endedAt: Date.now(), run: undefined,
    transcript: notice ? [...d.transcript, utterance(d, "notice", notice)] : d.transcript,
  }));
  // 안 닫으면 다음 토의가 discussion-session-other로 막힌다.
  await closeDiscussionSessions(id);
}

// ── 루프 ──────────────────────────────────────────────────────────────

const inFlight = new Map<string, { again: boolean }>();
/** 토의 하나의 단계를 직렬로 돌린다. 도는 중에 다시 불리면 끝난 뒤 한 번 더 돈다. */
function kick(id: string): void {
  const flight = inFlight.get(id);
  if (flight) { flight.again = true; return; }
  const state = { again: false };
  inFlight.set(id, state);
  void (async () => {
    try {
      do { state.again = false; await stepDiscussion(id); } while (state.again && running(id));
    } catch (error) {
      console.warn("discussion: step failed", error);
      if (running(id)) pause(id, "send-failed", errorCode(error));
    } finally { inFlight.delete(id); }
  })();
}
export function isDiscussionStepping(id: string): boolean { return inFlight.has(id); }

/** 한 단계. 기다릴 것이 있으면 바로 끝나고, 신호가 오면 다시 불린다. 테스트가 직접 부른다. */
export async function stepDiscussion(id: string): Promise<void> {
  const d = running(id);
  if (!d?.run) return;
  switch (d.run.phase) {
    case "opening": return stepOpening(id);
    case "moderator": return stepModerator(id);
    case "speakers": return stepSpeakers(id);
    case "wrap_up": return finish(id);
    case "user": return;
  }
}

function sessionOf(d: Discussion, agentId: string) {
  const key = discussionSessionAgentId(d.id, agentId);
  return key ? useDiscussionSessionStore.getState().sessions[key] : undefined;
}
const isLive = (status: string | undefined) => status === "starting" || status === "running";

/**
 * 차례를 넣는다. 살아 있는 세션이면 델타와 함께 보내고(턴이 도는 중이면 기다린다), 없으면 첫 프롬프트에 실어 연다.
 * 넣었으면 그 시각을 since로 남기고, 이 세션이 본 기록을 지금 길이로 맞춘다.
 */
async function deliver(id: string, agentId: string): Promise<void> {
  const d = running(id);
  const turn = d?.run?.turns.find((x) => x.agentId === agentId);
  if (!d || !turn || turn.sentAt !== undefined) return;
  const session = sessionOf(d, agentId);
  let sessionId: string, sentAt: number;
  if (session && isLive(session.status)) {
    if (session.turn?.phase === "working" || session.turn?.phase === "waiting") return;
    try {
      ({ submittedAt: sentAt } = await sendDiscussionTurn(session.agentId, turnText(d, turn, session.sessionId)));
    } catch (error) {
      const code = errorCode(error);
      if (running(id)) pause(id, code === "work-send-human-input" ? "human-input" : "send-failed", code);
      return;
    }
    sessionId = session.sessionId;
  } else {
    const open = () => openDiscussionSession(d.id, agentId, { turn: turnText(d, turn, undefined), focus: false });
    let error = await open();
    // 렌더러만 다시 읽혀 네이티브 세션이 남아 있으면 버리고 다시 연다.
    if (error === "discussion-session-running") {
      const key = discussionSessionAgentId(d.id, agentId)!;
      await tauriApi.disposeSession(key).catch(() => undefined);
      error = await open();
    }
    const opened = sessionOf(d, agentId);
    if (error || !opened) {
      if (running(id)) pause(id, error === "work-send-human-input" ? "human-input" : "open-failed", error ?? "discussion-session-missing");
      return;
    }
    ({ sessionId, submittedAt: sentAt } = opened);
  }
  // 보내는 사이 토의가 끝났거나 단계가 바뀌었으면 기록만 맞추지 않는다.
  updateRun(id, (run, latest) => ({
    ...run,
    turns: run.turns.map((x) => (x.agentId === agentId ? { ...x, sessionId, sentAt } : x)),
    seen: { ...run.seen, [agentId]: { sessionId, count: latest.transcript.length } },
  }));
}

/** 보낸 차례 하나를 받는다. 턴이 끝나지 않았으면 null. 발언이 없으면 한 번 재촉하고, 또 없으면 멈춘다. */
async function receive(id: string, agentId: string): Promise<string | null> {
  const d = running(id);
  const turn = d?.run?.turns.find((x) => x.agentId === agentId);
  if (!d || !turn || turn.sentAt === undefined || turn.text !== undefined) return null;
  const session = sessionOf(d, agentId);
  if (!session || session.sessionId !== turn.sessionId || !isLive(session.status) || session.turn?.phase === "ended") {
    pause(id, "session-ended", memberName(agentId));
    return null;
  }
  if (session.turn?.phase !== "idle" || session.turn.at < turn.sentAt) return null;
  const got = await collectDiscussionUtterance(session.agentId, turn.sentAt).catch((error: unknown) => {
    console.warn("discussion: collect failed", error);
    return null;
  });
  if (!running(id)) return null;
  const text = got?.text.trim();
  if (text) return text;
  await nudge(id, agentId, t("discussion:run.prompt.nudge"), "no-utterance");
  return null;
}

/** 같은 차례를 한 번 더 요청한다. 이미 재촉했으면 멈춘다. */
async function nudge(id: string, agentId: string, text: string, reason: DiscussionPauseReason, detail?: string): Promise<void> {
  const d = running(id);
  const turn = d?.run?.turns.find((x) => x.agentId === agentId);
  if (!d || !turn) return;
  if (turn.nudged) return pause(id, reason, detail ?? memberName(agentId));
  const session = sessionOf(d, agentId);
  if (!session || !isLive(session.status)) return pause(id, "session-ended", memberName(agentId));
  try {
    const { submittedAt } = await sendDiscussionTurn(session.agentId, text);
    patchTurn(id, agentId, { sentAt: submittedAt, nudged: true });
  } catch (error) {
    const code = errorCode(error);
    if (running(id)) pause(id, code === "work-send-human-input" ? "human-input" : "send-failed", code);
  }
}

/** 동시 첫 라운드. 전원에게 한꺼번에 차례를 주고, 다 받으면 한꺼번에 공개한다. */
async function stepOpening(id: string): Promise<void> {
  const d = running(id)!;
  const unsent = d.run!.turns.filter((x) => x.sentAt === undefined).map((x) => x.agentId);
  await Promise.all(unsent.map((agentId) => deliver(id, agentId)));
  for (const turn of running(id)?.run?.turns ?? []) {
    if (turn.sentAt === undefined || turn.text !== undefined) continue;
    const text = await receive(id, turn.agentId);
    if (!running(id)) return;
    if (text !== null) patchTurn(id, turn.agentId, { text });
  }
  const latest = running(id);
  if (!latest?.run || latest.run.phase !== "opening" || latest.run.turns.some((x) => x.text === undefined)) return;
  update(id, (x) => ({
    ...x,
    transcript: [...x.transcript, ...x.run!.turns.map((turn) => utterance(x, "opening", turn.text!, turn.agentId))],
    run: { ...x.run!, phase: "moderator", turns: [{ agentId: x.moderatorAgentId }] },
  }));
  return stepDiscussion(id);
}

/** 사회자 차례. 지시를 받아 지목·질문·정리로 넘어간다. 못 읽으면 형식을 다시 알려 주고, 또 못 읽으면 멈춘다. */
async function stepModerator(id: string): Promise<void> {
  const d = running(id)!;
  const moderator = d.moderatorAgentId;
  const turn = d.run!.turns.find((x) => x.agentId === moderator);
  if (!turn) return pause(id, "moderator-invalid");
  if (turn.sentAt === undefined) return deliver(id, moderator);
  const text = await receive(id, moderator);
  if (text === null) return;
  const latest = running(id)!;
  const parsed = parseModeratorDirective(text, latest, names(latest));
  if ("problem" in parsed) {
    const problem = t(`discussion:run.problems.${parsed.problem}`);
    return nudge(id, moderator, t("discussion:run.prompt.invalid", { problem, format: MODERATOR_FORMAT }), "moderator-invalid", parsed.problem);
  }
  const directive = parsed.directive;
  if (directive.action === "next") {
    update(id, (x) => {
      const round = x.round + 1;
      const lines = directive.speakers.map((s) => `- ${memberName(s.agentId)}: ${s.question}`).join("\n");
      return {
        ...x, round,
        transcript: [...x.transcript, { ...utterance(x, "moderator", lines, moderator), round }],
        run: { ...x.run!, phase: "speakers", turns: directive.speakers.map((s) => ({ agentId: s.agentId, question: s.question })) },
      };
    });
    return stepDiscussion(id);
  }
  if (directive.action === "ask_user") {
    update(id, (x) => ({
      ...x, state: "waiting_user",
      transcript: [...x.transcript, utterance(x, "question", directive.question, moderator)],
      run: { ...x.run!, phase: "user", turns: [], question: directive.question },
    }));
    void maybeSendOsNotification(t("discussion:run.notify.question", { topic: latest.topic }), directive.question);
    return;
  }
  update(id, (x) => ({
    ...x, state: "wrapping_up",
    transcript: [...x.transcript, utterance(x, "moderator", directive.summary, moderator)],
    run: { ...x.run!, phase: "wrap_up", turns: [] },
  }));
  return stepDiscussion(id);
}

/** 지목된 토론자가 순서대로 말한다. 뒤 사람은 앞 사람의 발언을 델타로 받는다. 다 말하면 사회자 차례. */
async function stepSpeakers(id: string): Promise<void> {
  const d = running(id)!;
  const turn = d.run!.turns.find((x) => x.text === undefined);
  if (!turn) {
    update(id, (x) => ({ ...x, run: { ...x.run!, phase: "moderator", turns: [{ agentId: x.moderatorAgentId }] } }));
    return stepDiscussion(id);
  }
  if (turn.sentAt === undefined) return deliver(id, turn.agentId);
  const text = await receive(id, turn.agentId);
  if (text === null) return;
  update(id, (x) => ({
    ...x,
    transcript: [...x.transcript, utterance(x, "statement", text, turn.agentId)],
    run: { ...x.run!, turns: x.run!.turns.map((y) => (y.agentId === turn.agentId ? { ...y, text } : y)) },
  }));
  return stepDiscussion(id);
}

// ── 설치 ──────────────────────────────────────────────────────────────

/**
 * 앱을 열 때: 토의 세션은 재시작 때 버려지므로 진행 중·정리 중이던 토의는 멈춤(restarted)으로 돌린다.
 * 사람 답을 기다리던 토의는 그대로 둔다(답하면 세션을 다시 연다).
 */
export function restoreDiscussionsAfterRestart(): void {
  for (const d of useDiscussionStore.getState().discussions) {
    if (!d.run || (d.state !== "running" && d.state !== "wrapping_up")) continue;
    update(d.id, (x) => ({ ...x, state: "paused", run: { ...x.run!, pauseReason: "restarted", pauseDetail: undefined } }));
  }
}

/** 세션 턴 단계·토의 변화마다 진행 중인 토의를 깨운다. 할 일이 없으면 단계가 바로 끝난다. */
export function installDiscussionRunner(): () => void {
  let installed = true;
  // 복원 전에 깨우면 재시작 전 토의가 "세션이 끝남"으로 멈춘다. 복원한 뒤부터 깨운다.
  let restored = false;
  const wake = () => {
    if (!installed || !restored) return;
    for (const d of useDiscussionStore.getState().discussions) if (d.run && (d.state === "running" || d.state === "wrapping_up")) kick(d.id);
  };
  void useDiscussionStore.getState().ensureLoaded().then(() => {
    if (!installed) return;
    restoreDiscussionsAfterRestart();
    restored = true;
    // 차례 지시의 역할 이름에 쓴다. 없어도 진행은 된다.
    if (useDiscussionStore.getState().discussions.length) void useRoleStore.getState().ensureLoaded();
  });
  const offSessions = useDiscussionSessionStore.subscribe((s, prev) => { if (s.sessions !== prev.sessions) wake(); });
  const offDiscussions = useDiscussionStore.subscribe((s, prev) => { if (s.discussions !== prev.discussions) wake(); });
  return () => { installed = false; offSessions(); offDiscussions(); };
}

/** 테스트 전용. */
export function resetDiscussionRunForTest(): void { inFlight.clear(); }
