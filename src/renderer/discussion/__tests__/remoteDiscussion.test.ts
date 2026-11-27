import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CollectedUtterance, Discussion, DiscussionCollectRequest, DiscussionSessionRequest, LinkDiscussionState, RemoteOffice } from "@shared/types";
import { discussionCollectionMode, discussionSessionAgentId } from "@shared/types";

const { api } = vi.hoisted(() => ({
  api: {
    openDiscussionSession: vi.fn(), disposeSession: vi.fn(), sendDiscussionTurn: vi.fn(), collectDiscussionUtterance: vi.fn(),
    saveDiscussion: vi.fn(), loadDiscussions: vi.fn(), loadUsageSnapshot: vi.fn(), saveDiscussionMemo: vi.fn(), linkWork: vi.fn(),
  },
}));
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    ...api, onSessionState: () => () => {}, onActivity: () => () => {}, onNotification: () => () => {}, onTurnUsage: () => () => {},
    onLinkDiscussionSession: () => () => {},
  },
}));
vi.mock("../../terminal/TerminalRegistry", () => ({ terminalRegistry: { ensure: vi.fn(), destroy: vi.fn(), get: vi.fn(), has: vi.fn() } }));
vi.mock("../../ipc/osNotify", () => ({ maybeSendOsNotification: vi.fn() }));
import { initI18nForTest } from "../../i18n";
import { useAppStore } from "../../store/appStore";
import { useDiscussionStore } from "../discussionStore";
import { useRoleStore } from "../roleStore";
import { applyDiscussionTurn, resetDiscussionSessionStoreForTest, useDiscussionSessionStore } from "../discussionSessionStore";
import { resetUsageForecast } from "../../usage/usageForecastStore";
import { setRemoteOffice } from "../../link/remoteFloors";
import { discussionStartBlockedReason, installDiscussionRunner, isDiscussionStepping, resetDiscussionRunForTest, startDiscussion } from "../discussionRun";
import {
  pollRemoteDiscussionSessions, remoteDiscussionKey, remoteOpenPayload, resetRemoteDiscussionForTest, useRemoteDiscussionStore,
} from "../remoteDiscussion";
import { officeDiscussionStage } from "../officeStage";
import { remoteParticipantProfile } from "../participants";

const now = 1_800_000_000_000;
const initialApp = useAppStore.getState();
const R = "dev1:ra";
let clock = now;
let seq = 0;
let replies: Record<string, (string | null)[]>;
/** 원격 세션이 그 머신에서 보이는 모습(`discuss-state` 답). */
let remoteState: LinkDiscussionState;
let off: () => void;

function office(online: boolean): RemoteOffice {
  return { machines: [{
    deviceId: "dev1", name: "Studio", online, repos: [],
    characters: [{ id: R, agentId: "ra", name: "Ra", role: "", appearance: { seed: "ra-seed", archetype: "elf" }, needsHuman: false, clockedOut: false }],
  }] };
}
function discussion(): Discussion {
  return {
    id: "d1", topic: "Pick a format", context: "", cwd: "/host/repo/app", moderatorAgentId: "m",
    participants: [
      { agentId: "m", roleId: "preset-moderator", agent: { cli: "claude" } },
      { agentId: R, roleId: "preset-critic", agent: { cli: "codex" } },
      { agentId: "b", agent: { cli: "claude" } },
    ],
    limits: { maxRounds: 4, maxMinutes: 30, maxCostUsd: 5, usageGuard: true }, state: "ready", round: 0,
    transcript: [{ at: 1, round: 0, kind: "notice", text: "old" }], costUsd: 0, createdAt: now, updatedAt: now,
  };
}
const d1 = () => useDiscussionStore.getState().discussions.find((d) => d.id === "d1")!;
const sid = (p: string) => discussionSessionAgentId("d1", p)!;
const remoteKey = remoteDiscussionKey("d1", R)!;
const calls = (kind: string) => (api.linkWork.mock.calls as [string, string, Record<string, unknown>][]).filter(([, k]) => k === kind).map(([, , d]) => d);
async function settle(): Promise<void> {
  for (let i = 0; i < 50; i++) {
    await new Promise((r) => setTimeout(r, 0));
    if (!isDiscussionStepping("d1")) return;
  }
}
async function finishLocal(p: string, reply: string): Promise<void> {
  (replies[p] ??= []).push(reply);
  const s = useDiscussionSessionStore.getState().sessions[sid(p)];
  applyDiscussionTurn(s.agentId, s.sessionId, "working", ++clock);
  applyDiscussionTurn(s.agentId, s.sessionId, "idle", ++clock);
  await settle();
}
/** 원격 참가자의 턴이 끝났다고 그 머신이 답하게 하고 한 번 묻는다. */
async function finishRemote(reply: string, usage: LinkDiscussionState["usage"] = []): Promise<void> {
  (replies[R] ??= []).push(reply);
  remoteState = { alive: true, turn: { phase: "idle", at: ++clock }, usage };
  await pollRemoteDiscussionSessions((id, u) => void import("../discussionRun").then((m) => m.addDiscussionCost(id, u)));
  await settle();
}

beforeEach(() => {
  initI18nForTest();
  Object.values(api).forEach((f) => f.mockReset());
  clock = now; seq = 0; replies = {};
  remoteState = { alive: true, turn: null, usage: [] };
  api.openDiscussionSession.mockImplementation(async (r: DiscussionSessionRequest) => ({
    agentId: sid(r.agentId), sessionId: `s${++seq}`, submittedAt: ++clock, collection: discussionCollectionMode("claude"), observed: true,
  }));
  api.sendDiscussionTurn.mockImplementation(async (r: { sessionId: string }) => ({ sessionId: r.sessionId, submittedAt: ++clock }));
  api.collectDiscussionUtterance.mockImplementation(async (r: DiscussionCollectRequest): Promise<CollectedUtterance | null> => {
    const text = replies[r.agentId.split(".")[1]]?.shift();
    return text ? { text, source: "submitted" } : null;
  });
  api.linkWork.mockImplementation(async (agentId: string, kind: string) => {
    expect(agentId).toBe(R);
    if (kind === "discuss-open") return { agentId: "discuss-d1.ra", sessionId: `r${++seq}`, submittedAt: ++clock, collection: "last-reply", observed: true, cwd: "/clone/app" };
    if (kind === "discuss-turn") return { submittedAt: ++clock };
    if (kind === "discuss-state") return remoteState;
    if (kind === "discuss-collect") { const text = replies[R]?.shift(); return text ? { text, source: "last-reply" } : null; }
    return {};
  });
  api.disposeSession.mockResolvedValue(undefined);
  api.saveDiscussion.mockResolvedValue(undefined);
  api.saveDiscussionMemo.mockImplementation(async (id: string) => `/data/discussions/memos/${id}.md`);
  const agents = Object.fromEntries([["m", "Mina"], ["b", "Bo"]].map(([id, name]) => [id, { id, name, role: "", seed: id, createdAt: now, deskIndex: 0 }]));
  useAppStore.setState({ ...initialApp, agents, agentOrder: Object.keys(agents), recentAgentIds: [], activeTerminalAgentId: null,
    appSettings: { ...initialApp.appSettings, observerEnabled: true } }, true);
  useDiscussionStore.setState({ discussions: [discussion()], status: "ready", dialog: null });
  useRoleStore.setState({ roles: [
    { id: "preset-moderator", name: "Moderator", perspective: "", stance: "moderator", criteria: "" },
    { id: "preset-critic", name: "Critic", perspective: "", stance: "critical", criteria: "" },
    { id: "unused", name: "Unused", perspective: "", stance: "free", criteria: "" },
  ], status: "ready" });
  setRemoteOffice(office(true));
  resetDiscussionSessionStoreForTest();
  resetRemoteDiscussionForTest();
  resetDiscussionRunForTest();
  resetUsageForecast();
  off = installDiscussionRunner();
});
afterEach(async () => { off(); await settle(); setRemoteOffice({ machines: [] }); });

describe("머신 간 토의", () => {
  it("끊긴 머신 참가자나 모르는 원격 참가자가 있으면 시작하지 않는다", () => {
    const app = useAppStore.getState();
    setRemoteOffice(office(false));
    expect(discussionStartBlockedReason(d1(), [d1()], app.agents, true)).toBe("discussion-remote-offline");
    setRemoteOffice({ machines: [] });
    expect(discussionStartBlockedReason(d1(), [d1()], app.agents, true)).toBe("discussion-missing-member");
    setRemoteOffice(office(true));
    expect(discussionStartBlockedReason(d1(), [d1()], app.agents, true)).toBeUndefined();
  });

  it("그 머신에 보낼 토의는 기록·진행을 빼고 이름과 쓰는 역할만 싣는다", () => {
    const payload = remoteOpenPayload({ ...d1(), run: { phase: "opening", turns: [], seen: {} } }, R, "Go", "en");
    expect(payload).toMatchObject({ discussionId: "d1", participant: R, turn: "Go", lang: "en", members: { m: "Mina", [R]: "Ra · Studio", b: "Bo" } });
    const sent = payload.discussion as Discussion;
    expect(sent.transcript).toEqual([]);
    expect(sent.run).toBeUndefined();
    expect((payload.roles as { id: string }[]).map((r) => r.id).sort()).toEqual(["preset-critic", "preset-moderator"]);
  });

  it("원격 참가자가 섞인 토의가 호스트에서 끝까지 돌고 메모가 호스트에 남는다", async () => {
    expect(startDiscussion("d1")).toBeUndefined();
    await settle();
    // 첫 라운드: 이 머신 참가자는 이 머신 탭, 원격 참가자는 그 머신 세션.
    expect(api.openDiscussionSession.mock.calls.map(([r]) => (r as DiscussionSessionRequest).agentId)).toEqual(["b"]);
    expect(calls("discuss-open")).toHaveLength(1);
    expect(useDiscussionSessionStore.getState().order).not.toContain(remoteKey);
    await finishRemote("Use JSON lines.", [{ seq: 1, at: now, tokens: { model: "claude-opus-5-5", input: 100_000 }, partial: false }]);
    expect(d1().run?.turns.find((t) => t.agentId === R)?.text).toBe("Use JSON lines.");
    expect(d1().costUsd).toBeGreaterThan(0);
    await finishLocal("b", "Keep JSON.");
    expect(d1().run?.phase).toBe("moderator");
    expect(d1().transcript.map((u) => u.agentId)).toEqual([R, "b"]);

    // 사회자가 원격 참가자를 지목한다 — 같은 그 머신 세션에 차례를 넣는다.
    await finishLocal("m", '{"action":"next","speakers":[{"id":"dev1:ra","question":"Why lines?"}]}');
    expect(calls("discuss-turn")).toHaveLength(1);
    expect(String(calls("discuss-turn")[0].text)).toContain("Why lines?");
    expect(calls("discuss-open")).toHaveLength(1);

    // 그 머신이 끊기면 멈추고, 다시 붙으면 스스로 이어 간다(세션은 그 머신에 남아 있다).
    setRemoteOffice(office(false));
    await finishRemote("Appends are cheap.");
    expect(d1()).toMatchObject({ state: "paused", run: { pauseReason: "link-offline", pauseDetail: "Studio" } });
    setRemoteOffice(office(true));
    await settle();
    expect(d1().state).toBe("running");
    await pollRemoteDiscussionSessions();
    await settle();
    expect(d1().transcript[d1().transcript.length - 1]).toMatchObject({ kind: "statement", agentId: R, text: "Appends are cheap." });
    expect(calls("discuss-turn")).toHaveLength(1);

    await finishLocal("m", '{"action":"wrap","summary":"JSON lines."}');
    expect(d1().state).toBe("review");
    expect(d1().decisionMemoPath).toBe("/data/discussions/memos/d1.md");
    const memo = api.saveDiscussionMemo.mock.calls[0][1] as string;
    expect(memo).toContain("Ra · Studio");
    expect(calls("discuss-close")).toHaveLength(1);
    expect(useRemoteDiscussionStore.getState().sessions).toEqual({});
  });

  it("그 머신 세션이 끝나면 그 차례에서 멈춘다", async () => {
    expect(startDiscussion("d1")).toBeUndefined();
    await settle();
    remoteState = { alive: false, turn: { phase: "ended", at: ++clock }, usage: [] };
    await pollRemoteDiscussionSessions();
    await settle();
    expect(d1()).toMatchObject({ state: "paused", run: { pauseReason: "session-ended", pauseDetail: "Ra · Studio" } });
  });

  it("회의실 분신은 원격 참가자 외형을 받는다", () => {
    const stage = officeDiscussionStage([{ ...d1(), state: "running" }], (id) => remoteParticipantProfile(id));
    expect(stage?.members.find((m) => m.agentId === R)?.profile).toMatchObject({ id: R, name: "Ra", seed: "ra-seed", archetype: "elf" });
    expect(stage?.members.find((m) => m.agentId === "b")?.profile).toBeUndefined();
  });
});
