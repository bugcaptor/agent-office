// src/renderer/bootstrap.ts
//
// App boot sequence: `loadState` -> `hydrate` -> `getAppSettings` ->
// today-worked-total base load -> `installSessionBridge` -> `adoptDetachedSessions`
// seed -> `installPersistence` -> ... -> `installTaskLabelSummarizer` ->
// `installQuitGuard` -> `installSoundManager` -> `installDayRollover`.
//
// Pulled out of `main.tsx` into its own function so it's unit-testable
// without a real DOM root / ReactDOM.render. Order matters:
// - `hydrate` must run before `installSessionBridge`/`installPersistence` so
//   neither reacts to the initial load as if it were a live change (in
//   particular, installing persistence first would queue a pointless
//   just-loaded-it save the moment `hydrate` populates `agents`).
// - `installSessionBridge` before `installPersistence` is the established
//   order; the two don't otherwise depend on each other.
// - Session-handoff adoption runs right after the bridge is live: the
//   backend's adopt path shares `install_session` with normal session
//   creation, so it also emits its own `session-state` event — seeding here
//   is a defensive, redundant fill (plus cols/rows, which that event doesn't
//   carry) against any invoke/event-arrival race, not the sole source of truth.
import { useAppStore } from "./store/appStore";
import { applyLanguageSetting } from "./i18n";
import { installSessionBridge } from "./ipc/sessionBridge";
import { installMascotBridge } from "./ipc/mascotBridge";
import { installDisplayFocusBridge } from "./ipc/displayFocusBridge";
import { installWindowFocusTracking } from "./ipc/windowFocus";
import { installPersistence } from "./store/persist";
import { installPortraitCache } from "./portrait/portraitCache";
import { installSpriteCache } from "./sprite/spriteCache";
import { installMinimiCache } from "./sprite/minimiCache";
import { installMemoCleanup } from "./memo/memoCleanup";
import { installTaskLabelSummarizer } from "./labels/summarizer";
import { installGitBranchWatcher } from "./labels/gitBranchWatcher";
import { installSessionSummaryRecorder } from "./workpanels/sessionSummaryStore";
import { installWorkMonitor } from "./workpanels/workMonitor";
import { installWorkInboxLifecycle } from "./workpanels/lifecycle";
import { stopWorkInbox } from "./workpanels/workInbox";
import { useWorkItemStore } from "./workpanels/workItemStore";
import { installWorkLogRecorder } from "./diary/workLog";
import {
  installWorkLogPersister,
  restoreWorkLogs,
} from "./diary/workLogPersister";
import { installDiaryAutoWriter } from "./diary/diaryAutoWriter";
import { installQuitGuard } from "./quitGuard";
import { installSoundManager } from "./sound/soundManager";
import { useAwardsStore } from "./awards/awardsStore";
import { tauriApi } from "./ipc/tauriApi";
import { adoptDetachedSessions } from "./ipc/sessionAdoption";
import { loadProfiles } from "./store/profileLoad";
import {
  msUntilNextLocalMidnight,
  startOfLocalDay,
  sumWorkedSince,
} from "./timeline/todayTotal";

/**
 * 이 달의 우수사원(docs/employee-of-the-month-design.md §3) 부팅 시 자동 확정:
 * 시상 기록을 읽고, 완료됐지만 아직 레코드가 없는 달을 소급 확정한다(최대
 * 12개월). `adoptDetachedSessions`와 달리 **await하지 않는다** —
 * 최대 12번의 순차 IPC 왕복이 필요해 부팅 시퀀스 전체(요약기·git 브랜치 워처·
 * 일기·quitGuard 등 이후 단계)를 지연시킬 수 있고, 화면에 당장 필요한 데이터도
 * 아니다(다이얼로그를 열 때 한 번 더 보정되므로 이 호출이 끝나기 전에 사용자가
 * 열어도 그때 다시 시도된다). 실패해도 스토어의 `error`에만 남고 부팅에는
 * 영향 없다(두 액션 모두 내부에서 try/catch로 삼킨다).
 */
function finalizeAwardsInBackground(): void {
  void (async () => {
    await useAwardsStore.getState().load();
    await useAwardsStore.getState().ensureFinalized();
  })().catch((err) =>
    console.warn("bootstrap: award backfill finalization failed", err),
  );
}

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
      const memorySum = Object.values(
        useAppStore.getState().timeTracking,
      ).reduce((a, t) => a + t.workedMs, 0);
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
 * `loadState` failing (backend not ready / IPC error / corrupt profiles.json)
 * must NOT abort the boot: `main.tsx` fires this as `void bootApp()`, so a
 * throw here would silently skip the bridge + persistence installs — the
 * shell would paint but notifications/badge would be dead (half-boot).
 * `loadProfiles` falls back to an empty state and continues, but marks the
 * profiles read-only so persistence never overwrites the unread file with
 * that empty list (수정 F2).
 */
export async function bootApp(): Promise<() => void> {
  // false면 프로필을 못 읽은 읽기 전용 상태 — 입양은 재시도 성공 뒤로 미룬다
  // (빈 목록 기준으로 입양하면 남은 세션의 주인을 알 수 없다).
  const profilesLoaded = await loadProfiles();

  // 앱 설정 로드 — 실패해도 부팅은 계속(전부 OFF 기본값 유지, 첫 실행
  // 다이얼로그는 안 띄움: 백엔드 불통 상태에서 온보딩을 저장할 수 없다).
  // installTaskLabelSummarizer보다 먼저 — summarizer가 이 설정을 읽는다.
  try {
    const { settings, firstRun } = await tauriApi.getAppSettings();
    useAppStore.getState().hydrateSettings(settings, firstRun);
    // 저장된 언어를 적용한다. i18n은 이미 localStorage 캐시로 초기화돼 있으므로
    // 보통은 같은 값이라 no-op이고, 다르면(첫 실행·설정 파일을 밖에서 고침)
    // 여기서 한 번 전환된다 — 캐시도 함께 갱신돼 다음 부팅은 플래시가 없다.
    applyLanguageSetting(settings.language);
  } catch (err) {
    console.warn(
      "bootstrap: app settings load failed, continuing with defaults (all off)",
      err,
    );
  }

  // "오늘 일한 시간" 헤드라인 베이스 — 실패해도 부팅 계속(base=0, loadState
  // 실패 패턴과 동일). 부팅 스냅샷은 이후 재읽기 안 함(계산 모델은
  // todayTotal.ts/설계 문서 참고) — 이번 실행 정산분은 메모리로만 누적된다.
  try {
    const records = await tauriApi.loadSessionTurns();
    const base = sumWorkedSince(records, startOfLocalDay(Date.now()));
    useAppStore.getState().setTodayWorkedBase(base, 0);
  } catch (err) {
    console.warn("bootstrap: today-worked load failed, starting from 0", err);
  }

  await useWorkItemStore.getState().load();
  const offSessionSummaries = await installSessionSummaryRecorder();
  const offWorkMonitor = installWorkMonitor();
  const offWorkInbox = installWorkInboxLifecycle();
  const offBridge = installSessionBridge();
  // 데스크톱 마스코트(이슈 #72) — 세션 브리지 직후. officeBus(=emitAgentClicked)를
  // 쓰므로 그 뒤여야 하고, 설정 hydrate 이후라 mascotEnabled를 곧바로 읽는다.
  const offMascot = installMascotBridge();
  const offDisplayFocus = installDisplayFocusBridge();
  // 창 포커스 추적(이슈 #39) — 브리지 직후. 비포커스면 알림 억제 해제 + OS 알림.
  const offFocus = installWindowFocusTracking();
  if (profilesLoaded) await adoptDetachedSessions();
  const offPersistence = installPersistence();
  const offPortraits = installPortraitCache();
  const offSprites = installSpriteCache();
  // 서브에이전트 미니미 픽셀아트 — 스프라이트와 같은 지점·같은 방식(로드 +
  // 캐릭터 삭제 시 PNG 정리).
  const offMinimis = installMinimiCache();
  // 포스트잇 메모(#79) 정리 브리지 — 초상/스프라이트와 같은 지점·같은 방식으로
  // `agents`에서 사라진 캐릭터의 메모 폴더를 지운다.
  const offMemos = installMemoCleanup();
  const offSummarizer = installTaskLabelSummarizer();
  // 라벨의 "프로젝트 (브랜치)" 표시용 cwd→브랜치 폴링. 요약기와 같은 라벨
  // 계열이라 나란히 둔다 — 서로 의존하지 않고 각자 taskLabels를 읽을 뿐이다.
  const offGitBranch = installGitBranchWatcher();
  // 캐릭터 일기(#60) 작업 로그 영속화 — 디스크 스냅샷을 버퍼로 복원한 뒤(recorder가
  // 새 항목을 붙이기 전에 과거 세션 로그가 자리 잡게), 영속화기를 설치해 이후 변경을
  // 디바운스 저장한다. diaryEnabled ON일 때만 디스크에 쓴다(복원은 게이트 무관).
  await restoreWorkLogs();
  const workLogPersister = installWorkLogPersister();
  // 캐릭터 일기(#56)의 원천 — taskLabels를 구독해 작업 로그를 누적한다. 설정과
  // 무관하게 항상 수집하되(상한), 일기 생성 자체는 diaryEnabled opt-in.
  const offWorkLog = installWorkLogRecorder();
  // 세션 종료 시 자동 일기(#60) — diaryEnabled면 종료된 세션 로그로 한 편을 쓴다.
  // 작업 로그 누적(recorder) 직후에 설치해 종료 시점 로그가 이미 반영돼 있게 한다.
  const offDiaryAuto = installDiaryAutoWriter();
  const offQuitGuard = installQuitGuard();
  // 설정 하이드레이트 이후에 설치 — fireImmediately 구독이 최신 사운드
  // 설정(typingSoundEnabled/notifySoundEnabled/soundVolume)을 읽는다.
  const offSound = installSoundManager();
  const offDayRollover = installDayRollover();
  // 세션 브로커 v2 주기 스냅샷 업로드(브로커 모드일 때만 타이머 가동).
  // 이 달의 우수사원 소급 확정 — fire-and-forget(위 함수 주석 참고).
  finalizeAwardsInBackground();

  return () => {
    offBridge();
    offSessionSummaries();
    offWorkMonitor();
    offWorkInbox();
    void stopWorkInbox().then(() => useWorkItemStore.getState().flush());
    offMascot();
    offDisplayFocus();
    offFocus();
    offPersistence();
    offPortraits();
    offSprites();
    offMinimis();
    offMemos();
    offSummarizer();
    offGitBranch();
    offWorkLog();
    workLogPersister.dispose();
    offDiaryAuto();
    offQuitGuard();
    offSound();
    offDayRollover();
  };
}
