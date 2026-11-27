// src/renderer/discussion/remoteDiscussion.ts
//
// 머신 연동 호스트: 다른 머신 캐릭터(`기기id:캐릭터id`) 참가자의 토의 세션. 진행·결정 메모는 이 머신 실행기
// (`discussionRun.ts`)가 쥐고, 그 머신에는 세션 열기·차례 보내기·발언 수집·닫기만 넘긴다(Rust `link::discuss`).
// 그 머신이 참가자 세션을 자기 폴더(같은 저장소 클론, 없으면 캐릭터 시작 폴더)에서 열고, `office-discuss say`
// 제출이나 마지막 응답을 발언으로 돌려준다.
//
// - 세션은 탭이 아니다(터미널 화면은 보내지 않는다). 이 스토어에만 있고 `discussionSessionStore`의 탭 목록에 없다.
// - 턴 단계·살아 있는지·턴 토큰은 몇 초마다 그 머신에 묻는다(`discuss-state`). 시각은 그 머신 시계다 — 차례를 보낸
//   시각(submittedAt)도 그 머신 것이라 같은 시계끼리 비교한다.
// - 머신이 끊기면 `offline`으로 둔다. 실행기는 그 차례에서 멈추고(link-offline) 다시 붙으면 스스로 이어 간다.
// - 원격 세션 key(`discuss-<토의 id>.<기기id:캐릭터id>`)는 이 머신 안에서만 쓴다. `:` 때문에 Rust 토의 세션 id
//   형식이 아니어서 이 머신 네이티브 세션·비용 경로와 섞이지 않는다.
import { create } from "zustand";
import {
  DISCUSSION_SESSION_PREFIX,
  type CollectedUtterance,
  type Discussion,
  type LinkDiscussionState,
  type SessionEventTokens,
} from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { useRemoteFloors } from "../link/remoteFloors";
import type { DiscussionSession } from "./discussionSessionStore";
import { useRoleStore } from "./roleStore";
import { isRemoteParticipantId, participantName, remoteDeviceId, remoteParticipant } from "./participants";

export interface RemoteDiscussionSession extends DiscussionSession {
  deviceId: string;
  /** 읽은 마지막 턴 토큰 seq. */
  usageSeq: number;
  /** 마지막으로 물을 때 그 머신이 끊겨 있었다. */
  offline?: boolean;
  /** 그 머신이 연 작업 폴더(없으면 홈). 화면 안내용. */
  cwd?: string | null;
}

interface RemoteDiscussionState {
  sessions: Record<string, RemoteDiscussionSession>;
  opening: Record<string, true>;
}
export const useRemoteDiscussionStore = create<RemoteDiscussionState>(() => ({ sessions: {}, opening: {} }));

/** 원격 참가자 세션 key. 원격 참가자 id가 아니면 null. */
export function remoteDiscussionKey(discussionId: string, participantAgentId: string): string | null {
  return isRemoteParticipantId(participantAgentId) ? `${DISCUSSION_SESSION_PREFIX}${discussionId}.${participantAgentId}` : null;
}
export function remoteDiscussionSession(discussionId: string, participantAgentId: string): RemoteDiscussionSession | undefined {
  const key = remoteDiscussionKey(discussionId, participantAgentId);
  return key ? useRemoteDiscussionStore.getState().sessions[key] : undefined;
}
export function liveRemoteDiscussionSessions(): RemoteDiscussionSession[] {
  return Object.values(useRemoteDiscussionStore.getState().sessions).filter((s) => s.status === "running");
}
/** 그 원격 참가자의 머신이 지금 붙어 있는가. */
export function remoteParticipantOnline(agentId: string): boolean {
  return !!remoteParticipant(agentId)?.machine.online;
}

function errorCode(error: unknown): string { return error instanceof Error ? error.message : String(error); }
function setSession(key: string, patch: Partial<RemoteDiscussionSession>): void {
  useRemoteDiscussionStore.setState((s) => {
    const old = s.sessions[key];
    return old ? { sessions: { ...s.sessions, [key]: { ...old, ...patch } } } : s;
  });
}
function forget(key: string): void {
  useRemoteDiscussionStore.setState((s) => {
    const sessions = { ...s.sessions };
    delete sessions[key];
    return { sessions };
  });
}

/**
 * 그 머신에 보낼 토의: 기록·진행 상태는 빼고(첫 프롬프트에 필요 없다 — 차례 지시에 기록을 싣는다) 참가자 이름과
 * 쓰는 역할 카드를 붙인다. 그 머신은 이 머신 프로필·역할을 모른다.
 */
export function remoteOpenPayload(d: Discussion, participantAgentId: string, turn: string, lang?: string): Record<string, unknown> {
  const { transcript: _t, run: _r, decisionMemoPath: _m, ...rest } = d;
  const used = new Set(d.participants.map((p) => p.roleId).filter(Boolean));
  const members = Object.fromEntries(d.participants.map((p) => [p.agentId, participantName(p.agentId) ?? ""]));
  return {
    discussionId: d.id,
    participant: participantAgentId,
    discussion: { ...rest, transcript: [] },
    members,
    roles: useRoleStore.getState().roles.filter((r) => used.has(r.id)),
    turn,
    ...(lang ? { lang } : {}),
  };
}

/** 그 머신에 참가자 세션을 연다(첫 프롬프트에 차례를 싣는다). 실패하면 오류 코드. */
export async function openRemoteDiscussionSession(d: Discussion, participantAgentId: string, options: { turn: string; lang?: string }): Promise<string | undefined> {
  const key = remoteDiscussionKey(d.id, participantAgentId);
  if (!key) return "discussion-session-invalid-id";
  const { sessions, opening } = useRemoteDiscussionStore.getState();
  if (opening[key]) return "discussion-session-busy";
  if (sessions[key]?.status === "running") return undefined;
  const participant = d.participants.find((p) => p.agentId === participantAgentId);
  if (!participant) return "discussion-session-not-participant";
  const found = remoteParticipant(participantAgentId);
  if (!found) return "discussion-missing-member";
  if (!found.machine.online) return "link-device-offline";
  useRemoteDiscussionStore.setState((s) => ({ opening: { ...s.opening, [key]: true } }));
  try {
    const result = await tauriApi.linkWork(participantAgentId, "discuss-open", remoteOpenPayload(d, participantAgentId, options.turn, options.lang)) as {
      sessionId: string; submittedAt: number; collection: DiscussionSession["collection"]; observed: boolean; cwd?: string | null;
    };
    const session: RemoteDiscussionSession = {
      agentId: key, discussionId: d.id, participantAgentId, sessionId: result.sessionId, cli: participant.agent.cli, status: "running",
      collection: result.collection, observed: result.observed, openedAt: Date.now(), submittedAt: result.submittedAt,
      deviceId: found.machine.deviceId, usageSeq: 0, cwd: result.cwd ?? null,
    };
    useRemoteDiscussionStore.setState((s) => ({ sessions: { ...s.sessions, [key]: session } }));
    return undefined;
  } catch (error) {
    return errorCode(error);
  } finally {
    useRemoteDiscussionStore.setState((s) => { const opening = { ...s.opening }; delete opening[key]; return { opening }; });
  }
}

function sessionArgs(session: RemoteDiscussionSession): Record<string, unknown> {
  return { discussionId: session.discussionId, sessionId: session.sessionId };
}

/** 차례 지시 하나. 돌려준 submittedAt(그 머신 시계)이 발언 수집의 since다. */
export async function sendRemoteDiscussionTurn(key: string, text: string): Promise<{ submittedAt: number }> {
  const session = useRemoteDiscussionStore.getState().sessions[key];
  if (!session || session.status !== "running") throw new Error("work-send-session-changed");
  const result = await tauriApi.linkWork(session.participantAgentId, "discuss-turn", { ...sessionArgs(session), text }) as { submittedAt: number };
  return { submittedAt: result.submittedAt };
}

export async function collectRemoteDiscussionUtterance(key: string, since: number): Promise<CollectedUtterance | null> {
  const session = useRemoteDiscussionStore.getState().sessions[key];
  if (!session) return null;
  return await tauriApi.linkWork(session.participantAgentId, "discuss-collect", { ...sessionArgs(session), since }) as CollectedUtterance | null;
}

/** 그 머신 세션을 끝낸다(끊겨 있으면 그 머신에 남는다 — 그 머신 탭에서 닫을 수 있다). */
export async function closeRemoteDiscussionSession(key: string): Promise<void> {
  const session = useRemoteDiscussionStore.getState().sessions[key];
  if (!session) return;
  forget(key);
  await tauriApi.linkWork(session.participantAgentId, "discuss-close", sessionArgs(session)).catch(() => undefined);
}
export async function closeRemoteDiscussionSessions(discussionId: string): Promise<void> {
  const keys = Object.values(useRemoteDiscussionStore.getState().sessions).filter((s) => s.discussionId === discussionId).map((s) => s.agentId);
  await Promise.all(keys.map(closeRemoteDiscussionSession));
}

const PHASES: Record<NonNullable<LinkDiscussionState["turn"]>["phase"], NonNullable<DiscussionSession["turn"]>["phase"]> = {
  working: "working", "needs-human": "waiting", idle: "idle", ended: "ended",
};

/** 물어 온 그 머신 세션 상태를 반영한다. 턴 토큰마다 `onUsage`를 부른다. */
export function applyRemoteDiscussionState(
  key: string,
  state: LinkDiscussionState,
  onUsage?: (discussionId: string, u: { tokens: SessionEventTokens; at: number; partial: boolean }) => void,
): void {
  const session = useRemoteDiscussionStore.getState().sessions[key];
  if (!session) return;
  const fresh = state.usage.filter((u) => u.seq > session.usageSeq);
  const patch: Partial<RemoteDiscussionSession> = { offline: false };
  if (!state.alive && session.status === "running") patch.status = "exited";
  const next = state.turn ? { phase: PHASES[state.turn.phase], at: state.turn.at } : undefined;
  const old = session.turn;
  // 턴 단계는 discussionSessionStore.applyDiscussionTurn과 같은 규칙: 옛 신호·끝난 뒤의 질문 신호는 버린다.
  const accept = next && !(old && old.at > next.at) && !(next.phase === "waiting" && (old?.phase === "idle" || old?.phase === "ended"))
    && !(old?.phase === "ended" && next.phase !== "ended");
  if (accept && (old?.phase !== next.phase || old.at !== next.at)) patch.turn = next;
  if (!state.alive) patch.turn = { phase: "ended", at: Math.max(old?.at ?? 0, next?.at ?? 0) };
  if (fresh.length) patch.usageSeq = Math.max(...fresh.map((u) => u.seq));
  const changed = patch.status !== undefined || patch.turn !== undefined || patch.usageSeq !== undefined || session.offline;
  if (changed) setSession(key, patch);
  for (const u of fresh) onUsage?.(session.discussionId, u);
}

/** 그 머신이 끊겼다. 세션은 그 머신에 남아 있을 수 있어 상태는 그대로 두고 표시만 한다. */
function markOffline(key: string): void {
  if (!useRemoteDiscussionStore.getState().sessions[key]?.offline) setSession(key, { offline: true });
}

/** 상태를 묻는 주기. 턴 종료를 이만큼 늦게 안다. */
export const REMOTE_POLL_MS = 2_000;

/** 살아 있는 원격 세션마다 한 번 묻는다. 테스트가 직접 부른다. */
export async function pollRemoteDiscussionSessions(
  onUsage?: (discussionId: string, u: { tokens: SessionEventTokens; at: number; partial: boolean }) => void,
): Promise<void> {
  const live = liveRemoteDiscussionSessions();
  await Promise.all(live.map(async (session) => {
    const online = remoteParticipantOnline(session.participantAgentId);
    if (!online) { markOffline(session.agentId); return; }
    try {
      const state = await tauriApi.linkWork(session.participantAgentId, "discuss-state", { ...sessionArgs(session), after: session.usageSeq }) as LinkDiscussionState;
      // 묻는 사이 닫았거나 다시 열었으면 버린다.
      if (useRemoteDiscussionStore.getState().sessions[session.agentId]?.sessionId !== session.sessionId) return;
      applyRemoteDiscussionState(session.agentId, state, onUsage);
    } catch (error) {
      if (errorCode(error) === "link-device-offline") markOffline(session.agentId);
    }
  }));
}

/** 원격 세션 상태를 계속 묻는다(실행기가 건다). */
export function installRemoteDiscussionPolling(
  onUsage: (discussionId: string, u: { tokens: SessionEventTokens; at: number; partial: boolean }) => void,
): () => void {
  let busy = false;
  const tick = () => {
    if (busy || !liveRemoteDiscussionSessions().length) return;
    busy = true;
    void pollRemoteDiscussionSessions(onUsage).finally(() => { busy = false; });
  };
  const timer = setInterval(tick, REMOTE_POLL_MS);
  // 머신이 끊기거나 다시 붙으면 바로 본다.
  const off = useRemoteFloors.subscribe((s, prev) => { if (s.office !== prev.office) tick(); });
  return () => { clearInterval(timer); off(); };
}

/** 기기 id(실행기 멈춤 안내에 머신 이름을 쓴다). */
export function remoteMachineName(agentId: string): string {
  return remoteParticipant(agentId)?.machine.name ?? remoteDeviceId(agentId);
}

/** 테스트 전용. */
export function resetRemoteDiscussionForTest(): void {
  useRemoteDiscussionStore.setState({ sessions: {}, opening: {} });
}
