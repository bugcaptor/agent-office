// Pixi의 셰이더/유니폼 파서는 기본적으로 `new Function`(eval)을 쓰는데,
// release 빌드에서만 적용되는 CSP(`script-src 'self'`)가 이를 차단해
// WebGL 렌더러 init이 통째로 실패한다(사무실 캔버스가 백지가 되는 원인).
// eval 없이 동작하는 공식 대체 구현을 렌더러 생성 전에 주입한다.
import "pixi.js/unsafe-eval";
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { bootApp } from "./bootstrap";
import { applyTheme } from "./theme/applyTheme";
import { useAppStore } from "./store/appStore";

// 저장된 테마(스토어 초기값 = loadStoredThemeId)를 첫 render 전에 동기
// 적용 — 잘못된 테마로 페인트되는 플래시를 원천 차단한다.
applyTheme(useAppStore.getState().theme);

// Render immediately so the shell (bars, placeholder office) shows up without
// waiting on the async `loadState` round-trip; the store hydrating a moment
// later just triggers the normal reactive re-render (boot sequence is
// extracted to `bootApp` — see that file for the exact ordering).
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

void bootApp();
