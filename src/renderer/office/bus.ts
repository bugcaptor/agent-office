// src/renderer/office/bus.ts
//
// Renderer-internal event bus contract between subsystem B (this office
// scene) and A (main/IPC, via C) / C (React shell).
//
// The interface is what subsystem B depends on; the concrete
// zustand-store-backed implementation is subsystem C's responsibility and is
// injected via `<OfficeCanvas bus={...} .../>`. `createMockOfficeBus` below
// is a dependency-free stand-in used by this task's own tests and by manual
// skeleton verification before C wires the real one.

import type { SessionState } from "../../shared/types";

/** 머리 위 라벨 앵커(화면좌표 px). TaskLabelLayer가 소비. */
export interface LabelAnchor {
  x: number;
  y: number;
}

export interface OfficeBus {
  // B subscribes (A -> B, relayed through C). 구독 즉시 현재 상태를 1회
  // replay한다(기본값 상태 false/idle은 생략) — 재마운트된 씬도 상태를 받는다.
  onNotificationChanged(cb: (agentId: string, hasPending: boolean) => void): () => void;
  onSessionStateChanged(cb: (agentId: string, state: SessionState) => void): () => void;
  // B emits (B -> C/A).
  emitAgentClicked(agentId: string): void;
  /** B가 캐릭터 hover in/out을 알림(호버 카드용). agentId=null 이면 hover 해제. */
  emitAgentHoverChanged(agentId: string | null, screenX: number, screenY: number): void;
  /** C가 hover 변화를 구독. */
  onAgentHoverChanged(
    cb: (agentId: string | null, screenX: number, screenY: number) => void
  ): () => void;
  /** B가 매 프레임 캐릭터 머리 위 화면좌표를 발행(라벨 레이어용). Map은 재사용될 수 있으므로 보관하려면 복사할 것. */
  emitLabelAnchorsChanged(anchors: ReadonlyMap<string, LabelAnchor>): void;
  /** C가 라벨 앵커를 구독. */
  onLabelAnchorsChanged(cb: (anchors: ReadonlyMap<string, LabelAnchor>) => void): () => void;
  /** B가 책상(슬롯) 클릭을 알림 — 책상 주인 지정 메뉴용. 좌표는 화면 px. */
  emitDeskClicked(deskIndex: number, screenX: number, screenY: number): void;
  /** C가 책상 클릭을 구독. */
  onDeskClicked(cb: (deskIndex: number, screenX: number, screenY: number) => void): () => void;
  /** B가 부모별 활성 서브에이전트 수 변화를 구독(미니 캐릭터 표시용). */
  onSubagentCountChanged(cb: (agentId: string, count: number) => void): () => void;
  /** B가 보스 책상 클릭을 알림(책상 주인 지정과 별개 — 보스 자리 클릭 전용 채널). */
  emitBossDeskClicked(): void;
  /** B가 휴가 모드 on/off를 구독 — on이면 줄 전원 이탈.
   * 구독 즉시 현재값을 1회 재생(replay)한다 — 늦게 마운트된 씬도 초기값 수신. */
  onVacationModeChanged(cb: (on: boolean) => void): () => void;
}

type NotificationListener = (agentId: string, hasPending: boolean) => void;
type SessionStateListener = (agentId: string, state: SessionState) => void;

/** Test/manual-verification-only extensions beyond the frozen `OfficeBus` contract. */
export interface MockOfficeBus extends OfficeBus {
  /** Drives the A -> B direction from a test or manual harness. */
  triggerNotificationChanged(agentId: string, hasPending: boolean): void;
  triggerSessionStateChanged(agentId: string, state: SessionState): void;
  /** Drives subagent-count changes from a test/manual harness. */
  triggerSubagentCountChanged(agentId: string, count: number): void;
  /** Records every agentId passed to `emitAgentClicked` (B -> A/C direction), in order. */
  readonly clickedAgentIds: readonly string[];
  /** Drives the C -> B direction for vacation mode from a test/manual harness. */
  triggerVacationModeChanged(on: boolean): void;
  /** Counts every `emitBossDeskClicked` call (B -> A/C direction). */
  readonly bossDeskClickCount: number;
}

/** In-memory pub/sub `OfficeBus` implementation. No Pixi/DOM/IPC dependency. */
export function createMockOfficeBus(): MockOfficeBus {
  const notificationListeners = new Set<NotificationListener>();
  const sessionStateListeners = new Set<SessionStateListener>();
  const hoverListeners = new Set<
    (agentId: string | null, x: number, y: number) => void
  >();
  const labelAnchorListeners = new Set<(a: ReadonlyMap<string, LabelAnchor>) => void>();
  const deskClickListeners = new Set<(deskIndex: number, x: number, y: number) => void>();
  const subagentCountListeners = new Set<(agentId: string, count: number) => void>();
  const vacationModeListeners = new Set<(on: boolean) => void>();
  const clickedAgentIds: string[] = [];
  let bossDeskClickCount = 0;
  let vacationMode = false;
  // 구독 시점 replay용 마지막 상태.
  const lastPending = new Map<string, boolean>();
  const lastSessionState = new Map<string, SessionState>();

  return {
    onNotificationChanged(cb) {
      notificationListeners.add(cb);
      for (const [id, p] of lastPending) if (p) cb(id, true);
      return () => notificationListeners.delete(cb);
    },
    onSessionStateChanged(cb) {
      sessionStateListeners.add(cb);
      for (const [id, st] of lastSessionState) cb(id, st);
      return () => sessionStateListeners.delete(cb);
    },
    emitAgentClicked(agentId) {
      clickedAgentIds.push(agentId);
    },
    emitAgentHoverChanged(agentId, screenX, screenY) {
      for (const cb of hoverListeners) cb(agentId, screenX, screenY);
    },
    onAgentHoverChanged(cb) {
      hoverListeners.add(cb);
      return () => hoverListeners.delete(cb);
    },
    emitLabelAnchorsChanged(anchors) {
      for (const cb of labelAnchorListeners) cb(anchors);
    },
    onLabelAnchorsChanged(cb) {
      labelAnchorListeners.add(cb);
      return () => labelAnchorListeners.delete(cb);
    },
    emitDeskClicked(deskIndex, screenX, screenY) {
      for (const cb of deskClickListeners) cb(deskIndex, screenX, screenY);
    },
    onDeskClicked(cb) {
      deskClickListeners.add(cb);
      return () => deskClickListeners.delete(cb);
    },
    onSubagentCountChanged(cb) {
      subagentCountListeners.add(cb);
      return () => subagentCountListeners.delete(cb);
    },
    triggerSubagentCountChanged(agentId, count) {
      for (const cb of subagentCountListeners) cb(agentId, count);
    },
    triggerNotificationChanged(agentId, hasPending) {
      lastPending.set(agentId, hasPending);
      for (const cb of notificationListeners) cb(agentId, hasPending);
    },
    triggerSessionStateChanged(agentId, state) {
      lastSessionState.set(agentId, state);
      for (const cb of sessionStateListeners) cb(agentId, state);
    },
    clickedAgentIds,
    emitBossDeskClicked() {
      bossDeskClickCount += 1;
    },
    onVacationModeChanged(cb) {
      vacationModeListeners.add(cb);
      cb(vacationMode);
      return () => vacationModeListeners.delete(cb);
    },
    triggerVacationModeChanged(on) {
      vacationMode = on;
      for (const cb of vacationModeListeners) cb(on);
    },
    get bossDeskClickCount() {
      return bossDeskClickCount;
    },
  };
}
