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
