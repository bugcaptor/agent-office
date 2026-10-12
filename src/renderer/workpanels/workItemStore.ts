import { create } from "zustand";
import { nanoid } from "nanoid";
import type { WorkItem, WorkStatus } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { hasWorkContent, type WorkItemDraft } from "./workItemDraft";

export const WORK_STATUS_ORDER: WorkStatus[] = ["review", "in_progress", "todo", "done"];
let timer: ReturnType<typeof setTimeout> | undefined;
let saving: Promise<void> | undefined;
let loading: Promise<void> | undefined;
let revision = 0;
interface WorkItemState {
  items: WorkItem[];
  loaded: boolean;
  dirty: boolean;
  error: "load" | "save" | null;
  selectedId: string | null;
  load(): Promise<void>;
  add(draft: string | WorkItemDraft): string | undefined;
  edit(id: string, patch: Partial<Pick<WorkItem, "title" | "body" | "assigneeAgentId" | "agent" | "status">>): void;
  recordSend(id: string, result: { sessionId?: string; error?: string }): void;
  remove(id: string): void;
  reorder(id: string, targetId: string, after?: boolean): void;
  select(id: string | null): void;
  flush(): Promise<void>;
}
export function hasPendingWorkItems(): boolean { return useWorkItemStore.getState().dirty || saving !== undefined; }
function changed() {
  revision++;
  clearTimeout(timer);
  timer = setTimeout(() => void useWorkItemStore.getState().flush(), 1000);
}
export const useWorkItemStore = create<WorkItemState>((set, get) => ({
  items: [], loaded: false, dirty: false, error: null, selectedId: null,
  async load() {
    if (get().loaded) return;
    if (loading) return loading;
    loading = (async () => {
      try {
        const items = await tauriApi.loadWorkItems();
        set({ items, loaded: true, error: null });
      } catch (err) {
        console.warn("work-items: load failed", err);
        // 로드 실패를 빈 목록으로 간주해 덮어쓰지 않는다. 편집은 재시도 성공 후에만.
        set({ error: "load" });
      }
    })();
    await loading;
    loading = undefined;
  },
  add(input) {
    const draft: WorkItemDraft = typeof input === "string" ? { title: input, body: "", status: "todo" } : input;
    if (!get().loaded || !hasWorkContent(draft)) return;
    const at = Date.now();
    const id = nanoid();
    const order = Math.max(-1, ...get().items.filter((i) => i.status === draft.status).map((i) => i.order)) + 1;
    set({ items: [...get().items, { ...draft, id, title: draft.title.trim(), order, sessionIds: [], createdAt: at, updatedAt: at }], dirty: true, selectedId: id });
    changed();
    return id;
  },
  edit(id, patch) {
    if (!get().loaded) return;
    const item = get().items.find((i) => i.id === id);
    if (!item || !hasWorkContent({ ...item, ...patch })) return;
    if (patch.title !== undefined) patch = { ...patch, title: patch.title.trim() };
    const order = patch.status && patch.status !== item.status
      ? Math.max(-1, ...get().items.filter((i) => i.status === patch.status).map((i) => i.order)) + 1 : item.order;
    set({ items: get().items.map((i) => i.id === id ? { ...i, ...patch, order, updatedAt: Date.now() } : i), dirty: true });
    changed();
  },
  recordSend(id, result) {
    const item = get().items.find((i) => i.id === id);
    if (!get().loaded || !item) return;
    const status = result.sessionId ? "in_progress" : item.status;
    const order = status !== item.status ? Math.max(-1, ...get().items.filter((i) => i.status === status).map((i) => i.order)) + 1 : item.order;
    set({ items: get().items.map((i) => i.id === id ? { ...i, status, order, lastError: result.error, sessionIds: result.sessionId ? [...new Set([...i.sessionIds, result.sessionId])] : i.sessionIds, updatedAt: Date.now() } : i), dirty: true });
    changed();
  },
  remove(id) {
    if (!get().loaded) return;
    set({ items: get().items.filter((i) => i.id !== id), dirty: true, selectedId: get().selectedId === id ? null : get().selectedId });
    changed();
  },
  reorder(id, beforeId, after = false) {
    const { items } = get();
    const item = items.find((i) => i.id === id);
    const before = items.find((i) => i.id === beforeId);
    if (!item || !before || item.status !== before.status || id === beforeId) return;
    const group = items.filter((i) => i.status === item.status && i.id !== id).sort((a, b) => a.order - b.order);
    group.splice(group.findIndex((i) => i.id === beforeId) + (after ? 1 : 0), 0, item);
    const ranks = new Map(group.map((i, n) => [i.id, n]));
    set({ items: items.map((i) => ranks.has(i.id) ? { ...i, order: ranks.get(i.id)!, updatedAt: Date.now() } : i), dirty: true });
    changed();
  },
  select(selectedId) { set({ selectedId }); if (selectedId === null) void get().flush(); },
  async flush() {
    clearTimeout(timer); timer = undefined;
    if (saving) { await saving; if (get().dirty && get().error !== "save") await get().flush(); return; }
    if (!get().loaded || !get().dirty) return;
    const savedRevision = revision;
    const items = get().items;
    saving = (async () => {
      try {
        await tauriApi.saveWorkItems(items);
        set({ dirty: revision !== savedRevision, error: null });
      } catch (err) {
        console.warn("work-items: save failed", err);
        set({ dirty: true, error: "save" });
      }
    })();
    await saving; saving = undefined;
    if (get().dirty && get().error === null) await get().flush();
  },
}));
