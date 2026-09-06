import { create } from "zustand";
import type { RepositoryAuditItem } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";

interface RepositoryAuditState {
  open: boolean;
  items: RepositoryAuditItem[];
  loading: boolean;
  error: boolean;
  openOverlay(): void;
  close(): void;
  refresh(): Promise<void>;
}

export const useRepositoryAuditStore = create<RepositoryAuditState>((set, get) => ({
  open: false,
  items: [],
  loading: false,
  error: false,
  openOverlay: () => {
    set({ open: true, items: [], error: false });
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
}));
