// src/renderer/diary/diaryStore.ts
//
// 캐릭터 일기(#56) 열람/생성 오버레이 전용 zustand 스토어. markdownStore/
// workdirStore와 같은 관례로 appStore에서 분리했다 — 이 상태(오버레이/일기 캐시)는
// 오피스 씬·세션과 무관한 독립 서브시스템이라 커플링을 피한다. 비동기
// 오케스트레이션(로드/생성)은 스토어 액션이 직접 tauriApi·generateDiary를
// 호출한다(테스트는 목으로 검증).
import { create } from "zustand";
import { tauriApi } from "../ipc/tauriApi";
import type { DiaryEntry } from "@shared/types";
import { generateDiary, type DiaryResult } from "./diaryGenerator";

/** 열린 오버레이가 가리키는 캐릭터. null = 닫힘. */
export interface DiaryOverlayTarget {
  agentId: string;
  /** 헤더 표시용 이름 스냅샷(열 때 캡처). */
  agentName: string;
}

/** 생성 결과 사유 → 사용자 안내 문구. */
function noticeFor(result: DiaryResult): string {
  if (result.ok) return "일기를 한 편 썼습니다.";
  switch (result.reason) {
    case "disabled":
      return "설정에서 ‘캐릭터 일기’를 먼저 켜 주세요.";
    case "no-work":
      return "아직 기록할 작업이 없습니다. 세션에서 무언가 한 뒤 다시 시도하세요.";
    case "in-flight":
      return "이미 일기를 쓰는 중입니다.";
    case "cli-missing":
      return "선택한 CLI를 찾지 못해 일기를 쓰지 못했습니다.";
    case "failed":
      return "일기 생성에 실패했습니다. 잠시 후 다시 시도하세요.";
  }
}

interface DiaryState {
  overlay: DiaryOverlayTarget | null;
  /** 현재 열린 캐릭터의 일기(작성순). 표시는 컴포넌트가 역순으로 한다. */
  entries: DiaryEntry[];
  loading: boolean;
  generating: boolean;
  /** 마지막 동작에 대한 사용자 안내(성공/실패). null = 없음. */
  notice: string | null;

  /** 오버레이를 연다 + 일기 로드 트리거. */
  openDiary(agentId: string, agentName: string): void;
  closeDiary(): void;
  /** 현재 열린(또는 지정한) 캐릭터의 일기를 다시 읽는다. */
  refresh(agentId: string): Promise<void>;
  /** 지금까지의 작업 로그로 일기 한 편을 생성한다(수동 트리거). */
  writeNow(agentId: string): Promise<void>;
}

export const useDiaryStore = create<DiaryState>((set, get) => ({
  overlay: null,
  entries: [],
  loading: false,
  generating: false,
  notice: null,

  openDiary: (agentId, agentName) => {
    set({ overlay: { agentId, agentName }, entries: [], loading: true, notice: null });
    void get().refresh(agentId);
  },

  closeDiary: () => set({ overlay: null, entries: [], notice: null }),

  refresh: async (agentId) => {
    set({ loading: true });
    try {
      const entries = await tauriApi.loadDiary(agentId);
      // 이미 다른 캐릭터로 전환됐으면 반영하지 않는다(stale 방지).
      if (get().overlay?.agentId !== agentId) return;
      set({ entries, loading: false });
    } catch (err) {
      console.warn("diary: 일기 로드 실패", err);
      if (get().overlay?.agentId !== agentId) return;
      set({ loading: false, notice: "일기를 불러오지 못했습니다." });
    }
  },

  writeNow: async (agentId) => {
    if (get().generating) return;
    set({ generating: true, notice: null });
    const result = await generateDiary(agentId);
    set({ generating: false, notice: noticeFor(result) });
    if (result.ok) await get().refresh(agentId);
  },
}));
