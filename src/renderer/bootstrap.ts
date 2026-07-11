// src/renderer/bootstrap.ts
//
// App boot sequence: `loadState` -> `hydrate` -> `getAppSettings` ->
// today-worked-total base load -> `installSessionBridge` -> `installPersistence`
// -> ... -> `installTaskLabelSummarizer` -> `installQuitGuard` ->
// `installSoundManager` -> `installDayRollover`.
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
import { msUntilNextLocalMidnight, startOfLocalDay, sumWorkedSince } from "./timeline/todayTotal";

/**
 * "오늘 일한 시간" 헤드라인의 로컬 자정 리셋 타이머. 발화 시 베이스를 0으로,
 * 기준선을 그 시점의 Σ메모리 workedMs로 세팅(파일 재읽기 없음 — 계산 모델은
 * docs/superpowers/specs/2026-07-11-today-worked-total-design.md 참고)한 뒤
 * 다음 자정으로 재예약한다. `window.setTimeout`이 아니라 전역 `setTimeout`을
 * 쓴다 — persist.ts의 디바운스 타이머와 동일 컨벤션이며, `window`가 없는
 * Node 테스트 환경(bootstrap.test.ts)에서도 안전하다.
 */
function installDayRollover(): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;

  const schedule = () => {
    timer = setTimeout(() => {
      const memorySum = Object.values(useAppStore.getState().timeTracking).reduce(
        (a, t) => a + t.workedMs,
        0
      );
      useAppStore.getState().setTodayWorkedBase(0, memorySum);
      schedule();
    }, msUntilNextLocalMidnight(Date.now()));
  };
  schedule();

  return () => {
    if (timer !== null) clearTimeout(timer);
  };
}

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

  // "오늘 일한 시간" 헤드라인 베이스 — 실패해도 부팅 계속(base=0, loadState
  // 실패 패턴과 동일). 부팅 스냅샷은 이후 재읽기 안 함(계산 모델은
  // todayTotal.ts/설계 문서 참고) — 이번 실행 정산분은 메모리로만 누적된다.
  try {
    const records = await tauriApi.loadSessionTurns();
    const base = sumWorkedSince(records, startOfLocalDay(Date.now()));
    useAppStore.getState().setTodayWorkedBase(base, 0);
  } catch (err) {
    console.warn("bootstrap: 오늘 작업 시간 로드 실패 — 0으로 시작", err);
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
  const offDayRollover = installDayRollover();

  return () => {
    offBridge();
    offPersistence();
    offPortraits();
    offSprites();
    offSummarizer();
    offQuitGuard();
    offSound();
    offDayRollover();
  };
}
