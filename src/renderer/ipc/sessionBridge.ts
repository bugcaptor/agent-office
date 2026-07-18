// src/renderer/ipc/sessionBridge.ts
//
// Wires the frozen `tauriApi` adapter to the zustand app store, and
// implements the `OfficeBus` contract (subsystem B <-> C, `office/bus.ts`)
// as a thin store-backed relay. Installed once at app boot, after
// `loadState` -> `hydrate`.
//
// `onData` is deliberately NOT wired here — each `TerminalMount` subscribes
// to it directly: it's a high-frequency stream and routing it through the
// store would cause a render storm.
//
// Deviations from the original design sketch (which predated `tauriApi` and
// used placeholder names):
// - `window.api` -> `tauriApi` (the sketch's `window.api` is what became the
//   `tauriApi` module).
// - `setSessionState` is given the wire `SessionStateEvent.state`
//   (`SessionState`) directly, with no cast: `SessionStatus` (shared/types)
//   is defined as exactly `SessionState | "idle"`, so every `SessionState`
//   value is already a valid `SessionStatus`. The sketch's `status: e.state
//   as any` was papering over a mismatch that doesn't actually exist once
//   the shared types are used consistently — the `as any` has been dropped.
// - `stateCbs`'s payload is typed `SessionState`, not `string`, matching the
//   frozen `OfficeBus.onSessionStateChanged` signature in `office/bus.ts`
//   (which subsystem B depends on).
//
// Badge/mute addition: a second subscription on `s.muted` resyncs the badge
// the instant the BottomBar mute toggle flips, instead of waiting for the
// next notification event.
import type { SessionState } from "@shared/types";
import type { LabelAnchor, OfficeBus } from "../office/bus";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "./tauriApi";
import { sessionOptsFor } from "./sessionOpts";
import { SubagentCountTracker } from "./subagentCounts";
import { maybeSendOsNotification } from "./osNotify";

/** OS 알림 본문 길이 상한(제목 옆 본문은 짧게). */
const OS_NOTIFY_BODY_MAX = 120;

type NotifCb = (agentId: string, hasPending: boolean) => void;
type StateCb = (agentId: string, state: SessionState) => void;
type HoverCb = (agentId: string | null, screenX: number, screenY: number) => void;
type LabelAnchorCb = (anchors: ReadonlyMap<string, LabelAnchor>) => void;
type DeskClickCb = (deskIndex: number, screenX: number, screenY: number) => void;

const notifCbs = new Set<NotifCb>();
const stateCbs = new Set<StateCb>();
const hoverCbs = new Set<HoverCb>();
const labelAnchorCbs = new Set<LabelAnchorCb>();
const deskClickCbs = new Set<DeskClickCb>();
const subagentCounts = new SubagentCountTracker();

// Agents with an in-flight createSession, so a double-click (two
// emitAgentClicked in a row) can only ever produce ONE createSession call.
const startingInFlight = new Set<string>();

/** createSession invoke가 settle되지 않을 때의 복구 한계선. 백엔드 커맨드가
 * 패닉하면 Tauri invoke 프라미스는 영원히 settle되지 않는다(2026-07-11
 * "터미널 영구 고착" 실사고) — 이 시간이 지나면 실패로 간주해 상태를
 * exited로 되돌리고 재시도를 가능하게 한다. */
export const CREATE_SESSION_TIMEOUT_MS = 15_000;

function withTimeout<T>(p: Promise<T>, ms: number, label: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const t = setTimeout(() => reject(new Error(`${label} timed out after ${ms}ms`)), ms);
    p.then(
      (v) => {
        clearTimeout(t);
        resolve(v);
      },
      (e) => {
        clearTimeout(t);
        reject(e);
      },
    );
  });
}

/**
 * createSession 호출 공통 가드 — ensureSession과 restartAgentSession이 공유.
 * 호출 전 상태를 "starting"으로 만든 뒤 불러야 한다.
 *
 * - 성공: 결과의 state를 반영한다(단, 스토어가 아직 "starting"일 때만 —
 *   invoke 응답보다 먼저 도착한 백엔드 상태 이벤트를 덮어쓰지 않는다).
 *   백엔드가 살아있는 세션을 재사용한 경우 상태 이벤트를 방출하지 않고
 *   결과만 돌려주므로, 이 반영이 없으면 "starting"에 영구 고착된다.
 * - 실패/타임아웃: exited로 되돌려 이후 클릭·재시작이 재시도할 수 있게 한다.
 *
 * `overrides.startupCommand`는 이번 1회 생성에만 프로필의 startupCommand를
 * 대체한다(Claude 세션 이어하기 — resumeAgentSession). 부재면 프로필 그대로.
 */
export async function runGuardedCreateSession(
  agentId: string,
  overrides?: { startupCommand?: string },
): Promise<void> {
  const agent = useAppStore.getState().agents[agentId];
  try {
    const res = await withTimeout(
      tauriApi.createSession(agentId, sessionOptsFor(agent, overrides)),
      CREATE_SESSION_TIMEOUT_MS,
      `createSession(${agentId})`,
    );
    const cur = useAppStore.getState().sessions[agentId]?.status;
    if (cur === "starting" && res?.state) {
      useAppStore.getState().setSessionState({
        agentId,
        status: res.state === "disposed" ? "exited" : res.state,
      });
    }
  } catch (err) {
    useAppStore.getState().setSessionState({ agentId, status: "exited" });
    console.warn(`createSession failed for ${agentId}`, err);
  }
}

/**
 * Ensures a live backend session exists for `agentId`. If the
 * store shows the agent as `idle`/`exited` (or has no session row yet), mark it
 * `starting` and start a PTY. Recreating a session for the same agentId reuses
 * the renderer's already-attached output Channel on the backend (see
 * SessionManager sink re-keying), so no re-subscribe is needed here.
 *
 * Idempotent per agent while a start is in flight; on failure/timeout the
 * status flips to `exited` so a later click retries.
 */
export function ensureSession(agentId: string): void {
  const { sessions, setSessionState } = useAppStore.getState();
  const status = sessions[agentId]?.status;
  const needsStart = status === undefined || status === "idle" || status === "exited";
  if (!needsStart || startingInFlight.has(agentId)) return;

  startingInFlight.add(agentId);
  setSessionState({ agentId, status: "starting" });
  void runGuardedCreateSession(agentId).finally(() => {
    startingInFlight.delete(agentId);
  });
}

/** Store-backed `OfficeBus` implementation injected into subsystem B (`<OfficeCanvas bus={officeBus} .../>`). */
export const officeBus: OfficeBus = {
  onNotificationChanged(cb) {
    notifCbs.add(cb);
    return () => notifCbs.delete(cb);
  },
  onSessionStateChanged(cb) {
    stateCbs.add(cb);
    return () => stateCbs.delete(cb);
  },
  onAgentHoverChanged(cb) {
    hoverCbs.add(cb);
    return () => hoverCbs.delete(cb);
  },
  emitAgentHoverChanged(agentId, screenX, screenY) {
    hoverCbs.forEach((cb) => cb(agentId, screenX, screenY));
  },
  emitLabelAnchorsChanged(anchors) {
    labelAnchorCbs.forEach((cb) => cb(anchors));
  },
  onLabelAnchorsChanged(cb) {
    labelAnchorCbs.add(cb);
    return () => labelAnchorCbs.delete(cb);
  },
  emitDeskClicked(deskIndex, screenX, screenY) {
    deskClickCbs.forEach((cb) => cb(deskIndex, screenX, screenY));
  },
  onDeskClicked(cb) {
    deskClickCbs.add(cb);
    return () => deskClickCbs.delete(cb);
  },
  onSubagentCountChanged(cb) {
    return subagentCounts.subscribe(cb);
  },
  emitAgentClicked(agentId) {
    // 클릭 시 호버 카드 즉시 숨김.
    hoverCbs.forEach((cb) => cb(null, 0, 0));
    // Recreate the session for persisted/exited agents before opening
    // the terminal, so clicking a character that has no live PTY restarts it.
    ensureSession(agentId);
    useAppStore.getState().openTerminal(agentId);
    tauriApi.clearNotifications(agentId);
  },
};

/**
 * Subscribes the store to `tauriApi`'s events and keeps the dock badge in
 * sync. Call once at app boot; returns an unsubscribe for symmetry /
 * test teardown (the app itself never tears this down during its lifetime).
 */
export function installSessionBridge(): () => void {
  const offState = tauriApi.onSessionState((e) => {
    // 스토어에는 disposed를 exited로 흡수한다(runGuardedCreateSession과 동일).
    // disposed가 스토어에 남으면 ensureSession(idle/exited만 재시작)과
    // TerminalHost의 exited 배너가 막혀 재소환이 안 된다. 시간 정산과
    // officeBus 릴레이에는 원본 e.state를 그대로 준다 — 둘 다 disposed를
    // 이미 종료/비활성으로 처리한다.
    useAppStore.getState().setSessionState({
      agentId: e.agentId,
      status: e.state === "disposed" ? "exited" : e.state,
    });
    // 시간 추적: 세션 종료(exited/disposed) 시 열린 턴 강제 정산(백엔드 e.at 사용).
    useAppStore.getState().applySessionTiming(e.agentId, e.state, e.at);
    stateCbs.forEach((cb) => cb(e.agentId, e.state));
    if (e.state === "exited" || e.state === "disposed") subagentCounts.reset(e.agentId);
  });

  const offNotif = tauriApi.onNotification((e) => {
    const store = useAppStore.getState();
    store.pushNotification(e);
    // 시간 추적은 pushNotification 억제와 무관하게 항상 공급(활성 터미널이어도 집계).
    // Stop 카운트 reset 안전망은 백엔드 Stop→sub-count(0 fallback)로 이동했다.
    store.applyNotificationTiming(e);
    // 이슈 #39: 창이 비포커스일 때만 OS 데스크탑 알림 발송(터미널이 열려 있어도).
    // 제목=에이전트 이름/ID, 본문=메시지 excerpt.
    if (!store.windowFocused) {
      const agent = store.agents[e.agentId];
      const title = agent?.name ?? e.agentId;
      const body =
        e.message.length > OS_NOTIFY_BODY_MAX
          ? e.message.slice(0, OS_NOTIFY_BODY_MAX - 1) + "…"
          : e.message;
      void maybeSendOsNotification(title, body);
    }
  });

  const offCleared = tauriApi.onNotificationCleared(({ agentId, ids }) => {
    useAppStore.getState().clearNotificationByIds(agentId, ids);
  });

  const offActivity = tauriApi.onActivity((e) => {
    if (e.kind === "sub-start") {
      subagentCounts.bump(e.agentId, +1, e.at);
      return;
    }
    if (e.kind === "sub-stop") {
      subagentCounts.bump(e.agentId, -1, e.at);
      return;
    }
    if (e.kind === "sub-count") {
      subagentCounts.setAbsolute(e.agentId, e.count ?? 0, e.at);
      return;
    }
    useAppStore.getState().applyActivityEvent(e);
  });

  // notifications changed -> relay hasPending per known agent + sync the dock badge
  // (badge count = number of distinct agents with an unread notification;
  // this only respects the mute flag, it doesn't own the toggle).
  const offPending = useAppStore.subscribe(
    (s) => s.notifications,
    (notifications) => {
      const pending = new Set(notifications.map((n) => n.agentId));
      const { agents, muted } = useAppStore.getState();
      for (const id of Object.keys(agents)) {
        notifCbs.forEach((cb) => cb(id, pending.has(id)));
      }
      if (!muted) tauriApi.setBadgeCount(pending.size);
    }
  );

  // Toggling `muted` itself (BottomBar) must resync the
  // badge immediately rather than waiting for the next notification: mute ->
  // force the dock badge to 0; unmute -> resync to the current pending count
  // (which may have drifted from the badge while muted).
  const offMuted = useAppStore.subscribe(
    (s) => s.muted,
    (muted) => {
      const pending = new Set(useAppStore.getState().notifications.map((n) => n.agentId));
      tauriApi.setBadgeCount(muted ? 0 : pending.size);
    }
  );

  return () => {
    offState();
    offNotif();
    offCleared();
    offActivity();
    offPending();
    offMuted();
  };
}
