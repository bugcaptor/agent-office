# 서브시스템 C 상세 설계 — Renderer UI / 상태관리 / TerminalHost

상태: 정본 — 2026-07-20 현행화. 이슈별 증분(§10 등)으로 갱신되어 온 문서라 본문 대부분이 유효하다. 구현 태스크 분해(구 §8)만 제거했고, 절 번호는 외부 참조 보존을 위해 유지(§8 결번). 초기 절의 개별 파일 경로가 코드와 어긋나면 코드가 정본 — 현재 폴더 지도는 §0.5.

> 설계: Opus 하위 설계 / 주요 판단: Fable. 계약 정합화 결과는 마스터 플랜(이력)의 "계약 정합화" 절이 우선했다.
> **정합화 반영 사항**:
> 1. `window.api` 표면은 마스터 플랜의 정식 정의(`AgentOfficeApi`)를 따른다(아래 §0은 그 반영본). `onData`는 unsubscribe 함수를 반환한다(A에 반영됨). **Tauri v2 개정**: 이 API의 공급 주체는 Electron preload가 아니라 `src/renderer/ipc/tauriApi.ts` 어댑터(@tauri-apps/api invoke/Channel/listen — 서브시스템 A 문서 §3.7)다. 이 문서의 `window.api.*` 호출은 모두 `tauriApi` 참조로 읽는다(전역 주입 또는 모듈 import — 구현 시 모듈 import 채택).
> 2. `SessionStatus`는 `'idle'|'starting'|'running'|'exited'|'disposed'` — `needs_input`은 세션 상태가 아니라 알림 유무에서 파생한다. `error`는 `exited`(intentional=false)로 흡수.
> 3. B와의 연동은 `sceneRef` 직접 호출이 아니라 **`OfficeBus` 구현 + `<OfficeCanvas profiles>` prop**으로 한다. `scene.addAgent/removeAgent/setPending`은 존재하지 않는다(B의 `syncAgents` diff가 대체).
> 4. 스프라이트 프리뷰는 `import { generateSpritePreview } from '../office/gen/characterFactory'` 순수 함수 직접 호출.
> 5. `NotificationEvent`는 A가 발급한 `id`를 그대로 사용(렌더러에서 재발급 금지 — clear 동기화 때문). `type`은 `source`에서 파생: hook→question, stop→done, bell→info.

agent-office의 렌더러(React 18) 계층 전체 설계다.

---

## 0. 전제 계약 (정합화 반영본)

```ts
// tauriApi 어댑터가 구현하는 AgentOfficeApi 형태 (마스터 플랜 "계약 정합화" 절이 원본)
interface AgentOfficeApi {
  createSession(agentId: string, opts?: { cols?: number; rows?: number; cwd?: string }): Promise<{ sessionId: string; state: string }>;
  disposeSession(agentId: string): Promise<void>;
  writeInput(agentId: string, data: string): void;
  resize(agentId: string, cols: number, rows: number): void;
  clearNotifications(agentId: string, ids?: string[]): void;
  listNotifications(agentId: string): Promise<NotificationEvent[]>;
  loadState(): Promise<PersistedState>;
  saveState(state: PersistedState): Promise<void>;
  setBadgeCount(n: number): void;
  onData(agentId: string, cb: (data: string) => void): () => void;      // unsubscribe 반환
  onSessionState(cb: (e: SessionStateEvent) => void): () => void;
  onNotification(cb: (n: NotificationEvent) => void): () => void;
  onNotificationCleared(cb: (p: { agentId: string; ids: string[] }) => void): () => void;
}
```

---

## 0.5 현재 파일 레이아웃 (2026-07-20)

`src/renderer/`는 24개 기능 폴더로 모듈화돼 있다(구조 건강 — REBUILD-PLAN §2.5 진단).
계약 타입은 `src/shared/types/{common,session,notification,bot,profile,diary,memo,usage,settings,markdown,git,api}.ts` 분할 배럴(`shared/types.ts`는 재수출), 커맨드명은 `src/shared/ipc.ts`.

| 폴더 | 담당 (정본 절/문서) |
|---|---|
| `ipc/` | `tauriApi.ts` 어댑터·`sessionBridge`·windowFocus·osNotify (A §2) |
| `store/` | zustand `appStore`·selectors·persist (§2) |
| `terminal/` | TerminalRegistry/Host/Overlay·AgentTabStrip·BotOverlay·botGuard (§3, 봇은 bot-mode 문서) |
| `office/` | PixiJS 씬 — 서브시스템 B 문서 |
| `layout/` | TopBar/BottomBar/UIChrome (§1) |
| `notification/` | 알림 티커 (§5) |
| `profile/` | 프로필 생성 플로우 (§4) |
| `theme/`, `styles/` | 픽셀 테마·토큰 (§6) |
| `workdir/`, `markdown/` | 작업 폴더 팔레트·문서 뷰어 (§10) |
| `analytics/` | 활동 분석 패널 (session-analytics 문서) |
| `usage/` | 사용량 위젯·다이얼로그 (usage-design 문서) |
| `timeline/` | turnReducer·SessionTimePanel |
| `diary/`, `labels/`, `desk/`, `agent/`, `portrait/`, `sprite/` | 일기·라벨·책상·캐릭터 편집·초상·스프라이트 편집 |
| `memo/` | 에이전트별 포스트잇 메모 — 위젯·아카이브 다이얼로그 (§13) |
| `settings/`, `about/`, `sound/`, `ui/` | 설정(CLI 제어 승인 UI 포함)·정보·사운드·공용 위젯(ContextMenu 등) |

## 1. 앱 레이아웃 & 윈도우 설계

### 1.1 결론: 단일 BrowserWindow + z-index 레이어 스택

> 예외(이슈 #72): 앱에는 이제 두 번째 웹뷰 창 `mascot`이 있다 — 투명·최상단·
> 장식 없는 데스크톱 마스코트. 이 절의 "단일 창" 전제는 **앱 본체 UI**에 대한
> 것이고, 마스코트는 스토어도 Pixi도 없는 얇은 소비자라 아래 레이어 스택과
> 무관하다. 설계는 `docs/mascot-window-design.md` 정본.

`frame: false` 커스텀 프레임은 MVP 범위를 넘으므로 표준 프레임 유지. 창 내부는 **4개 레이어의 절대 위치 스택**:

```
Layer 0 (z:0)   OfficeCanvas       — PixiJS canvas, 창 전체 채움 (position:absolute inset:0)
Layer 1 (z:10)  UI Chrome          — TopBar, BottomBar, NotificationTicker (pointer-events 선택적)
Layer 2 (z:20)  TerminalOverlay    — 활성 터미널 패널 (활성 시에만 표시)
Layer 3 (z:30)  ModalRoot          — 프로필 생성/편집 다이얼로그
```

핵심 포인트: **Layer 1의 UI Chrome 컨테이너는 `pointer-events: none`**, 실제 인터랙티브 자식(버튼, 티커 아이템)만 `pointer-events: auto`. 이렇게 해야 캔버스 위 빈 영역 클릭이 PixiJS로 전달되어 캐릭터 클릭이 동작한다.

### 1.2 터미널 패널: 중앙 오버레이 패널 (~72%) 채택

**드로어(right-docked) vs 중앙 오버레이** 중 **중앙 오버레이 채택.** 근거:

- 에이전트가 많아졌을 때 **빠른 문맥 전환**이 핵심 UX인데, 오버레이는 화면 중앙에 크게 떠서 "현재 이 에이전트에 집중" 감각을 준다(타이쿤 게임의 유닛 상세 팝업 메타포와 일치).
- 오버레이는 `xterm`에 넓은 고정 폭을 주어 리사이즈 빈도를 줄인다. 드로어는 폭이 좁고 열고 닫을 때마다 cols가 바뀌어 PTY resize가 잦다.
- 배경 오피스가 살짝 보이며 dim 처리되어, 게임 팝업 인터랙션이 자연스럽다.

단, **오직 하나의 오버레이만 보이되 N개의 xterm은 모두 마운트 유지**(3장 참조). 오버레이는 "뷰포트"고, 그 안에서 `activeTerminalAgentId`에 해당하는 TerminalMount만 `display:block`, 나머지는 `display:none`.

빠른 전환 지원:
- 오버레이 상단에 **AgentTabStrip**(열려있는/최근 에이전트 탭) — 클릭 시 오버레이 유지한 채 active만 스위치(리마운트 없음, 즉시 전환).
- 단축키 `Cmd/Ctrl+1..9`로 탭 전환, 닫기는 헤더 X 버튼과 `Cmd+W`.

### 1.3 상/하 바

- **TopBar** (높이 40px): 좌측 앱 타이틀 로고, 우측 에이전트 수 배지(`{n} agents`), 실행중/대기중 카운트.
- **BottomBar** (높이 48px): 좌측 큰 **"＋ New Agent"** 픽셀 버튼, 중앙 상태 텍스트(예: "3 running · 1 needs input"), 우측 뮤트/설정 아이콘.

### 1.4 우측 알림 티커 컬럼

- 창 우측 가장자리에 고정 폭 컬럼(width: 260px), `top: TopBar 아래 ~ bottom: BottomBar 위`.
- `pointer-events: none` 컨테이너 + 각 카드 `pointer-events: auto`.
- **최신이 위(top)**, 5장까지 표시, 초과분은 "+N more" 요약 카드.

### 1.5 레이아웃 컴포넌트 트리

```
<App>
  <OfficeCanvas bus={officeBus} profiles={agentList} />   // z:0, PixiJS mount (B 소유)
  <UIChrome>                            // z:10, pointer-events:none
    <TopBar />
    <NotificationTicker />
    <BottomBar />
  </UIChrome>
  <TerminalOverlay>                     // z:20, 닫힘 = display:none (언마운트 금지!)
    <AgentTabStrip />
    <TerminalHost />                    // N개 TerminalMount 보관 (항상 마운트)
  </TerminalOverlay>
  <ModalRoot>                           // z:30
    <ProfileDialog />                   // open 시에만
  </ModalRoot>
</App>
```

주의: `TerminalHost`는 **오버레이가 닫혀도 언마운트되면 안 된다.** 오버레이의 "닫힘"은 `TerminalOverlay` 루트에 `display:none`을 주는 방식으로 구현한다(조건부 `{open && ...}` 렌더링 금지). 이게 keep-alive의 최상위 보장이다.

---

## 2. 상태관리 — zustand 채택

### 2.1 근거

- 세션 이벤트(`onData`, `onNotification`, `onSessionState`)는 **React 트리 바깥의 IPC 콜백**에서 발생한다. zustand는 `store.getState()/setState()`를 컴포넌트 밖에서 자유롭게 호출할 수 있어 IPC 브리지 계층이 훅에 의존하지 않는다.
- selector 기반 구독으로 "티커만 리렌더", "탭스트립만 리렌더" 같은 세밀한 최적화가 쉽다.
- `zustand/middleware`의 `subscribeWithSelector`로 상태 변경 부수효과(배지, OfficeBus 중계)를 걸 수 있다.

### 2.2 스토어 타입 정의

`src/renderer/store/types.ts`

> (구현 노트: 아래 스케치는 `AgentProfile`/`PersistedState`/`SessionStatus`/
> `NotificationEvent`·`NotificationType`/`notificationType()`을 이 파일에서
> 다시 선언한다 — 하지만 이 타입들은 이미 `src/shared/types.ts`(서브시스템 A
> 소유, 렌더러<->백엔드 wire 계약의 단일 정의처)에 얼어붙어(frozen) 있다. 여기서
> 좁은 사본을 재선언하면 스토어의 `AgentProfile` 같은 타입이 wire 계약에서 조용히
> 어긋날 수 있다(R5). 실제 구현은 이 타입들을 `@shared/types`에서 import해
> re-export만 하고, 스토어에만 있는 타입만 이 파일에서 새로 정의한다. 또한 세션
> 로컬 런타임 타입은 백엔드 라이프사이클 타입인 공유 `SessionState`(유니온)과
> 이름이 충돌하지 않도록 `SessionRuntime`으로 명명한다.)

```ts
// src/renderer/store/types.ts (실제 구현 반영)
import type {
  AgentProfile,
  NotificationEvent,
  NotificationType,
  PersistedState,
  SessionStatus,
} from '@shared/types';
import { notificationType } from '@shared/types';

export type { AgentProfile, NotificationEvent, NotificationType, PersistedState, SessionStatus };
export { notificationType };

/** 스토어 전용 세션 런타임 상태. wire 계약이 아니며 (cols/rows/lastActivityAt은 UI 전용),
 *  공유 `SessionState`(백엔드 라이프사이클 유니온)과의 이름 충돌을 피하기 위해
 *  `SessionRuntime`으로 명명. */
export interface SessionRuntime {
  agentId: string;
  status: SessionStatus;
  cols: number;
  rows: number;
  lastActivityAt: number;
}

/** 스토어 표시용 알림: 백엔드 `NotificationEvent`에서 `notificationType()`으로 파생,
 *  표시용 축약 `excerpt` 추가. */
export interface Notification {
  id: string;                 // A가 발급한 id 그대로 (clear 동기화)
  agentId: string;
  type: NotificationType;
  message: string;            // 원문
  excerpt: string;            // 표시용 축약(<=80자)
  createdAt: number;
}

export type ModalState =
  | { kind: 'none' }
  | { kind: 'profile-create' }
  | { kind: 'profile-edit'; agentId: string };
```

`src/renderer/store/appStore.ts`

```ts
import { create } from 'zustand';
import { subscribeWithSelector } from 'zustand/middleware';
import type {
  AgentProfile, SessionRuntime, SessionStatus, Notification,
  NotificationEvent, ModalState, PersistedState,
} from './types';
import { notificationType } from './types';

const MAX_EXCERPT = 80;

interface AppState {
  // ---- data ----
  agents: Record<string, AgentProfile>;      // agentId -> profile
  agentOrder: string[];                       // 생성 순서 (탭/카운트용)
  sessions: Record<string, SessionRuntime>;   // agentId -> session (SessionRuntime — 공유 SessionState와 이름 충돌 방지)
  notifications: Notification[];              // 전역 큐, createdAt desc 정렬 유지
  activeTerminalAgentId: string | null;       // null = 오버레이 닫힘
  recentAgentIds: string[];                    // 탭스트립 순서 (LRU, 최근이 앞)
  modal: ModalState;
  muted: boolean;

  // ---- profile actions ----
  addAgent(profile: AgentProfile): void;
  updateAgent(agentId: string, patch: Partial<AgentProfile>): void;
  removeAgent(agentId: string): void;

  // ---- session actions ----
  setSessionState(e: { agentId: string; status: SessionStatus }): void;
  setSessionSize(agentId: string, cols: number, rows: number): void;

  // ---- notification actions ----
  pushNotification(e: NotificationEvent): void;
  clearNotificationsFor(agentId: string): void;
  clearNotificationByIds(agentId: string, ids: string[]): void;

  // ---- terminal overlay ----
  openTerminal(agentId: string): void;   // active 설정 + recent 갱신 + 알림 클리어
  closeTerminal(): void;                  // 오버레이 닫기 (세션 유지)

  // ---- modal ----
  openModal(modal: ModalState): void;
  closeModal(): void;

  // ---- persistence hydration ----
  hydrate(state: PersistedState): void;
}

export const useAppStore = create<AppState>()(
  subscribeWithSelector((set, get) => ({
    agents: {},
    agentOrder: [],
    sessions: {},
    notifications: [],
    activeTerminalAgentId: null,
    recentAgentIds: [],
    modal: { kind: 'none' },
    muted: false,

    addAgent: (profile) =>
      set((s) => ({
        agents: { ...s.agents, [profile.id]: profile },
        agentOrder: [...s.agentOrder, profile.id],
        sessions: {
          ...s.sessions,
          [profile.id]: {
            agentId: profile.id, status: 'starting',
            cols: 80, rows: 24, lastActivityAt: Date.now(),
          },
        },
      })),

    updateAgent: (agentId, patch) =>
      set((s) =>
        s.agents[agentId]
          ? { agents: { ...s.agents, [agentId]: { ...s.agents[agentId], ...patch } } }
          : s
      ),

    removeAgent: (agentId) =>
      set((s) => {
        const agents = { ...s.agents }; delete agents[agentId];
        const sessions = { ...s.sessions }; delete sessions[agentId];
        return {
          agents,
          sessions,
          agentOrder: s.agentOrder.filter((id) => id !== agentId),
          recentAgentIds: s.recentAgentIds.filter((id) => id !== agentId),
          notifications: s.notifications.filter((n) => n.agentId !== agentId),
          activeTerminalAgentId:
            s.activeTerminalAgentId === agentId ? null : s.activeTerminalAgentId,
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

    pushNotification: (e) =>
      set((s) => {
        // active 터미널이 이미 이 에이전트를 보고 있으면 알림 억제 —
        // 단 창이 포커스일 때만(이슈 #39). 앱이 백그라운드면 터미널이 열려
        // 있어도 티커/배지/사운드로 노출한다(창 포커스는 `windowFocused`,
        // `installWindowFocusTracking`가 갱신). OS 데스크탑 알림 발송은
        // sessionBridge의 onNotification이 `!windowFocused`일 때만 수행한다.
        if (s.activeTerminalAgentId === e.agentId && s.windowFocused) return s;
        const n: Notification = {
          id: e.id,                       // 정합화: A의 id 그대로
          agentId: e.agentId,
          type: notificationType(e.source),
          message: e.message,
          excerpt: e.message.length > MAX_EXCERPT
            ? e.message.slice(0, MAX_EXCERPT - 1) + '…'
            : e.message,
          createdAt: e.at,
        };
        return { notifications: [n, ...s.notifications] };  // 최신이 앞
      }),

    clearNotificationsFor: (agentId) =>
      set((s) => ({ notifications: s.notifications.filter((n) => n.agentId !== agentId) })),

    clearNotificationByIds: (agentId, ids) =>
      set((s) => {
        const drop = new Set(ids);
        return { notifications: s.notifications.filter((n) => n.agentId !== agentId || !drop.has(n.id)) };
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

    openModal: (modal) => set({ modal }),
    closeModal: () => set({ modal: { kind: 'none' } }),

    hydrate: (state) =>
      set(() => {
        const agents: Record<string, AgentProfile> = {};
        const sessions: Record<string, SessionRuntime> = {};
        for (const a of state.agents) {
          agents[a.id] = a;
          sessions[a.id] = {
            agentId: a.id, status: 'idle',
            cols: 80, rows: 24, lastActivityAt: a.createdAt,
          };
        }
        return { agents, sessions, agentOrder: state.agents.map((a) => a.id) };
      }),
  }))
);
```

### 2.3 파생 selector 헬퍼

`src/renderer/store/selectors.ts`

```ts
import { useAppStore } from './appStore';
import { useShallow } from 'zustand/react/shallow';

// 주의: 매 렌더 새 배열을 만드는 selector는 useShallow로 감싼다 (무한 리렌더 방지)
export const useAgentList = () =>
  useAppStore(useShallow((s) => s.agentOrder.map((id) => s.agents[id])));

export const useAgentCount = () => useAppStore((s) => s.agentOrder.length);

export const useRunningCount = () =>
  useAppStore((s) =>
    Object.values(s.sessions).filter((x) => x.status === 'running').length
  );

export const usePendingCount = () =>
  useAppStore((s) => new Set(s.notifications.map((n) => n.agentId)).size);

export const useActiveAgentId = () => useAppStore((s) => s.activeTerminalAgentId);
```

### 2.4 IPC ↔ 스토어 브리지 + OfficeBus 구현

`src/renderer/ipc/sessionBridge.ts` — React 밖에서 앱 부트 시 1회 실행. (정합화: OfficeBus를 여기서 스토어 백킹으로 구현해 B에 주입한다. sceneRef 없음.)

```ts
import { useAppStore } from '../store/appStore';
import type { OfficeBus } from '../office/bus';

type NotifCb = (agentId: string, hasPending: boolean) => void;
type StateCb = (agentId: string, state: string) => void;

const notifCbs = new Set<NotifCb>();
const stateCbs = new Set<StateCb>();

/** B(OfficeScene)에 주입할 버스. 스토어를 단일 소스로 사용. */
export const officeBus: OfficeBus = {
  onNotificationChanged(cb) { notifCbs.add(cb); return () => notifCbs.delete(cb); },
  onSessionStateChanged(cb) { stateCbs.add(cb); return () => stateCbs.delete(cb); },
  emitAgentClicked(agentId) {
    const s = useAppStore.getState();
    s.openTerminal(agentId);
    window.api.clearNotifications(agentId);
  },
};

export function installSessionBridge(): () => void {
  const store = useAppStore.getState;

  const offState = window.api.onSessionState((e) => {
    store().setSessionState({ agentId: e.agentId, status: e.state as any });
    stateCbs.forEach((cb) => cb(e.agentId, e.state));
  });

  const offNotif = window.api.onNotification((e) => {
    store().pushNotification(e);
  });

  const offCleared = window.api.onNotificationCleared(({ agentId, ids }) => {
    store().clearNotificationByIds(agentId, ids);
  });

  // notifications 변경 → hasPending 플래그 중계 + 배지 동기화
  const offPending = useAppStore.subscribe(
    (s) => s.notifications,
    (notifications) => {
      const pending = new Set(notifications.map((n) => n.agentId));
      for (const id of Object.keys(useAppStore.getState().agents)) {
        notifCbs.forEach((cb) => cb(id, pending.has(id)));
      }
      if (!useAppStore.getState().muted) window.api.setBadgeCount(pending.size);
    }
  );

  return () => { offState(); offNotif(); offCleared(); offPending(); };
}
```

`onData`는 스토어를 거치지 않고 **각 TerminalMount가 직접 구독**한다(고빈도 스트림을 스토어에 넣으면 리렌더 폭발). 3장 참조.

---

## 3. TerminalHost — N개 xterm keep-alive

### 3.1 설계 원칙

1. 에이전트마다 정확히 하나의 `Terminal` 인스턴스. 생성은 `createSession` 성공 후 최초 1회, **hide 시 절대 dispose 안 함**.
2. 모든 `TerminalMount`는 항상 DOM에 존재. 비활성은 `display:none`.
3. show될 때만 `fit()` 실행 + PTY `resize()` 송신 + `focus()`.

### 3.2 display:none vs visibility 트레이드오프

- `display:none`: 숨은 동안 컨테이너 크기가 0 → 그 상태에서 `fit()` 호출하면 cols/rows가 비정상. 그래서 **fit은 반드시 show된 직후(display 복원 후) 다음 프레임에** 실행한다.
- 채택: `display:none` + **show 시 `requestAnimationFrame`으로 레이아웃 확정 후 fit**. 숨은 터미널이 레이아웃 비용을 아예 안 내는 게 이득. 스크롤백/버퍼는 xterm 내부에 유지되므로 display 토글로 소실되지 않는다.

### 3.3 TerminalRegistry (마운트 레지스트리)

`src/renderer/terminal/TerminalRegistry.ts`

```ts
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { resolveXtermTheme } from './theme';   // 팔레트는 테마 레지스트리에서 (§3.6)

interface Entry {
  term: Terminal;
  fit: FitAddon;
  disposeData: () => void;   // onData unsubscribe
  container: HTMLDivElement; // TerminalMount가 붙이는 실제 DOM
  opened: boolean;           // term.open() 호출됨?
}

class TerminalRegistry {
  private entries = new Map<string, Entry>();

  /** 세션 최초 오픈 시 1회. 이미 있으면 기존 반환(keep-alive 보장). */
  ensure(agentId: string): Entry {
    let e = this.entries.get(agentId);
    if (e) return e;

    const term = new Terminal({
      theme: this.currentTheme(),   // 현재 유효 팔레트(setTheme으로 갱신) — §3.6
      fontFamily: '"SF Mono", "Menlo", "Consolas", "Liberation Mono", monospace',
      fontSize: 13,
      lineHeight: 1.2,
      cursorBlink: true,
      scrollback: 5000,
      allowProposedApi: true,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);

    // 사용자 입력 -> PTY
    term.onData((data) => window.api.writeInput(agentId, data));

    // PTY 출력 -> 화면 (스토어 우회, 직접 write)
    const disposeData = window.api.onData(agentId, (data) => term.write(data));

    const container = document.createElement('div');
    container.className = 'terminal-mount-inner';

    e = { term, fit, disposeData, container, opened: false };
    this.entries.set(agentId, e);
    return e;
  }

  get(agentId: string): Entry | undefined { return this.entries.get(agentId); }

  /** DOM 노드에 term을 attach (최초 1회 term.open) */
  attach(agentId: string, host: HTMLElement) {
    const e = this.ensure(agentId);
    if (!e.container.isConnected) host.appendChild(e.container);
    if (!e.opened) { e.term.open(e.container); e.opened = true; }
  }

  /** show 시: 레이아웃 확정 후 fit + resize IPC + focus */
  activate(agentId: string, onResize: (cols: number, rows: number) => void) {
    const e = this.entries.get(agentId);
    if (!e || !e.opened) return;
    requestAnimationFrame(() => {
      try {
        e.fit.fit();
        onResize(e.term.cols, e.term.rows);
        e.term.focus();
      } catch { /* container 크기 0 방어 */ }
    });
  }

  /** ResizeObserver 콜백에서 호출 (활성 터미널만) */
  refit(agentId: string, onResize: (cols: number, rows: number) => void) {
    const e = this.entries.get(agentId);
    if (!e || !e.opened) return;
    e.fit.fit();
    onResize(e.term.cols, e.term.rows);
  }

  /** removeAgent 시에만 진짜 폐기 */
  destroy(agentId: string) {
    const e = this.entries.get(agentId);
    if (!e) return;
    e.disposeData();
    e.term.dispose();
    e.container.remove();
    this.entries.delete(agentId);
  }

  has(agentId: string) { return this.entries.has(agentId); }
}

export const terminalRegistry = new TerminalRegistry();
```

### 3.4 TerminalHost + TerminalMount 컴포넌트

`src/renderer/terminal/TerminalHost.tsx`

```tsx
import { useEffect, useRef } from 'react';
import { useAppStore } from '../store/appStore';
import { useShallow } from 'zustand/react/shallow';
import { terminalRegistry } from './TerminalRegistry';

const RESIZE_DEBOUNCE_MS = 120;

export function TerminalHost() {
  // 세션이 생성되어야 하는 모든 에이전트 (starting/running/...)
  const agentIds = useAppStore(
    useShallow((s) => s.agentOrder.filter((id) => s.sessions[id]?.status !== 'idle'))
  );

  return (
    <div className="terminal-host">
      {agentIds.map((id) => (
        <TerminalMount key={id} agentId={id} />
      ))}
    </div>
  );
}

function TerminalMount({ agentId }: { agentId: string }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const isActive = useAppStore((s) => s.activeTerminalAgentId === agentId);
  const setSessionSize = useAppStore((s) => s.setSessionSize);

  // 최초 마운트: registry attach (term.open 1회)
  useEffect(() => {
    if (hostRef.current) terminalRegistry.attach(agentId, hostRef.current);
    // 언마운트해도 destroy 하지 않음 (removeAgent에서만 destroy)
  }, [agentId]);

  // active 전환 시: fit + resize + focus
  useEffect(() => {
    if (!isActive) return;
    terminalRegistry.activate(agentId, (cols, rows) => {
      setSessionSize(agentId, cols, rows);
      window.api.resize(agentId, cols, rows);
    });
  }, [isActive, agentId, setSessionSize]);

  // ResizeObserver (활성일 때만 refit, 디바운스)
  useEffect(() => {
    if (!isActive || !hostRef.current) return;
    let t: ReturnType<typeof setTimeout> | undefined;
    const ro = new ResizeObserver(() => {
      clearTimeout(t);
      t = setTimeout(() => {
        terminalRegistry.refit(agentId, (cols, rows) => {
          setSessionSize(agentId, cols, rows);
          window.api.resize(agentId, cols, rows);
        });
      }, RESIZE_DEBOUNCE_MS);
    });
    ro.observe(hostRef.current);
    return () => { clearTimeout(t); ro.disconnect(); };
  }, [isActive, agentId, setSessionSize]);

  return (
    <div
      ref={hostRef}
      className="terminal-mount"
      style={{ display: isActive ? 'block' : 'none' }}
      data-agent-id={agentId}
    />
  );
}
```

### 3.5 포커스 관리

- `activate()`에서 `term.focus()` — 오버레이 열림 즉시 키 입력 가능.
- 오버레이가 닫히면(`closeTerminal`) 별도 blur 불필요. **Esc는 셸에 전달**(vim 등 TUI 앱에서 필요)하고 오버레이 닫기는 헤더의 X 버튼과 `Cmd+W`로 한다. Esc를 UI 닫기에 쓰면 TUI 앱이 망가진다.

### 3.6 xterm ITheme — 테마 시스템에 편입

터미널 색은 **테마 레지스트리의 일부**다. 초기안의 단일 상수 `XTERM_THEME`(midnight 값)은 폐기하고, 테마마다 전용 xterm 팔레트를 갖는다.

- **실값의 단일 원천**: `src/renderer/theme/themes.ts`의 `THEMES[id].xterm` (`ITheme & { background: string; foreground: string }` — `--term-bg` 주입과 대비 보장을 위해 두 필드는 필수로 좁힌다). midnight의 팔레트가 초기안 `XTERM_THEME` 값 그대로다("green-CRT meets modern dark").
- **해석/영속**: `src/renderer/terminal/theme.ts` — 색 값은 없고 "어느 테마의 팔레트를 쓸 것인가"만 정한다.
  ```ts
  type XtermThemeOverride = ThemeId | 'auto';           // 기본 'auto'
  resolveXtermTheme(themeId, override): ITheme          // auto면 앱 테마를 따름
  xtermBackground(themeId, override): string
  loadStoredXtermThemeOverride() / persistXtermThemeOverride()  // 'agent-office.xterm-theme'
  ```
  스토어/xterm 런타임 의존이 없는 순수 모듈(`ITheme`은 type-only import)이라 `appStore`와 `applyTheme` 양쪽이 안전하게 import한다.
- **스토어**: `xtermTheme: XtermThemeOverride` 슬라이스(기본 `"auto"`) + `setXtermTheme` — 테마와 같은 계층(zustand + localStorage, `PersistedState` 아님)이라 `updateAppSettings`(Rust 영속)가 아니다. 설정 다이얼로그의 "터미널 색상" 셀렉터가 여기에 직접 바인딩된다.
- **라이브 재도색**: `App.tsx`의 효과가 `[theme, xtermTheme]` 변화 시 `terminalRegistry.setTheme(resolveXtermTheme(...))`을 호출한다(Pixi 팔레트 배선과 같은 모양이되, 소비처가 React 트리 밖이라 효과로 밀어 넣는다). `TerminalRegistry.setTheme`은 팔레트를 보관해 이후 생성분에 쓰고, keep-alive 중인 전 인스턴스에 `term.options.theme`을 재대입한다(xterm 5.x는 재대입으로 즉시 다시 그린다 — 숨어 있는 터미널도 함께 갱신되므로 탭을 옮겨도 옛 색이 남지 않는다).
- **DOM 배경**: 유효 팔레트의 `background`를 `documentElement`에 `--term-bg`로 주입한다. 부팅 경로는 `applyTheme()`이 저장된 선택을 읽어 첫 페인트 전에 세팅하고(main.tsx가 render 전 1회 호출), 이후 변경은 `applyTerminalBg()`가 갱신한다. `terminal.css`의 `.terminal-host`는 `background: var(--term-bg, #12131a)`(폴백 = 테마 도입 이전 값).

테마별 팔레트 설계 원칙:

| 테마 | 배경/전경 | ANSI 16색 |
| --- | --- | --- |
| midnight | `#12131a` / `#c8d0e0` | 초기안 값 그대로(룩 보존 계약) |
| daylight | `#fbf6ea` / `#3a3428` | 밝은 배경이라 명도 관계가 뒤집힌다 — 노랑/흰색 계열을 어둡게 보정하고 bright는 더 진하게(대비 강화) |
| sakura | `#fcf0f5` / `#4a2b3c` | daylight와 같은 보정 + 플럼/핑크 쪽으로 기울인 색상 |
| pipboy | `#071209` / `#66ff99` | 인광 초록 모노크롬. **완전 단색화 금지** — 빨강 에러/노랑 경고 구분이 죽는다. 색상(hue) 정체성은 남기고 채도만 낮춰 초록 쪽으로 기울인다 |

레지스트리 무결성은 `theme/__tests__/theme.test.ts`가 전 테마 순회로 검사한다(배경/전경/커서 + ANSI 16색이 전부 `#rrggbb`, 어두운 테마는 배경 < 전경 명도).

---

## 4. 프로필 생성 플로우

### 4.1 랜덤 생성기 — Korean-friendly 워드 리스트

`src/renderer/profile/wordlists.ts`

```ts
// 이름: 픽셀 캐릭터에 어울리는 한국어 별명
export const NAME_WORDS: string[] = [
  '방구석코더', '야근요정', '카페인킴', '버그사냥꾼', '리팩토리',
  '깃발든자', '무한루프', '세미콜론', '널포인터', '스택오버',
  '컴파일러', '핫픽스박', '메모리조', '스레드리', '캐시최',
  '로그남작', '픽셀공주', '터미널곰', '주니어양', '시니어형',
];

export const ROLE_WORDS: string[] = [
  '프론트엔드', '백엔드', '데브옵스', 'QA엔지니어', '풀스택',
  'AI리서처', '데이터분석', '보안담당', 'PM', '테크리드',
  'UX디자이너', 'SRE', '모바일개발', '게임개발', '임베디드',
  '플랫폼', '인프라', 'DBA', 'ML엔지니어', '아키텍트',
];

export const PERSONALITY_WORDS: string[] = [
  '꼼꼼한', '느긋한', '열정적인', '침착한', '엉뚱한',
  '완벽주의', '낙천적인', '분석적인', '창의적인', '집요한',
  '수다스러운', '과묵한', '호기심많은', '신중한', '대담한',
  '유쾌한', '까칠한', '성실한', '즉흥적인', '전략적인',
];
```

`src/renderer/profile/generate.ts`

```ts
import { nanoid } from 'nanoid';
import { NAME_WORDS, ROLE_WORDS, PERSONALITY_WORDS } from './wordlists';
import type { AgentProfile } from '../store/types';

const pick = <T,>(arr: T[]) => arr[Math.floor(Math.random() * arr.length)];

export interface DraftProfile {
  name: string;
  role: string;
  note: string;        // 레거시. 현행은 항상 "" — 아래 4.2.3 참고
  seed: string;
}

export function generateDraft(): DraftProfile {
  const personality = pick(PERSONALITY_WORDS);
  return {
    name: pick(NAME_WORDS),
    role: pick(ROLE_WORDS),
    note: `${personality} 성격`,
    seed: nanoid(8),
  };
}

export function draftToProfile(d: DraftProfile, deskIndex: number): AgentProfile {
  return {
    id: nanoid(),
    name: d.name.trim() || pick(NAME_WORDS),
    role: d.role.trim(),
    note: d.note.trim(),
    seed: d.seed,
    createdAt: Date.now(),
    deskIndex,
  };
}
```

### 4.2 ProfileDialog 컴포넌트

`src/renderer/profile/ProfileDialog.tsx` (핵심 로직 — 정합화: 프리뷰는 순수 함수 직접 호출, 씬 호출 없음)

```tsx
import { useEffect, useState } from 'react';
import { nanoid } from 'nanoid';
import { useAppStore } from '../store/appStore';
import { generateDraft, draftToProfile, type DraftProfile } from './generate';
import { generateSpritePreview } from '../office/gen/characterFactory';

export function ProfileDialog() {
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const addAgent = useAppStore((s) => s.addAgent);
  const updateAgent = useAppStore((s) => s.updateAgent);
  const agentOrder = useAppStore((s) => s.agentOrder);

  const editing = modal.kind === 'profile-edit';
  const editingAgent = useAppStore((s) =>
    modal.kind === 'profile-edit' ? s.agents[modal.agentId] : undefined
  );

  const [draft, setDraft] = useState<DraftProfile>(() => generateDraft());
  const [spriteUrl, setSpriteUrl] = useState<string>('');

  // 편집 모드 진입 시 기존 값 로드
  useEffect(() => {
    if (editingAgent) {
      setDraft({
        name: editingAgent.name, role: editingAgent.role,
        note: editingAgent.note, seed: editingAgent.seed,
      });
    }
  }, [editingAgent]);

  // seed 변경 시 라이브 스프라이트 프리뷰 (B의 순수 함수 — 동기)
  useEffect(() => {
    setSpriteUrl(generateSpritePreview(draft.seed));
  }, [draft.seed]);

  const regenSeed = () => setDraft((d) => ({ ...d, seed: nanoid(8) }));
  const regenAll = () => setDraft(generateDraft());

  const onSave = async () => {
    if (editing && editingAgent) {
      updateAgent(editingAgent.id, {
        name: draft.name, role: draft.role,
        note: draft.note, seed: draft.seed,
      });
    } else {
      const profile = draftToProfile(draft, agentOrder.length);
      addAgent(profile);                                    // status: 'starting'
      // 캐릭터 등장은 profiles prop 변화 → B의 syncAgents가 처리 (정합화)
      await window.api.createSession(profile.id);           // PTY 시작
    }
    closeModal();
  };

  if (modal.kind === 'none') return null;
  return (
    <div className="modal-backdrop" onClick={closeModal}>
      <div className="pixel-panel profile-dialog" onClick={(e) => e.stopPropagation()}>
        <h2 className="pixel-title">{editing ? '에이전트 편집' : '새 에이전트'}</h2>
        <div className="sprite-preview">
          {spriteUrl && <img src={spriteUrl} alt="sprite" width={96} height={96} />}
          <button className="pixel-btn" onClick={regenSeed}>스프라이트 재생성</button>
        </div>
        <label>이름
          <input value={draft.name}
            onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
        </label>
        <label>역할
          <input value={draft.role}
            onChange={(e) => setDraft({ ...draft, role: e.target.value })} />
        </label>
        <label>성격 프롬프트
          <textarea value={draft.personalityPrompt}
            onChange={(e) => setDraft({ ...draft, personalityPrompt: e.target.value })} />
        </label>
        <div className="dialog-actions">
          {!editing && <button className="pixel-btn" onClick={regenAll}>전체 랜덤</button>}
          <button className="pixel-btn primary" onClick={onSave}>저장</button>
          <button className="pixel-btn" onClick={closeModal}>취소</button>
        </div>
      </div>
    </div>
  );
}
```

플로우 요약: `＋New Agent` → `openModal({kind:'profile-create'})` → 다이얼로그가 `generateDraft()`로 초기값 → 사용자 편집/재생성 → `저장`시 `addAgent`(스토어, status=starting) → profiles prop 변화로 B `syncAgents`가 캐릭터 생성 → `createSession`(PTY) → A가 `onSessionState`로 `running` 통지 → BottomBar 카운트 반영.

위 코드는 설계 당시 스케치다. 현행 다이얼로그의 "외형" 섹션은 스프라이트 카드
안에 **미니미(서브에이전트 분신) 소섹션**을 함께 갖는다 — 현재 미니미 프리뷰
(커스텀 없으면 스프라이트 축소판임을 안내) + 업로드/변경/제거. 업로드는
`SpriteEditor`를 `target="minimi"`로 재사용해 크롭/줌 UX를 공유하되 **단일 N×N
프레임**만 저장한다. 저장·로드·삭제·번들 왕복의 전체 계약은
`docs/subsystem-b-office.md` §4.8 표가 정본이다.

#### 4.2.1 "외형" 섹션의 모드 탭 (kbm #2fa)

외형을 **만드는 방법**은 두 갈래인데 예전에는 버튼이 평면으로 늘어서 있어
무엇을 눌러야 할지 알기 어려웠다. 지금은 한 번에 한 쪽만 보여 준다.

- **탭 바깥(항상 보임)**: 스프라이트/미니미/초상 프리뷰 카드, `스프라이트 재생성`,
  `커스텀 제거`·`초상 제거`, 미니미 업로드/변경/제거. 아래쪽의 `외모 힌트`·
  `픽셀아트 의뢰 문구`·`미니미 의뢰 문구` 입력도 두 모드 공통이다(두 경로의
  프롬프트에 함께 반영).
- **`[직접 만들기]` 탭**: `초상 프롬프트 복사` + `이미지 업로드/변경`,
  `픽셀아트 프롬프트 복사` + `픽셀아트 업로드/변경`, `미니미 프롬프트 복사` +
  `미니미 업로드/변경`. 프롬프트를 아무 이미지 생성 도구에나 넣고 결과를 올리는
  경로다.
- **`[Codex로 생성]` 탭** (`src/renderer/portrait/CodexGenPanel.tsx`): 로컬
  `codex` CLI 설치 탐지 한 줄 + `초상 생성`/`스프라이트 생성`/`미니미 생성` +
  진행·결과·오류
  캡션. 패널은 **표시 전용**이고 생성 오케스트레이션(편집 세션 토큰 가드,
  결과를 편집기로 넘기기)은 `ProfileDialog`가 그대로 들고 있다 —
  `ProfileDialog`는 상시 마운트(닫힘 = `return null`)라 늦은 응답 무효화를
  unmount가 아니라 세션 토큰으로 해야 하기 때문이다.

탭 마크업은 `SettingsDialog`의 `role="tablist"` 관례를 작은 크기로 재사용한다
(`.appearance-tabs` / `.appearance-tab` / `.appearance-tabpanel`).

두 경로 모두 **최종 규격화는 기존 크롭 편집기가 담당한다**. Codex 생성은 크롭
여유가 있는 큰 캔버스(초상 1024×1536, 스프라이트·미니미 1024×1024 투명 배경)를
의뢰하고, 결과 data URL을 `PortraitEditor`/`SpriteEditor`의 `initialImage`로
프리로드해 연다(둘 다 마운트 시점 값만 1회 소비하는 attempt-once 패턴).
백엔드 계약은 `src-tauri/src/codex_imagegen/` — 파일 저장 경로(`./out.png`)를
백엔드가 소유하고 **파일 존재가 유일한 성공 판정**이며, 실패는 앱 공통의
`"{code}: {상세}"` 문자열(미설치는 `codex-not-found:`)로 온다.

#### 4.2.2 미니미(소환수) 전용 프롬프트

미니미는 **본체의 소환수**다 — 서브에이전트가 일하는 동안 주인 머리 옆에 뜨는
작은 분신. 그래서 스프라이트 프롬프트를 그대로 재활용하지 않고 전용 빌더를 둔다
(`promptBuilder.ts`의 `buildMinimiPrompt` / `buildCodexMinimiPrompt`).

- 프로필에 `minimiRequest`(‘미니미 의뢰 문구’) 필드가 있다. 적으면 그대로
  `Familiar: …` 줄로 들어간다.
- **비면 자동 위임 문구**(`autoMinimiRequestLine`)가 대신 들어간다 — "따로 요청이
  없으니 주인에게 어울리는 소환수를 알아서 디자인하라"는 한 줄. 사용자가 아무것도
  적지 않아도 본체와 어울리는 결과가 나오게 하기 위한 기본값이다.
- 주인의 `spriteRequest`(없으면 `appearance`)는 `Master's appearance:` 줄로만
  실린다 — 소환수가 주인을 닮게 하는 맥락이지 소환수 자신의 묘사가 아니다.
- 본문은 16×16 단일 프레임 지시(작게 읽히는 단순 실루엣)이고, codex 버전은
  마지막 규격 줄만 1024×1024 투명 배경으로 갈아 끼운다. 결과는
  `SpriteEditor target="minimi"`에 `initialImage`로 프리로드된다.

#### 4.2.3 ‘메모’를 ‘성격 프롬프트’로 통합

예전에는 **메모**(자유 설명, 초상 프롬프트에만 반영)와 **성격 프롬프트**(세션
시스템 프롬프트에 주입)가 따로 있었는데, 둘 다 "이 캐릭터가 어떤 존재인가"를
적는 칸이라 사용자가 늘 헷갈렸다. 지금은 **성격 프롬프트 하나**만 둔다.

- 새 캐릭터의 랜덤 초안은 `personalityPrompt`에 `"{성격} 성격"`을 채운다
  (`note`는 `""`). 즉 랜덤 성격이 실제로 세션에도 반영된다.
- 초상 프롬프트의 `Personality / notes:` 줄은 이제 `personalityPrompt`를 받는다.
- 스키마의 `AgentProfile.note`는 **레거시로 남긴다**(Rust 미러도 그대로). 편집기를
  열 때 `mergeLegacyNote(personalityPrompt, note)`로 기존 메모를 성격 프롬프트에
  합쳐 보여 주고, 저장하면 `note`는 `""`로 비워진다 — 값이 사라지지 않으면서
  단일 출처가 된다. 같은 병합을 캐릭터 번들 가져오기에도 적용한다
  (`applyBundleToDraft`). 이미 합쳐진 문자열을 다시 병합해도 중복되지 않는다.
- 포스트잇 메모(§13)와는 무관하다 — 그쪽은 사람이 쓰는 작업 기억이다.

---

## 5. 알림 티커 UX

### 5.1 아이템 콘텐츠 & 구조

각 카드:
- **타입 아이콘**(❓question / ✅done / ℹ️info) — 픽셀 이모지.
- **에이전트 이름** (bold, 픽셀 폰트).
- **메시지 excerpt** (<=80자, 말줄임).
- **상대 시각** ("방금", "2분 전") — `Intl.RelativeTimeFormat('ko')`.

### 5.2 스택/오버플로 규칙

- 최신이 위. 최대 5장 표시. 초과 시 6번째 자리에 "+N more" 요약 카드.
- **에이전트당 최신 1건만 티커에 노출**(같은 에이전트 다중 알림은 최신으로 합쳐 시각적 폭주 방지). 내부 큐엔 다 있지만 렌더 시 dedupe.

### 5.3 상호작용

- 카드 클릭 → `openTerminal(agentId)`(active 설정 + 해당 에이전트 알림 클리어) + `window.api.clearNotifications(agentId)`(A쪽 상태도 클리어).
- `notifications` 변화 → `sessionBridge`의 subscribe가 hasPending 중계 + `setBadgeCount` 갱신.

### 5.4 애니메이션

- 진입: `transform: translateX(16px)` + `opacity:0` → `0/1`, 180ms ease-out. `@keyframes tickerIn`.
- 퇴장: React `key` 제거 시 즉시. MVP는 진입만으로 충분(과한 모션 지양).

### 5.5 Dock/Taskbar 배지

- `window.api.setBadgeCount(pendingAgentCount)` — Tauri 커맨드 `set_badge_count` → `WebviewWindow::set_badge_count`(macOS dock/Linux). MVP 대상은 macOS 우선. `muted`면 배지 0 유지.

### 5.6 컴포넌트 스켈레톤

`src/renderer/notification/dedupe.ts` (순수 함수 — 테스트 대상)

```ts
import type { Notification } from '../store/types';

/** 에이전트당 최신 1건, 최신순 유지. 입력은 이미 최신순 정렬 가정. */
export function dedupeLatestPerAgent(list: readonly Notification[]): Notification[] {
  const seen = new Set<string>();
  const out: Notification[] = [];
  for (const n of list) {
    if (seen.has(n.agentId)) continue;
    seen.add(n.agentId);
    out.push(n);
  }
  return out;
}
```

`src/renderer/notification/NotificationTicker.tsx`

```tsx
import { useMemo } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { useAppStore } from '../store/appStore';
import { dedupeLatestPerAgent } from './dedupe';
import type { Notification } from '../store/types';

const MAX_VISIBLE = 5;
const rtf = new Intl.RelativeTimeFormat('ko', { numeric: 'auto' });

function relTime(ts: number): string {
  const diffSec = Math.round((ts - Date.now()) / 1000);
  if (Math.abs(diffSec) < 45) return '방금';
  const diffMin = Math.round(diffSec / 60);
  if (Math.abs(diffMin) < 60) return rtf.format(diffMin, 'minute');
  return rtf.format(Math.round(diffMin / 60), 'hour');
}

const TYPE_ICON: Record<Notification['type'], string> = {
  question: '❓', done: '✅', info: 'ℹ️',
};

export function NotificationTicker() {
  const notifications = useAppStore(useShallow((s) => s.notifications));
  const agents = useAppStore(useShallow((s) => s.agents));
  const openTerminal = useAppStore((s) => s.openTerminal);

  const deduped = useMemo(() => dedupeLatestPerAgent(notifications), [notifications]);
  const visible = deduped.slice(0, MAX_VISIBLE);
  const overflow = deduped.length - visible.length;

  const onClick = (agentId: string) => {
    openTerminal(agentId);
    window.api.clearNotifications(agentId);
  };

  return (
    <div className="notification-ticker" aria-live="polite">
      {visible.map((n) => {
        const agent = agents[n.agentId];
        return (
          <button
            key={n.id}
            className={`ticker-card pixel-panel type-${n.type}`}
            onClick={() => onClick(n.agentId)}
          >
            <span className="ticker-icon">{TYPE_ICON[n.type]}</span>
            <span className="ticker-body">
              <span className="ticker-name">{agent?.name ?? n.agentId}</span>
              <span className="ticker-msg">{n.excerpt}</span>
              <span className="ticker-time">{relTime(n.createdAt)}</span>
            </span>
          </button>
        );
      })}
      {overflow > 0 && (
        <div className="ticker-overflow pixel-panel">+{overflow} more</div>
      )}
    </div>
  );
}
```

---

## 6. 레트로/타이쿤 비주얼 스타일링

### 6.1 픽셀 폰트

- **DungGeunMo(둥근모꼴)** — 완성형 한글 전 영역 커버, 픽셀 감성, 무료(공개 폰트, 배포 허용). UI 텍스트용.
- **Galmuri11** — SIL OFL 1.1(임베드/재배포 자유, 폰트 자체 판매 금지). 등폭 정렬이 안정적인 픽셀 폰트.
- **터미널 셀은 일반 모노스페이스 전용** — xterm 셀 안에는 픽셀 폰트를 쓰지 않는다(`"SF Mono", "Menlo", "Consolas", "Liberation Mono", monospace`, `fontSize: 13`, `lineHeight: 1.2`). 픽셀 감성은 UI 크롬 전용이고, 터미널 화면은 코드/로그 가독성이 우선이라 실제 구현에서 Galmuri11 대신 표준 모노스페이스로 확정했다(터미널=Galmuri11 우선이라던 초기안은 폐기).
- **두 폰트 파일을 `src/renderer/assets/fonts/`에 번들하고 `@font-face`로 로드**(CDN 금지, Electron 오프라인 보장).

```css
@font-face {
  font-family: 'DungGeunMo';
  src: url('./assets/fonts/DungGeunMo.woff2') format('woff2');
  font-display: block;
}
@font-face {
  font-family: 'Galmuri11';
  src: url('./assets/fonts/Galmuri11.woff2') format('woff2');
  font-display: block;
}
```

### 6.2 CSS 접근: 플레인 CSS Modules 채택

- Tailwind는 픽셀 아트 특유의 `image-rendering:pixelated`, 다중 계단식 border, 커스텀 `@font-face` 조합에서 유틸 클래스가 오히려 장황해진다. 컴포넌트 수가 적고(10여 개) 디자인 언어가 고정적이므로 **CSS Modules + 전역 토큰 파일**이 명료하다.
- 전역 토큰 `src/renderer/styles/tokens.css`, 공용 픽셀 패널 클래스 `src/renderer/styles/pixel.css`.

### 6.3 2px 픽셀 패널 미학

```css
/* tokens.css */
:root {
  --bg-base: #12131a;
  --bg-panel: #1e2130;
  --bg-panel-hi: #2a2e42;
  --border-lite: #4a5170;
  --border-dark: #0a0b12;
  --accent: #7CFF6B;
  --accent-warn: #ffd866;
  --accent-error: #ff5c6a;
  --text: #c8d0e0;
  --text-dim: #8a93b0;
  --unit: 4px;                 /* 픽셀 그리드 단위 */
}

/* pixel.css — 공용 패널 (2px border, 계단식 하이라이트/섀도) */
.pixel-panel {
  background: var(--bg-panel);
  border: 2px solid var(--border-dark);
  box-shadow:
    inset 2px 2px 0 var(--border-lite),   /* 좌상 하이라이트 */
    inset -2px -2px 0 var(--border-dark),  /* 우하 섀도 */
    0 0 0 2px var(--border-dark);          /* 외곽 하드 라인 */
  image-rendering: pixelated;
  border-radius: 0;            /* 라운드 금지 */
}
.pixel-btn {
  font-family: 'DungGeunMo', monospace;
  background: var(--bg-panel-hi);
  color: var(--text);
  border: 2px solid var(--border-dark);
  box-shadow: inset 2px 2px 0 var(--border-lite), inset -2px -2px 0 var(--border-dark);
  padding: calc(var(--unit) * 2) calc(var(--unit) * 3);
  cursor: pointer;
}
.pixel-btn:active { box-shadow: inset -2px -2px 0 var(--border-lite), inset 2px 2px 0 var(--border-dark); }
.pixel-btn.primary { background: var(--accent); color: #0a0b12; }
```

- 모든 코너 각지게(border-radius:0), `image-rendering:pixelated`로 스프라이트 확대 시 픽셀 유지.
- 애니메이션은 하드 트랜지션 위주(부드러운 그라디언트 지양)로 게임 감성 유지.

> 위 `:root` 블록은 **초기안(단일 다크 테마=midnight) 시절의 값**이다. 지금은 §6.4의 테마 레지스트리가 색 토큰의 단일 원천이고, `tokens.css`의 `:root`는 기본 테마(daylight)와 같은 값의 부트/무JS 폴백일 뿐이다.

### 6.4 테마 레지스트리

`src/renderer/theme/themes.ts`가 색의 단일 원천이다. 한 테마(`ThemeDef`)는 세 팔레트를 함께 들고 있다.

| 필드 | 소비처 | 적용 방식 |
| --- | --- | --- |
| `css` (CSS_TOKEN_KEYS 10종) | DOM 전체 | `applyTheme()`이 `documentElement`에 `data-theme` + 인라인 커스텀 프로퍼티로 주입(`:root` 폴백을 이김) |
| `pixi` (타일 18종 + background/text) | 오피스 씬 | `App → OfficeCanvas → useOfficeScene → OfficeScene.setTheme()`이 타일 텍스처 재베이크 |
| `xterm` (`ITheme`) | 터미널 | §3.6 — 앱 테마를 따르는 게 기본이되 별도 고정 가능 |

- **테마 목록**: `daylight`(기본·밝음) / `midnight`(테마 도입 이전 룩 보존) / `sakura`(파스텔 핑크) / `pipboy`(핍보이 — 폴아웃 Pip-Boy 오마주, 인광 초록 모노크롬 CRT).
- **캐릭터 스프라이트 팔레트(`office/gen/palette.ts`, `archetypes.ts`)와 분석 차트 색은 테마 대상이 아니다** — 에이전트별 절차 생성/데이터 인코딩이라 테마가 바뀌어도 불변이다.
- **영속**: `localStorage("agent-office.theme")`. 스토어 초기값이 `loadStoredThemeId()`이고 `main.tsx`가 첫 `render()` 전에 `applyTheme()`을 동기 호출하므로 잘못된 테마로 페인트되는 플래시가 없다.
- **픽커**: BottomBar의 "테마: {라벨}" 버튼 → 공유 `ContextMenu` 드롭다운(전 테마, 현재 테마에 ✔). 순환 버튼이던 초기 구현은 테마가 4개가 되면서 폐기했다(`nextThemeId`는 순서 계약·향후 단축키용으로 남아 있다). 라벨은 짧게 유지한다 — BottomBar는 800px 창에서 이미 폭이 포화다(docs/usage-design.md).
- **CRT 오버레이**: `theme === "pipboy"`일 때만 App 루트에 `.crt-overlay` 1장을 마운트한다. `position:fixed; inset:0; z-index:100; pointer-events:none`에 순수 CSS 두 겹(4px 주기 스캔라인 `rgba(0,0,0,.06)` + 옅은 비네트)뿐 — 애니메이션·블러가 없어 합성 비용이 사실상 0이고 모달/터미널 상호작용을 그대로 통과시킨다. 다른 테마에서는 아예 마운트되지 않는다.

### 6.5 풍경(scene) 레지스트리

`src/renderer/office/scenes/`가 **테마와 직교한 두 번째 축**이다. 테마가 "색"을 고르면 풍경은 "무대"를 고른다 — 어떤 조합도 유효하다.

| 필드(`SceneDef`) | 내용 |
| --- | --- |
| `map` | 그 풍경의 타일 맵. 좌석·휴게 공간·보스 자리·줄 슬롯이 전부 여기서 유도된다(docs/subsystem-b-office.md §2.2) |
| `resolve(theme)` | 현재 테마로 확정된 `{ background, drawTile }`. 16px 한 칸을 무엇으로 그릴지가 풍경의 몫이다 |

- **풍경 목록**: `office`(기본·사무실) / `beach`(바캉스 해변) / `valley`(산 계곡).
- **소비 경로**: `App → OfficeCanvas → useOfficeScene → OfficeScene.setScene()`. 테마 경로(`setTheme`)와 나란한 별도 효과라 둘 중 하나만 바뀌어도 각자 처리된다. `setTheme`은 색만 다시 칠하고(바닥 재베이크 + 가구 교체 + 팻말 재도색), `setScene`은 거기에 더해 좌석 히트영역·보스 책상·`OfficeWorld`의 맵까지 재구축한다.
- **테마 × 풍경 색**: `office`는 `theme.pixi`를 그대로 쓴다(도입 이전 룩 보존). `beach`/`valley`는 "한낮의 원색" 팔레트 한 벌만 두고 `scenes/sceneColor.ts`가 테마별로 변환한다 — daylight·sakura는 원색 그대로, midnight는 황혼(채도·명도를 낮추고 야청색으로 기울임), pipboy는 휘도를 감마·양자화해 인광 초록 램프로. 씬 안 텍스트("휴가중" 팻말)와 보스 명패 색만은 예외로 **테마** 팔레트를 계속 쓴다.
- **영속**: `localStorage("agent-office.scene")`. 테마와 같은 관례로 스토어 초기값이 `loadStoredSceneId()`이고, 잘못된 값이면 `office`로 폴백한다. Rust `AppSettings`와는 무관한 순수 프런트 취향값이다.
- **픽커**: BottomBar의 "풍경: {라벨}" 버튼 → 테마와 같은 `ContextMenu` 드롭다운(현재 풍경에 ✔). 라벨은 테마와 같은 이유로 짧게 유지한다.

---

## 7. vitest / 테스트 시임 (구체적 5개)

`src/renderer/store/appStore.test.ts`, `notification/dedupe.test.ts`, `terminal/TerminalRegistry.test.ts`

**T1. addAgent가 세션을 starting으로 시드한다**
```ts
it('addAgent seeds session as starting', () => {
  const s = useAppStore.getState();
  s.addAgent(mkProfile({ id: 'a1' }));
  const st = useAppStore.getState();
  expect(st.agentOrder).toContain('a1');
  expect(st.sessions['a1'].status).toBe('starting');
  expect(st.sessions['a1'].cols).toBe(80);
});
```

**T2. openTerminal이 해당 에이전트 알림만 클리어하고 active/recent를 갱신한다**
```ts
it('openTerminal clears only that agent notifications', () => {
  const s = useAppStore.getState();
  s.addAgent(mkProfile({ id: 'a1' })); s.addAgent(mkProfile({ id: 'a2' }));
  s.pushNotification(mkNotifEvent({ agentId: 'a1' }));
  s.pushNotification(mkNotifEvent({ agentId: 'a2' }));
  s.openTerminal('a1');
  const st = useAppStore.getState();
  expect(st.activeTerminalAgentId).toBe('a1');
  expect(st.recentAgentIds[0]).toBe('a1');
  expect(st.notifications.every((n) => n.agentId !== 'a1')).toBe(true);
  expect(st.notifications.some((n) => n.agentId === 'a2')).toBe(true);
});
```

**T3. active인 에이전트로 온 알림은 억제된다**
```ts
it('suppresses notification for the active agent', () => {
  const s = useAppStore.getState();
  s.addAgent(mkProfile({ id: 'a1' }));
  s.openTerminal('a1');
  s.pushNotification(mkNotifEvent({ agentId: 'a1', source: 'hook', message: 'need input' }));
  expect(useAppStore.getState().notifications).toHaveLength(0);
});
```

**T4. 티커 dedupe — 에이전트당 최신 1건, 최신순**
```ts
it('dedupes to latest per agent, newest first', () => {
  const list = [
    mkNotif({ agentId: 'a1', createdAt: 300 }),
    mkNotif({ agentId: 'a2', createdAt: 250 }),
    mkNotif({ agentId: 'a1', createdAt: 100 }),  // 오래된 a1
  ];
  const out = dedupeLatestPerAgent(list);
  expect(out.map((n) => n.agentId)).toEqual(['a1', 'a2']);
  expect(out[0].createdAt).toBe(300);
});
```

**T5. TerminalRegistry keep-alive — ensure는 동일 인스턴스 반환, destroy 전까지 유지**
```ts
it('ensure returns same Terminal instance across calls (keep-alive)', () => {
  // window.api mock: onData -> unsubscribe spy
  const e1 = terminalRegistry.ensure('a1');
  const e2 = terminalRegistry.ensure('a1');
  expect(e1.term).toBe(e2.term);           // 리마운트해도 같은 인스턴스
  expect(terminalRegistry.has('a1')).toBe(true);
  terminalRegistry.destroy('a1');
  expect(terminalRegistry.has('a1')).toBe(false);
  expect(unsubscribeSpy).toHaveBeenCalledTimes(1);  // onData 정리 확인
});
```

테스트 환경: xterm은 jsdom에서 캔버스 측정이 제한되므로 `@xterm/xterm`을 `vi.mock`으로 스텁한다. registry 테스트는 `window.api`를 `vi.stubGlobal`로 목킹. store 테스트는 각 `beforeEach`에서 `useAppStore.setState(initialState, true)`로 리셋.

---

## 8. (결번 — 구현 태스크 분해는 제거됨, 2026-07-20)

---

## 9. 핵심 설계 결정 요약

| 항목 | 결정 | 이유 |
|---|---|---|
| 터미널 패널 | 중앙 오버레이 72% + 탭스트립 | 문맥 전환 집중 UX, 리사이즈 최소화, 게임 팝업 메타포 |
| keep-alive | TerminalRegistry 싱글턴 + display:none 토글 | React 리마운트와 무관하게 xterm 인스턴스·스크롤백 보존 |
| 고빈도 스트림 | onData는 스토어 우회, 직접 `term.write` | 리렌더 폭발 방지 |
| 상태관리 | zustand + subscribeWithSelector | IPC 콜백(React 밖)에서 setState, 세밀 구독 |
| 알림 억제 | active 에이전트 알림 push 무시 + 열람 시 클리어 | 보고 있는 세션은 알림 불필요 |
| 폰트/CSS | DungGeunMo+Galmuri11 번들, CSS Modules | 오프라인·픽셀 감성·장황함 회피 |
| fit 타이밍 | show 후 rAF에서 fit + debounced resize IPC | display:none 상태의 0크기 fit 방지 |

---

## 10. 작업 폴더 보기 & 마크다운 탐색 (이슈 #10 / #11 / #54)

에이전트 cwd를 앱 안에서 직접 들여다보는 오버레이 서브시스템. 오피스 씬·세션과
무관한 독립 슬라이스라 `store/appStore`가 아니라 전용 zustand 스토어
(`renderer/workdir/workdirStore.ts`, `renderer/markdown/markdownStore.ts`)로 분리한다.
진입은 탭 우클릭 컨텍스트 메뉴(`작업 폴더 보기` / `문서`) 또는 상세 페인 버튼.
백엔드는 `src-tauri/src/workdir/`(R-3 리팩터로 모듈화 — `{git_runner,status,diff,listing,model,commands}.rs`,
커맨드 표면은 서브시스템 A §2.2)와 `markdown.rs`. z-index: 팔레트 40, 마크다운 편집기 50(모달 30·터미널 20 위).

### 10.1 작업 폴더 팔레트 (`WorkdirPalette`)

- **평면 퍼지 리스트**(트리 아님) + `[전체 | 변경만]` 필터 + `git 상태` on/off 토글
  (전역 설정과 동일 값 — 상태 이원화 방지). "전체"는 파일 목록에 git 상태를
  relPath로 매칭해 뱃지를 얹고, "변경만"은 git 엔트리 자체를 목록으로 쓴다(삭제·
  root 밖 `../` 파일 포함). git 꺼짐/비저장소면 "변경만"·"커밋 로그"는 비활성.
- **뷰 모드 2종(#54)**: `viewMode: "files" | "log"` — 헤더 `파일 | 커밋 로그` 세그로 전환.

### 10.2 진입 흐름 — "메뉴 우선" (#54)

- **모든 파일 클릭 → 상세(메뉴) 페인**(`openDetail`). 변경 파일은 기본 `변경점` 탭,
  변경 없는 파일은 기본 `히스토리` 탭(clean 파일도 깃 로그를 항상 노출; git 상태
  토글이 꺼져 있어도 히스토리·diff는 조회됨 — 토글은 status **폴링**만 게이팅).
- **빠른 열기**: `⌘/Ctrl-클릭`·더블클릭·`⌘+Enter`는 기존 자동 라우팅(`openEntry`,
  .md→인앱·그 외→`open_in_vscode`). 리스트 행은 `mouseDown`=선택만·`click`=열기로
  분리해 더블클릭이 성립하게 한다.
- **#11 → #54 전환**: 이전에는 변경 파일만 상세로 보내고(`isChangedStatus` 분기)
  나머지는 즉시 열었으나, "일반 파일도 열기 전에 로그·메뉴를 보고 싶다"는 요구로
  분기를 제거하고 메뉴 우선으로 통합.

### 10.3 상세 페인 (`WorkdirDetailPane`)

- 상단 액션 **명시적 2버튼**(#54): `외부 프로그램으로 열기`(마크다운도 강제 외부) +
  (마크다운만) `인앱 뷰어로 열기`. 이전 자동 라우팅(.md면 무조건 인앱)을 대체.
- `변경점` 탭: 추적 파일은 3관점 세그(`worktreeVsHead` 합본=기본·`indexVsHead`
  스테이지·`worktreeVsIndex` 미스테이지), 미추적은 단일 뷰. `gen` 카운터로 모드
  전환 시 늦게 도착한 stale 응답 폐기.
- `히스토리` 탭: `git log --follow` 커밋 목록. **커밋 행 인라인 확장(#54)** — `▸/▾`
  토글(`toggleCommitExpand`)로 그 커밋이 바꾼 파일 목록을 인라인 표시(`더 보기…`
  페이징), 파일 선택 시 그 커밋의 해당 파일 diff. 펼치지 않고 커밋만 고르면
  (`selectCommit`) 지금 파일의 그 커밋 시점 diff. 하단 diff는 `(selectedCommit,
  selectedCommitFile)` 기준.
- **인앱 뷰어 복귀(#54)**: `openInApp`은 팔레트+detail 스냅샷을 잡고 열며,
  markdownStore `openFile(…, onClose)` 콜백에서 그 스냅샷을 복원 → **인앱 마크다운
  뷰어를 닫으면 작업 폴더 탐색 상태로 복귀**. (빠른 열기 `openEntry`는 무변경 —
  닫으면 오피스로.)
- **초기 뷰 모드(#76)**: `openFile`은 `.md`류를 열 때 `mode: "preview"`(미리보기)를
  기본으로 연다("읽기" 우선). 비-md 유입 시엔 `"source"`로 폴백. 소스↔미리보기 토글은
  세션 간 기억 없이 매 오픈마다 이 기본으로 초기화.

### 10.4 커밋 로그 브라우저 (`WorkdirRepoLogPane`, #54 2단계)

- 파일을 먼저 지목하지 않고 **로그 → 커밋 → 변경파일 → diff** 순으로 훑는다.
  좌: 커밋 목록(메시지 검색 300ms 디바운스·`전체 브랜치`(`--all`) 체크박스·`더 보기…`),
  우: 선택 커밋의 변경파일 → 파일 선택 → diff + 외부 도구 비교.
- 스토어 `repoLog` 슬라이스는 root별 캐시 + `gen` 스테일 가드(검색/브랜치 전환 시
  증가 → 늦은 응답 폐기). 검색은 `git log --grep=<q> -i -F`(고정 문자열·대소문자 무시).
- **combined diff**: `git show`/`git log`의 기본 동작(머지는 `--cc`)을 그대로 사용.
  병합 커밋은 combined라 변경파일이 빌 수 있어 "표시할 파일 변경 없음(병합)" 안내.

### 10.5 diff 렌더 & 백엔드 커맨드

- `DiffView.tsx`: 새 npm 의존성 없이 unified diff 텍스트를 줄 종류(meta/hunk/add/del)
  로 자체 색상 렌더(marked 자체 렌더 철학과 동일).
- 백엔드 읽기 전용 커맨드: `workdir_diff_file`·`workdir_file_history`·
  `workdir_diff_commit`(#11), `workdir_commit_files`(`git show --name-status -M -z`
  파서·페이징)·`workdir_repo_log`(#54). `launch_difftool`만 외부 GUI를
  fire-and-forget. **안전장치**: `sanitize_rel_path`(절대경로·`..` 거부, 항상 `--`
  pathspec)·`valid_commit`(hex 7~40)·diff 상한(1MiB·5000줄)·쿼리 타임아웃(§10.5.1).

#### 10.5.1 조회 타임아웃 & 사용자 취소

- **문제**: 예전 타임아웃(status 3s / diff·log 10s)은 거대 저장소에서 UX 상한이
  아니라 **무조건 실패**였다 — 사용자가 기다릴 의사가 있어도 조회가 끊겼다.
- **모델 전환**: 타임아웃은 **폭주 방지 백스톱**으로만 남기고(status 120s /
  diff·log 300s), 1차 탈출구는 **사용자 취소**다.
  - 프런트가 조회마다 `opId`(UUID)를 만들어 커맨드에 함께 넘긴다. 백엔드
    `git_runner`의 프로세스 전역 레지스트리(`opId → AtomicBool`)에 취소 플래그가
    등록되고, `CancelGuard`의 Drop이 조회 종료 시 반드시 해제한다.
  - `workdir_git_cancel(opId)`가 그 플래그를 세우면 `proc_runner` 폴 루프(15ms)가
    자식 git을 kill하고 `ProcOutcome::Canceled`로 끝낸다. 없는 opId는 조용한 no-op
    (프런트는 fire-and-forget으로 부른다).
  - 취소는 **에러가 아니라** 결과의 `canceled: true`(status/diff/history/commitFiles
    공통)로 돌아온다 — `timedOut`과 구분해 UI 문구가 갈린다.
- **blocking 격리**: 타임아웃이 분 단위가 되면서 조회 커맨드 본문은 전부
  `tauri::async_runtime::spawn_blocking`으로 옮겼다(async 워커 점유 방지).
- **프런트**: 로딩 플래그 옆에 진행 중 opId를 보관한다(`gitOpId[root]`,
  detail의 `diffOpId`/`historyOpId`/`commitDiffOpId`/`commitFilesOpId`, repoLog의
  `opId`/`filesOpId`/`fileDiffOpId`). 모든 "불러오는 중…" 자리에 **취소** 버튼,
  취소된 결과에는 "조회를 취소했습니다" + **다시 시도**(새 opId로 재조회).
  `refreshGit`은 같은 root의 in-flight 조회가 있으면 스킵하고(중복 스택 방지),
  팔레트/상세 닫기·diff 모드 전환·뷰 전환·다른 커밋/파일 선택·로그 `gen` 증가처럼
  진행 중 op가 무의미해지는 지점에서는 자동으로 취소한다.
- 취소된 조회의 빈 결과는 **캐시하지 않는다**(`history`/`commitFiles`/`commits`를
  `undefined`로 남김) — "커밋/변경 없음"과 "취소됨"을 UI가 구분해야 하기 때문.

### 10.6 미추적 파일 전개 & 긴 경로 표시 (#70 / #71)

- **새 폴더 안의 새 파일(#70)**: `git status`는 기본 `-unormal`이라 미추적 디렉터리를
  접어 `? docs/` 한 줄로만 보고한다 → 내부 파일이 목록에 없고, 그 폴더 엔트리를
  누르면 `untracked` diff가 디렉터리를 가리켜 "변경 없음"이 된다. `status.rs`의
  인자에 **`--untracked-files=all`**을 더해 파일 단위(`? docs/new/a.md`)로 펼친다 —
  파서·프런트 변경 없이 기존 흐름(`?` 뱃지 → `--no-index` 신규 diff)이 그대로 성립.
  `.gitignore` 대상은 여전히 제외된다(`--ignored`를 주지 않으므로).
- **엔트리 상한**: `-uall`은 gitignore되지 않은 대량 산출물 폴더에서 수만 건을 낼 수
  있어 `parse_porcelain_v2`가 **5000개**(`MAX_STATUS_ENTRIES`, listing의 `MAX_LIST`와
  동일)에서 끊고 `GitStatusResult.truncated`를 세운다. 브랜치 헤더(`# …`)는 항상
  엔트리보다 앞이라 중단해도 손실이 없다. 프런트는 절단 시 목록 위 안내와 헤더
  `변경 5000+개` 표기를 보여준다. *폴더 단위 통합 diff는 이번 범위 밖(후속).*
- **긴 경로 표시(#71)**: 행은 `[뱃지] [파일명] [상대경로]`인데 말줄임이 뒤에 걸리면
  식별에 필요한 리프 디렉터리가 통째로 사라진다. `.wd-item-path`/`.wd-cf-path`/
  `.wd-detail-path`에 **head-ellipsis**(`direction: rtl` + `text-align: left`)를 적용해
  끝을 보존한다. RTL 문단에서 선행 중립문자가 재배열돼 root 밖 경로 `../a/b.ts`가
  `a/b.ts../`로 뒤집히는 문제는 `::before { content: "\200E" }`(LRM, 강한 L)로 막는다
  (`unicode-bidi: plaintext`는 문단 방향을 LTR로 되돌려 말줄임이 다시 뒤로 가므로 쓰지
  않는다). 표시 내용은 **전체 상대경로 유지**(dirname만 노출하지 않음) + 행 전체에
  `title` 툴팁.

### 10.7 핵심 설계 결정 요약

| 항목 | 결정 | 이유 |
|---|---|---|
| 파일 열기 진입 | 모든 파일 클릭=메뉴 우선, ⌘/더블클릭=즉시 열기 | 열기 전 깃 로그·외부/인앱 선택 노출(#54) |
| clean 파일 기본 탭 | 히스토리 탭 | 변경 없어도 로그를 항상 볼 수 있게 |
| git 토글 범위 | status 폴링만 게이팅, diff/history는 항상 조회 | 거대 저장소 대비는 status에만 필요 |
| 커밋 변경파일 | `git show --name-status -M -z` + 페이징(상한 2만) | 인라인 확장·로그 브라우저 공용 재사용 |
| 머지 커밋 diff | git 기본 combined(`--cc`) | 사용자 확정; 빈 목록은 안내로 |
| 인앱 뷰어 복귀 | markdownStore `onClose` 콜백 + 탐색 상태 스냅샷 | 뷰어 닫으면 탐색으로 복귀(#54) |
| diff 렌더 | 의존성 없이 자체 줄단위 색상 | marked 자체 렌더 철학 일관 |
| 로그 검색 | `--grep -i -F`(메시지, 고정 문자열) | 예측 가능·주입 안전. 작성자 검색은 후속 |
| 미추적 표시 | `--untracked-files=all` + 5000개 상한 | 새 폴더 안 파일도 개별 diff(#70) |
| 긴 경로 말줄임 | head-ellipsis(`direction: rtl` + LRM) | 식별에 중요한 끝을 보존(#71) |

## 11. 터미널 오버레이 뷰 모드 (이슈 #69)

터미널 오버레이 패널을 넓게 쓰기 위한 확대 모드. **windowed ↔ filled 2스테이트 토글**
로 확정(초기 기획의 OS 전체화면 3-스테이트에서 축소 — OS 전체화면 상태 동기화
복잡성을 걷어내고 인앱 CSS만으로 처리).

- **windowed**: 화면 중앙 `72%×72%` 오버레이(기존 동작, 배경 딤 유지).
- **filled**: 오버레이가 앱 창 전체를 덮음 — 배경 딤 제거, 탭 스트립·요약 바는 유지.

### 11.1 상태/영속

- 스토어 필드 `terminalViewMode: "windowed" | "filled"` + 액션
  `setTerminalViewMode`/`cycleTerminalViewMode`(2스테이트 토글).
- 영속은 **테마와 동일하게 `localStorage`**(`agent-office.terminal-view-mode`) —
  `PersistedState`(agents/vacationMode)를 확장하지 않는다. 순수 로직·영속은
  `renderer/terminal/terminalViewMode.ts`로 분리(스토어/Tauri 의존 없음 → appStore가
  안전하게 import, 순환 방지). 부수효과(localStorage)는 액션에서 직접 수행(theme 패턴).

### 11.2 렌더/불변식

- `TerminalOverlay`가 루트에 `mode-*` 클래스만 토글하고, `layout.css`의
  `.terminal-overlay.mode-filled`가 패널 100%×100% + 배경 딤 제거를 담당한다.
- **keep-alive 불변식 유지**: 조건부 렌더가 아니라 CSS 클래스 변경뿐이라 xterm/PTY가
  리마운트되지 않는다. 패널이 커지면 `TerminalHost`의 active-only `ResizeObserver`가
  debounce 후 자동 refit → `setSessionSize`/`resize`가 PTY `cols/rows`를 갱신(별도
  refit 로직 불필요).
- filled에서는 패널이 오버레이 루트를 완전히 덮어 **backdrop mousedown 닫기 경로가
  도달 불가** → X 버튼/`Cmd+W`가 유일한 닫기 경로(의도된 동작).

### 11.3 조작

- 헤더 토글 버튼 하나(`문서`와 닫기 사이). 아이콘은 현재 모드(`⤢`↔`❐`), title은
  다음 모드 안내. filled일 때 악센트색으로 강조(현재 모드 시각 구분).
- 단축키: OS "확대" 관례 — **macOS `Ctrl+Cmd+F`, 그 외 `F11`** 이 windowed↔filled 토글.
  `mod` 게이트보다 먼저 처리(F11은 수식키 없음). `Esc`는 여전히 미사용(TUI가 실제
  Escape를 받아야 함).

### 11.4 핵심 설계 결정 요약

| 항목 | 결정 | 이유 |
|---|---|---|
| 모드 수 | windowed ↔ filled 2스테이트 | OS 전체화면 제외로 상태 동기화 리스크 제거(사용자 확정) |
| 확대 방식 | CSS 클래스 토글만 | keep-alive 불변식 유지(리마운트 없음) |
| refit | 별도 로직 없음 | 컨테이너 리사이즈를 ResizeObserver가 자동 처리 |
| 영속 | `localStorage`(테마 패턴) | PersistedState 미확장, 마지막 모드 유지 |
| 단축키 | mac Ctrl+Cmd+F / 그 외 F11 | OS 확대 관례 |

## 12. 캐릭터 일기 내보내기 (이슈 #65)

`DiaryDialog`(읽기 전용 열람)에 **일기 전체를 파일로 저장**하는 액션을 더한 것.
저장 방식은 셸 출력 내보내기(#42, 임시 폴더 + 외부 에디터 자동 열기)가 아니라
**캐릭터 번들 내보내기(#77)의 네이티브 저장 다이얼로그 패턴**을 따른다 — 일기는
"보관·공유"가 목적이라 사용자가 위치를 고르는 편이 맞고, 임시 폴더는 GC 대상이다.

### 12.1 포맷 — 확장자가 곧 선택

Markdown(기본)과 JSON 두 가지를 지원하되 **UI에는 버튼 하나**뿐이다. 저장
다이얼로그에 `md`/`json` 두 필터를 걸고, **사용자가 고른 경로의 확장자로 본문을
택한다**(`.json`이면 JSON, 그 외·확장자 없음은 Markdown).

- 다이얼로그가 닫힌 뒤에는 렌더러에 되물을 수 없으므로, 커맨드
  `export_diary_file(defaultName, markdown, json)`은 **두 본문을 모두 미리 받는다**.
  택일은 백엔드의 순수 함수 `diary_content_for(path, md, json)`이 하고, 이 함수만
  단위 테스트한다(다이얼로그 없이 검증 가능).
- Markdown: `# <이름>의 일기` → `총 N편` → 편별 `## <날짜 시각>` + 본문.
  **작성순(오래된 → 최신)** — 화면은 최신 먼저지만 문서로 읽을 땐 시간순이 자연스럽다.
  본문은 이스케이프하지 않고 원문 보존(코드블록·`#`가 들어 있어도 그대로).
- JSON: 자기완결형 번들 `DiaryBundle`(`kind: "agent-office.diary"`,
  `schemaVersion: 1`, `agentName`, `exportedAt`, 원본 `entries`) —
  `CharacterBundle`(#77)과 같은 판별자 관례. 현재는 내보내기 전용(가져오기 범위 밖)이라
  파서는 두지 않는다.

### 12.2 계층

포맷팅은 렌더러의 순수 모듈 `renderer/diary/diaryExport.ts`(`formatDiaryMarkdown` /
`formatDiaryJson` / `diaryFileName` / `sanitizeFileBase` / `formatWhen`)가 맡고,
파일 I/O는 백엔드가 한다(`characterIo.ts`와 같은 관례). `formatWhen`은 목록 표시와
내보낸 문서가 같은 표기를 쓰도록 이 모듈로 옮겨 `DiaryDialog`가 import한다.
초기 파일명 `<이름>-일기-<YYYYMMDD-HHmm>.md`의 이름 부분은 Rust
`shell_export::sanitize_agent_name`을 미러한 `sanitizeFileBase`로 안전화한다(경로
구분자가 파일명으로 새지 않게).

### 12.3 스토어/UI

- `diaryStore.exportNow(agentId)` + `exporting` 플래그(중복 실행 방지). 저장
  다이얼로그가 떠 있는 사이 목록이 바뀌어도 흔들리지 않게 **호출 시점의 `entries`로
  두 포맷을 먼저 만들어** 넘긴다.
- 결과는 기존 `notice` 배너로: 성공은 저장 경로, 취소(`null` 반환)는 "취소했습니다"
  (실패로 취급하지 않음), 예외는 실패 안내. 다른 캐릭터로 전환됐으면 안내를 덮지
  않는다(스토어 stale 가드 관례).
- 버튼은 **다이얼로그 헤더에만** 둔다(캐릭터 컨텍스트 메뉴 비대화 방지, 사용자 결정).
  일기 0편이면 비활성.

## 13. 에이전트별 포스트잇 메모 (이슈 #79)

터미널 오버레이 **우상단에 붙은 쪽지 한 장**. 사람이 캐릭터별 작업 컨텍스트를
직접 적어 두는 곳이며, 에이전트가 읽거나 쓰지 않는다 — 일기(#56/#60)가 LLM이
쓰는 기록인 것과 정반대다. `AgentProfile.note`와도 무관하다(그쪽은 프로필 속성).

### 13.1 왜 파일이고, 왜 plain text인가

저장 형식은 `<app_data>/memos/<agentId>/<sheetId>.txt` — **장(sheet) 단위 파일**에
Obsidian식 frontmatter 헤더 + plain text 본문이다.

```
---
created: 2026-07-30T12:34:56+09:00
updated: 2026-07-30T13:00:00+09:00
archived: 2026-07-31T09:00:00+09:00   (넘긴 장에만)
---
본문
```

- **JSON이 아닌 이유**: 이 파일은 사용자가 에디터로 직접 열어 읽고 고칠 수 있어야
  한다. 본문이 JSON 문자열로 이스케이프되면 그 성질을 잃는다. 그래서 시각도
  epoch ms가 아니라 로컬 오프셋이 붙은 RFC3339 문자열이다(사람이 읽는 헤더).
- **마크다운 렌더링 없음**: 위젯 본문은 `<textarea>`이고 렌더 단계가 없다. "쓰는
  대로 보인다"가 요점이라 마크다운 뷰어(#10)와 다른 갈래다.
- **파서는 손으로 짠 `key: value`**: 우리가 쓰는 키는 3개뿐이라 yaml 크레이트를
  들이지 않는다. 본문에 `---` 줄이 있어도 깨지지 않게, **헤더는 파일 첫 줄이 정확히
  `---`일 때만** 시작하고 **두 번째 `---`까지**다. 닫는 `---`가 없으면 헤더가 아닌
  것으로 보고 파일 전체를 본문으로 돌린다(모르는 키는 무시 — 전방 호환).
- 쓰기는 tmp+rename 원자 교체(`work_log_store.rs`와 같은 패턴), `agentId`/`sheetId`는
  경로 요소로 쓰기 전 `validate_id`로 거른다(`diary_store.rs` 규칙 복제 — 공유 헬퍼
  없음이 이 저장소 관례).

### 13.2 "한 장 넘기기" = 스탬프, 이동 아님

**현재 장 = frontmatter에 `archived` 키가 없는 유일한 파일**이라는 불변식 하나로
전체 모델이 돈다. 넘기기는 파일을 옮기거나 지우지 않고 헤더에 `archived` 스탬프만
더한다 — 그래서 `sheetId`(=파일명)가 영구히 고정되고, 아카이브 열람은 같은 폴더를
다시 읽는 것으로 끝난다.

- `sheetId`는 생성 시각 기반(`20260730T123456`)이라 **사전순 = 시간순**이다. 같은
  초에 두 장이 생기면 `-1`, `-2` 접미사가 붙어 그 성질을 지킨다.
- `save`는 이미 붙은 `archived` 스탬프를 **절대 지우지 않는다**. 넘기기 직후 뒤늦게
  도착한 디바운스 저장이 스탬프를 되살리면 "현재 장 둘"이 되기 때문이다.
- 그래도 `archived` 없는 장이 둘 이상 발견되면(외부 편집 등) `load_current`가 가장
  최근 것만 현재 장으로 남기고 나머지에 스탬프를 찍는다 — 삭제가 아니라 아카이브로
  밀어 넣는 방어적 복구다.

### 13.3 저장 버튼이 없다

사용자가 인지하는 저장은 전부 자동이다. `memoStore`가 세 경로를 보장한다:

1. 타이핑 → `edit()`이 1초 디바운스 후 `saveMemo`.
2. blur / 에이전트 전환 / 위젯 닫힘 / 넘기기 직전 / 아카이브 열기 → `flush()`가
   예약된 타이머를 취소하고 즉시 저장.
3. 저장 실패는 `dirty`를 되살려 다음 `flush`에서 재시도한다.

전환 시에는 **`agentId`를 갈아치우기 전에** 이전 캐릭터의 `flush`를 await 한다 —
그러지 않으면 마지막 타이핑이 새 캐릭터의 장에 쓰인다.

### 13.4 콘솔 연동은 복붙뿐

텍스트 선택 복사 + "전체 복사" 버튼(`navigator.clipboard`) 둘뿐이다. 프롬프트
주입/전송은 **의도적으로 만들지 않았다** — 메모는 사람의 작업 기억이고, 자동 주입은
봇 모드(#57)가 다루는 별개 문제다.

### 13.5 배치와 격리

- 위젯은 `TerminalOverlay`의 `.terminal-overlay-panel` 안에 `position: absolute`로
  얹힌다(그래서 패널에 `position: relative`가 필요하다 — 없으면 fixed인
  `.terminal-overlay`가 기준이 되어 windowed 모드에서 쪽지가 패널 밖에 붙는다).
- **keep-alive 불변식과 무관**하다: 그 불변식은 `AgentTabStrip`/`TerminalHost`와 그
  아래 xterm 인스턴스에 걸린 것이고, 이 위젯은 형제이며 본문의 진실은 디스크다.
  그래서 닫으면 조건부 렌더로 언마운트한다(언마운트 직전 `flush`).
- 위젯 안의 keydown/keyup은 `stopPropagation` 한다. 터미널 오버레이가 window에
  `Cmd/Ctrl+1..9`·`Cmd+W`를 걸어 두므로, 그대로 두면 메모를 타이핑하다 탭이 바뀐다
  (React는 루트 컨테이너에 리스너를 붙이므로 여기서 멈추면 window까지 못 간다).
- 로드가 끝나기 전에는 `<textarea>`를 disabled로 둔다 — 빈 `draft`가 디스크의 본문을
  덮어쓰는 사고 방지.

### 13.6 상태/파일 배치

- 열림 여부는 **전역**이고 localStorage에 영속한다(`memoVisibility.ts`,
  `terminalViewMode.ts`와 같은 순수 모듈 관례). 위젯은 늘 활성 탭의 장을 보여주므로
  "포스트잇을 띄워 둔다"는 사용자의 작업 습관이지 캐릭터의 속성이 아니다.
- `memoStore`는 appStore와 비커플링(독립 스토어 관례). 활성 탭이 무엇인지는
  `PostItWidget`이 appStore에서 읽어 `focusAgent`로 밀어 넣는다.
- 캐릭터 삭제 시 정리는 `memoCleanup.ts`가 appStore의 `agents` 구독에서 사라진 id를
  잡아 `deleteMemos`를 부른다 — 초상/스프라이트/미니미(`portraitCache`/`spriteCache`/
  `minimiCache`)와 **같은 지점·같은 방식**이다(이 저장소에는 "캐릭터 삭제" 백엔드
  커맨드가 없다).
- 진입점 둘: 터미널 헤더의 `🗒` 토글 버튼(문서 버튼과 뷰 모드 버튼 사이)과 탭 우클릭
  메뉴의 "포스트잇 열기/닫기"·"메모 아카이브". 비활성 탭에서 메뉴로 열면 그 탭으로
  함께 전환한다(사용자가 지목한 캐릭터의 메모가 보이도록).
