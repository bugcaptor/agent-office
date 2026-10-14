import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile, InboxEntry } from "@shared/types";
const { api } = vi.hoisted(() => ({
  api: { loadWorkItems: vi.fn(), saveWorkItems: vi.fn(), scanWorkInbox: vi.fn(), ackWorkInbox: vi.fn(), rejectWorkInbox: vi.fn(), warnWorkInbox: vi.fn() },
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: api }));
import { useAppStore } from "../../store/appStore";
import { useWorkItemStore } from "../workItemStore";
import { assigneeIssue, resolveAssignee, startWorkInbox, stopWorkInbox, hasPendingWorkInbox } from "../workInbox";
const initialApp = useAppStore.getState(); const initialWork = useWorkItemStore.getState();
const profile = (id: string, name: string): AgentProfile => ({ id, name, role: "dev", seed: id, createdAt: 1, deskIndex: 0 });
const entry = (fileName: string, title: string, extra: Partial<InboxEntry> = {}): InboxEntry => ({ fileName, key: fileName.padEnd(16, "0"), title, body: "", ...extra });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>((r) => { resolve = r; }); return { promise, resolve }; }
beforeEach(() => {
  vi.useFakeTimers(); vi.clearAllMocks();
  api.loadWorkItems.mockResolvedValue([]); api.saveWorkItems.mockResolvedValue(undefined);
  api.scanWorkInbox.mockResolvedValue([]); api.ackWorkInbox.mockResolvedValue(undefined);
  api.rejectWorkInbox.mockResolvedValue(undefined); api.warnWorkInbox.mockResolvedValue(undefined);
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
  it("배정하지 못한 담당 값의 이유를 알려준다", () => {
    expect(assigneeIssue("a", profiles)).toBeUndefined();
    expect(assigneeIssue(undefined, profiles)).toBeUndefined();
    expect(assigneeIssue(" ", profiles)).toBeUndefined();
    expect(assigneeIssue(" 둘", profiles)).toBe("ambiguous-assignee");
    expect(assigneeIssue("없음", profiles)).toBe("unknown-assignee");
  });
});

describe("importInbox", () => {
  it("todo 끝에 추가하고 같은 내용의 재수신과 다른 내용의 id 충돌을 구분하며 선택은 그대로 둔다", () => {
    const store = useWorkItemStore.getState();
    const done = store.add({ title: "Done", body: "", status: "done" })!; const todo = store.add("Todo")!;
    store.select(done);
    expect(store.importInbox([{ id: "inbox-1", title: "One", body: "" }, { id: "inbox-2", title: "", body: "Two" }, { id: "inbox-3", title: " ", body: "" }]))
      .toEqual([{ kind: "added", dropped: [] }, { kind: "added", dropped: [] }, { kind: "skipped" }]);
    expect(store.importInbox([{ id: "inbox-1", title: "One", body: "" }, { id: "inbox-1", title: "Again", body: "" }, { id: "inbox-4", title: "Four", body: "" }, { id: "inbox-4", title: "Other", body: "" }]))
      .toEqual([{ kind: "replayed" }, { kind: "duplicate" }, { kind: "added", dropped: [] }, { kind: "duplicate" }]);
    const state = useWorkItemStore.getState();
    expect(state.selectedId).toBe(done); expect(state.dirty).toBe(true);
    const todos = state.items.filter((i) => i.status === "todo").sort((a, b) => a.order - b.order);
    expect(todos.map((i) => i.id)).toEqual([todo, "inbox-1", "inbox-2", "inbox-4"]);
    expect(state.items.find((i) => i.id === "inbox-1")?.title).toBe("One");
    expect(state.items.find((i) => i.id === "inbox-4")?.title).toBe("Four");
  });
  it("버린 선행 작업을 사유와 함께 돌려준다", () => {
    const store = useWorkItemStore.getState();
    const base = store.add("Base")!;
    // A·B가 서로를 기다리면 먼저 처리한 A 쪽 선행이 순환으로 빠진다.
    expect(store.importInbox([
      { id: "t-a", title: "A", body: "", dependsOn: ["t-b", base] },
      { id: "t-b", title: "B", body: "", dependsOn: ["t-a", "t-b", "t-ghost"] },
    ])).toEqual([
      { kind: "added", dropped: [{ id: "t-b", reason: "cycle" }] },
      { kind: "added", dropped: [{ id: "t-b", reason: "self" }, { id: "t-ghost", reason: "unknown" }] },
    ]);
    const items = useWorkItemStore.getState().items;
    expect(items.find((i) => i.id === "t-a")?.dependsOn).toEqual([base]);
    expect(items.find((i) => i.id === "t-b")?.dependsOn).toEqual(["t-a"]);
  });
  it("로드 전이나 오류 중에는 아무것도 하지 않는다", () => {
    useWorkItemStore.setState({ loaded: false });
    expect(useWorkItemStore.getState().importInbox([{ id: "inbox-1", title: "One", body: "" }])).toEqual([]);
    expect(useWorkItemStore.getState().applyInbox([{ kind: "done", ids: ["inbox-1"] }])).toEqual([]);
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
    api.scanWorkInbox.mockResolvedValue([entry("1.md", "Fresh", { id: "t-fresh" }), entry("2.md", "", { done: [design, "t-fresh"] })]);
    startWorkInbox(); await vi.advanceTimersByTimeAsync(3000);
    expect(api.ackWorkInbox).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(3000);
    expect(api.ackWorkInbox).toHaveBeenCalledWith(["1.md", "2.md"]);
    expect(api.rejectWorkInbox).not.toHaveBeenCalled();
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
    expect(api.warnWorkInbox).toHaveBeenCalledWith([{ fileName: "2.md", taskId: "t-2", code: "unknown-prerequisite", detail: "t-missing" }]);
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
    // 재수신(old.md)은 경고를 다시 남기지 않는다.
    expect(api.warnWorkInbox).toHaveBeenCalledWith([{ fileName: "dup.md", taskId: `inbox-${entry("dup.md", "").key}`, code: "ambiguous-assignee", detail: "둘" }]);
    const items = useWorkItemStore.getState().items;
    expect(items.find((i) => i.title === "New")?.assigneeAgentId).toBe("a");
    expect(items.find((i) => i.title === "Dup")?.assigneeAgentId).toBeUndefined();
  });
  it("id 충돌과 없는 업무 완료 요청은 반영하지 않고 rejected로 보낸다", async () => {
    const keep = useWorkItemStore.getState().add("Keep")!;
    useWorkItemStore.getState().importInbox([{ id: "t-old", title: "Old", body: "" }]);
    api.scanWorkInbox.mockResolvedValueOnce([
      entry("1.md", "Different", { id: "t-old" }),
      entry("2.md", "Nobody", { id: "t-new", assignee: "없는사람" }),
      entry("3.md", "", { done: [keep, "t-nothere1"] }),
      entry("4.md", "", { done: ["t-new"] }),
    ]);
    startWorkInbox(); await vi.advanceTimersByTimeAsync(3000);
    expect(api.ackWorkInbox).toHaveBeenCalledWith(["2.md", "4.md"]);
    expect(api.rejectWorkInbox).toHaveBeenCalledWith([
      { fileName: "1.md", reason: "duplicate-id", detail: "t-old" },
      { fileName: "3.md", reason: "unknown-task", detail: "t-nothere1" },
    ]);
    expect(api.warnWorkInbox).toHaveBeenCalledWith([{ fileName: "2.md", taskId: "t-new", code: "unknown-assignee", detail: "없는사람" }]);
    const items = useWorkItemStore.getState().items;
    expect(items.find((i) => i.id === "t-old")?.title).toBe("Old");
    expect(items.find((i) => i.id === keep)?.status).toBe("todo");
    expect(items.find((i) => i.id === "t-new")).toMatchObject({ status: "done", assigneeAgentId: undefined });
  });
  it("수정·삭제 요청을 파일 순서대로 적용하고 버린 선행·모르는 담당은 경고로 남긴다", async () => {
    const store = useWorkItemStore.getState();
    const a = store.add({ title: "A", body: "old", status: "todo", assigneeAgentId: "a" })!;
    const b = store.add({ title: "B", body: "", status: "todo", dependsOn: [a] })!;
    const c = store.add({ title: "C", body: "", status: "review", dependsOn: [b] })!;
    store.select(c);
    api.scanWorkInbox.mockResolvedValueOnce([
      entry("1.md", "", { edit: { id: b, title: " B2 ", body: "new", status: "review", removeAfter: [a], addAfter: ["t-fresh", c, "t-ghost"] } }),
      entry("2.md", "", { edit: { id: a, assignee: "" } }),
      entry("3.md", "", { edit: { id: a, assignee: "없는사람", after: [] } }),
      entry("4.md", "", { delete: [c] }),
      entry("5.md", "", { edit: { id: c, title: "gone" } }),
      entry("6.md", "", { edit: { id: a, title: "", body: "" } }),
      entry("7.md", "Fresh", { id: "t-fresh" }),
    ]);
    startWorkInbox(); await vi.advanceTimersByTimeAsync(3000);
    expect(api.ackWorkInbox).toHaveBeenCalledWith(["7.md", "1.md", "2.md", "3.md", "4.md"]);
    expect(api.rejectWorkInbox).toHaveBeenCalledWith([
      { fileName: "5.md", reason: "unknown-task", detail: c },
      { fileName: "6.md", reason: "empty-task", detail: a },
    ]);
    expect(api.warnWorkInbox).toHaveBeenCalledWith([
      { fileName: "1.md", taskId: b, code: "cyclic-prerequisite", detail: c },
      { fileName: "1.md", taskId: b, code: "unknown-prerequisite", detail: "t-ghost" },
      { fileName: "3.md", taskId: a, code: "unknown-assignee", detail: "없는사람" },
    ]);
    const state = useWorkItemStore.getState();
    // 같은 주기에 추가된 업무를 선행으로 잇고, 순환을 만드는 선행은 뺀다.
    expect(state.items.find((i) => i.id === b)).toMatchObject({ title: "B2", body: "new", status: "review", dependsOn: ["t-fresh"] });
    // 미배정으로 바꾼 뒤 모르는 담당은 담당을 그대로 둔다.
    expect(state.items.find((i) => i.id === a)).toMatchObject({ title: "A", body: "old", assigneeAgentId: undefined, dependsOn: undefined });
    expect(state.items.find((i) => i.id === c)).toBeUndefined();
    expect(state.selectedId).toBeNull();
  });
  it("경고 기록에 실패해도 업무 파일은 ack한다", async () => {
    api.warnWorkInbox.mockRejectedValueOnce(new Error("disk"));
    api.scanWorkInbox.mockResolvedValueOnce([entry("a.md", "A", { after: ["t-ghost"] })]);
    startWorkInbox(); await vi.advanceTimersByTimeAsync(3000);
    expect(api.ackWorkInbox).toHaveBeenCalledWith(["a.md"]);
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
