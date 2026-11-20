import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ActivityEvent, Discussion, DiscussionSessionRequest, NotificationEvent, SessionStateEvent, TurnUsageEvent,
} from "@shared/types";
import type { CollectedUtterance, DiscussionCollectRequest, DiscussionSessionResult } from "@shared/types";
import requestFixture from "@shared/contract-fixtures/open-discussion-session-request.json";
import resultFixture from "@shared/contract-fixtures/open-discussion-session-result.json";
import collectFixture from "@shared/contract-fixtures/collect-discussion-utterance-request.json";
import utteranceFixture from "@shared/contract-fixtures/collected-utterance.json";
import { DISCUSSION_MAX_SESSIONS, discussionCollectionMode, discussionSessionAgentId, parseDiscussionSessionAgentId } from "@shared/types";
const { api, registry, cbs } = vi.hoisted(() => ({
  api: { openDiscussionSession: vi.fn(), disposeSession: vi.fn(), sendDiscussionTurn: vi.fn(), collectDiscussionUtterance: vi.fn() },
  registry: { ensure: vi.fn(), destroy: vi.fn(), get: vi.fn(), has: vi.fn() },
  cbs: {
    state: new Set<(e: SessionStateEvent) => void>(),
    activity: new Set<(e: ActivityEvent) => void>(),
    notification: new Set<(e: NotificationEvent) => void>(),
    usage: new Set<(e: TurnUsageEvent) => void>(),
  },
}));
vi.mock("../../ipc/tauriApi", () => {
  const sub = <T,>(set: Set<(e: T) => void>) => (cb: (e: T) => void) => { set.add(cb); return () => set.delete(cb); };
  return { tauriApi: {
    ...api,
    onSessionState: sub(cbs.state), onActivity: sub(cbs.activity), onNotification: sub(cbs.notification), onTurnUsage: sub(cbs.usage),
  } };
});
vi.mock("../../terminal/TerminalRegistry", () => ({ terminalRegistry: registry }));
import { useAppStore } from "../../store/appStore";
import { useDiscussionStore } from "../discussionStore";
import { useRoleStore } from "../roleStore";
import {
  closeDiscussionSession, closeDiscussionSessions, collectDiscussionUtterance, discussionSessionBlockedReason,
  discussionSessionErrorText, discussionSessionTitle, installDiscussionSessionTracking, liveDiscussionSessionCount,
  openDiscussionSession as openWithTurn, requestCloseDiscussionSession, resetDiscussionSessionStoreForTest, sendDiscussionTurn,
  useDiscussionSessionStore,
} from "../discussionSessionStore";

const now = 1_800_000_000_000;
const initialApp = useAppStore.getState();
let off: () => void;

function discussion(id: string, people: string[], patch: Partial<Discussion> = {}): Discussion {
  return {
    id, topic: `Topic ${id}`, context: "", cwd: "/repo", moderatorAgentId: people[0],
    participants: people.map((agentId, n) => ({ agentId, agent: { cli: n === 1 ? "codex" : "claude" }, ...(n === 0 ? { roleId: "preset-moderator" } : {}) })),
    limits: { maxRounds: 4, maxMinutes: 30, maxCostUsd: 5, usageGuard: true }, state: "running", round: 1, transcript: [],
    costUsd: 0, createdAt: now, updatedAt: now, ...patch,
  };
}
const sid = (d: string, p: string) => discussionSessionAgentId(d, p)!;
function emitState(e: Partial<SessionStateEvent>) { cbs.state.forEach((cb) => cb({ agentId: "", sessionId: "", state: "running", at: now, ...e } as SessionStateEvent)); }
/** 차례 지시가 없어도 되는 검사용. 세션은 늘 차례를 싣고 열린다. */
const openDiscussionSession = (discussionId: string, agentId: string, options: { turn?: string; focus?: boolean } = {}) =>
  openWithTurn(discussionId, agentId, { turn: "Speak.", ...options });
function lastRequest(): DiscussionSessionRequest { const calls = api.openDiscussionSession.mock.calls; return calls[calls.length - 1][0]; }

beforeEach(() => {
  Object.values(api).forEach((f) => f.mockReset());
  Object.values(registry).forEach((f) => f.mockReset());
  let n = 0;
  api.openDiscussionSession.mockImplementation(async (r: DiscussionSessionRequest) => {
    const cli = r.agentId === "b" ? "codex" : "claude";
    return { agentId: sid(r.discussionId, r.agentId), sessionId: `sid-${++n}`, submittedAt: now, collection: discussionCollectionMode(cli), observed: true };
  });
  api.disposeSession.mockResolvedValue(undefined);
  const agents = Object.fromEntries(["m", "b", "c", "d", "e", "f"].map((id) => [id, { id, name: `Name ${id}`, role: "", seed: id, createdAt: now, deskIndex: 0 }]));
  useAppStore.setState({ ...initialApp, agents, agentOrder: Object.keys(agents), recentAgentIds: ["m"], activeTerminalAgentId: null }, true);
  useDiscussionStore.setState({
    discussions: [discussion("d1", ["m", "b", "c", "d", "e"]), discussion("d2", ["f", "m"])], status: "ready", dialog: null,
  });
  useRoleStore.setState({ roles: [{ id: "preset-moderator", name: "사회자", perspective: "", stance: "moderator", criteria: "" }], status: "ready" });
  resetDiscussionSessionStoreForTest();
  off = installDiscussionSessionTracking();
});
afterEach(() => { off(); });

describe("토의 세션 id와 읽기 전용 미러", () => {
  it("agentId는 discuss-<토의>.<참가자>이고 되읽을 수 있으며, 안전하지 않은 id는 거절한다", () => {
    expect(sid("d1", "V1St-_x")).toBe("discuss-d1.V1St-_x");
    expect(parseDiscussionSessionAgentId("discuss-a-b.c-d")).toEqual({ discussionId: "a-b", participantAgentId: "c-d" });
    expect(discussionSessionAgentId("a.b", "c")).toBeNull();
    expect(discussionSessionAgentId("a", "../x")).toBeNull();
    expect(parseDiscussionSessionAgentId("discuss-a.b.c")).toBeNull();
    expect(parseDiscussionSessionAgentId("task-chat-a.b")).toBeNull();
  });
  it("계약 fixture가 TS 타입과 맞고 결과 agentId가 id 규칙을 따른다", () => {
    const request: DiscussionSessionRequest = requestFixture;
    const result = resultFixture as DiscussionSessionResult;
    const collect = collectFixture as DiscussionCollectRequest;
    const utterance = utteranceFixture as CollectedUtterance;
    expect(result.agentId).toBe(sid(request.discussionId, request.agentId));
    expect(collect.agentId).toBe(result.agentId);
    expect(["submit", "last-reply"]).toContain(result.collection);
    expect(["submitted", "last-reply"]).toContain(utterance.source);
  });
  it("CLI별 수집 방식은 Rust와 같다", () => {
    expect(["claude", "codex", "pi", "agy", "kilo"].map((c) => discussionCollectionMode(c as never))).toEqual(["last-reply", "last-reply", "submit", "submit", "submit"]);
  });
});

describe("토의 세션 열기", () => {
  it("결과를 받은 뒤 탭을 만들고, 이름은 캐릭터 이름 · 역할이다", async () => {
    expect(await openDiscussionSession("d1", "m", { turn: "Open." })).toBeUndefined();
    expect(lastRequest()).toMatchObject({ discussionId: "d1", agentId: "m", turn: "Open." });
    expect(registry.ensure).toHaveBeenCalledWith(sid("d1", "m"));
    const session = useDiscussionSessionStore.getState().sessions[sid("d1", "m")];
    expect(session).toMatchObject({ discussionId: "d1", participantAgentId: "m", sessionId: "sid-1", cli: "claude", collection: "last-reply", status: "running" });
    expect(useAppStore.getState().activeTerminalAgentId).toBe(sid("d1", "m"));
    expect(useAppStore.getState().recentAgentIds).toEqual(["m"]);
    expect(discussionSessionTitle(session)).toBe("Name m · 사회자");
    // 역할이 없으면 "역할 없음"(사회자면 "사회자"), 탭을 보여 주지 않고 열 수도 있다.
    await openDiscussionSession("d1", "b", { focus: false });
    const codex = useDiscussionSessionStore.getState().sessions[sid("d1", "b")];
    expect(codex).toMatchObject({ cli: "codex", collection: "last-reply" });
    expect(discussionSessionTitle(codex)).toBe("Name b · 역할 없음");
    useDiscussionStore.setState((s) => ({ discussions: s.discussions.map((d) => ({ ...d, participants: d.participants.map((p) => ({ ...p, roleId: undefined })) })) }));
    expect(discussionSessionTitle(session)).toBe("Name m · 사회자");
    expect(useAppStore.getState().activeTerminalAgentId).toBe(sid("d1", "m"));
    expect(useDiscussionSessionStore.getState().order).toEqual([sid("d1", "m"), sid("d1", "b")]);
  });

  it("살아 있는 세션은 다시 만들지 않고, 여는 중이면 막는다", async () => {
    let resolve!: (v: unknown) => void;
    api.openDiscussionSession.mockImplementationOnce(() => new Promise((r) => { resolve = r; }));
    const pending = openDiscussionSession("d1", "m");
    expect(discussionSessionBlockedReason("d1", "m")).toBe("discussion-session-busy");
    expect(await openDiscussionSession("d1", "m")).toBe("discussion-session-busy");
    resolve({ agentId: sid("d1", "m"), sessionId: "s-x", submittedAt: now, collection: "submit", observed: true });
    await pending;
    expect(await openDiscussionSession("d1", "m")).toBeUndefined();
    expect(api.openDiscussionSession).toHaveBeenCalledTimes(1);
  });

  it(`한 토의에 ${DISCUSSION_MAX_SESSIONS}개까지, 다른 토의는 앞 토의 탭이 남아 있으면 열지 않는다`, async () => {
    for (const p of ["m", "b", "c", "d", "e"]) expect(await openDiscussionSession("d1", p, { focus: false })).toBeUndefined();
    expect(liveDiscussionSessionCount()).toBe(DISCUSSION_MAX_SESSIONS);
    expect(await openDiscussionSession("d2", "f")).toBe("discussion-session-other");
    expect(discussionSessionErrorText("discussion-session-other")).not.toBe("discussion-session-other");
    // 상한: 같은 토의라도 여섯 번째는 막힌다(가상의 여섯 번째 참가자).
    useDiscussionStore.setState((s) => ({ discussions: s.discussions.map((d) => (d.id === "d1" ? discussion("d1", ["m", "b", "c", "d", "e", "f"]) : d)) }));
    expect(await openDiscussionSession("d1", "f")).toBe("discussion-session-limit");
    await closeDiscussionSessions("d1");
    expect(liveDiscussionSessionCount()).toBe(0);
    expect(api.disposeSession).toHaveBeenCalledTimes(5);
    expect(await openDiscussionSession("d2", "f")).toBeUndefined();
  });

  it("없는 토의·참가자가 아닌 사람·실패한 열기는 탭을 만들지 않는다", async () => {
    expect(await openDiscussionSession("zz", "m")).toBe("discussion-session-missing");
    expect(await openDiscussionSession("d2", "c")).toBe("discussion-session-not-participant");
    expect(await openDiscussionSession("d.1", "m")).toBe("discussion-session-invalid-id");
    api.openDiscussionSession.mockRejectedValueOnce("work-send-create-failed");
    expect(await openDiscussionSession("d1", "m")).toBe("work-send-create-failed");
    expect(registry.destroy).toHaveBeenCalledWith(sid("d1", "m"));
    expect(useDiscussionSessionStore.getState().order).toEqual([]);
    expect(api.openDiscussionSession).toHaveBeenCalledTimes(1);
  });
});

describe("토의 세션 닫기", () => {
  it("살아 있으면 확인창을 띄우고, 닫으면 세션을 버리고 이웃 탭으로 옮긴다", async () => {
    await openDiscussionSession("d1", "m");
    await openDiscussionSession("d1", "b");
    requestCloseDiscussionSession(sid("d1", "b"));
    expect(useAppStore.getState().modal).toEqual({ kind: "confirm-close-discussion-session", agentId: sid("d1", "b") });
    await closeDiscussionSession(sid("d1", "b"));
    expect(api.disposeSession).toHaveBeenCalledWith(sid("d1", "b"));
    expect(registry.destroy).toHaveBeenCalledWith(sid("d1", "b"));
    expect(useAppStore.getState().activeTerminalAgentId).toBe(sid("d1", "m"));
    // 끝난 세션은 확인 없이 닫고 dispose도 하지 않는다.
    emitState({ agentId: sid("d1", "m"), sessionId: "sid-1", state: "exited" });
    useAppStore.setState({ modal: { kind: "none" } } as never);
    requestCloseDiscussionSession(sid("d1", "m"));
    await Promise.resolve();
    expect(useDiscussionSessionStore.getState().order).toEqual([]);
    expect(api.disposeSession).toHaveBeenCalledTimes(1);
    expect(useAppStore.getState().activeTerminalAgentId).toBe("m");
  });
});

describe("턴 단계와 발언 수집", () => {
  it("관찰 신호로 턴 단계를 따르고, 옛 신호·턴 종료 뒤 질문 신호·다른 세션 신호는 버린다", async () => {
    await openDiscussionSession("d1", "m");
    const id = sid("d1", "m");
    const phase = () => useDiscussionSessionStore.getState().sessions[id].turn;
    cbs.activity.forEach((cb) => cb({ agentId: id, sessionId: "sid-1", kind: "prompt", at: now + 1 } as ActivityEvent));
    expect(phase()).toEqual({ phase: "working", at: now + 1 });
    cbs.notification.forEach((cb) => cb({ id: "n1", agentId: id, sessionId: "sid-1", source: "hook", message: "allow?", at: now + 2 } as NotificationEvent));
    expect(phase()?.phase).toBe("waiting");
    cbs.usage.forEach((cb) => cb({ agentId: id, sessionId: "sid-1", at: now + 3, partial: true } as TurnUsageEvent));
    expect(phase()?.phase).toBe("waiting");
    cbs.notification.forEach((cb) => cb({ id: "n2", agentId: id, sessionId: "sid-1", source: "stop", message: "done", at: now + 4 } as NotificationEvent));
    expect(phase()).toEqual({ phase: "idle", at: now + 4 });
    cbs.notification.forEach((cb) => cb({ id: "n3", agentId: id, sessionId: "sid-1", source: "hook", message: "idle", at: now + 5 } as NotificationEvent));
    expect(phase()?.phase).toBe("idle");
    cbs.activity.forEach((cb) => cb({ agentId: id, sessionId: "sid-1", kind: "tool", at: now }));
    expect(phase()?.phase).toBe("idle");
    cbs.activity.forEach((cb) => cb({ agentId: id, sessionId: "old", kind: "tool", at: now + 9 } as ActivityEvent));
    cbs.activity.forEach((cb) => cb({ agentId: "m", sessionId: "sid-1", kind: "tool", at: now + 9 } as ActivityEvent));
    expect(phase()?.phase).toBe("idle");
    cbs.usage.forEach((cb) => cb({ agentId: id, sessionId: "sid-1", at: now + 6, partial: false } as TurnUsageEvent));
    expect(phase()).toEqual({ phase: "idle", at: now + 6 });
    emitState({ agentId: id, sessionId: "sid-1", state: "exited", at: now + 7 });
    expect(phase()?.phase).toBe("ended");
    cbs.activity.forEach((cb) => cb({ agentId: id, sessionId: "sid-1", kind: "prompt", at: now + 8 } as ActivityEvent));
    expect(phase()?.phase).toBe("ended");
  });

  it("탭이 생기기 전에 온 상태·턴 신호로 시작한다", async () => {
    const id = sid("d1", "m");
    emitState({ agentId: id, sessionId: "sid-1", state: "running" });
    cbs.activity.forEach((cb) => cb({ agentId: id, sessionId: "sid-1", kind: "prompt", at: now } as ActivityEvent));
    await openDiscussionSession("d1", "m");
    expect(useDiscussionSessionStore.getState().sessions[id].turn).toEqual({ phase: "working", at: now });
  });

  it("차례 보내기와 수집은 그 세션 id와 CLI로 네이티브를 부른다", async () => {
    await openDiscussionSession("d1", "b");
    const id = sid("d1", "b");
    api.sendDiscussionTurn.mockResolvedValue({ sessionId: "sid-1", submittedAt: now + 10 });
    expect(await sendDiscussionTurn(id, "Your turn")).toEqual({ submittedAt: now + 10 });
    expect(api.sendDiscussionTurn).toHaveBeenCalledWith({ agentId: id, sessionId: "sid-1", text: "Your turn" });
    api.collectDiscussionUtterance.mockResolvedValue({ text: "My view", source: "last-reply" });
    expect(await collectDiscussionUtterance(id, now + 10)).toEqual({ text: "My view", source: "last-reply" });
    expect(api.collectDiscussionUtterance).toHaveBeenCalledWith({ agentId: id, sessionId: "sid-1", cli: "codex", since: now + 10 });
    expect(await collectDiscussionUtterance(sid("d1", "c"), 0)).toBeNull();
    emitState({ agentId: id, sessionId: "sid-1", state: "exited" });
    await expect(sendDiscussionTurn(id, "again")).rejects.toThrow("work-send-session-changed");
  });
});
