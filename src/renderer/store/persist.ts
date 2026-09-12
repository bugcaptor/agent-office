// src/renderer/store/persist.ts
//
// Persistence wiring: a debounced
// `tauriApi.saveState` triggered by agent-profile and vacation-mode changes.
//
// Subscribes on the selectors `(s) => s.agents` and `(s) => s.vacationMode`
// specifically (not the whole store) so that high-frequency, purely-runtime
// state changes — incoming PTY output driving `sessions`/`lastActivityAt`,
// notifications arriving/clearing, terminal tab switches, mute toggling —
// never trigger a save. Saving on every unrelated store change would just be
// wasted IPC calls with an unchanged payload. zustand's `subscribeWithSelector`
// already skips the listener when the selected value is referentially
// unchanged, so this falls out for free — the agents listener only fires when
// `addAgent`/`updateAgent`/`removeAgent`/`hydrate` produce a new `agents`
// object, and the vacationMode listener only fires when `toggleVacationMode`/
// `hydrate` flip it. Both share one debounce timer (`queueSave`) so a burst
// touching both within the window still collapses into a single saveState call.
import { useAppStore } from "./appStore";
import { tauriApi } from "../ipc/tauriApi";
import type { AgentProfile, PersistedState } from "./types";

const DEBOUNCE_MS = 500;
let flushPendingPersistence: (() => Promise<void>) | null = null;

function currentState(): PersistedState {
  const { agents, agentOrder, vacationMode } = useAppStore.getState();
  return {
    agents: agentOrder.map((id) => agents[id]).filter((a): a is AgentProfile => a != null),
    version: 1,
    vacationMode,
  };
}

/** Writes the current profile snapshot now, cancelling the normal debounce. */
export function flushPersistence(): Promise<void> {
  return flushPendingPersistence?.() ?? tauriApi.saveState(currentState());
}

/**
 * Installs the debounced save. Call once at app boot, after `hydrate()` has
 * already applied the loaded state (so the initial hydrate doesn't itself
 * queue a redundant save). Returns an unsubscribe that also cancels any
 * still-pending debounced save.
 */
export function installPersistence(): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let saveChain: Promise<void> = Promise.resolve();
  let saveInFlight = false;
  const writeCurrent = (): Promise<void> => {
    const snapshot = currentState();
    // A slow older debounce write must finish before the newer connection
    // snapshot writes, otherwise it can overwrite the profile cwd the backend
    // validates during connect_ide_session.
    if (!saveInFlight) {
      const result = tauriApi.saveState(snapshot);
      saveChain = Promise.resolve(result);
      // Test doubles may be synchronous; in that case there is nothing that
      // can overtake a later write, so keep the legacy immediate-call timing.
      if (!result || typeof (result as Promise<void>).then !== "function") return saveChain;
      saveInFlight = true;
    } else {
      saveChain = saveChain.catch(() => {}).then(() => tauriApi.saveState(snapshot));
    }
    const thisWrite = saveChain;
    const settle = () => {
      if (saveChain === thisWrite) saveInFlight = false;
    };
    // Do not leave a rejected promise from `finally` unobserved; callers own
    // the write error while this handler only releases the serialization gate.
    void thisWrite.then(settle, settle);
    return saveChain;
  };

  const queueSave = () => {
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      void writeCurrent().catch((error) => console.warn("profile save failed", error));
    }, DEBOUNCE_MS);
  };

  const unsubscribeAgents = useAppStore.subscribe((s) => s.agents, queueSave);
  const unsubscribeVacation = useAppStore.subscribe((s) => s.vacationMode, queueSave);
  flushPendingPersistence = async () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    await writeCurrent();
  };

  return () => {
    unsubscribeAgents();
    unsubscribeVacation();
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    if (flushPendingPersistence) flushPendingPersistence = null;
  };
}
