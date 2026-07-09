// src/renderer/quitGuard.ts
//
// Gates app quit in the FRONTEND: intercepts the window's `CloseRequested`
// event (user clicks the OS close button / Alt+F4) and, if any agent is still
// on duty (has NOT clocked out), blocks the close and asks for confirmation
// via the `confirm-quit` modal instead. Rust's `ExitRequested` cleanup
// (dispose_all) is untouched — a confirmed quit calls
// `getCurrentWindow().destroy()`, which does NOT re-emit `CloseRequested` (so
// no re-entrancy guard is needed here) but still triggers the backend's
// `ExitRequested` handler.
//
// "On duty" mirrors the office canvas / `useLightsOff` signal: an agent that
// is present but not `clockedOut`. An empty office (no agents) or one where
// everyone has clocked out closes without a prompt — "다 퇴근했으면 그냥 닫는다".
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useAppStore } from "./store/appStore";

/** Registers the app-quit confirmation gate. Returns an unlisten fn. */
export function installQuitGuard(): () => void {
  const unlistenP = getCurrentWindow().onCloseRequested((event) => {
    const { agents, agentOrder, openModal } = useAppStore.getState();
    const anyOnDuty = agentOrder.some((id) => agents[id] && !agents[id].clockedOut);
    if (!anyOnDuty) return;
    event.preventDefault();
    openModal({ kind: "confirm-quit" });
  });
  return () => {
    void unlistenP.then((un) => un());
  };
}
