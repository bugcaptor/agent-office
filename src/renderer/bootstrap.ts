// src/renderer/bootstrap.ts
//
// App boot sequence: `loadState` -> `hydrate` -> `getAppSettings` ->
// `installSessionBridge` -> `installPersistence` -> ... ->
// `installTaskLabelSummarizer` -> `installQuitGuard` -> `installSoundManager`.
//
// Pulled out of `main.tsx` into its own function so it's unit-testable
// without a real DOM root / ReactDOM.render. Order matters:
// - `hydrate` must run before `installSessionBridge`/`installPersistence` so
//   neither reacts to the initial load as if it were a live change (in
//   particular, installing persistence first would queue a pointless
//   just-loaded-it save the moment `hydrate` populates `agents`).
// - `installSessionBridge` before `installPersistence` is the established
//   order; the two don't otherwise depend on each other.
import { useAppStore } from "./store/appStore";
import { installSessionBridge } from "./ipc/sessionBridge";
import { installPersistence } from "./store/persist";
import { installPortraitCache } from "./portrait/portraitCache";
import { installSpriteCache } from "./sprite/spriteCache";
import { installTaskLabelSummarizer } from "./labels/summarizer";
import { installQuitGuard } from "./quitGuard";
import { installSoundManager } from "./sound/soundManager";
import { tauriApi } from "./ipc/tauriApi";
import type { PersistedState } from "./store/types";

/**
 * Runs the full boot sequence once. Returns a combined teardown (tests only —
 * the running app never calls this, the bridge/persistence live for the
 * app's lifetime).
 *
 * `loadState` failing (backend not ready / IPC error) must NOT abort the
 * boot: `main.tsx` fires this as `void bootApp()`, so a throw here would
 * silently skip the bridge + persistence installs — the shell would paint
 * but notifications/badge/saving would be dead (half-boot). Instead, fall
 * back to an empty `PersistedState` and continue unconditionally.
 */
export async function bootApp(): Promise<() => void> {
  let state: PersistedState;
  try {
    state = await tauriApi.loadState();
  } catch (err) {
    console.warn("bootApp: loadState failed, continuing with empty state", err);
    state = { agents: [], version: 1 };
  }
  useAppStore.getState().hydrate(state);

  // 앱 설정 로드 — 실패해도 부팅은 계속(전부 OFF 기본값 유지, 첫 실행
  // 다이얼로그는 안 띄움: 백엔드 불통 상태에서 온보딩을 저장할 수 없다).
  // installTaskLabelSummarizer보다 먼저 — summarizer가 이 설정을 읽는다.
  try {
    const { settings, firstRun } = await tauriApi.getAppSettings();
    useAppStore.getState().hydrateSettings(settings, firstRun);
  } catch (err) {
    console.warn("bootstrap: 앱 설정 로드 실패 — 기본값(전부 OFF)으로 진행", err);
  }

  const offBridge = installSessionBridge();
  const offPersistence = installPersistence();
  const offPortraits = installPortraitCache();
  const offSprites = installSpriteCache();
  const offSummarizer = installTaskLabelSummarizer();
  const offQuitGuard = installQuitGuard();
  // 설정 하이드레이트 이후에 설치 — fireImmediately 구독이 최신 사운드
  // 설정(soundEnabled/soundVolume)을 읽는다.
  const offSound = installSoundManager();

  return () => {
    offBridge();
    offPersistence();
    offPortraits();
    offSprites();
    offSummarizer();
    offQuitGuard();
    offSound();
  };
}
