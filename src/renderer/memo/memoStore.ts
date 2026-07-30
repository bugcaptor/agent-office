// src/renderer/memo/memoStore.ts
//
// 에이전트별 포스트잇 메모(#79) 전용 zustand 스토어. diaryStore/markdownStore와
// 같은 관례로 appStore에서 분리했다 — 이 상태(위젯 열림/현재 장/아카이브
// 다이얼로그)는 오피스 씬·세션과 무관한 독립 서브시스템이라 커플링을 피한다.
// 활성 탭이 무엇인지는 위젯(PostItWidget)이 appStore에서 읽어 `focusAgent`로
// 밀어 넣는다 — 스토어가 appStore를 import하지 않는 이유다.
//
// 저장 규약(저장 버튼 없음):
//   - 타이핑 → `edit()`가 1초 디바운스 후 `saveMemo`.
//   - blur / 에이전트 전환 / 위젯 닫힘 → `flush()`가 디바운스를 앞당겨 즉시 저장.
// 즉 사용자가 인지하는 "저장"은 항상 자동이고, 마지막 타이핑은 어떤 경로로든
// 반드시 디스크에 닿는다.
import { create } from "zustand";
import { tauriApi } from "../ipc/tauriApi";
import type { MemoSheet, MemoSheetMeta } from "@shared/types";
import { loadStoredMemoVisible, persistMemoVisible } from "./memoVisibility";

/** 자동저장 디바운스(ms). 타이핑 중에는 쓰지 않고, 손을 멈추면 저장한다. */
export const SAVE_DEBOUNCE_MS = 1000;

/** 열린 아카이브 다이얼로그가 가리키는 캐릭터. null = 닫힘. */
export interface MemoArchiveTarget {
  agentId: string;
  /** 헤더 표시용 이름 스냅샷(열 때 캡처). */
  agentName: string;
}

/** 디바운스 타이머는 모듈 스코프에 둔다 — 스토어 상태가 아니므로(렌더 유발 없음)
 *  그리고 스토어가 재생성되지 않는 싱글턴이므로 안전하다. */
let saveTimer: ReturnType<typeof setTimeout> | null = null;

function cancelPendingSave(): void {
  if (saveTimer !== null) {
    clearTimeout(saveTimer);
    saveTimer = null;
  }
}

/** 클립보드 복사. 지원하지 않는 환경(jsdom/구형)에서는 false를 돌려준다. */
async function writeClipboard(text: string): Promise<boolean> {
  try {
    if (typeof navigator === "undefined" || !navigator.clipboard) return false;
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

interface MemoState {
  /** 위젯 열림 여부(localStorage 영속, 전역). */
  visible: boolean;
  /** 지금 위젯이 보여주는 캐릭터. null = 아직 어떤 장도 로드하지 않음. */
  agentId: string | null;
  /** 로드된 현재 장의 메타(본문의 진실은 `draft`). */
  sheet: MemoSheet | null;
  /** textarea가 보여주는 본문(편집 중 값). */
  draft: string;
  /** 마지막 저장 이후 미저장 편집이 있는지. */
  dirty: boolean;
  loading: boolean;
  /** 한 장 넘기기 진행 중(중복 실행 방지 겸 버튼 상태). */
  archiving: boolean;
  /** 위젯 하단 안내(복사/넘기기/실패). null = 없음. */
  notice: string | null;

  /** 아카이브 다이얼로그 대상. null = 닫힘. */
  archive: MemoArchiveTarget | null;
  archiveItems: MemoSheetMeta[];
  archiveLoading: boolean;
  /** 목록에서 고른 장의 전체 내용. null = 아직 고르지 않음. */
  archiveSelected: MemoSheet | null;
  archiveNotice: string | null;

  /** 위젯 열림 상태 설정. 닫을 때는 미저장 편집을 먼저 flush 한다. */
  setVisible(visible: boolean): void;
  toggleVisible(): void;
  /** 위젯이 보여줄 캐릭터를 바꾼다(탭 전환). 이전 캐릭터의 편집을 먼저 flush. */
  focusAgent(agentId: string): Promise<void>;
  /** 본문 편집 — 디바운스 자동저장을 예약한다. */
  edit(content: string): void;
  /** 예약된 저장을 앞당겨 즉시 수행한다(blur/전환/닫힘/넘기기 직전). */
  flush(): Promise<void>;
  /** 현재 장을 통째로 아카이브하고 새 빈 장으로 넘어간다. */
  archiveNow(): Promise<void>;
  /** 현재 본문 전체를 클립보드에 복사한다(콘솔로의 유일한 연동 경로). */
  copyAll(): Promise<void>;

  openArchive(agentId: string, agentName: string): Promise<void>;
  closeArchive(): void;
  /** 아카이브 목록의 한 장을 골라 본문을 읽어 온다. */
  selectSheet(sheetId: string): Promise<void>;
  /** 고른 아카이브 장의 본문을 복사한다. */
  copySelected(): Promise<void>;
}

export const useMemoStore = create<MemoState>((set, get) => ({
  visible: loadStoredMemoVisible(),
  agentId: null,
  sheet: null,
  draft: "",
  dirty: false,
  loading: false,
  archiving: false,
  notice: null,

  archive: null,
  archiveItems: [],
  archiveLoading: false,
  archiveSelected: null,
  archiveNotice: null,

  setVisible: (visible) => {
    if (get().visible === visible) return;
    // 닫기 전에 마지막 타이핑을 확정한다(저장 버튼이 없으므로 이게 유일한 보장).
    if (!visible) void get().flush();
    persistMemoVisible(visible);
    set({ visible, notice: null });
  },

  toggleVisible: () => get().setVisible(!get().visible),

  focusAgent: async (agentId) => {
    const prev = get();
    if (prev.agentId === agentId && prev.sheet !== null) return;
    // 이전 캐릭터의 미저장 편집을 먼저 내려쓴다 — agentId를 갈아치우기 전에.
    await get().flush();
    set({
      agentId,
      sheet: null,
      draft: "",
      dirty: false,
      loading: true,
      notice: null,
    });
    try {
      const sheet = await tauriApi.loadMemo(agentId);
      // 그 사이 또 전환됐으면 반영하지 않는다(stale 방지).
      if (get().agentId !== agentId) return;
      set({ sheet, draft: sheet.content, dirty: false, loading: false });
    } catch (err) {
      console.warn("memo: 메모 로드 실패", err);
      if (get().agentId !== agentId) return;
      set({ loading: false, notice: "메모를 불러오지 못했습니다." });
    }
  },

  edit: (content) => {
    set({ draft: content, dirty: true, notice: null });
    cancelPendingSave();
    saveTimer = setTimeout(() => {
      saveTimer = null;
      void get().flush();
    }, SAVE_DEBOUNCE_MS);
  },

  flush: async () => {
    cancelPendingSave();
    const { agentId, sheet, draft, dirty } = get();
    if (!dirty || !agentId || !sheet) return;
    // dirty를 먼저 내린다 — 저장 중에 들어온 타이핑이 다시 dirty로 올려
    // 다음 디바운스에 잡히게(저장 유실 없이 최신값이 다시 쓰인다).
    set({ dirty: false });
    try {
      await tauriApi.saveMemo(agentId, sheet.sheetId, draft);
    } catch (err) {
      console.warn("memo: 메모 저장 실패", err);
      // 실패는 되돌린다 — 다음 flush에서 재시도된다.
      set({ dirty: true, notice: "메모를 저장하지 못했습니다." });
    }
  },

  archiveNow: async () => {
    const { agentId, sheet, draft, archiving } = get();
    if (!agentId || !sheet || archiving) return;
    // 빈 장은 넘길 게 없다(버튼도 비활성 — 여기 가드는 이중 안전장치).
    if (draft.trim() === "") return;
    set({ archiving: true, notice: null });
    // 넘기기 전에 본문을 확정한다 — 아카이브된 장에 마지막 타이핑이 담기도록.
    await get().flush();
    try {
      const fresh = await tauriApi.archiveMemoSheet(agentId);
      if (get().agentId !== agentId) {
        set({ archiving: false });
        return;
      }
      set({
        sheet: fresh,
        draft: fresh.content,
        dirty: false,
        archiving: false,
        notice: "한 장 넘겼습니다. 지난 장은 ‘메모 아카이브’에서 볼 수 있습니다.",
      });
    } catch (err) {
      console.warn("memo: 한 장 넘기기 실패", err);
      if (get().agentId !== agentId) {
        set({ archiving: false });
        return;
      }
      set({ archiving: false, notice: "한 장 넘기지 못했습니다." });
    }
  },

  copyAll: async () => {
    const text = get().draft;
    if (text === "") {
      set({ notice: "복사할 내용이 없습니다." });
      return;
    }
    set({
      notice: (await writeClipboard(text))
        ? "메모 전체를 복사했습니다."
        : "복사에 실패했습니다.",
    });
  },

  openArchive: async (agentId, agentName) => {
    // 아카이브를 열기 전에 현재 장을 확정해 둔다 — 방금 넘긴 장의 본문이
    // 목록/열람에서 최신으로 보이게.
    await get().flush();
    set({
      archive: { agentId, agentName },
      archiveItems: [],
      archiveSelected: null,
      archiveLoading: true,
      archiveNotice: null,
    });
    try {
      const items = await tauriApi.listMemoArchive(agentId);
      if (get().archive?.agentId !== agentId) return;
      set({ archiveItems: items, archiveLoading: false });
    } catch (err) {
      console.warn("memo: 아카이브 목록 로드 실패", err);
      if (get().archive?.agentId !== agentId) return;
      set({ archiveLoading: false, archiveNotice: "아카이브를 불러오지 못했습니다." });
    }
  },

  closeArchive: () =>
    set({
      archive: null,
      archiveItems: [],
      archiveSelected: null,
      archiveNotice: null,
      archiveLoading: false,
    }),

  selectSheet: async (sheetId) => {
    const target = get().archive;
    if (!target) return;
    const { agentId } = target;
    set({ archiveSelected: null, archiveNotice: null });
    try {
      const sheet = await tauriApi.readMemoSheet(agentId, sheetId);
      if (get().archive?.agentId !== agentId) return;
      set({ archiveSelected: sheet });
    } catch (err) {
      console.warn("memo: 아카이브 장 읽기 실패", err);
      if (get().archive?.agentId !== agentId) return;
      set({ archiveNotice: "이 장을 읽지 못했습니다." });
    }
  },

  copySelected: async () => {
    const sheet = get().archiveSelected;
    if (!sheet) return;
    set({
      archiveNotice: (await writeClipboard(sheet.content))
        ? "이 장을 복사했습니다."
        : "복사에 실패했습니다.",
    });
  },
}));
