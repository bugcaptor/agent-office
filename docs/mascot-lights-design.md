# 마스코트 신호등(status lights strip) 설계

상태: 정본 — 2026-08-28 설계 확정(결정 1·2·6은 사용자가 직접 택했다). 구현 완료(리뷰 반영 포함).
**2026-08-28 개정: 칸 표시 이름 선택 설정(`mascotLightsLabel`) 추가**
(§6·§7) — 칸 아래 이름을 자동(현행)/에이전트 이름/프로젝트 이름/작업명 중
고를 수 있다. 값이 그 칸에서 비면(예: cwd 없는 에이전트에 `project`) 자동으로
폴백해 빈 칸을 만들지 않는다. 새로 생긴 `tooltip`(이름·프로젝트명·작업명을
있는 것만 이어 붙인 전체 텍스트)이 잘린 label을 호버로 보완한다. `task`를
고르면 60자 절단 텍스트가 잘 보이도록 칸 폭을 54→96px로 넓히고(가로 배열만)
최대 칸 수도 8→5로 줄인다.
**2026-08-28 개정: 칸 얼굴 스프라이트/초상 선택 설정(`mascotLightsFace`) 추가**
(§6·§7) — 얼굴 원판에 현행 스프라이트 대신 초상화를 띄우는 옵션. 기본은
스프라이트(기존 동작 그대로).
**2026-08-28 개정(사용자 요청, §6·§5.1)**: 칸이 18px 원 하나에서 **[프로필 얼굴 +
대상 이름] 타일(54×48)**로 바뀌었고, 스프라이트 영역의 죽은 공간(140−96=44px)을
없애 캐릭터가 strip 위에 올라선 모습이 되도록 창의 스프라이트 몫을 102px로 줄였다.
칸이 넓어진 만큼 상한도 12칸 → 8칸.
전제: docs/mascot-window-design.md(이슈 #72, 구현 완료)의 마스코트 창 위에 얹는다.

마스코트 창 아래에 **신호등 줄(strip)**을 붙인다. 칸(램프) 하나가 대상 하나의
상태를 색으로 보여준다: 꺼짐(OFF)=작업 없음, 초록(GREEN)=작업 중(오른쪽 진행
화살표), 노랑(YELLOW)=사용자의 답/확인 대기(노란 바탕 검은 `!`). 대상은 설정에
따라 (a) 근무 중인 에이전트들, 또는 (b) 사용자가 고른 프로젝트 저장소들이며,
프로젝트 모드에서는 **저장소(폴더)마다 정확히 1칸**으로 집계한다. 기본은 가로
배열, 설정으로 세로 배열.

마스코트가 "지금 알릴 1명"을 표면화하는 **이벤트 표면**이라면, 신호등은 여러
대상의 현재 상태를 상시 노출하는 **상태 대시보드**다. 이 목적 차이 때문에
표시 규칙(가시성·waiting 취급)이 마스코트 본체와 일부 다르며, 그 다름은 전부
의도된 것이다(§2 결정 1·6).

## 1. 목표와 범위

**포함(v1)**

- 마스코트 창(label `mascot`, 120×102 논리px, tauri.conf.json) 하단에 신호등
  strip 렌더. 가로(기본)/세로 배열 설정.
- 상태 3종: `off` / `working`(초록) / `attention`(노랑) — 색만. 집계·선정은
  main 렌더러의 순수 함수(§3)로 하고, 기존 `mascot-state` 이벤트 페이로드를
  additive 확장해 밀어 넣는다(§4).
- 에이전트 모드: **상태가 OFF가 아닌 에이전트만**(working ∪ attention), `agentOrder`
  순서. 일이 없는 에이전트는 칸 자체가 사라진다(사용자 확정, 결정 2).
- 프로젝트 모드: 설정에 저장된 폴더 목록(순서 유지), 폴더당 1칸, 소속 세션들의
  상태를 YELLOW > GREEN > OFF로 접는다. 외부 attach 세션
  (docs/external-session-attach-design.md)도 스토어에 같은 슬라이스로 흐르므로
  자동 포함.
- 칸 클릭 → 대표 에이전트의 터미널 열기(기존 `mascot_activate` 재사용).
  호버 툴팁(title) = 대상 이름.
- 신호등이 켜져 있으면 마스코트 창은 활동이 없어도 **계속 떠 있는다**(OFF도
  정보다). 스프라이트 영역은 기존 linger 규칙 그대로 접힌다.
- 창 크기 동적 조정(칸 수·방향·스프라이트 유무에 따라) + 하단중앙 앵커 유지.
- 설정 3종(`mascotLightsMode`/`mascotLightsVertical`/`mascotLightsProjects`)
  — settings.json(AppSettings) 영속, SettingsDialog에서 편집.

**제외(후속)**

- 앱에 세션이 전혀 없는(attach도 안 된) 저장소의 실제 상태 감지 — v1은 항상
  OFF(§2 결정 3). 후속 후보: `agent-office ctl` 훅만 등록하는 "watch-only
  attach" 또는 저장소 폴링.
- 마스코트 없이 신호등만 켜는 모드(§2 결정 6 — 문은 열어 둔다).
- 칸별 커스텀(색·별칭·순서 드래그), 프로젝트 자동 발견(git root 스캔),
  메인 창 내 미니 신호등, 웹 원격 노출.
- 진행률/경과시간 등 상태 3종을 넘는 표현.

## 2. 확정 결정

| # | 질문 | 결정 | 근거 |
|---|------|------|------|
| 1 | GREEN/YELLOW/OFF를 무엇에 매핑하나 | **YELLOW = pending 알림 보유(`pendingAgentIds`, selectors.ts:44) ∪ `phase==="waiting"`(turnReducer.ts:9). GREEN = `phase==="working"`. 나머지 OFF. 우선순위 YELLOW > GREEN**(waiting 중에도 백그라운드 tool이 돌 수 있으나 사용자 응답 대기가 더 급한 정보) | **사용자 확정(2026-08-28).** 마스코트 pick(selectors.ts:74 `pickMascotTarget`)은 waiting을 제외하지만 그 근거("알림을 지운 waiting은 사용자가 인지한 상태")는 **알림 표면**용이다. 대시보드에서는 "터미널을 열어 알림은 지웠지만 아직 답을 안 한" 에이전트가 노란불로 남아야 진실이다. waiting은 prompt(답함)/tool(재개)/stop에서 풀리므로(turnReducer.ts:101-121) 노란불도 정확히 그때 꺼진다 |
| 2 | 에이전트 모드에서 어떤 에이전트가 칸을 받나 | **상태가 OFF가 아닌 에이전트만** — `state !== "off"`(working ∪ attention)인 근무 중 에이전트, `agentOrder` 순서. 일이 없으면 칸이 사라지고 뒤 칸이 앞으로 밀려온다 | **사용자 확정(2026-08-28).** 요구 원문 "현재 작업중인 에이전트"를 액면대로 따른다. 대가는 턴 경계마다 칸 수·자리가 바뀌고 창이 리사이즈된다는 것 — 플래핑은 레이아웃 적용 디바운스로 흡수한다(§5.3). OFF 상태 표현은 프로젝트 모드(등록했지만 세션 없는 저장소)에서 의미를 갖는다. clockedOut 제외는 마스코트·오피스 캔버스와 같은 판단(selectors.ts:18) |
| 3 | 프로젝트 모드: 저장소당 1칸의 소속 판정과 집계, 세션 없는 repo | **소속 = 에이전트 실효 cwd(`effectiveCwd`, labelText.ts:37 — `taskLabels[id].cwd ?? profile.cwd`)가 프로젝트 폴더와 같거나 그 하위**(labelText.ts:55 `isInsideCwd` 재사용 — `~` 접두 처리 포함, export 필요). 집계 = 소속 에이전트들 상태의 max(YELLOW>GREEN>OFF). **세션이 없는(소속 에이전트가 없는) repo는 항상 OFF** | git root 해석(`rev-parse --show-toplevel`)은 백엔드에 없다(workdir/status.rs는 abbrev-ref만). 경로 포함 판정이면 저장소 내부 워크트리(.claude/worktrees/ 하위)도 자동 포함되고, 라벨 표면·gitBranchWatcher(gitBranchWatcher.ts:41)와 같은 cwd 규약을 쓴다. 외부 attach 세션은 SessionRegistry를 거쳐 notifications/timeTracking에 그대로 흐르므로 추가 작업 없이 포함. attach조차 안 된 repo의 상태를 아는 채널은 현재 없다 — v1은 OFF로 정직하게 표시(추측이 아니라 조사로 확인) |
| 4 | 설정을 어디에 저장하나 | **셋 다 Rust `AppSettings`(settings_store.rs, settings.json): `mascot_lights_mode`(enum, 기본 `off`), `mascot_lights_vertical`(bool, 기본 false), `mascot_lights_projects`(Vec<String>, 기본 빈 목록)** | mascot-window-design.md 결정 #5의 관례: SettingsDialog가 편집하는 opt-in은 settings.json(`mascotEnabled` 선례, settings.ts:127), 위치 같은 순수 UI 취향만 localStorage. 프로젝트 목록은 잃으면 아까운 사용자 데이터라 localStorage 부적합. 방향(vertical)은 UI 취향에 가깝지만 셋을 한 다이얼로그·한 저장소에 모으는 단순함이 낫다 |
| 5 | 창 크기: 고정 확보 vs 동적 리사이즈, 앵커 | **동적 리사이즈.** 목표 크기는 순수 함수(§5)로 계산, **하단중앙 앵커**(창의 bottom-center 화면좌표 보존)로 위치 보정, 리사이즈+이동은 Rust 커맨드 `set_mascot_layout` 한 번으로 원자 수행. 위치 영속은 top-left가 아니라 **앵커 좌표**로 저장 방식 변경(§5.3) | 최대 칸수만큼 투명 영역을 미리 확보하면 그 영역이 데스크톱 클릭을 삼킨다(Tauri는 픽셀 단위 히트테스트가 없다 — 현 창이 작아서만 용인된 문제). 하단중앙 앵커는 현 CSS가 스프라이트를 하단 정렬(mascot.css `.mascot-root` align-items:flex-end)하는 것과 일치하고, 스프라이트가 linger 후 접힐 때 strip이 제자리에 남는다. Rust 경유는 capability 추가(`allow-set-size` 등)를 피하고 Windows의 non-resizable 리사이즈 제약(§10)을 한 곳에서 완화한다 |
| 6 | 마스코트 OFF일 때 신호등만? 활동 없을 때 가시성은? | **사용자 확정(2026-08-28): `mascotEnabled`가 상위 게이트 — 마스코트 OFF면 신호등도 없다.** 창 가시성 규칙 확장: `visible = mascotEnabled && (스프라이트 대상 있음 ∨ linger 중 ∨ lights.length > 0)`. 즉 신호등 모드가 켜져 있고 칸이 1개 이상이면 활동이 없어도 창이 뜬다 | 신호등은 OFF 상태도 정보이므로 상시 표시가 존재 이유다. 독립 게이트는 창 라이프사이클·설정 UI를 복잡하게 하므로 v1 제외 — 단 이 결정은 설정 enum 하나로 나중에 뒤집을 수 있어(mode≠off가 창을 켜게) 되돌리기 싸다. 닫히는 문 아님 |
| 7 | 칸 클릭 동작 | **대표 에이전트를 `mascot_activate`(misc.rs:55)로 활성화**(main 포커스+터미널 열기+알림 클리어 — 기존 경로 재사용). 대표 선정: ① 그 칸에서 pending 최신 ② waiting 중 `waitingSince` 최신 ③ working 중 `turnStartedAt` 최신 ④ (프로젝트 모드) 소속 에이전트 중 agentOrder 첫째. 소속이 없으면 `clickAgentId=null` → 클릭 no-op | 에이전트 모드는 자명(칸=에이전트). 프로젝트 모드는 "그 프로젝트에서 지금 봐야 할 세션"이 클릭 의도다 — 마스코트 pick과 같은 급성도 순서를 프로젝트 범위로 좁혀 적용 |
| 8 | 넘칠 때 | **최대 8칸(MAX_LIGHTS). 초과 시 앞 7칸 + `+k` 오버플로 칩 1칸**(칩 클릭 no-op). 접기는 mascot 쪽 순수 함수 — main은 항상 전체 목록을 보낸다 | 개정 전에는 18px 원 12칸(폭 294px)이었다. 칸이 [얼굴+이름] 타일(54px)이 되면서 12칸은 폭 726px로 데스크톱 위젯 한도를 넘는다 — 8칸 가로 = 486px로 맞췄다. 렌더 관심사(몇 개까지 그릴지)는 렌더 쪽에 두면 프로토콜이 단순하다 |
| 9 | i18n | **마스코트 창 안에는 번역 문자열을 넣지 않는다.** 툴팁 = 이름(에이전트명/폴더 basename), 오버플로 칩 = `+숫자` — 전부 비번역 텍스트. 설정 다이얼로그 문자열만 locale 6종에 추가(§7) | MascotApp.tsx 상단 주석이 명시한 한계: 마스코트 창은 언어 변경이 실시간 전파되지 않는다. 상태 단어("작업중")를 툴팁에 넣는 순간 그 한계에 걸린다 — 넣지 않으면 문제 자체가 없다. 하드코딩 문자열 테스트도 자연 통과 |
| 10 | 렌더 기술 | **DOM + CSS(+인라인 SVG 화살표).** 캔버스 미사용 | 형태 3종·개수 ≤12·애니는 CSS keyframes로 충분. 배지(.mascot-badge)가 이미 같은 방식이고, 캔버스는 스프라이트 전용으로 남긴다 |

## 3. 데이터 모델과 집계 (main 쪽, 순수)

신규 `src/renderer/store/mascotLights.ts` (전부 순수 — vitest 대상):

```ts
export type MascotLightState = "off" | "working" | "attention";

export interface MascotLight {
  /** 안정 키 — agentId 또는 프로젝트 폴더 경로(설정 원문). */
  id: string;
  /** 툴팁 텍스트 — 에이전트 이름 또는 폴더 basename. 비번역(결정 9). */
  label: string;
  state: MascotLightState;
  /** 클릭 시 활성화할 대표 에이전트(결정 7). null = 클릭 no-op. */
  clickAgentId: string | null;
  /** 칸에 얼굴을 띄울 에이전트의 스프라이트 좌표(개정 §6). 대표 에이전트와
   *  같은 에이전트다 — "누르면 누가 나오나"를 그림으로 미리 보여 준다.
   *  null(세션 없는 폴더)이면 이름 첫 글자 원판으로 대체. */
  avatar: { agentId: string; seed: string; archetype: string | null;
            colors: ColorOverrides | null; spriteUpdatedAt: number | null } | null;
}

export function computeMascotLights(input: {
  mode: "off" | "agents" | "projects";
  projects: ReadonlyArray<string>;                 // 설정 순서 유지
  agentOrder: ReadonlyArray<string>;
  agents: Record<string, { name?: string; cwd?: string; clockedOut?: boolean } | undefined>;
  timeTracking: Record<string, { phase: TurnPhase; turnStartedAt: number | null; waitingSince: number | null }>;
  notifications: ReadonlyArray<{ agentId: string }>; // newest-first
  taskLabels: Record<string, { cwd?: string } | undefined>;
}): MascotLight[]   // mode==="off" 또는 결과 0칸이면 빈 배열
```

- 에이전트별 원자 상태: `attention` if `agentId ∈ pendingAgentIds(notifications)`
  ∨ `phase==="waiting"`; else `working` if `phase==="working"`; else `off`.
- 프로젝트 소속: `isInsideCwd(effectiveCwd(taskLabels[id], agents[id].cwd), project)`
  — `labelText.ts`의 `isInsideCwd`(현재 private, 55행)를 **export로 승격**해
  재사용(경로 정규화 `\`→`/`·트레일링 제거·`~` 접두 규칙까지 동일).
- **에이전트 모드 필터**: 원자 상태가 `off`인 에이전트는 결과에서 제외한다
  (결정 2). 프로젝트 모드에는 이 필터를 적용하지 않는다 — 등록된 폴더는
  세션이 없어도 OFF 칸으로 남는다.
- 프로젝트 집계: 소속 에이전트 상태의 max(attention > working > off).
- 대표(clickAgentId): 결정 7의 우선순위. clockedOut 에이전트는 소속·집계·대표
  전부에서 제외(에이전트 모드와 동일 기준).
- **알려진 제약(E2, 문서화로 충분 — 결정 3 위배 아님)**: 중첩 폴더(예: A와
  A/sub)를 둘 다 등록하면 A/sub 세션이 두 칸 모두의 소속에 잡혀 두 칸이 같이
  켜지고 대표도 같다 — 경로 포함 판정(`isInsideCwd`)의 자연스러운 귀결이라
  v1은 이대로 둔다.

## 4. 프로토콜 확장 (`src/renderer/mascot/protocol.ts`)

`MascotState`(protocol.ts:26)에 additive 필드 2개. renderer↔renderer 이벤트라
frozen contract/fixture 무관 — 파서가 가드한다.

```ts
export interface MascotState {
  // ...기존 9필드 그대로...
  /** 신호등 칸 목록. 빈 배열 = 기능 꺼짐(strip 미렌더). */
  lights: MascotLight[];
  /** true = 세로 배열. */
  lightsVertical: boolean;
}
```

- `parseMascotState`(protocol.ts:83): `lights` 부재/비배열 → `[]`(하위호환),
  각 항목은 모양 검증 실패 시 개별 드롭. `state` 값이 3종 밖이면 `"off"`로 강등.
- `sameMascotState`(protocol.ts:100): lights 배열을 항목별 필드 비교로 확장
  (dedupe가 생명 — 매 스토어 변경마다 emit 폭주 방지).
- `HIDDEN_MASCOT_STATE`: `lights: [], lightsVertical: false` 추가.

페이로드 예시(프로젝트 모드, 가로):

```json
{
  "visible": true,
  "agentId": "a1", "name": "철수", "seed": "a1", "archetype": null,
  "colors": null, "spriteUpdatedAt": null,
  "hasPending": false, "working": true,
  "lightsVertical": false,
  "lights": [
    { "id": "/Users/me/dev/agent-office", "label": "agent-office", "state": "working",   "clickAgentId": "a1" },
    { "id": "/Users/me/dev/ecis-2026",    "label": "ecis-2026",    "state": "attention", "clickAgentId": "b2" },
    { "id": "/Users/me/dev/idle-repo",    "label": "idle-repo",    "state": "off",       "clickAgentId": null }
  ]
}
```

### mascotBridge 변경 (`src/renderer/ipc/mascotBridge.ts`)

- 구독 슬라이스 추가: `taskLabels`, `appSettings`(mascotLightsMode/Vertical/
  Projects — 기존 4개 구독(166-168행)에 2개 추가).
- `buildState`(116행): `computeMascotLights` 호출 결과를 상태에 합치고,
  가시성 규칙을 결정 6으로 교체:
  `visible = mascotEnabled && (pick ≠ null ∨ linger 중 ∨ lights.length > 0)`.
- linger(133-144행)는 **스프라이트 필드에만** 적용 — lights는 라이브 상태 그대로
  둔다(신호등은 대시보드라 여운이 오히려 거짓 정보).
- 스프라이트 대상이 없고 lights만 있을 때: `agentId:null` + lights 채워서 emit
  (마스코트 쪽이 스프라이트 영역을 접는다, §5.2).

**주의 — 현 `buildState`의 조기 반환을 반드시 해체해야 한다.** 지금 코드는
`pick.agentId === null`이면 함수 전체가 `HIDDEN_MASCOT_STATE`를 반환하고, linger
만료 콜백도 `publish(HIDDEN_MASCOT_STATE)`를 직접 부른다. 이대로면 lights가
있어도 창이 꺼진다. 두 곳을 함께 고친다:

- linger 만료 콜백 → `spriteLingerDone = true; publish(buildState())`
  (스프라이트 linger 소진 여부를 별도 플래그로 들고, 가시성은 buildState가 다시 판단).
- linger 진입 가드 `last.visible && last.agentId !== null` → **`last.agentId !== null`만**.
  lights 때문에 `visible=true`이면서 `agentId=null`인 상태가 새로 생기기 때문이다.

통과 조건: **lights가 빈 배열일 때 기존 linger 동작이 완전히 불변**이어야 한다
(기존 `mascotBridge.test.ts`가 그대로 green).

## 5. 창 크기·레이아웃

### 5.1 치수 (protocol.ts 상수 추가, 전부 논리 px)

```
LIGHT_AVATAR_PX = 28 // 칸 안 프로필 원판 지름
LIGHT_TILE_W = 54    // 칸(타일) 폭
LIGHT_TILE_H = 48    // 칸(타일) 높이
LIGHT_TILE_H_TALL = 60 // 두 줄 라벨(projecttask) 칸 높이(§7 개정)
LIGHT_GAP = 6        // 칸 간격
LIGHT_STRIP_PAD = 6  // strip 내부 여백
MAX_LIGHTS = 8       // 오버플로 접기 상한(칩 포함 8칸)

MASCOT_SPRITE_PX = 96          // 스프라이트 렌더 박스
MASCOT_SPRITE_HEADROOM = 6     // 알림 hop(-4px)·배지가 잘리지 않을 머리 위 여유
MASCOT_WINDOW_H = 96 + 6 = 102 // 창의 스프라이트 몫
```

- 가로 모드: 두께 = 48 + 6×2 = **60**, stripW(n) = 12 + 54n + 6(n−1).
  n=4면 246, n=8이면 486. 창 폭 = max(120·스프라이트 표시 시, stripW).
  창 높이 = (스프라이트 102 or 0) + 60.
- 세로 모드: 두께 = 54 + 12 = **66**, stripH(n) = 12 + 48n + 6(n−1).
  창 폭 = max(스프라이트 폭, 66), 창 높이 = 스프라이트 + stripH.
  n=8·스프라이트 포함 = 102+438 = 540.
- **죽은 공간 제거(개정)**: 예전 스프라이트 몫 140은 96px 캔버스 아래로 44px의
  빈 공간을 남겨(≈0.46배) 캐릭터가 strip 위에 붕 떠 보였다. 102로 줄이고
  캔버스를 래퍼 하단 정렬(`align-items:flex-end`)해 발이 strip 윗변에 닿는다.
- **`projecttask` 라벨(개정)**: 칸을 두 줄(첫 줄 프로젝트명, 둘째 줄 작업명)로
  그리는 만큼 타일 높이가 48→60(`LIGHT_TILE_H_TALL`)으로 커진다 — wide(96px
  폭)도 함께 켜진다(작업명이 실리므로). 가로 모드 두께는 60+12=72, 세로 모드
  strip 길이의 타일 변도 60을 쓴다.
- 신규 `src/renderer/mascot/layout.ts`(순수, vitest 대상):
  `computeMascotLayout({ lightCount, vertical, hasSprite }) → { width, height }`
  + `foldOverflow(lights, MAX_LIGHTS) → { shown, overflowCount }`.

### 5.2 DOM 배치 (mascot.css 개편)

`.mascot-root`를 세로 플렉스 컬럼·하단 정렬로: `[스프라이트 래퍼(102px, 배지
포함)] → [strip]`. 배지(.mascot-badge)의 absolute 기준을 창 루트에서 **스프라이트
래퍼**로 옮긴다(strip이 아래 붙어도 배지 위치 불변). 스프라이트 대상이 없으면
래퍼를 DOM에서 제거(높이 0) — strip만 남는다.

### 5.3 리사이즈와 위치 영속 — 앵커 방식으로 변경

현 영속(position.ts)은 창 top-left(물리 px)를 저장한다. 창 크기가 상태에 따라
변하면 top-left는 "그때의 레이아웃"에 종속된 값이 되어, 재부팅·모드 변경 시
마스코트가 수십 px 미끄러진다. **저장 좌표를 하단중앙 앵커점으로 바꾼다**:

- 저장: `onMoved` 디바운스 시 `anchor = { x: pos.x + outerW/2, y: pos.y + outerH }`
  (물리 px)를 기존 키 `agent-office.mascot.pos`에 `{ ax, ay }` 형태로 저장.
  구형 `{ x, y }` 값을 읽으면 마이그레이션: 읽는 시점의 `outerSize()`로 앵커를
  환산한다 — 마운트 직후라 창은 아직 기본 크기(물리)이므로 "당시 기본 창으로
  가정"이 곧 실측값이고, **오차 없이** 같은 자리가 나온다.
- 복원·리사이즈 공통: `topLeft = { x: ax − w/2, y: ay − h }` → 모니터 클램프
  (`isOnMonitor` 재사용 + 신규 `clampToArea` — 리사이즈로 화면 밖 침범 시
  안으로 밀어 넣기) → 적용.
- 적용 경로: 신규 Rust 커맨드 **`set_mascot_layout(width, height, x, y)`**
  (물리 px, misc.rs — `set_mascot_visible` 옆). 내부에서
  `set_resizable(true)` → `set_size` → `set_position` → `set_resizable(false)`
  순으로 원자 수행(Windows에서 non-resizable 창의 프로그램 리사이즈가 막히는
  사례 회피 — §10). 창 부재 시 no-op Ok. `src/shared/ipc.ts` Commands·
  `tauriApi`·`shared/types/api.ts`에 등재, lib.rs invoke_handler에 등록.
- 트리거: MascotApp이 `lights.length | lightsVertical | hasSprite` 파생값 변화
  시에만 호출(매 상태 emit마다 호출 금지). **추가로 300ms 디바운스** — 결정 2
  때문에 에이전트 모드에서는 턴 경계마다 칸 수가 바뀌므로, 연속 변화가
  창 리사이즈 폭주로 번지지 않게 마지막 값만 적용한다(램프 색·개수의 DOM
  렌더는 디바운스하지 않는다 — 창 크기만 늦게 따라온다).
- capability(capabilities/mascot.json) 변경 없음 — 앱 자체 커맨드는 ACL 비대상.

## 6. 시각 명세

칸 하나 = **[프로필 얼굴 원판 + 대상 이름] 타일**(54×48, radius 9,
`rgba(18,19,24,.55)` 판). 예전에는 18px 원 하나뿐이라 "무엇이 켜졌는지"를
알려면 툴팁을 띄워야 했다 — 대시보드로서 반쪽이라 이름과 얼굴을 칸에 넣었다.
색은 테마 토큰을 쓰지 않는다(마스코트 창은 전역 스타일 미로드 원칙).

- **얼굴**: 28px 원판(`border: 2px solid` = 상태색). 대표 에이전트 스프라이트
  idle0의 **머리 영역만 잘라** 채운다(16×16 셀 기준 (2,1)–(13,12) 정사각형,
  `avatar.ts AVATAR_CROP`) — 전신을 28px에 넣으면 얼굴이 3~4px로 뭉개진다.
  애니메이션 없음(칸마다 raf를 돌리지 않는다). 스프라이트 확보 경로는 본체와
  같은 `loadMascotFrames`(커스텀 시트 → 실패 시 절차 생성)이고, 좌표+배율 키로
  캐시한다. `avatar === null`이면 **이름 첫 글자**(비번역) 원판.
  - **개정(2026-08-28) — 초상 선택**: 설정 `mascotLightsFace==="portrait"`이고
    대표 에이전트에 초상이 있으면(`avatar.portraitUpdatedAt != null`) 위
    스프라이트 캔버스 대신 초상 이미지(`avatar.ts loadPortraitUrl` →
    `tauriApi.loadPortrait`)를 `<img class="mascot-light-portrait">`로 띄운다.
    초상은 main 창의 초상 캐시와 별개로 마스코트 창이 직접 읽는다(창 간
    프로토콜은 좌표(agentId+portraitUpdatedAt)만 나른다 — 픽셀 비전송 규약).
    초상이 없거나(`portraitUpdatedAt === null`) 로드가 아직 안 끝났거나
    실패하면 **항상 그려 둔 스프라이트 캔버스**가 그대로 보인다 — 깜빡임 없는
    자동 폴백. 초상은 240×320 세로 이미지라 `object-fit: cover;
    object-position: center top`으로 얼굴(위쪽)만 원판에 채운다.
- **이름**: 8px/9px, `#e6e8ee`, 한 줄 말줄임(`text-overflow: ellipsis`),
  어두운 text-shadow로 어떤 바탕에서도 읽히게. 원문 그대로라 비번역(결정 9).
  - **개정(2026-08-28) — 표시 이름 선택**: 설정 `mascotLightsLabel`로 무엇을
    보여줄지 고른다(`store/mascotLights.ts`가 칸마다 계산) — `auto`(기존
    동작: agents 모드=에이전트 이름, projects 모드=폴더 basename), `agent`(항상
    에이전트 이름 — projects 모드는 대표 에이전트), `project`(프로젝트명 —
    agents 모드는 `projectAnchorCwd(세션 cwd, 프로필 cwd)`의 basename,
    projects 모드는 폴더명으로 `auto`와 동일), `task`(목표(goal) > 저장된 요청
    폴백 > 첫 프롬프트 요청 문장을 60자로 절단 — projects 모드는 대표
    에이전트의 것). **고른 값이 그 칸에서 비면(cwd/작업 정보 없음) 항상
    `auto`로 폴백**해 빈 칸을 만들지 않는다.
  - **`tooltip`(신규 필드)**: `MascotLight.tooltip`은 labelMode와 무관하게
    `[에이전트 이름, 프로젝트명, 작업명]` 중 있는 것만 `" · "`로 이어 붙인
    전체 텍스트다. 잘린 `label`을 호버로 보완한다 — `title={light.tooltip ||
    light.label}`.
- **상태 표식(폐기, 2026-08-28)**: 예전에는 얼굴 우하단에 13px 원 배지로
  working `▶`, attention `!`를 얹었다. 28px 원판에서 배지가 얼굴을 그만큼
  가려 **걷어냈다** — 상태는 테두리 색·글로우·애니메이션만으로 알린다
  (`.mascot-light-mark`와 관련 JSX는 삭제).
- **wide 칸(개정 2026-08-28)**: `mascotLightsLabel==="task"`면 60자 절단
  텍스트가 54px 타일에서 심하게 잘리므로 타일 폭을 96px로 넓힌다
  (`LIGHT_TILE_W_WIDE`, `mascot.css .mascot-lights-wide .mascot-light`).
  가로 배열에서만 최대 칸 수도 8→5로 줄인다(`maxLightsFor` — 세로 배열은
  칸이 늘어도 폭 1개분만 차지해 줄일 이유가 없다). 이 판단은 main 창이
  하고(`mascotBridge.ts`: `lightsWide = mascotLightsLabel==="task"`),
  마스코트 창은 그 결과(불리언)만 받는다 — 다른 렌더 관심사(`lightsFace`/
  `lightsVertical`)와 같은 규약.

| 상태 | 얼굴 테두리 | 글로우 | 애니메이션 |
|---|---|---|---|
| `off` | `#4a4d57` | 없음 | 없음. 타일 opacity 0.62 |
| `working` | `#35c04a` | `0 0 0 1px rgba(53,192,74,.55), 0 0 9px rgba(53,192,74,.9)` | `lights-pulse`: opacity 1↔0.7, 1200ms |
| `attention` | `#ffcc33` | `0 0 0 1px rgba(255,204,51,.6), 0 0 9px rgba(255,204,51,.95)` | `lights-blink`: translateY 0↔−2px, 600ms — 배지 바운스와 동주기 |
| 오버플로 칩 | — | — | 없음(`+k` `#cfd3dc` 12px, 타일과 같은 54×48 판) |

표식을 걷어낸 뒤 색이 유일한 신호가 됐으므로 글로우에 1px 링을 덧대 세기를
올렸다. **테두리 두께(2px)는 상태별로 바꾸지 않는다** — border-box 28px에서
두께가 바뀌면 안쪽 24px 얼굴이 상태에 따라 다르게 잘려 덜컹인다.

- strip 배경은 완전 투명(칸 사이 틈은 창이 클릭을 삼키지만 면적이 작아 수용).
- 호버: `title={label}` (OS 네이티브 툴팁 — 이름이 잘렸을 때의 전체 이름도 여기).
- 클릭 히트 = 타일 전체. **드래그는 타일 위에서도 시작할 수 있다**(개정) —
  칸이 커져 strip 여백이 6px뿐이라 예전처럼 타일발 pointerdown을 무시하면 창을
  잡을 곳이 없다. 단 타일발 pointerdown에는 **포인터 캡처를 걸지 않는다**:
  캡처하면 이어지는 click이 strip으로 리타깃돼 칸 클릭이 죽는다.

## 7. 설정

| 키(TS / Rust) | 타입 | 기본 | UI |
|---|---|---|---|
| `mascotLightsMode` / `mascot_lights_mode` | `"off"\|"agents"\|"projects"` (Rust enum, serde lowercase — `ExternalTerminal` 선례) | `"off"` | 마스코트 토글(SettingsDialog.tsx:567) 아래 라디오/셀렉트. `mascotEnabled=false`면 비활성화 |
| `mascotLightsVertical` / `mascot_lights_vertical` | bool | false | 체크박스 "세로로 표시" |
| `mascotLightsProjects` / `mascot_lights_projects` | string[] / `Vec<String>` | `[]` | 폴더 목록 편집기: 추가(`tauriApi.pickDirectory` — ProfileDialog.tsx:255 선례), 제거. 순서 = 표시 순서. projects 모드일 때만 노출 |
| `mascotLightsFace` / `mascot_lights_face` | `"sprite"\|"portrait"` (Rust enum, serde lowercase — `MascotLightsMode` 선례) | `"sprite"` | 셀렉트 "칸에 띄울 얼굴". `portrait`을 골라도 초상 없는 캐릭터는 스프라이트로 폴백(§6 개정) |
| `mascotLightsLabel` / `mascot_lights_label` | `"auto"\|"agent"\|"project"\|"task"\|"projecttask"` (Rust enum, serde lowercase) | `"auto"` | 셀렉트 "칸에 표시할 이름". 값이 그 칸에서 비면 `auto`로 폴백. `task`는 칸 폭을 96px로 넓히고 가로 배열 최대 칸 수를 5로 줄인다(§6 개정). `projecttask`는 칸을 두 줄(첫 줄=프로젝트명, 둘째 줄=작업명)로 그리며 폭(96px)과 높이(60px, `LIGHT_TILE_H_TALL`)를 함께 넓힌다 — 작업명이 없는 칸은 둘째 줄이 비지만 타일 높이는 그대로다 |

Rust `AppSettings`(settings_store.rs)에 `#[serde(default)]` 5필드 추가 —
mascot_enabled와 같은 요령. 갱신 대상: `Default` impl, settings_store
테스트의 구조체 리터럴, `contract_fixtures.rs`, `get-app-settings-result.json`
픽스처, TS `AppSettings`(settings.ts:127 아래), appStore `DEFAULT_APP_SETTINGS`
와 `updateAppSettings` Pick 유니온.

i18n — `src/shared/i18n/locales/{en,fr,ja,ko,zh-Hans,zh-Hant}/settings.json`의
`system` 섹션(6개 locale 전부):

```
system.mascotLightsTitle        "상태 신호등"
system.mascotLightsHelp         (모드 설명 — OFF/작업중/응답 대기 의미 포함)
system.mascotLightsModeOff      "끄기"
system.mascotLightsModeAgents   "에이전트별"
system.mascotLightsModeProjects "프로젝트별"
system.mascotLightsVerticalTitle "세로로 표시"
system.mascotLightsProjectsTitle "프로젝트 폴더"
system.mascotLightsProjectsAdd   "폴더 추가…"
system.mascotLightsProjectsRemove "제거"
system.mascotLightsProjectsEmpty "아직 등록된 폴더가 없습니다."
system.mascotLightsFaceTitle     "칸에 띄울 얼굴"
system.mascotLightsFaceHelp      (초상 선택 시 폴백 안내 포함)
system.mascotLightsFaceSprite    "스프라이트"
system.mascotLightsFacePortrait  "초상화"
system.mascotLightsLabelTitle    "칸에 표시할 이름"
system.mascotLightsLabelHelp     (작업명 선택 시 칸 확장 + 값 없을 때 auto 폴백 안내)
system.mascotLightsLabelAuto     "자동(모드에 맞춤)"
system.mascotLightsLabelAgent    "에이전트 이름"
system.mascotLightsLabelProject  "프로젝트 이름"
system.mascotLightsLabelTask     "작업명"
system.mascotLightsLabelProjectTask "프로젝트 + 작업"
```

마스코트 창 내부에는 번역 문자열 없음(결정 9).

## 8. 파일별 변경 지도

| 파일 | 신규/수정 | 내용 |
|---|---|---|
| `src/renderer/store/mascotLights.ts` (+`__tests__`) | 신규 | `computeMascotLights` 순수 집계(§3) |
| `src/renderer/labels/labelText.ts` | 수정 | `isInsideCwd` export 승격(로직 무변경) — `normalizeCwd`는 모듈 내부에서만 쓰여 승격이 불필요했다(실제로 하지 않음) |
| `src/renderer/mascot/protocol.ts` | 수정 | `MascotLight`·상수·`lights`/`lightsVertical` 필드·파서·dedupe(§4) |
| `src/renderer/mascot/layout.ts` (+tests) | 신규 | `computeMascotLayout`·`foldOverflow`(§5.1) |
| `src/renderer/mascot/MascotApp.tsx` | 수정 | strip 렌더·램프 클릭(`mascotActivate`)·레이아웃 변화 시 `setMascotLayout` 호출·스프라이트 영역 접기 |
| `src/renderer/mascot/mascot.css` | 수정 | 컬럼 배치·배지 기준 이동·램프/애니 스타일(§6) |
| `src/renderer/mascot/position.ts` (+tests) | 수정 | 앵커 저장/복원·구형 마이그레이션·`clampToArea`(§5.3) |
| `src/renderer/ipc/mascotBridge.ts` (+tests) | 수정 | 구독 확장·가시성 규칙·lights 합성(§4) |
| `src/shared/ipc.ts`, `src/shared/types/api.ts`, `src/renderer/ipc/tauriApi.ts` | 수정 | `setMascotLayout`/`set_mascot_layout` 등재 |
| `src-tauri/src/ipc/commands/misc.rs`, `src-tauri/src/lib.rs` | 수정 | `set_mascot_layout` 커맨드(+핸들러 등록) |
| `src-tauri/src/persistence/settings_store.rs`, `contract_fixtures.rs`, `fixtures/get-app-settings-result.json` | 수정 | 설정 3필드 + 픽스처(§7) |
| `src/shared/types/settings.ts`, `src/renderer/store/appStore.ts` | 수정 | TS 미러 + 기본값 + Pick 유니온 |
| `src/renderer/settings/SettingsDialog.tsx` | 수정 | 모드·방향·프로젝트 목록 UI(§7) |
| `src/shared/i18n/locales/*/settings.json` ×6 | 수정 | §7 키 |
| `docs/mascot-window-design.md` | 수정 | 부록 B에 "신호등이 가시성·창 크기 규칙을 확장" 각주 |

변경하지 않는 것: capabilities/mascot.json (tauri.conf.json은 초기 높이만 102로 —
첫 상태 수신 후 JS가 레이아웃 적용), 오피스 씬·알림 파이프라인 전부.

## 9. 테스트 계획 (vitest)

- `computeMascotLights`: 모드별 칸 구성(agents=OFF 아닌 에이전트만·agentOrder
  순서·전원 idle이면 빈 배열, projects=설정 순서로 OFF 칸 유지), YELLOW>GREEN>OFF 집계, waiting→YELLOW 매핑, pending만 있는 idle 에이전트
  →YELLOW, clockedOut 제외, `~` 프로필 cwd 소속 판정, 대표 선정 우선순위 4단계,
  소속 없음→clickAgentId null, mode off→빈 배열.
- `computeMascotLayout`/`foldOverflow`: 가로/세로 치수 공식, 스프라이트 유무,
  0칸, 12칸 경계, 13칸→11+칩(+k 값).
- `parseMascotState`: lights 부재 하위호환, 불량 항목 개별 드롭, 상태값 강등.
  `sameMascotState`: lights 항목 차이 감지(dedupe 회귀).
- `position.ts`: 앵커 저장/복원 왕복, 구형 `{x,y}` 마이그레이션, 리사이즈 후
  클램프.
- `mascotBridge`: lights 있을 때 스프라이트 없이도 visible, linger가 lights에
  비적용, 설정 변경(모드/방향/목록) 시 재방출, OFF 즉시 숨김 회귀.
- Rust: settings_store 라운드트립(신필드 default), contract fixture.

## 10. 위험과 함정

| 위험 | 완화 |
|---|---|
| Windows에서 `resizable:false` 창의 프로그램 리사이즈가 무시되는 사례 | `set_mascot_layout`이 set_resizable(true)→resize→restore로 감싼다. Windows 눈검증 항목 고정(#73 선례처럼 실기 확인) |
| 리사이즈↔위치영속 상호작용으로 창이 미끄러짐 | §5.3 앵커 저장으로 원천 차단. `setPosition`이 유발하는 `onMoved`도 앵커 재계산이라 멱등 |
| 상태 변화마다 리사이즈가 발생해 깜빡임/성능 저하 | **결정 2(작업중만 표시) 때문에 에이전트 모드에서는 실제로 자주 변한다.** 리사이즈 트리거를 (칸 수, 방향, 스프라이트 유무) 파생값 변화로 한정하고 300ms 디바운스(§5.3). 눈검증 항목 1에서 여러 에이전트가 동시에 턴을 오갈 때 창이 떨리지 않는지 확인 — 떨리면 축소만 지연시키는 2차 완화(빈 칸을 몇 초 유지) 검토 |
| lights 배열이 dedupe를 뚫고 emit 폭주(taskLabels는 턴 중 초 단위 갱신) | `computeMascotLights` 출력이 실질 동일하면 `sameMascotState`가 걸러낸다 — 항목별 비교 테스트 필수 |
| 프로젝트 경로 대소문자/심링크 불일치로 소속 누락 | `isInsideCwd`는 대소문자 보존 비교(labelText.ts:44 주석) — macOS 대소문자 무시 FS에서 오탐 가능. v1은 pickDirectory 절대경로 사용을 전제로 수용, 문서화 |
| 세션 없는 repo가 늘 OFF인 것을 사용자가 "고장"으로 오해 | 설정 help 문구에 명시("이 앱에 연결된 세션이 있는 폴더만 불이 들어옵니다") |
| strip 영역이 데스크톱 클릭을 삼킴 | 창을 내용에 밀착 리사이즈(결정 5). 남는 틈은 소면적 수용 — 기존 창의 동일 트레이드오프 |
| 배지 기준 이동(CSS 개편)으로 기존 배지 위치 회귀 | 눈검증 항목에 배지 재확인 포함(이슈 #74 z-index 회귀 포함) |

## 11. 미해결 질문 (구현 전 확인)

1. `set_size`+`set_position` 2호출 사이 1프레임 어긋남이 macOS에서 시각적으로
   보이는지 — 보이면 순서 조정(축소는 move→resize, 확대는 resize→move).
2. 프로젝트 목록의 항목 편집 UX 상세(순서 변경 필요 여부) — v1은 추가/제거만,
   순서는 추가 순.
3. `visibleOnAllWorkspaces` 창에서 434px(세로 12칸) 높이가 실사용상 과한지 —
   눈검증 후 MAX_LIGHTS 하향 여지.

## 12. 눈검증 목록 (사람이 직접)

1. agents 모드: 놀고 있으면 칸 없음, 턴 시작→칸이 생기며 GREEN 펄스,
   질문 발생→YELLOW+`!`(배지와 동주기), 답 입력→GREEN 복귀, 완료+알림 클리어
   →칸이 사라지고 뒤 칸이 앞으로 밀려온다. 두세 에이전트가 동시에 턴을
   오갈 때 창 리사이즈가 떨리지 않는다(§5.3 디바운스).
2. 알림을 지웠지만 답하지 않은 세션(waiting)이 YELLOW로 남는다 — 마스코트
   스프라이트는 기존대로 그 캐릭터를 강조하지 않는다(의도된 차이).
3. projects 모드: 두 에이전트가 같은 repo에서 작업 시 1칸으로 집계, 상태 max.
   목록에만 있고 세션 없는 repo는 OFF. 외부 attach 세션이 소속 repo 칸에 반영.
4. 칸 클릭→대표 에이전트 터미널. OFF·소속 없는 칸 클릭은 무반응. 호버 툴팁.
5. 가로↔세로 전환·칸 수 변화·스프라이트 등장/linger 소멸 시 **strip이 화면에서
   움직이지 않는다**(하단중앙 앵커). 드래그 후 재시작 시 같은 자리 복원.
   구버전 저장 좌표에서 업그레이드해도 대략 같은 자리.
6. 활동이 전혀 없어도 신호등 모드가 켜져 있으면 창이 떠 있고(스프라이트 없이
   strip만), 모드 off + 활동 없음이면 기존처럼 15초 linger 후 숨김.
7. 13개 이상일 때 `+k` 칩. 화면 하단 근처에서 세로 모드 전환 시 창이 화면 밖으로
   나가지 않고 클램프.
8. 배지 위치/z-index 회귀 없음(이슈 #74). 투명 배경·포커스 스틸 없음 회귀.
9. Windows: 프로그램 리사이즈 동작(§10 1번), 배율 다른 모니터에서 램프 선명도.

## 부록 — 검토하고 버린 대안

1. **별도 신호등 전용 창**: 마스코트와 독립된 라이프사이클·위치를 갖는 두 번째
   투명 창. 리사이즈 문제는 사라지지만 창 생성/파괴·위치 영속·capability·종료
   동반 파괴가 통째로 중복되고, 요구 원문이 "마스코트 아래"다. 마스코트 다중화가
   현실화되면 그때 재검토.
2. **최대 칸수만큼 창 크기 고정 예약**: 리사이즈 코드가 없어지지만 투명 여백이
   데스크톱 클릭을 상시 삼킨다(Tauri는 픽셀 히트테스트 없음). 기각.
3. **백엔드(Rust) 집계 + git 폴링으로 무세션 repo 상태 감지**: 진실 원천이
   렌더러 스토어에 있는 현 구조를 거스르고, "git 상태"와 "에이전트 작업 상태"는
   다른 것이라 폴링으로도 GREEN/YELLOW를 알 수 없다. 무세션 repo 감지는
   watch-only attach(외부 attach 설계의 확장)로 푸는 것이 맞다 — 후속.
4. **YELLOW를 pending으로만 한정(마스코트 pick과 동일)**: 알림을 지운 순간
   노란불이 꺼져 "답 대기 중" 정보가 사라진다. 대시보드 목적에 반해 기각(결정 1).
5. **에이전트 모드에서 근무 중 전원에게 칸 부여(idle은 OFF 칸)**: 칸 수가 고정돼
   자리 기억이 되고 리사이즈도 드물다. 그러나 요구 원문에서 멀고, 에이전트가
   많으면 꺼진 칸만 늘어선다. 사용자가 "작업중인 에이전트만"을 택해 기각(결정 2).
