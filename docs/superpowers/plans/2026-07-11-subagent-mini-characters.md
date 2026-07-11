# 서브에이전트 미니 캐릭터 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Claude Code 서브에이전트(Task 툴)가 도는 동안 부모 캐릭터 머리 옆에 축소 클론 미니 캐릭터를 카운트 기반으로 띄운다.

**Architecture:** 서브에이전트 생명주기를 Claude Code 훅 2개(`PreToolUse` matcher `"Task"`, `SubagentStop`)로 감지 → 기존 activity-event 파이프라인(`ActivityKind` → `hub.ingest_activity` → `activity-event` → `tauriApi.onActivity`)에 실어 렌더러로 전달 → 렌더러가 부모별 카운트를 소유하고, 부모 `CharacterEntity`의 자식 오버레이(`ThinkingOverlay` 패턴)로 미니를 표시. 백엔드에는 카운트 상태를 두지 않는다.

**Tech Stack:** Rust(Tauri, axum, serde) 백엔드 / TypeScript + React + Pixi.js v8 렌더러 / vitest + cargo test.

> **정정 (2026-07-11, 실앱 검증 게이트, 커밋 46a4b9d):** Task 4는 "+1" 감지에
> `PreToolUse`(matcher `"Task"`)를 설치하지만, 공식 훅 문서 확인 결과 `"Task"`는
> 유효한 PreToolUse 툴명이 아니다. 최종 구현은 전용 이벤트 **`SubagentStart`**
> (matcher 빈값=전체 agent type)를 사용한다. hook_settings의 `entry_matched`
> 헬퍼는 제거됨(모든 훅이 빈 matcher). 소스 문자열·라우팅·카운팅은 불변.

## Global Constants (모든 태스크 공통, 스펙에서 그대로 인용)

- 미니 스케일 = 부모 `spriteScale` × **0.5**, alpha = **0.75**.
- 미니 최대 표시 = **3마리**, "+N" 표기 없음.
- 머리 옆 슬롯 x = **[-11, +11, -15]**, 오버레이 루트 y = **-TILE_SIZE**(부모 머리 위, 기존 오버레이와 동일 높이). 부유는 오버레이 내부에서 y=0 근처로.
- 부유(bob): 진폭 **1.5px**, 주기 **1200ms**, 미니 i마다 위상차.
- 활성 서브에이전트 N개 → 미니 min(N, 3)마리. **특정 Task와 1:1 식별 안 함.**
- 카운트 리셋: 세션 `exited`/`disposed`, 알림 `source==="stop"` → 해당 agent 0. 카운트는 **0에서 클램프**(음수 금지).
- 훅 source 문자열: `"sub-start"`(PreToolUse:Task), `"sub-stop"`(SubagentStop). Rust `ActivityKind`는 serde rename `"sub-start"`/`"sub-stop"`(kebab, lowercase 규칙 예외).
- **테스트 실행**: 렌더러 vitest는 `npx vitest run --dir src <path>` (루트 vitest 설정 준수), Rust는 `cd src-tauri && cargo test <name>`. macOS에서 `bash_wrapper` 1건은 **기존부터 실패**이며 이 작업과 무관 — 새로 깨진 것만 실패로 본다.
- 커밋 메시지 말미: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

## File Structure

- `src-tauri/src/types.rs` (수정) — `ActivityKind`에 `SubStart`/`SubStop` 추가.
- `src-tauri/src/session_events/recording_events.rs` (수정) — exhaustive 매치 컴파일 픽스(서브 신호는 시계열 기록 제외, 릴레이만).
- `src-tauri/src/notification/hook_server.rs` (수정) — `sub-start`/`sub-stop` 라우팅.
- `src-tauri/src/notification/hook_settings.rs` (수정) — `PreToolUse`(matcher "Task")·`SubagentStop` 훅.
- `src/shared/types.ts` (수정) — `ActivityKind` 유니온 확장.
- `src/renderer/store/appStore.ts` (수정) — `applyActivityEvent` 타입 가드(서브 종류 무시).
- `src/renderer/ipc/subagentCounts.ts` (신규) — `SubagentCountTracker` 순수 모듈.
- `src/renderer/office/bus.ts` (수정) — `OfficeBus.onSubagentCountChanged` + mock.
- `src/renderer/ipc/sessionBridge.ts` (수정) — 트래커 인스턴스화·offActivity 분기·리셋·`officeBus` 구현.
- `src/renderer/office/entities/MiniAgentsOverlay.ts` (신규) — 축소 클론 미니 오버레이.
- `src/renderer/office/entities/CharacterEntity.ts` (수정) — 미니 오버레이 자식 배선 + `setSubagentCount`.
- `src/renderer/office/world/OfficeWorld.ts` (수정) — 카운트 캐시 + 버스 구독 + 생성 시 적용.

---

## Task 1: Rust `ActivityKind`에 SubStart/SubStop 추가

**Files:**
- Modify: `src-tauri/src/types.rs:85-90` (`ActivityKind` enum)
- Test: `src-tauri/src/types.rs` (기존 `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `ActivityKind::SubStart` (serde `"sub-start"`), `ActivityKind::SubStop` (serde `"sub-stop"`). 다른 모든 태스크가 이 변형과 wire 문자열에 의존.

- [ ] **Step 1: Write the failing test**

`src-tauri/src/types.rs`의 `mod tests` 안(기존 `activity_kind_serializes_lowercase` 근처)에 추가:

```rust
    #[test]
    fn activity_kind_serializes_subagent_variants_as_kebab() {
        assert_eq!(serde_json::to_string(&ActivityKind::SubStart).unwrap(), "\"sub-start\"");
        assert_eq!(serde_json::to_string(&ActivityKind::SubStop).unwrap(), "\"sub-stop\"");
    }

    #[test]
    fn activity_kind_deserializes_subagent_variants_from_ts_literal() {
        let a: ActivityKind = serde_json::from_str("\"sub-start\"").unwrap();
        let b: ActivityKind = serde_json::from_str("\"sub-stop\"").unwrap();
        assert_eq!(a, ActivityKind::SubStart);
        assert_eq!(b, ActivityKind::SubStop);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --lib types::tests::activity_kind_serializes_subagent_variants_as_kebab`
Expected: 컴파일 실패 — `no variant named SubStart`.

- [ ] **Step 3: Write minimal implementation**

`src-tauri/src/types.rs`의 `ActivityKind`를 교체(변형별 rename으로 kebab 지정 — enum 레벨 `rename_all = "lowercase"`는 그대로 두되 새 변형은 명시 rename):

```rust
/// activity 신호 종류. TS ActivityKind와 동일.
/// prompt = UserPromptSubmit(턴 시작), tool = PostToolUse(하트비트).
/// sub-start = PreToolUse:Task(서브에이전트 소환), sub-stop = SubagentStop(종료).
/// 뒤 둘은 카운트 기반 미니 캐릭터 전용 — 시간 추적/시계열엔 기록하지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityKind {
    Prompt,
    Tool,
    #[serde(rename = "sub-start")]
    SubStart,
    #[serde(rename = "sub-stop")]
    SubStop,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test --lib types::tests::activity_kind`
Expected: 새 테스트 2건 PASS. (이 시점엔 `recording_events.rs`가 아직 exhaustive 매치라 **crate 전체 빌드는 깨질 수 있음** — Task 2에서 고친다. 이 단계는 `types` 모듈 테스트만 통과 확인.)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/types.rs
git commit -m "feat(rust): ActivityKind에 SubStart/SubStop 추가

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: `recording_events.rs` exhaustive 매치 컴파일 픽스

**Files:**
- Modify: `src-tauri/src/session_events/recording_events.rs:78-90` (`activity_event`)
- Test: `src-tauri/src/session_events/recording_events.rs` (기존 `mod tests`)

**Interfaces:**
- Consumes: `ActivityKind::{SubStart, SubStop}` (Task 1).
- Produces: 서브 신호는 시계열 세션이벤트로 **기록되지 않고**, `inner.activity_event`로만 릴레이된다는 계약.

- [ ] **Step 1: Write the failing test**

`recording_events.rs`의 `mod tests` 안에 추가(기존 테스트가 `RecordingEvents`/`SessionEventStore`를 쓰는 패턴을 따름):

```rust
    #[test]
    fn subagent_activity_is_relayed_but_not_recorded_as_session_event() {
        let dir = std::env::temp_dir().join(format!("ao-rec-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let store = Arc::new(SessionEventStore::new(dir.join("events.jsonl")));
        let inner = Arc::new(RecordingEvents::default());
        let rec = RecordingSessionEvents::new(store.clone(), inner.clone());

        rec.activity_event(&ActivityEvent {
            agent_id: "a1".into(),
            session_id: "s1".into(),
            kind: ActivityKind::SubStart,
            at: 1,
            text: None,
        });

        // inner(렌더러 릴레이)로는 전달된다.
        assert_eq!(inner.activities().len(), 1);
        assert_eq!(inner.activities()[0].kind, ActivityKind::SubStart);
        // 시계열 스토어에는 기록되지 않는다(Prompt/Tool만 기록).
        assert!(store.read_all().unwrap().is_empty(), "서브 신호는 시계열 기록 대상이 아니다");
    }
```

> 참고: `RecordingSessionEvents`의 정확한 생성자/필드명(`new`, `record`, `inner`)과 `SessionEventStore::new`/`read_all` 시그니처는 파일 상단·기존 테스트에서 확인 후 그대로 맞춘다. 스토어 read API 이름이 다르면(예: `load`/`entries`) 기존 테스트가 쓰는 이름을 사용.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --lib session_events::recording_events`
Expected: 컴파일 실패 — `activity_event`의 `match event.kind`가 `SubStart`/`SubStop`를 다루지 않아 non-exhaustive.

- [ ] **Step 3: Write minimal implementation**

`recording_events.rs:78-90`의 `activity_event`를 교체:

```rust
    fn activity_event(&self, event: &ActivityEvent) {
        // 서브에이전트 카운트 신호(SubStart/SubStop)는 시각 효과 전용 —
        // 턴 시계열엔 기록하지 않고 렌더러 릴레이만 한다.
        let kind = match event.kind {
            ActivityKind::Prompt => Some(SessionEventKind::Prompt),
            ActivityKind::Tool => Some(SessionEventKind::Tool),
            ActivityKind::SubStart | ActivityKind::SubStop => None,
        };
        if let Some(kind) = kind {
            self.record(SessionEventDraft::simple(
                event.agent_id.clone(),
                event.session_id.clone(),
                kind,
                event.at,
            ));
        }
        self.inner.activity_event(event);
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test --lib session_events::recording_events`
Expected: 새 테스트 PASS + 기존 테스트 유지. 이제 crate 전체가 빌드된다: `cd src-tauri && cargo build` → 성공.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/session_events/recording_events.rs
git commit -m "fix(rust): 서브에이전트 activity는 시계열 기록 제외하고 릴레이만

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: 훅 서버 `sub-start`/`sub-stop` 라우팅

**Files:**
- Modify: `src-tauri/src/notification/hook_server.rs:48-54` (`handle_hook`의 match)
- Test: `src-tauri/src/notification/hook_server.rs` (기존 `mod tests`)

**Interfaces:**
- Consumes: `ActivityKind::{SubStart, SubStop}` (Task 1), `hub.ingest_activity(&session, kind)`.
- Produces: HTTP `POST /hook?session=<sid>&source=sub-start|sub-stop` → 해당 세션 agent로 `ActivityEvent{kind}` 방출.

- [ ] **Step 1: Write the failing test**

`hook_server.rs`의 `mod tests`에 추가(기존 `hook_post_with_tool_source_emits_activity` 패턴):

```rust
    #[tokio::test]
    async fn hook_post_with_sub_start_source_emits_substart_activity() {
        let (hub, events) = fixture();
        let (tx, rx) = oneshot::channel();
        let (port, handle) = serve(hub.clone(), rx).await.unwrap();

        reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/hook?session=s1&source=sub-start"))
            .body("")
            .send()
            .await
            .unwrap();

        wait_for(|| !events.activities().is_empty()).await;
        let acts = events.activities();
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].kind, crate::types::ActivityKind::SubStart);
        assert!(events.notifications().is_empty());

        let _ = tx.send(());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn hook_post_with_sub_stop_source_emits_substop_activity() {
        let (hub, events) = fixture();
        let (tx, rx) = oneshot::channel();
        let (port, handle) = serve(hub.clone(), rx).await.unwrap();

        reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/hook?session=s1&source=sub-stop"))
            .body("")
            .send()
            .await
            .unwrap();

        wait_for(|| !events.activities().is_empty()).await;
        assert_eq!(events.activities()[0].kind, crate::types::ActivityKind::SubStop);

        let _ = tx.send(());
        handle.await.unwrap();
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --lib notification::hook_server::tests::hook_post_with_sub_start_source_emits_substart_activity`
Expected: FAIL — `source=sub-start`가 unknown source로 무시되어 `events.activities()`가 비어 timeout 패닉.

- [ ] **Step 3: Write minimal implementation**

`hook_server.rs:48-54`의 match에 팔 2개 추가(`"tool"` 아래):

```rust
    match q.source.as_str() {
        "stop" => hub.ingest_hook(&q.session, NotificationSource::Stop, &body),
        "prompt" => hub.ingest_activity_with_body(&q.session, ActivityKind::Prompt, &body),
        "tool" => hub.ingest_activity(&q.session, ActivityKind::Tool),
        "sub-start" => hub.ingest_activity(&q.session, ActivityKind::SubStart),
        "sub-stop" => hub.ingest_activity(&q.session, ActivityKind::SubStop),
        "" | "hook" => hub.ingest_hook(&q.session, NotificationSource::Hook, &body),
        other => eprintln!("hook_server: ignoring unknown hook source '{other}' (session={})", q.session),
    }
```

> `ActivityKind`가 이 파일 상단에서 이미 import되어 있는지 확인(`use crate::types::{ActivityKind, NotificationSource};`). 없으면 import에 `ActivityKind` 추가.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test --lib notification::hook_server`
Expected: 새 테스트 2건 PASS + 기존 라우팅 테스트 유지.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/notification/hook_server.rs
git commit -m "feat(rust): 훅서버 sub-start/sub-stop → SubStart/SubStop activity 라우팅

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: 훅 설정에 `PreToolUse`(Task)·`SubagentStop` 추가

**Files:**
- Modify: `src-tauri/src/notification/hook_settings.rs:77-90` (`build`)
- Test: `src-tauri/src/notification/hook_settings.rs` (기존 `mod tests`)

**Interfaces:**
- Produces: 세션 settings JSON에 `hooks.PreToolUse`(matcher `"Task"`, source `sub-start`)와 `hooks.SubagentStop`(source `sub-stop`)가 포함.

- [ ] **Step 1: Write the failing test**

`hook_settings.rs`의 `mod tests`에 추가:

```rust
    #[test]
    fn build_includes_pretooluse_task_and_subagentstop_hooks() {
        let writer = HookSettingsWriter::new(scratch_dir());
        let value = writer.build("sess-1", 52413);
        let hooks = value.get("hooks").expect("top-level `hooks` key");

        // PreToolUse: matcher "Task", command에 source=sub-start
        let pre = &hooks["PreToolUse"][0];
        assert_eq!(pre["matcher"], "Task");
        let pre_cmd = pre["hooks"][0]["command"].as_str().unwrap();
        assert!(pre_cmd.contains("source=sub-start"), "{pre_cmd}");

        // SubagentStop: source=sub-stop
        let stop = &hooks["SubagentStop"][0];
        let stop_cmd = stop["hooks"][0]["command"].as_str().unwrap();
        assert!(stop_cmd.contains("source=sub-stop"), "{stop_cmd}");

        // 기존 훅 유지 회귀 방지.
        assert!(hooks.get("Notification").is_some());
        assert!(hooks.get("PostToolUse").is_some());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --lib notification::hook_settings::tests::build_includes_pretooluse_task_and_subagentstop_hooks`
Expected: FAIL — `hooks["PreToolUse"]`가 없어 index 패닉/null.

- [ ] **Step 3: Write minimal implementation**

`hook_settings.rs`의 `build`를 교체(matcher를 받는 `entry_matched` 추가):

```rust
    pub fn build(&self, session_id: &str, port: u16) -> serde_json::Value {
        let entry = |source: &str| {
            json!([{
                "matcher": "",
                "hooks": [{ "type": "command", "command": Self::curl(port, session_id, source) }]
            }])
        };
        let entry_matched = |source: &str, matcher: &str| {
            json!([{
                "matcher": matcher,
                "hooks": [{ "type": "command", "command": Self::curl(port, session_id, source) }]
            }])
        };
        json!({ "hooks": {
            "Notification": entry("hook"),
            "Stop": entry("stop"),
            "UserPromptSubmit": entry("prompt"),
            "PostToolUse": entry("tool"),
            // 서브에이전트 미니 캐릭터: Task 툴 소환 = sub-start, 종료 = sub-stop.
            "PreToolUse": entry_matched("sub-start", "Task"),
            "SubagentStop": entry("sub-stop"),
        } })
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test --lib notification::hook_settings`
Expected: 새 테스트 PASS + 기존 테스트 유지.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/notification/hook_settings.rs
git commit -m "feat(rust): 세션 훅에 PreToolUse(Task)/SubagentStop 추가

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: TS `ActivityKind` 확장 + `appStore` 타입 가드

**Files:**
- Modify: `src/shared/types.ts:109` (`ActivityKind`)
- Modify: `src/renderer/store/appStore.ts:413-420` (`applyActivityEvent`)
- Test: `src/renderer/store/__tests__/appStore.*.test.ts` (없으면 신규 파일)

**Interfaces:**
- Produces: `ActivityKind = "prompt" | "tool" | "sub-start" | "sub-stop"`. `applyActivityEvent`는 서브 종류를 **무시**(시간추적/라벨에 영향 없음).

- [ ] **Step 1: Write the failing test**

`src/renderer/store/__tests__/appStore.subagentActivity.test.ts` 신규 생성:

```ts
import { describe, expect, it } from "vitest";
import { useAppStore } from "../appStore";

describe("applyActivityEvent: 서브에이전트 신호 무시", () => {
  it("sub-start/sub-stop는 timeTracking을 변경하지 않는다", () => {
    const before = useAppStore.getState().timeTracking["ag-x"];
    useAppStore.getState().applyActivityEvent({
      agentId: "ag-x",
      sessionId: "s1",
      kind: "sub-start",
      at: 1000,
    });
    const after = useAppStore.getState().timeTracking["ag-x"];
    expect(after).toBe(before); // 새 턴 상태 생성 안 됨(undefined 그대로)
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run --dir src src/renderer/store/__tests__/appStore.subagentActivity.test.ts`
Expected: FAIL — `kind: "sub-start"`가 `ActivityKind`에 없어 **타입 에러**, 그리고/또는 `applyActivityEvent`가 `reduceTurn`을 호출해 `timeTracking["ag-x"]`가 생성됨.

- [ ] **Step 3: Write minimal implementation**

(a) `src/shared/types.ts:109` 교체:

```ts
export type ActivityKind = "prompt" | "tool" | "sub-start" | "sub-stop";
```

(b) `src/renderer/store/appStore.ts`의 `applyActivityEvent`(413행) 본문 맨 앞에 가드 추가 — `reduceTurn`이 `TurnInputKind`("prompt"|"tool"|...)만 받으므로 여기서 종류를 좁혀야 타입/런타임 모두 안전:

```ts
    applyActivityEvent: (e) =>
      set((s) => {
        // 서브에이전트 카운트 신호는 시간 추적/라벨 대상이 아니다(카운트는
        // sessionBridge가 별도 소유). reduceTurn의 TurnInputKind로 좁히기 위해서도 필요.
        if (e.kind !== "prompt" && e.kind !== "tool") return {};
        const prevTurn = s.timeTracking[e.agentId] ?? initialTurnState();
        const nextTurn = reduceTurn(prevTurn, { kind: e.kind, at: e.at });
        // ...(이하 기존 코드 그대로)
```

> 413~437행의 나머지는 변경하지 않는다. 가드 한 줄만 `set((s) => {` 직후에 삽입.

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run --dir src src/renderer/store/__tests__/appStore.subagentActivity.test.ts`
그리고 타입 체크: `npx tsc --noEmit`
Expected: 테스트 PASS, tsc 에러 없음.

- [ ] **Step 5: Commit**

```bash
git add src/shared/types.ts src/renderer/store/appStore.ts src/renderer/store/__tests__/appStore.subagentActivity.test.ts
git commit -m "feat(ts): ActivityKind에 sub-start/sub-stop + applyActivityEvent 가드

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: `SubagentCountTracker` 순수 모듈

**Files:**
- Create: `src/renderer/ipc/subagentCounts.ts`
- Test: `src/renderer/ipc/__tests__/subagentCounts.test.ts`

**Interfaces:**
- Produces:
  - `type SubagentCountCb = (agentId: string, count: number) => void`
  - `class SubagentCountTracker` with:
    - `subscribe(cb: SubagentCountCb): () => void`
    - `bump(agentId: string, delta: number): void` — 클램프 `>= 0`, 값이 바뀔 때만 통지
    - `reset(agentId: string): void` — 이미 0이면 no-op(통지 안 함)
    - `get(agentId: string): number`

- [ ] **Step 1: Write the failing test**

`src/renderer/ipc/__tests__/subagentCounts.test.ts` 신규:

```ts
import { describe, expect, it, vi } from "vitest";
import { SubagentCountTracker } from "../subagentCounts";

describe("SubagentCountTracker", () => {
  it("bump으로 증감하고 0에서 클램프한다", () => {
    const t = new SubagentCountTracker();
    t.bump("a", +1);
    t.bump("a", +1);
    expect(t.get("a")).toBe(2);
    t.bump("a", -1);
    t.bump("a", -1);
    t.bump("a", -1); // 이미 0 → 클램프
    expect(t.get("a")).toBe(0);
  });

  it("값이 실제로 바뀔 때만 구독자에게 통지한다", () => {
    const t = new SubagentCountTracker();
    const cb = vi.fn();
    t.subscribe(cb);
    t.bump("a", +1); // 0→1 통지
    t.bump("a", -1); // 1→0 통지
    t.bump("a", -1); // 0→0 통지 안 함
    expect(cb.mock.calls).toEqual([["a", 1], ["a", 0]]);
  });

  it("reset은 0으로 만들고, 0이 아니었을 때만 통지한다", () => {
    const t = new SubagentCountTracker();
    const cb = vi.fn();
    t.bump("a", +1);
    t.subscribe(cb);
    t.reset("a"); // 1→0 통지
    t.reset("a"); // 0→0 통지 안 함
    expect(t.get("a")).toBe(0);
    expect(cb.mock.calls).toEqual([["a", 0]]);
  });

  it("unsubscribe 후에는 통지하지 않는다", () => {
    const t = new SubagentCountTracker();
    const cb = vi.fn();
    const off = t.subscribe(cb);
    off();
    t.bump("a", +1);
    expect(cb).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run --dir src src/renderer/ipc/__tests__/subagentCounts.test.ts`
Expected: FAIL — `Cannot find module '../subagentCounts'`.

- [ ] **Step 3: Write minimal implementation**

`src/renderer/ipc/subagentCounts.ts` 신규:

```ts
// src/renderer/ipc/subagentCounts.ts
//
// 부모 agentId별 "활성 서브에이전트 수"를 소유하는 순수 렌더러 모듈.
// 백엔드에는 카운트 상태가 없다 — sub-start/sub-stop activity로 여기서 증감하고,
// 세션 종료/턴 종료(Stop)에서 reset한다. 카운트는 순수 시각 효과라 휘발이 정답.
// zustand가 아닌 plain 콜백 릴레이(리렌더 불필요, Pixi 전용 신호).

export type SubagentCountCb = (agentId: string, count: number) => void;

export class SubagentCountTracker {
  private counts = new Map<string, number>();
  private cbs = new Set<SubagentCountCb>();

  subscribe(cb: SubagentCountCb): () => void {
    this.cbs.add(cb);
    return () => this.cbs.delete(cb);
  }

  get(agentId: string): number {
    return this.counts.get(agentId) ?? 0;
  }

  bump(agentId: string, delta: number): void {
    this.set(agentId, this.get(agentId) + delta);
  }

  reset(agentId: string): void {
    this.set(agentId, 0);
  }

  private set(agentId: string, next: number): void {
    const clamped = next < 0 ? 0 : next;
    if (clamped === this.get(agentId)) return; // 변화 없으면 통지 생략
    this.counts.set(agentId, clamped);
    this.cbs.forEach((cb) => cb(agentId, clamped));
  }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run --dir src src/renderer/ipc/__tests__/subagentCounts.test.ts`
Expected: 4건 PASS.

- [ ] **Step 5: Commit**

```bash
git add src/renderer/ipc/subagentCounts.ts src/renderer/ipc/__tests__/subagentCounts.test.ts
git commit -m "feat(ts): SubagentCountTracker 순수 카운트 모듈

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: `OfficeBus.onSubagentCountChanged` + sessionBridge 배선

**Files:**
- Modify: `src/renderer/office/bus.ts` (인터페이스 `OfficeBus`, `MockOfficeBus`, `createMockOfficeBus`)
- Modify: `src/renderer/ipc/sessionBridge.ts` (트래커 인스턴스화, `officeBus` 구현, `offActivity` 분기, 리셋)
- Test: `src/renderer/office/__tests__/bus.test.ts` (기존 파일에 추가)

**Interfaces:**
- Consumes: `SubagentCountTracker` (Task 6).
- Produces:
  - `OfficeBus.onSubagentCountChanged(cb: (agentId: string, count: number) => void): () => void`
  - `MockOfficeBus.triggerSubagentCountChanged(agentId: string, count: number): void`
  - `officeBus`(sessionBridge)가 `onSubagentCountChanged`를 트래커에 위임하고, activity `sub-start`/`sub-stop`에서 카운트를 증감, `exited`/`disposed`/알림 `stop`에서 리셋.

- [ ] **Step 1: Write the failing test**

`src/renderer/office/__tests__/bus.test.ts`에 추가:

```ts
import { describe, expect, it, vi } from "vitest";
import { createMockOfficeBus } from "../bus";

describe("OfficeBus: onSubagentCountChanged", () => {
  it("구독자가 triggerSubagentCountChanged로 (agentId, count)를 받는다", () => {
    const bus = createMockOfficeBus();
    const cb = vi.fn();
    const off = bus.onSubagentCountChanged(cb);
    bus.triggerSubagentCountChanged("a1", 2);
    expect(cb).toHaveBeenCalledWith("a1", 2);
    off();
    bus.triggerSubagentCountChanged("a1", 3);
    expect(cb).toHaveBeenCalledTimes(1);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run --dir src src/renderer/office/__tests__/bus.test.ts`
Expected: FAIL — `onSubagentCountChanged`/`triggerSubagentCountChanged`가 mock에 없음(타입/런타임).

- [ ] **Step 3: Write minimal implementation**

(a) `bus.ts`의 `OfficeBus` 인터페이스에 추가(다른 `on...` 구독과 같은 위치):

```ts
  /** B가 부모별 활성 서브에이전트 수 변화를 구독(미니 캐릭터 표시용). */
  onSubagentCountChanged(cb: (agentId: string, count: number) => void): () => void;
```

(b) `MockOfficeBus`에 추가:

```ts
  /** Drives subagent-count changes from a test/manual harness. */
  triggerSubagentCountChanged(agentId: string, count: number): void;
```

(c) `createMockOfficeBus`에 리스너 셋과 구현 추가:

```ts
  const subagentCountListeners = new Set<(agentId: string, count: number) => void>();
```
반환 객체에:
```ts
    onSubagentCountChanged(cb) {
      subagentCountListeners.add(cb);
      return () => subagentCountListeners.delete(cb);
    },
    triggerSubagentCountChanged(agentId, count) {
      for (const cb of subagentCountListeners) cb(agentId, count);
    },
```

(d) `sessionBridge.ts`:
- 상단 import 추가: `import { SubagentCountTracker } from "./subagentCounts";`
- 모듈 스코프에 인스턴스 추가(다른 `const ...Cbs` 근처):
```ts
const subagentCounts = new SubagentCountTracker();
```
- `officeBus` 객체에 메서드 추가(예: `onSessionStateChanged` 아래):
```ts
  onSubagentCountChanged(cb) {
    return subagentCounts.subscribe(cb);
  },
```
- `installSessionBridge`의 `offActivity`를 교체(서브 종류는 카운트로 분기, 나머지는 기존대로):
```ts
  const offActivity = tauriApi.onActivity((e) => {
    if (e.kind === "sub-start") {
      subagentCounts.bump(e.agentId, +1);
      return;
    }
    if (e.kind === "sub-stop") {
      subagentCounts.bump(e.agentId, -1);
      return;
    }
    useAppStore.getState().applyActivityEvent(e);
  });
```
- `offState`(175행)에서 세션 종료 시 리셋 — 콜백 본문 끝(`stateCbs.forEach(...)` 뒤)에 추가:
```ts
    if (e.state === "exited" || e.state === "disposed") subagentCounts.reset(e.agentId);
```
- `offNotif`(190행)에서 Stop 리셋 — `applyNotificationTiming` 뒤에 추가:
```ts
    if (e.source === "stop") subagentCounts.reset(e.agentId);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run --dir src src/renderer/office/__tests__/bus.test.ts`
그리고 `npx tsc --noEmit`
Expected: 테스트 PASS, tsc 에러 없음(`officeBus`가 확장된 `OfficeBus`를 만족).

- [ ] **Step 5: Commit**

```bash
git add src/renderer/office/bus.ts src/renderer/ipc/sessionBridge.ts
git commit -m "feat(ts): OfficeBus.onSubagentCountChanged + sessionBridge 카운트 배선

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: `MiniAgentsOverlay` 축소 클론 오버레이

**Files:**
- Create: `src/renderer/office/entities/MiniAgentsOverlay.ts`
- Test: `src/renderer/office/entities/__tests__/MiniAgentsOverlay.test.ts`

**Interfaces:**
- Consumes: Pixi `Texture`(부모 `assets.idle[0]`), 부모 `spriteScale`.
- Produces:
  - `class MiniAgentsOverlay` with `readonly root: Container`, `setCount(n: number): void`, `update(dt: number): void`, `destroy(): void`.
  - `root.children`는 항상 3개(미리 생성, `visible` 토글). 표시 수 = `min(max(n,0),3)`.

- [ ] **Step 1: Write the failing test**

`src/renderer/office/entities/__tests__/MiniAgentsOverlay.test.ts` 신규(ThinkingOverlay 테스트 패턴 + BufferImageSource 텍스처):

```ts
import { describe, expect, it } from "vitest";
import { BufferImageSource, Texture, type Sprite } from "pixi.js";
import { MiniAgentsOverlay } from "../MiniAgentsOverlay";

const tex = (): Texture =>
  new Texture({
    source: new BufferImageSource({ resource: new Uint8Array([255, 255, 255, 255]), width: 1, height: 1, label: "t" }),
    label: "t",
  });

const visibleCount = (o: MiniAgentsOverlay): number =>
  o.root.children.filter((c) => c.visible).length;

describe("MiniAgentsOverlay", () => {
  it("항상 3개 스프라이트를 미리 만들고, 초기 표시 수는 0", () => {
    const o = new MiniAgentsOverlay(tex(), 1);
    expect(o.root.children.length).toBe(3);
    expect(visibleCount(o)).toBe(0);
  });

  it("setCount(2)는 2개만 보이게 한다", () => {
    const o = new MiniAgentsOverlay(tex(), 1);
    o.setCount(2);
    expect(visibleCount(o)).toBe(2);
  });

  it("setCount는 3에서 캡한다", () => {
    const o = new MiniAgentsOverlay(tex(), 1);
    o.setCount(7);
    expect(visibleCount(o)).toBe(3);
  });

  it("setCount(0)/음수는 모두 숨긴다", () => {
    const o = new MiniAgentsOverlay(tex(), 1);
    o.setCount(3);
    o.setCount(-4);
    expect(visibleCount(o)).toBe(0);
  });

  it("스케일은 부모 spriteScale의 절반, alpha 0.75", () => {
    const o = new MiniAgentsOverlay(tex(), 2);
    const s = o.root.children[0] as Sprite;
    expect(s.scale.x).toBeCloseTo(1); // 2 * 0.5
    expect(s.alpha).toBeCloseTo(0.75);
  });

  it("update(dt)는 보이는 미니의 y를 밥(bob)으로 흔든다", () => {
    const o = new MiniAgentsOverlay(tex(), 1);
    o.setCount(1);
    const s = o.root.children[0] as Sprite;
    const y0 = s.y;
    o.update(300); // 주기 1200ms의 1/4 → sin 최대 근처로 이동
    expect(s.y).not.toBeCloseTo(y0);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run --dir src src/renderer/office/entities/__tests__/MiniAgentsOverlay.test.ts`
Expected: FAIL — `Cannot find module '../MiniAgentsOverlay'`.

- [ ] **Step 3: Write minimal implementation**

`src/renderer/office/entities/MiniAgentsOverlay.ts` 신규:

```ts
// src/renderer/office/entities/MiniAgentsOverlay.ts
//
// 부모 캐릭터 머리 옆에 떠다니는 "미니 서브에이전트" 표시. 부모 스프라이트
// 텍스처를 그대로 재사용(복제 아님)해 축소 클론으로 그린다. 카운트 기반:
// setCount(n)이 min(n,3)마리를 보이게 하고, update(dt)가 sin 밥으로 흔든다.
// ThinkingOverlay 패턴(자식 Container + dt 구동 + 캐릭터가 소유/파괴)을 따른다.

import { Container, Sprite, type Texture } from "pixi.js";

const MAX_MINIS = 3;
const SLOT_X = [-11, 11, -15]; // 머리 옆 고정 슬롯
const SLOT_BASE_Y = [0, -1, 2]; // 미니마다 살짝 다른 기준 높이(머리 라인 근처)
const MINI_SCALE_FACTOR = 0.5; // 부모 spriteScale 대비
const MINI_ALPHA = 0.75;
const BOB_AMPLITUDE_PX = 1.5;
const BOB_PERIOD_MS = 1200;
const BOB_PHASE_STEP = (Math.PI * 2) / 3; // 미니 간 위상차

export class MiniAgentsOverlay {
  readonly root = new Container();
  private minis: Sprite[];
  private count = 0;
  private t = 0;

  constructor(texture: Texture, spriteScale: number) {
    this.minis = SLOT_X.map((x, i) => {
      const s = new Sprite(texture);
      s.anchor.set(0.5, 1); // feet-aligned, 부모와 동일
      s.scale.set(spriteScale * MINI_SCALE_FACTOR);
      s.alpha = MINI_ALPHA;
      s.position.set(x, SLOT_BASE_Y[i]);
      s.visible = false;
      this.root.addChild(s);
      return s;
    });
  }

  setCount(n: number): void {
    const clamped = Math.max(0, Math.min(MAX_MINIS, Math.floor(n)));
    this.count = clamped;
    this.minis.forEach((s, i) => {
      s.visible = i < clamped;
    });
  }

  /** dt: ms. 보이는 미니만 sin 밥으로 흔든다(숨김이면 계산 생략). */
  update(dt: number): void {
    if (this.count === 0) return;
    this.t += dt;
    for (let i = 0; i < this.count; i++) {
      const phase = (this.t / BOB_PERIOD_MS) * Math.PI * 2 - i * BOB_PHASE_STEP;
      this.minis[i].y = SLOT_BASE_Y[i] + Math.sin(phase) * BOB_AMPLITUDE_PX;
    }
  }

  destroy(): void {
    // 텍스처는 부모 소유 → destroy에서 파괴 금지(children Container만 정리).
    this.root.destroy({ children: true, texture: false });
  }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run --dir src src/renderer/office/entities/__tests__/MiniAgentsOverlay.test.ts`
Expected: 6건 PASS.

- [ ] **Step 5: Commit**

```bash
git add src/renderer/office/entities/MiniAgentsOverlay.ts src/renderer/office/entities/__tests__/MiniAgentsOverlay.test.ts
git commit -m "feat(ts): MiniAgentsOverlay 축소 클론 미니 오버레이

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: `CharacterEntity`에 미니 오버레이 배선

**Files:**
- Modify: `src/renderer/office/entities/CharacterEntity.ts` (import, 필드, 생성자, `update`, `destroy`, `setSubagentCount`)
- Test: `src/renderer/office/entities/__tests__/CharacterEntity.test.ts` (기존 파일에 추가)

**Interfaces:**
- Consumes: `MiniAgentsOverlay` (Task 8).
- Produces: `CharacterEntity.setSubagentCount(n: number): void`.

- [ ] **Step 1: Write the failing test**

`CharacterEntity.test.ts`는 엔티티를 인라인 생성한다: `new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, makeMap(), () => 0.5)` (파일 상단에 `SEAT = { tx: 2, ty: 2 }`, `makeMap`, `makeTestCharacterAssets` 이미 존재). 자식 순서는 `[sprite(0), exclamation(1), think(2)]`이며, 생성자에서 미니 오버레이를 마지막에 추가하므로 미니 루트 = `children[3]`. 다음을 추가:

```ts
  // 미니 오버레이 루트는 생성 순서상 마지막 자식(children[3]); 그 안의 보이는 Sprite 수를 센다.
  const miniVisible = (e: CharacterEntity): number => {
    const mini = e.root.children[3] as unknown as { children: { visible: boolean }[] };
    return mini.children.filter((s) => s.visible).length;
  };

  it("setSubagentCount가 미니 표시 수를 바꾸고(0~3 캡) update/destroy가 안전하다", () => {
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, makeMap(), () => 0.5);
    expect(miniVisible(e)).toBe(0); // 초기 0
    e.setSubagentCount(2);
    expect(miniVisible(e)).toBe(2);
    e.setSubagentCount(9);
    expect(miniVisible(e)).toBe(3); // 3 캡
    e.update(16); // 예외 없이 bob 갱신
    e.setSubagentCount(0);
    expect(miniVisible(e)).toBe(0);
    e.destroy(); // 미니까지 정리, 예외 없음
  });
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run --dir src src/renderer/office/entities/__tests__/CharacterEntity.test.ts`
Expected: FAIL — `setSubagentCount`가 `CharacterEntity`에 없음.

- [ ] **Step 3: Write minimal implementation**

`CharacterEntity.ts`:
- import 추가(다른 오버레이 import 근처):
```ts
import { MiniAgentsOverlay } from "./MiniAgentsOverlay";
```
- 필드 추가(`thinkOverlay` 근처):
```ts
  private miniOverlay: MiniAgentsOverlay;
```
- 생성자에서 `spriteScale` 계산(90행) 이후, `thinkOverlay` 블록 아래에 추가:
```ts
    this.miniOverlay = new MiniAgentsOverlay(this.assets.idle[0], this.spriteScale);
    this.miniOverlay.root.position.set(0, -TILE_SIZE); // 머리 위(기존 오버레이와 동일 높이)
    this.root.addChild(this.miniOverlay.root);
```
- public 메서드 추가(`setSessionActive` 근처):
```ts
  /** 활성 서브에이전트 수 반영(0~3 미니 표시). */
  setSubagentCount(n: number): void {
    this.miniOverlay.setCount(n);
  }
```
- `update(dt)` 안, `this.overlay.update(dt)` 옆에 추가:
```ts
    this.miniOverlay.update(dt);
```
- `destroy()`에 추가(`this.thinkOverlay.destroy()` 옆):
```ts
    this.miniOverlay.destroy();
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run --dir src src/renderer/office/entities/__tests__/CharacterEntity.test.ts`
Expected: 새 테스트 PASS + 기존 CharacterEntity 테스트 전부 유지.

- [ ] **Step 5: Commit**

```bash
git add src/renderer/office/entities/CharacterEntity.ts src/renderer/office/entities/__tests__/CharacterEntity.test.ts
git commit -m "feat(ts): CharacterEntity에 미니 오버레이 + setSubagentCount 배선

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 10: `OfficeWorld` 카운트 캐시 + 버스 구독

**Files:**
- Modify: `src/renderer/office/world/OfficeWorld.ts` (필드, 생성자 구독, drop/create 루프, destroy)
- Test: `src/renderer/office/world/__tests__/OfficeWorld.test.ts` (기존 파일에 추가)

**Interfaces:**
- Consumes: `OfficeBus.onSubagentCountChanged` (Task 7), `CharacterEntity.setSubagentCount` (Task 9).
- Produces: 버스가 카운트를 방출하면 해당 엔티티에 반영되고, 엔티티 재생성 시 마지막 카운트가 재적용된다.

- [ ] **Step 1: Write the failing test**

`OfficeWorld.test.ts`는 프로필을 `profile(id)` 헬퍼(`= (id) => ({ id, name: id, role: "eng", seed: id })`)로 만들고, `makeMap()`·`createMockOfficeBus`가 이미 있다. 미니 표시 수는 Task 9와 동일하게 엔티티 root의 `children[3]`에서 센다. `characterLayer`에 addChild되는 엔티티 root를 통해 검사:

```ts
  it("버스의 subagent count 변화를 해당 엔티티 미니 표시에 반영한다", () => {
    const bus = createMockOfficeBus();
    const characterLayer = new Container();
    const world = new OfficeWorld({ bus, characterLayer, overlayLayer: new Container(), map: makeMap() });
    world.syncAgents([profile("p1")]);

    bus.triggerSubagentCountChanged("p1", 2);

    // 엔티티 root = characterLayer의 첫 자식; 미니 루트 = 그 root의 children[3].
    const entityRoot = characterLayer.children[0] as unknown as { children: { children: { visible: boolean }[] }[] };
    const miniRoot = entityRoot.children[3];
    expect(miniRoot.children.filter((s) => s.visible).length).toBe(2);

    world.destroy();
  });

  it("카운트가 온 뒤 외형 변경으로 엔티티가 재생성돼도 카운트가 재적용된다", () => {
    const bus = createMockOfficeBus();
    const characterLayer = new Container();
    const world = new OfficeWorld({ bus, characterLayer, overlayLayer: new Container(), map: makeMap() });
    const p = profile("p1");
    world.syncAgents([p]);
    bus.triggerSubagentCountChanged("p1", 2);

    // seed 변경 → appearanceKey 변경 → 엔티티 재생성.
    world.syncAgents([{ ...p, seed: p.seed + "x" }]);

    const entityRoot = characterLayer.children[0] as unknown as { children: { children: { visible: boolean }[] }[] };
    expect(entityRoot.children[3].children.filter((s) => s.visible).length).toBe(2);

    world.destroy();
  });
```

> 두 테스트 모두 이미 mock으로 대체된 `createCharacterAssets`(파일 상단 `vi.mock`)를 통해 실제 캔버스 없이 동작한다.

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run --dir src src/renderer/office/world/__tests__/OfficeWorld.test.ts`
Expected: FAIL — `triggerSubagentCountChanged` 방출 시 `OfficeWorld`가 아직 구독하지 않아 아무 일도 안 하거나(첫 테스트는 통과할 수도), 재적용 로직 부재. (최소한 구독 배선 전에는 카운트 반영 경로가 없어 후속 표시 검증이 실패.)

- [ ] **Step 3: Write minimal implementation**

`OfficeWorld.ts`:
- 필드 추가(`sessionActive` 근처):
```ts
  private subagentCounts = new Map<string, number>();
```
- 생성자 구독 추가(`onSessionStateChanged` 구독 아래):
```ts
    this.unsub.push(
      o.bus.onSubagentCountChanged((agentId, count) => {
        this.subagentCounts.set(agentId, count);
        this.entities.get(agentId)?.setSubagentCount(count);
      }),
    );
```
- drop 루프(`!next.has(id)`, 90-96행)에서 캐시 정리 추가:
```ts
      this.subagentCounts.delete(id);
```
- create 루프(124-138행)에서 엔티티 생성 직후, `entity.setSessionActive(...)` 옆에 추가:
```ts
      entity.setSubagentCount(this.subagentCounts.get(p.id) ?? 0);
```
- `destroy()`에 추가:
```ts
    this.subagentCounts.clear();
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run --dir src src/renderer/office/world/__tests__/OfficeWorld.test.ts`
그리고 `npx tsc --noEmit`
Expected: 새 테스트 PASS + 기존 OfficeWorld 테스트 유지, tsc 에러 없음.

- [ ] **Step 5: Commit**

```bash
git add src/renderer/office/world/OfficeWorld.ts src/renderer/office/world/__tests__/OfficeWorld.test.ts
git commit -m "feat(ts): OfficeWorld 서브에이전트 카운트 캐시 + 버스 구독

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 11: 전체 빌드·테스트·실제 앱 검증

**Files:** (없음 — 검증 전용)

- [ ] **Step 1: 렌더러 전체 타입체크 + 테스트**

Run: `npx tsc --noEmit && npx vitest run --dir src`
Expected: tsc 에러 0, 신규/기존 vitest 전부 PASS.

- [ ] **Step 2: Rust 전체 테스트**

Run: `cd src-tauri && cargo test --lib`
Expected: 새 테스트 전부 PASS. 기존 실패는 macOS `bash_wrapper` 1건뿐(사전 존재, 무관). 그 외 새 실패 0.

- [ ] **Step 3: 프로덕션 빌드**

Run: `npm run build`
Expected: `tsc && vite build` 성공(타입/번들 에러 0).

- [ ] **Step 4: 실제 앱에서 눈으로 확인**

`/run` 스킬 또는 `npm run tauri dev`로 앱을 띄우고:
1. 에이전트 하나를 소환해 터미널에서 `claude` 실행.
2. Claude에게 **서브에이전트를 병렬로 2~3개 소환**하는 작업 지시(예: "Task 툴로 파일 3개를 각각 다른 서브에이전트가 동시에 읽게 해줘").
3. **관찰**: 서브에이전트가 도는 동안 부모 캐릭터 머리 옆에 축소 클론 미니가 (활성 수만큼, 최대 3) 떠다니며 bob 하는지.
4. 서브에이전트가 끝나거나 턴이 종료(Stop)되면 미니가 사라지는지.
5. 미니를 여러 개 띄운 상태에서 캐릭터가 탕비실로 걸어가도 미니가 부모를 따라가는지(자식 오버레이라 자동 추종).

Expected: 위 5개 관찰이 모두 성립. 어긋나면 systematic-debugging으로 회귀.

- [ ] **Step 5: 브랜치 마무리**

verification-before-completion 스킬로 위 증거를 확인한 뒤, finishing-a-development-branch 스킬로 병합/PR 방식을 사용자와 결정.

---

## Notes / 엣지케이스 (스펙에서 이월)

- **병렬 Task N개**: 훅이 각각 발화, dedup 없음 → 정확히 N(단 표시는 3 캡).
- **Task 실패/서브에이전트 미생성**: `SubagentStop` 미발화 → 카운트 누수 → 턴 종료(Stop) 리셋이 자가치유.
- **curl 유실(2s 타임아웃)**: 드리프트 → Stop 리셋이 치유.
- **세션 죽음/앱 재시작**: exited/disposed 리셋 + 카운트는 휘발(복원 없음).
