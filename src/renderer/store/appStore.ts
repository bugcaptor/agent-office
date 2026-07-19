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
} from "./types";
import { initialTurnState, reduceTurn } from "../timeline/turnReducer";
import type { AgentTurnState, TurnInput } from "../timeline/turnReducer";
import { requestSentence } from "../labels/labelText";
import { applyTheme, loadStoredThemeId } from "../theme/applyTheme";
import type { ThemeId } from "../theme/themes";
import type { ActivityEvent, AppSettings, SessionState, UsageSnapshot } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";

const MAX_EXCERPT = 80;
/** 도구 요약 라벨 갱신 최소 간격(ms). 도구가 빠르게 연달아 와도 라벨이 튀지 않게 스로틀. */
const TOOL_LABEL_MIN_INTERVAL_MS = 2000;
/** goalFallback 갱신 최소 문자 수 — 이보다 짧은 요청 문장은 목적을 담기 어렵다(이슈 #44). */
const GOAL_FALLBACK_MIN_CHARS = 6;
/** 맞장구성 지시 판정: 이 토큰으로 "시작"하고 뒤에 공백·부호가 오거나 그 자체로
 * 끝날 때만. "네트워크"류 오탐을 막기 위해 토큰 경계를 요구한다(이슈 #44 작업 A). */
const BACKCHANNEL_START = /^(응|네|넵|예|그래|좋아|오케이|오케|ㅇㅋ|알겠|고마|감사)(?=[\s,.!?~…]|$)/;

/** 요청 문장이 목표 폴백을 갱신할 만한가 — 충분히 길고 맞장구성이 아니어야 한다. */
function isMeaningfulGoalFallback(cand: string): boolean {
  return Array.from(cand).length >= GOAL_FALLBACK_MIN_CHARS && !BACKCHANNEL_START.test(cand);
}
const DEFAULT_APP_SETTINGS: AppSettings = {
  version: 1,
  summarizerEnabled: false,
  summaryProvider: "claude",
  observerEnabled: false,
  soundEnabled: true,
  soundVolume: 0.5,
  externalTerminal: "terminal",
  externalEditor: "system",
  attentionHoldMs: 5000,
  gitStatusEnabled: true,
  cliEnabled: false,
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
  /** Tauri 창(웹뷰)이 OS 포커스를 가졌는지. 기본 true. sessionBridge의
   * 포커스 추적이 갱신한다(이슈 #39). 비포커스면 터미널이 열려 있어도
   * 알림을 억제하지 않고 OS 데스크탑 알림까지 보낸다. */
  windowFocused: boolean;
  /** Tab strip order (LRU, most-recent first). */
  recentAgentIds: string[];
  modal: ModalState;
  muted: boolean;
  /** 현재 테마 id. localStorage("agent-office.theme")로 영속 — PersistedState 아님. */
  theme: ThemeId;
  /** 초상 dataURL 캐시(agentId -> "data:image/png;base64,..."). 런타임 전용, 영속 안 함. */
  portraits: Record<string, string>;
  /** 커스텀 스프라이트 프리뷰(idle0 확대) dataURL 캐시. 런타임 전용, 영속 안 함. */
  spritePreviews: Record<string, string>;
  /** 에이전트별 턴 집계(메모리 전용, 순수 리듀서 상태). */
  timeTracking: Record<string, AgentTurnState>;
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
   * 터미널 재시작 에폭(agentId -> 정수, 기본 0). TerminalMount의 key에
   * 반영되어, 증가하면 강제 리마운트 -> attach()가 새 xterm을 만든다.
   * 런타임 전용, 영속 안 함(persist.ts는 agents만 저장).
   */
  terminalEpochs: Record<string, number>;
  /** 앱 전역 opt-in 설정. 기본값 전부 OFF — 부트 시 getAppSettings로 하이드레이트. */
  appSettings: AppSettings;
  /** true = settings.json 부재(첫 실행) — 온보딩 다이얼로그 표시 트리거. */
  settingsFirstRun: boolean;
  /** 구독 사용량 스냅샷. null = 아직 로드 전. UsageWidget이 60초 폴링으로 채운다.
   * 런타임 전용(비영속). */
  usage: UsageSnapshot | null;

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

  // ---- session actions ----
  setSessionState(e: { agentId: string; status: SessionStatus }): void;
  setSessionSize(agentId: string, cols: number, rows: number): void;

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

  // ---- theme ----
  /** 테마 전환: DOM 적용(applyTheme) + localStorage 영속 + 상태 갱신. */
  setTheme(id: ThemeId): void;

  // ---- time tracking (feeds turnReducer) ----
  applyActivityEvent(e: ActivityEvent): void;
  applyNotificationTiming(e: NotificationEvent): void;
  applySessionTiming(agentId: string, state: SessionState, at: number): void;
  /**
   * "오늘" 헤드라인 베이스+기준선을 함께 세팅. 부팅 시 `(base, 0)`,
   * 로컬 자정 리셋 시 `(0, 현재 Σ메모리 workedMs)`.
   */
  setTodayWorkedBase(baseMs: number, baselineMs: number): void;

  // ---- overhead task label ----
  setTaskLabelSummary(agentId: string, patch: { goal?: string; currentSummary?: string }): void;

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
        | "summarizerEnabled"
        | "summaryProvider"
        | "observerEnabled"
        | "soundEnabled"
        | "soundVolume"
        | "externalTerminal"
        | "externalEditor"
        | "attentionHoldMs"
        | "gitStatusEnabled"
        | "cliEnabled"
      >
    >,
  ): void;
  /** 첫 실행 온보딩 선택 저장 + firstRun 종료. */
  completeFirstRun(
    choice: Pick<AppSettings, "summarizerEnabled" | "summaryProvider" | "observerEnabled">,
  ): void;
}

export const useAppStore = create<AppState>()(
  subscribeWithSelector((set, get) => ({
    agents: {},
    agentOrder: [],
    sessions: {},
    notifications: [],
    activeTerminalAgentId: null,
    windowFocused: true,
    recentAgentIds: [],
    modal: { kind: "none" },
    muted: false,
    theme: loadStoredThemeId(), // 스토어 생성 시점(첫 render 전)에 저장값 복원 → 플래시 없음
    portraits: {},
    spritePreviews: {},
    timeTracking: {},
    todayWorkedBaseMs: 0,
    memoryWorkedBaselineMs: 0,
    taskLabels: {},
    terminalEpochs: {},
    appSettings: DEFAULT_APP_SETTINGS,
    settingsFirstRun: false,
    usage: null,

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
        const timeTracking = { ...s.timeTracking };
        delete timeTracking[agentId];
        const taskLabels = { ...s.taskLabels };
        delete taskLabels[agentId];
        const terminalEpochs = { ...s.terminalEpochs };
        delete terminalEpochs[agentId];
        return {
          agents,
          sessions,
          portraits,
          spritePreviews,
          timeTracking,
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

    setSessionState: ({ agentId, status }) =>
      set((s) => {
        const prev = s.sessions[agentId];
        if (!prev) return s;
        return {
          sessions: {
            ...s.sessions,
            [agentId]: { ...prev, status, lastActivityAt: Date.now() },
          },
        };
      }),

    setSessionSize: (agentId, cols, rows) =>
      set((s) => {
        const prev = s.sessions[agentId];
        if (!prev || (prev.cols === cols && prev.rows === rows)) return s;
        return { sessions: { ...s.sessions, [agentId]: { ...prev, cols, rows } } };
      }),

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

    setTheme: (id) => {
      // 부수효과(DOM/localStorage)를 액션에서 직접 수행 — 이 스토어는 React
      // 밖(IPC 콜백 등)에서도 호출되므로 별도 구독자 계층을 두지 않는다.
      applyTheme(id);
      set({ theme: id });
    },

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

    applyActivityEvent: (e) =>
      set((s) => {
        // 서브에이전트 카운트 신호는 시간 추적/라벨 대상이 아니다(카운트는
        // sessionBridge가 별도 소유). reduceTurn의 TurnInputKind로 좁히기 위해서도 필요.
        // resume(이슈 #39)은 완료 후 출력 지속 신호 — 턴 목적상 tool과 동일하게
        // 취급해 idle→working으로 복귀시킨다(라벨 갱신 대상은 아니다).
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
        return { agents, sessions, agentOrder: state.agents.map((a) => a.id) };
      }),

    setUsage: (snapshot) => set({ usage: snapshot }),

    hydrateSettings: (settings, firstRun) => set({ appSettings: settings, settingsFirstRun: firstRun }),

    updateAppSettings: (patch) => {
      const next = { ...get().appSettings, ...patch };
      set({ appSettings: next });
      // fire-and-forget: 저장 실패는 콘솔 경고로만(다음 부팅 때 이전 값 복원됨).
      void tauriApi.setAppSettings(next).catch((err) => console.warn("settings: 저장 실패", err));
    },

    completeFirstRun: (choice) => {
      get().updateAppSettings(choice);
      set({ settingsFirstRun: false });
    },
  }))
);
