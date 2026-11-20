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

/** 화면좌표 사각형(px). 회의록 보드가 회의실 옆에 붙을 때 쓴다. */
export interface ScreenRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * "이 달의 우수사원" 확정 수상자를
 * 씬에 전달하는 최소 정보. 스냅샷 전체가 아니라 씬이 그리는 데 필요한 값만 —
 * 실제 초상 픽셀은 별도로 `tauriApi.loadAwardPortrait(month)`가 담당한다.
 */
export interface OfficeAwardee {
  agentId: string;
  /** 확정 시점 이름(스냅샷) — 씬은 표시에만 쓰고 프로필 이름과 동기화하지 않는다. */
  name: string;
  /** 확정된 달 "YYYY-MM" — 초상 로드 키. */
  month: string;
  /** 그 달 초상 스냅샷이 존재하는지(false/로드 실패 시 씬이 실루엣으로 대체). */
  hasPortrait: boolean;
}

/** 진행 중인 토의 참가자 한 명(씬 연출용). */
export interface OfficeDiscussionMember {
  agentId: string;
  moderator: boolean;
  /** 지금 차례를 받아 말하는 중(차례를 보냈고 발언을 아직 받지 않음). */
  speaking: boolean;
}

/**
 * 진행 중인 토의(동시에 하나뿐)를 씬에 전하는 최소 정보. `members` 순서가 회의실 의자 순서다 —
 * 0번 = 사회자, 이어서 토론자. 끝났거나(결론 확인·끝남) 시작 전이면 null.
 */
export interface OfficeDiscussionStage {
  discussionId: string;
  members: readonly OfficeDiscussionMember[];
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
  /** 공용 서버 랙 클릭 — 저장소 점검 오버레이를 연다. */
  emitRepositoryServerClicked(): void;
  /** B가 휴가 모드 on/off를 구독 — on이면 줄 전원 이탈.
   * 구독 즉시 현재값을 1회 재생(replay)한다 — 늦게 마운트된 씬도 초기값 수신. */
  onVacationModeChanged(cb: (on: boolean) => void): () => void;
  /** B가 "이 달의 우수사원" 확정 수상자를 구독 — 자격 수상자가 없으면 null.
   * 구독 즉시 현재값을 1회 재생(replay)한다 — 늦게 마운트된 씬도 초기값 수신. */
  onAwardeeChanged(cb: (awardee: OfficeAwardee | null) => void): () => void;
  /** B가 진행 중인 토의를 구독 — 회의실 이동·발언 말풍선·사회자 표시. 없으면 null.
   * 구독 즉시 현재값을 1회 재생(replay)한다 — 늦게 마운트된 씬도 초기값 수신. */
  onDiscussionStageChanged(cb: (stage: OfficeDiscussionStage | null) => void): () => void;
  /** B가 회의실의 화면 사각형을 알림(카메라·풍경이 바뀔 때만). 회의실이 없는 풍경이면 null. */
  emitMeetingRoomRectChanged(rect: ScreenRect | null): void;
  /** C(회의록 보드)가 회의실 사각형을 구독. 구독 즉시 마지막 값을 1회 재생(replay)한다. */
  onMeetingRoomRectChanged(cb: (rect: ScreenRect | null) => void): () => void;
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
  readonly repositoryServerClickCount: number;
  /** Drives the C -> B direction for the current awardee from a test/manual harness. */
  triggerAwardeeChanged(awardee: OfficeAwardee | null): void;
  /** Drives the C -> B direction for the running discussion from a test/manual harness. */
  triggerDiscussionStageChanged(stage: OfficeDiscussionStage | null): void;
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
  const awardeeListeners = new Set<(a: OfficeAwardee | null) => void>();
  const stageListeners = new Set<(s: OfficeDiscussionStage | null) => void>();
  let stage: OfficeDiscussionStage | null = null;
  const roomRectListeners = new Set<(r: ScreenRect | null) => void>();
  let roomRect: ScreenRect | null = null;
  const clickedAgentIds: string[] = [];
  let bossDeskClickCount = 0;
  let repositoryServerClickCount = 0;
  let vacationMode = false;
  let awardee: OfficeAwardee | null = null;
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
    emitRepositoryServerClicked() { repositoryServerClickCount += 1; },
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
    get repositoryServerClickCount() { return repositoryServerClickCount; },
    onAwardeeChanged(cb) {
      awardeeListeners.add(cb);
      cb(awardee);
      return () => awardeeListeners.delete(cb);
    },
    triggerAwardeeChanged(next) {
      awardee = next;
      for (const cb of awardeeListeners) cb(next);
    },
    onDiscussionStageChanged(cb) {
      stageListeners.add(cb);
      cb(stage);
      return () => stageListeners.delete(cb);
    },
    triggerDiscussionStageChanged(next) {
      stage = next;
      for (const cb of stageListeners) cb(next);
    },
    emitMeetingRoomRectChanged(rect) {
      roomRect = rect;
      for (const cb of roomRectListeners) cb(rect);
    },
    onMeetingRoomRectChanged(cb) {
      roomRectListeners.add(cb);
      cb(roomRect);
      return () => roomRectListeners.delete(cb);
    },
  };
}
