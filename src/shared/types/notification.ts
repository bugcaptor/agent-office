// src/shared/types/notification.ts
//
// Domain slice: notification pipeline (hook/stop/bell) and the activity
// time-tracking signal. See src/shared/types.ts for the frozen-contract overview.

import type { AgentId, SessionId } from './common';

/**
 * Notification source. Mirrors Rust `NotificationSource` (serde lowercase).
 */
export type NotificationSource = "hook" | "stop" | "bell";

/**
 * Renderer-facing notification display type, derived from `NotificationSource`
 * Never stored/transmitted directly — always computed via
 * `notificationType()`.
 */
export type NotificationType = "question" | "done" | "info";

/**
 * Derives the renderer display type from a notification's source:
 * hook -> "question", stop -> "done", bell -> "info".
 */
export function notificationType(source: NotificationSource): NotificationType {
  switch (source) {
    case "hook":
      return "question";
    case "stop":
      return "done";
    case "bell":
      return "info";
  }
}

/**
 * Normalized notification event. hook POST and BEL fallback both converge to
 * this shape. Event name: "notification-new". Mirrors Rust `NotificationEvent`.
 *
 * `id` is issued by the Rust `NotificationHub` (uuid v4) — the renderer
 * must never reissue/regenerate ids, so that `clearNotifications` stays in
 * sync with the backend.
 */
export interface NotificationEvent {
  id: string;
  sessionId: SessionId;
  agentId: AgentId;
  source: NotificationSource;
  message: string;
  dedupKey: string;
  at: number;
}

/**
 * Activity kind for the time-tracking pipeline. Mirrors Rust `ActivityKind`
 * (serde lowercase). `prompt` = UserPromptSubmit (turn start), `tool` =
 * PostToolUse (heartbeat / waiting→working signal). `resume` = the backend's
 * post-completion output heuristic (이슈 #39) deciding the agent is still
 * working after a Stop; the renderer treats it like `tool` for turn purposes.
 */
export type ActivityKind =
  | "prompt"
  | "tool"
  | "sub-start"
  | "sub-stop"
  | "sub-count"
  | "resume";

/**
 * Activity signal for session time tracking. Emitted as the `activity-event`
 * Tauri event, bypassing the notification dedup/queue entirely. Mirrors
 * Rust `ActivityEvent`. `at` is the backend `now_ms()`
 * epoch-ms timestamp — the renderer must settle turns from this, never from
 * its own clock.
 */
export interface ActivityEvent {
  agentId: AgentId;
  sessionId: SessionId;
  kind: ActivityKind;
  at: number;
  /** kind="prompt"일 때 사용자 프롬프트 원문(최대 2,000자 절단), kind="tool"일 때
   * 도구 요약("Bash: npm test" 등, 최대 60자). 파싱 실패/부재 시 undefined. */
  text?: string;
  /** kind="tool"일 때 턴 중간 assistant 내레이션(claude transcript 꼬리, 스로틀
   * 적용). 그 외 kind/codex/부재는 undefined. */
  assistantText?: string;
  /** kind="prompt"일 때 훅 body top-level cwd(세션 실제 작업 디렉터리, 라벨
   * 프로젝트명 표시용, 이슈 #44 작업 D). 그 외 kind/부재는 undefined. */
  cwd?: string;
  /** kind="sub-count"일 때 현재 실행 중 서브에이전트 절대 수. */
  count?: number;
}

/**
 * Notifications-cleared broadcast. Event name: "notification-cleared".
 * Mirrors Rust `NotificationClearedEvent`.
 */
export interface NotificationClearedEvent {
  agentId: AgentId;
  ids: string[];
}
