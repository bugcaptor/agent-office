// src/renderer/quitGuard.ts
//
// Gates app quit in the FRONTEND: intercepts the window's `CloseRequested`
// event (user clicks the OS close button / Alt+F4) and, if any agent has an
// open turn, blocks the close and asks for confirmation via the
// `confirm-quit` modal instead. Rust's `ExitRequested` cleanup (dispose_all)
// is untouched — a confirmed quit calls `getCurrentWindow().destroy()`,
// which does NOT re-emit `CloseRequested` (so no re-entrancy guard is
// needed here) but still triggers the backend's `ExitRequested` handler.
//
// "In progress" uses the same signal as SessionTimePanel's `anyOpen`: any
// agent whose turn `phase !== "idle"` (NOT `session.status` — a bare idle
// shell shouldn't block quit).
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useAppStore } from "./store/appStore";

/** Registers the app-quit confirmation gate. Returns an unlisten fn. */
export function installQuitGuard(): () => void {
  const unlistenP = getCurrentWindow().onCloseRequested((event) => {
    const { timeTracking, openModal } = useAppStore.getState();
    const anyOpen = Object.values(timeTracking).some((t) => t.phase !== "idle");
    if (!anyOpen) return;
    event.preventDefault();
    openModal({ kind: "confirm-quit" });
  });
  return () => {
    void unlistenP.then((un) => un());
  };
}
