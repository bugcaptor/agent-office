import { create } from "zustand";
import type { RepositoryAuditItem } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";

interface RepositoryAuditState {
  open: boolean;
  items: RepositoryAuditItem[];
  loading: boolean;
  error: boolean;
  /** push가 진행 중인 저장소 경로. 저장소마다 따로 누를 수 있다. */
  pushing: Record<string, true>;
  /** 마지막 push가 실패한 저장소와 git이 남긴 거부 사유(없으면 빈 문자열). */
  pushErrors: Record<string, string>;
  openOverlay(): void;
  close(): void;
  refresh(): Promise<void>;
  push(path: string): Promise<void>;
}

function without<T>(record: Record<string, T>, key: string): Record<string, T> {
  const { [key]: _, ...rest } = record;
  return rest;
}

export const useRepositoryAuditStore = create<RepositoryAuditState>((set, get) => ({
  open: false,
  items: [],
  loading: false,
  error: false,
  pushing: {},
  pushErrors: {},
  openOverlay: () => {
    set({ open: true, items: [], error: false, pushErrors: {} });
    void get().refresh();
  },
  close: () => set({ open: false }),
  refresh: async () => {
    if (get().loading) return;
    set({ loading: true, error: false });
    try {
      const items = await tauriApi.repositoryAuditList();
      if (get().open) set({ items });
      set({ loading: false });
    } catch (error) {
      console.warn("repository audit: load failed", error);
      if (get().open) set({ error: true });
      set({ loading: false });
    }
  },
  push: async (path) => {
    if (get().pushing[path]) return;
    set((s) => ({ pushing: { ...s.pushing, [path]: true }, pushErrors: without(s.pushErrors, path) }));
    try {
      const fresh = await tauriApi.repositoryAuditPush(path);
      // 행 순서는 다음 새로고침까지 그대로 둔다. 누른 행이 목록에서 튀지 않게
      // 상태만 바꾸고, 최근 작업 시각은 push 결과에 없으므로 기존 값을 유지한다.
      set((s) => ({
        items: s.items.map((item) => (item.path === path ? { ...fresh, lastWorkedAt: item.lastWorkedAt } : item)),
      }));
    } catch (error) {
      console.warn("repository audit: push failed", error);
      set((s) => ({ pushErrors: { ...s.pushErrors, [path]: typeof error === "string" ? error : "" } }));
    } finally {
      set((s) => ({ pushing: without(s.pushing, path) }));
    }
  },
}));
