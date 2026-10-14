# 서브시스템 C — Renderer UI / 상태관리 / TerminalHost

상태: 정본 (2026-10-14) — 구현 전 코드 스케치(초기 §0~§7)는 [archive/subsystem-c-ui-sketch.md](archive/subsystem-c-ui-sketch.md)로 분리했다. §0~§9는 현재 코드를 파일+심볼로 다시 쓴 것이고, §4.2.1~§4.2.4·§10~§14는 이슈별 증분 명세를 이어 온 것이다. 절 번호는 외부 참조를 위해 유지한다(§8 결번). 세부 값은 해당 심볼이 정본이다. 이번 정리에서 화면을 새로 확인하지는 않았다.

렌더러(React 18 + zustand + xterm.js) 계층 전체를 다룬다. 오피스 씬(PixiJS)은 [subsystem-b-office.md](subsystem-b-office.md), 백엔드·IPC는 [subsystem-a-sessions.md](subsystem-a-sessions.md)가 정본이다.

---

## 0. 전제 계약

| 계약 | 파일 + 심볼 | 요점 |
|---|---|---|
| 백엔드 API | `src/shared/types/api.ts` — `AgentOfficeApi` | 렌더러가 부르는 전체 표면. 구현은 `src/renderer/ipc/tauriApi.ts::tauriApi`(`@tauri-apps/api`의 `invoke`/`Channel`/`listen` 어댑터). 커맨드·이벤트 이름은 `src/shared/ipc.ts`의 `Commands`/`Events` 상수만 쓴다 |
| 출력 스트림 | `tauriApi.onData(agentId, cb)` | 에이전트당 `Channel` 하나를 JS 쪽 콜백 집합으로 팬아웃한다. 반환값은 동기 unsubscribe(멱등). 구독자 하나가 던져도 다른 구독자와 채널은 산다 |
| 세션 상태 | `@shared/types` — `SessionStatus = SessionState \| "idle"` | `needs_input`은 상태가 아니라 알림 유무에서 파생한다. 스토어는 `disposed`를 `exited`로 흡수한다(§2.4) |
| 알림 | `@shared/types` — `NotificationEvent`, `notificationType()` | 백엔드가 발급한 `id`를 그대로 쓴다(clear 동기화 때문에 재발급 금지). `type`은 `source`에서 파생: hook→question, stop→done, bell→info |
| B 연동 | `src/renderer/office/bus.ts` — `OfficeBus` | `sceneRef` 직접 호출이 아니라 bus 구현 + `<OfficeCanvas profiles>` prop으로만 만난다(B §1.3) |
| 스프라이트 미리보기 | `office/gen/characterFactory.ts::generateSpritePreview` | 순수 함수 직접 호출. 씬을 거치지 않는다 |

계약 타입은 `src/shared/types/{common,session,notification,profile,diary,memo,usage,settings,markdown,git,api}.ts` 분할 파일에 있고 `src/shared/types.ts`가 재수출한다.

## 0.5 현재 파일 레이아웃 (2026-10-14)

`src/renderer/`는 기능별 폴더로 모듈화돼 있다.

| 폴더 | 담당 (정본 절/문서) |
|---|---|
| `ipc/` | `tauriApi` 어댑터, `sessionBridge`(스토어 배선 + `officeBus`), `windowFocus`, `osNotify`, `sessionAdoption`, `subagentCounts`, `mascotBridge`, `settingsSyncBridge`, `displayFocusBridge` (§2.4) |
| `store/` | zustand `appStore`, `selectors`, `persist`, `profileLoad`, `mascotLights` (§2) |
| `terminal/` | `TerminalRegistry`/`TerminalHost`/`TerminalOverlay`/`AgentTabStrip`/`TerminalSummaryBar`, IME·스크롤백 보호, xterm 팔레트 (§3, §11) |
| `office/` | PixiJS 씬 — 서브시스템 B 문서 |
| `layout/` | `UIChrome`/`TopBar`/`BottomBar`/`ProfileLoadBanner` (§1) |
| `notification/` | 알림 티커 (§5) |
| `profile/` | 프로필 생성·편집 (§4) |
| `theme/`, `styles/` | 테마 레지스트리·픽셀 토큰·폰트 (§6) |
| `workdir/`, `markdown/` | 작업 폴더 팔레트·문서 뷰어 (§10) |
| `memo/` | 에이전트별 포스트잇 메모 (§13) |
| `diary/` | 캐릭터 일기·내보내기 (§12) |
| `analytics/`, `usage/`, `timeline/`, `sessionlog/` | 활동 분석, 사용량, 턴 타이밍, 세션 로그 — 각 design 문서 |
| `workpanels/` | 업무 패널 — [work-panels-design.md](work-panels-design.md) |
| `run/`, `repository-audit/`, `awards/`, `mascot/` | 실행 레시피, 저장소 점검, 이 달의 우수사원, 마스코트 창 — 각 design 문서 |
| `labels/`, `desk/`, `agent/`, `portrait/`, `sprite/` | 머리 위 작업 라벨, 책상 주인 지정, 캐릭터 조작·확인 대화상자, 초상·Codex 생성, 스프라이트 편집 |
| `settings/`, `about/`, `sound/`, `power/`, `ui/`, `shared/` | 설정(CLI 제어 승인 UI 포함)·정보·사운드·잠자기 방지·공용 위젯(`ContextMenu` 등)·렌더러 공용 유틸 |
| `i18n/` (+ `src/shared/i18n/`) | i18next 런타임·번역 카탈로그·언어별 프롬프트/판정 규칙/낱말 (§14, [i18n-design.md](i18n-design.md)) |

부트 순서: `main.tsx`가 `pixi.js/unsafe-eval` → `./i18n` → `applyTheme(store.theme)`를 render 전에 동기로 처리하고, `<App/>`을 그린 뒤 `bootstrap.ts::bootApp`을 부른다. `bootApp`은 프로필 로드 → hydrate → 설정 → `installSessionBridge` → 세션 입양 → `installPersistence` → … 순서를 지킨다(순서 근거는 파일 머리 주석).

## 1. 앱 레이아웃 & 윈도우 설계

### 1.1 단일 창 + z-index 레이어 스택

> 예외(이슈 #72): 앱에는 두 번째 웹뷰 창 `mascot`이 있다 — 투명·최상단·
> 장식 없는 데스크톱 마스코트. 이 절의 "단일 창" 전제는 **앱 본체 UI**에 대한
> 것이고, 마스코트는 스토어도 Pixi도 없는 얇은 소비자라 아래 레이어 스택과
> 무관하다. 설계는 [mascot-window-design.md](mascot-window-design.md)가 정본.

창은 표준 프레임을 쓴다. 본체 UI는 `App.tsx::App`의 절대 위치 레이어 스택이다(값은 각 CSS 파일이 정본).

| z | 레이어 | 파일 |
|---|---|---|
| 0 | `.office-viewport` 안의 `OfficeCanvas` | `App.tsx`, `layout/layout.css` |
| 3 | `.office-lights-off` — 전원 퇴근 소등(`selectors.ts::useLightsOff`) | `layout/layout.css` |
| 5 | `TaskLabelLayer` — 머리 위 작업 라벨 | `labels/labels.css` |
| 10 | `UIChrome` — TopBar·업무 패널·티커 컬럼·BottomBar | `layout/UIChrome.tsx` |
| 20 | `TerminalOverlay` | `layout/layout.css` `.terminal-overlay` |
| 25 | `AgentHoverCard` | `portrait/portrait.css` |
| 30 | `.modal-root` / `.modal-backdrop` — 각 다이얼로그 | `layout/layout.css` |
| 31 | 저장소 점검 오버레이 | `repository-audit/repositoryAudit.css` |
| 40 | 작업 폴더·마크다운·실행 레시피·일기·세션 로그·메모 아카이브 오버레이 | 각 폴더 CSS |
| 50 | 마크다운 편집기 내부 백드롭 | `markdown/markdown.css` |
| 100 | `.crt-overlay`(pipboy 전용) | `layout/layout.css` |
| 1000 | `ContextMenu` | `ui/contextMenu.css` |

- **`UIChrome` 루트는 `pointer-events: none`** 이고 바·티커 카드만 `auto`로 되돌린다. 빈 영역 클릭이 캔버스에 닿아야 캐릭터 클릭이 된다.
- 상시 마운트 + self-gate: 다이얼로그·오버레이는 항상 마운트돼 있고 각자 스토어 상태(`modal.kind` 등)로 `null`을 렌더한다. `FirstRunDialog`만 `settingsFirstRun`으로 게이트한다.
- 업무 패널이 켜지면 `.app-root`에 `--work-left`/`--work-right`가 실려 좌우 폭을 비운다(`workpanels/panelLayoutStore.ts::panelExtent`).

### 1.2 터미널 패널: 중앙 오버레이

`terminal/TerminalOverlay.tsx::TerminalOverlay` — 기본(windowed) 72%×72% 중앙 패널, 배경 딤. 드로어 대신 오버레이를 쓰는 이유: 에이전트 간 빠른 문맥 전환, 넓은 고정 폭으로 PTY resize 빈도 감소, 게임 팝업 메타포. 확대(filled) 모드는 §11.

- **닫힘은 루트 `display:none`이지 조건부 렌더가 아니다.** `AgentTabStrip`/`TerminalHost`와 그 아래 xterm 인스턴스는 오버레이가 닫혀도 살아 있어야 한다(keep-alive, §3).
- 닫기 경로 3개: 헤더 X, `Cmd/Ctrl+W`, 패널 밖 backdrop mousedown. **Esc는 닫기에 쓰지 않는다** — vim 등 TUI가 실제 Escape를 받아야 한다.
- 패널 안 구성: `AgentTabStrip`(열린/최근 탭, `Cmd/Ctrl+1..9` 전환), `TerminalSummaryBar`, `TerminalHost`, `PostItWidget`(§13), `UsageFloat`.

### 1.3 상/하 바

- `layout/TopBar.tsx::TopBar`(40px) — 좌측 앱 타이틀, 우측 에이전트 수·가동·대기 카운트. 바 전체가 "터미널 다시 열기" 버튼이다: 최근 탭(없으면 첫 에이전트)으로 `officeBus.emitAgentClicked`를 부른다. 걸어 다니는 캐릭터를 맞히기 어려워서 둔 큰 클릭 대상이다.
- `layout/BottomBar.tsx::BottomBar`(48px) — 좌→우: `＋ New Agent`, 출근(퇴근자 배지), 전체 출퇴근 메뉴, 가동/대기 요약, `UsageWidget`, 기록 메뉴(분석·우수사원), 설정, 정보, `🎨 {풍경}·{테마}` 메뉴(§6.4·§6.5), 음소거. 순서와 항목 근거는 파일 머리 주석이 정본이다. 800px 창에서 이미 폭이 포화라 라벨은 짧게 유지한다.
- `layout/ProfileLoadBanner.tsx` — 프로필을 못 읽었을 때의 읽기 전용 배너와 재시도(`store/profileLoad.ts::retryProfileLoad`).

### 1.4 우측 알림 티커 컬럼

`.ticker-column`은 TopBar 아래 ~ BottomBar 위, 우측 고정 폭(280px)이다. `timeline/SessionTimePanel`이 같은 컬럼의 티커 위에 놓인다 — 별도 fixed 오버레이였을 때 티커·TopBar와 겹쳤던 것을 한 컬럼으로 묶어 구조적으로 막았다. 카드 규칙은 §5.

---

## 2. 상태관리 — zustand

### 2.1 근거

- 세션 이벤트는 React 밖의 IPC 콜백에서 온다. zustand는 `getState()/setState()`를 컴포넌트 밖에서 부를 수 있어 브리지가 훅에 의존하지 않는다.
- selector 구독으로 "티커만", "탭스트립만" 같은 세밀한 리렌더가 된다.
- `subscribeWithSelector`로 상태 변화 부수효과(배지, bus 중계, 영속화)를 건다.

### 2.2 스토어 타입(`store/types.ts`)

`AgentProfile`·`NotificationEvent`·`PersistedState`·`SessionStatus`·`notificationType`은 `@shared/types`에서 import해 재수출만 한다. 스토어에서 다시 선언하면 wire 계약과 조용히 어긋난다. 스토어 전용 타입만 여기서 정의한다.

| 타입 | 내용 |
|---|---|
| `SessionRuntime` | `{ agentId, status, cols, rows, lastActivityAt }`. 공유 `SessionState`와 이름이 겹치지 않게 따로 이름 지었다 |
| `Notification` | 백엔드 id 그대로 + `type`(파생) + `message` + `excerpt`(≤80자) + `createdAt` |
| `ModalState` | `kind` 판별 유니온. `profile-create`/`profile-edit`, 확인 대화상자(삭제·재시작·재개·종료·퇴근·전원 퇴근·앱 종료), `settings`/`analytics`/`usage`/`about`/`awards` |
| `AgentTaskLabel` | 머리 위 작업 라벨의 런타임 소스(첫/최신 프롬프트, LLM 목표·요약, 도구·내레이션 꼬리, cwd) |

### 2.3 앱 스토어(`store/appStore.ts::useAppStore`)

한 파일에 여러 슬라이스가 있다. 액션 시그니처는 `AppState` 인터페이스가 정본이다.

| 슬라이스 | 필드 / 액션 | 계약 |
|---|---|---|
| 프로필 | `agents`, `agentOrder`, `addAgent(profile, { startSession? })`, `updateAgent`, `removeAgent`, `assignDesk` | `addAgent`는 세션을 `starting`(또는 `startSession:false`면 `idle`)으로 씨앗한다. `hydrate`는 전부 `idle`로 만들고 `archetype` 없는 레거시 프로필을 `human`으로 채운다 |
| 세션 | `sessions`, `setSessionState`, `setSessionSize`, `clockOut`/`clockIn`, `terminalEpochs`/`bumpTerminalEpoch` | 에폭을 올리면 `TerminalHost`가 그 마운트를 강제로 다시 만든다(재시작) |
| 알림 | `notifications`(최신이 앞), `pushNotification`, `clearNotificationsFor`, `clearNotificationByIds` | `pushNotification`은 **활성 터미널이 그 에이전트이고 창이 포커스일 때만** 억제한다(이슈 #39). 창이 백그라운드면 터미널이 열려 있어도 티커·배지·사운드로 알린다 |
| 터미널 | `activeTerminalAgentId`(null = 닫힘), `recentAgentIds`(LRU), `openTerminal`, `closeTerminal`, `terminalViewMode` | `openTerminal`은 active·recent 갱신과 그 에이전트 알림 클리어를 한 번에 한다 |
| 창·표시 | `windowFocused`, `muted`, `vacationMode`, `theme`, `scene`, `xtermTheme` | `setTheme`/`setScene`/`setXtermTheme`/`setTerminalViewMode`는 액션 안에서 DOM·localStorage 부수효과를 직접 수행한다 — 스토어가 React 밖에서도 호출되기 때문 |
| 외형 캐시 | `portraits`, `spritePreviews`, `minimiPreviews` | 디코드 완료가 프로필 필드를 안 바꾸므로 B 재동기화 신호로 쓴다(B §1.2) |
| 시간·사용량 | `timeTracking`, `sessionUsage*`, `todayWorkedBaseMs`, `applyActivityEvent`, `applyTurnUsage` 등 | [session-analytics-design.md](session-analytics-design.md), [usage-design.md](usage-design.md) |
| 설정 | `appSettings`(기본값 `DEFAULT_APP_SETTINGS` 형태), `hydrateSettings`, `receiveAppSettings`, `updateAppSettings`, `completeFirstRun` | Rust `AppSettings` 영속. 테마·풍경·xterm·뷰 모드 같은 순수 프런트 취향값은 여기가 아니라 localStorage다 |

`store/selectors.ts`: 매 렌더 새 배열을 만드는 selector는 `useShallow`로 감싼다(무한 리렌더 방지). `useAgentList`(B에 넘기는 안정 참조), `useAgentCount`, `useRunningCount`, `pendingAgentIds`/`usePendingCount`, `useClockedOutAgents`, `useAwayFromDeskCount`, `useLightsOff`, `useSessionTimeRows`, `useTodayWorkedMs`, `pickMascotTarget`.

`store/persist.ts::installPersistence`: `agents`와 `vacationMode` selector만 구독해 500ms 디바운스로 `tauriApi.saveState`를 부른다. PTY 출력·알림·탭 전환 같은 런타임 변화로는 저장하지 않는다. `flushPersistence`는 종료 경로가 쓴다.

### 2.4 IPC ↔ 스토어 브리지 + OfficeBus 구현(`ipc/sessionBridge.ts`)

- `officeBus` — B의 `OfficeBus`를 스토어 백킹으로 구현한 싱글턴. `onNotificationChanged`·`onSessionStateChanged`·`onVacationModeChanged`·`onAwardeeChanged`는 구독 즉시 현재값을 재생한다.
- `officeBus.emitAgentClicked(agentId)` — 호버 카드 숨김 → `ensureSession`(세션 없는/종료된 에이전트면 재기동) → `openTerminal` → `tauriApi.clearNotifications`. **티커 카드·TopBar·캐릭터 클릭이 모두 이 한 경로를 탄다**(진입점끼리 어긋나지 않게).
- `installSessionBridge()` — 부팅 시 1회. `onSessionState`(스토어에는 `disposed`→`exited` 흡수, 시간 정산·bus 중계에는 원본 상태), `onNotification`(소환 직후 억제 창 `agent/summonSuppress.ts`, OS 알림은 `!windowFocused`일 때만 `osNotify.ts::maybeSendOsNotification`), `onNotificationCleared`, 서브에이전트 수, 잠자기 방지 재계산, 알림·음소거 변화 → bus 중계 + `setBadgeCount`.
- `ensureSession`, `runGuardedCreateSession`(`CREATE_SESSION_TIMEOUT_MS` 15초), `reserveSessionReplacement` — 세션 생성 경합 가드.
- **`onData`는 여기서 배선하지 않는다.** 고빈도 스트림이라 스토어를 거치면 리렌더 폭주가 난다. `TerminalRegistry`가 직접 구독한다(§3).

---

## 3. TerminalHost — N개 xterm keep-alive

### 3.1 설계 원칙

1. 에이전트마다 `Terminal` 인스턴스는 정확히 하나다. 숨길 때 dispose하지 않는다.
2. 모든 `TerminalMount`는 DOM에 상주하고, 비활성은 `display:none`이다.
3. 보일 때만 fit + PTY resize + focus를 한다.

### 3.2 display:none과 fit 타이밍

숨은 동안 컨테이너 크기가 0이라 그때 `fit()`하면 cols/rows가 깨진다. 그래서 **보이게 된 다음 프레임(`requestAnimationFrame`)에 fit**한다. 숨은 터미널은 레이아웃 비용을 내지 않고, 스크롤백은 xterm 내부에 남는다.

### 3.3 `terminal/TerminalRegistry.ts::terminalRegistry`

| 메서드 | 계약 |
|---|---|
| `ensure(agentId)` | 없으면 `Terminal` 생성(모노스페이스 `"SF Mono", "Menlo", "Consolas", "Liberation Mono", monospace` 13px, lineHeight 1.2, scrollback 5000) + `FitAddon` + `SerializeAddon`. 키 입력→PTY는 `imeBridge.ts::createImeBridge`가, PTY→화면은 `onData` 직접 `write`가 맡는다(`scrollbackGuard.ts::createScrollbackGuard`가 `ESC[3J`를 떼어 낸다) |
| `attach(agentId, host)` | 컨테이너를 붙이고 최초 1회만 `term.open` |
| `activate(agentId, onResize)` | 다음 프레임에 fit → `syncViewport` → `onResize` → focus. 입양된 세션이면 크기가 그대로일 때만 1회 redraw nudge(rows-1 → rows) |
| `refit(agentId, onResize)` | ResizeObserver 경로. 포커스는 건드리지 않는다 |
| `markAdopted(ids)` | 부팅 입양 직후 1회. 다음 `activate`에 nudge를 건다 |
| `serializeAll`, `getPlainText` | 종료 핸드오프 스냅샷([session-handoff-design.md](session-handoff-design.md))과 텍스트 추출 |
| `setTheme(theme)` | 이후 생성분 팔레트를 보관하고 살아 있는 전 인스턴스에 `term.options.theme`을 다시 넣는다(§3.6) |
| `destroy(agentId)` | 진짜 폐기는 여기뿐이다(에이전트 삭제·재시작) |

`syncViewport`: 숨은 동안 출력이 오면 xterm 뷰포트의 스크롤 영역 높이가 한 화면 짧게 굳는다(xterm.js #494). 다시 보일 때 사적 API `syncScrollArea(true)`로 강제 재계산한다. 없으면 조용히 넘어간다.

### 3.4 `terminal/TerminalHost.tsx`

- `TerminalHost` — `status !== "idle"`인 에이전트마다 `TerminalMount`를 그린다. key는 `${id}#${epoch}`라 재시작 시 강제 리마운트된다.
- `TerminalMount` — 최초 마운트에 `attach`(언마운트해도 destroy하지 않음), active가 되면 `activate`, active인 동안만 ResizeObserver(`RESIZE_DEBOUNCE_MS` 120ms) → `refit`. 두 경로 모두 `setSessionSize` + `tauriApi.resize`를 부른다. `exited`면 재기동 배너(`ensureSession`)를 띄운다.

### 3.5 포커스

`activate`가 `term.focus()`를 해서 오버레이가 열리자마자 입력이 된다. 닫을 때 별도 blur는 없다. Esc는 셸로 보낸다(§1.2).

### 3.6 xterm 팔레트

터미널 색은 테마 레지스트리의 일부이되 **따로 고정할 수도 있다.**

- 원천 두 곳: 앱 테마마다 하나씩 있는 `theme/themes.ts::THEMES[id].xterm`, 그리고 앱 테마와 짝이 없는 터미널 전용 `terminal/palettes.ts::XTERM_PALETTES`(Catppuccin latte/frappe/macchiato/mocha, Apple Clear dark/light). 전용 팔레트는 원본 스킴 값을 그대로 옮긴 것이라 앱 테마의 대비 계약을 적용하지 않는다.
- 해석·영속: `terminal/theme.ts` — `XtermThemeOverride = ThemeId | XtermPaletteId | "auto"`(기본 `"auto"` = 앱 테마를 따름), `resolveXtermTheme`, `xtermBackground`, `loadStoredXtermThemeOverride`/`persistXtermThemeOverride`(`localStorage "agent-office.xterm-theme"`). 스토어·xterm 런타임 의존이 없는 순수 모듈이다.
- 라이브 반영: `App.tsx`의 효과가 `[theme, xtermTheme]` 변화 시 `terminalRegistry.setTheme(resolveXtermTheme(...))`와 `applyTheme.ts::applyTerminalBg`를 부른다. 배경은 `--term-bg`로 `documentElement`에 주입되고, `.terminal-host`는 `var(--term-bg, …)`를 쓴다.
- 앱 테마별 원칙: midnight는 테마 도입 이전 값 보존, daylight·sakura는 밝은 배경에 맞게 노랑·흰색 계열을 어둡게 보정, pipboy는 인광 초록 쪽으로 기울이되 빨강 에러·노랑 경고 구분은 남긴다(완전 단색화 금지).
- 무결성: `theme/__tests__/theme.test.ts`(전 테마 순회), `terminal/__tests__/xtermTheme.test.ts`, `xtermPalettes.test.ts`.

---

## 4. 프로필 생성 플로우

### 4.1 랜덤 초안(`profile/generate.ts`)

- `generateDraft()` — 이름·역할·성격 낱말은 `i18n/wordlists.ts::currentWordlists()`에서 **호출 시점에** 고른다(모듈 로드 때 굳히면 언어를 바꿔도 옛 언어 초안이 나온다). 시드는 `nanoid(8)`, 아키타입은 `"auto"`, 성격 프롬프트는 `profile:generate.personalityPrompt` 번역문으로 채운다.
- `draftToProfile(draft, deskIndex)` — 공백 정리, `normalizeColors`(빈 색 슬롯 제거, 남는 게 없으면 `undefined`), `"auto"` 아키타입을 시드 추첨으로 확정(`office/gen/archetypes.ts::pickArchetype`).
- `mergeLegacyNote` — 레거시 `note`를 성격 프롬프트에 합치는 순수 함수(§4.2.3).
- 난수 시임: `pick`은 `Math.random`을 직접 읽는다. 테스트는 `vi.spyOn(Math, "random")`로 고정한다.

### 4.2 ProfileDialog(`profile/ProfileDialog.tsx`)

다이얼로그는 뼈대다. 상태는 훅 셋, 화면은 섹션 셋에 나눈다.

| 파일 | 역할 |
|---|---|
| `useProfileDraft.ts` | 초안 상태, 편집 모드 진입 시 기존 값 로드, 파생 색 |
| `useAppearanceImages.ts` | 초상·스프라이트·미니미 생성·삭제·프롬프트 |
| `useCharacterIo.ts` (+ `characterIo.ts`) | 캐릭터 번들 내보내기/가져오기(이슈 #77) |
| `sections/IdentitySection.tsx`, `AppearanceSection.tsx`, `TerminalSection.tsx` | 정체성 / 외형 / 터미널(cwd·셸·시작 명령·키보드 사운드 등) |
| `ArchetypePicker.tsx`, `ColorPickerDialog.tsx`(+ `colorPickerMath.ts`) | 종족 선택, 팔레트 슬롯 색 오버라이드 |

저장(`onSave`): 생성 모드는 초안을 정규화해 `addAgent`(세션 `starting`) → `tauriApi.createSession`(PTY 시작) → 닫기. 캐릭터 등장은 `profiles` prop 변화로 B의 `syncAgents`가 처리한다. 편집 모드는 프로필을 제자리에서 고치고 새 세션을 시작하지 않는다. 스프라이트 미리보기는 `generateSpritePreview(seed, …, archetype, colors)`를 직접 부른다.

`ProfileDialog`는 상시 마운트(닫힘 = `null` 렌더)다. 그래서 늦은 비동기 응답은 unmount가 아니라 편집 세션 토큰으로 무효화한다(§4.2.1).

"외형" 섹션의 스프라이트 카드 안에는 **미니미(서브에이전트 분신) 소섹션**이 있다 — 현재 미니미 프리뷰(커스텀이 없으면 스프라이트 축소판임을 안내) + 업로드/변경/제거. 업로드는 `SpriteEditor`를 `target="minimi"`로 재사용해 크롭/줌 UX를 공유하되 **단일 N×N 프레임**만 저장한다. 저장·로드·삭제·번들 왕복의 전체 계약은 [subsystem-b-office.md](subsystem-b-office.md) §4.6 표가 정본이다.

#### 4.2.1 "외형" 섹션의 모드 탭 (kbm #2fa)

외형을 **만드는 방법**은 두 갈래인데 예전에는 버튼이 평면으로 늘어서 있어
무엇을 눌러야 할지 알기 어려웠다. 지금은 한 번에 한 쪽만 보여 준다.

- **탭 바깥(항상 보임)**: 스프라이트/미니미/초상 프리뷰 카드, `스프라이트 재생성`,
  `커스텀 제거`·`초상 제거`, 미니미 업로드/변경/제거, **키 컬러**(§4.2.4).
  아래쪽의 `초상화 추가 프롬프트`·`스프라이트 추가 프롬프트`·`미니미 추가 프롬프트`
  입력도 두 모드 공통이다(직접 만들기/Codex 두 경로의 프롬프트에 함께 반영).
- **`[직접 만들기]` 탭**: `초상 프롬프트 복사` + `이미지 업로드/변경`,
  `스프라이트 프롬프트 복사` + `스프라이트 업로드/변경`, `미니미 프롬프트 복사` +
  `미니미 업로드/변경`. 프롬프트를 아무 이미지 생성 도구에나 넣고 결과를 올리는
  경로다.
- **`[Codex로 생성]` 탭** (`src/renderer/portrait/CodexGenPanel.tsx`): 로컬
  `codex` CLI 설치 탐지 한 줄 + `초상 생성`/`스프라이트 생성`/`미니미 생성` +
  진행·결과·오류
  캡션. 패널은 **표시 전용**이고 생성 오케스트레이션(편집 세션 토큰 가드,
  결과를 편집기로 넘기기)은 `ProfileDialog`의 `useAppearanceImages` 훅이 들고
  있다 — `ProfileDialog`는 상시 마운트(닫힘 = `return null`)라 늦은 응답
  무효화를 unmount가 아니라 세션 토큰으로 해야 하기 때문이다.

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

- 프로필에 `minimiRequest`(‘미니미 추가 프롬프트’) 필드가 있다. 적으면 그대로
  `Familiar: …` 줄로 들어간다.
- **비면 자동 위임 문구**(`autoMinimiRequestLine`)가 대신 들어간다 — "따로 요청이
  없으니 주인에게 어울리는 소환수를 알아서 디자인하라"는 한 줄. 사용자가 아무것도
  적지 않아도 본체와 어울리는 결과가 나오게 하기 위한 기본값이다.
- 주인의 `spriteRequest`는 `Master's appearance:` 줄로만 실린다 — 소환수가
  주인을 닮게 하는 맥락이지 소환수 자신의 묘사가 아니다. (예전에는 비면
  `appearance`로 폴백했지만, 칸이 §4.2.4처럼 갈라지며 폴백을 없앴다.)
- 본문은 16×16 단일 프레임 지시(작게 읽히는 단순 실루엣)이고, codex 버전은
  마지막 규격 줄만 1024×1024 투명 배경으로 갈아 끼운다. 결과는
  `SpriteEditor target="minimi"`에 `initialImage`로 프리로드된다.

#### 4.2.3 ‘메모’를 ‘성격 프롬프트’로 통합

예전에는 **메모**(자유 설명, 초상 프롬프트에만 반영)와 **성격 프롬프트**(세션
시스템 프롬프트에 주입)가 따로 있었는데, 둘 다 "이 캐릭터가 어떤 존재인가"를
적는 칸이라 사용자가 늘 헷갈렸다. 지금은 **성격 프롬프트 하나**만 둔다.

- 새 캐릭터의 랜덤 초안은 `personalityPrompt`에 `"{성격} 성격"`을 채운다
  즉 랜덤 성격이 실제로 세션에도 반영된다.
- 초상 프롬프트의 `Personality / notes:` 줄은 이제 `personalityPrompt`를 받는다.
- 스키마에서 `note`는 사라졌다. 남아 있는 값은 **로드 시 자동 통합**된다:
  `ProfileStore::load` → `migrate_loaded`가 `personality_prompt`에 합치고
  레거시 필드를 비운다(Rust `AgentProfile.legacy_note`는 `rename="note"` +
  `skip_serializing` — 읽기만 하고 다시 쓰지 않으므로 다음 저장에서 키 자체가
  사라진다). 편집창을 한 번도 열지 않은 사용자도 통합된다. 캐릭터 번들
  가져오기도 같은 규칙을 쓴다(`applyBundleToDraft` + `mergeLegacyNote`,
  `PortableProfile.legacyNote`). 이미 합쳐진 문자열을 다시 병합해도 중복되지 않는다.
- 포스트잇 메모(§13)와는 무관하다 — 그쪽은 사람이 쓰는 작업 기억이다.

#### 4.2.4 그림 프롬프트 3칸과 키 컬러 노출 (kbm #2fh)

**칸 이름이 그림 종류를 그대로 가리킨다.** 예전에는 `외모 힌트`(초상에 쓰이면서
스프라이트의 폴백이기도 함)와 `픽셀아트 의뢰 문구`가 섞여 있어, 어느 칸이 어느
그림에 어떻게 쓰이는지 알 수 없었다. 지금은 세 칸이 **각자 자기 그림에만** 덧붙는다.

| 필드 | 라벨 | 들어가는 프롬프트 |
| --- | --- | --- |
| `portraitRequest` | 초상화 추가 프롬프트 | `buildPortraitPrompt`의 `Appearance details:` |
| `spriteRequest` | 스프라이트 추가 프롬프트 | `buildSpritePrompt`의 `Details:` (+ 미니미의 `Master's appearance:`) |
| `minimiRequest` | 미니미 추가 프롬프트 | `buildMinimiPrompt`의 `Familiar:` (비면 자동 위임 문구) |

칸 사이 폴백은 **없다**. 옛 `appearance` 값은 로드 시 1회 마이그레이션으로
`portraitRequest`·`spriteRequest` 양쪽(비어 있는 쪽만)에 복사돼 그림 결과가
달라지지 않는다 — Rust `legacy_appearance`(`rename="appearance"` +
`skip_serializing`)가 그 1회성을 보장한다. 값을 옮긴 뒤 필드를 즉시 비우므로,
사용자가 나중에 한쪽을 지워도 다시 채워지지 않는다.

**키 컬러 노출.** 시드+아키타입이 낳는 팔레트는 `colorHints` 줄로 프롬프트에
실리지만 내부 자료라, 몇 번을 다시 생성해도 "왜 이 색인지" 알 수 없었다. 이제
편집창 외형 섹션이 그 색을 그대로 보여 준다(`.key-colors` — 칩+한국어 라벨+hex,
`title`에 영문 프롬프트 문구). 단일 출처는 `archetypes.ts`:
`ArchetypePromptDescriptor.keyColors`(아키타입마다 `Hair/Fur/Skin/Plating/
Chassis/Clothing/Accent/Body` 슬롯 조합)에서 `colorHints` 문자열이 파생되고,
UI는 `keyColorsFor(seed, archetype)`로 같은 값을 읽는다 — 표시와 프롬프트가
어긋날 수 없다. 색을 바꾸는 방법은 시드 변경(`스프라이트 재생성`)이나 아키타입
변경뿐이라는 점도 힌트로 적어 둔다.

---

## 5. 알림 티커 UX

### 5.1 카드 내용(`notification/NotificationTicker.tsx`)

카드 하나 = 타입 아이콘(`TYPE_ICON`: ❓question / ✅done / ℹ️info) + 에이전트 이름 + `excerpt`(≤80자) + 상대 시각(`relTime` — 45초 미만은 `time.justNow` 번역문, 그 위는 `shared/relativeTime.ts::relativeTimeFormatter`가 현재 언어로 분·시 단위 문구를 만든다).

### 5.2 스택/오버플로

- 최신이 위, `MAX_VISIBLE` 5장. 넘치면 "+N more" 요약 카드.
- **에이전트당 최신 1건만** 보인다: `notification/dedupe.ts::dedupeLatestPerAgent`(순수, 최신순 입력 가정). 내부 큐에는 다 있고 렌더 때만 합친다.

### 5.3 상호작용

- 카드 클릭 → `officeBus.emitAgentClicked(agentId)`(§2.4). 캐릭터·TopBar 클릭과 같은 경로라 세션 재기동·터미널 열기·스토어/백엔드 알림 클리어가 한 번에 된다.
- `notifications` 변화 → `sessionBridge`가 hasPending을 B에 중계하고 배지를 갱신한다.

### 5.4 애니메이션

진입만 있다: `ticker.css`의 `@keyframes tickerIn`(살짝 옆에서 페이드인). 퇴장은 즉시다(과한 모션 지양).

### 5.5 Dock/Taskbar 배지

`tauriApi.setBadgeCount(n)` → Tauri 커맨드 `set_badge_count`(`src-tauri/src/ipc/commands/misc.rs`, `WebviewWindow::set_badge_count`). n은 대기 중인 **에이전트 수**(`pendingAgentIds`)다. `muted`면 0이고, 음소거 토글 즉시 다시 맞춘다.

---

## 6. 레트로/타이쿤 비주얼 스타일링

### 6.1 픽셀 폰트

- **DungGeunMo(둥근모꼴)** — 완성형 한글 전 영역, 공개 폰트. UI 텍스트용.
- **Galmuri11** — SIL OFL 1.1. 등폭 정렬이 안정적인 픽셀 폰트.
- 파일은 `src/renderer/styles/fonts/`(`.woff2` + 라이선스 텍스트)에 번들하고 `styles/pixel.css`의 `@font-face`로 로드한다. CDN을 쓰지 않는다(오프라인 보장). 모든 소비처의 `font-family` 끝에 `monospace`를 둬 로드 실패에도 읽힌다.
- **터미널 셀에는 픽셀 폰트를 쓰지 않는다** — 표준 모노스페이스(§3.3). 픽셀 감성은 UI 크롬 몫이고 터미널은 가독성이 우선이다.

### 6.2 CSS 접근: 기능 폴더별 평문 CSS + 전역 토큰

- Tailwind도 CSS Modules도 쓰지 않는다. 각 기능 폴더가 자기 `.css`(예: `layout/layout.css`, `terminal/terminal.css`, `notification/ticker.css`)를 컴포넌트에서 import하고, 클래스 이름에 폴더 접두(`wd-`, `md-`, `slog-` 등)를 붙여 충돌을 피한다.
- 전역: `styles/tokens.css`(색·단위 토큰의 부트/무JS 폴백), `styles/pixel.css`(폰트 + `.pixel-panel`/`.pixel-btn` 공용 클래스).

### 6.3 2px 픽셀 패널 미학

- `.pixel-panel`/`.pixel-btn`(`styles/pixel.css`): 2px 단색 테두리 + inset 계단식 하이라이트/섀도 + 외곽 하드 라인, `border-radius: 0`, `image-rendering: pixelated`. 버튼 `:active`는 하이라이트와 섀도를 뒤집는다.
- 하드 트랜지션 위주, 부드러운 그라디언트 지양.
- 색은 하드코딩하지 않고 CSS 토큰(`var(--bg-panel)` 등)만 쓴다. 값은 §6.4 테마가 주입한다.

### 6.4 테마 레지스트리(`theme/themes.ts`)

`THEMES`가 색의 단일 원천이다. 한 테마(`ThemeDef`)는 세 팔레트를 함께 든다.

| 필드 | 소비처 | 적용 |
|---|---|---|
| `css`(`CSS_TOKEN_KEYS`) | DOM 전체 | `theme/applyTheme.ts::applyTheme`이 `documentElement`에 `data-theme` + 인라인 커스텀 프로퍼티로 주입(`tokens.css` `:root` 폴백을 이김) |
| `pixi`(`TILE_PALETTE_KEYS` + background/text) | 오피스 씬 | `App → OfficeCanvas → useOfficeScene → OfficeScene.setTheme()`(B §1.2) |
| `xterm`(`ITheme`) | 터미널 | §3.6 |

- 테마: `daylight`(기본·밝음) / `midnight`(테마 도입 이전 룩) / `sakura`(파스텔 핑크) / `pipboy`(인광 초록 CRT). 순서는 `THEME_ORDER`, 기본은 `DEFAULT_THEME_ID`. 라벨은 번역 키(`labelKey`)다(§14).
- 캐릭터 스프라이트 팔레트(`office/gen/palette.ts`, `archetypes.ts`)와 분석 차트 색은 테마 대상이 아니다 — 에이전트별 절차 생성·데이터 인코딩이라 불변이다.
- 영속: `localStorage "agent-office.theme"`(`THEME_STORAGE_KEY`). 스토어 초기값이 `loadStoredThemeId()`이고 `main.tsx`가 첫 render 전에 `applyTheme`을 동기로 불러 잘못된 테마 플래시가 없다.
- 픽커: BottomBar의 `🎨 {풍경}·{테마}` 버튼 → 공유 `ContextMenu`. "풍경"/"테마" 섹션 헤더로 나뉘고 현재 선택에 ✔. `nextThemeId`는 순서 계약·향후 단축키용으로 남아 있다.
- CRT 오버레이: `theme === "pipboy"`일 때만 `App`이 `.crt-overlay`를 마운트한다. `position:fixed; pointer-events:none`에 순수 CSS 두 겹(스캔라인 + 옅은 비네트)뿐이라 합성 비용이 거의 없고 상호작용을 그대로 통과시킨다.

### 6.5 풍경(scene) 레지스트리

`src/renderer/office/scenes/`가 테마와 **직교한 두 번째 축**이다. 테마가 색을 고르면 풍경은 무대를 고른다. 어떤 조합도 유효하다.

| 필드(`SceneDef`) | 내용 |
| --- | --- |
| `map` | 그 풍경의 타일 맵. 좌석·휴게 공간·보스 자리·줄 슬롯이 전부 여기서 유도된다([subsystem-b-office.md](subsystem-b-office.md) §2.1·§2.2) |
| `resolve(theme)` | 현재 테마로 확정된 `{ background, drawTile }` |

- 풍경: `office`(기본)·`beach`·`valley`·`spaceship`·`castle`·`steppe`·`cruise`·`factory`·`volcano`·`zombie` 10개(`SCENE_ORDER`).
- 소비 경로: `App → OfficeCanvas → useOfficeScene → OfficeScene.setScene()`. 테마 경로와 나란한 별도 효과라 하나만 바뀌어도 각자 처리된다. `setTheme`은 색만, `setScene`은 좌석 히트영역·보스 책상·`OfficeWorld` 맵까지 재구축한다.
- 테마 × 풍경 색: `office`는 `theme.pixi`를 그대로 쓴다. 나머지는 원색 팔레트 한 벌을 `scenes/sceneColor.ts`가 테마별로 변환한다 — daylight·sakura는 한낮(soft), midnight는 황혼(dusk), pipboy는 인광 초록 램프(green). 씬 안 텍스트(휴가 팻말)와 보스 명패 색은 테마 팔레트를 쓴다.
- 영속: `localStorage "agent-office.scene"`(`scenes/sceneStorage.ts`). 스토어 초기값이 `loadStoredSceneId()`이고 잘못된 값은 `office`로 폴백한다. Rust `AppSettings`와 무관한 프런트 취향값이다.
- 픽커: §6.4의 같은 `🎨` 메뉴.

---

## 7. 테스트 시임

테스트는 각 폴더의 `__tests__/`에 있다. vitest 기본 환경은 `node`이고, DOM이 필요한 파일만 `@vitest-environment jsdom`을 단다(`vitest.config.ts`). 테스트 언어는 `src/test-setup.ts`가 ko로 고정한다.

| 파일 | 고정하는 계약 |
|---|---|
| `store/__tests__/appStore.test.ts` (+ `appStore.*.test.ts`) | `addAgent` 세션 씨앗, `openTerminal`의 그 에이전트 알림만 클리어 + active/recent 갱신, 활성·포커스 에이전트 알림 억제, hydrate |
| `store/__tests__/persist.test.ts`, `selectors.*.test.ts` | 영속 트리거 범위·디바운스, selector 파생값 |
| `notification/__tests__/dedupe.test.ts`, `NotificationTicker.test.tsx` | 에이전트당 최신 1건·최신순, 카드 클릭 경로 |
| `terminal/__tests__/TerminalRegistry.test.ts` | keep-alive: `ensure`가 같은 인스턴스를 돌려주고 `destroy` 전까지 유지, `onData` 구독 정리 |
| `terminal/__tests__/TerminalHost.test.tsx`, `TerminalOverlay.test.tsx`, `AgentTabStrip.test.tsx` | 마운트 상주·`display` 토글, 닫기 경로, 단축키 |
| `terminal/__tests__/imeBridge.test.ts`, `scrollbackGuard.test.ts`, `xtermTheme.test.ts`, `xtermPalettes.test.ts`, `terminalViewMode.test.ts` | IME 조합, `ESC[3J` 필터, 팔레트 해석·영속, 뷰 모드 |
| `ipc/__tests__/sessionBridge*.test.ts`, `tauriApi.test.ts`, `windowFocus.test.ts`, `osNotify.test.ts` | 이벤트 → 스토어 배선, bus 재생, 배지, `onData` 팬아웃·unsubscribe |
| `profile/__tests__/generate.test.ts`, `ProfileDialog*.test.tsx`, `characterIo.test.ts` | 초안 생성·정규화, 다이얼로그 저장·Codex 생성 경로, 번들 왕복 |
| `theme/__tests__/theme.test.ts` | 전 테마 레지스트리 무결성(CSS 토큰, xterm 16색 `#rrggbb`, 어두운 테마 명도 관계) |

xterm은 jsdom에서 캔버스 측정이 제한되므로 `@xterm/xterm`을 `vi.mock`으로 스텁한다. `tauriApi`는 모듈 목킹한다. 스토어 테스트는 `beforeEach`에서 `useAppStore.setState(initialState, true)`로 리셋한다.

---

## 8. (결번 — 구현 태스크 분해는 제거됨, 2026-07-20)

---

## 9. 핵심 설계 결정 요약

| 항목 | 결정 | 이유 |
|---|---|---|
| 터미널 패널 | 중앙 오버레이 72% + 탭스트립(확대 모드 §11) | 문맥 전환 집중 UX, 리사이즈 최소화, 게임 팝업 메타포 |
| keep-alive | `TerminalRegistry` 싱글턴 + `display:none` 토글, 닫힘도 `display:none` | React 리마운트와 무관하게 xterm 인스턴스·스크롤백 보존 |
| 고빈도 스트림 | `onData`는 스토어 우회, 레지스트리가 직접 `term.write` | 리렌더 폭주 방지 |
| 상태관리 | zustand + `subscribeWithSelector` | IPC 콜백(React 밖)에서 setState, 세밀 구독 |
| 알림 억제 | 활성 터미널 + 창 포커스일 때만 push 무시, 열람 시 클리어 | 보고 있는 세션은 알릴 필요 없음. 백그라운드면 알림(#39) |
| 클릭 진입점 | 캐릭터·티커·TopBar 모두 `officeBus.emitAgentClicked` | 경로 하나라 동작이 어긋나지 않음 |
| 폰트/CSS | DungGeunMo+Galmuri11 번들, 폴더별 평문 CSS + 토큰 | 오프라인, 픽셀 감성, 장황함 회피 |
| fit 타이밍 | 보인 뒤 rAF에서 fit + 디바운스된 resize IPC | `display:none` 상태의 0크기 fit 방지 |
| 취향값 영속 | 테마·풍경·xterm·뷰 모드·메모 표시는 localStorage | Rust 설정과 분리, 첫 페인트 전 동기 적용 |

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
- **미리보기 링크 탐색**: 현재 문서를 기준으로 해석한 상대 `.md`/`.mdx`/`.markdown`
  링크는 같은 인앱 뷰어에서 열고, 상단 뒤로 버튼으로 직전 문서 상태를 복원한다.
  문서가 아닌 로컬 파일은 OS 기본 프로그램으로, `http(s)`·`mailto`·`tel` 같은 외부
  URL은 OS 기본 브라우저/처리기로 연다. 링크 경로는 쿼리·fragment를 제외하고 판별하며,
  실제 문서 읽기의 root 경계 검증은 `markdown.rs`의 canonicalize 봉쇄를 그대로 거친다.

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

### 10.7 마크다운 팔레트의 최근 본 문서

- 검색어 없이 팔레트를 열면 **최근에 연 문서 최대 4개**가 맨 위로 올라오고 `최근`
  배지가 붙는다. 그 아래는 기존 순서(루트 README 우선 → relPath 사전순)를 그대로 쓴다.
  문서 보기는 대개 조금 전에 보던 파일을 다시 여는 일이라, 매번 이름을 쳐서 찾게
  하지 않으려는 것이다. 검색어가 있으면 최근 기록에 어떤 가산점도 주지 않는다 —
  이름을 친 사람은 퍼지 순위를 기대하기 때문.
- 기록은 `renderer/markdown/recentDocs.ts`가 갖는다. 순수 함수(`pushRecentDoc`,
  `recentPathsFor`)와 localStorage 읽기·쓰기만 있고 스토어·Tauri 의존이 없다
  (`terminal/terminalViewMode.ts`와 같은 결). 키는 `agent-office.markdown.recent-docs`,
  저장 상한 40개, 표시 4개.
- 목록은 root를 섞어 한 배열에 최신순으로 쌓고 **보여줄 때 root로 거른다.** 저장소를
  오가도 각자의 최근 문서가 남게 하려는 것. 같은 문서를 다시 열면 중복 없이 맨 앞으로
  올라간다.
- 기록 시점은 **읽기에 성공한 뒤**다(`openFile`·`openLinkedFile` 모두). 열다 실패한
  파일이나 경합으로 밀린 요청은 남기지 않는다. 목록에서 사라진 파일은 팔레트가
  끌어올릴 때 조용히 건너뛴다 — 지워진 파일이 계속 위에 뜨는 걸 막는다.

### 10.8 핵심 설계 결정 요약

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
| 최근 본 문서 | 빈 쿼리에서만 상위 4개 승격, localStorage 영속 | 방금 보던 문서를 다시 찾게 하지 않기 |

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
쓰는 기록인 것과 정반대다. 프로필의 성격 프롬프트(§4.2.3)와도 무관하다.

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
이 메모 기능의 범위 밖이다. 봇 모드(#57)는 제거됨(2026-10-14, [archive/bot-mode-design.md](archive/bot-mode-design.md)).

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

## 14. UI 다국어(i18n)

정본은 **[docs/i18n-design.md](i18n-design.md)**. 여기서는 UI 서브시스템에 걸리는
부분만 요약한다.

- **런타임**: i18next + react-i18next. 인스턴스는 `src/renderer/i18n/index.ts`가
  `createInstance()`로 만들고 **모듈 로드 시점에 동기로** init한다(`main.tsx`가 `App`보다
  먼저 `import "./i18n"` — 테마 선적용과 같은 이유로 첫 페인트 문구 플래시를 막는다).
  진짜 설정(`AppSettings.language`)은 비동기라, 그때까지는 `localStorage`에 캐시해 둔
  지난번 해석 결과(`agent-office.lang`)를 쓰고 부팅이 `applyLanguageSetting`으로 교정한다.
- **카탈로그**: `src/shared/i18n/locales/{ko,en,ja,zh-Hans,zh-Hant,fr}/*.json`(11 네임스페이스
  × 6 언어). `catalog.ts`가 `import.meta.glob` eager로 모아 `resources`를 만든다. **지원 언어
  목록·설정 드롭다운 항목이 전부 이 glob에서 도출되므로 언어 추가 = 폴더 추가**다. 카탈로그는
  UI 런타임에 의존하지 않는 공통 데이터라 renderer가 아니라 shared에 둔다.
- **React 밖에서도 쓴다**: PixiJS 씬(`OfficeScene`의 휴가 팻말)·zustand 스토어
  (`workdirStore`/`markdownStore`/`sessionLogStore`)·순수 모듈(`relativeTime`,
  `themes.ts`)이 모듈 `t`를 직접 import한다. 컴포넌트는 `useTranslation()`을 써야
  언어 변경 시 리렌더가 걸린다.
- **모듈 최상위에서 `t()`를 부르지 않는다.** 레지스트리 상수(§6.4 테마, §6.5 풍경,
  사운드 팩, xterm 팔레트, 종족)는 번역문 대신 **`labelKey`**를 담고 소비처가 렌더 시점에
  번역한다. `packs.ts`의 목록 정렬 기준도 라벨이 아니라 키다 — 번역문으로 정렬하면 모듈
  로드 시점의 언어로 순서가 굳는다.
- **뷰모델은 문구를 만들지 않는다.** `usageView`·`sessionlog/format`은 완성 문장 대신
  `TextKey {key, params}`(`src/renderer/shared/textKey.ts`)를 돌려주고 렌더 컴포넌트가
  `renderText(text, t)`로 푼다. 보간값 안에 다시 `TextKey`를 중첩할 수 있다(i18next `$t()`는
  고정 키만 되므로 런타임에 고르는 키는 이쪽이어야 한다). 문구 조립이 아예 필요 없으면
  더 가볍게 — `agentStats`는 `label` + `departed: boolean`만 돌려주고 "(퇴사)"는 렌더가 붙인다.
- **백엔드 에러**: Rust는 `"{code}"` 또는 `"{code}: {상세}"`로 실패하고 문구는 프런트가
  고른다(`src/renderer/shared/backendError.ts` → `common:errors.*`). 기술적 상세는
  번역하지 않고 괄호에 원문 그대로 붙인다. `src/shared`도 같은 규약이다
  (`parseCharacterBundle`은 `CharacterBundleError` 코드를 돌려준다 — shared는 renderer에
  의존할 수 없어 `t()`를 부를 수 없다).
- **콘텐츠는 번역하지 않는다.** 일기 본문·수상 소감·포스트잇은 생성
  당시 언어로 쓰인 사료라 원문 그대로 보존한다. 다이얼로그는 껍데기(제목·버튼·빈 상태)만
  번역한다.
- **지킴 장치**: `catalogParity.test.ts`(키 집합·보간 이름·빈 값·en 한글 잔류),
  `noHardcodedHangul.test.ts` + `hangulScan.ts`(TS AST 스캔 — 이 저장소는 한국어 주석이
  많아 grep으로는 못 한다. 래칫 98 → 0, 지금은 전면 금지), `src/test-setup.ts`(테스트
  언어를 ko 고정 — jsdom `navigator.language`가 `en-US`라 안 그러면 기존 단언이 전부 깨진다).
- **마스코트 창은 별도 webview**라 언어 전환이 실시간 전파되지 않는다(다음 창 생성 때 반영).
