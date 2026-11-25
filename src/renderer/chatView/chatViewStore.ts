// src/renderer/chatView/chatViewStore.ts
//
// 탭마다 "터미널 보기 ↔ 대화 보기" 중 무엇을 보이는가. CLI는 어느 보기든 PTY에서 그대로 돈다.
// 고른 보기는 탭(캐릭터) id별로 localStorage에 남긴다.
import { create } from "zustand";

export const CHAT_VIEW_STORAGE_KEY = "agent-office.chat-view-tabs";

function loadStored(): Record<string, boolean> {
  try {
    const raw = JSON.parse(localStorage.getItem(CHAT_VIEW_STORAGE_KEY) ?? "{}");
    if (!raw || typeof raw !== "object") return {};
    return Object.fromEntries(Object.entries(raw).filter(([, v]) => v === true)) as Record<string, boolean>;
  } catch {
    return {};
  }
}

function persist(on: Record<string, boolean>): void {
  try {
    localStorage.setItem(CHAT_VIEW_STORAGE_KEY, JSON.stringify(on));
  } catch {
    // 저장 불가 환경 — 보기 전환 자체는 유효하다.
  }
}

interface ChatViewState {
  /** 대화 보기를 켠 탭. */
  on: Record<string, boolean>;
  toggle: (agentId: string) => void;
  set: (agentId: string, value: boolean) => void;
}

export const useChatViewStore = create<ChatViewState>((set, get) => ({
  on: loadStored(),
  toggle: (agentId) => get().set(agentId, !get().on[agentId]),
  set: (agentId, value) => {
    const on = { ...get().on };
    if (value) on[agentId] = true;
    else delete on[agentId];
    persist(on);
    set({ on });
  },
}));
