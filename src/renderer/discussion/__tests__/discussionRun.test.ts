import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CollectedUtterance, Discussion, DiscussionCollectRequest, DiscussionSessionRequest, UsageSnapshot } from "@shared/types";
import { discussionCollectionMode, discussionReadOnlyMode, discussionSessionAgentId } from "@shared/types";

const { api, notify } = vi.hoisted(() => ({
  api: {
    openDiscussionSession: vi.fn(), disposeSession: vi.fn(), sendDiscussionTurn: vi.fn(), collectDiscussionUtterance: vi.fn(),
    saveDiscussion: vi.fn(), loadDiscussions: vi.fn(), loadUsageSnapshot: vi.fn(), saveDiscussionMemo: vi.fn(),
  },
  notify: vi.fn(),
}));
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: { ...api, onSessionState: () => () => {}, onActivity: () => () => {}, onNotification: () => () => {}, onTurnUsage: () => () => {} },
}));
vi.mock("../../terminal/TerminalRegistry", () => ({ terminalRegistry: { ensure: vi.fn(), destroy: vi.fn(), get: vi.fn(), has: vi.fn() } }));
vi.mock("../../ipc/osNotify", () => ({ maybeSendOsNotification: notify }));
import { i18n, initI18nForTest, t } from "../../i18n";
import { useAppStore } from "../../store/appStore";
import { DEFERRED_SAVE_MS, useDiscussionStore } from "../discussionStore";
import { useRoleStore } from "../roleStore";
import { applyDiscussionSessionState, applyDiscussionTurn, resetDiscussionSessionStoreForTest, useDiscussionSessionStore } from "../discussionSessionStore";
import { resetUsageForecast } from "../../usage/usageForecastStore";
import {
  MODERATOR_FORMAT, TURN_BUDGET_BYTES, WRAP_FORMAT, WRAP_GRACE_MS, addDiscussionTurnCost, deltaText, discussionPromptLang, confirmDiscussion, handleDiscussionTurnUsage, checkDiscussionUsageWaits, discussionLimitReached, answerDiscussion, discussionStartBlockedReason, endDiscussion, installDiscussionRunner, isDiscussionStepping,
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
    id: "d1", topic: "Where does the meeting room go?", context: "", moderatorAgentId: "m",
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
  api.saveDiscussionMemo.mockImplementation(async (id: string) => `/data/discussions/memos/${id}.md`);
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
  resetUsageForecast();
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
  it("정리 지시의 메모 칸을 읽고, 문자열 안의 날 줄바꿈과 안쪽 코드 블록도 견딘다", () => {
    const raw = [
      "Wrapping up.",
      "```json",
      '{"action":"wrap","summary":"Corner wins.',
      "",
      "Run:",
      "```sh",
      "npm test",
      '```","reasons":["Quiet", "Light"],"rejected":[{"option":"Pantry","reason":"Noisy"}, "Roof"],',
      '"open_issues":"Budget","userChecks":["Friday use?"],"ideas":[{"group":"Places","items":["Corner","Pantry"]}]}',
      "```",
    ].join("\n");
    expect(parseModeratorDirective(raw, d, names)).toEqual({ directive: {
      action: "wrap", summary: "Corner wins.\n\nRun:\n```sh\nnpm test\n```",
      memo: {
        reasons: ["Quiet", "Light"], rejected: [{ option: "Pantry", reason: "Noisy" }, { option: "Roof", reason: "" }],
        openIssues: ["Budget"], userChecks: ["Friday use?"], ideas: [{ group: "Places", items: ["Corner", "Pantry"] }],
      },
    } });
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

    // 색 제어열·CR이 섞인 답(예: ctl discuss answer "$(git log --color)")은 걸러 기록한다. 그대로 두면 이후 차례 보내기가 모두 거절된다.
    expect(answerDiscussion("d1", "\u001b[33mYes\u001b[0m,\r\nit is free.\u0007")).toBe(true);
    await settle();
    expect(d1()).toMatchObject({ state: "running", run: { phase: "moderator" } });
    expect(d1().run!.question).toBeUndefined();
    expect(d1().transcript.slice(-1)[0]).toMatchObject({ kind: "answer", text: "Yes,\nit is free." });
    expect(sentTurns("m").slice(-1)[0]).toContain("Yes,\nit is free.");
  });
});

describe("종료", () => {
  it("사회자가 정리하면 결정 메모를 남기고 토의 탭을 모두 닫고 알린 뒤 결론 확인을 기다린다", async () => {
    await throughOpening();
    await finishTurn("m", '{"action":"wrap","summary":"North corner, because it is quiet.","reasons":["Quiet"],"rejected":[{"option":"Pantry","reason":"Noisy"}],"user_checks":["Is the corner free?"]}');
    expect(d1()).toMatchObject({ state: "review", decisionMemoPath: "/data/discussions/memos/d1.md" });
    expect(api.saveDiscussionMemo).toHaveBeenCalledTimes(1);
    const [memoId, memo] = api.saveDiscussionMemo.mock.calls[0] as [string, string];
    expect(memoId).toBe("d1");
    expect(memo).toMatch(/^# .*Where does the meeting room go\?/);
    for (const text of ["North corner, because it is quiet.", "- Quiet", "**Pantry** — Noisy", "- Is the corner free?", "**Mina**", "**Ari**", "Critic"]) expect(memo).toContain(text);
    expect(notify).toHaveBeenCalledWith(t("discussion:run.notify.concluded", { topic: d1().topic }), t("discussion:run.notify.concludedBody"));
    expect(d1().run).toBeUndefined();
    expect(d1().endedAt).toBeGreaterThan(0);
    expect(d1().transcript.slice(-1)[0]).toMatchObject({ kind: "moderator", text: "North corner, because it is quiet." });
    expect(api.disposeSession.mock.calls.map(([id]) => id).sort()).toEqual([sid("a"), sid("b"), sid("m")]);
    expect(useDiscussionSessionStore.getState().order).toEqual([]);
    // 결론 확인은 다음 토의를 막지 않고, 사람이 확인하면 끝남이다.
    const agents = useAppStore.getState().agents;
    expect(discussionStartBlockedReason(discussion({ id: "d2" }), [d1(), discussion({ id: "d2" })], agents, true)).toBeUndefined();
    expect(confirmDiscussion("d1")).toBe(true);
    expect(d1()).toMatchObject({ state: "finished", decisionMemoPath: "/data/discussions/memos/d1.md" });
    expect(confirmDiscussion("d1")).toBe(false);
  });

  it("메모 저장이 끝날 때까지 결론 확인을 공개하지 않는다", async () => {
    let release!: (path: string) => void;
    api.saveDiscussionMemo.mockImplementation(() => new Promise<string>((resolve) => { release = resolve; }));
    await throughOpening();
    const turn = finishTurn("m", directive({ action: "wrap", summary: "Done." }));
    await vi.waitFor(() => expect(api.saveDiscussionMemo).toHaveBeenCalled());
    expect(d1().state).toBe("wrapping_up");
    expect(d1().run?.pendingMemo).toContain("Done.");
    expect(confirmDiscussion("d1")).toBe(false);
    release("/memo.md");
    await turn;
    await settle();
    expect(d1()).toMatchObject({ state: "review", decisionMemoPath: "/memo.md" });
  });

  it("다시 열면 정리 중이던 메모 저장만 마친다", async () => {
    off();
    useDiscussionStore.setState({ discussions: [discussion({ state: "wrapping_up",
      run: { phase: "wrap_up", turns: [], seen: {}, pendingMemo: "# Memo" } })] });
    off = installDiscussionRunner();
    await settle();
    expect(api.saveDiscussionMemo).toHaveBeenCalledWith("d1", "# Memo");
    expect(api.openDiscussionSession).not.toHaveBeenCalled();
    expect(d1().state).toBe("review");
  });

  it("메모를 저장하지 못해도 탭은 닫고 결론 확인으로 둔다", async () => {
    api.saveDiscussionMemo.mockRejectedValue(new Error("disk full"));
    await throughOpening();
    await finishTurn("m", directive({ action: "wrap", summary: "Corner." }));
    expect(d1().state).toBe("review");
    expect(d1().decisionMemoPath).toBeUndefined();
    expect(useDiscussionSessionStore.getState().order).toEqual([]);
    expect(notify).toHaveBeenLastCalledWith(expect.any(String), t("discussion:run.notify.concludedNoMemo"));
  });

  it("탭을 여는 사이에 끝내면 늦게 열린 탭도 닫는다", async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => { release = r; });
    const original = api.openDiscussionSession.getMockImplementation()!;
    api.openDiscussionSession.mockImplementation(async (r: DiscussionSessionRequest) => { await gate; return original(r); });
    startDiscussion("d1");
    await settle();
    await endDiscussion("d1");
    expect(d1().state).toBe("finished");
    release();
    await settle();
    await vi.waitFor(() => expect(useDiscussionSessionStore.getState().order).toEqual([]));
    expect(api.disposeSession.mock.calls.map(([id]) => id).sort()).toEqual([sid("a"), sid("b")]);
  });

  it("사람이 끝내면 어느 단계에서든 끝남이 되고 알림 항목을 남긴다", async () => {
    startDiscussion("d1");
    await settle();
    pauseDiscussion("d1");
    expect(d1()).toMatchObject({ state: "paused", run: { pauseReason: "user" } });
    await endDiscussion("d1");
    expect(d1()).toMatchObject({ state: "finished" });
    expect(d1().transcript.slice(-1)[0]).toMatchObject({ kind: "notice", text: "ended-by-user" });
    // 사람이 도중에 끝내면 결정 메모는 쓰지 않는다.
    expect(api.saveDiscussionMemo).not.toHaveBeenCalled();
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

  it("부팅하면 저장된 진행 중 토의를 읽어 세션 끝남이 아니라 restarted로 멈춘다", async () => {
    off(); off = () => {};
    const run = { phase: "opening" as const, turns: [{ agentId: "a", sessionId: "old", sentAt: now }, { agentId: "b" }], seen: {} };
    api.loadDiscussions.mockResolvedValue([discussion({ state: "running", round: 1, run })]);
    useDiscussionStore.setState({ discussions: [], status: "idle" });
    off = installDiscussionRunner();
    await vi.waitFor(() => expect(d1()?.state).toBe("paused"));
    await settle();
    expect(d1().run!.pauseReason).toBe("restarted");
    expect(api.openDiscussionSession).not.toHaveBeenCalled();
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

// ── 안전장치(라운드·시간·비용 상한, 사용량 대기) ─────────────────────────

const HOUR = 60 * 60 * 1000;
/** Claude 5시간 창 하나뿐인 조회값. */
function usage(percent: number, resetsAtMs = Date.now() + HOUR): UsageSnapshot {
  const never = { outcome: "never_attempted", detail: null, lastAttemptMs: null, lastSuccessMs: null } as const;
  return {
    claude: { provider: "claude", fetchedAtMs: Date.now(), planLabel: null, windows: [{ kind: "session", label: null, usedPercent: percent, resetsAtMs, windowMinutes: 300, isActive: null }] },
    codex: null, opencode: null, openrouter: null,
    claudeLive: { ...never, tokenSource: null, via: null }, codexLive: never, opencodeLive: never, openrouterLive: never,
  };
}
/** 진행 시간 누계를 바꾼다(스토어가 바뀌면 실행기가 깨어 상한을 본다). */
async function setElapsed(ms: number): Promise<void> {
  useDiscussionStore.setState((s) => ({ discussions: s.discussions.map((d) => (d.id === "d1" ? { ...d, activeMs: ms, activeSince: Date.now() } : d)) }));
  await settle();
}
const notices = () => d1().transcript.filter((e) => e.kind === "notice").map((e) => e.text);
const forcedPrompt = (reason: string) => t("discussion:run.prompt.forcedWrap", { reason, format: WRAP_FORMAT });

describe("라운드 상한", () => {
  it("마지막 라운드가 끝나면 평소 지시 대신 정리 차례 하나를 주고, 받은 요약으로 끝낸다", async () => {
    useDiscussionStore.setState({ discussions: [discussion({ limits: { maxRounds: 2, maxMinutes: 30, maxCostUsd: 5, usageGuard: true } })] });
    await throughOpening();
    // 1라운드 뒤 사회자는 평소처럼 지목한다.
    expect(openedTurn("m")).toContain(MODERATOR_FORMAT);
    await finishTurn("m", directive({ action: "next", speakers: [{ id: "a", question: "Why the pantry?" }] }));
    expect(d1().round).toBe(2);
    await finishTurn("a", "Coffee is there.");
    // 2라운드가 최대: 정리 차례.
    expect(d1()).toMatchObject({ state: "wrapping_up", run: { phase: "wrap_up", forced: "rounds", turns: [{ agentId: "m" }] } });
    const reason = t("discussion:run.limit.rounds", { max: 2 });
    expect(notices()).toEqual([t("discussion:run.notice.limit", { reason })]);
    expect(sentTurns("m").slice(-1)[0]).toContain(forcedPrompt(reason));
    expect(sentTurns("m").slice(-1)[0]).toContain("Coffee is there.");
    await finishTurn("m", directive({ action: "wrap", summary: "Pantry, for the coffee." }));
    expect(d1()).toMatchObject({ state: "review", run: undefined });
    const memo = api.saveDiscussionMemo.mock.calls[0][1] as string;
    expect(memo).toContain("Pantry, for the coffee.");
    expect(memo).toContain(t("discussion:memo.endedForced", { reason: t("discussion:run.limitName.rounds") }));
    expect(d1().transcript.slice(-1)[0]).toMatchObject({ kind: "moderator", agentId: "m", text: "Pantry, for the coffee." });
    expect(sentTurns("m")).toHaveLength(1);
    expect(useDiscussionSessionStore.getState().order).toEqual([]);
  });

  it("최대 1라운드면 사회자의 첫 차례가 정리 차례다", async () => {
    useDiscussionStore.setState({ discussions: [discussion({ limits: { maxRounds: 1, maxMinutes: 30, maxCostUsd: 5, usageGuard: true } })] });
    await throughOpening();
    expect(openedTurn("m")).toContain(forcedPrompt(t("discussion:run.limit.rounds", { max: 1 })));
    expect(openedTurn("m")).not.toContain(MODERATOR_FORMAT);
    expect(openedTurn("m")).toContain("Put it in the north corner.");
  });
});

describe("시간 상한", () => {
  it("동시 첫 라운드 도중에 닿으면 받은 발언은 공개하고, 사회자 탭을 정리 차례로 열어 답변 전체를 요약으로 받는다", async () => {
    startDiscussion("d1");
    await settle();
    await finishTurn("a", "Put it by the pantry.");
    await setElapsed(30 * 60_000);
    expect(d1()).toMatchObject({ state: "wrapping_up", run: { phase: "wrap_up", forced: "time" } });
    expect(d1().transcript.map((e) => [e.kind, e.agentId])).toEqual([["opening", "a"], ["notice", undefined]]);
    expect(d1().run!.forcedAtMs).toBeGreaterThanOrEqual(30 * 60_000);
    expect(openedTurn("m")).toContain(forcedPrompt(t("discussion:run.limit.time", { max: 30 })));
    expect(openedTurn("m")).toContain("Put it by the pantry.");
    // 늦게 끝난 토론자의 발언은 받지 않는다.
    await finishTurn("b", "Too late.");
    expect(d1().transcript.some((e) => e.text === "Too late.")).toBe(false);
    await finishTurn("m", "Pantry it is: the only voice so far, nothing against it.");
    expect(d1()).toMatchObject({ state: "review" });
    expect(api.saveDiscussionMemo.mock.calls[0][1]).toContain("Pantry it is: the only voice so far, nothing against it.");
    expect(d1().transcript.slice(-1)[0]).toMatchObject({ kind: "moderator", text: "Pantry it is: the only voice so far, nothing against it." });
  });

  it("정리 차례가 유예 안에 끝나지 않으면(권한 대기 등) 요약 없이 끝낸다", async () => {
    await throughOpening();
    await setElapsed(30 * 60_000);
    expect(d1().state).toBe("wrapping_up");
    // 사회자가 권한 승인을 기다리며 멈춰 있다.
    applyDiscussionTurn(session("m").agentId, session("m").sessionId, "waiting", ++clock);
    await settle();
    expect(d1().state).toBe("wrapping_up");
    await setElapsed(d1().run!.forcedAtMs! + WRAP_GRACE_MS);
    expect(d1()).toMatchObject({ state: "review" });
    expect(notices().slice(-1)[0]).toBe(t("discussion:run.notice.wrapTimeout"));
    // 요약 없이 끝나도 메모는 남기고, 결론 자리에 까닭을 적는다.
    const memo = api.saveDiscussionMemo.mock.calls[0][1] as string;
    expect(memo).toContain(t("discussion:run.notice.wrapTimeout"));
    expect(memo).toContain(`## ${t("discussion:memo.participants")}`);
    expect(useDiscussionSessionStore.getState().order).toEqual([]);
  });

  it("진행 시간은 진행 중에만 흐르고, 멈춤·재시작 동안은 세지 않는다", async () => {
    startDiscussion("d1");
    await settle();
    expect(d1().activeMs).toBe(0);
    expect(d1().activeSince).toBeGreaterThan(0);
    pauseDiscussion("d1");
    expect(d1().activeSince).toBeUndefined();
    const paused = d1().activeMs!;
    expect(paused).toBeGreaterThanOrEqual(0);
    resumeDiscussion("d1");
    expect(d1().activeSince).toBeGreaterThan(0);
    await settle();
    // 재시작: 앱이 꺼진 시간은 빼고 마지막으로 쓴 시각에서 구간을 닫는다.
    off(); off = () => {};
    useDiscussionStore.setState({ discussions: [{ ...d1(), state: "running", activeMs: 60_000, activeSince: now, updatedAt: now + 90_000 }] });
    restoreDiscussionsAfterRestart();
    expect(d1()).toMatchObject({ state: "paused", activeMs: 150_000, activeSince: undefined });
  });
});

describe("비용 상한", () => {
  // claude-opus-5-5 입력 $4/M → 1.5M 토큰이면 $6.
  const tokens = { model: "claude-opus-5-5", input: 1_500_000 };

  it("토의 탭의 턴 토큰(중간 갱신 포함)을 요율로 셈해 누계에 더한다", async () => {
    addDiscussionTurnCost({ agentId: sid("a"), tokens: { model: "claude-opus-5-5", input: 100_000 }, at: now });
    expect(d1().costUsd).toBe(0);  // 시작 전은 세지 않는다.
    startDiscussion("d1");
    await settle();
    addDiscussionTurnCost({ agentId: sid("a"), tokens: { model: "claude-opus-5-5", input: 100_000 }, at: now });
    addDiscussionTurnCost({ agentId: sid("b"), tokens: { model: "claude-opus-5-5", output: 10_000 }, at: now });
    addDiscussionTurnCost({ agentId: "a", tokens: { model: "claude-opus-5-5", input: 1_000_000 }, at: now });
    expect(d1().costUsd).toBeCloseTo(0.4 + 0.2, 6);
  });

  it("상한에 닿으면 지목 차례를 버리고 정리 차례를 주며, 정리 차례는 재촉하지 않는다", async () => {
    await throughOpening();
    await finishTurn("m", directive({ action: "next", speakers: [{ id: "a", question: "Why?" }, { id: "b", question: "And you?" }] }));
    expect(d1().run!.phase).toBe("speakers");
    addDiscussionTurnCost({ agentId: sid("a"), tokens, at: now });
    await settle();
    expect(d1()).toMatchObject({ state: "wrapping_up", run: { forced: "cost", turns: [{ agentId: "m" }] } });
    const reason = t("discussion:run.limit.cost", { max: 5 });
    expect(sentTurns("m").slice(-1)[0]).toContain(forcedPrompt(reason));
    // 정리 차례 중 비용이 더 들어도 다시 강제하지 않는다.
    addDiscussionTurnCost({ agentId: sid("m"), tokens, at: now });
    await settle();
    expect(notices()).toHaveLength(1);
    const sends = sentTurns("m").length;
    await finishTurn("m", null);
    expect(sentTurns("m")).toHaveLength(sends);
    expect(d1()).toMatchObject({ state: "review" });
    expect(notices().slice(-1)[0]).toBe(t("discussion:run.notice.wrapMissing"));
  });

  it("발언 차례를 끝낸 턴이 상한을 넘기면 그 발언을 받은 뒤에 정리한다", async () => {
    await throughOpening();
    await finishTurn("m", directive({ action: "next", speakers: [{ id: "a", question: "Why?" }, { id: "b", question: "And you?" }] }));
    (replies.a ??= []).push("Because of the coffee.");
    const s = session("a");
    applyDiscussionTurn(s.agentId, s.sessionId, "working", ++clock);
    // 턴 종료와 비용이 한 이벤트로 온다(관찰 훅의 Stop). idle 신호보다 먼저 처리돼도 발언을 잃지 않는다.
    handleDiscussionTurnUsage({ agentId: s.agentId, sessionId: s.sessionId, at: ++clock, tokens, partial: false });
    await settle();
    const kinds = d1().transcript.slice(-2).map((e) => [e.kind, e.text]);
    expect(kinds[0]).toEqual(["statement", "Because of the coffee."]);
    expect(kinds[1][0]).toBe("notice");
    expect(d1()).toMatchObject({ state: "wrapping_up", run: { forced: "cost" } });
    expect(sentTurns("b")).toHaveLength(0);
  });

  it.each(["ask_user", "next", "wrap"])("상한을 넘긴 사회자 지시 %s를 마무리한다", async (action) => {
    await throughOpening();
    replies.m = [directive({ action, question: "Budget?", speakers: [{ id: "a", question: "Why?" }], summary: "Done." })];
    const s = session("m");
    handleDiscussionTurnUsage({ agentId: s.agentId, sessionId: s.sessionId, at: ++clock, tokens, partial: false });
    await settle();
    expect(d1().costUsd).toBeGreaterThanOrEqual(d1().limits.maxCostUsd);
    if (action === "wrap") expect(d1().state).toBe("review");
    else expect(d1()).toMatchObject({ state: "wrapping_up", round: 1, run: { forced: "cost" } });
  });

  it("상한 검사: 시간·비용", () => {
    const base = discussion({ state: "running", activeMs: 0 });
    expect(discussionLimitReached(base)).toBeUndefined();
    expect(discussionLimitReached({ ...base, activeMs: 29 * 60_000 })).toBeUndefined();
    expect(discussionLimitReached({ ...base, activeMs: 10 * 60_000, activeSince: now }, now + 20 * 60_000)).toBe("time");
    expect(discussionLimitReached({ ...base, activeMs: 10 * 60_000, activeSince: now, state: "paused" }, now + 20 * 60_000)).toBeUndefined();
    expect(discussionLimitReached({ ...base, costUsd: 5 })).toBe("cost");
  });
});

describe("차례 크기", () => {
  const size = (text: string) => new TextEncoder().encode(text).length;

  it("기록이 길면 발언마다 끝부분을 남겨 고르게 줄이고, 지시는 온전히 둔다", async () => {
    // 한글 32k자 ≈ 96KB. 두 사람이면 사회자의 첫 차례가 예산을 넘는다.
    const long = (tag: string) => `${tag}-start ${"가".repeat(40_000)} ${tag}-end`;
    expect(startDiscussion("d1")).toBeUndefined();
    await settle();
    await finishTurn("a", long("ari"));
    await finishTurn("b", long("bo"));
    const turn = openedTurn("m");
    expect(size(turn)).toBeLessThanOrEqual(TURN_BUDGET_BYTES);
    expect(turn.endsWith(t("discussion:run.prompt.moderator", { round: 1, max: 4, format: MODERATOR_FORMAT, roster: "- Ari (Critic): `a`\n- Bo: `b`" }))).toBe(true);
    expect(turn).toContain(t("discussion:run.prompt.trimmed"));
    expect(turn).toContain("ari-end");
    expect(turn).toContain("bo-end");
    expect(turn).not.toContain("ari-start");
  });

  it("발언마다 최소분만 남겨도 넘치면 오래된 기록부터 빼고 그 수를 알린다", () => {
    const transcript = Array.from({ length: 20 }, (_, i) => ({ at: now + i, round: 1, kind: "statement" as const, agentId: "a", text: `#${i} ${"x".repeat(1000)}` }));
    const d = discussion({ state: "running", transcript, run: { phase: "moderator", turns: [{ agentId: "m" }], seen: {} } });
    const out = deltaText(d, "m", undefined, 10_000);
    expect(size(out)).toBeLessThanOrEqual(10_000);
    expect(out).toContain("#19 ");
    expect(out).not.toContain("#0 ");
    const omitted = 20 - (out.match(/#### /g) ?? []).length;
    expect(omitted).toBeGreaterThan(0);
    expect(out).toContain(t("discussion:run.prompt.omitted", { count: omitted }));
    // 넉넉하면 그대로 싣는다.
    expect(deltaText(d, "m", undefined)).not.toContain(t("discussion:run.prompt.trimmed"));
  });
});

describe("프롬프트 언어", () => {
  afterEach(async () => { await initI18nForTest(); });

  it("첫 프롬프트(ko·en)와 같은 언어로 차례를 쓰고 그 언어를 열기 요청에 싣는다", async () => {
    expect(discussionPromptLang("ko-KR")).toBe("ko");
    expect(discussionPromptLang("ja")).toBe("en");
    expect(discussionPromptLang("zh-Hant")).toBe("en");
    await initI18nForTest("ja");
    expect(startDiscussion("d1")).toBeUndefined();
    await settle();
    const [request] = (api.openDiscussionSession.mock.calls as [DiscussionSessionRequest][]).find(([r]) => r.agentId === "a")!;
    expect(request.lang).toBe("en");
    expect(request.turn).toBe(i18n.getFixedT("en")("discussion:run.prompt.opening"));
  });
});

describe("비용 저장", () => {
  afterEach(() => { vi.useRealTimers(); });

  it("턴 중간 비용은 스토어만 바로 고치고 저장은 미루며, 턴이 끝나면 바로 저장한다", async () => {
    startDiscussion("d1");
    await settle();
    vi.useFakeTimers();
    api.saveDiscussion.mockClear();
    const tokens = { model: "claude-opus-5-5", input: 100_000 };
    addDiscussionTurnCost({ agentId: sid("a"), tokens, at: now, partial: true });
    addDiscussionTurnCost({ agentId: sid("a"), tokens, at: now, partial: true });
    expect(d1().costUsd).toBeCloseTo(0.8, 6);
    expect(api.saveDiscussion).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(DEFERRED_SAVE_MS);
    expect(api.saveDiscussion).toHaveBeenCalledTimes(1);
    expect(api.saveDiscussion.mock.calls[0][0]).toMatchObject({ id: "d1", costUsd: d1().costUsd });

    // 미룬 저장이 남은 채 다른 저장이 나가면 그 저장에 실리고 타이머는 버린다.
    addDiscussionTurnCost({ agentId: sid("a"), tokens, at: now, partial: true });
    addDiscussionTurnCost({ agentId: sid("a"), tokens, at: now, partial: false });
    await vi.advanceTimersByTimeAsync(0);  // 저장은 앞 저장 뒤에 이어 나간다.
    expect(api.saveDiscussion).toHaveBeenCalledTimes(2);
    expect(api.saveDiscussion.mock.calls[1][0]).toMatchObject({ costUsd: d1().costUsd });
    await vi.advanceTimersByTimeAsync(DEFERRED_SAVE_MS * 2);
    expect(api.saveDiscussion).toHaveBeenCalledTimes(2);
  });

  it("미룬 저장이 나가기 전에 토의를 지우면 되살리지 않는다", async () => {
    startDiscussion("d1");
    await settle();
    vi.useFakeTimers();
    addDiscussionTurnCost({ agentId: sid("a"), tokens: { model: "claude-opus-5-5", input: 100_000 }, at: now, partial: true });
    api.saveDiscussion.mockClear();
    useDiscussionStore.setState({ discussions: [] });
    await vi.advanceTimersByTimeAsync(DEFERRED_SAVE_MS);
    expect(api.saveDiscussion).not.toHaveBeenCalled();
  });
});

describe("사용량 대기", () => {
  it("차례를 받을 참가자의 구독이 한도에 닿았으면 아무에게도 보내지 않고 기다리다가, 풀리면 스스로 이어 간다", async () => {
    useAppStore.setState({ usage: usage(96) });
    expect(startDiscussion("d1")).toBeUndefined();
    await settle();
    expect(api.openDiscussionSession).not.toHaveBeenCalled();
    expect(d1()).toMatchObject({ state: "paused", activeSince: undefined, run: { pauseReason: "usage-wait", pauseDetail: "Ari", usageWait: { agentId: "a", provider: "claude", kind: "session", limit: 95 } } });
    expect(api.saveDiscussion.mock.calls.slice(-1)[0][0]).toEqual(d1());
    // 아직 모자라면 그대로 기다린다.
    await checkDiscussionUsageWaits();
    expect(d1().state).toBe("paused");
    // 조회값이 내려가면(리셋) 이어 간다.
    useAppStore.setState({ usage: usage(10) });
    await vi.waitFor(() => expect(d1().state).toBe("running"));
    await settle();
    expect(api.openDiscussionSession).toHaveBeenCalledTimes(2);
    expect(d1().run!.usageWait).toBeUndefined();
  });

  it("다시 볼 시각이 지나면 사용량을 새로 받아 이어 간다", async () => {
    useAppStore.setState({ usage: usage(96) });
    startDiscussion("d1");
    await settle();
    const until = d1().run!.usageWait!.until;
    api.loadUsageSnapshot.mockResolvedValue(usage(5));
    await checkDiscussionUsageWaits(until + 1);
    await settle();
    expect(api.loadUsageSnapshot).toHaveBeenCalled();
    expect(d1().state).toBe("running");
  });

  it("Codex 참가자는 Claude 창 때문에 기다리지 않는다", async () => {
    useAppStore.setState({ usage: usage(96) });
    await throughOpeningWithoutGuard();
    // 다음은 Codex 토론자 b의 차례다.
    await finishTurn("m", directive({ action: "next", speakers: [{ id: "b", question: "Why?" }] }));
    expect(d1()).toMatchObject({ state: "running", run: { phase: "speakers" } });
    expect(sentTurns("b")).toHaveLength(1);
  });

  it("사람이 계속하면 다음 차례 하나는 검사 없이 보내고, 사용량 대기를 끄면 검사하지 않는다", async () => {
    useAppStore.setState({ usage: usage(96) });
    startDiscussion("d1");
    await settle();
    expect(resumeDiscussion("d1")).toBeUndefined();
    await settle();
    expect(api.openDiscussionSession).toHaveBeenCalledTimes(2);
    expect(d1()).toMatchObject({ state: "running", run: { usageOverride: undefined } });
    // 다음 사회자 차례(Claude)는 다시 기다린다.
    await finishTurn("a", "Pantry.");
    await finishTurn("b", "Corner.");
    expect(d1()).toMatchObject({ state: "paused", run: { phase: "moderator", pauseReason: "usage-wait" } });
    // 사람이 멈추면 스스로 이어 가지 않는다.
    pauseDiscussion("d1");
    useAppStore.setState({ usage: usage(10) });
    await settle();
    expect(d1().run!.pauseReason).toBe("user");

    useDiscussionStore.setState({ discussions: [discussion({ limits: { maxRounds: 4, maxMinutes: 30, maxCostUsd: 5, usageGuard: false } })] });
    resetDiscussionSessionStoreForTest();
    api.openDiscussionSession.mockClear();
    useAppStore.setState({ usage: usage(96) });
    startDiscussion("d1");
    await settle();
    expect(api.openDiscussionSession).toHaveBeenCalledTimes(2);
  });
});

/** 사용량 검사 없이 첫 라운드를 마친 뒤 사용량 대기를 켠다. */
async function throughOpeningWithoutGuard(): Promise<void> {
  const limits = { maxRounds: 4, maxMinutes: 30, maxCostUsd: 5, usageGuard: false };
  useDiscussionStore.setState({ discussions: [discussion({ limits })] });
  await throughOpening();
  useDiscussionStore.setState((s) => ({ discussions: s.discussions.map((d) => ({ ...d, limits: { ...limits, usageGuard: true } })) }));
  await settle();
}
