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
import { applyTheme, loadStoredThemeId } from "../theme/applyTheme";
import type { ThemeId } from "../theme/themes";
import type { ActivityEvent, AppSettings, SessionState } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";

const MAX_EXCERPT = 80;

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

  // ---- overhead task label ----
  setTaskLabelSummary(agentId: string, patch: { goal?: string; currentSummary?: string }): void;

  // ---- persistence hydration ----
  hydrate(state: PersistedState): void;

  // ---- app settings ----
  /** 부트 시 백엔드 getAppSettings 결과 반영. */
  hydrateSettings(settings: AppSettings, firstRun: boolean): void;
  /** 스토어 갱신 + 백엔드 저장(fire-and-forget). */
  updateAppSettings(patch: Partial<Pick<AppSettings, "claudeCliEnabled" | "claudeHooksEnabled">>): void;
  /** 첫 실행 온보딩 선택 저장 + firstRun 종료. */
  completeFirstRun(choice: { claudeCliEnabled: boolean; claudeHooksEnabled: boolean }): void;
}

export const useAppStore = create<AppState>()(
  subscribeWithSelector((set, get) => ({
    agents: {},
    agentOrder: [],
    sessions: {},
    notifications: [],
    activeTerminalAgentId: null,
    recentAgentIds: [],
    modal: { kind: "none" },
    muted: false,
    theme: loadStoredThemeId(), // 스토어 생성 시점(첫 render 전)에 저장값 복원 → 플래시 없음
    portraits: {},
    spritePreviews: {},
    timeTracking: {},
    taskLabels: {},
    terminalEpochs: {},
    appSettings: { version: 1, claudeCliEnabled: false, claudeHooksEnabled: false },
    settingsFirstRun: false,

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
        return { agents: { ...s.agents, [agentId]: rest as typeof agent } };
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

    pushNotification: (e) =>
      set((s) => {
        // Suppress: the active terminal is already showing this agent.
        if (s.activeTerminalAgentId === e.agentId) return s;
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
        const prevTurn = s.timeTracking[e.agentId] ?? initialTurnState();
        const nextTurn = reduceTurn(prevTurn, { kind: e.kind, at: e.at });
        logSettledTurn(e.agentId, prevTurn, nextTurn, e.at);
        const timeTracking = { ...s.timeTracking, [e.agentId]: nextTurn };
        // 라벨 소스: text 실린 prompt만 반영. tool/text 없음 → 통과.
        if (e.kind !== "prompt" || !e.text) return { timeTracking };
        const prev = s.taskLabels[e.agentId];
        const label: AgentTaskLabel =
          prev && prev.sessionId === e.sessionId
            ? {
                ...prev,
                latestPromptText: e.text,
                latestPromptAt: e.at,
                currentSummary: undefined, // 새 지시 → 재요약 대상
              }
            : {
                // 새 세션(또는 첫 이벤트): 목표 포함 전체 리셋
                sessionId: e.sessionId,
                firstPromptText: e.text,
                latestPromptText: e.text,
                latestPromptAt: e.at,
              };
        return { timeTracking, taskLabels: { ...s.taskLabels, [e.agentId]: label } };
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
        return { timeTracking: { ...s.timeTracking, [e.agentId]: next } };
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
