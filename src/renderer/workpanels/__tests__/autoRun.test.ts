import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile, AutoRunJudgeResult, WorkItem, WorkSendTarget } from "@shared/types";
const { api, restart, create, reserve, flushProfile } = vi.hoisted(() => ({
  api: {
    loadWorkItems: vi.fn(), saveWorkItems: vi.fn(), inspectWorkSendTarget: vi.fn(), sendWorkItem: vi.fn(),
    sendWorkFollowUp: vi.fn(), workLastReply: vi.fn(), autoRunJudge: vi.fn(),
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
import { requestWorkSend, settleWorkSends, useWorkSendStore } from "../workSendStore";
import { useWorkMonitorStore } from "../workMonitor";
import { installAutoRun, isAutoRunStepping, judgePrompt, pauseAutoRun, planItems, resumeAutoRun, startAutoRun, startBlockedReason } from "../autoRun";

const initialApp = useAppStore.getState(); const initialWork = useWorkItemStore.getState();
const profile: AgentProfile = { id: "a", name: "Ada", role: "dev", seed: "s", createdAt: 1, deskIndex: 0, cwd: "/work" };
const other: AgentProfile = { ...profile, id: "b", name: "Bea", deskIndex: 1 };
const shell: WorkSendTarget = { sessionId: "s", humanInputRevision: 0, restartRequired: false, hasUncommitted: false };
const judged = (decision: AutoRunJudgeResult["decision"], message = "", note = ""): AutoRunJudgeResult => ({ decision, message, note, costUsd: 0.01 });
let off: () => void = () => {};

function task(title: string, patch: Partial<WorkItem> = {}): string {
  const id = useWorkItemStore.getState().add({ title, body: `${title} body`, status: "todo" })!;
  useWorkItemStore.getState().edit(id, patch);
  return id;
}
function get(id: string): WorkItem { return useWorkItemStore.getState().items.find((i) => i.id === id)!; }
/** 루트 R ← B ← A. 진행자는 a. */
function plan() {
  const a = task("Schema"); const b = task("API", { dependsOn: [a] });
  const root = task("Ship feature", { assigneeAgentId: "a", dependsOn: [b] });
  useWorkItemStore.getState().edit(root, { body: "Use Postgres." });
  return { a, b, root };
}
async function settled(root: string) { await vi.waitFor(() => expect(isAutoRunStepping(root)).toBe(false)); }
/** 모니터가 Stop을 받아 검토 대기로 옮긴 것과 같다. */
function turnEnded(id: string) { useWorkItemStore.getState().edit(id, { status: "review" }); }

beforeEach(() => {
  vi.clearAllMocks();
  api.loadWorkItems.mockResolvedValue([]); api.saveWorkItems.mockResolvedValue(undefined);
  api.inspectWorkSendTarget.mockResolvedValue(shell); api.sendWorkItem.mockResolvedValue({ sessionId: "s", submittedAt: 100 });
  api.workLastReply.mockResolvedValue("Done. Tests pass.");
  api.sendWorkFollowUp.mockResolvedValue({ sessionId: "s", submittedAt: Date.now() });
  restart.mockResolvedValue({ sessionId: "s", state: "running" }); reserve.mockReturnValue(vi.fn()); flushProfile.mockResolvedValue(undefined);
  useAppStore.setState(initialApp, true);
  useAppStore.setState({ agents: { a: profile, b: other }, agentOrder: ["a", "b"], appSettings: { ...initialApp.appSettings, observerEnabled: true } });
  useWorkItemStore.setState({ ...initialWork, loaded: true }, true);
  useWorkSendStore.setState({ busy: {}, confirmation: null }); useWorkMonitorStore.setState({ sessions: {} });
  off = installAutoRun();
});
afterEach(async () => { off(); await settleWorkSends(); await useWorkItemStore.getState().flush(); });

describe("자동 진행 계획", () => {
  it("루트가 간접으로 기다리는 업무까지 선행 작업 순서로 모은다", () => {
    const { a, b, root } = plan();
    const c = task("Docs"); useWorkItemStore.getState().edit(root, { dependsOn: [b, c] });
    task("Unrelated");
    const ids = planItems(get(root), useWorkItemStore.getState().items).map((i) => i.id);
    expect(ids).toHaveLength(3);
    expect(ids.indexOf(a)).toBeLessThan(ids.indexOf(b));
    expect(ids).toContain(c);
  });
  it("다른 캐릭터의 업무·관찰 꺼짐·남은 작업 없음이면 시작을 막는다", () => {
    const { a, b, root } = plan();
    const items = () => useWorkItemStore.getState().items;
    const agents = useAppStore.getState().agents;
    expect(startBlockedReason(get(root), items(), agents, true)).toBeUndefined();
    expect(startBlockedReason(get(root), items(), agents, false)).toBe("auto-run-observer-off");
    const other = task("Other plan", { assigneeAgentId: "a", dependsOn: [a] });
    useWorkItemStore.getState().setAutoRun(other, { state: "paused", judgeSessionId: "x", judgeStarted: false, followUps: 0, costUsd: 0, log: [], startedAt: 1 });
    expect(startBlockedReason(get(root), items(), agents, true)).toBe("auto-run-runner-busy");
    useWorkItemStore.getState().setAutoRun(other, undefined);
    useWorkItemStore.getState().edit(a, { assigneeAgentId: "b" });
    expect(startBlockedReason(get(root), items(), agents, true)).toBe("auto-run-foreign-task");
    useWorkItemStore.getState().edit(a, { status: "done" }); useWorkItemStore.getState().edit(b, { status: "done" });
    expect(startBlockedReason(get(root), items(), agents, true)).toBe("auto-run-no-plan");
  });
  it("판단자 첫 호출에만 규칙과 플랜을 넣고, 메모가 바뀌면 새 메모를 덧붙인다", () => {
    const { a, root } = plan();
    const base = { root: get(root), plan: planItems(get(root), useWorkItemStore.getState().items), task: get(a), followUps: 0, reply: "ok" };
    const first = judgePrompt({ ...base, first: true, memoChanged: true });
    expect(first).toContain("Use Postgres."); expect(first).toContain("1. Schema"); expect(first).toContain("Schema body");
    const later = judgePrompt({ ...base, followUps: 1, first: false, memoChanged: false });
    expect(later).not.toContain("Use Postgres."); expect(later).not.toContain("Schema body"); expect(later).toContain("ok");
    expect(judgePrompt({ ...base, first: false, memoChanged: true })).toContain("Use Postgres.");
  });
});

describe("자동 진행 루프", () => {
  it("태스크를 차례로 보내고 판단에 따라 완료·후속 지시·정지·마침을 적용한다", async () => {
    const { a, b, root } = plan();
    api.autoRunJudge
      .mockResolvedValueOnce(judged("next", "", "Tables live in db/schema.sql"))
      .mockResolvedValueOnce(judged("followup", "Add the missing test"))
      .mockResolvedValueOnce(judged("stop", "Needs a product decision"))
      .mockResolvedValueOnce(judged("next"));
    expect(startAutoRun(root)).toBeUndefined();
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    await settled(root);
    // 첫 태스크는 진행자에게 배정되고 플랜 메모가 붙어 나간다.
    expect(get(a)).toMatchObject({ status: "in_progress", assigneeAgentId: "a" });
    expect(api.sendWorkItem.mock.calls[0][0].body).toContain("Use Postgres.");

    turnEnded(a);
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(2));
    await settled(root);
    expect(api.autoRunJudge.mock.calls[0][0]).toMatchObject({ resume: false });
    expect(api.workLastReply).toHaveBeenCalledWith("a", "claude", "/work");
    expect(get(a).status).toBe("done");
    // 다음 태스크는 앞 태스크의 인계 메모를 받는다.
    expect(api.sendWorkItem.mock.calls[1][0].body).toContain("Tables live in db/schema.sql");
    expect(get(root).autoRun).toMatchObject({ currentId: b, judgeStarted: true });

    turnEnded(b);
    await vi.waitFor(() => expect(api.sendWorkFollowUp).toHaveBeenCalledTimes(1));
    await settled(root);
    expect(api.sendWorkFollowUp).toHaveBeenCalledWith({ agentId: "a", sessionId: "s", text: "Add the missing test" });
    expect(api.autoRunJudge.mock.calls[1][0]).toMatchObject({ resume: true, sessionId: get(root).autoRun!.judgeSessionId });
    expect(get(b).status).toBe("in_progress");
    expect(get(root).autoRun!.followUps).toBe(1);

    turnEnded(b);
    await vi.waitFor(() => expect(get(root).autoRun?.state).toBe("paused"));
    expect(get(root).autoRun).toMatchObject({ pauseReason: "judge-stop", pauseDetail: "Needs a product decision" });

    resumeAutoRun(root);
    await vi.waitFor(() => expect(get(root).autoRun?.state).toBe("finished"));
    expect(get(b).status).toBe("done");
    expect(get(root).status).toBe("review");
    expect(get(root).autoRun!.costUsd).toBeCloseTo(0.04);
    expect(get(root).autoRun!.log.map((e) => e.kind)).toEqual(["start", "send", "next", "send", "followup", "pause", "resume", "next", "finish"]);
  });

  it("CLI가 돌고 있으면 확인 없이 재시작하되 미제출 입력이 있으면 건드리지 않고 멈춘다", async () => {
    const { a, root } = plan();
    api.inspectWorkSendTarget.mockResolvedValue({ ...shell, restartRequired: true, hasUncommitted: true });
    startAutoRun(root);
    await vi.waitFor(() => expect(get(root).autoRun?.state).toBe("paused"));
    expect(get(root).autoRun).toMatchObject({ pauseReason: "human-input", currentId: undefined });
    expect(restart).not.toHaveBeenCalled(); expect(useWorkSendStore.getState().confirmation).toBeNull();
    expect(get(a).status).toBe("todo");

    api.inspectWorkSendTarget.mockResolvedValueOnce({ ...shell, restartRequired: true }).mockResolvedValue(shell);
    resumeAutoRun(root);
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    expect(restart).toHaveBeenCalledTimes(1); expect(useWorkSendStore.getState().confirmation).toBeNull();
  });

  it("판단자 첫 호출이 실패하면 새 세션 id로 바꿔 멈추고, 사람 입력 중에는 후속 지시를 넣지 않는다", async () => {
    const { a, root } = plan();
    api.autoRunJudge.mockRejectedValueOnce(new Error("claude-not-found")).mockResolvedValueOnce(judged("followup", "Fix lint"));
    startAutoRun(root);
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    await settled(root);
    const firstId = get(root).autoRun!.judgeSessionId;
    turnEnded(a);
    await vi.waitFor(() => expect(get(root).autoRun?.state).toBe("paused"));
    expect(get(root).autoRun).toMatchObject({ pauseReason: "judge-failed", pauseDetail: "claude-not-found", judgeStarted: false });
    expect(get(root).autoRun!.judgeSessionId).not.toBe(firstId);

    api.sendWorkFollowUp.mockRejectedValueOnce(new Error("work-send-human-input"));
    resumeAutoRun(root);
    await vi.waitFor(() => expect(get(root).autoRun?.pauseReason).toBe("human-input"));
    expect(api.autoRunJudge.mock.calls[1][0]).toMatchObject({ resume: false });
    expect(get(a).status).toBe("review");
    expect(get(root).autoRun!.followUps).toBe(0);
  });

  it("진행 중에는 사람이 진행자에게 다른 업무를 보낼 수 없고, 일시정지하면 다시 보낼 수 있다", async () => {
    const { root } = plan();
    const side = task("Side job", { assigneeAgentId: "a" });
    startAutoRun(root);
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    await settled(root);
    await requestWorkSend(side);
    expect(get(side).lastError).toBe("work-send-auto-run");
    pauseAutoRun(root);
    await requestWorkSend(side);
    expect(api.sendWorkItem).toHaveBeenCalledTimes(2);
  });

  it("사람이 현재 태스크를 완료로 바꾸면 판단 없이 다음으로 넘어가고, 할 일로 되돌리면 멈춘다", async () => {
    const { a, b, root } = plan();
    startAutoRun(root);
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    await settled(root);
    useWorkItemStore.getState().edit(a, { status: "done" });
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(2));
    await settled(root);
    expect(api.autoRunJudge).not.toHaveBeenCalled();
    useWorkItemStore.getState().edit(b, { status: "todo" });
    await vi.waitFor(() => expect(get(root).autoRun?.pauseReason).toBe("task-changed"));
  });

  it("세션이 끝나면 멈춘다", async () => {
    const { root } = plan();
    startAutoRun(root);
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    await settled(root);
    useWorkMonitorStore.setState({ sessions: { s: { phase: "ended", at: Date.now(), agentId: "a", transition: false } } });
    await vi.waitFor(() => expect(get(root).autoRun?.pauseReason).toBe("session-ended"));
  });
});

describe("자동 진행 복원", () => {
  it("앱을 다시 켜면 돌던 자동 진행은 일시정지로 불러온다", async () => {
    const run = { state: "running" as const, judgeSessionId: "x", judgeStarted: true, followUps: 0, costUsd: 0, log: [], startedAt: 1 };
    api.loadWorkItems.mockResolvedValue([{ id: "r", title: "Root", body: "", status: "todo", order: 0, sessionIds: [], createdAt: 1, updatedAt: 1, autoRun: run }]);
    useWorkItemStore.setState({ ...initialWork }, true);
    await useWorkItemStore.getState().load();
    expect(get("r").autoRun).toMatchObject({ state: "paused", pauseReason: "app-restarted" });
    expect(useWorkItemStore.getState().dirty).toBe(true);
  });
});
