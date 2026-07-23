# 데스크톱 마스코트 창 설계 (이슈 #72)

상태: 정본 — 설계 확정 + 구현 완료(2026-07-23). 실제 구현에서 갈라진 부분은
부록 B에 적었다(그쪽이 코드의 진실).
초안: 이슈 #72의 fable-planner 기획 코멘트. 본 문서가 초안의 사실오류를 바로잡고
열린 질문을 전부 확정한 구현 지시서다. 구현자는 이 문서만 따라가면 된다.

활동 중인 캐릭터 1명을 앱 창과 독립된 **투명 배경·항상 최상단 데스크톱 마스코트
창**으로 띄운다. 알림(pending)이 생기면 마스코트가 그 자리에서 알리고, 클릭하면
메인 창이 앞으로 오며 해당 캐릭터의 터미널 오버레이가 열린다.

## 목표와 범위

**포함(v1)**

- 새 Tauri 웹뷰 창 `mascot`: transparent · decorations:false · alwaysOnTop ·
  skipTaskbar · resizable:false · focus:false · shadow:false, 120×140 논리픽셀.
- 활동 중 캐릭터 **1명**의 스프라이트만 렌더(절차 생성 + 커스텀 시트 모두).
  **Pixi 미사용** — 순수 `generateSheet` + 2D 캔버스로 충분하다(§4.2).
- main(진실의 원천, zustand) → mascot 상태 푸시(Tauri 이벤트) + ready 핸드셰이크.
- pending 시 느낌표 배지 + 바운스 애니메이션, 클리어 시 정지.
- 클릭 → main 포커스 + 해당 에이전트 터미널 열기. 드래그로 위치 이동(영속).
- 설정 토글 `mascotEnabled`(기본 OFF, settings.json 영속).

**제외(후속)**

- 마스코트 다중 표시(여러 캐릭터 동시). 마스코트 창 안의 사무실 씬/말풍선/미니게임.
- 클릭스루(`set_ignore_cursor_events`) — 창이 스프라이트에 타이트해 v1 불필요.
- Windows/Linux 완전 동등 검증(§8 OS 방침 참조). OS 데스크톱 알림(기존 osNotify 소관).

## 확정 결정 (초안의 열린 질문 5개)

| # | 질문 | 결정 | 근거 |
|---|------|------|------|
| 1 | "활동 중" 정의·우선순위 | **활동 = pending 알림 보유 ∪ 턴 phase="working"** (clockedOut 제외). 우선순위: ① pending 최신(`notifications[0]`이 newest-first) ② working 중 `turnStartedAt` 최신. 단 **sticky**: 현재 표시 중인 캐릭터가 여전히 활동 중이면 working끼리는 교체하지 않고, pending 후보만 즉시 인터럽트한다 | pending 소스는 `pendingAgentIds`(selectors.ts:44, 단일 소스), working은 `timeTracking[].phase`(turnReducer.ts:9 `"idle"\|"working"\|"waiting"`). waiting(질문 대기)은 알림이 이미 pending으로 표면화되고, 알림을 지운 waiting은 "사용자가 인지한 상태"라 제외. keepAwake의 `computeAnyWorking`도 waiting을 제외하는 동일 판단(power/keepAwake.ts) |
| 2 | 다중 마스코트 | **1명만.** 페이로드/프로토콜은 단일 대상 고정(배열 아님) | 초안 가정 유지. 다중은 창 N개·좌표 관리 폭증 — 수요 확인 후 별도 이슈 |
| 3 | 대상 OS | **macOS 1차(눈검증 대상), Windows 2차(빌드·컴파일 보장, 검증은 추후), Linux best-effort.** 구현은 OS 분기 없이 Tauri 공통 API만 사용 | 코드베이스에 `cfg(windows/target_os)` 분기 153개 — Windows는 실지원 대상. 그러나 개발·검증 환경이 macOS이고, 투명 창은 macOS만 특수 요건(§4.1 macOSPrivateApi + Cargo feature)이 있다. Windows WebView2는 transparent+alwaysOnTop+skipTaskbar를 표준 지원하므로 분기 없는 구현이 그대로 동작할 가능성이 높다 |
| 4 | 활동 캐릭터 없을 때 | **숨김.** 단 활동 종료 후 `MASCOT_HIDE_LINGER_MS = 15_000` 유지 후 숨긴다(턴 사이 깜빡임 방지). 대기(졸기) 스프라이트 없음 | 이슈 제목 자체가 "활동중인 캐릭터를 표시". 유휴 시에도 최상단 투명 창이 화면을 점유하면 방해. linger는 keepAwake의 release-delay(60s)와 같은 flap 방지 관례를 축소 적용 |
| 5 | 기본 on/off·위치 | **기본 OFF**(설정 다이얼로그에서 켬). 위치는 **자유 이동(스냅 없음)** + `localStorage("agent-office.mascot.pos")` 영속. 기본 위치 = 주 모니터 우하단(우 24px·하 80px 마진). 복원 좌표가 어느 모니터에도 안 들어가면 기본 위치로 클램프 | 시스템 표면을 건드리는 기능(cliEnabled, keepAwakeEnabled)은 기본 OFF가 이 앱의 관례(settings_store.rs). 위치 같은 UI 전용 선호는 theme/terminalViewMode처럼 localStorage 영속이 관례("PersistedState 아님", appStore.ts:104,117) |

## 1. 아키텍처 개요

```
[main 창]  zustand 스토어 = 진실의 원천
  mascotBridge (신규, src/renderer/ipc/mascotBridge.ts)
    ├─ 구독: notifications · timeTracking · agents · appSettings.mascotEnabled
    ├─ pickMascotTarget(순수) → MascotState 계산
    ├─ emit("mascot-state", state)  ──────────────►  [mascot 창]
    ├─ invoke(set_mascot_visible)   ──► [Rust] show/hide     │ listen
    ├─ listen("mascot-ready") ◄────────────────────  부팅 시 emit (replay 요청)
    └─ listen("mascot-open-terminal") ◄─ [Rust] emit_to("main")
                                              ▲
[mascot 창]  얇은 소비자(스토어 없음, Pixi 없음)      │
  클릭 → invoke(mascot_activate(agentId)) ──► [Rust] main show+unminimize+set_focus
  커스텀 시트 → invoke(load_sprite(agentId))          + emit_to("main", "mascot-open-terminal")
  드래그 → window.startDragging() / onMoved → localStorage 저장
```

- **mascot은 순수 소비자**다. 스토어를 두 번째로 hydrate하지 않는다(상태 표류 원천 차단).
- 스프라이트 픽셀 데이터는 이벤트로 실어 나르지 않는다: 절차 생성은 `seed`+`archetype`으로
  mascot이 직접 재생성(결정적 — main과 동일 외형 보장), 커스텀은 mascot이
  `load_sprite` 커맨드로 직접 로드한다.
- Tauri v2에서 **앱 자체 커맨드는 capability ACL 대상이 아니다**(ACL은 core:/플러그인
  권한만 관장). 따라서 mascot 창에서 `load_sprite`/`mascot_activate` invoke가
  별도 권한 없이 된다. capability는 core 창/이벤트 권한용으로만 새로 만든다(§3).

## 2. 창 정의와 플랫폼 요건

### 2.1 tauri.conf.json

`app.windows[]`에 mascot 창을 추가하고, 기존 main 창에 명시적 label을 단다:

```jsonc
"app": {
  "macOSPrivateApi": true,          // macOS 투명 창 필수 (신규)
  "windows": [
    { "label": "main", "title": "Agent Office", "width": 800, "height": 600 },
    {
      "label": "mascot",
      "url": "mascot.html",
      "title": "Agent Office Mascot",
      "width": 120, "height": 140,
      "transparent": true,
      "decorations": false,
      "alwaysOnTop": true,
      "skipTaskbar": true,
      "resizable": false,
      "maximizable": false,
      "minimizable": false,
      "shadow": false,
      "focus": false,               // show() 시 포커스 스틸 방지
      "acceptFirstMouse": true,     // macOS: 비포커스 상태 첫 클릭도 히트
      "visibleOnAllWorkspaces": true, // macOS Spaces 어디서든 보임
      "visible": false              // 표시 여부는 main의 mascotBridge가 결정
    }
  ],
  "security": { "csp": "…기존 그대로…" }
}
```

- CSP는 변경 불필요: mascot 페이지는 Pixi/WebGL을 쓰지 않으므로 eval 이슈 자체가
  없다(§4.2). `connect-src ipc:` 등 기존 값이 두 창 모두에 적용된다.
- 창은 **항상 conf에서 생성**(visible:false)하고 show/hide로만 제어한다 — 런타임
  create/destroy보다 라이프사이클이 단순하고 capability window 매칭이 정적이다.

### 2.2 Cargo.toml (macOS 필수 — 초안 누락분)

```toml
tauri = { version = "2", features = ["macos-private-api"] }
```

`macOSPrivateApi: true`는 conf만으로는 **빌드 에러**가 난다 — Cargo feature
`macos-private-api`를 함께 켜야 한다. 이 feature는 macOS 외 타깃에서는 no-op이다.

### 2.3 라이프사이클 (고아 창 방지 — 메커니즘 확정)

`quitGuard.ts`는 확정 종료 시 `getCurrentWindow().destroy()`를 호출한다. Tauri는
**모든 창이 닫혀야** `ExitRequested`를 발화하므로, mascot이 살아 있으면 앱이
죽지 않고 유령 마스코트만 남는다. 반드시 `lib.rs`의 `.run()` 핸들러에 추가:

```rust
RunEvent::WindowEvent { label, event: tauri::WindowEvent::Destroyed, .. }
    if label == "main" =>
{
    if let Some(m) = app.get_webview_window("mascot") {
        let _ = m.destroy();
    }
}
```

기존 `RunEvent::ExitRequested` 정리 로직(dispose_all 등, lib.rs:531)은 그대로 —
mascot destroy 후 자연히 도달한다. mascot 창 자체는 closable하지 않게 두고
(decorations 없음 + 사용자 close 경로 없음), 사용자가 끄는 길은 설정 토글뿐이다.

## 3. Capability (src-tauri/capabilities/mascot.json 신규)

현 `default.json`은 `windows:["main"]`으로 스코프되어 mascot에는 아무 core 권한이
없다(검증됨). 신규 파일:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "mascot",
  "description": "Capability for the mascot window",
  "windows": ["mascot"],
  "permissions": [
    "core:default",
    "core:window:allow-start-dragging",
    "core:window:allow-set-position",
    "core:window:allow-outer-position",
    "core:window:allow-current-monitor",
    "core:window:allow-available-monitors",
    "core:window:allow-scale-factor"
  ]
}
```

- `core:default`가 이벤트 listen/emit을 포함한다. window 권한은 default 포함
  여부가 버전에 따라 애매한 것들을 **명시 나열**(중복 무해).
- 초안이 제안한 `core:window:allow-set-focus`/`allow-show/hide/close`는 **불필요** —
  포커스·표시·종료는 전부 Rust 커맨드가 수행한다(권한 표면 최소화).
- 알림(notification:*) 권한도 불필요(OS 알림은 main 소관 유지).

## 4. 렌더러 설계

### 4.1 Vite 멀티페이지 + 엔트리

`vite.config.ts`에 rollup 입력 추가(기존 옵션 구조 유지):

```ts
build: {
  rollupOptions: {
    input: {
      main: fileURLToPath(new URL("./index.html", import.meta.url)),
      mascot: fileURLToPath(new URL("./mascot.html", import.meta.url)),
    },
  },
},
```

- dev 모드는 설정 불필요: Vite dev 서버가 `/mascot.html`을 경로 그대로 서빙하므로
  `devUrl(http://localhost:1420)` + 창 url `mascot.html`이 그대로 동작한다.
- 신규 `mascot.html`(루트, index.html과 병렬): `<div id="root">` +
  `<script type="module" src="/src/renderer/mascot/main.tsx">`. body 배경 투명
  필수(`html,body{background:transparent}`) — 전역 스타일이 배경색을 칠하지 않도록
  mascot 전용 최소 CSS만 로드한다(styles/ 전역 import 금지).

신규 파일(전부 `src/renderer/mascot/`):

| 파일 | 역할 |
|---|---|
| `main.tsx` | 엔트리. ReactDOM 마운트 + `emit("mascot-ready")` |
| `MascotApp.tsx` | 상태 수신(listen "mascot-state") → 캔버스/배지 렌더, 클릭/드래그 처리 |
| `protocol.ts` | `MascotState` 타입 + 이벤트 페이로드 파서(런타임 가드). **shared/types에 두지 않는다** — renderer↔renderer 이벤트라 Rust serde 미러/contract-fixture 대상이 아니다 |
| `sheet.ts` | 시트 확보: 절차(`generateSheet`) / 커스텀(`load_sprite`+디코드) + 96px 리샘플(순수 함수로 분리, vitest 대상) |
| `position.ts` | 기본 위치 계산·모니터 클램프·localStorage 저장/복원(순수 로직 분리, vitest 대상) |
| `drag.ts` | 클릭/드래그 판정 상태기(순수, vitest 대상) |

### 4.2 스프라이트 렌더 — Pixi 미사용 (초안 수정)

초안은 `createCharacterAssets`/`generateSpritePreview` 재사용을 제안했으나:

- `createCharacterAssets`(characterFactory.ts:147)는 **Pixi Texture를 만든다** —
  두 번째 창에 Pixi 렌더러+unsafe-eval 셰이더 셋업을 끌고 들어온다.
- `generateSpritePreview`(:199)는 idle0 **정지 1프레임 dataURL**이라 애니가 없다.

마스코트는 순수 경로만 쓴다:

- **절차 생성**: `generateSheet(seed, defaultCanvasFactory, resolveArchetype(archetype, seed))`
  (characterFactory.ts:56 — "순수 시트 생성", DOM 캔버스만 사용, Pixi 무관).
  4N×N 시트 캔버스에서 idle0/idle1 셀을 `drawImage`로 슬라이스.
- **커스텀 시트**: `tauriApi.loadSprite(agentId)`(base64 PNG) → `spriteCache.ts`의
  `decodeSheet`와 동일한 디코드(코드 재사용을 위해 decode 함수를 export하거나
  mascot/sheet.ts에 동일 규약으로 구현 — `sheetCanvasDims` 순수 함수는 그대로 재사용).
- **렌더 대상**: 96×96 CSS px 박스(=CELL 16 × 프리뷰 관례 scale 6), 창 하단 중앙 정렬.
  캔버스 백킹은 `devicePixelRatio` 배율, `imageSmoothingEnabled=false` +
  CSS `image-rendering: pixelated`.
- **고해상 커스텀 시트(N > 96·dpr)**: 이슈 #47과 동일하게
  `areaDownscalePremul`(office/gen/spriteResample, 순수)로 프레임별 프리필터 후
  nearest로 찍는다. N ≤ 96·dpr이면 nearest 업스케일.
- **idle 애니**: idle0↔idle1 토글, 주기 `ANIM_IDLE_MS = 480ms`
  (CharacterEntity.ts:40과 동일 값 — 오피스와 호흡 일치). `requestAnimationFrame`
  루프 1개, 숨김(visible=false) 상태에서는 루프 정지.

### 4.3 알림(pending) 표현 — 확정 수치

ExclamationOverlay(entities/ExclamationOverlay.ts)의 감성을 DOM으로 재현:

- 배지: 지름 28px 원, 배경 `#ffcc33`, 테두리 1px `#8a5a00`, 글리프 `!` `#3a2600`
  (오버레이의 코드 색상 그대로). 위치: 스프라이트 머리 위(창 상단 여백 44px 안).
- 배지 바운스: sine, 주기 **600ms**(=BOUNCE_PERIOD_MS), 진폭 **6px**, CSS 애니메이션.
- 캐릭터 hop: pending 동안 주기 600ms, 진폭 **4px**의 translateY hop(배지와 동주기).
- `hasPending=false` 수신 즉시 둘 다 정지(위상 리셋).

### 4.4 클릭 vs 드래그 판정 — 확정 수치

`data-tauri-drag-region`은 mousedown 즉시 OS 드래그로 넘어가 클릭을 삼킨다 → 수동 판정:

1. `pointerdown`: 시작 좌표 기록.
2. `pointermove`: 누적 이동 거리 > **4px**(CSS px) → `getCurrentWindow().startDragging()`
   호출(이후 pointerup은 오지 않음 — 정상).
3. `pointerup`: 이동 4px 이하였으면 **클릭** 확정(시간 조건 없음) →
   `invoke("mascot_activate", { agentId })`.

### 4.5 위치 영속 (`position.ts`)

- 저장: `getCurrentWindow().onMoved` 구독 → 500ms 디바운스 →
  `localStorage["agent-office.mascot.pos"] = JSON.stringify({ x, y })`(논리 px).
- 복원(부팅 시): 저장값이 `availableMonitors()` 중 어느 모니터 사각형에라도
  (8px 허용오차로) 들어가면 `setPosition(new LogicalPosition(x, y))`, 아니면 기본
  위치 = 주 모니터(`primaryMonitor()`, 실패 시 currentMonitor) 우하단에서
  (우 24px, 하 80px) 마진. `workArea`를 API가 제공하면 그것을 우선 사용.
- localStorage는 두 창이 같은 오리진을 공유하지만 이 키는 mascot만 읽고 쓴다.

## 5. main 쪽 배선

### 5.1 `pickMascotTarget` (순수 함수, `src/renderer/store/selectors.ts`에 추가)

```ts
export interface MascotPick { agentId: string | null; hasPending: boolean; working: boolean }
export function pickMascotTarget(input: {
  notifications: ReadonlyArray<{ agentId: string }>; // newest-first
  timeTracking: Record<string, AgentTurnState>;
  agents: Record<string, AgentProfile>;              // clockedOut 필터용
  prevAgentId: string | null;                        // sticky 기준
}): MascotPick
```

규칙(§확정 결정 1): pending 최신 → (sticky: prev가 여전히 working이면 prev) →
working 중 turnStartedAt 최신 → null. clockedOut/미존재 에이전트는 전 단계에서 제외.

### 5.2 `mascotBridge` (신규 `src/renderer/ipc/mascotBridge.ts`)

`installMascotBridge(): () => void` — bootstrap.ts에서 `installSessionBridge()` 직후 설치.

- 구독: `useAppStore.subscribe`로 `notifications`, `timeTracking`, `agents`,
  `appSettings.mascotEnabled` 4슬라이스(sessionBridge의 offPending/offKeepAwake 패턴).
- 재계산 → `MascotState` 산출:

```ts
// src/renderer/mascot/protocol.ts (양쪽 엔트리가 import)
export interface MascotState {
  visible: boolean;            // mascotEnabled && (target != null || linger 중)
  agentId: string | null;
  name: string | null;
  seed: string | null;         // 절차 생성 소스 (profile.seed || profile.id)
  archetype: string | null;    // resolveArchetype 입력(원본 그대로 전달)
  spriteUpdatedAt: number | null; // 커스텀 존재 표시 + 캐시 무효화 키
  hasPending: boolean;
  working: boolean;
}
```

- 방출: 직전 상태와 deep-equal이면 스킵, 아니면 `emit("mascot-state", state)` +
  visible 변화 시에만 `tauriApi.setMascotVisible(visible)`.
- linger: target이 null이 된 시점에 15초 타이머 — 만료 시 visible=false 재방출.
  타이머 중 target 복귀 시 취소. 설정 OFF는 **linger 없이 즉시** 숨김(keepAwake 관례).
- 핸드셰이크: `listen("mascot-ready", …)` → 현재 상태 즉시 재emit(부팅 레이스 해소).
- 터미널 열기: `listen("mascot-open-terminal", ({agentId}) => officeBus.emitAgentClicked(agentId))`
  — emitAgentClicked(sessionBridge.ts:199)가 이미 ensureSession + openTerminal +
  clearNotifications를 전부 수행하므로 재사용(중복 구현 금지).

### 5.3 설정

- Rust `AppSettings`(settings_store.rs)에 `#[serde(default)] pub mascot_enabled: bool`
  추가(기본 false, 구파일 하위호환 자동). `Default` impl·기존 테스트의 구조체
  리터럴들·`get-app-settings-result.json` 픽스처·`contract_fixtures.rs` 갱신.
- TS `AppSettings`(shared/types/settings.ts)에 `mascotEnabled: boolean`,
  appStore의 `DEFAULT_APP_SETTINGS`와 `updateAppSettings`의 Pick 유니온에 추가.
- SettingsDialog: keepAwakeEnabled 토글 옆에 "데스크톱 마스코트" 체크박스
  (`updateAppSettings({ mascotEnabled })` — 즉시 반영, 별도 백엔드 부수효과 없음).

## 6. IPC 표면 (이름 확정)

### 6.1 커맨드 (`src/shared/ipc.ts` Commands + `ipc/commands/misc.rs` + lib.rs 등록)

| TS 키 | wire 이름 | 시그니처(Rust) | 동작 |
|---|---|---|---|
| `setMascotVisible` | `set_mascot_visible` | `(app: AppHandle, visible: bool)` | `get_webview_window("mascot")` → show()/hide(). 창 부재는 no-op Ok |
| `mascotActivate` | `mascot_activate` | `(app: AppHandle, agent_id: String)` | main 창 `show()+unminimize()+set_focus()` 후 `emit_to("main", "mascot-open-terminal", json!({"agentId": agent_id}))` |

`set_badge_count`(misc.rs:12)의 `get_webview_window("main")` 패턴을 그대로 따른다.
tauriApi.ts에는 `AgentOfficeApi`(shared/types/api.ts) 인터페이스에 두 메서드를 추가.

### 6.2 이벤트 (`src/shared/ipc.ts` Events에 추가 — 기존 kebab-case 관례)

| 상수 | wire 이름 | 방향 | 페이로드 |
|---|---|---|---|
| `mascotState` | `mascot-state` | main → mascot (emit 브로드캐스트) | `MascotState`(§5.2) |
| `mascotReady` | `mascot-ready` | mascot → main | 없음 |
| `mascotOpenTerminal` | `mascot-open-terminal` | Rust → main (`emit_to`) | `{ agentId: string }` |

초안의 `mascot:state`/`mascot:click` 콜론 표기는 기존 이벤트 관례
(`session-state`, `notification-new`)와 어긋나 kebab-case로 확정.
`MascotState`는 Rust를 거치지 않으므로 shared frozen contract/fixture **대상 아님**
(§4.1 protocol.ts). `mascot-open-terminal` 페이로드만 Rust가 `serde_json::json!`으로
만들며, 필드가 1개뿐이라 전용 struct 없이 인라인으로 충분하다.

## 7. 단계별 구현 계획

각 단계는 독립 커밋·독립 검증 가능. 공통 회귀: `npx vitest run --dir src`(현 1249+),
`cargo test --manifest-path src-tauri/Cargo.toml`(출력은 `rtk proxy` 경유), `tsc --noEmit`.

1. **백엔드 골격**: `mascot_enabled` 설정 필드(+TS 미러·픽스처·기존 테스트 갱신),
   `set_mascot_visible`/`mascot_activate` 커맨드 + invoke_handler 등록,
   `.run()`의 main-Destroyed → mascot destroy, `capabilities/mascot.json`,
   tauri.conf.json(창 2개 + macOSPrivateApi), Cargo feature.
   검증: `cargo test`(settings_store 라운드트립 + contract_fixtures), 앱 기동 회귀.
2. **멀티페이지 + 빈 마스코트 창**: vite input, mascot.html, 최소 엔트리
   (투명 배경 + 임시 사각형). 검증: vitest 회귀 + `tsc` + 눈검증(투명·최상단·
   visible:false 기동, `set_mascot_visible` 수동 호출로 표시).
3. **상태 선정 + 브리지(main)**: `pickMascotTarget` + `mascotBridge`(+bootstrap 배선).
   검증: vitest — pickMascotTarget(우선순위·sticky·clockedOut 제외),
   mascotBridge(mock emit/invoke로 dedupe·linger 15s 타이머·ready replay·OFF 즉시 숨김).
4. **마스코트 렌더**: protocol 파서, sheet.ts(절차/커스텀 분기·96px 리샘플),
   idle 애니, 배지/바운스. 검증: vitest — 시트 소스 결정·리샘플 크기 계산·파서
   가드(순수 파트); 눈검증 — main과 동일 외형(같은 seed), 커스텀 시트 일치.
5. **인터랙션**: drag.ts 판정, 클릭→`mascot_activate`→main 포커스+터미널,
   position.ts 저장/복원·클램프. 검증: vitest — drag 임계·클램프 순수 로직;
   눈검증 — 클릭/드래그 구분, 멀티모니터 복원.
6. **설정 UI + 마감**: SettingsDialog 토글, 라이프사이클 총정리(종료 동반 파괴,
   linger), `docs/subsystem-c-ui.md` §1 "단일 BrowserWindow" 전제에 마스코트 예외
   각주 추가. 검증: 전체 스위트 + §9 눈검증 일괄.

## 8. 리스크와 완화

| 리스크 | 완화 |
|---|---|
| macOS 투명 실패(Cargo feature 누락 시 빌드 에러, conf 누락 시 흰 배경) | 1·2단계에서 즉시 눈검증. 두 설정을 같은 커밋에 |
| main 종료 후 마스코트 잔존(quitGuard가 destroy 사용) | §2.3 Destroyed 핸들러 + 눈검증 항목 고정 |
| 부팅 레이스(mascot 리스너 설치 전 main이 emit) | `mascot-ready` 핸드셰이크 + mascotBridge의 상태 재방출(멱등) |
| show()가 포커스를 훔쳐 타이핑 방해 | 창 config `focus: false`. 눈검증: 작업 중 표시 전환 시 포커스 유지 |
| 활동 전환 flap(턴 사이 숨김/표시 반복) | 15s linger + sticky 규칙 |
| 고해상 커스텀 시트 지글거림(이슈 #47 재발) | `areaDownscalePremul` 재사용(동일 알고리즘) |
| Windows/Linux 미검증 동작 | OS 분기 없는 공통 API만 사용, macOS 외는 best-effort 선언. Windows 검증은 후속 이슈 |
| AppSettings 필드 추가가 기존 테스트/픽스처 다수를 깨뜨림 | 1단계에서 일괄 갱신(구조체 리터럴 사용처: settings_store.rs 테스트, lib.rs 테스트, contract_fixtures.rs, get-app-settings-result.json) |
| 전역 CSS가 mascot 배경을 칠함 | mascot.html은 전용 최소 스타일만 로드, 전역 styles import 금지 + 눈검증 |

## 9. 눈검증 목록 (사람이 직접)

1. 마스코트 창이 투명 배경·그림자 없음·최상단·독 앱 전환(⌘Tab)/작업표시줄에 안 보임.
2. 설정 OFF(기본)일 때 어떤 상황에도 안 뜸; ON 직후 활동 캐릭터가 있으면 즉시 뜸.
3. 같은 seed 캐릭터가 main 오피스와 동일 외형(절차 생성·커스텀 각 1회), idle 2프레임 애니 동작.
4. 알림 발생 → 배지+바운스 시작, 터미널 열어 클리어 → 정지·(다른 활동 없으면 15초 후 숨김).
5. 활동 캐릭터 교체(pending 인터럽트/working sticky) 동작.
6. 클릭 → main이 최전면 포커스 + 해당 캐릭터 터미널 오버레이 열림(백그라운드·최소화 상태에서도).
7. 드래그 이동 가능, 4px 미만 움직임은 클릭으로 판정. 앱 재시작 후 위치 복원.
8. 모니터 해제(노트북 단독) 후 재시작 → 기본 위치로 복귀(화면 밖 미아 없음).
9. 작업 중 표시/숨김 전환이 현재 입력 포커스를 훔치지 않음.
10. main 종료(확정 종료) 시 마스코트 동반 소멸, 프로세스 잔존 없음. macOS Spaces 전환 시에도 보임.

## 부록 A — 초안(fable-planner 코멘트) 대비 바로잡은 사실

1. **캐릭터 팩토리 경로/행번호**: `src/renderer/gen/characterFactory.ts:931,952`가 아니라
   `src/renderer/office/gen/characterFactory.ts`(총 222행). `createCharacterAssets`=147행,
   `generateSpritePreview`=199행. 그리고 둘 다 마스코트에 **부적합**(전자는 Pixi 의존,
   후자는 정지 1프레임) — 순수 `generateSheet`(56행)+2D 캔버스로 확정(§4.2).
2. **CSP/unsafe-eval**: `main.tsx:5`의 `import "pixi.js/unsafe-eval"`은 CSP에
   unsafe-eval을 넣는 게 아니라 Pixi의 eval-free 셰이더 파서 주입이다. 마스코트는
   Pixi 미사용이므로 "동일 처리 필요"라는 초안 전제 자체가 소멸.
3. **macOSPrivateApi**: conf 설정 외에 Cargo feature `macos-private-api`가 필수(초안 누락).
   현 Cargo.toml은 `tauri = { version = "2", features = [] }`.
4. **invoke_handler 위치**: lib.rs:463(초안 480). misc.rs:12(set_badge_count)는 정확.
5. **mascot capability 권한**: `allow-set-focus`/`show`/`hide`/`close`는 불필요 —
   해당 동작은 전부 Rust 커맨드로 이관(§3). 앱 자체 커맨드는 ACL 비대상이라
   mascot에서 invoke 가능.
6. **`MascotStateEvent`의 shared/types + contract-fixtures 등재**: 불필요 —
   renderer↔renderer 이벤트로 Rust serde를 거치지 않는다. frozen contract 대상은
   `AppSettings.mascotEnabled`뿐.
7. **이벤트명**: `mascot:state`/`mascot:click` → 기존 kebab-case 관례에 맞춰
   `mascot-state`/`mascot-ready`/`mascot-open-terminal`.
8. **고아 창 방지 메커니즘**: 초안의 수용 기준("main 닫으면 mascot 종료")만으로는
   quitGuard의 `destroy()` 경로에서 앱이 안 죽는 함정이 있다 — `RunEvent::WindowEvent
   Destroyed(main)` 처리로 확정(§2.3).
9. **"활동 중" 소스**: sessionBridge가 아니라 스토어 슬라이스가 소스다 —
   pending=`notifications`(selectors.pendingAgentIds), working=`timeTracking[].phase`
   (turnReducer). `activeTerminalAgentId`/`recentAgentIds`는 "활동" 판정에서 제외
   (최근 연 터미널은 사용자가 이미 보고 있는 것 — 마스코트로 알릴 대상이 아님).

## 부록 B — 구현에서 설계와 갈라진 부분

1. **`generateSheet`/`selectLayers`를 `office/gen/sheetGen.ts`로 분리**(신규 파일).
   설계는 characterFactory의 `generateSheet`를 그대로 쓰라고 했지만, 그 모듈은
   상단에서 `pixi.js`를 import한다 — 마스코트가 import하는 순간 Pixi 렌더러가
   두 번째 창 번들에 딸려온다("Pixi 미사용" 전제가 무너진다). 순수 두 함수만
   새 모듈로 옮기고 characterFactory가 재수출해 기존 호출부는 무변경.
   결과: 마스코트 청크 5.5 kB + 공유(React) 청크만 로드하고 Pixi 청크는 없다.
2. **위치는 물리 픽셀로 다룬다**(설계는 논리 px). Tauri의 `Monitor.position/size`,
   `outerPosition/outerSize`가 전부 물리 픽셀이라, 논리로 환산하면 DPI가 다른
   모니터 사이에서 스케일 팩터를 섞어야 한다. 전부 물리로 통일하고 여백만
   모니터 배율로 환산한다(`defaultPosition`). 창 크기도 상수가 아니라
   `outerSize()` 실측을 쓴다.
3. **capability에 `allow-outer-size`/`allow-inner-size`/`allow-primary-monitor` 추가** —
   위치 복원이 창 실측 크기와 주 모니터를 읽는다.
4. **커스텀 시트 디코드는 `mascot/sheet.ts`에 자체 구현**. `spriteCache.decodeSheet`는
   zustand 스토어를 끌고 오므로(마스코트는 스토어 없음 원칙) `spriteNormalize`의
   순수 `detectSheet`만 재사용해 같은 규약으로 다시 썼다.
5. **`mascot-ready` emit 시점**: 설계의 "엔트리(main.tsx)"가 아니라 `MascotApp`이
   `listen()` 프라미스가 resolve된 뒤에 쏜다 — 리스너가 실제로 걸리기 전에
   ready를 보내면 main의 재방출을 놓쳐 핸드셰이크가 무의미해진다.
6. **StrictMode 미사용**(마스코트 엔트리). 이중 마운트가 ready 핸드셰이크를
   두 번 쏘는 것 말고는 해가 없지만, 굳이 켤 이유도 없다.
7. **클릭 히트 영역 = 캔버스 엘리먼트**. 창의 나머지 여백에는 핸들러가 없어
   눌러도 아무 일이 없다(설계 §4.4의 "창=스프라이트"를 DOM 수준에서 만족).
8. **`Monitor.workArea` 기반 기본 위치 + 배율 변화 추적**(이슈 #73 1·2번, 후속 반영).
   - 기본 위치가 모니터 **전체 경계 + 하단 고정 여백 80px**이었다. macOS Dock은
     늘 하단이라 통했지만 Windows 작업표시줄은 상·하·좌·우 어디든 갈 수 있다.
     `@tauri-apps/api` 2.11.1의 `Monitor.workArea`를 쓰도록 바꿨다(`usableArea`).
     workArea가 없으면 기존 어림(여백 24 + 인셋 56 = 80)으로 폴백해 동작이 같다.
     **화면 안/밖 판정(`isOnMonitor`)은 여전히 전체 경계로 한다** — 사용자가
     마스코트를 작업표시줄 위에 놓았다면 그 자리도 유효한 위치이므로, 작업
     영역 기준으로 판정하면 복원 때 화면 밖으로 오인해 되돌려 버린다.
   - `devicePixelRatio`를 마운트 시 1회만 읽어, 배율이 다른 모니터로 옮기면
     캔버스 백킹 해상도와 커스텀 시트 프리필터(`mascotDetailCell`)가 낡은
     배율에 묶였다. `onScaleChanged`를 구독해 dpr을 상태로 들고,
     dpr을 스프라이트 재생성 키에 포함시켰다(리샘플 재실행). 캔버스 크기가
     바뀌면 내용이 지워지므로 애니 루프도 `framesVersion`/`backing`으로 다시 건다.
   - 남은 #73 항목: Windows 실기 컴파일·눈검증, 투명 여백 클릭 동작 확인.
