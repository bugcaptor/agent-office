import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile, InboxEntry } from "@shared/types";
const { api } = vi.hoisted(() => ({
  api: { loadWorkItems: vi.fn(), saveWorkItems: vi.fn(), scanWorkInbox: vi.fn(), ackWorkInbox: vi.fn() },
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: api }));
import { useAppStore } from "../../store/appStore";
import { useWorkItemStore } from "../workItemStore";
import { resolveAssignee, startWorkInbox, stopWorkInbox, hasPendingWorkInbox } from "../workInbox";
const initialApp = useAppStore.getState(); const initialWork = useWorkItemStore.getState();
const profile = (id: string, name: string): AgentProfile => ({ id, name, role: "dev", seed: id, createdAt: 1, deskIndex: 0 });
const entry = (fileName: string, title: string, extra: Partial<InboxEntry> = {}): InboxEntry => ({ fileName, key: fileName.padEnd(16, "0"), title, body: "", ...extra });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>((r) => { resolve = r; }); return { promise, resolve }; }
beforeEach(() => {
  vi.useFakeTimers(); vi.clearAllMocks();
  api.loadWorkItems.mockResolvedValue([]); api.saveWorkItems.mockResolvedValue(undefined);
  api.scanWorkInbox.mockResolvedValue([]); api.ackWorkInbox.mockResolvedValue(undefined);
  useAppStore.setState(initialApp, true); useAppStore.setState({ agents: { a: profile("a", "하나"), b: profile("b", "둘"), c: profile("c", "둘") } });
  useWorkItemStore.setState({ ...initialWork, loaded: true }, true);
});
afterEach(async () => { await stopWorkInbox(); vi.useRealTimers(); });

describe("resolveAssignee", () => {
  const profiles = [profile("a", "하나"), profile("b", "둘"), profile("c", "둘"), profile("d", " 넷 ")];
  it("id 일치를 우선하고 이름은 단일 일치일 때만 배정한다", () => {
    expect(resolveAssignee("a", profiles)).toBe("a");
    expect(resolveAssignee(" 하나 ", profiles)).toBe("a");
    expect(resolveAssignee("넷", profiles)).toBe("d");
    expect(resolveAssignee("둘", profiles)).toBeUndefined();
    expect(resolveAssignee("없음", profiles)).toBeUndefined();
    expect(resolveAssignee(undefined, profiles)).toBeUndefined();
    expect(resolveAssignee("", profiles)).toBeUndefined();
  });
});

describe("importInbox", () => {
  it("todo 끝에 추가하고 중복 id는 건너뛰되 반영 목록에 넣으며 선택은 그대로 둔다", () => {
    const store = useWorkItemStore.getState();
    const done = store.add({ title: "Done", body: "", status: "done" })!; const todo = store.add("Todo")!;
    store.select(done);
    expect(store.importInbox([{ id: "inbox-1", title: "One", body: "" }, { id: "inbox-2", title: "", body: "Two" }, { id: "inbox-3", title: " ", body: "" }])).toEqual(["inbox-1", "inbox-2"]);
    expect(store.importInbox([{ id: "inbox-1", title: "Again", body: "" }])).toEqual(["inbox-1"]);
    const state = useWorkItemStore.getState();
    expect(state.selectedId).toBe(done); expect(state.dirty).toBe(true);
    const todos = state.items.filter((i) => i.status === "todo").sort((a, b) => a.order - b.order);
    expect(todos.map((i) => i.id)).toEqual([todo, "inbox-1", "inbox-2"]);
    expect(state.items.find((i) => i.id === "inbox-1")?.title).toBe("One");
  });
  it("로드 전이나 오류 중에는 아무것도 하지 않는다", () => {
    useWorkItemStore.setState({ loaded: false });
    expect(useWorkItemStore.getState().importInbox([{ id: "inbox-1", title: "One", body: "" }])).toEqual([]);
    useWorkItemStore.setState({ loaded: true, error: "save" });
    expect(useWorkItemStore.getState().importInbox([{ id: "inbox-1", title: "One", body: "" }])).toEqual([]);
    expect(useWorkItemStore.getState().items).toEqual([]);
  });
});

describe("receive loop", () => {
  it("완료 요청은 같은 주기의 추가 뒤에 적용해 후속 업무의 대기를 풀고 저장 실패 시 다시 받는다", async () => {
    const design = useWorkItemStore.getState().add("Design")!;
    useWorkItemStore.getState().importInbox([{ id: "t-build", title: "Build", body: "", dependsOn: [design] }]);
    await useWorkItemStore.getState().flush();
    api.saveWorkItems.mockRejectedValueOnce(new Error("disk"));
    api.scanWorkInbox.mockResolvedValue([entry("1.md", "Fresh", { id: "t-fresh" }), entry("2.md", "", { done: [design, "t-fresh", "t-missing"] })]);
    startWorkInbox(); await vi.advanceTimersByTimeAsync(3000);
    expect(api.ackWorkInbox).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(3000);
    expect(api.ackWorkInbox).toHaveBeenCalledWith(["1.md", "2.md"]);
    const items = useWorkItemStore.getState().items;
    expect(items.find((i) => i.id === design)?.status).toBe("done");
    expect(items.find((i) => i.id === "t-fresh")?.status).toBe("done");
    expect(items.filter((i) => i.title === "Fresh")).toHaveLength(1);
    expect(items.find((i) => i.id === "t-build")?.status).toBe("todo");
  });
  it("생산자 id를 그대로 쓰고 같은 주기나 기존 업무를 선행 작업으로 잇는다", async () => {
    const existing = useWorkItemStore.getState().add("Existing")!;
    api.scanWorkInbox.mockResolvedValueOnce([
      entry("1.md", "Step 1", { id: "t-1", after: [existing] }),
      entry("2.md", "Step 2", { id: "t-2", after: ["t-1", "t-missing"] }),
      entry("3.md", "Legacy"),
    ]);
    startWorkInbox(); await vi.advanceTimersByTimeAsync(3000);
    expect(api.ackWorkInbox).toHaveBeenCalledWith(["1.md", "2.md", "3.md"]);
    const items = useWorkItemStore.getState().items;
    expect(items.find((i) => i.id === "t-1")?.dependsOn).toEqual([existing]);
    expect(items.find((i) => i.id === "t-2")?.dependsOn).toEqual(["t-1"]);
    expect(items.find((i) => i.title === "Legacy")?.id).toBe(`inbox-${entry("3.md", "").key}`);
  });
  it("저장 뒤 건너뛴 파일까지 ack하고 담당을 판정한다", async () => {
    useWorkItemStore.getState().importInbox([{ id: `inbox-${entry("old.md", "").key}`, title: "Old", body: "" }]);
    api.scanWorkInbox.mockResolvedValueOnce([entry("new.md", "New", { assignee: "하나" }), entry("old.md", "Old"), entry("dup.md", "Dup", { assignee: "둘" })]);
    startWorkInbox(); await vi.advanceTimersByTimeAsync(3000);
    expect(api.saveWorkItems).toHaveBeenCalled();
    expect(api.ackWorkInbox).toHaveBeenCalledWith(["new.md", "old.md", "dup.md"]);
    const items = useWorkItemStore.getState().items;
    expect(items.find((i) => i.title === "New")?.assigneeAgentId).toBe("a");
    expect(items.find((i) => i.title === "Dup")?.assigneeAgentId).toBeUndefined();
  });
  it("저장 실패 시 ack하지 않고 다음 주기에 다시 저장한 뒤 ack한다", async () => {
    api.scanWorkInbox.mockResolvedValue([entry("a.md", "A")]);
    api.saveWorkItems.mockRejectedValueOnce(new Error("disk"));
    startWorkInbox(); await vi.advanceTimersByTimeAsync(3000);
    expect(api.ackWorkInbox).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(3000);
    expect(api.ackWorkInbox).toHaveBeenCalledWith(["a.md"]);
    expect(useWorkItemStore.getState().items).toHaveLength(1);
  });
  it("주기를 겹치지 않고 정지하면 뒤늦은 scan 결과를 버린다", async () => {
    const scan = deferred<InboxEntry[]>();
    api.scanWorkInbox.mockReturnValueOnce(scan.promise);
    startWorkInbox(); await vi.advanceTimersByTimeAsync(9000);
    expect(api.scanWorkInbox).toHaveBeenCalledTimes(1);
    const stopped = stopWorkInbox(); expect(hasPendingWorkInbox()).toBe(true);
    scan.resolve([entry("late.md", "Late")]); await stopped;
    expect(hasPendingWorkInbox()).toBe(false);
    expect(useWorkItemStore.getState().items).toEqual([]); expect(api.ackWorkInbox).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(9000); expect(api.scanWorkInbox).toHaveBeenCalledTimes(1);
  });
  it("업무 목록 로드에 실패했으면 수신함을 읽지 않는다", async () => {
    useWorkItemStore.setState({ loaded: false, error: "load" });
    startWorkInbox(); await vi.advanceTimersByTimeAsync(3000);
    expect(api.scanWorkInbox).not.toHaveBeenCalled();
  });
});
