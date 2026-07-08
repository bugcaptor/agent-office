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

type NotifCb = (agentId: string, hasPending: boolean) => void;
type StateCb = (agentId: string, state: SessionState) => void;
type HoverCb = (agentId: string | null, screenX: number, screenY: number) => void;
type LabelAnchorCb = (anchors: ReadonlyMap<string, LabelAnchor>) => void;

const notifCbs = new Set<NotifCb>();
const stateCbs = new Set<StateCb>();
const hoverCbs = new Set<HoverCb>();
const labelAnchorCbs = new Set<LabelAnchorCb>();

// Agents with an in-flight createSession, so a double-click (two
// emitAgentClicked in a row) can only ever produce ONE createSession call.
const startingInFlight = new Set<string>();

/**
 * Ensures a live backend session exists for `agentId`. If the
 * store shows the agent as `idle`/`exited` (or has no session row yet), mark it
 * `starting` and start a PTY. Recreating a session for the same agentId reuses
 * the renderer's already-attached output Channel on the backend (see
 * SessionManager sink re-keying), so no re-subscribe is needed here.
 *
 * Idempotent per agent while a start is in flight; on failure the status flips
 * to `exited` so a later click retries.
 */
export function ensureSession(agentId: string): void {
  const { sessions, agents, setSessionState } = useAppStore.getState();
  const status = sessions[agentId]?.status;
  const needsStart = status === undefined || status === "idle" || status === "exited";
  if (!needsStart || startingInFlight.has(agentId)) return;

  const cwd = agents[agentId]?.cwd;
  startingInFlight.add(agentId);
  setSessionState({ agentId, status: "starting" });
  tauriApi
    .createSession(agentId, cwd ? { cwd } : undefined)
    .catch((err) => {
      useAppStore.getState().setSessionState({ agentId, status: "exited" });
      console.warn(`ensureSession: createSession failed for ${agentId}`, err);
    })
    .finally(() => {
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
    useAppStore.getState().setSessionState({ agentId: e.agentId, status: e.state });
    // 시간 추적: 세션 종료(exited/disposed) 시 열린 턴 강제 정산(백엔드 e.at 사용).
    useAppStore.getState().applySessionTiming(e.agentId, e.state, e.at);
    stateCbs.forEach((cb) => cb(e.agentId, e.state));
  });

  const offNotif = tauriApi.onNotification((e) => {
    useAppStore.getState().pushNotification(e);
    // 시간 추적은 pushNotification 억제와 무관하게 항상 공급(활성 터미널이어도 집계).
    useAppStore.getState().applyNotificationTiming(e);
  });

  const offCleared = tauriApi.onNotificationCleared(({ agentId, ids }) => {
    useAppStore.getState().clearNotificationByIds(agentId, ids);
  });

  const offActivity = tauriApi.onActivity((e) => {
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
