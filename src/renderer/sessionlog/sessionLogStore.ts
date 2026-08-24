// src/renderer/sessionlog/sessionLogStore.ts
//
// 세션 로그 보기 오버레이 전용 zustand 스토어(diaryStore와 같은 관례).
// 설계: docs/session-log-design.md §7.
//
// 목록은 10개씩 페이징한다 -- 한 캐릭터가 한 달치를 쌓으면 수백 개가 되므로
// 전부 들고 오지 않는다. 항목을 고르면 그때 동작(편집기로 열기 / 학습자료
// 만들기)이 노출된다.
import { create } from "zustand";
import { t } from "@renderer/i18n";
import { tauriApi } from "../ipc/tauriApi";
import type { SessionLogItem } from "@shared/types";
import { useMarkdownStore } from "../markdown/markdownStore";

/** 한 페이지 크기. 사용자가 정한 값(10개)이다. */
export const PAGE_SIZE = 10;

export interface SessionLogOverlayTarget {
  agentId: string;
  /** 헤더 표시용 이름 스냅샷(열 때 캡처). */
  agentName: string;
}

interface SessionLogState {
  overlay: SessionLogOverlayTarget | null;
  items: SessionLogItem[];
  total: number;
  /** 0-based 페이지 번호. */
  page: number;
  /** 선택된 로그의 `path`. null이면 동작 영역을 보여주지 않는다. */
  selected: string | null;
  loading: boolean;
  /** 학습자료 생성 중(수십 초 걸린다). 중복 실행 방지 겸 버튼 상태. */
  generating: boolean;
  notice: string | null;

  open(agentId: string, agentName: string): void;
  close(): void;
  setPage(page: number): Promise<void>;
  select(path: string | null): void;
  refresh(): Promise<void>;
  /** 선택된 로그를 외부 에디터로 연다. */
  openInEditor(): Promise<void>;
  /** 선택된 로그로 학습자료를 만들고 인앱 마크다운 뷰어로 연다. */
  makeStudyMaterial(): Promise<void>;
}

/**
 * 백엔드 실패 문자열 → 사용자 안내.
 *
 * React 밖(zustand 액션)이라 훅이 아니라 모듈 `t`를 쓴다 — 다만 **호출
 * 시점**에만 부른다(모듈 최상위에서 부르면 언어를 바꿔도 문구가 굳는다).
 *
 * 빈 로그 판정만 정규식인 건, 백엔드(`session_log/study.rs`)가 한국어 에러
 * 문자열("빈 로그입니다")을 그대로 돌려주기 때문이다. 이건 화면에 나가는
 * 문구가 아니라 **백엔드 응답을 알아보는 패턴**이라 카탈로그 대상이 아니다.
 */
function noticeForError(err: unknown): string {
  const message = err instanceof Error ? err.message : String(err);
  if (message.includes("summarizer-disabled")) {
    return t("activity:sessionLog.errSummarizerOff");
  }
  if (message.includes("-not-found")) {
    return t("activity:sessionLog.errCliNotFound");
  }
  if (message === "timeout") {
    return t("activity:sessionLog.errTimeout");
  }
  if (/빈 로그/.test(message)) {
    return t("activity:sessionLog.errEmptyLog");
  }
  return t("activity:sessionLog.errGeneric", { message });
}

export const useSessionLogStore = create<SessionLogState>((set, get) => ({
  overlay: null,
  items: [],
  total: 0,
  page: 0,
  selected: null,
  loading: false,
  generating: false,
  notice: null,

  open: (agentId, agentName) => {
    set({
      overlay: { agentId, agentName },
      items: [],
      total: 0,
      page: 0,
      selected: null,
      loading: true,
      notice: null,
    });
    void get().refresh();
  },

  close: () =>
    set({ overlay: null, items: [], total: 0, page: 0, selected: null, notice: null }),

  setPage: async (page) => {
    // 페이지를 넘기면 선택은 풀린다 -- 화면에 없는 항목이 선택된 채 남지 않게.
    set({ page, selected: null });
    await get().refresh();
  },

  select: (path) => set({ selected: path }),

  refresh: async () => {
    const overlay = get().overlay;
    if (!overlay) return;
    const { agentId } = overlay;
    const page = get().page;
    set({ loading: true });
    try {
      const result = await tauriApi.listSessionLogs(agentId, page * PAGE_SIZE, PAGE_SIZE);
      // 그 사이 다른 캐릭터로 전환/닫힘이면 반영하지 않는다(stale 방지).
      if (get().overlay?.agentId !== agentId) return;
      set({ items: result.items, total: result.total, loading: false });
    } catch (err) {
      console.warn("session-log: failed to load the list", err);
      if (get().overlay?.agentId !== agentId) return;
      set({ loading: false, notice: t("activity:sessionLog.noticeListError") });
    }
  },

  openInEditor: async () => {
    const path = get().selected;
    if (!path) return;
    try {
      await tauriApi.openSessionLog(path);
    } catch (err) {
      console.warn("session-log: failed to open in the editor", err);
      set({ notice: t("activity:sessionLog.noticeOpenError") });
    }
  },

  makeStudyMaterial: async () => {
    const overlay = get().overlay;
    const path = get().selected;
    if (!overlay || !path || get().generating) return;
    const { agentId } = overlay;

    set({ generating: true, notice: t("activity:sessionLog.noticeGenerating") });
    try {
      const result = await tauriApi.generateStudyMaterial(agentId, path);
      set({ generating: false, notice: null });
      // 인앱 마크다운 뷰어로 넘긴다(미리보기가 기본 뷰). 뷰어를 닫으면 이
      // 오버레이로 돌아오도록 다시 열어 준다.
      const { agentName } = overlay;
      get().close();
      void useMarkdownStore.getState().openFile(result.dir, result.fileName, agentId, () => {
        useSessionLogStore.getState().open(agentId, agentName);
      });
    } catch (err) {
      console.warn("session-log: failed to generate study material", err);
      set({ generating: false, notice: noticeForError(err) });
    }
  },
}));
