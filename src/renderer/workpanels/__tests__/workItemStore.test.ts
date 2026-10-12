import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const { load, save } = vi.hoisted(() => ({ load: vi.fn(), save: vi.fn() }));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: { loadWorkItems: load, saveWorkItems: save } }));
import { useWorkItemStore } from "../workItemStore";
const initial = useWorkItemStore.getState();
beforeEach(() => { vi.useFakeTimers(); load.mockReset().mockResolvedValue([]); save.mockReset().mockResolvedValue(undefined); useWorkItemStore.setState(initial, true); });
afterEach(async () => { await useWorkItemStore.getState().flush(); vi.useRealTimers(); });
describe("work items", () => {
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
