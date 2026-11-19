import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile, WorkItem, WorkSchedule, WorkSendTarget } from "@shared/types";
const { api, restart, create, reserve, flushProfile, notify } = vi.hoisted(() => ({
  api: {
    loadWorkItems: vi.fn(), saveWorkItems: vi.fn(), inspectWorkSendTarget: vi.fn(), sendWorkItem: vi.fn(),
    sendWorkFollowUp: vi.fn(), workLastReply: vi.fn(), autoRunReport: vi.fn(), autoRunJudge: vi.fn(),
  },
  restart: vi.fn(), create: vi.fn(), reserve: vi.fn(), flushProfile: vi.fn(), notify: vi.fn(),
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: api }));
vi.mock("../../ipc/sessionBridge", () => ({ reserveSessionReplacement: reserve, runGuardedCreateSession: create }));
vi.mock("../../agent/restartAgentSession", () => ({ restartAgentSession: restart }));
vi.mock("../../store/persist", () => ({ flushPersistence: flushProfile }));
vi.mock("../../ipc/osNotify", () => ({ maybeSendOsNotification: notify }));
vi.mock("../sessionSummaryStore", async (original) => ({ ...await original<object>(), linkWorkSession: vi.fn() }));
import { useAppStore } from "../../store/appStore";
import { useWorkItemStore } from "../workItemStore";
import { requestWorkSend, settleWorkSends, useWorkSendStore } from "../workSendStore";
import { useWorkMonitorStore } from "../workMonitor";
import { isAutoRunStepping, startBlockedReason } from "../autoRun";
import { checkSchedules, MISSED_GRACE_MS } from "../scheduleRunner";

const initialApp = useAppStore.getState(); const initialWork = useWorkItemStore.getState();
const profile: AgentProfile = { id: "a", name: "Ada", role: "dev", seed: "s", createdAt: 1, deskIndex: 0, cwd: "/work" };
const shell: WorkSendTarget = { sessionId: "s", humanInputRevision: 0, restartRequired: false, hasUncommitted: false };
const NOW = new Date(2026, 10, 19, 9, 0, 30).getTime();
const NINE = new Date(2026, 10, 19, 9, 0).getTime();
const daily = (patch: Partial<WorkSchedule> = {}): WorkSchedule => ({ rule: { kind: "daily", time: "09:00" }, enabled: true, nextAt: NINE, ...patch });

function task(title: string, patch: Partial<WorkItem> = {}, schedule?: WorkSchedule): string {
  const id = useWorkItemStore.getState().add({ title, body: `${title} body`, status: "todo", assigneeAgentId: "a" })!;
  useWorkItemStore.getState().edit(id, patch);
  if (schedule) useWorkItemStore.getState().setSchedule(id, schedule);
  return id;
}
const items = () => useWorkItemStore.getState().items;
const get = (id: string) => items().find((i) => i.id === id)!;
const runsOf = (id: string) => items().filter((i) => i.scheduledFrom === id);

beforeEach(() => {
  vi.clearAllMocks();
  api.loadWorkItems.mockResolvedValue([]); api.saveWorkItems.mockResolvedValue(undefined);
  api.inspectWorkSendTarget.mockResolvedValue(shell); api.sendWorkItem.mockResolvedValue({ sessionId: "s", submittedAt: 100 });
  api.autoRunReport.mockResolvedValue(null);
  restart.mockResolvedValue({ sessionId: "s", state: "running" }); reserve.mockReturnValue(vi.fn()); flushProfile.mockResolvedValue(undefined);
  useAppStore.setState(initialApp, true);
  useAppStore.setState({ agents: { a: profile }, agentOrder: ["a"], appSettings: { ...initialApp.appSettings, observerEnabled: true, workPanelsEnabled: true } });
  useWorkItemStore.setState({ ...initialWork, loaded: true }, true);
  useWorkSendStore.setState({ busy: {}, confirmation: null }); useWorkMonitorStore.setState({ sessions: {} });
});
afterEach(async () => { await settleWorkSends(); await useWorkItemStore.getState().flush(); });

describe("반복 예약", () => {
  it("템플릿을 복제해 보내고 다음 회차를 먼저 저장한다", async () => {
    const id = task("Daily check", {}, daily());
    await checkSchedules(NOW);
    const [run] = runsOf(id);
    expect(run).toMatchObject({ title: "Daily check", status: "in_progress", sessionIds: ["s"], assigneeAgentId: "a" });
    expect(api.sendWorkItem).toHaveBeenCalledWith(expect.objectContaining({ title: "Daily check" }));
    expect(get(id)).toMatchObject({ status: "todo", sessionIds: [] });
    expect(get(id).schedule).toEqual({ ...daily(), nextAt: new Date(2026, 10, 20, 9).getTime(), lastRunAt: NINE });
    // 보내기 전 저장의 스냅샷에 이미 다음 회차가 들어 있다.
    const firstSave = api.saveWorkItems.mock.calls[0][0] as WorkItem[];
    expect(firstSave.find((i) => i.id === id)!.schedule!.nextAt).toBe(new Date(2026, 10, 20, 9).getTime());
    expect(notify).not.toHaveBeenCalled();
  });
  it("놓친 회차는 실행하지 않고 건너뛰어 알린다", async () => {
    const id = task("Daily check", {}, daily());
    await checkSchedules(NINE + MISSED_GRACE_MS + 1);
    expect(runsOf(id)).toEqual([]);
    expect(api.sendWorkItem).not.toHaveBeenCalled();
    expect(get(id).schedule).toMatchObject({ enabled: true, nextAt: new Date(2026, 10, 20, 9).getTime(), lastSkip: { at: NINE, reason: "missed" } });
    expect(notify).toHaveBeenCalledTimes(1);
  });
  it("지난 회차가 안 끝났으면 건너뛰고, 같은 이유가 이어지면 처음만 알린다", async () => {
    const id = task("Daily check", {}, daily());
    const open = task("old run"); useWorkItemStore.setState({ items: items().map((i) => i.id === open ? { ...i, scheduledFrom: id } : i) });
    await checkSchedules(NOW);
    expect(get(id).schedule!.lastSkip).toEqual({ at: NINE, reason: "previous-open" });
    const next = get(id).schedule!.nextAt!;
    await checkSchedules(next + 1000);
    expect(get(id).schedule!.lastSkip).toEqual({ at: next, reason: "previous-open" });
    expect(notify).toHaveBeenCalledTimes(1);
    expect(api.sendWorkItem).not.toHaveBeenCalled();
  });
  it("CLI가 떠 있는 세션은 턴이 끝난 것을 본 때만 새 세션으로 바꾼다", async () => {
    const id = task("Daily check", {}, daily());
    const live: WorkSendTarget = { sessionId: "s0", humanInputRevision: 0, restartRequired: true, hasUncommitted: false };
    api.inspectWorkSendTarget.mockResolvedValue(live);
    const retry = async () => { useWorkItemStore.getState().setSchedule(id, daily()); await checkSchedules(NOW); return get(id).schedule!.lastSkip; };
    // 앱을 다시 켠 뒤처럼 이 세션의 신호가 아직 없다.
    expect(await retry()).toEqual({ at: NINE, reason: "agent-unknown" });
    useWorkMonitorStore.setState({ sessions: { s0: { phase: "working", at: 1, agentId: "a", transition: true } } });
    expect(await retry()).toEqual({ at: NINE, reason: "agent-working" });
    useWorkMonitorStore.setState({ sessions: { s0: { phase: "waiting", at: 2, agentId: "a", transition: false } } });
    expect(await retry()).toEqual({ at: NINE, reason: "agent-working" });
    api.inspectWorkSendTarget.mockResolvedValue({ ...live, hasUncommitted: true });
    useWorkMonitorStore.setState({ sessions: { s0: { phase: "review", at: 3, agentId: "a", transition: true } } });
    expect(await retry()).toEqual({ at: NINE, reason: "blocked", detail: "work-send-human-input" });
    expect(restart).not.toHaveBeenCalled();
    expect(runsOf(id)).toEqual([]);
    // 턴이 끝난 세션이면 자동 진행처럼 새 세션으로 바꿔 보낸다.
    api.inspectWorkSendTarget.mockResolvedValueOnce(live).mockResolvedValueOnce(live).mockResolvedValueOnce(live).mockResolvedValue(shell);
    useWorkItemStore.getState().setSchedule(id, daily());
    await checkSchedules(NOW);
    expect(restart).toHaveBeenCalledTimes(1);
    expect(runsOf(id)[0]).toMatchObject({ status: "in_progress", sessionIds: ["s"] });
  });
  it("관찰이 꺼져 있으면 건너뛰고, 업무 패널이 꺼져 있으면 실행하지 않는다", async () => {
    const id = task("Daily check", {}, daily());
    useAppStore.setState({ appSettings: { ...useAppStore.getState().appSettings, workPanelsEnabled: false } });
    await checkSchedules(NOW);
    expect(get(id).schedule).toEqual(daily());
    useAppStore.setState({ appSettings: { ...useAppStore.getState().appSettings, workPanelsEnabled: true, observerEnabled: false } });
    await checkSchedules(NOW);
    expect(get(id).schedule!.lastSkip).toEqual({ at: NINE, reason: "blocked", detail: "auto-run-observer-off" });
    expect(api.sendWorkItem).not.toHaveBeenCalled();
  });
  it("다른 보내기에 합류해 세션 없이 끝나도 실패로 보고 복제본을 지운다", async () => {
    const id = task("Daily check", {}, daily());
    const other = task("Manual");
    let release: (value: unknown) => void = () => {};
    api.sendWorkItem.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
    const manual = requestWorkSend(other);
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    useWorkSendStore.setState({ busy: {} }); // 점검을 지나 합류 경로까지 가게 한다.
    const fired = checkSchedules(NOW);
    await vi.waitFor(() => expect(runsOf(id)).toHaveLength(1));
    release({ sessionId: "s", submittedAt: 100 });
    await manual; await fired;
    expect(runsOf(id)).toEqual([]);
    expect(get(id).schedule!.lastSkip).toMatchObject({ reason: "failed", detail: "work-send-busy" });
  });
  it("보내지 못한 복제본은 지우고 이유를 남긴다", async () => {
    const id = task("Daily check", {}, daily());
    api.sendWorkItem.mockRejectedValue(new Error("work-send-launch-timeout"));
    await checkSchedules(NOW);
    expect(runsOf(id)).toEqual([]);
    expect(get(id).schedule).toMatchObject({ enabled: true, lastSkip: { at: NINE, reason: "failed", detail: "work-send-launch-timeout" } });
    expect(notify).toHaveBeenCalledTimes(1);
  });
  it("플랜 템플릿은 루트와 단계를 복제해 자동 진행으로 실행한다", async () => {
    const a = task("Collect"); const b = task("Report", { dependsOn: [a] });
    const root = task("Weekly report", { dependsOn: [b], plan: true }, daily());
    await checkSchedules(NOW);
    const runs = runsOf(root);
    expect(runs).toHaveLength(3);
    const runRoot = runs.find((i) => i.plan)!;
    const runB = runs.find((i) => i.title === "Report")!;
    const runA = runs.find((i) => i.title === "Collect")!;
    expect(runRoot.dependsOn).toEqual([runB.id]);
    expect(runB.dependsOn).toEqual([runA.id]);
    expect(get(runRoot.id).autoRun?.state).toBeDefined();
    await vi.waitFor(() => expect(isAutoRunStepping(runRoot.id)).toBe(false));
    expect(api.sendWorkItem).toHaveBeenCalledWith(expect.objectContaining({ title: "Collect" }));
    expect([a, b, root].map((id) => get(id).status)).toEqual(["todo", "todo", "todo"]);
    expect(get(root).autoRun).toBeUndefined();
  });
  it("템플릿과 그 단계는 직접 보내거나 자동 진행할 수 없다", async () => {
    const a = task("Collect");
    const root = task("Weekly report", { dependsOn: [a], plan: true }, daily({ enabled: false, nextAt: undefined }));
    expect(startBlockedReason(get(root), items(), useAppStore.getState().agents, true)).toBe("auto-run-schedule-template");
    await requestWorkSend(a);
    expect(get(a).lastError).toBe("work-send-schedule-template");
    expect(api.sendWorkItem).not.toHaveBeenCalled();
  });
});

describe("한 번 예약", () => {
  const once = (): WorkSchedule => ({ rule: { kind: "once", at: NINE }, enabled: true, nextAt: NINE });
  it("그 업무 자체를 보내고 예약을 지운다", async () => {
    const id = task("Release", {}, once());
    await checkSchedules(NOW);
    expect(get(id)).toMatchObject({ status: "in_progress", sessionIds: ["s"] });
    expect(get(id).schedule).toBeUndefined();
    expect(runsOf(id)).toEqual([]);
  });
  it("할 일이 아니면 끄고 이유를 남긴다", async () => {
    const id = task("Release", { status: "review" }, once());
    await checkSchedules(NOW);
    expect(get(id).schedule).toEqual({ rule: once().rule, enabled: false, lastSkip: { at: NINE, reason: "blocked", detail: "schedule-not-todo" } });
    expect(api.sendWorkItem).not.toHaveBeenCalled();
  });
  it("완료하면 예약이 꺼진다", () => {
    const id = task("Release", {}, once());
    useWorkItemStore.getState().edit(id, { status: "done" });
    expect(get(id).schedule).toEqual({ rule: once().rule, enabled: false });
  });
});

describe("사람만 예약을 켠다", () => {
  it("수신함이 만들거나 바꾼 예약은 꺼진 채로 둔다", () => {
    useWorkItemStore.getState().importInbox([{ id: "t-1", title: "From agent", body: "", scheduleRule: { kind: "interval", minutes: 30 } }]);
    expect(get("t-1").schedule).toEqual({ rule: { kind: "interval", minutes: 30 }, enabled: false });
    const id = task("Daily check", {}, daily());
    useWorkItemStore.getState().applyInbox([{ kind: "edit", id, patch: { scheduleRule: { kind: "daily", time: "10:00" } } }]);
    expect(get(id).schedule).toMatchObject({ rule: { kind: "daily", time: "10:00" }, enabled: false });
    expect(get(id).schedule!.nextAt).toBeUndefined();
    useWorkItemStore.getState().applyInbox([{ kind: "edit", id, patch: { scheduleRule: null } }]);
    expect(get(id).schedule).toBeUndefined();
  });
  it("수신함이 켜진 템플릿이나 그 단계의 내용을 고치면 끈다. 상태만 바꾸면 그대로 둔다", () => {
    const a = task("Collect");
    const root = task("Weekly report", { dependsOn: [a], plan: true }, daily());
    useWorkItemStore.getState().applyInbox([{ kind: "edit", id: a, patch: { status: "review" } }]);
    expect(get(root).schedule!.enabled).toBe(true);
    useWorkItemStore.getState().applyInbox([{ kind: "edit", id: a, patch: { body: "rm -rf" } }]);
    expect(get(root).schedule!.enabled).toBe(false);
    useWorkItemStore.getState().setSchedule(root, daily());
    useWorkItemStore.getState().applyInbox([{ kind: "delete", ids: [a] }]);
    expect(get(root).schedule!.enabled).toBe(false);
  });
  it("외부에서 고친 파일의 예약은 켜지지 않고, 켜진 예약의 내용을 바꾸면 끈다", async () => {
    const id = task("Daily check", {}, daily());
    const other = task("Other");
    await useWorkItemStore.getState().flush();
    const base = items();
    api.loadWorkItems.mockResolvedValue([
      ...base.map((i) => i.id === id ? { ...i, body: "changed outside" } : i.id === other ? { ...i, schedule: daily() } : i),
      { ...base[0], id: "ext", title: "External", schedule: daily() },
    ]);
    expect(await useWorkItemStore.getState().refresh()).toBe(true);
    expect(get(id).schedule!.enabled).toBe(false);
    expect(get(other).schedule!.enabled).toBe(false);
    expect(get("ext").schedule!.enabled).toBe(false);
  });
});
