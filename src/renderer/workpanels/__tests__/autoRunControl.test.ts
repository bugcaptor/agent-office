import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile, AutoRun, WorkItem, WorkSendTarget } from "@shared/types";
const { api, notify } = vi.hoisted(() => ({
  api: {
    loadWorkItems: vi.fn(), saveWorkItems: vi.fn(), inspectWorkSendTarget: vi.fn(), sendWorkItem: vi.fn(),
    sendWorkFollowUp: vi.fn(), workLastReply: vi.fn(), autoRunReport: vi.fn(), autoRunJudge: vi.fn(),
  },
  notify: vi.fn(),
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: api }));
vi.mock("../../ipc/osNotify", () => ({ maybeSendOsNotification: notify }));
vi.mock("../../ipc/sessionBridge", () => ({ reserveSessionReplacement: vi.fn(() => vi.fn()), runGuardedCreateSession: vi.fn() }));
vi.mock("../../agent/restartAgentSession", () => ({ restartAgentSession: vi.fn(async () => ({ sessionId: "s", state: "running" })) }));
vi.mock("../../store/persist", () => ({ flushPersistence: vi.fn(async () => undefined) }));
vi.mock("../sessionSummaryStore", async (original) => ({ ...await original<object>(), linkWorkSession: vi.fn() }));
import { useAppStore } from "../../store/appStore";
import { useWorkItemStore } from "../workItemStore";
import { settleWorkSends, useWorkSendStore } from "../workSendStore";
import { useWorkMonitorStore } from "../workMonitor";
import { checkPermissionWaits, ctlResumesSinceSend, isAutoRunStepping, MAX_CTL_RESUMES, PERMISSION_TIMEOUT_MS } from "../autoRun";
import { handleAutoRunControl } from "../autoRunControl";

const initialApp = useAppStore.getState(); const initialWork = useWorkItemStore.getState();
const profile: AgentProfile = { id: "a", name: "Ada", role: "dev", seed: "s", createdAt: 1, deskIndex: 0, cwd: "/work" };
const shell: WorkSendTarget = { sessionId: "s", humanInputRevision: 0, restartRequired: false, hasUncommitted: false };

function task(title: string, patch: Partial<WorkItem> = {}): string {
  const id = useWorkItemStore.getState().add({ title, body: `${title} body`, status: "todo" })!;
  useWorkItemStore.getState().edit(id, patch);
  return id;
}
function get(id: string): WorkItem { return useWorkItemStore.getState().items.find((i) => i.id === id)!; }
function lastLog(id: string) { const log = get(id).autoRun!.log; return log[log.length - 1]; }
function plan() {
  const step = task("Schema");
  const root = task("Ship", { assigneeAgentId: "a", dependsOn: [step], plan: true });
  return { step, root };
}
/** 단계 하나가 세션 s1에서 진행 중인 실행. */
function inFlight(patch: Partial<AutoRun> = {}) {
  const { step, root } = plan();
  useWorkItemStore.getState().edit(step, { status: "in_progress", assigneeAgentId: "a" });
  useWorkItemStore.setState((s) => ({ items: s.items.map((i) => (i.id === step ? { ...i, sessionIds: ["s1"] } : i)) }));
  const run: AutoRun = { state: "running", judgeSessionId: "j", judgeStarted: false, currentId: step, followUps: 0, costUsd: 0, startedAt: 1,
    log: [{ at: 1, kind: "start", text: "ctl" }, { at: 2, kind: "send", taskId: step }], ...patch };
  useWorkItemStore.getState().setAutoRun(root, run);
  return { step, root };
}
const control = (op: "start" | "resume" | "pause" | "end" | "escalate", rootId: string, options = {}) =>
  handleAutoRunControl({ requestId: "r", op, rootId, options });

beforeEach(() => {
  vi.clearAllMocks();
  api.loadWorkItems.mockResolvedValue([]); api.saveWorkItems.mockResolvedValue(undefined);
  api.inspectWorkSendTarget.mockResolvedValue(shell); api.sendWorkItem.mockResolvedValue({ sessionId: "s", submittedAt: 100 });
  useAppStore.setState(initialApp, true);
  useAppStore.setState({ agents: { a: profile }, agentOrder: ["a"], appSettings: { ...initialApp.appSettings, observerEnabled: true } });
  useWorkItemStore.setState({ ...initialWork, loaded: true }, true);
  useWorkSendStore.setState({ busy: {}, confirmation: null }); useWorkMonitorStore.setState({ sessions: {} });
});
afterEach(async () => { await settleWorkSends(); await useWorkItemStore.getState().flush(); });

describe("CLI 자동 진행 제어", () => {
  it("CLI 시작은 기록에 표시를 남기고, 도는 실행은 다시 시작하지 않는다", async () => {
    const { root } = plan();
    expect(control("start", "nope")).toBe("auto-run-no-plan");
    expect(control("start", root, { usageGuard: false })).toBeUndefined();
    const run = get(root).autoRun!;
    expect(run.log[0]).toMatchObject({ kind: "start", text: "ctl" });
    expect(run.usageGuard).toBe(false);
    expect(control("start", root)).toBe("auto-run-already-running");
    await vi.waitFor(() => expect(isAutoRunStepping(root)).toBe(false));
  });

  it("사람이 멈추거나 넘긴 실행은 CLI가 이어 가지 못하고, CLI가 멈춘 것은 이어 간다", () => {
    for (const pauseReason of ["user", "escalated", "human-input"] as const) {
      useWorkItemStore.setState({ ...initialWork, loaded: true }, true);
      const { root } = inFlight({ state: "paused", pauseReason });
      expect(control("resume", root)).toBe("auto-run-paused-by-human");
      expect(get(root).autoRun!.state).toBe("paused");
    }
    useWorkItemStore.setState({ ...initialWork, loaded: true }, true);
    const { root } = inFlight();
    expect(control("pause", root)).toBeUndefined();
    expect(get(root).autoRun!.pauseReason).toBe("ctl-pause");
    expect(control("resume", root)).toBeUndefined();
    expect(get(root).autoRun!.state).toBe("running");
    expect(lastLog(root)).toMatchObject({ kind: "resume", text: "ctl" });
    expect(control("resume", root)).toBe("auto-run-not-paused");
  });

  it("같은 단계에서 CLI 재개는 정해진 횟수까지만 되고, 사람이 이어 간 것은 세지 않는다", () => {
    const resumes = Array.from({ length: MAX_CTL_RESUMES }, (_, n) => [
      { at: 10 + n * 2, kind: "pause" as const, text: "judge-failed" }, { at: 11 + n * 2, kind: "resume" as const, text: "ctl" },
    ]).flat();
    const { root, step } = inFlight({ state: "paused", pauseReason: "judge-failed",
      log: [{ at: 1, kind: "start" }, { at: 2, kind: "send", taskId: "x" }, { at: 3, kind: "resume" }, ...resumes] });
    expect(ctlResumesSinceSend(get(root).autoRun!)).toBe(MAX_CTL_RESUMES);
    expect(control("resume", root)).toBe("auto-run-recovery-limit");
    // 다음 단계로 넘어가면 다시 셀 수 있다.
    useWorkItemStore.getState().setAutoRun(root, { ...get(root).autoRun!, log: [...get(root).autoRun!.log, { at: 99, kind: "send", taskId: step }] });
    expect(control("resume", root)).toBeUndefined();
  });

  it("넘기기는 멈추고 메시지를 남기며 OS 알림을 보낸다. 끝난 실행은 넘길 수 없다", () => {
    const { root } = inFlight();
    expect(control("escalate", root, { message: "마이그레이션을 되돌릴지 정해 주세요" })).toBeUndefined();
    const run = get(root).autoRun!;
    expect(run).toMatchObject({ state: "paused", pauseReason: "escalated", pauseDetail: "마이그레이션을 되돌릴지 정해 주세요" });
    expect(notify).toHaveBeenCalledWith(expect.stringContaining("Ship"), "마이그레이션을 되돌릴지 정해 주세요");
    useWorkItemStore.getState().setAutoRun(root, { ...run, state: "finished" });
    expect(control("escalate", root, { message: "x" })).toBe("auto-run-finished");
    expect(control("end", root)).toBeUndefined();
    expect(get(root).autoRun).toBeUndefined();
    expect(control("escalate", root, { message: "x" })).toBe("auto-run-no-run");
  });
});

describe("권한 승인 대기 시간 초과", () => {
  it("승인 대기가 길어지면 멈추고, 계속을 누른 뒤로는 다시 잰다", () => {
    const now = 10_000_000;
    const { root } = inFlight();
    useWorkMonitorStore.setState({ sessions: { s1: { phase: "waiting", at: now - PERMISSION_TIMEOUT_MS + 1000, agentId: "a", transition: false } } });
    checkPermissionWaits(now);
    expect(get(root).autoRun!.state).toBe("running");
    checkPermissionWaits(now + 1000);
    expect(get(root).autoRun).toMatchObject({ state: "paused", pauseReason: "permission-timeout", pauseDetail: "Schema" });

    // CLI가 이어 가면 그때부터 다시 잰다.
    expect(control("resume", root)).toBeUndefined();
    const resumedAt = lastLog(root).at;
    checkPermissionWaits(resumedAt + PERMISSION_TIMEOUT_MS - 1);
    expect(get(root).autoRun!.state).toBe("running");
    // 작업 중이면 재지 않는다.
    useWorkMonitorStore.setState({ sessions: { s1: { phase: "working", at: 1, agentId: "a", transition: false } } });
    checkPermissionWaits(resumedAt + PERMISSION_TIMEOUT_MS * 3);
    expect(get(root).autoRun!.state).toBe("running");
  });
});
