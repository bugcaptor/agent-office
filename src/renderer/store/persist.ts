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
import { isRemoteAgentId } from "../ipc/peerApi";
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

  const queueSave = () => {
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      const { agents, agentOrder, vacationMode } = useAppStore.getState();
      const state: PersistedState = {
        // 피어 세션 공유(#7k §결정 3): 원격 캐릭터(`peer:` 키)는 오피스에 서 있을
        // 뿐 이 앱의 프로필이 아니다 — 저장하면 호스트가 꺼진 뒤에도 유령 캐릭터로
        // 남고, 프로필 소유권(호스트 단독)도 깨진다. 저장 직전에 걸러낸다.
        agents: agentOrder
          .filter((id) => !isRemoteAgentId(id))
          .map((id) => agents[id])
          .filter((a): a is AgentProfile => a != null),
        version: 1,
        vacationMode,
      };
      void tauriApi.saveState(state);
    }, DEBOUNCE_MS);
  };

  const unsubscribeAgents = useAppStore.subscribe((s) => s.agents, queueSave);
  const unsubscribeVacation = useAppStore.subscribe((s) => s.vacationMode, queueSave);

  return () => {
    unsubscribeAgents();
    unsubscribeVacation();
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  };
}
