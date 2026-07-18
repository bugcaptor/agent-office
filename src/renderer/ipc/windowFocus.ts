// src/renderer/ipc/windowFocus.ts
//
// Tauri 창의 OS 포커스 상태를 추적해 appStore.windowFocused 로 반영한다
// (이슈 #39). 부트 시 1회 설치. 비포커스면 pushNotification 이 인앱 억제를
// 풀고(터미널이 열려 있어도 티커/배지/사운드), sessionBridge 가 OS 데스크탑
// 알림까지 발송한다.
//
// installQuitGuard 와 같은 방식으로 `@tauri-apps/api/window` 를 직접 쓴다.
// Tauri 런타임이 없는 테스트 환경(또는 목이 onFocusChanged/isFocused 를 갖지
// 않는 경우)에서도 안전하도록 전 구간을 방어적으로 감싼다 — 실패 시 기본값
// (windowFocused=true) 을 유지할 뿐이다.
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useAppStore } from "../store/appStore";

/** 창 포커스 추적을 설치한다. 초기값을 isFocused()로 시딩하고 onFocusChanged로
 * 이후 변화를 반영한다. 언리슨 fn 을 반환(테스트 teardown / 대칭성). */
export function installWindowFocusTracking(): () => void {
  const setFocused = (focused: boolean) => useAppStore.getState().setWindowFocused(focused);
  let unlisten: (() => void) | null = null;
  let disposed = false;

  try {
    const win = getCurrentWindow();

    // 초기 포커스 상태 시딩(비동기). 실패는 무시 — 기본 true 유지.
    void Promise.resolve(win.isFocused?.())
      .then((focused) => {
        if (typeof focused === "boolean") setFocused(focused);
      })
      .catch(() => {});

    const listening = win.onFocusChanged?.(({ payload }) => setFocused(payload));
    if (listening && typeof listening.then === "function") {
      void listening
        .then((un) => {
          if (disposed) un();
          else unlisten = un;
        })
        .catch(() => {});
    }
  } catch (err) {
    console.warn("windowFocus: 포커스 추적 설치 실패 — 항상 포커스로 간주", err);
  }

  return () => {
    disposed = true;
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
  };
}
