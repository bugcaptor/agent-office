import { useAppStore } from "./store/appStore";
import { applyLanguageSetting } from "./i18n";
import { installSessionBridge } from "./ipc/sessionBridge";
import { installSoundManager } from "./sound/soundManager";
import { installPersistence } from "./store/persist";
import { agentsNeedingPortraits, loadPortraitsFor } from "./portrait/portraitCache";
import { setRemoteTerminalViewModeStorage } from "./terminal/terminalViewMode";
import type { RemoteSnapshot } from "./ipc/remoteApi";

/** Remote windows hydrate exclusively from the host snapshot. */
export async function bootRemoteApp(snapshot: RemoteSnapshot): Promise<() => void> {
  setRemoteTerminalViewModeStorage(true);
  useAppStore.getState().hydrate(snapshot.state);
  useAppStore.getState().hydrateSettings(snapshot.settings, false);
  applyLanguageSetting(snapshot.settings.language, false);
  for (const session of snapshot.sessions) {
    useAppStore.getState().setSessionState({ agentId: session.agentId, status: session.state === "exited" ? "exited" : session.state === "disposed" ? "idle" : "running" });
    useAppStore.getState().setSessionSize(session.agentId, session.cols, session.rows);
  }
  // Read-only media cache: unlike installPortraitCache this deliberately does
  // not install the local deletion bridge.
  void loadPortraitsFor(agentsNeedingPortraits(useAppStore.getState().agents));
  const offBridge = installSessionBridge();
  // Same selector/debounce as local, but tauriApi now points at office.saveState
  // on the host. It is needed for creating the first remote profile and never
  // writes a local profile file.
  const offHostStateSave = installPersistence();
  const offSound = installSoundManager();
  return () => { offBridge(); offHostStateSave(); offSound(); setRemoteTerminalViewModeStorage(false); };
}
