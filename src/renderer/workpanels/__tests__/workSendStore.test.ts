import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile, WorkSendTarget } from "@shared/types";
const { api, restart, create, reserve, flushProfile, link } = vi.hoisted(() => ({
  api: { loadWorkItems: vi.fn(), saveWorkItems: vi.fn(), inspectWorkSendTarget: vi.fn(), sendWorkItem: vi.fn() },
  restart: vi.fn(), create: vi.fn(), reserve: vi.fn(), flushProfile: vi.fn(), link: vi.fn(),
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: api }));
vi.mock("../../ipc/sessionBridge", () => ({ reserveSessionReplacement: reserve, runGuardedCreateSession: create }));
vi.mock("../../agent/restartAgentSession", () => ({ restartAgentSession: restart }));
vi.mock("../../store/persist", () => ({ flushPersistence: flushProfile }));
vi.mock("../sessionSummaryStore", () => ({ linkWorkSession: link }));
import { useAppStore } from "../../store/appStore";
import { useWorkItemStore } from "../workItemStore";
import { useWorkSendStore, requestWorkSend, decideWorkSend, settleWorkSends, hasPendingWorkSends } from "../workSendStore";
import { useWorkMonitorStore } from "../workMonitor";
const initialApp = useAppStore.getState(); const initialWork = useWorkItemStore.getState();
const profile: AgentProfile = { id: "a", name: "Ada", role: "dev", seed: "s", createdAt: 1, deskIndex: 0, cwd: "/work", defaultAgent: { cli: "codex", model: "custom", effort: "high" } };
const shell: WorkSendTarget = { sessionId: "s", humanInputRevision: 0, restartRequired: false, hasUncommitted: false };
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>((r) => { resolve = r; }); return { promise, resolve }; }
function item(title = "Task") { const id = useWorkItemStore.getState().add(title)!; useWorkItemStore.getState().edit(id, { assigneeAgentId: "a", body: "First\nSecond" }); return id; }
async function confirming() { await vi.waitFor(() => expect(useWorkSendStore.getState().confirmation).not.toBeNull()); }
beforeEach(() => {
  vi.clearAllMocks();
  api.loadWorkItems.mockResolvedValue([]); api.saveWorkItems.mockResolvedValue(undefined);
  api.inspectWorkSendTarget.mockResolvedValue(shell); api.sendWorkItem.mockResolvedValue({ sessionId: "s", submittedAt: 100 });
  restart.mockResolvedValue({ sessionId: "new", state: "running" }); create.mockResolvedValue({ sessionId: "new", state: "running" });
  reserve.mockReturnValue(vi.fn()); flushProfile.mockResolvedValue(undefined);
  useAppStore.setState(initialApp, true); useAppStore.setState({ agents: { a: profile }, agentOrder: ["a"] });
  useWorkItemStore.setState({ ...initialWork, loaded: true }, true);
  useWorkSendStore.setState({ busy: {}, confirmation: null }); useWorkMonitorStore.setState({ sessions: {} });
});
afterEach(async () => { await settleWorkSends(); await useWorkItemStore.getState().flush(); });
describe("업무 보내기", () => {
  it("선행 작업이 끝나지 않으면 점검·입력 없이 오류만 남기고 완료되면 보낸다", async () => {
    const first = item("First"); const second = item("Second");
    useWorkItemStore.getState().edit(second, { dependsOn: [first] });
    await requestWorkSend(second);
    expect(api.inspectWorkSendTarget).not.toHaveBeenCalled();
    expect(useWorkItemStore.getState().items.find((i) => i.id === second)).toMatchObject({ status: "todo", lastError: "work-send-blocked" });
    useWorkItemStore.getState().edit(first, { status: "done" });
    await requestWorkSend(second);
    expect(api.sendWorkItem).toHaveBeenCalledTimes(1);
  });
  it("기본 에이전트/모델을 사용하고 제출 성공 후에만 업무와 세션을 연결한다", async () => {
    const id = item(); const result = deferred<{ sessionId: string; submittedAt: number }>(); api.sendWorkItem.mockReturnValueOnce(result.promise);
    const job = requestWorkSend(id); await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    expect(useWorkItemStore.getState().items[0].status).toBe("todo"); expect(hasPendingWorkSends()).toBe(true);
    expect(api.sendWorkItem).toHaveBeenCalledWith(expect.objectContaining({ sessionId: "s", freshSession: false, agent: profile.defaultAgent, title: "Task", body: "First\nSecond" }));
    result.resolve({ sessionId: "s", submittedAt: 100 }); await job;
    expect(useWorkItemStore.getState().items[0]).toMatchObject({ status: "in_progress", sessionIds: ["s"], lastError: undefined });
    expect(link).toHaveBeenCalledWith(id, "a", "s", 100); expect(hasPendingWorkSends()).toBe(false);
    expect(flushProfile).toHaveBeenCalledTimes(1); expect(restart).not.toHaveBeenCalled();
  });
  it("업무 오버라이드는 CLI 전체를 대체하고 같은 캐릭터에 중복 제출하지 않는다", async () => {
    const a = item(); const b = item("Second"); useWorkItemStore.getState().edit(a, { agent: { cli: "pi", model: "  custom/model  " } });
    const pending = deferred<{ sessionId: string; submittedAt: number }>(); api.sendWorkItem.mockReturnValueOnce(pending.promise);
    const first = requestWorkSend(a); expect(requestWorkSend(a)).toBe(first); expect(requestWorkSend(b)).toBe(first);
    await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    expect(api.sendWorkItem.mock.calls[0][0].agent).toEqual({ cli: "pi", model: "custom/model" });
    pending.resolve({ sessionId: "s", submittedAt: 100 }); await first;
    expect(useWorkItemStore.getState().items.find((i) => i.id === b)?.sessionIds).toEqual([]);
  });
  it("취소하면 재시작/입력/업무 변경을 하지 않는다", async () => {
    const id = item(); api.inspectWorkSendTarget.mockResolvedValue({ ...shell, restartRequired: true, hasUncommitted: true });
    const job = requestWorkSend(id); await confirming(); decideWorkSend(false); await job;
    expect(restart).not.toHaveBeenCalled(); expect(api.sendWorkItem).not.toHaveBeenCalled(); expect(useWorkItemStore.getState().items[0].lastError).toBeUndefined();
    expect(useWorkItemStore.getState().items[0].status).toBe("todo");
  });
  it("재시작 확인의 세션/revision을 넘기고 생성 결과와 같은 새 세션에만 보낸다", async () => {
    const id = item(); const old = { ...shell, humanInputRevision: 5, restartRequired: true };
    api.inspectWorkSendTarget.mockResolvedValueOnce(old).mockResolvedValueOnce(old).mockResolvedValueOnce({ ...shell, sessionId: "new" });
    api.sendWorkItem.mockResolvedValueOnce({ sessionId: "new", submittedAt: 100 });
    useAppStore.setState({ taskLabels: { a: { sessionId: "s", cwd: "/current" } } });
    const job = requestWorkSend(id); await confirming(); decideWorkSend(true); await job;
    expect(restart).toHaveBeenCalledWith("a", { cwd: "/current", expected: { sessionId: "s", humanInputRevision: 5 } });
    expect(api.sendWorkItem.mock.calls[0][0]).toMatchObject({ sessionId: "new", freshSession: true });
  });
  it("확인 후 입력이 바뀌면 폐기하지 않고 상태를 유지한다", async () => {
    const id = item(); useWorkItemStore.getState().edit(id, { status: "review" });
    api.inspectWorkSendTarget.mockResolvedValueOnce({ ...shell, restartRequired: true }).mockResolvedValueOnce({ ...shell, humanInputRevision: 1 });
    const job = requestWorkSend(id); await confirming(); decideWorkSend(true); await job;
    expect(restart).not.toHaveBeenCalled(); expect(api.sendWorkItem).not.toHaveBeenCalled();
    expect(useWorkItemStore.getState().items[0]).toMatchObject({ status: "review", lastError: "work-send-human-input" });
  });
  it("세션이 없으면 생성하고 생성 중 교체됐으면 교체된 세션에 입력하지 않는다", async () => {
    const id = item(); api.inspectWorkSendTarget.mockResolvedValueOnce({ ...shell, sessionId: undefined }).mockResolvedValueOnce({ ...shell, sessionId: "replacement" });
    await requestWorkSend(id);
    expect(create).toHaveBeenCalledWith("a"); expect(reserve).toHaveBeenCalledWith("a"); expect(api.sendWorkItem).not.toHaveBeenCalled();
    expect(useWorkItemStore.getState().items[0].lastError).toBe("work-send-session-changed");
  });
  it("프로필이나 저장이 바뀌면 입력 전에 중단하고 퇴근/시작 명령은 차단한다", async () => {
    const id = item(); api.saveWorkItems.mockRejectedValueOnce(new Error("disk")); await requestWorkSend(id);
    expect(api.inspectWorkSendTarget).not.toHaveBeenCalled(); expect(api.sendWorkItem).not.toHaveBeenCalled();
    for (const patch of [{ clockedOut: true }, { startupCommand: "claude" }]) {
      useAppStore.setState({ agents: { a: { ...profile, ...patch } } }); await requestWorkSend(id);
    }
    expect(api.sendWorkItem).not.toHaveBeenCalled();
  });
  it("보내기 실패 시 상태/이력을 보존하고 도중 삭제한 업무를 복구하지 않는다", async () => {
    const id = item(); useWorkItemStore.getState().recordSend(id, { sessionId: "old" }); useWorkItemStore.getState().edit(id, { status: "todo" });
    api.sendWorkItem.mockRejectedValueOnce("work-send-cli-returned"); await requestWorkSend(id);
    expect(useWorkItemStore.getState().items[0]).toMatchObject({ status: "todo", sessionIds: ["old"], lastError: "work-send-cli-returned" });
    const pending = deferred<{ sessionId: string; submittedAt: number }>(); api.sendWorkItem.mockReturnValueOnce(pending.promise);
    const job = requestWorkSend(id); await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(2)); useWorkItemStore.getState().remove(id);
    pending.resolve({ sessionId: "s", submittedAt: 100 }); await job; expect(useWorkItemStore.getState().items).toEqual([]);
  });
  it("종료 대기는 확인창을 취소하고 진행 중인 보내기의 성공 기록을 기다린다", async () => {
    const id = item(); api.inspectWorkSendTarget.mockResolvedValueOnce({ ...shell, restartRequired: true });
    const job = requestWorkSend(id); await confirming(); await settleWorkSends(); await job; expect(restart).not.toHaveBeenCalled();
    const pending = deferred<{ sessionId: string; submittedAt: number }>(); api.sendWorkItem.mockReturnValueOnce(pending.promise);
    const send = requestWorkSend(id); await vi.waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
    let settled = false; const closing = settleWorkSends().then(() => { settled = true; }); await Promise.resolve(); expect(settled).toBe(false);
    pending.resolve({ sessionId: "s", submittedAt: 100 }); await Promise.all([send, closing]); expect(settled).toBe(true);
    expect(useWorkItemStore.getState().items[0].sessionIds).toEqual(["s"]);
  });
});


it("제목 없는 업무의 확인창은 본문 제목을 표시하고 전송은 빈 제목과 본문 전체를 유지한다", async () => {
  const id = item(); useWorkItemStore.getState().edit(id, { title: "" });
  api.inspectWorkSendTarget.mockResolvedValueOnce({ ...shell, restartRequired: true }).mockResolvedValueOnce(shell).mockResolvedValueOnce({ ...shell, sessionId: "new" });
  api.sendWorkItem.mockResolvedValueOnce({ sessionId: "new", submittedAt: 100 });
  const job = requestWorkSend(id); await confirming();
  expect(useWorkSendStore.getState().confirmation?.title).toBe("First Second");
  decideWorkSend(true); await job;
  expect(api.sendWorkItem).toHaveBeenCalledWith(expect.objectContaining({ title: "", body: "First\nSecond" }));
});
