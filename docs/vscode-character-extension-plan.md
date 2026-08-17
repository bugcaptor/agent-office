# VSCode 캐릭터 확장 이식 계획

agent-office의 **캐릭터 관련 기능만** 떼어 독립 VSCode 확장으로 만드는 계획의 정본이다.
데스크톱 앱 없이 확장 단독으로, VSCode 통합 터미널에서 도는 AI 코딩 에이전트(claude 우선)를
픽셀 캐릭터로 시각화한다.

- 상태: 계획 수립 (2026-08-18)
- 이슈: kbm #2ce (너른바다/프로젝트관리/AgentOffice)
- 근거 문서: `subsystem-b-office.md`, `external-session-attach-design.md`, `mascot-window-design.md`

## 1. 왜 가능한가 — 절단면 분석 요약

코드 조사 결과 캐릭터 서브시스템의 경계는 놀랍도록 좁다.

**입력 경계 = 이벤트 3종.** 캐릭터 표시 전체(착석/탕비실/줄서기, "!" 배지, "..." 말풍선,
미니미, 머리 위 라벨, 대표 캐릭터 선정)를 구동하는 신호는 Tauri 이벤트 3개가 전부다:

| 이벤트 | 캐릭터가 쓰는 필드 |
|---|---|
| `session-state` | `agentId`, `state`(starting/running=active), `at` |
| `notification-new` / `-cleared` | `agentId`, `source`, `message`, `at` |
| `activity-event` | `agentId`, `kind`(prompt/tool/sub-*/resume), `at`, `text`, `assistantText`, `cwd`, `count` |

**출력 경계 = 콜백 3종.** 캐릭터가 앱에 되돌려주는 것은 `agentClicked`(터미널 열기+알림 클리어),
`agentHover`, `deskClicked`/`bossDeskClicked`뿐이다. 캐릭터가 백엔드에 직접 쓰는 것은 없다.

**PTY 비의존.** 상태 감지는 훅 HTTP 채널로 들어오며 PTY와 독립이다. 외부 세션 attach 기능
(`ctl attach`)이 이미 "PTY 없이 캐릭터가 완전 동작"함을 증명했다. VSCode 터미널이 PTY를 쥐는
구조와 정확히 같은 상황이다.

**핵심 로직이 순수 모듈.** 스프라이트 절차 생성(`gen/**`), 행동 FSM, 턴 리듀서, 라벨 파생,
책상 배정, 서브에이전트 카운터가 전부 DOM/Pixi/Tauri 비의존으로 설계돼 있어 그대로 이식된다.
마스코트 창이 이미 Pixi 없이 `sheetGen.ts`로 캐릭터를 그리고 있다.

## 2. 범위

### 포함 (확장의 기능)

| 기능 | 원본 | 비고 |
|---|---|---|
| 절차적 캐릭터 생성 (seed→시트, 종족 8종) | `office/gen/**` | 그대로 이식. 시드 결정성 계약 유지 — 같은 시드면 앱과 확장에서 같은 외형 |
| 오피스 씬 (타일맵, 착석/탕비실/보스줄) | `OfficeScene`/`OfficeWorld`/`CharacterEntity` | 렌더러만 Pixi→캔버스 2D 재작성, 로직 재사용 |
| 오버레이 3종 ("!"/"..."/미니미) | `entities/*Overlay.ts` | 캔버스 2D로 재작성 (코드 드로잉이라 소량) |
| 머리 위 작업 라벨 | `labels/labelText.ts` + `TaskLabelLayer` | 파생 로직 재사용, DOM 레이어는 웹뷰 내 재작성 |
| 훅 수신 파이프라인 (관측) | `observer/*` + `notification/hub.rs` | **TS 재구현** — §4 |
| 터미널 attach + 성격 주입 | `attach_script.rs`, `wrapper_script.rs` | VSCode 터미널 방식으로 대체 — §5 |
| 캐릭터 프로필 편집 (이름/역할/성격/종족/시드) | `ProfileDialog.tsx` 축소판 | 웹뷰 폼으로 재작성 |
| 캐릭터 번들 `.aoc.json` import/export | `characterBundle.ts` | **포맷 그대로** — 앱↔확장 캐릭터 이동 통로 |
| 커스텀 스프라이트 시트 (4N×N) + S-적응 프리필터 | `spriteNormalize.ts`, `spriteResample.ts` | 2차 (M3) |
| 대표 캐릭터 상태바 표시 | `pickMascotTarget` (selectors.ts) | 마스코트 창의 대체물 — §6 |

### 제외 (확장에서 버림)

| 기능 | 이유 |
|---|---|
| PTY 소유 전부 (sessiond/broker/handoff/xterm 터미널 UI) | VSCode 터미널이 PTY를 쥔다 |
| control 서버(`ctl`)와 토큰 승인 | VSCode command API로 대체 |
| 마스코트 창 | 투명·최상단 창 개념이 VSCode에 없음. 상태바 항목으로 대체 |
| resume-watch·attention hold의 출력폭주 감지, 타건음 | PTY 출력 바이트를 못 얻음(안정 API 없음). 외부 attach 세션과 동일한 강등이며 문서상 정상 동작 |
| TTS·일기·사용량·봇 모드·포스트잇 등 비캐릭터 부가기능 | 캐릭터 범위 밖. TTS는 추후 SecretStorage 기반으로 검토 가능 |
| PixelLab 스프라이트 AI 생성, 초상 | 2차 이후 검토(키 관리 필요). 초기엔 절차 생성+업로드만 |

## 3. 아키텍처

```
┌─ VSCode Extension Host (Node) ─────────────────────────────┐
│  HookServer        http 서버, POST /hook (기존 프로토콜 동일) │
│  port 파일         <globalStorage>/observer-port             │
│  SessionRegistry   sid → characterId (유일한 관문, 미등록 폐기)│
│  NotificationHub   hub.rs의 TS 포팅(dedup 3s, Stop running>0 │
│                    억제, attention hold 5s)                  │
│  HookParsers       event.rs의 TS 포팅(훅 body→ObserverEvent) │
│  TerminalBinder    캐릭터↔터미널 배선, settings.json 작성,    │
│                    attach 스크립트 sendText                   │
│  ProfileStore      globalStorage: profiles.json + sprites/   │
│  StatusBar         대표 캐릭터 1명 + 상태(pickMascotTarget)   │
└────────────┬───────────────────────────────────────────────┘
             │ postMessage — CharacterFeed(이벤트 3종) /
             │               CharacterSink(클릭·호버)
┌────────────▼──────────────────────────────────────────────┐
│  Webview (오피스 씬)                                        │
│  캔버스 2D 렌더러(Pixi 없음, 정수 스케일)                     │
│  재사용: gen/** · behaviorFsm · pathing · deskAssignment ·  │
│          turnReducer · labelText · subagentCounts           │
└───────────────────────────────────────────────────────────┘
             ▲ 훅 POST
┌────────────┴──────────────────────────────────────────────┐
│  VSCode 통합 터미널                                          │
│  env: AGENT_OFFICE_SESSION / HOOK_URL / APP_DATA /          │
│       SETTINGS / PERSONA  + claude 셸 함수 래퍼              │
│  claude → 훅 8종이 hook-forward 스크립트 실행 → HookServer   │
└───────────────────────────────────────────────────────────┘
```

절단선은 조사에서 확인한 `CharacterFeed`/`CharacterSink` 인터페이스로 승격한다. 웹뷰 쪽
캐릭터 코드는 이 인터페이스만 알고, extension host가 Tauri 대신 그것을 공급한다.

## 4. 훅 수신 파이프라인 재구현

기존 프로토콜을 **이름까지 그대로** 재현한다(계약 재사용, 앱과의 정신적 호환 유지).

- **서버**: Node `http`로 `127.0.0.1` 임의 포트, `POST /hook?session=<sid>&provider=claude&event=<Event>`,
  body는 벤더 훅 payload 원문. 항상 `200 {"ok":true}` fail-open.
- **포트 파일**: `<globalStorage>/observer-port` 평문 정수.
- **forwarder 대체**: 앱 바이너리 서브커맨드 대신 셸 스크립트 `<globalStorage>/hook-forward.sh` 를
  확장이 생성한다. env `AGENT_OFFICE_HOOK_URL`로 POST하되 실패 시
  `AGENT_OFFICE_APP_DATA/observer-port`를 다시 읽어 1회 재시도(**포트 스테일 대응, 이슈 #30 함정**).
  전송은 `curl`(macOS/Linux/Win10+ 기본 탑재), stdin을 그대로 body로. Windows는 PowerShell 변형(M4).
- **settings.json**: `<globalStorage>/observer/claude/<sid>.settings.json` — 내용 세션 무관,
  8개 이벤트 각각 `sh '<...>/hook-forward.sh' claude <Event>`. temp+rename 원자 쓰기, 30일 GC.
- **파서**: `event.rs`의 필드 규칙을 TS로 포팅 — `prompt`(2000자 절단, `!`/`/`/`#` 폐기), `cwd`,
  `transcript_path` 꼬리 64KB에서 assistant 텍스트, `tool_name/tool_input` 요약,
  `agent_id`/`background_tasks` 서브에이전트 규칙, `message`.
- **hub**: `hub.rs` 상태 머신을 TS로 포팅 — dedup `sha1(sid|source|msg)` 3초 창,
  `Stop{running>0}` 알림 억제, attention hold 5초(Prompt/Tool/SubStart/Stop 도착 시 폐기,
  출력폭주 갈래는 생략). resume-watch는 생략(§2 제외 참조).

**옮길 때 반드시 지킬 함정들** (원본 사고 이력):
1. 훅 커맨드에 포트를 박지 않는다 — env + 포트 파일 재시도.
2. `SubagentStop`의 `background_tasks` 카운트는 top-level `agent_id`가 있을 때만 신뢰,
   자기 자신 제외. 없으면 SubStop 델타로 강등 (off-by-one → 미니미 잔존 버그).
3. 훅 HTTP는 순서 보장이 없다 — `SubagentCountTracker`의 `lastAt` 워터마크 유지.
4. `agent_id` 있는 `Stop`/`UserPromptSubmit`은 서브에이전트 것이므로 폐기(부모 조기 회색화 방지).
5. 성격 텍스트는 임의 입력 — `sh_quote` 규칙(작은따옴표 인용) 그대로 이식, 셸 인젝션 차단.

## 5. 터미널 연동 (캐릭터 붙이기)

두 경로를 제공한다. 둘 다 결과적으로 `attach_script.rs`와 같은 env+래퍼를 심는다.

1. **캐릭터 터미널 열기** (주 경로): 명령/뷰에서 캐릭터 선택 →
   `window.createTerminal({ name: 캐릭터명, env: {...} })`. env에
   `AGENT_OFFICE_SESSION`(새 UUID) / `HOOK_URL` / `APP_DATA` / `SETTINGS` / `PERSONA`를 싣고,
   셸 기동 후 래퍼 함수 정의를 위해 attach 스크립트를 `terminal.sendText(eval 한 줄)`로 주입.
   (셸 rc 개조 없이 attach 방식을 재사용 — zsh/bash 공용 `render_posix` 이식.)
2. **기존 터미널에 붙이기**: 열려 있는 터미널을 골라 같은 스크립트를 sendText.
   `ctl attach`의 VSCode 판이다.

- 래퍼는 `claude` 함수: `--settings "$AGENT_OFFICE_SETTINGS" --append-system-prompt "$AGENT_OFFICE_PERSONA"`
  전치, 사용자가 `--settings`를 직접 주면 비켜남(`skip_if_present`), settings 파일이 사라졌으면
  관측 없이 원본 실행.
- **세션 수명**: `window.onDidCloseTerminal`로 registry 제거 + 캐릭터 퇴근 처리.
  (PID kill(0) 스윕 불필요 — VSCode가 터미널 수명을 알려준다. 원본보다 단순해지는 지점.)
- **활성 판정**: 터미널 존재+바인딩 = `running`. `session-state`를 TerminalBinder가 합성한다.
- codex/pi 지원은 원본 어댑터 구조를 따라 후순위(M4+)로.

## 6. UI 배치

- **오피스 씬 웹뷰**: 뷰 컨테이너(활동바 아이콘) 안 `WebviewView`. 씬 원본 크기 320×224(20×14타일)라
  사이드바 폭에서도 정수 스케일 1~2로 들어간다. 패널(하단)로 옮겨 띄우는 것도 가능하게.
  `retainContextWhenHidden` 없이 재마운트 대응 — officeBus의 "구독 즉시 현재값 replay" 계약을 유지하면 된다.
- **상태바 항목**: `pickMascotTarget` 재사용 — 알림 pending 최신 → sticky working → 최신 working.
  `$(bell) 김코딩 !` / `⋯ 작업중` 식 텍스트, 클릭 시 해당 터미널 포커스(=`agentClicked`).
  15초 linger 규칙도 그대로.
- **알림**: "!" 발생 시 뷰 배지(`WebviewView.badge`) + 상태바 강조. OS 알림은 설정 옵션.
- **호버 카드/라벨**: 웹뷰 내 DOM 오버레이로 원본 방식 재사용.

## 7. 코드 공유 전략 — npm workspaces 모노레포

이식이 아니라 **공유**한다. 시드 결정성(같은 시드=같은 외형)은 회귀 계약이라 사본 두 벌을
유지하면 반드시 어긋난다.

```
agent-office/
  packages/character-core/     # 신설 — 순수 모듈 이동 (Pixi/DOM/Tauri 비의존)
    gen/** (sheetGen·archetypes·parts·palette·prng·compositor·spriteResample)
    behaviorFsm · pathing · mapData · deskAssignment
    turnReducer · subagentCounts · labelText
    selectors(pendingAgentIds·pickMascotTarget 분리분)
    shared/types/** · characterBundle
    (+ 기존 vitest 결정성 테스트 동반 이동)
  packages/vscode-ext/         # 신설 — 확장 본체
  src/ · src-tauri/            # 기존 앱, core를 workspace 의존으로 import
```

- 앱 쪽 변경은 import 경로 치환뿐(동작 불변). vite/vitest alias로 단계적 전환 가능.
- Rust 쪽 hub/파서는 공유 불가 → TS 포팅본을 core에 두고, **동일 픽스처(훅 payload 샘플)로
  양쪽을 검증하는 계약 테스트**를 추가해 어긋남을 잡는다.

## 8. 마일스톤

| 단계 | 내용 | 완료 기준 |
|---|---|---|
| **M0** 코어 추출 | `packages/character-core` 신설, 순수 모듈 이동, 앱 import 전환 | 앱 동작 불변, vitest/cargo 전량 통과 |
| **M1** 파이프라인 | 확장 골격 + HookServer/포트파일/hook-forward.sh/settings.json/registry/hub-TS + 캐릭터 터미널 열기 + 상태바 표시 | VSCode 터미널의 claude 실행 → 상태바가 working/waiting/idle을 실시간 반영, "!"·클리어 동작 |
| **M2** 오피스 씬 | 캔버스 2D 렌더러 + 씬 웹뷰(캐릭터·오버레이 3종·라벨·클릭→터미널 포커스) | 다중 캐릭터가 착석/탕비실/줄서기/미니미까지 앱과 동일 거동 |
| **M3** 캐릭터 관리 | 프로필 편집 폼(이름/역할/성격/종족/시드), `.aoc.json` import/export, 커스텀 시트 업로드+프리필터 | 앱에서 export한 캐릭터가 확장에서 동일 외형·성격으로 동작 |
| **M4** 다듬기 | 기존 터미널 attach 명령, Windows(PowerShell forwarder·래퍼), 멀티 윈도우 정리, 마켓플레이스 패키징 | vsix 배포 가능 |

M1이 끝나면 렌더링 없이도 가치가 있고(상태바 프레즌스), M2부터 본래의 캐릭터 경험이 나온다.

## 9. 멀티 인스턴스/충돌 처리

- **데스크톱 앱과 동시 사용**: 충돌 없음. 훅 라우팅이 터미널별 env(`AGENT_OFFICE_*`)로 격리되고
  포트 파일 디렉터리가 다르다. 같은 캐릭터를 양쪽에 두고 싶으면 `.aoc.json`으로 복제.
- **VSCode 창 여러 개**: 창마다 확장 호스트가 각자 서버를 띄운다. 훅 스크립트가 참조하는
  `AGENT_OFFICE_APP_DATA`를 attach 시점에 그 창의 storage 경로로 박으므로 포트 재발견도 창별로 격리.
  프로필은 globalStorage 공유, 씬/바인딩은 창별(workspaceState).

## 10. 미결정 사항 (구현 착수 전 결정 필요)

1. **확장 이름/배포 형태** — 마켓플레이스 공개 여부, 저장소 분리 여부(계획은 모노레포 유지).
2. **씬 기본 위치** — 사이드바 vs 하단 패널 (기술적으론 양쪽 지원, 기본값만 결정).
3. **codex/pi 지원 시점** — M4 이후로 미뤄둠.
4. **초상/PixelLab/TTS** — 키 관리(SecretStorage) 포함 별도 설계 필요, 이번 범위 밖.
