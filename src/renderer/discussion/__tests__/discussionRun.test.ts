import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CollectedUtterance, Discussion, DiscussionCollectRequest, DiscussionSessionRequest } from "@shared/types";
import { discussionCollectionMode, discussionReadOnlyMode, discussionSessionAgentId } from "@shared/types";

const { api, notify } = vi.hoisted(() => ({
  api: {
    openDiscussionSession: vi.fn(), disposeSession: vi.fn(), sendDiscussionTurn: vi.fn(), collectDiscussionUtterance: vi.fn(),
    saveDiscussion: vi.fn(), loadDiscussions: vi.fn(),
  },
  notify: vi.fn(),
}));
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: { ...api, onSessionState: () => () => {}, onActivity: () => () => {}, onNotification: () => () => {}, onTurnUsage: () => () => {} },
}));
vi.mock("../../terminal/TerminalRegistry", () => ({ terminalRegistry: { ensure: vi.fn(), destroy: vi.fn(), get: vi.fn(), has: vi.fn() } }));
vi.mock("../../ipc/osNotify", () => ({ maybeSendOsNotification: notify }));
import { t } from "../../i18n";
import { useAppStore } from "../../store/appStore";
import { useDiscussionStore } from "../discussionStore";
import { useRoleStore } from "../roleStore";
import { applyDiscussionSessionState, applyDiscussionTurn, resetDiscussionSessionStoreForTest, useDiscussionSessionStore } from "../discussionSessionStore";
import {
  MODERATOR_FORMAT, answerDiscussion, discussionStartBlockedReason, endDiscussion, installDiscussionRunner, isDiscussionStepping,
  parseModeratorDirective, pauseDiscussion, resetDiscussionRunForTest, restoreDiscussionsAfterRestart, resumeDiscussion, startDiscussion,
} from "../discussionRun";

const now = 1_800_000_000_000;
const initialApp = useAppStore.getState();
let clock = now;
let seq = 0;
/** 참가자 → 다음 수집 결과(차례마다 하나씩 꺼낸다). */
let replies: Record<string, (string | null)[]>;
let off: () => void;

function discussion(patch: Partial<Discussion> = {}): Discussion {
  return {
    id: "d1", topic: "Where does the meeting room go?", context: "", cwd: "/repo", moderatorAgentId: "m",
    participants: [
      { agentId: "m", roleId: "preset-moderator", agent: { cli: "claude" } },
      { agentId: "a", roleId: "preset-critic", agent: { cli: "claude" } },
      { agentId: "b", agent: { cli: "codex" } },
    ],
    limits: { maxRounds: 4, maxMinutes: 30, maxCostUsd: 5, usageGuard: true }, state: "ready", round: 0, transcript: [],
    costUsd: 0, createdAt: now, updatedAt: now, ...patch,
  };
}
const sid = (p: string) => discussionSessionAgentId("d1", p)!;
const d1 = () => useDiscussionStore.getState().discussions.find((d) => d.id === "d1")!;
const session = (p: string) => useDiscussionSessionStore.getState().sessions[sid(p)];
async function settle(): Promise<void> {
  for (let i = 0; i < 50; i++) {
    await new Promise((r) => setTimeout(r, 0));
    if (!isDiscussionStepping("d1")) return;
  }
}
/** 참가자의 턴이 끝났다(관찰 훅 idle). 다음 수집 결과를 정해 둔다. */
async function finishTurn(p: string, reply: string | null): Promise<void> {
  (replies[p] ??= []).push(reply);
  const s = session(p);
  applyDiscussionTurn(s.agentId, s.sessionId, "working", ++clock);
  applyDiscussionTurn(s.agentId, s.sessionId, "idle", ++clock);
  await settle();
}
const openedTurn = (p: string) => (api.openDiscussionSession.mock.calls as [DiscussionSessionRequest][]).filter(([r]) => r.agentId === p).slice(-1)[0]?.[0].turn ?? "";
const sentTurns = (p: string) => (api.sendDiscussionTurn.mock.calls as [{ agentId: string; text: string }][]).filter(([r]) => r.agentId === sid(p)).map(([r]) => r.text);
const directive = (value: unknown) => `Here is my call.\n\n\`\`\`json\n${JSON.stringify(value)}\n\`\`\``;

beforeEach(() => {
  Object.values(api).forEach((f) => f.mockReset());
  notify.mockReset();
  clock = now; seq = 0; replies = {};
  api.openDiscussionSession.mockImplementation(async (r: DiscussionSessionRequest) => {
    const cli = r.agentId === "b" ? "codex" : "claude";
    return { agentId: sid(r.agentId), sessionId: `s${++seq}`, submittedAt: ++clock, readOnly: discussionReadOnlyMode(cli), collection: discussionCollectionMode(cli), observed: true };
  });
  api.sendDiscussionTurn.mockImplementation(async (r: { sessionId: string }) => ({ sessionId: r.sessionId, submittedAt: ++clock }));
  api.collectDiscussionUtterance.mockImplementation(async (r: DiscussionCollectRequest): Promise<CollectedUtterance | null> => {
    const participant = r.agentId.split(".")[1];
    const text = replies[participant]?.shift();
    return text ? { text, source: "submitted" } : null;
  });
  api.disposeSession.mockResolvedValue(undefined);
  api.saveDiscussion.mockResolvedValue(undefined);
  const agents = Object.fromEntries([["m", "Mina"], ["a", "Ari"], ["b", "Bo"], ["x", "Xu"]].map(([id, name]) => [id, { id, name, role: "", seed: id, createdAt: now, deskIndex: 0 }]));
  useAppStore.setState({ ...initialApp, agents, agentOrder: Object.keys(agents), recentAgentIds: [], activeTerminalAgentId: null,
    appSettings: { ...initialApp.appSettings, observerEnabled: true } }, true);
  useDiscussionStore.setState({ discussions: [discussion()], status: "ready", dialog: null });
  useRoleStore.setState({ roles: [
    { id: "preset-moderator", name: "Moderator", perspective: "", stance: "moderator", criteria: "" },
    { id: "preset-critic", name: "Critic", perspective: "", stance: "critical", criteria: "" },
  ], status: "ready" });
  resetDiscussionSessionStoreForTest();
  resetDiscussionRunForTest();
  off = installDiscussionRunner();
});
afterEach(async () => { off(); await settle(); });

/** 시작해서 동시 첫 라운드를 마치고 사회자 차례까지 간다. */
async function throughOpening(): Promise<void> {
  expect(startDiscussion("d1")).toBeUndefined();
  await settle();
  await finishTurn("a", "Put it by the pantry.");
  await finishTurn("b", "Put it in the north corner.");
}

describe("사회자 지시 읽기", () => {
  const d = discussion();
  const names = { m: "Mina", a: "Ari", b: "Bo" };
  it("산문·코드 블록 속 JSON에서 지시를 뽑고, id가 아니면 이름 하나로 맞을 때 받는다", () => {
    expect(parseModeratorDirective(directive({ action: "next", speakers: [{ id: "b", question: "Why?" }, { name: "@ari", question: "Reply." }] }), d, names))
      .toEqual({ directive: { action: "next", speakers: [{ agentId: "b", question: "Why?" }, { agentId: "a", question: "Reply." }] } });
    expect(parseModeratorDirective('Asking: {"action":"ask_user","question":"Budget?"} thanks', d, names))
      .toEqual({ directive: { action: "ask_user", question: "Budget?" } });
    expect(parseModeratorDirective('{"action":"wrap","summary":"Corner wins."}', d, names)).toEqual({ directive: { action: "wrap", summary: "Corner wins." } });
  });
  it("형식이 틀리면 이유를 돌려준다", () => {
    const problem = (v: unknown) => "problem" in (v = parseModeratorDirective(typeof v === "string" ? v : JSON.stringify(v), d, names)) ? (v as { problem: string }).problem : null;
    expect(problem("I think we should wrap up.")).toBe("no-json");
    expect(problem({ action: "vote" })).toBe("bad-action");
    expect(problem({ action: "next", speakers: [] })).toBe("no-speakers");
    expect(problem({ action: "next", speakers: [{ id: "zz", question: "?" }] })).toBe("unknown-speaker");
    expect(problem({ action: "next", speakers: [{ id: "m", question: "?" }] })).toBe("moderator-speaker");
    expect(problem({ action: "next", speakers: [{ id: "a", question: "?" }, { id: "a", question: "!" }] })).toBe("duplicate-speaker");
    expect(problem({ action: "next", speakers: [{ id: "a" }] })).toBe("empty-question");
    expect(problem({ action: "ask_user", question: " " })).toBe("empty-question");
    expect(problem({ action: "wrap" })).toBe("empty-summary");
  });
});

describe("시작", () => {
  it("다른 토의가 진행 중이거나, 관찰이 꺼졌거나, 삭제된 팀원이 있거나, 준비가 아니면 막는다", () => {
    const agents = useAppStore.getState().agents;
    expect(discussionStartBlockedReason(discussion(), [discussion(), discussion({ id: "d2", state: "paused" })], agents, true)).toBe("discussion-busy");
    expect(discussionStartBlockedReason(discussion(), [discussion(), discussion({ id: "d2", state: "finished" })], agents, true)).toBeUndefined();
    expect(discussionStartBlockedReason(discussion(), [], agents, false)).toBe("discussion-observer-off");
    const gone = discussion({ participants: [...discussion().participants, { agentId: "ghost", agent: { cli: "claude" } }] });
    expect(discussionStartBlockedReason(gone, [], agents, true)).toBe("discussion-missing-member");
    expect(discussionStartBlockedReason(discussion({ state: "finished" }), [], agents, true)).toBe("discussion-not-ready");
  });
});

describe("동시 첫 라운드", () => {
  it("토론자 전원의 탭을 차례와 함께 한꺼번에 열고, 전원이 낼 때까지 공개하지 않는다", async () => {
    expect(startDiscussion("d1")).toBeUndefined();
    await settle();
    expect(d1()).toMatchObject({ state: "running", round: 1, run: { phase: "opening" } });
    const opened = (api.openDiscussionSession.mock.calls as [DiscussionSessionRequest][]).map(([r]) => r.agentId);
    expect(opened.sort()).toEqual(["a", "b"]);
    expect(openedTurn("a")).toContain(t("discussion:run.prompt.opening"));
    // 탭을 열 때 화면을 빼앗지 않는다.
    expect(useAppStore.getState().activeTerminalAgentId).toBeNull();

    await finishTurn("a", "Put it by the pantry.");
    expect(d1().transcript).toEqual([]);
    expect(d1().run!.turns.find((x) => x.agentId === "a")!.text).toBe("Put it by the pantry.");
    expect(api.openDiscussionSession.mock.calls.some(([r]) => r.agentId === "m")).toBe(false);

    await finishTurn("b", "Put it in the north corner.");
    expect(d1().transcript.map((e) => [e.kind, e.agentId, e.round])).toEqual([["opening", "a", 1], ["opening", "b", 1]]);
    expect(d1().run).toMatchObject({ phase: "moderator", turns: [{ agentId: "m" }] });
    // 사회자는 첫 라운드가 끝난 뒤 열리고, 첫 프롬프트에 두 발언과 토론자 id 표·지시 형식이 실린다.
    const turn = openedTurn("m");
    expect(turn).toContain("Put it by the pantry.");
    expect(turn).toContain("Put it in the north corner.");
    expect(turn).toContain("- Ari (Critic): `a`");
    expect(turn).toContain(MODERATOR_FORMAT);
  });

  it("끝나기 전에 온 옛 idle 신호로는 받지 않는다", async () => {
    startDiscussion("d1");
    await settle();
    const s = session("a");
    replies.a = ["early"];
    applyDiscussionTurn(s.agentId, s.sessionId, "idle", s.submittedAt - 1);
    await settle();
    expect(api.collectDiscussionUtterance).not.toHaveBeenCalled();
    expect(d1().run!.turns.find((x) => x.agentId === "a")!.text).toBeUndefined();
  });
});

describe("사회자 지목", () => {
  it("지목한 순서대로 말하게 하고, 차례마다 그 세션이 못 본 기록만 전한다", async () => {
    await throughOpening();
    await finishTurn("m", directive({ action: "next", speakers: [{ id: "b", question: "Why the corner?" }, { id: "a", question: "Answer Bo." }] }));
    expect(d1()).toMatchObject({ round: 2, run: { phase: "speakers" } });
    expect(d1().transcript.slice(-1)[0]).toMatchObject({ kind: "moderator", agentId: "m", round: 2 });

    // Bo는 같은 세션이라 자기 첫 발언은 빼고 Ari 발언과 사회자 지시만 받는다. Ari 차례는 아직 오지 않는다.
    const toBo = sentTurns("b");
    expect(toBo).toHaveLength(1);
    expect(toBo[0]).toContain("Put it by the pantry.");
    expect(toBo[0]).not.toContain("Put it in the north corner.");
    expect(toBo[0]).toContain("Why the corner?");
    expect(sentTurns("a")).toHaveLength(0);

    await finishTurn("b", "Quiet, and away from the coffee machine.");
    expect(d1().transcript.slice(-1)[0]).toMatchObject({ kind: "statement", agentId: "b", round: 2 });
    const toAri = sentTurns("a");
    expect(toAri).toHaveLength(1);
    expect(toAri[0]).toContain("Quiet, and away from the coffee machine.");
    expect(toAri[0]).toContain("Answer Bo.");
    expect(toAri[0]).not.toContain("Put it by the pantry.");

    await finishTurn("a", "Noise is a fair point.");
    expect(d1().run).toMatchObject({ phase: "moderator", turns: [{ agentId: "m" }] });
    const toModerator = sentTurns("m");
    expect(toModerator).toHaveLength(1);
    expect(toModerator[0]).toContain("Quiet, and away from the coffee machine.");
    expect(toModerator[0]).toContain("Noise is a fair point.");
    expect(toModerator[0]).not.toContain("Put it by the pantry.");
    expect(toModerator[0]).toContain(t("discussion:run.prompt.moderator", { round: 2, max: 4, format: MODERATOR_FORMAT, roster: "- Ari (Critic): `a`\n- Bo: `b`" }));
  });
});

describe("사용자 질문", () => {
  it("사회자가 물으면 멈춰 알리고, 답하면 사회자 차례로 이어 간다", async () => {
    await throughOpening();
    await finishTurn("m", '{"action":"ask_user","question":"Is the north corner free?"}');
    expect(d1()).toMatchObject({ state: "waiting_user", run: { phase: "user", question: "Is the north corner free?" } });
    expect(d1().transcript.slice(-1)[0]).toMatchObject({ kind: "question", agentId: "m" });
    expect(notify).toHaveBeenCalledWith(expect.stringContaining("Where does the meeting room go?"), "Is the north corner free?");
    // 기다리는 동안은 아무 차례도 넣지 않는다. 빈 답은 받지 않는다.
    expect(api.sendDiscussionTurn).not.toHaveBeenCalled();
    expect(answerDiscussion("d1", "  ")).toBe(false);

    expect(answerDiscussion("d1", "Yes, it is free.")).toBe(true);
    await settle();
    expect(d1()).toMatchObject({ state: "running", run: { phase: "moderator" } });
    expect(d1().run!.question).toBeUndefined();
    expect(d1().transcript.slice(-1)[0]).toMatchObject({ kind: "answer", text: "Yes, it is free." });
    expect(sentTurns("m").slice(-1)[0]).toContain("Yes, it is free.");
  });
});

describe("종료", () => {
  it("사회자가 정리하면 끝남이 되고 토의 탭을 모두 닫는다", async () => {
    await throughOpening();
    await finishTurn("m", '{"action":"wrap","summary":"North corner, because it is quiet."}');
    expect(d1()).toMatchObject({ state: "finished" });
    expect(d1().run).toBeUndefined();
    expect(d1().endedAt).toBeGreaterThan(0);
    expect(d1().transcript.slice(-1)[0]).toMatchObject({ kind: "moderator", text: "North corner, because it is quiet." });
    expect(api.disposeSession.mock.calls.map(([id]) => id).sort()).toEqual([sid("a"), sid("b"), sid("m")]);
    expect(useDiscussionSessionStore.getState().order).toEqual([]);
  });

  it("사람이 끝내면 어느 단계에서든 끝남이 되고 알림 항목을 남긴다", async () => {
    startDiscussion("d1");
    await settle();
    pauseDiscussion("d1");
    expect(d1()).toMatchObject({ state: "paused", run: { pauseReason: "user" } });
    await endDiscussion("d1");
    expect(d1()).toMatchObject({ state: "finished" });
    expect(d1().transcript.slice(-1)[0]).toMatchObject({ kind: "notice", text: "ended-by-user" });
    expect(useDiscussionSessionStore.getState().order).toEqual([]);
  });
});

describe("멈춤·계속", () => {
  it("멈춘 동안 끝난 턴은 계속하면 받는다", async () => {
    startDiscussion("d1");
    await settle();
    pauseDiscussion("d1");
    await finishTurn("a", "Pantry.");
    expect(api.collectDiscussionUtterance).not.toHaveBeenCalled();
    expect(resumeDiscussion("d1")).toBeUndefined();
    await settle();
    expect(d1().run!.turns.find((x) => x.agentId === "a")!.text).toBe("Pantry.");
  });

  it("관찰 훅이 꺼져 있으면 계속하지 않는다", async () => {
    startDiscussion("d1");
    await settle();
    pauseDiscussion("d1");
    useAppStore.setState((s) => ({ appSettings: { ...s.appSettings, observerEnabled: false } }));
    expect(resumeDiscussion("d1")).toBe("discussion-observer-off");
    expect(d1().state).toBe("paused");
  });
});

describe("실패", () => {
  it("발언이 없으면 한 번 재촉하고, 또 없으면 멈춘다. 계속하면 다시 한 번 재촉한다", async () => {
    startDiscussion("d1");
    await settle();
    await finishTurn("a", null);
    expect(sentTurns("a")).toHaveLength(1);
    expect(sentTurns("a")[0]).toBe(t("discussion:run.prompt.nudge"));
    expect(d1().run!.turns.find((x) => x.agentId === "a")!.nudged).toBe(true);
    await finishTurn("a", null);
    expect(d1()).toMatchObject({ state: "paused", run: { pauseReason: "no-utterance", pauseDetail: "Ari" } });

    resumeDiscussion("d1");
    await settle();
    expect(sentTurns("a")).toHaveLength(2);
    await finishTurn("a", "Pantry, finally.");
    expect(d1().state).toBe("running");
    expect(d1().run!.turns.find((x) => x.agentId === "a")!.text).toBe("Pantry, finally.");
  });

  it("사회자 지시를 못 읽으면 이유와 형식을 알려 다시 받고, 또 못 읽으면 멈춘다", async () => {
    await throughOpening();
    await finishTurn("m", "Let Bo speak next.");
    expect(sentTurns("m").slice(-1)[0]).toContain(t("discussion:run.problems.no-json"));
    expect(sentTurns("m").slice(-1)[0]).toContain(MODERATOR_FORMAT);
    await finishTurn("m", '{"action":"next","speakers":[{"id":"m","question":"?"}]}');
    expect(d1()).toMatchObject({ state: "paused", run: { phase: "moderator", pauseReason: "moderator-invalid", pauseDetail: "moderator-speaker" } });
  });

  it("탭에 쓰다 만 사람 입력이 있으면 끼어들지 않고 멈춘다", async () => {
    await throughOpening();
    await finishTurn("m", '{"action":"next","speakers":[{"id":"a","question":"More?"}]}');
    // a 차례는 보내졌다. 다음 사회자 차례를 넣을 때 사람이 사회자 탭에 쓰는 중이었다.
    api.sendDiscussionTurn.mockRejectedValueOnce(new Error("work-send-human-input"));
    await finishTurn("a", "More detail.");
    expect(d1()).toMatchObject({ state: "paused", run: { phase: "moderator", pauseReason: "human-input" } });
    expect(d1().run!.turns[0].sentAt).toBeUndefined();
    resumeDiscussion("d1");
    await settle();
    expect(d1().run!.turns[0].sentAt).toBeGreaterThan(0);
  });

  it("차례를 기다리던 탭이 끝나면 멈추고, 계속하면 처음부터 기록을 실어 다시 연다", async () => {
    await throughOpening();
    const m = session("m");
    applyDiscussionSessionState({ agentId: m.agentId, sessionId: m.sessionId, state: "exited", at: ++clock } as never);
    await settle();
    expect(d1()).toMatchObject({ state: "paused", run: { pauseReason: "session-ended", pauseDetail: "Mina" } });
    resumeDiscussion("d1");
    await settle();
    expect(session("m").sessionId).not.toBe(m.sessionId);
    expect(openedTurn("m")).toContain(t("discussion:run.prompt.allHeader"));
    expect(openedTurn("m")).toContain("Put it by the pantry.");
  });

  it("탭을 열지 못하면 멈춘다", async () => {
    api.openDiscussionSession.mockRejectedValueOnce(new Error("work-send-create-failed"));
    startDiscussion("d1");
    await settle();
    expect(d1()).toMatchObject({ state: "paused", run: { pauseReason: "open-failed", pauseDetail: "work-send-create-failed" } });
  });
});

describe("앱 재시작", () => {
  it("진행 중이던 토의는 멈춤(restarted)이 되고, 질문 대기는 그대로 둔다", async () => {
    // 부팅 직후처럼 실행기가 깨우기 전에 복원한다.
    off(); off = () => {};
    const run = { phase: "speakers" as const, turns: [{ agentId: "a", question: "Q", sessionId: "old", sentAt: now }], seen: { a: { sessionId: "old", count: 1 } } };
    useDiscussionStore.setState({ discussions: [
      discussion({ state: "running", round: 2, run }),
      discussion({ id: "d2", state: "waiting_user", round: 2, run: { phase: "user", turns: [], seen: {}, question: "Q?" } }),
    ] });
    restoreDiscussionsAfterRestart();
    const [a, b] = useDiscussionStore.getState().discussions;
    expect(a).toMatchObject({ state: "paused", run: { pauseReason: "restarted" } });
    expect(b).toMatchObject({ state: "waiting_user", run: { phase: "user" } });
    await settle();
    expect(api.saveDiscussion).toHaveBeenCalledWith(expect.objectContaining({ id: "d1", state: "paused" }));
  });

  it("계속하면 세션이 사라진 차례를 새 탭으로 다시 보낸다(기록은 처음부터)", async () => {
    const transcript = [{ at: now, round: 1, kind: "opening" as const, agentId: "b", text: "Corner." }];
    useDiscussionStore.setState({ discussions: [discussion({
      state: "paused", round: 2, transcript,
      run: { phase: "speakers", turns: [{ agentId: "a", question: "Why not?", sessionId: "old", sentAt: now }], seen: { a: { sessionId: "old", count: 1 } }, pauseReason: "restarted" },
    })] });
    resumeDiscussion("d1");
    await settle();
    expect(openedTurn("a")).toContain("Corner.");
    expect(openedTurn("a")).toContain("Why not?");
    expect(d1().run!.turns[0].sessionId).toBe(session("a").sessionId);
  });
});

describe("저장", () => {
  it("발언을 거의 한꺼번에 받아도 저장 순서가 지켜져 마지막 저장이 최신 상태다", async () => {
    await throughOpening();
    const saved = (api.saveDiscussion.mock.calls as [Discussion][]).map(([d]) => d);
    expect(saved.slice(-1)[0]).toEqual(d1());
  });
});
