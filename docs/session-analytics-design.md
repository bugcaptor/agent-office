# 세션 활동 분석 패널 — 설계 스펙

- 날짜: 2026-07-17
- 상태: 설계 확정, 구현 착수
- 선행: `docs/session-event-timeseries-design.md` (수집 완료·가동 중). 그 설계의 비목표였던 "분석 UI"를 이번 범위로 승격한다.

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
- 수집 측(`SessionEventStore`, `RecordingAppEvents`) 변경.

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
