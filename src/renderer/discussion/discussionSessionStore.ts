// 토의 참가자 세션(전용 탭). 참가자마다 캐릭터 본 터미널이 아닌 `discuss-<토의 id>.<참가자>` 세션을 연다.
// 탭·상태·턴 단계는 여기만 쥔다(appStore.sessions·recentAgentIds에 넣지 않는다). 진행(누가 언제 말하나)은
// 실행기가 이 스토어의 열기·차례 보내기·발언 수집·턴 단계를 써서 정한다.
import { create } from "zustand";
import {
  DISCUSSION_MAX_SESSIONS,
  discussionSessionAgentId,
  isDiscussionSessionId,
  notificationType,
  parseDiscussionSessionAgentId,
} from "@shared/types";
import type {
  CollectedUtterance,
  DiscussionCollection,
  DiscussionReadOnly,
  SessionStateEvent,
  SessionStatus,
  WorkAgent,
} from "@shared/types";
import { t } from "../i18n";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import { terminalRegistry } from "../terminal/TerminalRegistry";
import { sendErrorText } from "../workpanels/sendErrorText";
import { useDiscussionStore } from "./discussionStore";
import { useRoleStore } from "./roleStore";

/**
 * 참가자 세션의 턴 단계. 캐릭터의 workMonitor·Rust TurnTracker와 같은 신호·규칙이다.
 * working = 턴이 돈다, waiting = 권한·질문으로 사람 대기, idle = 턴이 끝났다, ended = 세션이 끝났다.
 */
export type DiscussionTurnPhase = "working" | "waiting" | "idle" | "ended";

export interface DiscussionSession {
  /** 토의 세션 agentId. */
  agentId: string;
  discussionId: string;
  /** 참가자(캐릭터) agentId. */
  participantAgentId: string;
  sessionId: string;
  cli: WorkAgent["cli"];
  status: SessionStatus;
  readOnly: DiscussionReadOnly;
  collection: DiscussionCollection;
  /** 관찰 훅이 붙었는가. 거짓이면 턴 단계가 바뀌지 않는다. */
  observed: boolean;
  openedAt: number;
  /** 마지막 턴 단계와 그 시각(관찰 신호의 at). 첫 신호 전에는 없다. */
  turn?: { phase: DiscussionTurnPhase; at: number };
}

interface DiscussionSessionState {
  sessions: Record<string, DiscussionSession>;
  /** 탭 순서(연 순서). */
  order: string[];
  /** 여는 중인 토의 세션 agentId. */
  opening: Record<string, true>;
}
export const useDiscussionSessionStore = create<DiscussionSessionState>(() => ({ sessions: {}, order: [], opening: {} }));

function isLive(status: SessionStatus | undefined): boolean { return status === "starting" || status === "running"; }
export function liveDiscussionSessions(): DiscussionSession[] {
  const { sessions, order } = useDiscussionSessionStore.getState();
  return order.map((id) => sessions[id]).filter((s) => s && isLive(s.status));
}
export function liveDiscussionSessionCount(): number { return liveDiscussionSessions().length; }

/** 열기를 막는 이유(네이티브 `limit_problem`과 같은 규칙). 이미 살아 있으면 그 탭으로 가므로 막지 않는다. */
export function discussionSessionBlockedReason(discussionId: string, participantAgentId: string): string | undefined {
  const agentId = discussionSessionAgentId(discussionId, participantAgentId);
  if (!agentId) return "discussion-session-invalid-id";
  const { sessions, opening } = useDiscussionSessionStore.getState();
  if (opening[agentId]) return "discussion-session-busy";
  if (isLive(sessions[agentId]?.status)) return undefined;
  const live = liveDiscussionSessions();
  if (live.some((s) => s.discussionId !== discussionId)) return "discussion-session-other";
  if (live.length >= DISCUSSION_MAX_SESSIONS) return "discussion-session-limit";
  return undefined;
}

const ERROR_KEYS: Record<string, string> = {
  "discussion-session-busy": "busy",
  "discussion-session-running": "running",
  "discussion-session-limit": "limit",
  "discussion-session-other": "other",
  "discussion-session-invalid-id": "invalidId",
  "discussion-session-missing": "missing",
  "discussion-session-finished": "finished",
  "discussion-session-not-participant": "notParticipant",
  "discussion-session-invalid-cwd": "invalidCwd",
};
/** 토의 세션 오류 문구. 토의 고유 코드가 아니면 보내기 오류 문구를 쓴다. */
export function discussionSessionErrorText(error: string): string {
  const key = ERROR_KEYS[error.split(":")[0]];
  if (key === "limit") return t("discussion:session.errors.limit", { max: DISCUSSION_MAX_SESSIONS });
  return key ? t(`discussion:session.errors.${key}`) : sendErrorText(error);
}

/** 탭 이름: 캐릭터 이름 · 역할 이름. 역할이 없으면 사회자는 "사회자", 토론자는 "역할 없음"을 쓴다. */
export function discussionSessionTitle(session: Pick<DiscussionSession, "discussionId" | "participantAgentId">): string {
  const name = useAppStore.getState().agents[session.participantAgentId]?.name || t("discussion:board.missingAgent");
  const discussion = useDiscussionStore.getState().discussions.find((d) => d.id === session.discussionId);
  const roleId = discussion?.participants.find((p) => p.agentId === session.participantAgentId)?.roleId;
  const role = roleId ? useRoleStore.getState().roles.find((r) => r.id === roleId)?.name.trim() : undefined;
  const fallback = discussion?.moderatorAgentId === session.participantAgentId ? t("discussion:detail.moderator") : t("discussion:board.noRole");
  return `${name} · ${role || fallback}`;
}
/** 탭 툴팁: 주제와 사회자 여부. */
export function discussionSessionTooltip(session: Pick<DiscussionSession, "discussionId" | "participantAgentId">): string {
  const discussion = useDiscussionStore.getState().discussions.find((d) => d.id === session.discussionId);
  const topic = discussion?.topic.trim() || t("discussion:session.unknownTopic");
  return discussion?.moderatorAgentId === session.participantAgentId
    ? t("discussion:session.tooltipModerator", { topic })
    : t("discussion:session.tooltip", { topic });
}

/** 새 세션의 첫 크기. 지금 보이는(또는 최근) 터미널 크기를 빌린다. 탭을 열면 다시 맞춘다. */
function initialSize(): { cols?: number; rows?: number } {
  const { activeTerminalAgentId, recentAgentIds, sessions } = useAppStore.getState();
  const fitted = activeTerminalAgentId && terminalRegistry.get(activeTerminalAgentId)?.term;
  if (fitted && fitted.cols > 0 && fitted.rows > 0) return { cols: fitted.cols, rows: fitted.rows };
  const source = sessions[recentAgentIds[0] ?? ""];
  return source ? { cols: source.cols, rows: source.rows } : {};
}

function forget(agentId: string): void {
  terminalRegistry.destroy(agentId);
  latest.delete(agentId);
  useDiscussionSessionStore.setState((s) => {
    const sessions = { ...s.sessions };
    delete sessions[agentId];
    return { sessions, order: s.order.filter((id) => id !== agentId) };
  });
}

export interface OpenDiscussionSessionOptions {
  /** 첫 프롬프트 끝에 붙일 이번 차례 지시. */
  turn?: string;
  /** 연 뒤 그 탭을 보여 줄지. 실행기가 여러 명을 한꺼번에 열 때는 끈다. 기본 true. */
  focus?: boolean;
}

/**
 * 참가자 세션을 연다. 이미 살아 있으면 그대로 둔다(focus면 그 탭으로 간다).
 * 실패하면 오류 코드를 돌려준다(탭은 생기지 않는다). 캐릭터 본 터미널과 업무 상태는 건드리지 않는다.
 */
export async function openDiscussionSession(
  discussionId: string,
  participantAgentId: string,
  options: OpenDiscussionSessionOptions = {},
): Promise<string | undefined> {
  const agentId = discussionSessionAgentId(discussionId, participantAgentId);
  if (!agentId) return "discussion-session-invalid-id";
  const focus = options.focus ?? true;
  const existing = useDiscussionSessionStore.getState().sessions[agentId];
  if (existing && isLive(existing.status)) {
    if (focus) useAppStore.getState().openTerminal(agentId);
    return undefined;
  }
  const blocked = discussionSessionBlockedReason(discussionId, participantAgentId);
  if (blocked) return blocked;
  const discussion = useDiscussionStore.getState().discussions.find((d) => d.id === discussionId);
  const participant = discussion?.participants.find((p) => p.agentId === participantAgentId);
  if (!discussion) return "discussion-session-missing";
  if (!participant) return "discussion-session-not-participant";
  useDiscussionSessionStore.setState((s) => ({ opening: { ...s.opening, [agentId]: true } }));
  // 끝난 세션을 다시 열면 새 화면에서 시작한다. 탭을 열기 전에 출력을 구독해 첫 화면을 놓치지 않는다.
  if (existing) forget(agentId);
  terminalRegistry.ensure(agentId);
  try {
    const result = await tauriApi.openDiscussionSession({
      discussionId, agentId: participantAgentId, ...initialSize(),
      ...(options.turn ? { turn: options.turn } : {}),
    });
    const seen = latest.get(agentId);
    const status = seen?.sessionId === result.sessionId ? seen.status : "running";
    const session: DiscussionSession = {
      agentId, discussionId, participantAgentId, sessionId: result.sessionId, cli: participant.agent.cli, status,
      readOnly: result.readOnly, collection: result.collection, observed: result.observed, openedAt: Date.now(),
      ...(seen?.sessionId === result.sessionId && seen.turn ? { turn: seen.turn } : {}),
    };
    useDiscussionSessionStore.setState((s) => ({
      sessions: { ...s.sessions, [agentId]: session },
      order: [...s.order.filter((id) => id !== agentId), agentId],
    }));
    if (focus) useAppStore.getState().openTerminal(agentId);
    return undefined;
  } catch (error) {
    terminalRegistry.destroy(agentId);
    return error instanceof Error ? error.message : String(error);
  } finally {
    useDiscussionSessionStore.setState((s) => { const opening = { ...s.opening }; delete opening[agentId]; return { opening }; });
  }
}

/** 탭 닫기. CLI가 살아 있으면 확인창을 먼저 띄운다. */
export function requestCloseDiscussionSession(agentId: string): void {
  const session = useDiscussionSessionStore.getState().sessions[agentId];
  if (!session) return;
  if (isLive(session.status)) useAppStore.getState().openModal({ kind: "confirm-close-discussion-session", agentId });
  else void closeDiscussionSession(agentId);
}

/** 세션을 끝낸다. 세션을 버리고 탭을 없앤다. 보던 탭이면 이웃 토의 탭, 없으면 최근 캐릭터 탭으로 옮긴다. */
export async function closeDiscussionSession(agentId: string): Promise<void> {
  const { sessions, order } = useDiscussionSessionStore.getState();
  const session = sessions[agentId];
  if (!session) return;
  const app = useAppStore.getState();
  if (app.activeTerminalAgentId === agentId) {
    const idx = order.indexOf(agentId);
    const next = order[idx + 1] ?? order[idx - 1] ?? app.recentAgentIds[0];
    if (next) app.openTerminal(next);
    else app.closeTerminal();
  }
  forget(agentId);
  if (isLive(session.status)) {
    try { await tauriApi.disposeSession(agentId); } catch (err) { console.warn(`discussion: dispose failed for ${agentId}`, err); }
  }
}

/** 한 토의의 참가자 세션을 모두 끝낸다(토의를 마칠 때). */
export async function closeDiscussionSessions(discussionId: string): Promise<void> {
  const ids = useDiscussionSessionStore.getState().order.filter((id) => parseDiscussionSessionAgentId(id)?.discussionId === discussionId);
  for (const id of ids) await closeDiscussionSession(id);
}

/** 차례 지시 하나를 넣는다. 돌려준 submittedAt이 발언 수집의 since다. */
export async function sendDiscussionTurn(agentId: string, text: string): Promise<{ submittedAt: number }> {
  const session = useDiscussionSessionStore.getState().sessions[agentId];
  if (!session || !isLive(session.status)) throw new Error("work-send-session-changed");
  const { submittedAt } = await tauriApi.sendDiscussionTurn({ agentId, sessionId: session.sessionId, text });
  return { submittedAt };
}

/** 한 차례의 발언. 제출이 먼저고, 없으면 마지막 응답(claude·codex)이다. 없으면 null. */
export async function collectDiscussionUtterance(agentId: string, since: number): Promise<CollectedUtterance | null> {
  const session = useDiscussionSessionStore.getState().sessions[agentId];
  if (!session) return null;
  return await tauriApi.collectDiscussionUtterance({ agentId, sessionId: session.sessionId, cli: session.cli, since });
}

/** 마지막 세션 상태·턴 단계(탭이 생기기 전에 온 것도). 열기 결과의 sessionId와 맞으면 그 값으로 시작한다. */
const latest = new Map<string, { sessionId: string; status: SessionStatus; turn?: DiscussionSession["turn"] }>();

export function applyDiscussionSessionState(e: SessionStateEvent): void {
  const status: SessionStatus = e.state === "disposed" ? "exited" : e.state;
  const prev = latest.get(e.agentId);
  latest.set(e.agentId, { ...(prev?.sessionId === e.sessionId ? prev : {}), sessionId: e.sessionId, status });
  useDiscussionSessionStore.setState((s) => {
    const session = s.sessions[e.agentId];
    if (!session || session.sessionId !== e.sessionId || session.status === status) return s;
    return { sessions: { ...s.sessions, [e.agentId]: { ...session, status } } };
  });
  if (status === "exited") applyDiscussionTurn(e.agentId, e.sessionId, "ended", e.at);
}

/** 턴 단계 갱신. 늦게 온 옛 신호는 버리고, 턴이 끝난 뒤의 질문 신호(Claude idle_prompt)는 무시한다. */
export function applyDiscussionTurn(agentId: string, sessionId: string, phase: DiscussionTurnPhase, at: number): void {
  const prev = latest.get(agentId);
  const base = prev?.sessionId === sessionId ? prev : { sessionId, status: "running" as SessionStatus };
  const old = base.turn;
  if (old && old.at > at) return;
  if (phase === "waiting" && (old?.phase === "idle" || old?.phase === "ended")) return;
  if (old?.phase === "ended" && phase !== "ended") return;
  const turn = { phase, at };
  latest.set(agentId, { ...base, turn });
  useDiscussionSessionStore.setState((s) => {
    const session = s.sessions[agentId];
    if (!session || session.sessionId !== sessionId) return s;
    return { sessions: { ...s.sessions, [agentId]: { ...session, turn } } };
  });
}

/** 토의 세션 상태·턴 신호를 받는다. 캐릭터 중심 스토어는 이 접두사를 무시한다(sessionBridge 등). */
export function installDiscussionSessionTracking(): () => void {
  const offState = tauriApi.onSessionState((e) => { if (isDiscussionSessionId(e.agentId)) applyDiscussionSessionState(e); });
  const offActivity = tauriApi.onActivity((e) => {
    if (!isDiscussionSessionId(e.agentId)) return;
    if (e.kind === "prompt" || e.kind === "tool" || e.kind === "resume") applyDiscussionTurn(e.agentId, e.sessionId, "working", e.at);
  });
  const offNotification = tauriApi.onNotification((e) => {
    if (!isDiscussionSessionId(e.agentId)) return;
    const type = notificationType(e.source);
    if (type === "question") applyDiscussionTurn(e.agentId, e.sessionId, "waiting", e.at);
    else if (type === "done") applyDiscussionTurn(e.agentId, e.sessionId, "idle", e.at);
  });
  const offUsage = tauriApi.onTurnUsage((e) => {
    if (isDiscussionSessionId(e.agentId) && !e.partial) applyDiscussionTurn(e.agentId, e.sessionId, "idle", e.at);
  });
  return () => { offState(); offActivity(); offNotification(); offUsage(); };
}

/** 테스트 전용. */
export function resetDiscussionSessionStoreForTest(): void {
  latest.clear();
  useDiscussionSessionStore.setState({ sessions: {}, order: [], opening: {} });
}
