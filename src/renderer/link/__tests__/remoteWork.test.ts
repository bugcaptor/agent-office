import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile, AutoRunJudgeResult, LinkTaskEvent, RemoteOffice, UsageSnapshot, WorkItem } from "@shared/types";
const { api } = vi.hoisted(() => ({
  api: {
    loadWorkItems: vi.fn(), saveWorkItems: vi.fn(), inspectWorkSendTarget: vi.fn(), sendWorkItem: vi.fn(),
    sendWorkFollowUp: vi.fn(), workLastReply: vi.fn(), autoRunReport: vi.fn(), autoRunJudge: vi.fn(),
    linkWork: vi.fn(), onLinkTask: vi.fn(() => () => {}),
  },
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: api }));
vi.mock("../../ipc/sessionBridge", () => ({ reserveSessionReplacement: vi.fn(), runGuardedCreateSession: vi.fn() }));
vi.mock("../../agent/restartAgentSession", () => ({ restartAgentSession: vi.fn() }));
vi.mock("../../store/persist", () => ({ flushPersistence: vi.fn() }));
vi.mock("../../workpanels/sessionSummaryStore", async (original) => ({ ...await original<object>(), linkWorkSession: vi.fn() }));
import { useAppStore } from "../../store/appStore";
import { useWorkItemStore } from "../../workpanels/workItemStore";
import { settleWorkSends, useWorkSendStore } from "../../workpanels/workSendStore";
import { useWorkMonitorStore } from "../../workpanels/workMonitor";
import { installAutoRun, isAutoRunStepping, startAutoRun, startBlockedReason } from "../../workpanels/autoRun";
import { setRemoteOffice } from "../remoteFloors";
import { applyLinkTask, normalizeRepoUrl, remoteSendBlocked } from "../remoteWork";

const DEV = "d-1";
const ADA = `${DEV}:ada`;
const office = (online: boolean, repos = ["github.com/acme/app"]): RemoteOffice => ({ machines: [{
  deviceId: DEV, name: "Laptop", online, repos,
  characters: [{ id: ADA, agentId: "ada", name: "Ada", role: "", appearance: null, needsHuman: false, clockedOut: false }],
}] });
const host: AgentProfile = { id: "h", name: "Hana", role: "lead", seed: "s", createdAt: 1, deskIndex: 0, cwd: "/host" };
const judged = (decision: AutoRunJudgeResult["decision"], message = ""): AutoRunJudgeResult => ({ decision, message, note: "", costUsd: 0.01 });
const initialApp = useAppStore.getState(); const initialWork = useWorkItemStore.getState();
let off: () => void = () => {};
/** 그 머신이 보내기에 쓴 번호. 결과 보고에 다시 단다. */
let lastSeq = 0;

function task(title: string, patch: Partial<WorkItem> = {}): string {
  const id = useWorkItemStore.getState().add({ title, body: `${title} body`, status: "todo" })!;
  useWorkItemStore.getState().edit(id, patch);
  return id;
}
function get(id: string): WorkItem { return useWorkItemStore.getState().items.find((i) => i.id === id)!; }
function event(taskId: string, patch: Partial<LinkTaskEvent>): void {
  applyLinkTask({ taskId, status: "in_progress", deviceId: DEV, receivedAt: Date.now(), ...patch });
}
async function settled(root: string) { await vi.waitFor(() => expect(isAutoRunStepping(root)).toBe(false)); }

beforeEach(() => {
  vi.clearAllMocks();
  api.loadWorkItems.mockResolvedValue([]); api.saveWorkItems.mockResolvedValue(undefined);
  api.autoRunReport.mockResolvedValue("status: done");
  api.linkWork.mockImplementation(async (_agent: string, kind: string, data: { sendSeq?: number }) => {
    if (kind === "work-send") lastSeq = data.sendSeq!;
    if (kind === "work-reply") return { reply: "Remote done." };
    if (kind === "usage") return null;
    return {};
  });
  useAppStore.setState(initialApp, true);
  useAppStore.setState({ agents: { h: host }, agentOrder: ["h"], appSettings: { ...initialApp.appSettings, observerEnabled: true } });
  useWorkItemStore.setState({ ...initialWork, loaded: true }, true);
  useWorkSendStore.setState({ busy: {}, confirmation: null }); useWorkMonitorStore.setState({ sessions: {} });
  setRemoteOffice(office(true));
  off = installAutoRun();
});
afterEach(async () => { off(); await settleWorkSends(); await useWorkItemStore.getState().flush(); setRemoteOffice({ machines: [] }); });

describe("원격 담당", () => {
  it("저장소 주소는 Rust와 같은 정규형으로 비교한다", () => {
    for (const raw of ["git@github.com:acme/app.git", "https://user:tok@GitHub.com/acme/app.git/", "ssh://git@github.com:22/acme/app.git", "github.com:acme/app", "github.com/acme/app"]) {
      expect(normalizeRepoUrl(raw)).toBe("github.com/acme/app");
    }
    expect(normalizeRepoUrl("ssh://git@host.example:acme/app.git")).toBe("host.example/acme/app");
    expect(normalizeRepoUrl("/srv/git/app.git")).toBe("/srv/git/app");
    expect(normalizeRepoUrl("file:///srv/git/app.git")).toBe("/srv/git/app");
    expect(normalizeRepoUrl("no-colon")).toBeNull();
  });
  it("그 머신이 끊겼거나 클론이 없으면 맡기지 못한다", () => {
    expect(remoteSendBlocked({ assigneeAgentId: ADA })).toBeUndefined();
    expect(remoteSendBlocked({ assigneeAgentId: ADA, repo: { url: "git@github.com:acme/app.git" } })).toBeUndefined();
    expect(remoteSendBlocked({ assigneeAgentId: ADA, repo: { url: "github.com/acme/other" } })).toBe("link-repo-missing");
    expect(remoteSendBlocked({ assigneeAgentId: ADA, repo: { url: "???" } })).toBe("link-repo-invalid");
    expect(remoteSendBlocked({ assigneeAgentId: `${DEV}:ghost` })).toBe("work-send-agent-missing");
    setRemoteOffice(office(false));
    expect(remoteSendBlocked({ assigneeAgentId: ADA })).toBe("link-device-offline");
    // 시작은 저장소만 미리 본다(끊긴 머신은 그 단계 차례에 기다린다).
    const step = task("Remote step", { assigneeAgentId: ADA, repo: { url: "github.com/acme/other" } });
    const root = task("Plan", { assigneeAgentId: "h", dependsOn: [step], plan: true });
    expect(startBlockedReason(get(root), useWorkItemStore.getState().items, useAppStore.getState().agents, true)).toBe("link-repo-missing");
    useWorkItemStore.getState().edit(step, { repo: { url: "github.com/acme/app" } });
    expect(startBlockedReason(get(root), useWorkItemStore.getState().items, useAppStore.getState().agents, false)).toBeUndefined();
  });

  it("플랜의 원격 단계를 그 머신에 보내고, 올라온 진행 상태로 판단해 다음으로 넘긴다", async () => {
    const step = task("Remote step", { assigneeAgentId: ADA, repo: { url: "github.com/acme/app", subpath: "web", worktree: true } });
    const root = task("Plan", { assigneeAgentId: "h", dependsOn: [step], plan: true, body: "Memo." });
    api.autoRunJudge.mockResolvedValueOnce(judged("followup", "Add a test")).mockResolvedValueOnce(judged("next"));
    expect(startAutoRun(root)).toBeUndefined();
    await vi.waitFor(() => expect(api.linkWork).toHaveBeenCalledWith(ADA, "work-send", expect.anything()));
    const sent = api.linkWork.mock.calls.find((c) => c[1] === "work-send")![2];
    expect(sent).toMatchObject({ taskId: step, auto: true, repo: { url: "github.com/acme/app", subpath: "web", worktree: true } });
    expect(sent.body).toContain("Memo.");
    // 그 머신이 다른 보내기의 결과를 올리면 무시하고, 이번 번호의 결과로 세션을 잇는다.
    event(step, { sessionId: "old", sendSeq: lastSeq - 1 });
    event(step, { sessionId: "s1", phase: "working", transition: true, sendSeq: lastSeq });
    await settled(root);
    expect(get(step)).toMatchObject({ status: "in_progress", sessionIds: [`${DEV}:s1`] });
    // 턴이 끝났다(원격 세션 신호) → 판단자가 원격 응답을 읽고 후속 지시를 그 머신에 넣는다.
    event(step, { sessionId: "s1", phase: "review", transition: true, sendSeq: lastSeq });
    await vi.waitFor(() => expect(api.linkWork).toHaveBeenCalledWith(ADA, "work-followup", { sessionId: "s1", text: "Add a test" }));
    await settled(root);
    expect(api.linkWork).toHaveBeenCalledWith(ADA, "work-reply", { cli: "claude", taskId: step });
    expect(api.workLastReply).not.toHaveBeenCalled();
    expect(api.autoRunJudge.mock.calls[0][0].prompt).toContain("Remote done.");
    expect(get(step).status).toBe("in_progress");
    event(step, { sessionId: "s1", phase: "working", transition: true });
    event(step, { sessionId: "s1", phase: "review", transition: true });
    await vi.waitFor(() => expect(get(root).autoRun?.state).toBe("finished"));
    expect(get(step).status).toBe("done");
    expect(api.sendWorkItem).not.toHaveBeenCalled();
  });

  it("머신이 끊기면 진행 중 턴은 기다리고 판단 앞에서 멈췄다가, 다시 붙으면 스스로 이어 간다", async () => {
    const step = task("Remote step", { assigneeAgentId: ADA });
    const root = task("Plan", { assigneeAgentId: "h", dependsOn: [step], plan: true });
    api.autoRunJudge.mockResolvedValue(judged("next"));
    startAutoRun(root);
    await vi.waitFor(() => expect(api.linkWork).toHaveBeenCalledWith(ADA, "work-send", expect.anything()));
    event(step, { sessionId: "s1", phase: "working", transition: true, sendSeq: lastSeq });
    await settled(root);
    setRemoteOffice(office(false));
    await settled(root);
    expect(get(root).autoRun?.state).toBe("running");
    // 끊긴 동안 쌓였던 턴 끝 보고가 다시 붙기 전에 왔다고 친다(판단할 응답은 읽을 수 없다).
    event(step, { sessionId: "s1", phase: "review", transition: true });
    await vi.waitFor(() => expect(get(root).autoRun).toMatchObject({ state: "paused", pauseReason: "link-offline", pauseDetail: "Laptop" }));
    expect(api.autoRunJudge).not.toHaveBeenCalled();
    setRemoteOffice(office(true));
    await vi.waitFor(() => expect(get(root).autoRun?.state).toBe("finished"));
    expect(api.autoRunJudge).toHaveBeenCalledTimes(1);
  });

  it("보낼 차례에 머신이 끊겨 있으면 보내지 않고 기다리며, 그 머신 사용량이 모자라면 리셋을 기다린다", async () => {
    setRemoteOffice(office(false));
    const step = task("Remote step", { assigneeAgentId: ADA });
    const root = task("Plan", { assigneeAgentId: "h", dependsOn: [step], plan: true });
    startAutoRun(root);
    await vi.waitFor(() => expect(get(root).autoRun?.pauseReason).toBe("link-offline"));
    expect(api.linkWork).not.toHaveBeenCalled();
    const resetsAt = Date.now() + 3_600_000;
    const full = { claude: { provider: "claude", fetchedAtMs: Date.now(), planLabel: null, windows: [{ kind: "session", label: null, usedPercent: 99, resetsAtMs: resetsAt, windowMinutes: 300 }] }, codex: null } as unknown as UsageSnapshot;
    api.linkWork.mockImplementation(async (_a: string, kind: string) => (kind === "usage" ? full : {}));
    setRemoteOffice(office(true));
    await vi.waitFor(() => expect(get(root).autoRun?.pauseReason).toBe("usage-wait"));
    expect(get(root).autoRun?.usageWait).toMatchObject({ remote: DEV, stage: "task" });
    expect(api.linkWork.mock.calls.some((c) => c[1] === "work-send")).toBe(false);
  });
});
