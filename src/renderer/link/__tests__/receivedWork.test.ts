import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile, WorkSendTarget } from "@shared/types";
const { api, restart, create, reserve } = vi.hoisted(() => ({
  api: {
    loadWorkItems: vi.fn(), saveWorkItems: vi.fn(), inspectWorkSendTarget: vi.fn(), sendWorkItem: vi.fn(), linkTaskReport: vi.fn(),
  },
  restart: vi.fn(), create: vi.fn(), reserve: vi.fn(),
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: api }));
vi.mock("../../ipc/sessionBridge", () => ({ reserveSessionReplacement: reserve, runGuardedCreateSession: create }));
vi.mock("../../agent/restartAgentSession", () => ({ restartAgentSession: restart }));
vi.mock("../../store/persist", () => ({ flushPersistence: vi.fn() }));
vi.mock("../../workpanels/sessionSummaryStore", async (original) => ({ ...await original<object>(), linkWorkSession: vi.fn() }));
import { useAppStore } from "../../store/appStore";
import { useWorkItemStore } from "../../workpanels/workItemStore";
import { settleWorkSends, useWorkSendStore } from "../../workpanels/workSendStore";
import { observeWorkSession, useWorkMonitorStore } from "../../workpanels/workMonitor";
import { installReceivedWork, receiveHostWork } from "../receivedWork";

const ada: AgentProfile = { id: "ada", name: "Ada", role: "dev", seed: "s", createdAt: 1, deskIndex: 0, cwd: "/home/ada" };
const shell: WorkSendTarget = { sessionId: "old", humanInputRevision: 0, restartRequired: false, hasUncommitted: false };
const initialApp = useAppStore.getState(); const initialWork = useWorkItemStore.getState();
let off: () => void = () => {};
const item = (id: string) => useWorkItemStore.getState().items.find((i) => i.id === id);

beforeEach(() => {
  vi.clearAllMocks();
  api.loadWorkItems.mockResolvedValue([]); api.saveWorkItems.mockResolvedValue(undefined); api.linkTaskReport.mockResolvedValue(undefined);
  // 보내기는 확인 전·재시작 직전에 옛 세션을, 재시작 뒤에 새 세션을 본다.
  api.inspectWorkSendTarget.mockResolvedValueOnce(shell).mockResolvedValueOnce(shell).mockResolvedValue({ ...shell, sessionId: "new" });
  api.sendWorkItem.mockResolvedValue({ sessionId: "new", submittedAt: 100 });
  restart.mockResolvedValue({ sessionId: "new", state: "running" }); reserve.mockReturnValue(vi.fn());
  useAppStore.setState(initialApp, true);
  useAppStore.setState({ agents: { ada }, agentOrder: ["ada"] });
  useWorkItemStore.setState({ ...initialWork, loaded: true }, true);
  useWorkSendStore.setState({ busy: {}, confirmation: null }); useWorkMonitorStore.setState({ sessions: {} });
  off = installReceivedWork();
});
afterEach(async () => { off(); await settleWorkSends(); await useWorkItemStore.getState().flush(); });

describe("호스트에서 받은 업무", () => {
  it("같은 id의 받은 업무로 넣고 정한 폴더의 새 세션에서 보낸 뒤, 결과와 진행 상태를 호스트로 올린다", async () => {
    const answer = await receiveHostWork({ requestId: "r", op: "link-work-receive", itemId: "t-host", agentId: "ada", title: "Fix", body: "Do it", cwd: "/repo/web", auto: true, hostName: "Studio", sendSeq: 42 });
    expect(answer).toEqual({});
    expect(item("t-host")).toMatchObject({ title: "Fix", assigneeAgentId: "ada", received: { hostName: "Studio", cwd: "/repo/web" } });
    // 셸만 떠 있어도 그 폴더에서 새 세션을 연다.
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalled());
    expect(restart).toHaveBeenCalledWith("ada", expect.objectContaining({ cwd: "/repo/web", strictCwd: true }));
    await vi.waitFor(() => expect(api.linkTaskReport).toHaveBeenCalledWith(expect.objectContaining({ taskId: "t-host", sessionId: "new", sendSeq: 42 })));
    // 턴이 끝나면 상태가 바뀐 것만 올린다.
    api.linkTaskReport.mockClear();
    observeWorkSession("new", "ada", "review", Date.now() + 1000, true);
    await vi.waitFor(() => expect(api.linkTaskReport).toHaveBeenCalledWith(expect.objectContaining({ taskId: "t-host", status: "review", phase: "review", transition: true, sendSeq: 42 })));
    const calls = api.linkTaskReport.mock.calls.length;
    observeWorkSession("new", "ada", "review", Date.now() + 2000, true);
    expect(api.linkTaskReport.mock.calls.length).toBe(calls);
  });

  it("모르는 캐릭터면 거절하고, 다시 받으면 같은 업무를 할 일로 되돌려 다시 보낸다", async () => {
    expect(await receiveHostWork({ requestId: "r", op: "link-work-receive", itemId: "t-1", agentId: "ghost", cwd: "/x" })).toEqual({ error: "work-send-agent-missing" });
    expect(item("t-1")).toBeUndefined();
    await receiveHostWork({ requestId: "r", op: "link-work-receive", itemId: "t-1", agentId: "ada", title: "A", body: "", cwd: "/x", auto: true, sendSeq: 1 });
    await vi.waitFor(() => expect(item("t-1")?.status).toBe("in_progress"));
    await settleWorkSends();
    useWorkItemStore.getState().edit("t-1", { status: "done" });
    api.inspectWorkSendTarget.mockReset().mockResolvedValue({ ...shell, sessionId: "new" });
    await receiveHostWork({ requestId: "r", op: "link-work-receive", itemId: "t-1", agentId: "ada", title: "A again", body: "", cwd: "/x", auto: true, sendSeq: 2 });
    expect(useWorkItemStore.getState().items.filter((i) => i.id === "t-1")).toHaveLength(1);
    expect(item("t-1")?.title).toBe("A again");
  });
});
