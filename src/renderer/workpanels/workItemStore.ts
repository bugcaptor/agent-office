import { create } from "zustand";
import { nanoid } from "nanoid";
import type { WorkItem, WorkStatus } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { hasWorkContent, type WorkItemDraft } from "./workItemDraft";
import { explainDependsOn, normalizeDependsOn, type DroppedDependency } from "./workDependencies";
import type { ImportedWorkItem } from "./workItemFile";

export const WORK_STATUS_ORDER: WorkStatus[] = ["review", "in_progress", "todo", "done"];
let timer: ReturnType<typeof setTimeout> | undefined;
let saving: Promise<void> | undefined;
let loading: Promise<void> | undefined;
let revision = 0;
export type InboxImport = Pick<WorkItem, "id" | "title" | "body" | "assigneeAgentId" | "dependsOn">;
/**
 * 수신함 업무 하나의 처리 결과.
 * - added: 새로 넣음. `dropped`는 버린 선행 작업.
 * - replayed: 같은 id·제목·본문이 이미 있음(저장 뒤 ack 전에 죽은 재수신). 파일만 지운다.
 * - duplicate: 같은 id에 내용이 다름. 넣지 않고 거부한다.
 * - skipped: 제목·본문이 비어 건너뜀.
 */
export type InboxImportResult = { kind: "added"; dropped: DroppedDependency[] } | { kind: "replayed" | "duplicate" | "skipped" };
/** 수신함의 기존 업무 변경 요청. 수정의 `assigneeAgentId: null`은 미배정이다. */
export type InboxRequest =
  | { kind: "done"; ids: string[] }
  | { kind: "delete"; ids: string[] }
  | { kind: "edit"; id: string; patch: Partial<Pick<WorkItem, "title" | "body" | "status">> & { assigneeAgentId?: string | null; dependsOn?: string[]; addDependsOn?: string[]; removeDependsOn?: string[] } };
/**
 * 변경 요청 하나의 결과. `unknown`이 있거나 `empty`면 반영하지 않았다.
 * `dropped`는 수정에서 요청했지만 버린 선행 작업이다.
 */
export interface InboxRequestResult { unknown: string[]; empty?: boolean; dropped: DroppedDependency[] }
interface WorkItemState {
  items: WorkItem[];
  loaded: boolean;
  dirty: boolean;
  error: "load" | "save" | null;
  selectedId: string | null;
  load(): Promise<void>;
  add(draft: string | WorkItemDraft): string | undefined;
  /** 입력 순서대로 결과를 돌려준다. 로드 전·오류 중에는 빈 배열. */
  importInbox(entries: InboxImport[]): InboxImportResult[];
  /** 내보낸 파일 가져오기. 항상 새 id로 추가하고 추가한 수를 돌려준다. */
  importFile(entries: ImportedWorkItem[]): number;
  /**
   * 수신함 완료·삭제·수정 요청들을 차례로 적용한다. 요청마다 목록에 없는 id를 돌려주고,
   * 하나라도 없거나 수정 결과가 빈 업무면 그 요청은 반영하지 않는다(이미 완료는 건너뛴다).
   * 로드 전·오류 중에는 빈 배열.
   */
  applyInbox(requests: InboxRequest[]): InboxRequestResult[];
  edit(id: string, patch: Partial<Pick<WorkItem, "title" | "body" | "assigneeAgentId" | "agent" | "status" | "dependsOn">>): void;
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
    const dependsOn = normalizeDependsOn(id, draft.dependsOn, get().items);
    set({ items: [...get().items, { ...draft, id, title: draft.title.trim(), dependsOn, order, sessionIds: [], createdAt: at, updatedAt: at }], dirty: true, selectedId: id });
    changed();
    return id;
  },
  importInbox(entries) {
    if (!get().loaded || get().error) return [];
    const byId = new Map(get().items.map((item) => [item.id, item]));
    let order = Math.max(-1, ...get().items.filter((item) => item.status === "todo").map((item) => item.order)) + 1;
    const at = Date.now();
    const added: WorkItem[] = [];
    const results = entries.map((entry): InboxImportResult => {
      if (!entry.id || !hasWorkContent(entry)) return { kind: "skipped" };
      const existing = byId.get(entry.id);
      if (existing) return { kind: existing.title === entry.title && existing.body === entry.body ? "replayed" : "duplicate" };
      const item: WorkItem = { ...entry, status: "todo", order: order++, sessionIds: [], createdAt: at, updatedAt: at };
      byId.set(item.id, item);
      added.push(item);
      return { kind: "added", dropped: [] };
    });
    // 같은 주기에 함께 들어온 업무끼리도 이을 수 있도록 모두 넣은 목록으로 다듬는다.
    const all = [...get().items, ...added];
    entries.forEach((entry, n) => {
      const result = results[n];
      const item = result.kind === "added" ? byId.get(entry.id) : undefined;
      if (!item || result.kind !== "added") return;
      const { kept, dropped } = explainDependsOn(item.id, item.dependsOn, all);
      item.dependsOn = kept;
      result.dropped = dropped;
    });
    if (added.length) { set({ items: all, dirty: true }); changed(); }
    return results;
  },
  importFile(entries) {
    if (!get().loaded || get().error || !entries.length) return 0;
    // 파일 안 선행 작업은 새 id로 다시 잇는다. 파일 밖 id는 지금 목록에 있으면 그대로 둔다.
    const ids = new Map<string, string>();
    for (const entry of entries) if (entry.sourceId && !ids.has(entry.sourceId)) ids.set(entry.sourceId, nanoid());
    const used = new Set<string>();
    const orders = new Map(WORK_STATUS_ORDER.map((status) => [status, Math.max(-1, ...get().items.filter((i) => i.status === status).map((i) => i.order)) + 1]));
    const at = Date.now();
    // 세션·오류는 원래 카드의 실행 기록이라 복사본에 잇지 않는다.
    // 담당 이름은 파일의 참고 값이라 저장하지 않는다.
    const added: WorkItem[] = entries.map(({ sourceId, assigneeName: _name, ...entry }) => {
      const mapped = sourceId ? ids.get(sourceId) : undefined;
      const id = mapped && !used.has(mapped) ? mapped : nanoid();
      used.add(id);
      const order = orders.get(entry.status)!;
      orders.set(entry.status, order + 1);
      return { ...entry, id, dependsOn: entry.dependsOn?.map((d) => ids.get(d) ?? d), order, sessionIds: [], updatedAt: at };
    });
    const all = [...get().items, ...added];
    for (const item of added) item.dependsOn = normalizeDependsOn(item.id, item.dependsOn, all);
    set({ items: all, dirty: true });
    changed();
    return added.length;
  },
  applyInbox(requests) {
    if (!get().loaded || get().error) return [];
    let items = get().items;
    let selectedId = get().selectedId;
    const at = Date.now();
    const nextOrder = (status: WorkStatus) => Math.max(-1, ...items.filter((i) => i.status === status).map((i) => i.order)) + 1;
    const results = requests.map((request): InboxRequestResult => {
      const known = new Set(items.map((item) => item.id));
      const unknown = (request.kind === "edit" ? [request.id] : request.ids).filter((id) => !known.has(id));
      if (unknown.length) return { unknown, dropped: [] };
      if (request.kind === "done") {
        let order = nextOrder("done");
        if (items.some((item) => request.ids.includes(item.id) && item.status !== "done")) {
          items = items.map((item) => !request.ids.includes(item.id) || item.status === "done" ? item : { ...item, status: "done" as const, order: order++, updatedAt: at });
        }
        return { unknown, dropped: [] };
      }
      if (request.kind === "delete") {
        // 지운 업무를 기다리던 후속 업무는 그 선행 조건만 풀린다.
        items = items.filter((item) => !request.ids.includes(item.id)).map((item) => {
          if (!item.dependsOn?.some((d) => request.ids.includes(d))) return item;
          const dependsOn = item.dependsOn.filter((d) => !request.ids.includes(d));
          return { ...item, dependsOn: dependsOn.length ? dependsOn : undefined, updatedAt: at };
        });
        if (selectedId && request.ids.includes(selectedId)) selectedId = null;
        return { unknown, dropped: [] };
      }
      const { patch } = request;
      const item = items.find((i) => i.id === request.id)!;
      const next: WorkItem = { ...item, updatedAt: at };
      if (patch.title !== undefined) next.title = patch.title.trim();
      if (patch.body !== undefined) next.body = patch.body;
      if (patch.assigneeAgentId !== undefined) next.assigneeAgentId = patch.assigneeAgentId ?? undefined;
      if (patch.status && patch.status !== item.status) { next.status = patch.status; next.order = nextOrder(patch.status); }
      if (!hasWorkContent(next)) return { unknown, empty: true, dropped: [] };
      let dropped: DroppedDependency[] = [];
      if (patch.dependsOn || patch.addDependsOn?.length || patch.removeDependsOn?.length) {
        const base = (patch.dependsOn ?? item.dependsOn ?? []).filter((d) => !patch.removeDependsOn?.includes(d));
        const explained = explainDependsOn(item.id, [...base, ...patch.addDependsOn ?? []], items);
        next.dependsOn = explained.kept;
        // 이미 걸려 있던 선행이 아니라 이번에 요청한 것만 알린다.
        const requested = new Set([...patch.dependsOn ?? [], ...patch.addDependsOn ?? []]);
        dropped = explained.dropped.filter((d) => requested.has(d.id));
      }
      items = items.map((i) => i.id === item.id ? next : i);
      return { unknown, dropped };
    });
    if (items !== get().items) { set({ items, selectedId, dirty: true }); changed(); }
    return results;
  },
  edit(id, patch) {
    if (!get().loaded) return;
    const item = get().items.find((i) => i.id === id);
    if (!item || !hasWorkContent({ ...item, ...patch })) return;
    if (patch.title !== undefined) patch = { ...patch, title: patch.title.trim() };
    if ("dependsOn" in patch) patch = { ...patch, dependsOn: normalizeDependsOn(id, patch.dependsOn, get().items) };
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
    // 지운 업무를 기다리던 후속 업무는 그 선행 조건만 풀린다.
    const items = get().items.filter((i) => i.id !== id).map((i) => {
      if (!i.dependsOn?.includes(id)) return i;
      const dependsOn = i.dependsOn.filter((d) => d !== id);
      return { ...i, dependsOn: dependsOn.length ? dependsOn : undefined };
    });
    set({ items, dirty: true, selectedId: get().selectedId === id ? null : get().selectedId });
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
