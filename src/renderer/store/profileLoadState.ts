// src/renderer/store/profileLoadState.ts
//
// 프로필 로드 실패(읽기 전용) 상태만 담는 가벼운 스토어. persist.ts가 이것만
// 보고 저장을 막는다 — 로드·재시도 로직(profileLoad.ts)은 세션 입양을 끌고
// 와 터미널 모듈까지 import하므로 분리해 둔다.
import { create } from "zustand";

interface ProfileLoadState {
  /** 로드 실패 사유(백엔드 메시지). null이면 정상 — 저장 허용. */
  error: string | null;
  retrying: boolean;
}

export const useProfileLoadStore = create<ProfileLoadState>(() => ({
  error: null,
  retrying: false,
}));

/** 프로필 저장을 막아야 하는가(로드 실패 상태). */
export function profilesReadOnly(): boolean {
  return useProfileLoadStore.getState().error !== null;
}
