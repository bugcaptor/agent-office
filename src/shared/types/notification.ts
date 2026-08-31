// src/shared/types/notification.ts
//
// Domain slice: notification pipeline (hook/stop/bell) and the activity
// time-tracking signal. See src/shared/types.ts for the frozen-contract overview.

import type { AgentId, SessionId } from './common';
import type { SessionEventTokens } from './session';

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
 * 한 턴의 토큰 사용량. 이벤트명 "turn-usage". Rust `TurnUsageEvent` 미러.
 * 알림(notification-new)과 독립이라 서브에이전트로 억제된 Stop에서도 온다
 * (docs/session-analytics-design.md §9.1).
 *
 * `partial`: 턴이 끝나기 전의 중간 관측(true, claude 어댑터의 PostToolUse
 * 스로틀 통과 시)인지 턴이 실제로 끝난 것(false, Stop)인지(§11.9). 소비자
 * (`sessionCost.addTurn`)는 partial인 이벤트로 턴 수를 올리면 안 된다 — 한
 * 턴 안에서 도구를 여러 번 부르면 그만큼 partial 이벤트가 여러 번 온다.
 */
export interface TurnUsageEvent {
  agentId: AgentId;
  sessionId: SessionId;
  at: number;
  tokens: SessionEventTokens;
  partial: boolean;
}

/**
 * Activity kind for the time-tracking pipeline. Mirrors Rust `ActivityKind`
 * (serde lowercase). `prompt` = UserPromptSubmit (turn start), `tool` =
 * PostToolUse (heartbeat / waiting→working signal). `resume` = the backend's
 * post-completion output heuristic (이슈 #39) deciding the agent is still
 * working after a Stop; the renderer treats it like `tool` for turn purposes.
 * `idle` = 열린 턴을 알림 없이 정산하는 신호(kbm #2f9, 셸 포그라운드 명령
 * 종료). 셸 명령마다 완료 알림이 쌓이지 않도록 정산만 하는 갈래다.
 */
export type ActivityKind =
  | "prompt"
  | "tool"
  | "sub-start"
  | "sub-stop"
  | "sub-count"
  | "resume"
  | "idle";

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
