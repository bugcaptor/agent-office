import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile, AutoRunJudgeResult, UsageSnapshot, WorkItem, WorkSendTarget } from "@shared/types";
const { api, restart, create, reserve, flushProfile } = vi.hoisted(() => ({
  api: {
    loadWorkItems: vi.fn(), saveWorkItems: vi.fn(), inspectWorkSendTarget: vi.fn(), sendWorkItem: vi.fn(),
    sendWorkFollowUp: vi.fn(), workLastReply: vi.fn(), autoRunReport: vi.fn(), autoRunJudge: vi.fn(), loadUsageSnapshot: vi.fn(),
  },
  restart: vi.fn(), create: vi.fn(), reserve: vi.fn(), flushProfile: vi.fn(),
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: api }));
vi.mock("../../ipc/sessionBridge", () => ({ reserveSessionReplacement: reserve, runGuardedCreateSession: create }));
vi.mock("../../agent/restartAgentSession", () => ({ restartAgentSession: restart }));
vi.mock("../../store/persist", () => ({ flushPersistence: flushProfile }));
vi.mock("../sessionSummaryStore", async (original) => ({ ...await original<object>(), linkWorkSession: vi.fn() }));
import { useAppStore } from "../../store/appStore";
import { useWorkItemStore } from "../workItemStore";
import { settleWorkSends, useWorkSendStore } from "../workSendStore";
import { useWorkMonitorStore } from "../workMonitor";
import { checkUsageWaits, installAutoRun, isAutoRunStepping, pauseAutoRun, resumeAutoRun, setAutoRunUsageGuard, startAutoRun } from "../autoRun";
import { emptyModel, windowKey, WAKE_GRACE_MS, type ForecastModel } from "../../usage/usageForecast";
import { recordTurnUsage, resetUsageForecast, usageForecastModel } from "../../usage/usageForecastStore";

const HOUR = 60 * 60 * 1000;
const initialApp = useAppStore.getState(); const initialWork = useWorkItemStore.getState();
const profile: AgentProfile = { id: "a", name: "Ada", role: "dev", seed: "s", createdAt: 1, deskIndex: 0, cwd: "/work" };
const shell: WorkSendTarget = { sessionId: "s", humanInputRevision: 0, restartRequired: false, hasUncommitted: false };
const judged = (decision: AutoRunJudgeResult["decision"], message = ""): AutoRunJudgeResult => ({ decision, message, note: "", costUsd: 0.01 });
let off: () => void = () => {};

function task(title: string, patch: Partial<WorkItem> = {}): string {
  const id = useWorkItemStore.getState().add({ title, body: `${title} body`, status: "todo" })!;
  useWorkItemStore.getState().edit(id, patch);
  return id;
}
function get(id: string): WorkItem { return useWorkItemStore.getState().items.find((i) => i.id === id)!; }
function plan() {
  const a = task("Schema"); const b = task("API", { dependsOn: [a] });
  const root = task("Ship feature", { assigneeAgentId: "a", dependsOn: [b], plan: true });
  return { a, b, root };
}
async function settled(root: string) { await vi.waitFor(() => expect(isAutoRunStepping(root)).toBe(false)); }
function turnEnded(id: string) { useWorkItemStore.getState().edit(id, { status: "review" }); }

/** Claude 5시간 창 하나뿐인 조회값. */
function usage(percent: number, resetsAtMs: number, fetchedAtMs = Date.now()): UsageSnapshot {
  const never = { outcome: "never_attempted", detail: null, lastAttemptMs: null, lastSuccessMs: null } as const;
  return {
    claude: { provider: "claude", fetchedAtMs, planLabel: null, windows: [{ kind: "session", label: null, usedPercent: percent, resetsAtMs, windowMinutes: 300, isActive: null }] },
    codex: null, opencode: null, openrouter: null,
    claudeLive: { ...never, tokenSource: null, via: null }, codexLive: never, opencodeLive: never, openrouterLive: never,
  };
}
/** 1달러 = 2%p, 최근 태스크 10달러, 후속 지시 한 턴 10달러. 다음 태스크는 +20%p로 본다. */
function learned(): ForecastModel {
  const model = emptyModel();
  model.windows[windowKey("claude", { kind: "session", label: null })] = { sumPercent: 200, sumCost: 100, at: Date.now() };
  model.tasks = [1, 2, 3].map(() => ({ provider: "claude" as const, cost: 10, judgeCost: 0, durationMs: HOUR, at: Date.now() }));
  model.followups = [1, 2].map(() => ({ provider: "claude" as const, cost: 10, durationMs: HOUR / 4, at: Date.now() }));
  return model;
}

beforeEach(() => {
  vi.clearAllMocks();
  api.loadWorkItems.mockResolvedValue([]); api.saveWorkItems.mockResolvedValue(undefined);
  api.inspectWorkSendTarget.mockResolvedValue(shell); api.sendWorkItem.mockResolvedValue({ sessionId: "s", submittedAt: 100 });
  api.workLastReply.mockResolvedValue("Done. Tests pass.");
  api.autoRunReport.mockResolvedValue(null);
  api.sendWorkFollowUp.mockResolvedValue({ sessionId: "s", submittedAt: Date.now() });
  restart.mockResolvedValue({ sessionId: "s", state: "running" }); reserve.mockReturnValue(vi.fn()); flushProfile.mockResolvedValue(undefined);
  useAppStore.setState(initialApp, true);
  useAppStore.setState({ agents: { a: profile }, agentOrder: ["a"], appSettings: { ...initialApp.appSettings, observerEnabled: true } });
  useWorkItemStore.setState({ ...initialWork, loaded: true }, true);
  useWorkSendStore.setState({ busy: {}, confirmation: null }); useWorkMonitorStore.setState({ sessions: {} });
  resetUsageForecast(Date.now() - HOUR, learned());
  off = installAutoRun();
});
afterEach(async () => { off(); await settleWorkSends(); await useWorkItemStore.getState().flush(); });

describe("자동 진행의 사용량 대기", () => {
  it("다음 태스크가 한도를 넘길 것 같으면 보내지 않고 리셋까지 기다렸다가 스스로 이어 간다", async () => {
    const { a, root } = plan();
    const reset = Date.now() + HOUR;
    // 80% + 20%p = 100% > 95%.
    useAppStore.setState({ usage: usage(80, reset) });
    expect(startAutoRun(root)).toBeUndefined();
    await vi.waitFor(() => expect(get(root).autoRun?.state).toBe("paused"));
    expect(api.sendWorkItem).not.toHaveBeenCalled();
    expect(get(a).status).toBe("todo");
    expect(get(root).autoRun).toMatchObject({ pauseReason: "usage-wait", usageWait: { provider: "claude", kind: "session", stage: "task", until: reset + WAKE_GRACE_MS, worker: "claude" } });
    expect(get(root).autoRun!.usageWait!.addPercent).toBeCloseTo(20);

    // 아직 리셋 전이고 여전히 모자라면 그대로 기다린다.
    await checkUsageWaits(Date.now());
    expect(get(root).autoRun?.state).toBe("paused");

    // 리셋 시각이 지나면 사용량을 새로 받아 다시 보고, 넉넉하면 이어 간다.
    api.loadUsageSnapshot.mockResolvedValue(usage(1, reset + 5 * HOUR, Date.now() + 1));
    await checkUsageWaits(reset + WAKE_GRACE_MS);
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    await settled(root);
    expect(get(root).autoRun).toMatchObject({ state: "running", currentId: a, usageWait: undefined });
    expect(get(root).autoRun!.log.map((e) => e.kind)).toEqual(["start", "pause", "resume", "send"]);
  });

  it("리셋 전이라도 새 조회값으로 넉넉해지면 일찍 이어 간다", async () => {
    const { root } = plan();
    useAppStore.setState({ usage: usage(80, Date.now() + HOUR) });
    startAutoRun(root);
    await vi.waitFor(() => expect(get(root).autoRun?.pauseReason).toBe("usage-wait"));
    // 창이 일찍 리셋됐다(조회값이 바뀌면 다시 본다).
    useAppStore.setState({ usage: usage(2, Date.now() + 5 * HOUR, Date.now() + 1) });
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    await settled(root);
  });

  it("사람이 기다리지 않고 계속하면 한 번만 건너뛰고, 다음 태스크 앞에서 다시 본다", async () => {
    const { a, b, root } = plan();
    useAppStore.setState({ usage: usage(80, Date.now() + HOUR) });
    api.autoRunJudge.mockResolvedValue(judged("next"));
    startAutoRun(root);
    await vi.waitFor(() => expect(get(root).autoRun?.pauseReason).toBe("usage-wait"));
    resumeAutoRun(root);
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    await settled(root);
    expect(get(root).autoRun!.usageOverride).toBeUndefined();
    turnEnded(a);
    await vi.waitFor(() => expect(get(root).autoRun?.pauseReason).toBe("usage-wait"));
    expect(get(a).status).toBe("done");
    expect(get(b).status).toBe("todo");
    expect(api.sendWorkItem).toHaveBeenCalledTimes(1);
    // 기다리는 중에 일시정지하면 스스로 이어 가지 않는다.
    pauseAutoRun(root);
    expect(get(root).autoRun).toMatchObject({ state: "paused", pauseReason: "user" });
    useAppStore.setState({ usage: usage(0, Date.now() + 5 * HOUR, Date.now() + 1) });
    await checkUsageWaits(Date.now() + 2 * HOUR);
    expect(get(root).autoRun?.state).toBe("paused");
  });

  it("후속 지시가 모자라면 지시를 남겨 두고 기다렸다가, 다시 판단하지 않고 보낸다", async () => {
    const { a, root } = plan();
    useAppStore.setState({ usage: usage(50, Date.now() + HOUR) });
    api.autoRunJudge.mockResolvedValueOnce(judged("followup", "Add the missing test"));
    startAutoRun(root);
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    await settled(root);
    // 태스크가 도는 사이 다른 곳에서 많이 썼다. 85% + 20%p(후속 한 턴) > 95%.
    useAppStore.setState({ usage: usage(85, Date.now() + HOUR, Date.now() + 1) });
    turnEnded(a);
    await vi.waitFor(() => expect(get(root).autoRun?.pauseReason).toBe("usage-wait"));
    expect(get(root).autoRun).toMatchObject({ pendingFollowUp: "Add the missing test", usageWait: { stage: "followup" } });
    expect(api.sendWorkFollowUp).not.toHaveBeenCalled();
    resumeAutoRun(root);
    await vi.waitFor(() => expect(api.sendWorkFollowUp).toHaveBeenCalledWith({ agentId: "a", sessionId: "s", text: "Add the missing test" }));
    await settled(root);
    expect(api.autoRunJudge).toHaveBeenCalledTimes(1);
    expect(get(root).autoRun).toMatchObject({ followUps: 1, pendingFollowUp: undefined });
    expect(get(a).status).toBe("in_progress");
  });

  it("판단자 호출 전에 Claude 한도에 이미 닿았으면 기다린다", async () => {
    const { a, root } = plan();
    useAppStore.setState({ usage: usage(50, Date.now() + HOUR) });
    startAutoRun(root);
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    await settled(root);
    useAppStore.setState({ usage: usage(97, Date.now() + HOUR, Date.now() + 1) });
    turnEnded(a);
    await vi.waitFor(() => expect(get(root).autoRun?.pauseReason).toBe("usage-wait"));
    expect(get(root).autoRun!.usageWait).toMatchObject({ stage: "judge", addPercent: null });
    expect(api.autoRunJudge).not.toHaveBeenCalled();
  });

  it("사용량 대기를 끄면 모자라도 보내고, 기다리는 중에 끄면 바로 이어 간다", async () => {
    const { root } = plan();
    useAppStore.setState({ usage: usage(99, Date.now() + HOUR) });
    startAutoRun(root);
    await vi.waitFor(() => expect(get(root).autoRun?.pauseReason).toBe("usage-wait"));
    setAutoRunUsageGuard(root, false);
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    await settled(root);
    expect(get(root).autoRun!.usageGuard).toBe(false);

    const other = task("Other", { assigneeAgentId: "a" });
    const root2 = task("Other plan", { assigneeAgentId: "a", dependsOn: [other], plan: true });
    useWorkItemStore.getState().setAutoRun(root, undefined);
    expect(startAutoRun(root2, { usageGuard: false })).toBeUndefined();
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(2));
    await settled(root2);
  });

  it("받아들인 태스크의 실제 토큰 비용과 판단자 비용을 다음 예측의 표본으로 남긴다", async () => {
    const { a, root } = plan();
    resetUsageForecast(Date.now() - HOUR, emptyModel());
    api.autoRunJudge.mockResolvedValueOnce(judged("followup", "More")).mockResolvedValue(judged("next"));
    startAutoRun(root);
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    await settled(root);
    // Opus 입력 1M 토큰 = 4달러. 첫 턴과 후속 지시 턴에서 한 번씩.
    const opus = { agentId: "a", sessionId: "s", at: Date.now(), partial: false, tokens: { model: "claude-opus-5-5", input: 1_000_000 } };
    recordTurnUsage(opus);
    turnEnded(a);
    await vi.waitFor(() => expect(api.sendWorkFollowUp).toHaveBeenCalledTimes(1));
    await settled(root);
    recordTurnUsage(opus);
    // 다른 캐릭터의 사용량은 이 태스크 비용이 아니다.
    recordTurnUsage({ ...opus, agentId: "z" });
    turnEnded(a);
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(2));
    await settled(root);
    const model = usageForecastModel();
    expect(model.tasks).toHaveLength(1);
    expect(model.tasks[0]).toMatchObject({ provider: "claude" });
    expect(model.tasks[0].cost).toBeCloseTo(8);
    expect(model.tasks[0].judgeCost).toBeCloseTo(0.02);
    expect(model.followups).toHaveLength(1);
    expect(model.followups[0].cost).toBeCloseTo(4);
  });
});
