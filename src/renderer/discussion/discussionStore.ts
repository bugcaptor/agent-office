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
  /**
   * 실행기용 저장. 스토어를 먼저 바로 고치고 저장은 순서대로 내보낸다. 동시 라운드에서 여러 발언을
   * 거의 한꺼번에 받아도 옛 스냅샷이 새 것을 덮지 않는다. 저장 실패는 다음 저장이 통째로 바로잡는다.
   */
  commit(discussion: Discussion): Promise<void>;
  remove(id: string): Promise<void>;
  /**
   * 백엔드가 수신함에서 가져와 이미 저장한 토의를 붙인다. 아는 id는 건드리지 않는다 — 다시 읽으면(`load`) 실행기가
   * 스토어에만 먼저 고친 진행 상태를 디스크의 옛 값으로 되돌릴 수 있다.
   */
  adopt(discussions: readonly Discussion[]): void;
  openDialog(dialog: DiscussionDialog): void;
}

let inflight: Promise<void> | null = null;
let saving: Promise<void> = Promise.resolve();

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
  commit: (discussion) => {
    set((s) => ({
      discussions: s.discussions.some((d) => d.id === discussion.id)
        ? s.discussions.map((d) => (d.id === discussion.id ? discussion : d))
        : [...s.discussions, discussion],
    }));
    saving = saving.then(() => tauriApi.saveDiscussion(discussion)).catch((e: unknown) => { console.warn("discussion: save failed", e); });
    return saving;
  },
  remove: async (id) => {
    await tauriApi.deleteDiscussion(id);
    set((s) => ({ discussions: s.discussions.filter((d) => d.id !== id), dialog: null }));
  },
  adopt: (incoming) => {
    const fresh = incoming.filter((d, i) => !get().discussions.some((known) => known.id === d.id) && incoming.findIndex((x) => x.id === d.id) === i);
    if (fresh.length) set((s) => ({ discussions: [...s.discussions, ...fresh] }));
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
  saving = Promise.resolve();
  useDiscussionStore.setState({ discussions: [], status: "idle", error: undefined, dialog: null });
}
