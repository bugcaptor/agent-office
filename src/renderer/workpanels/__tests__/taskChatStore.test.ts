import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionStateEvent, SessionSummary, TaskChatRequest, WorkItem } from "@shared/types";
const { api, registry, stateCbs } = vi.hoisted(() => ({
  api: { openTaskChat: vi.fn(), disposeSession: vi.fn(), saveWorkItems: vi.fn() },
  registry: { ensure: vi.fn(), destroy: vi.fn(), get: vi.fn(), has: vi.fn() },
  stateCbs: new Set<(e: SessionStateEvent) => void>(),
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: {
  ...api,
  onSessionState: (cb: (e: SessionStateEvent) => void) => { stateCbs.add(cb); return () => stateCbs.delete(cb); },
} }));
vi.mock("../../terminal/TerminalRegistry", () => ({ terminalRegistry: registry }));
import { useAppStore } from "../../store/appStore";
import { useWorkItemStore } from "../workItemStore";
import { useSessionSummaryStore } from "../sessionSummaryStore";
import {
  MAX_TASK_CHATS, closeTaskChat, installTaskChatTracking, liveTaskChatCount, openTaskChat, requestCloseTaskChat,
  taskChatBlockedReason, taskChatErrorText, useTaskChatStore,
} from "../taskChatStore";

const now = 1_800_000_000_000;
const initialApp = useAppStore.getState();
const initialWork = useWorkItemStore.getState();
let off: () => void;
function task(id: string, patch: Partial<WorkItem> = {}): WorkItem {
  return { id, title: `Task ${id}`, body: `Body ${id}`, status: "todo", order: 0, sessionIds: [], createdAt: now, updatedAt: now, ...patch };
}
function emit(e: Partial<SessionStateEvent>) { stateCbs.forEach((cb) => cb({ agentId: "", sessionId: "", state: "running", at: now, ...e } as SessionStateEvent)); }
function lastRequest(): TaskChatRequest { const calls = api.openTaskChat.mock.calls; return calls[calls.length - 1][0]; }

beforeEach(() => {
  vi.useFakeTimers(); vi.setSystemTime(now);
  Object.values(api).forEach((f) => f.mockReset());
  Object.values(registry).forEach((f) => f.mockReset());
  let n = 0;
  api.openTaskChat.mockImplementation(async (r: TaskChatRequest) => ({ agentId: `task-chat-${r.taskId}`, sessionId: `sid-${++n}` }));
  api.disposeSession.mockResolvedValue(undefined);
  api.saveWorkItems.mockResolvedValue(undefined);
  useAppStore.setState({
    ...initialApp,
    agents: { a1: { id: "a1", name: "Hana", role: "", seed: "x", createdAt: now, deskIndex: 0, cwd: "/profile", shell: "zsh", defaultAgent: { cli: "codex", model: "gpt" } } },
    agentOrder: ["a1"], recentAgentIds: ["a1"], activeTerminalAgentId: null,
  }, true);
  useWorkItemStore.setState({ ...initialWork, items: [], loaded: true }, true);
  useSessionSummaryStore.setState({ summaries: {}, liveIds: [], error: false });
  useTaskChatStore.setState({ chats: {}, order: [], opening: {} });
  off = installTaskChatTracking();
});
afterEach(() => { off(); vi.useRealTimers(); });

describe("태스크 대화 열기", () => {
  it("작업 폴더는 최신 연결 세션 요약 → 담당 프로필 → 홈, 에이전트는 태스크 → 프로필 기본 → Claude 순으로 푼다", async () => {
    const summary = (sessionId: string, cwd?: string): SessionSummary => ({ sessionId, agentId: "a1", agentName: "Hana", cwd, startedAt: now - 1000, prompts: [] });
    useSessionSummaryStore.setState({ summaries: { s1: summary("s1", "/old"), s2: summary("s2", "/new"), s3: summary("s3") } });
    useWorkItemStore.setState({ items: [
      task("t1", { assigneeAgentId: "a1", sessionIds: ["s1", "s2", "s3"], agent: { cli: "claude", model: " opus " } }),
      task("t2", { assigneeAgentId: "a1" }),
      task("t3"),
    ] });
    await openTaskChat("t1");
    expect(lastRequest()).toMatchObject({ taskId: "t1", cwd: "/new", shell: "zsh", agent: { cli: "claude", model: "opus" } });
    expect(lastRequest().context.task.sessions).toEqual([
      { sessionId: "s1", agentId: "a1", sentAt: now - 1000 }, { sessionId: "s2", agentId: "a1", sentAt: now - 1000 }, { sessionId: "s3", agentId: "a1", sentAt: now - 1000 },
    ]);
    await openTaskChat("t2");
    expect(lastRequest()).toMatchObject({ cwd: "/profile", agent: { cli: "codex", model: "gpt" } });
    await openTaskChat("t3");
    expect(lastRequest().cwd).toBeUndefined();
    expect(lastRequest().agent).toEqual({ cli: "claude" });
  });

  it("결과를 받은 뒤에 탭을 열고, 살아 있는 대화는 새로 만들지 않고 그 탭으로 간다", async () => {
    useWorkItemStore.setState({ items: [task("t1")] });
    let resolve!: (v: { agentId: string; sessionId: string }) => void;
    api.openTaskChat.mockImplementationOnce(() => new Promise((r) => { resolve = r; }));
    const pending = openTaskChat("t1");
    expect(registry.ensure).toHaveBeenCalledWith("task-chat-t1");
    expect(useTaskChatStore.getState().order).toEqual([]);
    expect(taskChatBlockedReason("t1")).toBe("task-chat-busy");
    expect(await openTaskChat("t1")).toBe("task-chat-busy");
    resolve({ agentId: "task-chat-t1", sessionId: "sid-x" });
    expect(await pending).toBeUndefined();
    expect(useTaskChatStore.getState().order).toEqual(["task-chat-t1"]);
    expect(useAppStore.getState().activeTerminalAgentId).toBe("task-chat-t1");
    // 캐릭터 탭 순서에는 들어가지 않는다.
    expect(useAppStore.getState().recentAgentIds).toEqual(["a1"]);

    useAppStore.getState().openTerminal("a1");
    expect(await openTaskChat("t1")).toBeUndefined();
    expect(api.openTaskChat).toHaveBeenCalledTimes(1);
    expect(useAppStore.getState().activeTerminalAgentId).toBe("task-chat-t1");
  });

  it("실패하면 탭을 만들지 않고 미리 구독한 터미널을 버린다", async () => {
    useWorkItemStore.setState({ items: [task("t1")] });
    api.openTaskChat.mockRejectedValueOnce("work-send-launch-timeout");
    expect(await openTaskChat("t1")).toBe("work-send-launch-timeout");
    expect(registry.destroy).toHaveBeenCalledWith("task-chat-t1");
    expect(useTaskChatStore.getState().order).toEqual([]);
    expect(useTaskChatStore.getState().opening).toEqual({});
    expect(useAppStore.getState().activeTerminalAgentId).toBeNull();
  });

  it(`살아 있는 대화가 ${MAX_TASK_CHATS}개면 막고 가장 오래 안 본 대화를 안내한다. 끝난 대화는 세지 않는다`, async () => {
    useWorkItemStore.setState({ items: ["t1", "t2", "t3", "t4", "t5"].map((id) => task(id)) });
    for (const id of ["t1", "t2", "t3", "t4"]) { vi.advanceTimersByTime(1000); await openTaskChat(id); }
    vi.advanceTimersByTime(1000); useAppStore.getState().openTerminal("task-chat-t1");
    expect(liveTaskChatCount()).toBe(4);
    expect(await openTaskChat("t5")).toBe("task-chat-limit");
    expect(api.openTaskChat).toHaveBeenCalledTimes(4);
    expect(taskChatErrorText("task-chat-limit")).toContain("Task t2");
    emit({ agentId: "task-chat-t3", sessionId: "sid-3", state: "exited" });
    expect(liveTaskChatCount()).toBe(3);
    expect(await openTaskChat("t5")).toBeUndefined();
  });

  it("끝난 대화를 다시 열면 새 화면으로 새 세션을 만든다", async () => {
    useWorkItemStore.setState({ items: [task("t1")] });
    await openTaskChat("t1");
    emit({ agentId: "task-chat-t1", sessionId: "sid-1", state: "disposed" });
    expect(useTaskChatStore.getState().chats["task-chat-t1"].status).toBe("exited");
    registry.destroy.mockClear();
    await openTaskChat("t1");
    expect(registry.destroy).toHaveBeenCalledWith("task-chat-t1");
    expect(useTaskChatStore.getState().chats["task-chat-t1"]).toMatchObject({ sessionId: "sid-2", status: "running" });
  });

  it("자동 진행 중에 열어도 업무 상태·sessionIds·실행 상태를 바꾸지 않고 플랜 맥락만 넘긴다", async () => {
    const run = { state: "running" as const, judgeSessionId: "j", judgeStarted: true, currentId: "t2", followUps: 0, costUsd: 0, startedAt: now - 9000, log: [
      { at: now - 8000, kind: "send" as const, taskId: "t1" },
      { at: now - 7000, kind: "next" as const, taskId: "t1", text: "note from t1" },
      { at: now - 6000, kind: "send" as const, taskId: "t2" },
      { at: now - 5000, kind: "followup" as const, taskId: "t2", text: "add tests" },
      { at: now - 4000, kind: "next" as const, taskId: "t0", text: "later note" },
    ] };
    useWorkItemStore.setState({ items: [
      task("t1", { status: "done", assigneeAgentId: "a1", sessionIds: ["s1"] }),
      task("t2", { status: "in_progress", assigneeAgentId: "a1", sessionIds: ["s2"], dependsOn: ["t1"] }),
      task("root", { title: "Plan", body: "memo", assigneeAgentId: "a1", dependsOn: ["t2"], autoRun: run }),
    ] });
    const before = JSON.stringify(useWorkItemStore.getState().items);
    await openTaskChat("t2");
    expect(JSON.stringify(useWorkItemStore.getState().items)).toBe(before);
    expect(useWorkItemStore.getState().dirty).toBe(false);
    const { task: snapshot, plan } = lastRequest().context;
    expect(snapshot).toMatchObject({ title: "Task t2", status: "in_progress", assignee: "Hana", dependsOn: [{ id: "t1", status: "done" }], dependents: [{ id: "root", title: "Plan" }] });
    expect(plan).toMatchObject({ id: "root", title: "Plan", body: "memo", state: "running" });
    expect(plan!.log).toEqual([{ at: now - 6000, kind: "send" }, { at: now - 5000, kind: "followup", text: "add tests" }]);
    expect(plan!.handoffNotes).toEqual([{ taskId: "t1", title: "Task t1", text: "note from t1" }]);
  });
});

describe("대화 탭 닫기", () => {
  beforeEach(async () => {
    useWorkItemStore.setState({ items: [task("t1"), task("t2")] });
    await openTaskChat("t1"); await openTaskChat("t2");
  });
  it("CLI가 살아 있으면 확인창을 먼저 띄우고, 확인하면 세션을 버리고 이웃 탭으로 옮긴다", async () => {
    requestCloseTaskChat("task-chat-t2");
    expect(useAppStore.getState().modal).toEqual({ kind: "confirm-close-task-chat", agentId: "task-chat-t2" });
    expect(api.disposeSession).not.toHaveBeenCalled();
    await closeTaskChat("task-chat-t2");
    expect(api.disposeSession).toHaveBeenCalledWith("task-chat-t2");
    expect(registry.destroy).toHaveBeenCalledWith("task-chat-t2");
    expect(useTaskChatStore.getState().order).toEqual(["task-chat-t1"]);
    expect(useAppStore.getState().activeTerminalAgentId).toBe("task-chat-t1");
    await closeTaskChat("task-chat-t1");
    expect(useAppStore.getState().activeTerminalAgentId).toBe("a1");
  });
  it("끝난 대화는 확인 없이 닫고 세션을 다시 버리지 않는다", () => {
    emit({ agentId: "task-chat-t1", sessionId: "sid-1", state: "exited" });
    requestCloseTaskChat("task-chat-t1");
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
    expect(api.disposeSession).not.toHaveBeenCalled();
    expect(useTaskChatStore.getState().order).toEqual(["task-chat-t2"]);
  });
  it("다른 세션의 늦은 상태 이벤트는 탭 상태를 바꾸지 않는다", () => {
    emit({ agentId: "task-chat-t1", sessionId: "old", state: "exited" });
    expect(useTaskChatStore.getState().chats["task-chat-t1"].status).toBe("running");
  });
});
