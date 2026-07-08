// src/renderer/store/persist.ts
//
// Persistence wiring: a debounced
// `tauriApi.saveState` triggered by agent-profile changes only.
//
// Subscribes on the selector `(s) => s.agents` specifically (not the whole
// store) so that high-frequency, purely-runtime state changes — incoming
// PTY output driving `sessions`/`lastActivityAt`, notifications arriving/
// clearing, terminal tab switches, mute toggling — never trigger a save.
// `PersistedState` (shared/types) only has room for `agents` anyway; saving
// on every unrelated store change would just be wasted IPC calls with an
// unchanged payload. zustand's `subscribeWithSelector` already skips the
// listener when the selected value is referentially unchanged, so this falls
// out for free — the listener only fires when `addAgent`/`updateAgent`/
// `removeAgent`/`hydrate` produce a new `agents` object.
import { useAppStore } from "./appStore";
import { tauriApi } from "../ipc/tauriApi";
import type { AgentProfile, PersistedState } from "./types";

const DEBOUNCE_MS = 500;

/**
 * Installs the debounced save. Call once at app boot, after `hydrate()` has
 * already applied the loaded state (so the initial hydrate doesn't itself
 * queue a redundant save). Returns an unsubscribe that also cancels any
 * still-pending debounced save.
 */
export function installPersistence(): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;

  const unsubscribe = useAppStore.subscribe(
    (s) => s.agents,
    () => {
      if (timer !== null) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = null;
        const { agents, agentOrder } = useAppStore.getState();
        const state: PersistedState = {
          agents: agentOrder
            .map((id) => agents[id])
            .filter((a): a is AgentProfile => a != null),
          version: 1,
        };
        void tauriApi.saveState(state);
      }, DEBOUNCE_MS);
    }
  );

  return () => {
    unsubscribe();
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  };
}
