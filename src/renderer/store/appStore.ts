// src/renderer/store/appStore.ts
//
// Central zustand app store. Lives outside React
// so IPC callbacks (onData/onNotification/onSessionState, wired up by the
// session bridge in a later task) can call `getState()/setState()` directly
// without depending on hooks.
import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";
import { notificationType } from "./types";
import type {
  AgentProfile,
  AgentTaskLabel,
  ModalState,
  Notification,
  NotificationEvent,
  PersistedState,
  SessionRuntime,
  SessionStatus,
  TurnUsageEvent,
} from "./types";
import { initialTurnState, reduceTurn } from "../timeline/turnReducer";
import type { AgentTurnState, TurnInput } from "../timeline/turnReducer";
import { requestSentence } from "../labels/labelText";
import { currentTextRules } from "../i18n/textRules";
import { applyTerminalBg, applyTheme, loadStoredThemeId } from "../theme/applyTheme";
import type { ThemeId } from "../theme/themes";
import { loadStoredSceneId, persistSceneId } from "../office/scenes/sceneStorage";
import type { SceneId } from "../office/scenes/sceneTypes";
import {
  loadStoredTerminalViewMode,
  nextTerminalViewMode,
  persistTerminalViewMode,
} from "../terminal/terminalViewMode";
import type { TerminalViewMode } from "../terminal/terminalViewMode";
import {
  loadStoredXtermThemeOverride,
  persistXtermThemeOverride,
} from "../terminal/theme";
import type { XtermThemeOverride } from "../terminal/theme";
import type {
  ActivityEvent,
  AppSettings,
  BotAgentStatus,
  BotStatus,
  SessionState,
  UsageSnapshot,
} from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { backendErrorText } from "../shared/backendError";
import type { PendingPairing } from "../ipc/webRemoteApi";
import { addTurn, emptyTotals } from "../usage/sessionCost";
import type { SessionUsageTotals } from "../usage/sessionCost";

const MAX_EXCERPT = 80;
/** 도구 요약 라벨 갱신 최소 간격(ms). 도구가 빠르게 연달아 와도 라벨이 튀지 않게 스로틀. */
const TOOL_LABEL_MIN_INTERVAL_MS = 2000;
/**
 * 요청 문장이 목표 폴백을 갱신할 만한가 — 충분히 길고 맞장구성이 아니어야 한다.
 * 기준(최소 글자 수·맞장구 토큰)은 언어마다 다르므로 `i18n/textRules`가 갖고
 * 있고, **호출 시점에** 고른다(모듈 최상위에 굳히면 언어 변경이 안 먹는다).
 */
function isMeaningfulGoalFallback(cand: string): boolean {
  const rules = currentTextRules();
  return (
    Array.from(cand).length >= rules.goalFallbackMinChars &&
    !rules.backchannelStart.test(cand)
  );
}
const DEFAULT_APP_SETTINGS: AppSettings = {
  version: 1,
  language: "system",
  summarizerEnabled: false,
  summaryProvider: "claude",
  summaryModels: {
    claude: { light: "", heavy: "", command: "" },
    codex: { light: "", heavy: "", command: "" },
    agy: { light: "", heavy: "", command: "" },
    gemini: { light: "", heavy: "", command: "" },
    opencode: { light: "", heavy: "", command: "" },
    openrouter: { light: "", heavy: "", command: "" },
  },
  diaryEnabled: false,
  observerEnabled: false,
  typingSoundEnabled: true,
  notifySoundEnabled: true,
  soundVolume: 0.5,
  externalTerminal: "terminal",
  externalEditor: "system",
  attentionHoldMs: 5000,
  gitStatusEnabled: true,
  workdirShowIgnored: false,
  fileIndexBackend: "walker",
  cliEnabled: false,
  keepAwakeEnabled: false,
  sessionLogEnabled: true,
  runRecipesEnabled: false,
  mascotEnabled: false,
  mascotLightsMode: "off",
  mascotLightsVertical: false,
  mascotLightsProjects: [],
  mascotLightsFace: "sprite",
  mascotLightsLabel: "auto",
  usageFloatEnabled: true,
  sessionCostEnabled: true,
  ttsEnabled: false,
  ttsRewriteModelAnthropic: "claude-haiku-4-5",
  ttsRewriteModelOpenrouter: "openai/gpt-5.4-mini",
  ttsRewriteProvider: "auto",
  webRemoteBind: "tailnet",
  webRemotePort: 47800,
  webRemoteEnabled: false,
  talkEnabled: false,
  talkMaxTurns: 6,
  talkIdleQuietMs: 3000,
};

/**
 * 턴이 방금 종료됐으면(turns 증가) 그 턴의 시계열 기록을 로컬 로그에 append한다.
 * 순수 reducer를 건드리지 않고 prev/next 델타로 기록을 복원한다:
 * 시작=prev.turnStartedAt(닫힌 턴의 시작), 종료=at, 각 시간=next-prev 델타.
 * fire-and-forget — 저장 실패는 콘솔 경고로만.
 */
function logSettledTurn(
  agentId: string,
  prev: AgentTurnState,
  next: AgentTurnState,
  at: number
): void {
  if (next.turns <= prev.turns) return;
  tauriApi.appendSessionTurn({
    agentId,
    startedAt: prev.turnStartedAt ?? at,
    endedAt: at,
    totalMs: next.totalMs - prev.totalMs,
    workedMs: next.workedMs - prev.workedMs,
    waitedMs: next.waitedMs - prev.waitedMs,
  });
}

interface AppState {
  // ---- data ----
  agents: Record<string, AgentProfile>;
  /** Creation order (tab strip / count display). */
  agentOrder: string[];
  sessions: Record<string, SessionRuntime>;
  /** Global queue, kept sorted newest-first (`createdAt` desc). */
  notifications: Notification[];
  /** null = terminal overlay closed. */
  activeTerminalAgentId: string | null;
  /** 터미널 오버레이 뷰 모드(이슈 #69). windowed=중앙 72%, filled=앱 창 꽉 채움.
   * localStorage로 영속 — PersistedState 아님. */
  terminalViewMode: TerminalViewMode;
  /** Tauri 창(웹뷰)이 OS 포커스를 가졌는지. 기본 true. sessionBridge의
   * 포커스 추적이 갱신한다(이슈 #39). 비포커스면 터미널이 열려 있어도
   * 알림을 억제하지 않고 OS 데스크탑 알림까지 보낸다. */
  windowFocused: boolean;
  /** Tab strip order (LRU, most-recent first). */
  recentAgentIds: string[];
  modal: ModalState;
  muted: boolean;
  /** 휴가 모드(보스 책상 클릭으로 토글). true = 줄 전원 이탈. */
  vacationMode: boolean;
  /** 현재 테마 id. localStorage("agent-office.theme")로 영속 — PersistedState 아님. */
  theme: ThemeId;
  /**
   * 현재 풍경(오피스 씬) id. 테마(색 축)와 직교한 축 — 어떤 조합도 유효하다.
   * localStorage("agent-office.scene")로 영속 — PersistedState 아님(=Rust
   * AppSettings와 무관한 순수 프런트 취향값).
   */
  scene: SceneId;
  /**
   * 터미널(xterm) 색상 선택. "auto"(기본)면 `theme`를 따라가고, 특정 테마 id면
   * 앱 테마와 무관하게 터미널 색만 그 테마로 고정한다.
   * localStorage("agent-office.xterm-theme")로 영속 — PersistedState 아님.
   */
  xtermTheme: XtermThemeOverride;
  /** 초상 dataURL 캐시(agentId -> "data:image/png;base64,..."). 런타임 전용, 영속 안 함. */
  portraits: Record<string, string>;
  /** 커스텀 스프라이트 프리뷰(idle0 확대) dataURL 캐시. 런타임 전용, 영속 안 함. */
  spritePreviews: Record<string, string>;
  /** 커스텀 미니미 프리뷰(단일 프레임 확대) dataURL 캐시. 프로필 다이얼로그
   * 표시용 + "커스텀 미니미 있음" 판정용. 런타임 전용, 영속 안 함. */
  minimiPreviews: Record<string, string>;
  /** 에이전트별 턴 집계(메모리 전용, 순수 리듀서 상태). */
  timeTracking: Record<string, AgentTurnState>;
  /**
   * 터미널 요약 바에 그리는 "현재 세션" 토큰·비용 실시간 누계. 세션이 바뀌면
   * (또는 처음 잡히면) `{ sessionId, totals: emptyTotals() }`로 리셋된다 —
   * `noteUsageSession`/`applyTurnUsage` 참조. 런타임 전용(비영속).
   */
  sessionUsage: Record<string, { sessionId: string; totals: SessionUsageTotals }>;
  /**
   * `useSessionUsageSeed`가 앱 수명당 1회 심는 과거 시드(`{at, bySession}`).
   * `at` 이하 시각의 사용량은 이미 시드에 들어 있으므로 `applyTurnUsage`가
   * 이중 계산을 막는 데 쓴다. null = 아직 시딩 전(또는 설정이 꺼짐). 런타임
   * 전용(비영속) — `docs/session-analytics-design.md` §11.
   */
  sessionUsageSeed: { at: number; bySession: Record<string, SessionUsageTotals> } | null;
  /**
   * `applyTurnUsage`가 **실제로 누계에 반영한 첫 턴**의 `e.at`(무시된
   * 사용량은 기록하지 않는다). `useSessionUsageSeed`가 시드 컷오프를 여기에
   * 묶는다(`firstAt - 1`) — "훅이 언제 돌았나"가 아니라 "실시간이 언제부터
   * 세었나"에 묶어야, 시드가 늦게 심겨도(설정을 뒤늦게 켠 경우 등) 실시간과
   * 겹치는 구간이 구조적으로 없다(§11.3). null = 아직 실시간이 한 턴도 안
   * 반영함. 런타임 전용(비영속).
   */
  sessionUsageFirstAt: number | null;
  /**
   * "오늘 일한 시간" 헤드라인 베이스: 부팅 시 JSONL에서 산출한 오늘자 합
   * (자정 리셋 시 0). `memoryWorkedBaselineMs`와 함께 이후 Σ메모리 workedMs
   * 델타를 더해 오늘 총량을 구한다(계산은 selectors.useTodayWorkedMs).
   * 런타임 전용 — persist.ts는 agents만 저장하므로 대상 아님.
   */
  todayWorkedBaseMs: number;
  /** 위 베이스가 세팅된 시점의 Σ메모리 workedMs(이중 집계 방지 기준선). */
  memoryWorkedBaselineMs: number;
  /** 머리 위 작업 라벨 소스 상태. 비영속. */
  taskLabels: Record<string, AgentTaskLabel>;
  /**
   * 작업 폴더(cwd) → 현재 브랜치명. 라벨 표면이 "프로젝트 (브랜치) · 목표"를
   * 그릴 때만 쓴다. **git 저장소이고 브랜치가 있을 때만 키가 존재한다** —
   * 비저장소·detached HEAD는 키 자체를 넣지 않아, 조회 전과 "브랜치 없음"이
   * 표시상 같게(= 괄호 생략) 떨어진다. gitBranchWatcher가 30초 주기로 채우는
   * 런타임 전용 상태(비영속).
   */
  gitBranches: Record<string, string>;
  /**
   * 터미널 재시작 에폭(agentId -> 정수, 기본 0). TerminalMount의 key에
   * 반영되어, 증가하면 강제 리마운트 -> attach()가 새 xterm을 만든다.
   * 런타임 전용, 영속 안 함(persist.ts는 agents만 저장).
   */
  terminalEpochs: Record<string, number>;
  /** 앱 전역 opt-in 설정. 기본값 전부 OFF — 부트 시 getAppSettings로 하이드레이트. */
  appSettings: AppSettings;
  /** true = settings.json 부재(첫 실행) — 온보딩 다이얼로그 표시 트리거. */
  settingsFirstRun: boolean;
  /**
   * `hydrateSettings`가 실제로 불렸는지(부트 시 `getAppSettings` IPC 왕복이
   * 끝났는지). 스토어 생성 시점의 `appSettings`는 `DEFAULT_APP_SETTINGS`
   * 플레이스홀더라 그것만으로는 "진짜 설정이 왔는지" 구분이 안 된다 —
   * `useSessionUsageSeed`가 이 값이 true가 될 때까지 시딩을 미루는 게이트로
   * 쓴다(§11.5). `AppSettings`의 필드가 아니다(영속 대상 아님, 계약 밖).
   * 런타임 전용, 초기값 false.
   */
  settingsHydrated: boolean;
  /** 구독 사용량 스냅샷. null = 아직 로드 전. UsageWidget이 60초 폴링으로 채운다.
   * 런타임 전용(비영속). */
  usage: UsageSnapshot | null;
  /** 봇 모드가 켜진 탭(이슈 #57). agentId → 런타임 상태. 여기 있으면 그 탭은
   * "봇 운전 중" — 로컬 키 입력이 잠기고 배지가 뜬다. 런타임 전용(비영속,
   * 앱 재시작 시 꺼진 상태로 시작). */
  botMode: Record<string, BotAgentStatus>;
  /** 웹 원격: 승인 대기 중인 페어링(코드 표시용). 런타임 전용. */
  webRemotePending: PendingPairing[];

  // ---- profile actions ----
  addAgent(profile: AgentProfile): void;
  updateAgent(agentId: string, patch: Partial<AgentProfile>): void;
  removeAgent(agentId: string): void;
  /**
   * 책상 수동 지정: `agentId`에게 `deskIndex`를 배정하고, 그 책상을 갖고
   * 있던 다른 에이전트의 지정은 해제한다(책상당 주인 1명). `agentId=null`
   * 이면 지정 해제만 한다. agents 객체가 바뀌므로 persist가 자동 저장.
   */
  assignDesk(deskIndex: number, agentId: string | null): void;
  setPortrait(agentId: string, dataUrl: string): void;
  removePortrait(agentId: string): void;
  setSpritePreview(agentId: string, dataUrl: string): void;
  removeSpritePreview(agentId: string): void;
  setMinimiPreview(agentId: string, dataUrl: string): void;
  removeMinimiPreview(agentId: string): void;

  // ---- session actions ----
  /** `external`은 백엔드 `SessionStateEvent.external` 그대로 — true면 외부(논리)
   * 세션의 전이라 `kind`를 `external`로, 부재/false면 `pty`로 확정한다. */
  setSessionState(e: { agentId: string; status: SessionStatus; external?: boolean }): void;
  setSessionSize(agentId: string, cols: number, rows: number): void;

  // ---- 웹 원격 ----
  /** 승인 대기 페어링 목록 갱신. */
  setWebRemotePending(pending: PendingPairing[]): void;

  // ---- bot mode (이슈 #57) ----
  /** 이 탭의 봇 모드를 켠다 — 백엔드 폴링 태스크를 띄우고 로컬 입력을 잠근다.
   * 실패해도 상태(error)를 저장해 배지가 원인을 보여준다. */
  startBot(agentId: string): Promise<void>;
  /** 이 탭의 봇 모드를 끈다 — 백엔드 태스크를 내리고 로컬 조작으로 복귀. */
  stopBot(agentId: string): Promise<void>;
  /** bot_status 폴링 결과를 병합한다(이슈 번호·오류 갱신). */
  applyBotStatus(status: BotStatus): void;
  /**
   * 부팅 시 백엔드에 살아 있는 봇 태스크를 botMode에 심는다. `applyBotStatus`와
   * 달리 **없던 항목도 추가**한다 — 앱을 재시작하면 botMode(비영속)가 비어 있어
   * 5초 폴링(hasBots>0 게이트)이 아예 안 돌고, 그 결과 백엔드가 계속 운전 중인
   * 탭인데도 입력 잠금(isBotDriven)이 풀린 채로 남는 버그를 막는 시드다.
   */
  seedBotStatus(status: BotStatus): void;
  /** 이 탭이 봇 운전 중인지(로컬 입력 차단 여부 판정). */
  isBotDriven(agentId: string): boolean;

  // ---- window focus ----
  /** OS 창 포커스 상태 반영(이슈 #39). sessionBridge의 포커스 추적이 호출. */
  setWindowFocused(focused: boolean): void;

  // ---- clock in/out ----
  /** 퇴근: 프로필을 clockedOut=true로, 세션 런타임/최근탭에서 제거하고,
   * 활성 터미널이면 이웃 탭으로 전환(없으면 닫음). 프로필/초상/스프라이트/
   * timeTracking은 보존(되돌릴 수 있음). agents가 바뀌므로 persist가 자동 저장. */
  clockOut(agentId: string): void;
  /** 소환: clockedOut 플래그를 해제(필드 제거)한다. 캔버스 재등장은
   * useAgentList 필터가 처리하고, 세션/터미널 복구는 호출자(clockInAgent)가 한다. */
  clockIn(agentId: string): void;

  // ---- notification actions ----
  pushNotification(e: NotificationEvent): void;
  clearNotificationsFor(agentId: string): void;
  clearNotificationByIds(agentId: string, ids: string[]): void;

  // ---- terminal overlay ----
  /** Sets active + bumps recent + clears that agent's notifications. */
  openTerminal(agentId: string): void;
  /** Closes the overlay only; the underlying session keeps running. */
  closeTerminal(): void;
  /** 터미널 재시작: 에폭 +1 -> TerminalMount key 변경 -> 강제 리마운트. */
  bumpTerminalEpoch(agentId: string): void;

  // ---- modal ----
  openModal(modal: ModalState): void;
  closeModal(): void;

  // ---- mute ----
  /** Flips `muted`. Badge resync on toggle is the session bridge's job. */
  toggleMuted(): void;

  // ---- vacation mode ----
  /** Flips `vacationMode`. officeBus relay to the scene is the session bridge's job. */
  toggleVacationMode(): void;

  // ---- theme ----
  /** 테마 전환: DOM 적용(applyTheme) + localStorage 영속 + 상태 갱신. */
  setTheme(id: ThemeId): void;
  /**
   * 터미널 색상 선택: --term-bg 갱신 + localStorage 영속 + 상태 갱신.
   * 살아있는 xterm 인스턴스 재도색은 App의 효과(terminalRegistry.setTheme)가 맡는다.
   */
  setXtermTheme(override: XtermThemeOverride): void;

  // ---- office scene (풍경) ----
  /** 풍경 전환: localStorage 영속 + 상태 갱신. 실제 씬 재구축은 App의
   * `<OfficeCanvas scene>` 배선 → `OfficeScene.setScene`이 맡는다. */
  setScene(id: SceneId): void;

  // ---- terminal view mode (이슈 #69) ----
  /** 뷰 모드 지정 + localStorage 영속. */
  setTerminalViewMode(mode: TerminalViewMode): void;
  /** windowed ↔ filled 토글. */
  cycleTerminalViewMode(): void;

  // ---- time tracking (feeds turnReducer) ----
  applyActivityEvent(e: ActivityEvent): void;
  applyNotificationTiming(e: NotificationEvent): void;
  applySessionTiming(agentId: string, state: SessionState, at: number): void;

  // ---- session usage (터미널 요약 바 토큰·비용, docs/session-analytics-design.md §11) ----
  /** 이 에이전트가 지금 이 `sessionId`를 쓰고 있음을 알린다. 엔트리가
   * 없거나 세션이 바뀌었으면 누계를 0으로 새로 깐다(같으면 no-op). */
  noteUsageSession(agentId: string, sessionId: string): void;
  /** turn-usage 이벤트를 세션 누계에 더한다(알림과 분리된 채널이라 서브에이전트로
   * 억제된 Stop에서도 온다). 시드가 이미 깔려 있고 `e.at <= sessionUsageSeed.at`
   * 이면 그 턴은 시드에 이미 들어 있으므로 무시한다(이중 계산 방지). `e.partial`은
   * 그대로 `addTurn`에 넘긴다 — PostToolUse 중간 갱신(partial:true)은 토큰·비용은
   * 반영하되 턴 수는 안 올린다(§11.9). */
  applyTurnUsage(e: TurnUsageEvent): void;
  /** `useSessionUsageSeed`가 부팅 후 1회 호출. 이미 시드가 있으면 no-op. */
  setSessionUsageSeed(seed: { at: number; bySession: Record<string, SessionUsageTotals> }): void;
  /**
   * "오늘" 헤드라인 베이스+기준선을 함께 세팅. 부팅 시 `(base, 0)`,
   * 로컬 자정 리셋 시 `(0, 현재 Σ메모리 workedMs)`.
   */
  setTodayWorkedBase(baseMs: number, baselineMs: number): void;

  // ---- overhead task label ----
  setTaskLabelSummary(agentId: string, patch: { goal?: string; currentSummary?: string }): void;
  /** cwd→브랜치 맵을 통째로 교체한다(gitBranchWatcher의 폴링 1회분 결과).
   * 부분 병합·가지치기는 순수 함수(gitBranchWatcher.nextGitBranches)가 맡는다. */
  setGitBranches(next: Record<string, string>): void;

  // ---- persistence hydration ----
  hydrate(state: PersistedState): void;

  // ---- usage ----
  /** 폴링으로 받은 사용량 스냅샷 반영. */
  setUsage(snapshot: UsageSnapshot): void;

  // ---- app settings ----
  /** 부트 시 백엔드 getAppSettings 결과 반영. */
  hydrateSettings(settings: AppSettings, firstRun: boolean): void;
  /** 스토어 갱신 + 백엔드 저장(fire-and-forget). */
  updateAppSettings(
    patch: Partial<
      Pick<
        AppSettings,
        | "language"
        | "summarizerEnabled"
        | "summaryProvider"
        | "summaryModels"
        | "diaryEnabled"
        | "observerEnabled"
        | "typingSoundEnabled"
        | "notifySoundEnabled"
        | "soundVolume"
        | "externalTerminal"
        | "externalEditor"
        | "attentionHoldMs"
        | "gitStatusEnabled"
        | "workdirShowIgnored"
        | "fileIndexBackend"
        | "cliEnabled"
        | "keepAwakeEnabled"
        | "sessionLogEnabled"
        | "runRecipesEnabled"
        | "mascotEnabled"
        | "mascotLightsMode"
        | "mascotLightsVertical"
        | "mascotLightsProjects"
        | "mascotLightsFace"
        | "mascotLightsLabel"
        | "usageFloatEnabled"
        | "sessionCostEnabled"
        | "webRemoteBind"
        | "webRemotePort"
        | "webRemoteEnabled"
        | "ttsEnabled"
        | "ttsRewriteModelAnthropic"
        | "ttsRewriteModelOpenrouter"
        | "ttsRewriteProvider"
        | "talkEnabled"
        | "talkMaxTurns"
        | "talkIdleQuietMs"
      >
    >,
  ): void;
  /** 첫 실행 온보딩 선택 저장 + firstRun 종료. */
  completeFirstRun(
    choice: Pick<
      AppSettings,
      "summarizerEnabled" | "summaryProvider" | "diaryEnabled" | "observerEnabled"
    >,
  ): void;
}

export const useAppStore = create<AppState>()(
  subscribeWithSelector((set, get) => ({
    agents: {},
    agentOrder: [],
    sessions: {},
    notifications: [],
    activeTerminalAgentId: null,
    terminalViewMode: loadStoredTerminalViewMode(),
    windowFocused: true,
    recentAgentIds: [],
    modal: { kind: "none" },
    muted: false,
    vacationMode: false,
    theme: loadStoredThemeId(), // 스토어 생성 시점(첫 render 전)에 저장값 복원 → 플래시 없음
    scene: loadStoredSceneId(), // 테마와 같은 이유로 스토어 생성 시점에 복원
    xtermTheme: loadStoredXtermThemeOverride(),
    portraits: {},
    spritePreviews: {},
    minimiPreviews: {},
    timeTracking: {},
    sessionUsage: {},
    sessionUsageSeed: null,
    sessionUsageFirstAt: null,
    todayWorkedBaseMs: 0,
    memoryWorkedBaselineMs: 0,
    taskLabels: {},
    gitBranches: {},
    terminalEpochs: {},
    appSettings: DEFAULT_APP_SETTINGS,
    settingsFirstRun: false,
    settingsHydrated: false,
    usage: null,
    botMode: {},
    webRemotePending: [],

    setWebRemotePending: (pending) => set({ webRemotePending: pending }),

    addAgent: (profile) =>
      set((s) => ({
        agents: { ...s.agents, [profile.id]: profile },
        agentOrder: [...s.agentOrder, profile.id],
        sessions: {
          ...s.sessions,
          [profile.id]: {
            agentId: profile.id,
            status: "starting",
            cols: 80,
            rows: 24,
            lastActivityAt: Date.now(),
          },
        },
      })),

    updateAgent: (agentId, patch) =>
      set((s) =>
        s.agents[agentId]
          ? { agents: { ...s.agents, [agentId]: { ...s.agents[agentId], ...patch } } }
          : s
      ),

    assignDesk: (deskIndex, agentId) =>
      set((s) => {
        const agents = { ...s.agents };
        let changed = false;
        for (const [id, a] of Object.entries(agents)) {
          if (id === agentId) {
            if (a.assignedDeskIndex !== deskIndex) {
              agents[id] = { ...a, assignedDeskIndex: deskIndex };
              changed = true;
            }
          } else if (a.assignedDeskIndex === deskIndex) {
            // 이 책상의 기존 주인 — 지정 해제(필드 제거: undefined는 JSON
            // 직렬화에서 빠져 Rust Option<None>과 일치).
            const { assignedDeskIndex: _drop, ...rest } = a;
            agents[id] = rest as typeof a;
            changed = true;
          }
        }
        return changed ? { agents } : s;
      }),

    removeAgent: (agentId) =>
      set((s) => {
        const agents = { ...s.agents };
        delete agents[agentId];
        const sessions = { ...s.sessions };
        delete sessions[agentId];
        const portraits = { ...s.portraits };
        delete portraits[agentId];
        const spritePreviews = { ...s.spritePreviews };
        delete spritePreviews[agentId];
        const minimiPreviews = { ...s.minimiPreviews };
        delete minimiPreviews[agentId];
        const timeTracking = { ...s.timeTracking };
        delete timeTracking[agentId];
        const sessionUsage = { ...s.sessionUsage };
        delete sessionUsage[agentId];
        const taskLabels = { ...s.taskLabels };
        delete taskLabels[agentId];
        const terminalEpochs = { ...s.terminalEpochs };
        delete terminalEpochs[agentId];
        return {
          agents,
          sessions,
          portraits,
          spritePreviews,
          minimiPreviews,
          timeTracking,
          sessionUsage,
          taskLabels,
          terminalEpochs,
          agentOrder: s.agentOrder.filter((id) => id !== agentId),
          recentAgentIds: s.recentAgentIds.filter((id) => id !== agentId),
          notifications: s.notifications.filter((n) => n.agentId !== agentId),
          activeTerminalAgentId:
            s.activeTerminalAgentId === agentId ? null : s.activeTerminalAgentId,
        };
      }),

    clockOut: (agentId) =>
      set((s) => {
        const agent = s.agents[agentId];
        if (!agent || agent.clockedOut) return s;
        const sessions = { ...s.sessions };
        delete sessions[agentId];
        // 활성 터미널이면 이웃(다음, 없으면 이전)으로 전환. 이웃도 퇴근 대상일
        // 수는 없다(퇴근하는 건 agentId 하나뿐) — recentAgentIds에서 계산.
        const recent = s.recentAgentIds.filter((id) => id !== agentId);
        let active = s.activeTerminalAgentId;
        if (active === agentId) {
          const idx = s.recentAgentIds.indexOf(agentId);
          active = s.recentAgentIds[idx + 1] ?? s.recentAgentIds[idx - 1] ?? null;
        }
        return {
          agents: { ...s.agents, [agentId]: { ...agent, clockedOut: true } },
          sessions,
          recentAgentIds: recent,
          activeTerminalAgentId: active,
          notifications: s.notifications.filter((n) => n.agentId !== agentId),
        };
      }),

    clockIn: (agentId) =>
      set((s) => {
        const agent = s.agents[agentId];
        if (!agent || !agent.clockedOut) return s;
        const { clockedOut: _drop, ...rest } = agent;
        // clockOut이 지운 세션 런타임 엔트리를 되살린다 — 없으면
        // setSessionState가 prev 부재로 no-op이 되어 상태가 영영
        // starting/running으로 못 바뀌고, 머리 위 현황 UI가 (재시작 전까지)
        // 뜨지 않는다. addAgent와 동일한 초기값으로 재생성.
        return {
          agents: { ...s.agents, [agentId]: rest as typeof agent },
          sessions: {
            ...s.sessions,
            [agentId]: {
              agentId,
              status: "starting",
              cols: 80,
              rows: 24,
              lastActivityAt: Date.now(),
            },
          },
        };
      }),

    setSessionState: ({ agentId, status, external }) =>
      set((s) => {
        const prev = s.sessions[agentId];
        if (!prev) return s;
        // kind는 매 전이의 출처로 확정한다: 외부 세션 이벤트만 external=true를
        // 달고 오고(attach/detach 양쪽), PTY 경로(생성/재시작/입양/실패)는 이
        // 필드를 아예 보내지 않으므로 pty로 되돌아간다.
        return {
          sessions: {
            ...s.sessions,
            [agentId]: {
              ...prev,
              status,
              kind: external ? "external" : "pty",
              lastActivityAt: Date.now(),
            },
          },
        };
      }),

    setSessionSize: (agentId, cols, rows) =>
      set((s) => {
        const prev = s.sessions[agentId];
        if (!prev || (prev.cols === cols && prev.rows === rows)) return s;
        return { sessions: { ...s.sessions, [agentId]: { ...prev, cols, rows } } };
      }),

    startBot: async (agentId) => {
      // 낙관적으로 먼저 켠 상태로 표시(입력 잠금·배지 즉시 반영), 백엔드 응답으로
      // 실제 상태(이슈/오류)를 갱신한다. 실패해도 켠 상태는 유지하고 error만 표시.
      set((s) => ({
        botMode: {
          ...s.botMode,
          [agentId]: { running: true, phase: "starting", pollIntervalSec: 60 },
        },
      }));
      try {
        const status: BotAgentStatus = await tauriApi.botStart(agentId);
        set((s) => ({ botMode: { ...s.botMode, [agentId]: status } }));
      } catch (err) {
        set((s) => ({
          botMode: {
            ...s.botMode,
            [agentId]: {
              running: false,
              phase: "error",
              pollIntervalSec: 60,
              // 백엔드가 코드를 내면 카탈로그 문구로, 아니면 원문 그대로.
              error: backendErrorText(err),
            },
          },
        }));
      }
    },

    stopBot: async (agentId) => {
      try {
        await tauriApi.botStop(agentId);
      } catch (err) {
        console.warn("bot: stop failed", err);
      }
      set((s) => {
        const next = { ...s.botMode };
        delete next[agentId];
        return { botMode: next };
      });
    },

    applyBotStatus: (status) =>
      set((s) => {
        // 봇 모드가 켜진(로컬 상태에 있는) 탭만 갱신한다 — 방금 끈 탭이 폴링
        // 응답으로 되살아나지 않게.
        const next = { ...s.botMode };
        let changed = false;
        for (const id of Object.keys(next)) {
          const fresh = status.agents[id];
          if (fresh) {
            next[id] = fresh;
            changed = true;
          }
        }
        return changed ? { botMode: next } : s;
      }),

    seedBotStatus: (status) =>
      set((s) => {
        const entries = Object.entries(status.agents);
        if (entries.length === 0) return s;
        const next = { ...s.botMode };
        for (const [id, st] of entries) next[id] = st;
        return { botMode: next };
      }),

    isBotDriven: (agentId) => agentId in get().botMode,

    setWindowFocused: (focused) =>
      set((s) => (s.windowFocused === focused ? s : { windowFocused: focused })),

    pushNotification: (e) =>
      set((s) => {
        // Suppress only when the active terminal is already showing this agent
        // AND the app window has focus. If the window is backgrounded, surface
        // the notification (ticker + badge + sound) even with the terminal
        // open, so a completed task isn't missed (이슈 #39).
        if (s.activeTerminalAgentId === e.agentId && s.windowFocused) return s;
        const n: Notification = {
          id: e.id, // reuse the backend-issued id as-is.
          agentId: e.agentId,
          type: notificationType(e.source),
          message: e.message,
          excerpt:
            e.message.length > MAX_EXCERPT
              ? e.message.slice(0, MAX_EXCERPT - 1) + "…"
              : e.message,
          createdAt: e.at,
        };
        return { notifications: [n, ...s.notifications] }; // newest first
      }),

    clearNotificationsFor: (agentId) =>
      set((s) => ({ notifications: s.notifications.filter((n) => n.agentId !== agentId) })),

    clearNotificationByIds: (agentId, ids) =>
      set((s) => {
        const drop = new Set(ids);
        return {
          notifications: s.notifications.filter(
            (n) => n.agentId !== agentId || !drop.has(n.id)
          ),
        };
      }),

    openTerminal: (agentId) =>
      set((s) => {
        if (!s.agents[agentId]) return s;
        return {
          activeTerminalAgentId: agentId,
          recentAgentIds: [agentId, ...s.recentAgentIds.filter((id) => id !== agentId)],
          notifications: s.notifications.filter((n) => n.agentId !== agentId),
        };
      }),

    closeTerminal: () => set({ activeTerminalAgentId: null }),

    bumpTerminalEpoch: (agentId) =>
      set((s) => ({
        terminalEpochs: { ...s.terminalEpochs, [agentId]: (s.terminalEpochs[agentId] ?? 0) + 1 },
      })),

    openModal: (modal) => set({ modal }),
    closeModal: () => set({ modal: { kind: "none" } }),

    toggleMuted: () => set((s) => ({ muted: !s.muted })),

    toggleVacationMode: () => set((s) => ({ vacationMode: !s.vacationMode })),

    setTheme: (id) => {
      // 부수효과(DOM/localStorage)를 액션에서 직접 수행 — 이 스토어는 React
      // 밖(IPC 콜백 등)에서도 호출되므로 별도 구독자 계층을 두지 않는다.
      applyTheme(id);
      set({ theme: id });
    },

    setScene: (id) => {
      // theme와 동일 패턴: 부수효과(localStorage 영속)를 액션에서 직접 수행.
      persistSceneId(id);
      set({ scene: id });
    },

    setXtermTheme: (override) => {
      // theme와 동일 패턴: 부수효과(영속 + --term-bg)를 액션에서 직접 수행.
      persistXtermThemeOverride(override);
      applyTerminalBg(get().theme, override);
      set({ xtermTheme: override });
    },

    setTerminalViewMode: (mode) => {
      // theme와 동일 패턴: 부수효과(localStorage 영속)를 액션에서 직접 수행.
      persistTerminalViewMode(mode);
      set({ terminalViewMode: mode });
    },
    cycleTerminalViewMode: () =>
      get().setTerminalViewMode(nextTerminalViewMode(get().terminalViewMode)),

    setPortrait: (agentId, dataUrl) =>
      set((s) => ({ portraits: { ...s.portraits, [agentId]: dataUrl } })),

    removePortrait: (agentId) =>
      set((s) => {
        if (!(agentId in s.portraits)) return s;
        const portraits = { ...s.portraits };
        delete portraits[agentId];
        return { portraits };
      }),

    setSpritePreview: (agentId, dataUrl) =>
      set((s) => ({ spritePreviews: { ...s.spritePreviews, [agentId]: dataUrl } })),

    removeSpritePreview: (agentId) =>
      set((s) => {
        if (!(agentId in s.spritePreviews)) return s;
        const spritePreviews = { ...s.spritePreviews };
        delete spritePreviews[agentId];
        return { spritePreviews };
      }),

    setMinimiPreview: (agentId, dataUrl) =>
      set((s) => ({ minimiPreviews: { ...s.minimiPreviews, [agentId]: dataUrl } })),

    removeMinimiPreview: (agentId) =>
      set((s) => {
        if (!(agentId in s.minimiPreviews)) return s;
        const minimiPreviews = { ...s.minimiPreviews };
        delete minimiPreviews[agentId];
        return { minimiPreviews };
      }),

    applyActivityEvent: (e) =>
      set((s) => {
        // 서브에이전트 카운트 신호는 시간 추적/라벨 대상이 아니다(카운트는
        // sessionBridge가 별도 소유). reduceTurn의 TurnInputKind로 좁히기 위해서도 필요.
        // resume(이슈 #39)은 완료 후 출력 지속 신호 — 턴 목적상 tool과 동일하게
        // 취급해 idle→working으로 복귀시킨다(라벨 갱신 대상은 아니다).
        // idle(kbm #2f9): 셸 포그라운드 명령이 끝났다는 신호. 완료 알림
        // (source="stop")과 같은 정산·실황 정리를 하되 알림은 만들지 않는다 —
        // 셸 명령마다 알림 목록이 불어나면 안 되기 때문이다.
        if (e.kind === "idle") {
          const prevTurn = s.timeTracking[e.agentId] ?? initialTurnState();
          const nextTurn = reduceTurn(prevTurn, { kind: "settle", at: e.at });
          logSettledTurn(e.agentId, prevTurn, nextTurn, e.at);
          const timeTracking = { ...s.timeTracking, [e.agentId]: nextTurn };
          const label = s.taskLabels[e.agentId];
          if (!label) return { timeTracking };
          return {
            timeTracking,
            taskLabels: {
              ...s.taskLabels,
              [e.agentId]: {
                ...label,
                latestToolText: undefined,
                latestAssistantText: undefined,
                latestToolAt: undefined,
              },
            },
          };
        }
        if (e.kind !== "prompt" && e.kind !== "tool" && e.kind !== "resume") return {};
        const turnKind = e.kind === "resume" ? "tool" : e.kind;
        const prevTurn = s.timeTracking[e.agentId] ?? initialTurnState();
        const nextTurn = reduceTurn(prevTurn, { kind: turnKind, at: e.at });
        logSettledTurn(e.agentId, prevTurn, nextTurn, e.at);
        const timeTracking = { ...s.timeTracking, [e.agentId]: nextTurn };

        // ---- prompt: 라벨 소스 갱신(새 턴 시작 → 턴 중 실황 필드 리셋) ----
        if (e.kind === "prompt") {
          if (!e.text) return { timeTracking }; // text 없는 prompt → 라벨 미변경
          const prev = s.taskLabels[e.agentId];
          let label: AgentTaskLabel;
          if (prev && prev.sessionId === e.sessionId) {
            // 같은 세션 후속 프롬프트: 요청 문장이 의미 있으면 목표 폴백을 갱신하고,
            // 짧은 맞장구성 지시면 직전 폴백을 유지한다. cwd는 오면 갱신, 없으면 유지.
            const cand = requestSentence(e.text);
            label = {
              ...prev,
              latestPromptText: e.text,
              latestPromptAt: e.at,
              goalFallback:
                cand && isMeaningfulGoalFallback(cand) ? cand : prev.goalFallback,
              cwd: e.cwd ?? prev.cwd,
              currentSummary: undefined, // 새 지시 → 재요약 대상
              latestToolText: undefined, // 새 턴 → 이전 턴 실황 제거
              latestAssistantText: undefined,
              latestToolAt: undefined,
            };
          } else {
            // 새 세션(또는 첫 이벤트): 목표 포함 전체 리셋. 폴백은 항상 설정.
            label = {
              sessionId: e.sessionId,
              firstPromptText: e.text,
              latestPromptText: e.text,
              latestPromptAt: e.at,
              goalFallback: requestSentence(e.text),
              cwd: e.cwd,
            };
          }
          return { timeTracking, taskLabels: { ...s.taskLabels, [e.agentId]: label } };
        }

        // ---- tool: 턴 중 실황(도구 요약/assistant 내레이션) ----
        // 프롬프트 없이 tool만 온 세션은 라벨을 만들지 않고, 세션 불일치는 무시한다.
        if (e.kind === "tool") {
          const prev = s.taskLabels[e.agentId];
          if (!prev || prev.sessionId !== e.sessionId) return { timeTracking };
          const patch: Partial<AgentTaskLabel> = {};
          // assistant 내레이션은 러스트 5초 스로틀이 이미 적용됨 → 오면 항상 반영.
          if (e.assistantText) patch.latestAssistantText = e.assistantText;
          // 도구 요약은 2초 스로틀 + 동일 텍스트 스킵(불필요 리렌더 방지).
          if (
            e.text &&
            e.text !== prev.latestToolText &&
            e.at - (prev.latestToolAt ?? 0) >= TOOL_LABEL_MIN_INTERVAL_MS
          ) {
            patch.latestToolText = e.text;
            patch.latestToolAt = e.at;
          }
          if (Object.keys(patch).length === 0) return { timeTracking }; // 갱신 없음
          return { timeTracking, taskLabels: { ...s.taskLabels, [e.agentId]: { ...prev, ...patch } } };
        }

        // ---- resume: 시간 추적만, 라벨 비대상 ----
        return { timeTracking };
      }),

    setTaskLabelSummary: (agentId, patch) =>
      set((s) => {
        const prev = s.taskLabels[agentId];
        if (!prev) return s;
        return { taskLabels: { ...s.taskLabels, [agentId]: { ...prev, ...patch } } };
      }),

    setGitBranches: (next) => set({ gitBranches: next }),

    applyNotificationTiming: (e) =>
      set((s) => {
        // stop → 턴 종료, hook/bell → 대기 시작. (source는 이 셋뿐.)
        const kind: TurnInput["kind"] = e.source === "stop" ? "stop" : "notification";
        const prev = s.timeTracking[e.agentId] ?? initialTurnState();
        const next = reduceTurn(prev, { kind, at: e.at });
        logSettledTurn(e.agentId, prev, next, e.at);
        const timeTracking = { ...s.timeTracking, [e.agentId]: next };
        // 완료(stop) → 라벨의 턴 중 실황을 지운다(idle에서 마지막 도구 잔존 방지).
        if (e.source === "stop") {
          const label = s.taskLabels[e.agentId];
          if (label) {
            return {
              timeTracking,
              taskLabels: {
                ...s.taskLabels,
                [e.agentId]: {
                  ...label,
                  latestToolText: undefined,
                  latestAssistantText: undefined,
                  latestToolAt: undefined,
                },
              },
            };
          }
        }
        return { timeTracking };
      }),

    applySessionTiming: (agentId, state, at) =>
      set((s) => {
        // 세션 종료만 강제 정산 대상. 그 외 상태 전이는 턴 집계와 무관.
        if (state !== "exited" && state !== "disposed") return s;
        const prev = s.timeTracking[agentId] ?? initialTurnState();
        const next = reduceTurn(prev, { kind: "settle", at });
        logSettledTurn(agentId, prev, next, at);
        return { timeTracking: { ...s.timeTracking, [agentId]: next } };
      }),

    noteUsageSession: (agentId, sessionId) =>
      set((s) => {
        const prev = s.sessionUsage[agentId];
        if (prev && prev.sessionId === sessionId) return s; // 같은 세션 — no-op(같은 참조).
        return {
          sessionUsage: { ...s.sessionUsage, [agentId]: { sessionId, totals: emptyTotals() } },
        };
      }),

    applyTurnUsage: (e: TurnUsageEvent) =>
      set((s) => {
        // 시드가 이미 이 턴을 포함한다 — 실시간으로 다시 더하면 이중 계산.
        if (s.sessionUsageSeed && e.at <= s.sessionUsageSeed.at) return s;
        const prev = s.sessionUsage[e.agentId];
        const entry =
          prev && prev.sessionId === e.sessionId ? prev : { sessionId: e.sessionId, totals: emptyTotals() };
        return {
          sessionUsage: {
            ...s.sessionUsage,
            [e.agentId]: { sessionId: e.sessionId, totals: addTurn(entry.totals, e.tokens, e.partial) },
          },
          // 실시간이 실제로 반영한 첫 턴의 시각만 기록(이미 있으면 유지) — B의
          // 시드 컷오프 기준. 여기서 무시되고 return s로 빠진 사용량 이벤트(위
          // 시드 컷오프 가드)는 반영이 아니므로 잡지 않는다.
          sessionUsageFirstAt: s.sessionUsageFirstAt ?? e.at,
        };
      }),

    setSessionUsageSeed: (seed) =>
      set((s) => (s.sessionUsageSeed ? s : { sessionUsageSeed: seed })),

    setTodayWorkedBase: (baseMs, baselineMs) =>
      set({ todayWorkedBaseMs: baseMs, memoryWorkedBaselineMs: baselineMs }),

    hydrate: (state) =>
      set(() => {
        const agents: Record<string, AgentProfile> = {};
        const sessions: Record<string, SessionRuntime> = {};
        for (const a0 of state.agents) {
          // 레거시(archetype 부재) 프로필은 human으로 백필 — 외형 불변 보장.
          const a = a0.archetype === undefined ? { ...a0, archetype: "human" } : a0;
          agents[a.id] = a;
          sessions[a.id] = {
            agentId: a.id,
            status: "idle",
            cols: 80,
            rows: 24,
            lastActivityAt: a.createdAt,
          };
        }
        return {
          agents,
          sessions,
          agentOrder: state.agents.map((a) => a.id),
          vacationMode: state.vacationMode ?? false,
        };
      }),

    setUsage: (snapshot) => set({ usage: snapshot }),

    hydrateSettings: (settings, firstRun) =>
      set({ appSettings: settings, settingsFirstRun: firstRun, settingsHydrated: true }),

    updateAppSettings: (patch) => {
      const next = { ...get().appSettings, ...patch };
      set({ appSettings: next });
      // fire-and-forget: 저장 실패는 콘솔 경고로만(다음 부팅 때 이전 값 복원됨).
      void tauriApi.setAppSettings(next).catch((err) => console.warn("settings: save failed", err));
    },

    completeFirstRun: (choice) => {
      get().updateAppSettings(choice);
      set({ settingsFirstRun: false });
    },
  }))
);
