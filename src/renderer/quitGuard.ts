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
//
// 세션 핸드오프(docs/session-handoff-design.md §핵심 6): `handoff_supported()`는
// 백엔드 왕복이라 매 종료 시점마다 물어보면 늦다 — 부팅 시(이 모듈이 설치될 때)
// 1회 조회해 모듈 전역에 캐시하고, `ConfirmQuitDialog`가 3버튼/2버튼 분기에
// 그 캐시를 읽는다. 조회 실패(구버전 백엔드/미지원 플랫폼 등)는 미지원으로
// 취급 — 기존 2버튼 동작으로 안전하게 폴백.
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useAppStore } from "./store/appStore";
import { tauriApi } from "./ipc/tauriApi";

let handoffSupportedCache = false;

/** 캐시된 handoff_supported() 값. `ConfirmQuitDialog`가 버튼 구성 분기에 쓴다. */
export function isHandoffSupported(): boolean {
  return handoffSupportedCache;
}

/** Registers the app-quit confirmation gate. Returns an unlisten fn. */
export function installQuitGuard(): () => void {
  void tauriApi
    .handoffSupported()
    .then((supported) => {
      handoffSupportedCache = supported;
    })
    .catch(() => {
      handoffSupportedCache = false;
    });

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
