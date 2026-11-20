// src/renderer/discussion/discussionStore.ts
//
// 팀 토의(`discussions.json`)의 렌더러 캐시와 업무 패널의 토의 대화상자 상태.
// 저장은 토의 하나씩 upsert·삭제한다. 손상 파일(로드 reject)이면 목록을 비우고
// 저장도 백엔드가 거부한다.
import { create } from "zustand";
import type { Discussion } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";

type Status = "idle" | "loading" | "ready" | "error";

/** 업무 패널 위에 뜨는 토의 대화상자. 한 번에 하나만. */
export type DiscussionDialog = { kind: "new" } | { kind: "view"; id: string } | { kind: "edit"; id: string } | null;

interface DiscussionStoreState {
  discussions: Discussion[];
  status: Status;
  error?: string;
  dialog: DiscussionDialog;
  ensureLoaded(): Promise<void>;
  load(): Promise<void>;
  save(discussion: Discussion): Promise<void>;
  remove(id: string): Promise<void>;
  openDialog(dialog: DiscussionDialog): void;
}

let inflight: Promise<void> | null = null;

export const useDiscussionStore = create<DiscussionStoreState>((set, get) => ({
  discussions: [],
  status: "idle",
  dialog: null,
  ensureLoaded: () => (get().status === "ready" ? Promise.resolve() : get().load()),
  load: () => {
    if (inflight) return inflight;
    set({ status: "loading", error: undefined });
    inflight = (async () => {
      try {
        set({ discussions: await tauriApi.loadDiscussions(), status: "ready" });
      } catch (e) {
        set({ status: "error", error: String(e) });
      } finally {
        inflight = null;
      }
    })();
    return inflight;
  },
  save: async (discussion) => {
    await tauriApi.saveDiscussion(discussion);
    set((s) => ({
      discussions: s.discussions.some((d) => d.id === discussion.id)
        ? s.discussions.map((d) => (d.id === discussion.id ? discussion : d))
        : [...s.discussions, discussion],
    }));
  },
  remove: async (id) => {
    await tauriApi.deleteDiscussion(id);
    set((s) => ({ discussions: s.discussions.filter((d) => d.id !== id), dialog: null }));
  },
  openDialog: (dialog) => set({ dialog }),
}));

/** 카드 순서: 끝나지 않은 토의 먼저, 그 안에서는 최근에 바뀐 순. */
export function sortDiscussions(list: readonly Discussion[]): Discussion[] {
  return [...list].sort((a, b) => Number(a.state === "finished") - Number(b.state === "finished") || b.updatedAt - a.updatedAt);
}

/** 테스트용 초기화. */
export function resetDiscussionStoreForTest(): void {
  inflight = null;
  useDiscussionStore.setState({ discussions: [], status: "idle", error: undefined, dialog: null });
}
