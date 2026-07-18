// src/renderer/store/types.ts
//
// Store-local type contract. `AgentProfile`, `PersistedState`, `SessionStatus`,
// `NotificationEvent`/`NotificationType` and `notificationType()` already
// have a single frozen definition in `src/shared/types.ts` (owned by
// subsystem A). The original design separately re-declared narrower copies
// of all of these — that would let the store's notion of e.g. `AgentProfile`
// silently drift from the wire contract. This file imports the shared
// definitions instead of redefining them, and only adds the types that are
// genuinely store-local.
import type {
  AgentProfile,
  NotificationEvent,
  NotificationType,
  PersistedState,
  SessionStatus,
} from "@shared/types";
import { notificationType } from "@shared/types";

export type { AgentProfile, NotificationEvent, PersistedState, SessionStatus };
export { notificationType };

/**
 * Store-local per-agent session bookkeeping. Not part of the renderer<->
 * backend wire contract (`cols`/`rows`/`lastActivityAt` are UI/store-only
 * concerns) — deliberately NOT named `SessionState` to avoid colliding with
 * the shared `SessionState` union (the backend lifecycle state type used to
 * build `SessionStatus`).
 */
export interface SessionRuntime {
  agentId: string;
  status: SessionStatus;
  cols: number;
  rows: number;
  lastActivityAt: number;
}

/**
 * Store-facing notification: derived from a backend `NotificationEvent`
 * via `notificationType()`, with a display-ready truncated `excerpt`.
 */
export interface Notification {
  /** Backend-issued id — reused as-is, never regenerated. */
  id: string;
  agentId: string;
  type: NotificationType;
  /** Original, untruncated message. */
  message: string;
  /** Display-ready truncation (<=80 chars). */
  excerpt: string;
  createdAt: number;
}

export type ModalState =
  | { kind: "none" }
  | { kind: "profile-create" }
  | { kind: "profile-edit"; agentId: string }
  | { kind: "confirm-delete"; agentId: string }
  | { kind: "confirm-restart"; agentId: string }
  | { kind: "confirm-resume"; agentId: string; sessionId: string }
  | { kind: "confirm-terminate"; agentId: string }
  | { kind: "confirm-clock-out"; agentId: string }
  | { kind: "confirm-clock-out-all" }
  | { kind: "confirm-quit" }
  | { kind: "settings" }
  | { kind: "analytics" }
  | { kind: "usage" }
  | { kind: "about" };

/**
 * 머리 위 작업 라벨의 에이전트별 소스 상태. 비영속(런타임 전용).
 * `sessionId`가 이벤트와 다르면 세션 재시작으로 보고 전체 리셋한다.
 */
export interface AgentTaskLabel {
  sessionId: string;
  /** 세션 첫 프롬프트 원문 — 목표(goal) 생성 소스. */
  firstPromptText?: string;
  /** 최신 프롬프트 원문 — 현재 명령 요약 소스이자 폴백 표시. */
  latestPromptText?: string;
  latestPromptAt?: number;
  /** LLM 생성 목표(세션당 1회). */
  goal?: string;
  /** LLM 생성 현재 명령 요약. 새 프롬프트가 오면 무효화(undefined). */
  currentSummary?: string;
  /** 턴 중 최신 도구 요약("Bash: npm test"). stop/새 프롬프트에 리셋. */
  latestToolText?: string;
  /** 턴 중 assistant 내레이션(claude transcript 꼬리). stop/새 프롬프트에 리셋. */
  latestAssistantText?: string;
  /** 도구 요약 마지막 반영 시각(스로틀 기준, 백엔드 epoch ms). */
  latestToolAt?: number;
}
