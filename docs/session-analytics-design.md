# 세션 활동 분석 패널 — 설계 스펙

- 날짜: 2026-07-17 (상태 갱신: 2026-08-21 — §9 토큰·비용 확장 추가)
- 상태: 정본 — 구현 완료. 구현: `session_events/reader.rs`(Rust), `renderer/analytics/`(집계·UI), 커맨드 `load_session_events`(`ipc/commands/persistence.rs`). 토큰·비용은 `observer/event.rs`·`observer/codex_usage.rs`(추출), `renderer/analytics/pricing.ts`(단가).
- 선행: `docs/archive/session-event-timeseries-design.md` (수집 계층, archived — 코드가 정본). 그 설계의 비목표였던 "분석 UI"를 이번 범위로 승격한다.
- usage(한도) 표시와는 별개 기능 — 그쪽은 `docs/usage-design.md` 참조(상호 링크만, 데이터 소스가 다름).

## 1. 배경과 목표

`<app-data>/session-events/v1/YYYY-MM-DD.jsonl`에 세션 원천 이벤트가 쌓이고 있으나 앱 안에서 이를 볼 수단이 없다. 기존 `SessionTimePanel`은 실행 중 누적값만 보여주고 과거 추이를 복원하지 못한다.

이 기능은 쌓인 시계열에서 캐릭터별·일별 활동을 재구성해 앱 내 분석 패널로 보여준다.

## 2. 범위

### 목표

- 기간(최근 7/14/30일)을 선택해 일별 작업시간을 캐릭터별 스택 막대로 표시한다.
- 캐릭터별 요약(추정 작업시간, 턴 수, 도구 이벤트 수, 활동일 수)을 표로 표시한다.
- 삭제된 캐릭터도 `session_started` 스냅샷 이름으로 표시한다.
- 패널을 열 때만 디스크를 읽는다. 부트스트랩 프리로드 없음.

### 비목표

- 실시간 갱신(열려 있는 동안 새 이벤트 반영), CSV/이미지 내보내기, 시간대 히트맵.
- 차트 라이브러리 도입. SVG 자체 구현으로 한정한다.
- `session-times.jsonl`·`SessionTimePanel`의 변경 또는 대체.
- 수집 측(`SessionEventStore`, `RecordingAppEvents`) 변경. — **§9에서 해제됨**:
  턴 토큰을 실으려면 수집 측에 옵션 필드를 더해야 했다.

## 3. 선택한 접근

**백엔드는 원시 레코드만 돌려주고, 집계는 렌더러 순수 함수가 한다.**

- 백엔드 집계는 지표 변경마다 Rust·IPC 계약 수정을 강제한다. 원시 반환이면 지표 진화가 TS 안에서 끝난다.
- 데이터량은 기간 제한(≤31일)으로 억제된다. 현재 밀도(하루 수백~수천 줄, 줄당 ~200B)에서 30일이면 수 MB 이내다.
- 집계를 TS 순수 함수로 두면 vitest로 경계(자정, stop 유실, 다중 prompt)를 값싸게 검증할 수 있다.
- 로컬 날짜 기준 집계는 렌더러가 한다. 파일 파티션은 UTC지만 사용자는 로컬 하루 단위로 생각하기 때문이다.

## 4. 컴포넌트와 책임

### 4.1 Rust `session_events/reader.rs` (신설)

`SessionEventStore`는 쓰기 전용 원칙을 유지하고, 읽기는 별도 함수로 둔다.

```rust
pub fn load_session_events(root: &Path, from_at: u64, to_at: u64) -> Vec<SessionEventRecord>
```

- `from_at`의 UTC 날짜부터 `to_at`의 UTC 날짜까지 `YYYY-MM-DD.jsonl`을 순서대로 연다. 없는 파일은 건너뛴다.
- 각 줄을 `SessionEventRecord`로 파싱한다. 빈 줄·파싱 불가 줄은 조용히 건너뛴다(부분 기록 내구성, 선행 설계 §7).
- `from_at <= at <= to_at` 필터 후 `(at, runId, seq)`로 정렬해 반환한다.
- 파일 파티션 키가 `at`의 UTC 날짜이므로 이 스캔 범위가 필터 범위를 완전히 덮는다.
- I/O 오류는 해당 파일만 건너뛰고 계속한다. 반환은 항상 성공한다.

### 4.2 IPC 커맨드 `load_session_events`

기존 5접점 계약을 따른다: `ipc/commands.rs`, `lib.rs` generate_handler, `shared/ipc.ts` Commands, `renderer/ipc/tauriApi.ts`, `shared/types.ts` AgentOfficeApi. 요청은 `fromAt`/`toAt`(epoch ms), 응답은 `SessionEventRecord[]`.

`SessionEventRecord`의 TS 미러 타입을 `shared/types.ts`에 추가하고 `contract.test.ts`에 픽스처를 넣는다. serde 필드명은 수집 설계 §4와 동일(camelCase envelope + 옵션 필드 + snake_case `kind`/`state`).

### 4.3 렌더러 집계 `renderer/analytics/aggregate.ts` (순수 함수)

**턴 재구성** — `(agentId, sessionId)`별로 시간순 처리:

- `prompt` 수신 시 열린 턴이 없으면 턴 시작. 이미 열려 있으면 무시(연속 프롬프트는 같은 턴).
- `stop` 수신 시 열린 턴을 닫고 `workedMs += stop.at - start.at`. 열린 턴이 없으면 무시.
- `session_state`의 `exited`/`disposed` 수신 시 열린 턴이 있으면 그 시각으로 강제 마감(stop 유실 대비).
- 데이터 끝까지 안 닫힌 턴은 해당 세션의 마지막 이벤트 시각으로 마감한다.
- 자정을 걸치는 턴은 로컬 날짜 경계에서 분할해 일별 합산에 나눠 넣는다.

**일별 요약** — `dailySummary(events, turns, dayKeyFn)`:

- 로컬 날짜 키(`YYYY-MM-DD`)별·에이전트별 `{ workedMs, turns, toolEvents }`.
- 턴 수·도구 이벤트는 발생 시각의 로컬 날짜에 귀속한다.

**에이전트 메타** — `agentId`별 표시 이름은 현재 프로필 우선, 없으면(삭제됨) 기간 내 마지막 `session_started.agentName`, 그것도 없으면 ID 축약. 색상은 현재 프로필 팔레트에서 대표색을 뽑고, 삭제된 에이전트는 중립 회색 계열을 순환 배정한다.

### 4.4 UI `renderer/analytics/`

- `AnalyticsDialog.tsx`: `ModalState`에 `kind: "analytics"` 추가, `App.tsx` ModalRoot에 상시 마운트 + 셀프 게이팅(SettingsDialog 패턴). BottomBar에 열기 버튼 추가.
- 열릴 때 `loadSessionEvents(now-기간, now)` 1회 호출. 로딩/빈 데이터 상태 표시. 기간 변경 시 재호출.
- `DailyBarChart.tsx`: SVG 스택 막대. 가로 = 로컬 일, 세로 = 작업시간, 스택 = 에이전트. 막대 hover 시 title로 상세. 축은 시간 단위 자동(분/시간).
- 요약 표: 에이전트별 작업시간·턴·도구 이벤트·활동일, 작업시간 내림차순.
- 스타일은 `analytics.css` 신설, `pixel-panel`·`tokens.css` 토큰 재사용(timeline.css 패턴).

## 5. 데이터 흐름

```text
[분석 버튼] → openModal(analytics)
  → tauriApi.loadSessionEvents(fromAt, toAt)
    → IPC load_session_events → reader가 v1/*.jsonl 스캔 → SessionEventRecord[]
  → aggregate.ts (턴 재구성 → 일별 요약, 로컬 tz)
  → DailyBarChart + 요약 표
```

## 6. 오류 처리

- 읽기 실패·손상 줄은 결과 축소로만 나타난다. 패널은 절대 앱 동작을 막지 않는다.
- IPC 실패 시 패널에 오류 문구와 재시도 버튼을 보여준다.
- 기간 내 이벤트 0건이면 빈 상태 문구를 보여준다.

## 6.1 알려진 한계 (의도된 트레이드오프)

조회는 표시 기간 앞에 고정 24시간 lookback을 붙여 경계를 걸친 턴을 복원한다. 고정 창인 이상 다음 두 경우는 복원하지 못하며, 이는 수용한다.

- **lookback보다 먼저 시작한 턴**: prompt가 `fromAt - 24h` 이전이고 stop만 범위 안이면 그 턴은 집계에서 빠진다. 실제 턴은 분 단위이고 `session_state`의 exited/disposed도 턴을 마감하므로, 이 경우는 stop 없이 24시간 이상 열려 있던 병리적 턴에 한한다.
- **삭제 에이전트의 오래된 스냅샷 이름**: `session_started`가 lookback 밖이면 이름이 축약 ID로 폴백한다("(삭제됨)" 표기는 유지). 표시용 문제일 뿐 집계 수치에는 영향이 없다.

세션별 역방향 경계 탐색(직전 open-turn·마지막 session_started 조회)은 날짜 파티션 append-only 저장에 무한 역스캔 읽기 경로를 추가해야 해 도입하지 않는다. 실사용에서 이 한계가 관측되면 그때 lookback 상수 확대 또는 별도 인덱스를 재검토한다.

## 7. 테스트 전략

- **Rust reader**: 범위 내 다중 파일 스캔, 없는 파일 스킵, 손상 줄 스킵, `at` 경계 필터, 정렬. tempdir에 픽스처 파일 작성.
- **계약**: `SessionEventRecord` TS 미러 ↔ Rust serde 왕복 픽스처(`contract.test.ts`).
- **aggregate.ts**: prompt→stop 기본 페어링, 연속 prompt, stop 유실+exited 마감, 미마감 턴, 자정 분할, 로컬 날짜 귀속, 삭제 에이전트 이름 폴백.
- **UI**: 다이얼로그 열림/로딩/빈/오류 상태, 기간 전환 재호출(기존 dialog 테스트 패턴).
- **회귀**: 기존 vitest 전체(`npx vitest run --dir src`)·cargo 전체가 기준선 대비 실패 증가 없음.

## 8. 완료 조건

- 분석 버튼으로 패널을 열면 최근 7일 일별 스택 막대와 캐릭터 요약 표가 보인다.
- 기간 7/14/30일 전환이 동작한다.
- 삭제된 캐릭터의 과거 활동이 스냅샷 이름으로 보인다.
- 수집 경로 코드는 변경되지 않는다.
- 신규 테스트 전부 통과, 기존 기준선 실패 증가 없음.

## 9. 확장 — 턴 토큰 사용량과 API 환산 비용 (2026-08-21)

분석 표에 **토큰**과 **추정 비용(API 환산)** 열을 더한다. 원천은 CLI가 남기는
전사/rollout이며, 앱은 턴 종료 시점에 그 턴 몫만 뽑아 시계열에 적어 둔다.

### 9.1 기록 계층

`SessionEventRecord`에 옵션 필드 `tokens`를 더한다(`{input, output, cacheRead,
cacheWrite, model}`, 전부 옵션). **`schemaVersion`은 1을 유지한다** — 옵션 추가는
하위호환이고, 토큰이 없는 과거 파일과 한 디렉터리에 섞여도 그대로 읽힌다.
`kind="stop"` 레코드에만, 그것도 추출에 성공했을 때만 실린다.

배선은 기존 알림 경로를 그대로 탄다:
`ObserverEvent::Stop{tokens}` → `NotificationHub::ingest_with_tokens` →
`NotificationEvent{tokens}` → `RecordingAppEvents` → stop 레코드.
따라서 **알림이 나가지 않는 Stop(서브에이전트 진행 중 running>0, dedup 억제,
죽은 세션)은 사용량도 함께 버려진다** — 그 턴의 stop 레코드 자체가 없어 붙일
곳이 없기 때문이다. 의도된 트레이드오프다.

`input`은 **캐시를 제외한 순수 입력**으로 정규화한다(Claude `input_tokens`,
Codex `input_tokens - cached_input_tokens`). 세 입력 항목을 더해야 전체 입력이
되므로 비용 환산에서 이중 계산이 나지 않는다.

### 9.2 추출 — Claude

Stop 훅 body의 `transcript_path`(JSONL) **꼬리 2MB**를 읽어 뒤에서부터 스캔하며,
`type=="assistant"` 줄의 `message.usage`를 합산한다. 종료 조건은 "첫 진짜 사용자
프롬프트 줄"(`is_real_user_prompt` — tool_result만 있는 user 줄은 경계가 아니다)로,
완료 메시지 추출(§이슈 #39)이 쓰는 판정을 그대로 재사용한다.

두 가지 함정을 코드가 명시적으로 다룬다.

- **같은 응답이 여러 줄로 쪼개진다.** Claude는 assistant 응답 하나를 content
  블록별(thinking/text/tool_use)로 나눠 여러 줄에 쓰면서 `message.usage`를 매 줄에
  **복제**한다. 줄 단위 합산은 2~3배 과대 집계가 되므로 `message.id`로 중복을 제거한다.
- **서브에이전트(`isSidechain`) 줄.** 사용량은 합산한다(실제 청구되는 비용이고 이
  턴에 속한다). 다만 경계 판정에서는 스킵한다 — 서브에이전트의 프롬프트 줄을 메인
  턴 경계로 오인하면 스캔이 조기 종료된다. 대표 `model`은 가장 최근 **메인 세션**
  응답의 것을 쓴다(서브에이전트는 다른 모델일 수 있다).

**왜 "전 세션 누계의 델타"가 아닌가**: 전사에는 누계 필드가 없어 어차피 구간을
합산해야 하고, 이 방식은 상태를 들지 않아 앱 재시작·세션 입양 후에도 정확하다.
꼬리 2MB 안에서 경계를 못 찾으면 찾은 데까지만 합산한다(과소 집계로 강등).

### 9.3 추출 — Codex

Codex 훅 body에는 사용량이 없다. rollout(`<CODEX_HOME>/sessions/YYYY/MM/DD/
rollout-*.jsonl`)의 `token_count` 이벤트가 `info.total_token_usage`(**세션 누계**)를
담고 있으므로 **턴 경계 두 지점의 누계 차이**를 쓴다(`observer/codex_usage.rs`).

- UserPromptSubmit에서 기준 누계를 기억한다. 한 턴에 프롬프트가 여러 번 와도(작업
  중 추가 지시) 기준을 밀지 않는다 — 그 사이에 쓴 토큰이 새기 때문.
- Stop에서 `현재 − 기준`을 싣고 현재 값을 새 기준으로 갱신한다. 기준이 없으면(앱이
  세션 도중에 켜짐) **그 턴은 조용히 생략**하고 기준만 심는다 — 세션 누계 전체를 한
  턴에 몰아넣는 과대 집계보다 누락이 낫다.
- rollout 찾기는 훅 body의 `session_id`가 파일명 꼬리(`...-<id>.jsonl`)와 같다는
  규약을 먼저 쓰고, 없으면 `cwd`와 첫 줄 `session_meta.cwd`가 같은 최근 파일로
  폴백한다(`session_log/agent_transcript/codex.rs`와 같은 휴리스틱). 찾은 경로는
  세션별로 캐시한다.
- 모델은 `turn_context.payload.model`. 꼬리에서 만나면 그 값을, 못 만나면 파일
  앞머리(256KB)에서 읽어 캐시해 둔 첫 모델을 쓴다.

pi는 전사/rollout 경로가 없어 항상 `tokens: None`이다.

### 9.4 비용 환산 (프런트)

단가표는 `renderer/analytics/pricing.ts`. 집계가 이미 프런트 순수 함수라 단가도
프런트에 두는 편이 일관적이고, 요율이 바뀌어도 표 하나만 고치면 된다.

- 모델 ID를 소문자화해 **부분문자열 패턴 목록을 위에서 아래로 첫 일치**로 훑는다
  (`fable`/`mythos`/`opus`/`sonnet`/`haiku`/`gpt-5`/`gpt-4.1`/`gemini-2.5-pro`/`gemini`).
  날짜 접미사가 붙은 ID(`claude-opus-4-5-20250929`)도 자연히 잡힌다.
- **미지 모델은 비용에서 제외**하고 `costUnknownTurns`로 따로 센다. 화면은 그런 행의
  비용에 `~`를 붙여 부분 집계임을 드러낸다.
- 숫자는 공개 API 요율 기준 **대표값**이다. 장문 할증·배치 할인·계약 단가·구독제는
  반영하지 않으며, 표 아래 각주로 그렇게 못박는다.

### 9.5 표시

요약 표 열: 캐릭터 / 작업시간 / 턴 / 도구 / **토큰** / **추정 비용** / 활동일.
토큰 셀은 `input+output` 축약값이고 캐시 항목은 툴팁으로 보조한다. 토큰이나 비용이
없는 행(과거 기록·미지 모델)은 `—`.

토큰·비용은 작업시간과 달리 **자정 경계로 쪼개지 않고 stop 시각의 로컬 날짜에 통째로
귀속**한다 — 마감 시점에 한꺼번에 확정되는 값이라 시간 비례로 나눌 근거가 없다.
`range` 게이트는 도구 이벤트와 똑같이 적용한다(창 밖 stop의 토큰은 제외).

## 10. 확장 — 프롬프트 출처(봇 주입) 분리 (2026-08-25)

"이 달의 우수사원"이 사람 몫만 세려면 턴 하나하나가 사람에게서 온 것인지 봇에게서
온 것인지 알아야 한다. 결정과 근거의 정본은 `docs/employee-of-the-month-design.md`
§2이고, 여기에는 **시계열 계층에 무엇이 늘었는지**만 적는다.

`SessionEventRecord`에 옵션 필드 `origin`(`"bot"` 하나뿐)을 더한다. **`schemaVersion`은
1을 유지한다** — `tokens`(§9.1)와 같은 이유로 옵션 추가는 하위호환이고, 출처가 없는
과거 파일과 한 디렉터리에 섞여도 그대로 읽힌다. `kind="prompt"` 레코드에만, 그것도
봇이 주입했을 때만 실린다(사람 프롬프트는 키 자체가 없다).

봇은 별도 세션을 띄우지 않고 이미 떠 있는 터미널에 `write_input`으로 프롬프트를
밀어넣으므로 세션·agentId로는 구분할 수 없다. 그래서 배선은 알림 경로가 아니라
**주입 지점에서 시작하는 표식**이다: `bot/runner.rs::inject()`가 쓰기 직전에
`state.rs`의 `BotPromptArms`에 그 agent를 arm → `RecordingAppEvents`가 그 agent의
다음 prompt 이벤트 **하나**로 소비. 표식은 소비되면 사라지고 **TTL 120초**로
만료된다(주입 직후 세션이 죽어 프롬프트가 끝내 안 올 때, 남은 표식이 다음 사람
프롬프트를 오염시키는 것을 막는다).

집계(`aggregate.ts`)는 `Turn`에 턴을 연 prompt의 `origin`을 싣고, `AgentDailyStat`·
`AgentSummary`에 `botWorkedMs`/`botTurns`를 **분리 누적**한다. 기존 `workedMs`/`turns`는
**총계 그대로**다 — 분석 패널이 보여주는 수치는 바뀌지 않는다. 날짜 집합만은 뺄셈으로
복원할 수 없어 `humanActiveDays`를 따로 센다.
