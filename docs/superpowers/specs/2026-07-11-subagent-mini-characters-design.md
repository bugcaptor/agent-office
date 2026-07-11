# 서브에이전트 미니 캐릭터 설계

2026-07-11 · 브랜치 `subagent-mini-characters`

> **정정 (2026-07-11, 실앱 검증 게이트):** 아래 본문은 "+1" 감지에 `PreToolUse`
> matcher `"Task"`를 쓴다고 기술하지만, 공식 훅 문서 확인 결과 `"Task"`는
> 유효한 PreToolUse 툴명 matcher가 아니며 전용 이벤트 **`SubagentStart`**(서브
> 에이전트 소환 시 발화, matcher=agent type)가 존재한다. 최종 구현은
> `PreToolUse:Task` 대신 **`SubagentStart`(matcher 빈값=전체)** 를 사용한다 —
> `SubagentStop`과 대칭 쌍이라 서브에이전트당 +1/-1 카운팅이 더 깨끗하다.
> 소스 문자열(`sub-start`/`sub-stop`)·라우팅·카운팅은 그대로.

## 문제

Claude Code 세션이 서브에이전트(Task 툴)를 소환해도 오피스 화면에는 아무
변화가 없다. 지금 모델은 `SessionId` ↔ `AgentProfile` 1:1 flat이라
서브에이전트라는 개념 자체가 없다(`state.rs`, `OfficeWorld.entities`).

서브에이전트가 도는 동안 부모 캐릭터 옆에 **축소 클론 미니 캐릭터**를 띄워
"지금 일꾼을 부렸다"는 걸 보여주는 재미 요소를 추가한다.

## 결정 사항 (사용자 확정)

- **비주얼**: 부모 스프라이트의 **축소 클론** (같은 텍스처, 작게 + 살짝 반투명).
  서브에이전트 전용 아바타는 만들지 않는다.
- **동작**: **부모 머리 옆 부근에 떠다니며(float) 멀리 가지 않음.** 걷기·배회 없음.
- **정확도**: **카운트 기반.** 활성 서브에이전트 N개 → 미니 N마리. 특정 Task와
  1:1 식별은 하지 않는다.
- **턴 종료(Stop) 시 카운트 강제 리셋: 함.** 훅 유실·Task 실패로 남는 "유령
  미니" 누수를 매 턴 자가치유. 대가로 백그라운드 Task가 턴 종료 후에도 돌면
  미니가 일찍 사라지지만, 유령이 영구히 남는 것보다 낫다.

## 채택 접근: 기존 activity-event 채널 재사용 + 자식 오버레이

서브에이전트는 부모 세션 안에서 in-process로 돌 뿐 독립 `session_id`/PTY가
없다. 그래서 미니는 앱이 추적하는 "진짜 세션"이 아니라 **부모 캐릭터의 순수
시각적 자식**이다. 감지는 훅으로, 렌더는 기존 머리 위 오버레이 패턴
(`ThinkingOverlay`/`ExclamationOverlay`)으로 처리한다.

핵심 발견: `sub-start`/`sub-stop` 신호는 **이미 존재하는 activity-event
파이프라인**(`ActivityKind` → `hub.ingest_activity` → `activity-event` →
`tauriApi.onActivity` → `sessionBridge.ts:200` 구독)을 그대로 탄다. 이 경로는
이미 (a) **dedup 우회**라 병렬 Task 버스트가 억제되지 않고, (b) **죽은/미지
세션 훅 자동 폐기**(`resolve_agent` 실패 시 drop) 성질을 가진다. 따라서 새
IPC 채널·백엔드 상태를 만들지 않는다.

기각한 대안: (B) 백엔드(`state.rs`)에 카운트 상태 보관 — 리셋 트리거
(exited/disposed/Stop)를 렌더러가 이미 전부 구독 중이라 상태·이벤트타입·배선이
이중으로 늘어남. (C) 세션 JSONL 트랜스크립트에서 `isSidechain` 읽기 — 이 앱은
트랜스크립트를 아예 안 읽으므로(순수 훅 기반) 구조와 안 맞음. (D) 미니를 별도
월드 엔티티로 — 부모 추종·y-sort·생명주기를 수동 관리해야 함.

## 구현 단위

### 1. 훅 2개 추가 (Rust: `hook_settings.rs`)

`build()`의 `entry()` 헬퍼는 matcher를 `""`로 고정한다. `PreToolUse`는
matcher가 필요하므로 matcher를 받는 변형(예: `entry_matched(source, matcher)`)을
추가하고, 훅 맵에 두 항목을 넣는다:

- `PreToolUse` + matcher `"Task"` → `source=sub-start`
- `SubagentStop` + matcher `""` → `source=sub-stop`

기존 `Notification`/`Stop`/`UserPromptSubmit`/`PostToolUse`는 그대로 둔다.
`PostToolUse`의 전체-툴 하트비트도 유지(시간 추적용).

### 2. ActivityKind 확장 + 라우팅 (Rust: `types.rs`, `hook_server.rs`)

- `types.rs` `ActivityKind`에 `SubStart`, `SubStop` 추가
  (serde rename `"sub-start"` / `"sub-stop"`).
- `hook_server.rs` `handle_hook`의 match에 팔 2개 추가:
  `"sub-start"` → `hub.ingest_activity(&q.session, ActivityKind::SubStart)`,
  `"sub-stop"` → `..SubStop`. **body 파싱 불필요**(카운트 기반).

이게 Rust 변경 전부. **백엔드에 카운트 상태를 두지 않는다.**

### 3. TS 미러 (`src/shared/types.ts`)

`ActivityKind` 유니온에 `"sub-start" | "sub-stop"` 추가. `ActivityEvent`
구조·`onActivity` 시그니처는 그대로 재사용.

### 4. 카운트 소유 + 릴레이 (`sessionBridge.ts`, `bus.ts`)

카운트는 **렌더러 `sessionBridge.ts`가 소유**한다(zustand 아님 — React UI가
안 쓰는 Pixi 전용 신호라 리렌더 불필요. 기존 `hoverCbs`식 plain `Set<Cb>`
릴레이로 충분).

- 모듈 상태: `subagentCounts = new Map<string, number>()`, `subCbs = new Set<SubCb>()`.
- `tauriApi.onActivity` 콜백(이미 존재, `sessionBridge.ts:200`)에서 kind 분기:
  - `sub-start` → count +1
  - `sub-stop` → count −1, **0에서 클램프**(리셋 후 늦게 온 sub-stop 무해화)
  - 변경 시 `subCbs.forEach(cb => cb(agentId, count))`
- **리셋 규칙**:
  - `onSessionState`에서 `state === "exited" | "disposed"` → 해당 agent 0.
  - `onNotification`에서 `source === "stop"` → 해당 agent 0. (턴 종료 자가치유)
- `bus.ts`(`OfficeBus` 인터페이스 + mock)에 `onSubagentCountChanged(cb)` 추가.

### 5. 미니 오버레이 엔티티 (신규 `entities/MiniAgentsOverlay.ts`)

`ThinkingOverlay` 패턴 복제. 별도 월드 엔티티가 아니라 **`CharacterEntity.root`의
자식** — 부모의 걷기/y-sort/파괴를 자동 상속. API: `setCount(n)` / `update(dt)`
/ `destroy()`.

- **텍스처**: 생성자에서 부모의 `CharacterAssets`를 받아 `assets.idle[0]`를
  **재사용**(복제 아님). Pixi 기본 destroy는 텍스처를 보존하므로 부모와 공유해도
  파괴 안전.
- **스프라이트**: `anchor(0.5, 1)`, `scale = 부모 spriteScale × 0.5`(겉보기 ~8px),
  `alpha = 0.75`.
- **배치**: 머리 옆 고정 슬롯 `x = [-11, +11, -15]`, `y ≈ -TILE_SIZE + 4`.
  중앙 `(0, -TILE_SIZE)`의 "!"/"..." 오버레이와 겹치지 않게.
- **부유**: `update(dt)`에서 dt 누적 sin 밥(진폭 ~1.5px, 주기 ~1200ms, 미니별
  위상차). 프레임 애니메이션 없음(밥만으로 충분, YAGNI).
- **캡: 3마리, "+N" 없음.** 분위기 표시이므로 3 초과는 그냥 3마리(Text 객체/폰트
  부담 제거). 필요해지면 나중에 한 줄로 추가.

### 6. 배선 (`CharacterEntity.ts`, `OfficeWorld.ts`)

- `CharacterEntity`: 생성자에서 `MiniAgentsOverlay`를 만들어 `root`에 add하고
  `update(dt)`에서 tick. `setSubagentCount(n)` 메서드 노출. `update` 시그니처는
  기존 dt 기반 구동에 편승.
- `OfficeWorld`: `sessionActive`와 동일 패턴으로 `subagentCounts: Map<string,
  number>` 캐시 보유(엔티티 재생성 시 마지막 카운트 재적용). `bus.onSubagentCountChanged`
  구독 1줄 추가 → 해당 `CharacterEntity.setSubagentCount(n)` 호출.

## 데이터 흐름 (요약)

```
PreToolUse:Task 훅 ─curl─▶ axum(source=sub-start) ─▶ hub.ingest_activity(SubStart)
  ─activity-event▶ tauriApi.onActivity ─▶ sessionBridge: subagentCounts +1
  ─subCbs▶ OfficeWorld ─▶ CharacterEntity.setSubagentCount ─▶ MiniAgentsOverlay.setCount
SubagentStop 훅 ─▶ ... ─▶ −1 (0 클램프)
Stop 훅 / exited / disposed ─▶ 카운트 0 리셋
```

## 엣지케이스

- **병렬 Task N개**: 훅이 각각 발화, dedup 없음 → 정확히 N.
- **Task 권한거부/실패로 서브에이전트 미생성**: `SubagentStop` 미발화 → 누수.
  Stop 리셋이 자가치유.
- **curl 유실(2s 타임아웃)**: 카운트 드리프트 → Stop 리셋이 치유.
- **세션 죽음**: 레지스트리 제거로 이후 훅 폐기 + 렌더러 exited/disposed 리셋.
- **앱 재시작**: 카운트는 순수 시각 효과 → 휘발이 정답(복원 없음, YAGNI).

## 테스트

- **Rust**: `hook_settings` build 결과에 `PreToolUse`(matcher "Task") /
  `SubagentStop` 항목과 올바른 source가 들어가는지. `hook_server`가
  `source=sub-start`/`sub-stop`를 `ActivityKind::SubStart`/`SubStop`로
  라우팅하는지.
- **sessionBridge**: sub-start/sub-stop 수신 시 카운트 증감·0클램프, Stop/
  exited/disposed에서 0 리셋, `subCbs` 통지가 오는지.
- **MiniAgentsOverlay**: `setCount`가 스프라이트 수를 0~3으로 클램프하는지,
  `update(dt)`가 밥 위치를 갱신하는지(결정적 dt).
- **실행 확인**: vitest는 `--dir src`로(루트 설정 준수), cargo test는
  `src-tauri`에서. (macOS에서 bash_wrapper 1건은 기존 실패로 무관.)

## 에러 처리

- 훅 curl 실패(`|| true`로 삼킴): 카운트 드리프트로만 나타나며 Stop 리셋이 흡수.
- 알 수 없는 source: 기존 `handle_hook`처럼 warn 후 200 응답(반쪽 데이터 방지).
- 미니는 시각 효과라 실패 시에도 세션 동작에 영향 없음 — 별도 복구 로직 없음.
