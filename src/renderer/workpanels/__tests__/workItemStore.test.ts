import type { WorkItem } from "@shared/types";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const { load, save } = vi.hoisted(() => ({ load: vi.fn(), save: vi.fn() }));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: { loadWorkItems: load, saveWorkItems: save } }));
import { useWorkItemStore } from "../workItemStore";
const initial = useWorkItemStore.getState();
beforeEach(() => { vi.useFakeTimers(); load.mockReset().mockResolvedValue([]); save.mockReset().mockResolvedValue(undefined); useWorkItemStore.setState(initial, true); });
afterEach(async () => { await useWorkItemStore.getState().flush(); vi.useRealTimers(); });
describe("work items", () => {
  it("파일 가져오기는 항상 새 id로 추가하고 파일 안 선행 작업을 새 id로 다시 잇는다", async () => {
    const store = useWorkItemStore.getState(); await store.load();
    const existing = store.add("Existing")!;
    const added = store.importFile([
      { sourceId: existing, title: "Copy", body: "", status: "todo", createdAt: 1, assigneeAgentId: "a1", assigneeName: "Ada" },
      { sourceId: "b", title: "B", body: "", status: "done", dependsOn: [existing, "missing"], createdAt: 2 },
      { title: "Tail", body: "", status: "todo", dependsOn: ["b"], createdAt: 2 },
      { sourceId: "c", title: "Cycle", body: "", status: "todo", dependsOn: ["c2"], createdAt: 3 },
      { sourceId: "c2", title: "Cycle 2", body: "", status: "todo", dependsOn: ["c"], createdAt: 4 },
    ]);
    expect(added).toBe(5);
    const { items } = useWorkItemStore.getState();
    const by = (title: string) => items.find((i) => i.title === title)!;
    expect(new Set(items.map((i) => i.id)).size).toBe(6);
    expect(by("Copy").id).not.toBe(existing);
    expect(by("Copy")).toMatchObject({ order: 1, sessionIds: [], createdAt: 1, assigneeAgentId: "a1" });
    expect(by("Copy")).not.toHaveProperty("assigneeName");
    expect(by("Tail")).toMatchObject({ dependsOn: [by("B").id], order: 2 });
    // 파일 안 id가 지금 목록과 겹쳐도 파일 안 업무를 가리키고, 없는 id는 버린다.
    expect(by("B")).toMatchObject({ status: "done", order: 0, dependsOn: [by("Copy").id] });
    expect([by("Cycle").dependsOn, by("Cycle 2").dependsOn]).toEqual([undefined, [by("Cycle").id]]);
    expect(by("Existing").dependsOn).toBeUndefined();
    await vi.advanceTimersByTimeAsync(1000);
    expect(save).toHaveBeenCalledTimes(1);
  });
  it("로드 실패 상태에서는 파일을 가져오지 않는다", async () => {
    load.mockRejectedValueOnce(new Error("disk"));
    await useWorkItemStore.getState().load();
    expect(useWorkItemStore.getState().importFile([{ title: "x", body: "", status: "todo", createdAt: 0 }])).toBe(0);
  });
  it("로드 실패 시 빈 목록을 저장하지 않고 재시도한다", async () => {
    load.mockRejectedValueOnce(new Error("disk"));
    await useWorkItemStore.getState().load();
    expect(useWorkItemStore.getState().add("lost" )).toBeUndefined();
    await useWorkItemStore.getState().flush(); expect(save).not.toHaveBeenCalled();
    await useWorkItemStore.getState().load(); expect(useWorkItemStore.getState().loaded).toBe(true);
  });
  it("동시 로드를 공유해 뒤늦은 복원이 편집을 덮어쓰지 않는다", async () => {
    let resolve!: (items: []) => void;
    load.mockImplementationOnce(() => new Promise((r) => { resolve = r; }));
    const a = useWorkItemStore.getState().load(); const b = useWorkItemStore.getState().load();
    expect(load).toHaveBeenCalledTimes(1); resolve([]); await Promise.all([a,b]);
    const id = useWorkItemStore.getState().add("task"); await useWorkItemStore.getState().load();
    expect(useWorkItemStore.getState().items[0].id).toBe(id);
  });
  it("생성·편집·순서 변경을 자동저장하고 상태 변경은 그룹 끝에 배치한다", async () => {
    const store = useWorkItemStore.getState(); await store.load();
    const a = store.add(" Alpha ")!; const b = store.add("Beta")!;
    store.edit(a, { body: "notes", assigneeAgentId: "a1" }); store.reorder(b, a);
    await vi.advanceTimersByTimeAsync(1000);
    expect(save).toHaveBeenCalledTimes(1);
    expect([...save.mock.calls[0][0]].sort((a,b) => a.order-b.order).map((i) => i.title)).toEqual(["Beta", "Alpha"]);
    store.reorder(b, a, true);
    expect([...useWorkItemStore.getState().items].sort((a,b) => a.order-b.order).map((i) => i.title)).toEqual(["Alpha", "Beta"]);
    store.edit(a, { status: "done" }); store.edit(b, { status: "done" });
    expect(useWorkItemStore.getState().items.find((i) => i.id===b)?.order).toBeGreaterThan(useWorkItemStore.getState().items.find((i) => i.id===a)!.order);
  });
  it("저장 중 편집도 직렬 저장해 최신 스냅샷을 보존한다", async () => {
    const store = useWorkItemStore.getState(); await store.load(); const id = store.add("One")!;
    let resolve!: () => void;
    save.mockImplementationOnce(() => new Promise<void>((r) => { resolve = r; }));
    const first = store.flush(); store.edit(id, { title: "Two" }); const second = store.flush();
    expect(save).toHaveBeenCalledTimes(1); resolve(); await Promise.all([first,second]);
    expect(save).toHaveBeenCalledTimes(2); expect(save.mock.calls[1][0][0].title).toBe("Two");
    expect(useWorkItemStore.getState().dirty).toBe(false);
  });
  it("저장 실패는 dirty를 유지하고 재시도 뒤 삭제도 저장한다", async () => {
    const store = useWorkItemStore.getState(); await store.load(); const id = store.add("One")!;
    save.mockRejectedValueOnce(new Error("disk")); await store.flush(); expect(useWorkItemStore.getState().dirty).toBe(true);
    await store.flush(); expect(useWorkItemStore.getState().error).toBeNull();
    store.remove(id); await store.flush(); expect(save.mock.calls[save.mock.calls.length - 1]?.[0]).toEqual([]);
  });
});


it("빈 제목과 본문만 있는 태스크를 저장하며 둘 다 빈 편집은 거절한다", async () => {
  const store = useWorkItemStore.getState(); await store.load();
  expect(store.add({ title: " ", body: "\n", status: "todo" })).toBeUndefined();
  const id = store.add({ title: " ", body: "Body", status: "review", assigneeAgentId: "a", agent: { cli: "pi" } })!;
  await store.flush();
  expect(save.mock.calls[0][0][0]).toMatchObject({ title: "", body: "Body", status: "review", assigneeAgentId: "a", agent: { cli: "pi" } });
  store.edit(id, { body: " " });
  expect(useWorkItemStore.getState().items[0].body).toBe("Body");
  store.edit(id, { title: "Named", body: "" });
  expect(useWorkItemStore.getState().items[0]).toMatchObject({ title: "Named", body: "" });
});
it("선행 작업을 다듬어 저장하고 지운 업무는 후속 업무의 선행 조건에서 뺀다", async () => {
  const store = useWorkItemStore.getState(); await store.load();
  const a = store.add("A")!; const b = store.add({ title: "B", body: "", status: "todo", dependsOn: [a, a, "gone"] })!;
  expect(useWorkItemStore.getState().items.find((i) => i.id === b)?.dependsOn).toEqual([a]);
  store.edit(a, { dependsOn: [b] });
  expect(useWorkItemStore.getState().items.find((i) => i.id === a)?.dependsOn).toBeUndefined();
  store.remove(a);
  expect(useWorkItemStore.getState().items.find((i) => i.id === b)?.dependsOn).toBeUndefined();
});

const work = (id: string, extra: Partial<WorkItem> = {}): WorkItem =>
  ({ id, title: id, body: "", status: "todo", order: 0, sessionIds: [], createdAt: 1, updatedAt: 1, ...extra });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>((r) => { resolve = r; }); return { promise, resolve }; }

describe("external work snapshots", () => {
  it("동일 스냅샷은 참조를 유지하고 실행 중인 자동 진행을 일시정지시키지 않는다", async () => {
    const store = useWorkItemStore.getState(); await store.load();
    const id = store.add("Root")!;
    store.setAutoRun(id, { state: "running", judgeSessionId: "judge", judgeStarted: false, followUps: 0, costUsd: 0, log: [], startedAt: 1 });
    await store.flush();
    const before = useWorkItemStore.getState().items;
    load.mockResolvedValue(structuredClone(before));
    await store.refresh(); await store.refresh();
    expect(useWorkItemStore.getState().items).toBe(before);
    expect(useWorkItemStore.getState().items[0].autoRun?.state).toBe("running");
    expect(save).toHaveBeenCalledTimes(1);
  });
  it("외부의 오래된 실행 상태가 현재 자동 진행을 되돌리거나 새 실행을 시작하지 않는다", async () => {
    const store = useWorkItemStore.getState(); await store.load();
    const id = store.add("Root")!;
    const run = { state: "running" as const, judgeSessionId: "judge", judgeStarted: true, currentId: "step", followUps: 2, costUsd: 1, log: [], startedAt: 1 };
    store.setAutoRun(id, run);
    const inactive = store.add("Inactive")!; await store.flush();
    const current = useWorkItemStore.getState().items[0];
    const stale = { ...run, currentId: "old-step", followUps: 0 };
    load.mockResolvedValueOnce([{ ...current, body: "External memo", autoRun: stale }, work(inactive, { autoRun: stale }), work("new-root", { autoRun: stale })]);
    await store.refresh();
    const state = useWorkItemStore.getState();
    expect(state.items[0]).toMatchObject({ body: "External memo", autoRun: run });
    expect(state.items[0].autoRun).toBe(run);
    expect(state.items[1].autoRun).toBeUndefined();
    expect(state.items[2].autoRun).toMatchObject({ state: "paused", pauseReason: "app-restarted" });
    expect(state.dirty).toBe(true);
    await store.flush();
    expect(save.mock.lastCall![0][0].autoRun).toEqual(run);
    load.mockResolvedValueOnce(state.items.map((item) => item.id === id ? { ...item, status: "done", autoRun: stale } : item));
    await store.refresh();
    expect(useWorkItemStore.getState().items[0].autoRun).toMatchObject({ state: "finished", currentId: undefined });
  });
  it("읽는 동안 편집한 필드는 보존하고 외부 태스크 추가·연결·삭제를 합친다", async () => {
    const root = work("root", { body: "Old memo" });
    const removed = work("removed");
    load.mockResolvedValueOnce([root, removed]);
    const store = useWorkItemStore.getState(); await store.load(); store.select(removed.id);
    const read = deferred<WorkItem[]>();
    load.mockReturnValueOnce(read.promise);
    const refresh = store.refresh(); const duplicate = store.refresh();
    store.edit(root.id, { title: "Local title" });
    const external = [work(root.id, { body: "New memo", dependsOn: ["step"] }), work("step")];
    read.resolve(external); await Promise.all([refresh, duplicate]);
    expect(load).toHaveBeenCalledTimes(2);
    const state = useWorkItemStore.getState();
    expect(state.items.map((item) => item.id)).toEqual([root.id, "step"]);
    expect(state.items[0]).toMatchObject({ title: "Local title", body: "New memo", dependsOn: ["step"] });
    expect(state.selectedId).toBeNull(); expect(state.dirty).toBe(true);
    await store.flush();
    expect(save).toHaveBeenLastCalledWith(state.items, external);
  });
  it("외부에서 삭제하는 동안 앱에서 편집한 업무와 앱의 삭제·선택 필드 비우기를 보존한다", async () => {
    const edited = work("edited"), deleted = work("deleted"), cleared = work("cleared", { dependsOn: ["edited"] });
    load.mockResolvedValueOnce([edited, deleted, cleared]);
    const store = useWorkItemStore.getState(); await store.load();
    store.edit(edited.id, { body: "Keep local edit" }); store.remove(deleted.id);
    store.edit(cleared.id, { dependsOn: undefined });
    load.mockResolvedValueOnce([{ ...deleted, body: "External edit" }, { ...cleared, title: "External title" }]);
    await store.refresh();
    const items = useWorkItemStore.getState().items;
    expect(items.find((item) => item.id === edited.id)?.body).toBe("Keep local edit");
    expect(items.some((item) => item.id === deleted.id)).toBe(false);
    expect(items.find((item) => item.id === cleared.id)).toMatchObject({ title: "External title", dependsOn: undefined });
  });
  it("poll 전에 외부 변경이 들어오면 오래된 저장을 거절한 뒤 병합해서 재시도한다", async () => {
    const root = work("root");
    load.mockResolvedValueOnce([root]);
    const store = useWorkItemStore.getState(); await store.load(); store.edit(root.id, { title: "Local" });
    save.mockRejectedValueOnce("work-items-changed");
    const external = [work(root.id, { dependsOn: ["step"] }), work("step")];
    const read = deferred<WorkItem[]>(); load.mockReturnValueOnce(read.promise);
    const flush = store.flush(); await Promise.resolve();
    store.edit(root.id, { body: "Edited during conflict" });
    read.resolve(external); await flush;
    expect(save).toHaveBeenCalledTimes(2);
    expect(save.mock.calls[0][1]).toEqual([root]); expect(save.mock.calls[1][1]).toBe(external);
    expect(save.mock.calls[1][0][0]).toMatchObject({ title: "Local", body: "Edited during conflict", dependsOn: ["step"] });
    expect(useWorkItemStore.getState().items[1].id).toBe("step");
    expect(useWorkItemStore.getState().dirty).toBe(false);
  });
  it("읽기와 저장이 겹쳐도 옛 스냅샷을 되살리지 않고 교착 없이 직렬화한다", async () => {
    load.mockResolvedValueOnce([work("root")]);
    const store = useWorkItemStore.getState(); await store.load(); store.edit("root", { title: "Local" });
    const read = deferred<WorkItem[]>(); load.mockReturnValueOnce(read.promise);
    const refresh = store.refresh(), flush = store.flush();
    expect(save).not.toHaveBeenCalled();
    read.resolve([work("root", { body: "External" })]); await Promise.all([refresh, flush]);
    const written = deferred<void>(); save.mockReturnValueOnce(written.promise);
    store.edit("root", { title: "Latest" });
    const secondFlush = store.flush(); const latest = useWorkItemStore.getState().items;
    load.mockResolvedValueOnce(structuredClone(latest));
    const secondRefresh = store.refresh();
    written.resolve(); await Promise.all([secondFlush, secondRefresh]);
    expect(useWorkItemStore.getState().items).toBe(latest);
    expect(useWorkItemStore.getState().items[0]).toMatchObject({ title: "Latest", body: "External" });
  });
  it("읽기 실패와 계속되는 충돌은 편집을 잃지 않고 다음 시도에서 복구한다", async () => {
    load.mockResolvedValueOnce([work("root")]);
    const store = useWorkItemStore.getState(); await store.load(); store.edit("root", { title: "Local" });
    load.mockRejectedValueOnce(new Error("partial json"));
    expect(await store.refresh()).toBe(false);
    expect(useWorkItemStore.getState().items[0].title).toBe("Local");
    save.mockRejectedValue("work-items-changed"); load.mockResolvedValue([work("root")]);
    await store.flush();
    expect(save).toHaveBeenCalledTimes(3);
    expect(useWorkItemStore.getState()).toMatchObject({ dirty: true, error: "save" });
    save.mockResolvedValue(undefined); await store.flush();
    expect(useWorkItemStore.getState()).toMatchObject({ dirty: false, error: null });
  });
});
