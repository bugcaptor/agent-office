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
과거 파일은 `kind="stop"` 레코드에 실렸지만, 지금은 전용 `kind="usage"` 레코드에만
실린다(아래 배선 참고) — **소비자는 kind가 아니라 tokens 유무로 합산해야** 신구
파일을 모두 커버한다.

**배선은 알림과 분리된 별도 채널이다**: `ObserverEvent::Stop{tokens}` → hub가
알림 게이트(`running==0`)·dedup보다 **먼저** `AppEvents::turn_usage`를 호출 →
`RecordingAppEvents`가 `kind="usage"` 레코드를 기록하고 `TauriEvents`가
`"turn-usage"` 이벤트를 emit한다(`CompositeEvents`가 둘 다에 전달). 알림 경로
(`notification_new`)는 더 이상 tokens를 만지지 않으므로, 신규 `kind="stop"`
레코드에는 tokens가 애초에 안 실린다.

원래는 사용량 계측이 알림 방출에 업혀 있었다 — Stop이 dedup·hold·`running==0`
게이트로 억제되면 그 턴의 stop 레코드 자체가 안 생겨 토큰이 영구 유실됐다.
특히 백그라운드 서브에이전트가 남은 채 턴이 끝나는 흔한 경우가 문제였다: 그
턴의 `running==0` Stop은 영영 오지 않는다 — 서브 완료 후 claude가
`task-notification`을 새 user 프롬프트로 주입해 새 턴을 열어 버리고, 그
프롬프트가 다음 사용량 스캔의 경계가 되기 때문이다(실전사로 확인). 사용량을
알림에서 분리해 이 유실 경로를 없앴다 — 지금은 억제된 Stop에서도
`turn_usage`가 먼저 방출된다(§11.1에 실시간 wire 쪽 변화를 이어서 적는다).

전환기 이중 계산은 **구조적으로 없다**: 새로 쓰이는 `kind="stop"` 레코드에는
애초에 tokens가 안 실리므로, 과거 stop 레코드(tokens 있음)와 신규 usage
레코드가 겹쳐 같은 턴을 두 번 세는 경우가 생기지 않는다. 남는 유실은
**죽은/미지 세션뿐**이다 — hub가 `resolve_agent`로 agentId를 못 찾는
session_id의 Stop은 usage도 알림도 둘 다 폐기된다(붙일 곳이 없다).

`input`은 **캐시를 제외한 순수 입력**으로 정규화한다(Claude `input_tokens`,
Codex `input_tokens - cached_input_tokens`). 세 입력 항목을 더해야 전체 입력이
되므로 비용 환산에서 이중 계산이 나지 않는다.

### 9.2 추출 — Claude

Stop 훅 body의 `transcript_path`(JSONL) **꼬리 2MB**를 읽어 뒤에서부터 스캔하며,
`type=="assistant"` 줄의 `message.usage`를 합산한다. 스캔 종료 조건은
**워터마크 유무로 갈린다**(우선순위 규칙, 아래 참고): 워터마크가 없으면
"첫 진짜 사용자 프롬프트 줄"(`is_real_user_prompt` — tool_result만 있는 user
줄은 경계가 아니다, 완료 메시지 추출(§이슈 #39)이 쓰는 판정을 그대로 재사용)에서
멈추고, 워터마크가 있으면 프롬프트 경계를 넘어 워터마크 `message.id`를 만날
때까지 계속 스캔한다.

두 가지 함정을 코드가 명시적으로 다룬다.

- **같은 응답이 여러 줄로 쪼개진다.** Claude는 assistant 응답 하나를 content
  블록별(thinking/text/tool_use)로 나눠 여러 줄에 쓰면서 `message.usage`를 매 줄에
  **복제**한다. 줄 단위 합산은 2~3배 과대 집계가 되므로 `message.id`로 중복을 제거한다.
- **서브에이전트(`isSidechain`) 줄.** 사용량은 합산한다(실제 청구되는 비용이고 이
  턴에 속한다). 다만 경계 판정에서는 스킵한다 — 서브에이전트의 프롬프트 줄을 메인
  턴 경계로 오인하면 스캔이 조기 종료된다. 대표 `model`은 가장 최근 **메인 세션**
  응답의 것을 쓴다(서브에이전트는 다른 모델일 수 있다).

**서브에이전트 전사는 더 이상 메인 전사 안에 없다(2026-08-29 수정).** CLI 2.1.x는
Task 서브에이전트 대화를 별도 파일에 쓴다 — `<proj>/<session>.jsonl`(Stop 훅이
주는 경로) 옆의 `<proj>/<session>/subagents/agent-<id>.jsonl`이다(짝이 되는
`.meta.json`에 `agentType`·`toolUseId`·`spawnDepth`). 최근 14일 전사에서
메인 파일 안의 `isSidechain` 줄은 **0건**이었고 서브에이전트 파일은 2026-06-05
이후로 쌓여 있었다 — 즉 위의 "사이드체인 줄도 합산한다"는 그 사이 사실상 죽은
코드였고, 실측상 **세션 토큰의 약 2/3**(최근 7일 기준 input+output 68%, 캐시
66%)이 집계에서 통째로 빠져 있었다. 요약 바뿐 아니라 분석 패널도 같은 원천을
쓰므로 똑같이 1/3만 보였다(도구 수·작업시간 열은 PostToolUse가 `agent_id`가
있어도 통과하므로 원래 정상이었다).

그래서 Stop은 이제 **메인 전사 + `claude_subagent_transcripts`가 유도한 서브
전사들**을 함께 읽고 그 합을 싣는다(`ClaudeAdapter::turn_usage`).

- 유도 규칙은 경로에서 온다: `<dir>/<stem>.jsonl` → `<dir>/<stem>/subagents/*.jsonl`.
  깊이 2 서브에이전트(서브가 띄운 서브)도 같은 **평평한** 디렉터리에 들어간다(실측).
- **워터마크는 파일별**이다. 같은 맵(`transcript_usage_watermark`)에 서브 전사
  경로도 키로 들어가고, 각 파일이 자기 `message.id` 기준으로 델타만 낸다. 서브가
  메인 Stop 이후에도 계속 append하는 흔한 경로는 다음 Stop이 그 뒤를 이어 센다.
- 서브 전사 파일은 **모든 줄이 사이드체인**이라 프롬프트 경계가 아예 안 잡힌다 —
  덕분에 같은 스캔 함수(`claude_file_usage`)가 "워터마크가 없으면 (꼬리 상한 안)
  파일 전체, 있으면 그 뒤 전부"라는 원하는 동작을 그대로 낸다. 새 규칙을 따로
  만들지 않았다.
- **크기 기반 스킵**: 워터마크 엔트리에 마지막으로 본 파일 크기를 함께 들고,
  크기가 그대로면 스캔을 건너뛴다(전사는 append-only). 한 세션에 서브 전사가
  수십 개씩 쌓이고 대부분은 이미 끝나 있어서, 이게 없으면 매 Stop마다 수십 MB를
  다시 읽는다. 합산에 실패한 파일도 크기는 기록한다 — 안 그러면 매번 재시도한다.
- 대표 `model`은 여전히 메인 전사 것이다(서브 파일에서는 `!sidechain` 조건 때문에
  애초에 안 잡힌다).

**왜 "전 세션 누계의 델타"가 아닌가**: 전사에는 누계 필드가 없어 어차피 구간을
합산해야 하고, 이 방식은 상태를 들지 않아 앱 재시작·세션 입양 후에도 정확하다.
꼬리 2MB 안에서 경계를 못 찾으면 찾은 데까지만 합산한다(과소 집계로 강등).

**워터마크(이중 계산 방지) — 프롬프트 경계보다 우선한다**: `ClaudeAdapter`가
transcript_path별로 마지막 합산 시점의 가장 최근 `message.id`를
`transcript_usage_watermark: Mutex<HashMap<String, Arc<Mutex<Option<String>>>>>`에
들고 있다. `transcript_progress`(내레이션 스로틀용, Stop에서 지운다)와는
수명이 달라 별도 맵이다 — 같은 프롬프트 창 안에서 Stop이 두 번째로 와도
(백그라운드 서브에이전트가 남아 있던 `running>0` Stop 뒤 재호출 등) 그
경계를 넘어 남아 있어야 직전 Stop이 이미 합산한 몫을 다시 세지 않는다.

프롬프트 경계에서 그냥 멈추면 안 되는 이유가 따로 있다: 서브에이전트는
Stop 이후에도 사이드체인(`isSidechain:true`) 줄을 계속 append하고(대개 이 턴
비용의 대부분), 서브 완료 후 claude가 `task-notification`을 새 user 프롬프트로
주입한다. 그 주입 줄이 워터마크보다 먼저 걸려 버리면 그 사이드체인 몫이
통째로 새 나간다. 그래서 워터마크가 있을 때는 프롬프트 경계에서 **첫 진짜
사용자 프롬프트 지점까지의 합계를 스냅샷**만 해 두고 계속 스캔해 워터마크
`message.id`를 찾는다. 워터마크를 만나면 스냅샷은 버리고 그 지점까지의
**전체 합계**를 쓴다(정상 경로). 워터마크가 2MB 꼬리 밖으로 밀려나 끝내 못
찾으면 무제한 과대 집계 대신 **프롬프트 경계 스냅샷으로 강등**한다 — 이 경우
다음 Stop부터는 워터마크가 최근 id로 갱신돼 있으므로 다시 짧아진다.
워터마크가 아예 없으면(앱 재시작 직후 등) 종전과 동일하게 프롬프트 경계에서
멈춘다 — 이 경로는 동작 변화가 없다.

워터마크 비교는 **`message.id` 중복 제거(dedup) 삽입보다 먼저** 한다 —
꼬리부터 스캔하므로 같은 id의 더 새 줄이 먼저 나오는데, dedup을 먼저 태우면
그 줄이 카운트된 뒤에야 워터마크에 걸려 이중 계산이 되기 때문이다. 대표
`model`과 다음 워터마크 후보(`newest_id`, "뒤에서부터 처음 만난 것")는 이
스냅샷/강등과 무관하게 그대로 결정된다.

앱을 재시작하면 이 맵이 비므로, 재시작 후 첫 Stop이 잡는 턴 창이 실제보다
한 턴만큼 과대 집계될 수 있다 — 수용된 한계다. 맵은 세션이 끝나도 지워지지
않는 누수가 있지만 세션당 문자열 2개(경로+id) 규모라 무시할 만하다.

**동시성(TOCTOU)**: 훅 수신은 axum+tokio 멀티스레드고 forwarder에 1회 재시도가
있어 같은 Stop body가 겹쳐 들어올 수 있다. 조회(`get`)와 갱신(`insert`)을
따로 락을 잡으면 두 스레드가 같은 워터마크를 읽고 같은 구간을 두 번 합산하는
경합이 생긴다(막으려던 이중 계산이 되레 재발). 그래서 값 타입을
`Arc<Mutex<Option<String>>>`로 둬 전사 경로별 락을 얻은 뒤, 그 내부 락을
조회~전사 읽기~합산~갱신 내내 유지해 같은 경로에 대한 겹친 Stop을 직렬화한다.
바깥 맵 락은 그 경로별 락을 꺼내는 짧은 순간만 잡는다 — 전역 락을 IO 동안
잡으면 다른 세션의 Stop 훅(2초 타임아웃)까지 막힌다.

### 9.3 추출 — Codex

Codex 훅 body에는 사용량이 없다. rollout(`<CODEX_HOME>/sessions/YYYY/MM/DD/
rollout-*.jsonl`)의 `token_count` 이벤트가 `info.total_token_usage`(**세션 누계**)를
담고 있으므로 **턴 경계 두 지점의 누계 차이**를 쓴다(`observer/codex_usage.rs`).

- UserPromptSubmit에서 기준 누계를 기억한다. 한 턴에 프롬프트가 여러 번 와도(작업
  중 추가 지시) 기준을 밀지 않는다 — 그 사이에 쓴 토큰이 새기 때문.
- Stop에서 `현재 − 기준`을 싣고 현재 값을 새 기준으로 갱신한다. 기준이 없으면(앱이
  세션 도중에 켜짐) **그 턴은 조용히 생략**하고 기준만 심는다 — 세션 누계 전체를 한
  턴에 몰아넣는 과대 집계보다 누락이 낫다. **단, 신선한 세션은 예외다**:
  UserPromptSubmit 시점에 rollout을 찾았는데 꼬리에 `token_count`가 한 번도 없고,
  그 시점에 파일 전체가 꼬리 읽기 상한(`ROLLOUT_TAIL_BYTES` = 1MB) 안에 이미 다
  들어와 있으면(`metadata(path).len() <= ROLLOUT_TAIL_BYTES`) "누계가 진짜
  0"임이 증명된 것으로 보고 기준 0을 심는다 — 그래야 그 세션의 첫 턴이 통째로
  생략되지 않는다. 이 증명이 안 되는(꼬리 상한에 걸려 잘린) 파일은 기존대로
  생략한다 — 앞쪽 어딘가에 `token_count`가 있을 수 있는데 못 봤을 뿐인지,
  진짜 0인지 구분이 안 되기 때문이다.
- rollout 찾기는 훅 body의 `session_id`가 파일명 꼬리(`...-<id>.jsonl`)와 같다는
  규약을 먼저 쓰고, 없으면 `cwd`와 첫 줄 `session_meta.cwd`가 같은 최근 파일로
  폴백한다(`session_log/agent_transcript/codex.rs`와 같은 휴리스틱). 찾은 경로는
  세션별로 캐시한다.
- **서브스레드 rollout도 합산한다(2026-08-29 수정)**: Codex는 서브에이전트·
  guardian_review를 별도 스레드로 돌리고 각자의 rollout에 기록하는데,
  `total_token_usage`가 **스레드별 누계**라 메인 rollout만 보면 그 몫이 어디에도
  안 잡혔다(최근 10일 실측 메인 145M vs 서브 93M → **39% 누락**). 이제 Stop에서
  `parent_thread_id`가 이 세션의 스레드 id(`session_meta.payload.id`)에 닿는
  rollout들을 찾아(서브의 서브까지 부모 사슬을 따라간다) 파일별 기준값 델타를
  함께 더한다(`CodexUsageTracker::subthread_delta`). 후보는 "이미 기준을 심어 둔
  서브 + 턴 시작 이후 수정된 rollout"이고, 처음 보는 서브는 **그 스레드가 이 턴
  안에서 시작됐을 때만**(`session_meta.timestamp` ≥ 턴 시작) 누계 전체를 이 턴
  몫으로 싣는다 — 그보다 먼저 시작된 스레드는 메인 쪽 규칙과 같은 결로 기준만
  심고 생략한다. 두 시각의 출처가 달라(우리 시계 vs 파일시스템/Codex 기록) 경계에서
  엇갈리는 걸 막으려고 2초 여유를 준다. 첫 줄(`session_meta`)은 변하지 않으므로
  경로별로 영구 캐시한다.
- cwd 폴백(`find_by_cwd`)은 **부모 스레드가 있는 rollout을 제외**한다(메인 rollout을
  고르는 규칙이라 그대로다 — 합산 단계에서만 위와 같이 되불러온다). 처음엔
  `session_meta.payload.thread_source == "subagent"` 단일 비교였지만, 같은 cwd에
  `thread_source: "guardian_review"`이면서 `parent_thread_id`를 든 rollout이
  실측에서 확인됐다(서브 스레드인데 값이 "subagent"가 아니었다) — 지금은
  `payload.parent_thread_id`가 null이 아니면(어떤 thread_source든) 배제한다.
- 모델은 `turn_context.payload.model`. 꼬리에서 만나면 그 값을, 못 만나면 파일
  앞머리(256KB)에서 읽어 캐시해 둔 첫 모델을 쓴다.

**미대응 한계 — rollout 생성 레이스**: 새 세션의 rollout 파일이 아직 디스크에
안 나타난 극초반 구간에는 cwd 폴백이 옛 세션의 rollout을 잘못 고를 가능성이
이론상 있다. `session_meta`의 timestamp를 비교해 더 새 rollout만 고르는 보조
규칙을 검토했으나 구현하지 않았다 — 실사용에서 관측되면 그때 추가한다.

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

## 11. 확장 — 터미널 요약 바의 현재 세션 토큰·비용 (2026-08-28)

분석 패널(§9)은 열어야 보이고 기간(일) 단위다. 터미널을 열어 두고 일하는 동안
"지금 이 세션이 얼마나 먹었는지"를 실시간으로 보고 싶다는 요구가 따로 있어,
활성 탭 요약 바(`TerminalSummaryBar`, §4.4의 그 바가 아니라 이슈 #44 T1의 라벨
바) 오른쪽 끝에 같은 턴 단위 토큰·비용을 얹었다. **분석 패널과 데이터 원천은
같다**(§9.1의 `tokens` — 추출 로직도 동일) — 여기는 그것을 "지금 이 세션"으로
좁혀 실시간으로 보여줄 뿐, 새 추출·단가 로직을 만들지 않는다
(`renderer/analytics/pricing.ts` 그대로 재사용).

### 11.1 왜 알림에서 분리했나 — `turn-usage` 신설

처음엔 `NotificationEvent.tokens`를 그대로 wire에 실어 실시간 표시에 쓰는
쪽으로 갔었다(§9.1 도입 당시엔 백엔드 내부 운반 전용이라
`#[serde(skip_serializing)]`로 막혀 있었다). 하지만 알림 페이로드에 얹는
방식은 §9.1이 이미 갖고 있던 근본 문제를 그대로 물려받는다 — 알림이
dedup·hold·`running==0` 게이트로 억제되면 그 턴의 토큰도 함께 사라진다.
하필 실시간 표시가 가장 아쉬운 순간(백그라운드 서브에이전트가 남은 채 턴이
끝나는 흔한 경우)마다 이 게이트가 걸려, 터미널 요약 바의 토큰·비용이
대부분 세션에서 뜨지 않는 상태였다.

그래서 사용량을 알림에서 완전히 분리했다: `NotificationEvent.tokens` 필드는
**삭제**하고, 대신 독립 이벤트 `"turn-usage"`(`TurnUsageEvent`, Option이
아닌 `tokens` — 값이 있을 때만 방출한다는 계약을 타입으로 못박는다)를
신설해 hub가 알림 게이트·dedup보다 **먼저** 방출한다(§9.1). 실시간 표시는
더 이상 `NotificationEvent`를 거치지 않고 `tauriApi.onTurnUsage`를 구독해
`appStore.applyTurnUsage(e: TurnUsageEvent)`로 누계를 갱신한다.

이벤트를 새로 하나 추가하는 일이라 손대는 접점이 커맨드의 5접점(§4.2)과는
다르다 — 실제로 고친 7곳:

| # | 파일 | 역할 |
|---|---|---|
| 1 | `src-tauri/src/types.rs` | `TurnUsageEvent` 정의 |
| 2 | `src-tauri/src/state.rs` | `AppEvents::turn_usage` 기본 no-op / `TauriEvents` emit / `CompositeEvents` 전달 / 테스트용 `RecordingEvents` 수집 |
| 3 | `src/shared/ipc.ts` | `Events.turnUsage = "turn-usage"` |
| 4 | `src/shared/types/notification.ts` | TS `TurnUsageEvent` 미러 |
| 5 | `src/shared/types/api.ts` | `AgentOfficeApi.onTurnUsage` |
| 6 | `src/renderer/ipc/tauriApi.ts` | `onTurnUsage` 구현 |
| 7 | `src/renderer/ipc/sessionBridge.ts` | 구독 배선 |

`lib.rs`(커맨드 등록)와 `ipc/commands*`는 이번엔 손대지 않았다 — 이벤트는
커맨드가 아니라서 그 접점들이 아예 관여하지 않는다.

### 11.2 세션 경계와 리셋 규칙

누계는 **에이전트가 아니라 세션**에 붙는다 — 같은 에이전트라도 세션을 재시작하면
(재시작/재소환/resume) 이전 세션이 쓴 토큰은 새 세션의 누계가 아니다. 스토어는
`sessionUsage: Record<agentId, {sessionId, totals}>`로 "이 에이전트가 지금 붙어 있는
세션과 그 세션의 누계"만 들고, `noteUsageSession(agentId, sessionId)`가 세션이
바뀐 걸 감지하면 `totals`를 `emptyTotals()`로 리셋한다. 이 함수는 `session-state`
이벤트마다 불린다 — Stop이 아직 한 번도 안 온 새 세션에도 `sessionId`가 먼저
잡혀 있어야, 나중에 시드(§11.3)를 그 세션에 붙일 자리가 생기기 때문이다.

`bootstrap.ts`의 세션 핸드오프 입양 폴백(`adoptDetachedSessions`)도 같은 이유로
`noteUsageSession`을 부른다 — `session-state` 이벤트가 유실되거나 부팅 초반
경합으로 늦게 도착하면 `sessionUsage[agentId]`가 undefined로 남아, 시드에 그
세션의 누계가 멀쩡히 있어도 사용량 스팬이 아예 안 뜨는 문제가 있었다(코드
리뷰 C). `AdoptedSessionInfo`에 이미 `sessionId`가 실려 있어 추가 호출이
거저다 — 같은 sessionId로 두 번 불려도 `noteUsageSession`은 no-op이라
`session-state` 경로와 충돌하지 않는다.

### 11.3 시드 + 이중 계산 방지(3일 창의 한계)

방금 재시작한 앱은 실시간 사용량이 하나도 안 왔으니 누계가 0으로 보인다 — 그
세션이 어제부터 계속 진행 중이었어도. `useSessionUsageSeed` 훅이 **앱 수명당
1회**, `loadSessionEvents(cut - 3일, cut, ["stop", "usage"])`로 과거 세션
이벤트를 읽어 `aggregateSeed`(**tokens 존재** ∧ `at <= cut`만 세션별 합산 —
kind는 안 본다) 로 초기 누계를 만들고 `sessionUsageSeed = {at: cut, bySession}`에
한 번만 심는다(이미 있으면 no-op — 재시도하지 않는다는 뜻이기도 하다. 실패해도
조용히 삼키고 실시간 누계만 보여준다). `"stop"`·`"usage"` 둘 다 읽는 이유는
과거 파일이 tokens를 stop에, 신규 파일이 usage에 싣기 때문이다(§9.1) — kind로
걸러내면 한쪽 세대의 파일이 통째로 빠진다.

**이중 계산을 막는 경계(컷오프 `cut`)는 "훅이 도는 시각"이 아니라 "실시간이
실제로 처음 센 턴의 시각"이다.** 처음 버전은 시드를 부르는 시각(`Date.now()`)을
그대로 컷오프로 썼는데, 이건 **순서 의존**이었다 — 시드가 부르기 전에 실시간
사용량이 먼저 몇 턴을 반영해 버리면(예: A 수정으로 설정 하이드레이션을 기다리게
되면서 시딩이 늦게 시작되는 경우), 그 구간이 실시간 누계에도 들어가고 시드
조회 범위(`cut`이 그 이후 시각이므로)에도 다시 걸려 **토큰·비용·턴 수가 정확히
2배**로 집계됐다(코드 리뷰 B). 순서가 코드로 강제되지 않는 한 이 경합은 항상
재현 가능하다.

그래서 스토어에 `sessionUsageFirstAt: number | null`을 두고,
`applyTurnUsage`가 **실제로 누계에 반영한 첫 턴**의 `e.at`을 기록한다
(무시된 사용량은 기록하지 않는다 — 판정 자체가 바뀌진 않는다, 아래 참조). 훅은
`cut = firstAt !== null ? firstAt - 1 : Date.now()`로 컷오프를 잡는다 — 실시간이
이미 어떤 턴을 반영했으면 그 턴 바로 이전까지만 시드가 긁으므로, 시드가 아무리
늦게 도착해도 실시간과 겹치는 구간이 **구조적으로** 없다. 아직 실시간이 한
턴도 못 봤으면 지금 이 순간까지 긁는다 — 그 뒤에 오는 실시간 사용량은 이 시각보다
뒤에 온다(원래 경계와 동일한 안전성). `applyTurnUsage`의 판정 자체
(`e.at <= sessionUsageSeed.at`이면 버림)는 그대로다 — 늦게 도착한 과거 사용량을
거르는 방어는 여전히 필요하다.

화면에 낼 값은 `sessionUsage[activeId].totals`(cut 이후 실시간 누계)와
`sessionUsageSeed.bySession[그 sessionId]`(cut 이전 누계)를 `mergeTotals`로
더한 것이다.

**3일 창의 한계**: 세션이 3일보다 오래 열려 있었으면 그 이전 몫은 시드에서
빠져 화면 누계가 실제보다 적게 보인다. 받아들인 트레이드오프다 — `loadSessionEvents`
조회 범위를 무제한으로 늘리면 매 부팅마다 큰 JSONL을 읽어야 하고, 애초에 "지금
이 세션 누계"는 참고용 실시간 수치이지 정산 근거가 아니다(정확한 기간 집계는
분석 패널 몫).

### 11.4 해소된 문제와 남은 한계

**"억제된 Stop은 사용량도 버려진다"는 해소됐다.** 원래는 백그라운드
서브에이전트가 남은 채 턴이 끝나면 hub의 `running==0` 게이트가 알림을
억제하고, 그 턴의 stop 레코드 자체가 안 생겨 토큰이 영구 유실됐다(§9.1) —
그 턴의 `running==0` Stop은 영영 오지 않는다: 서브 완료 후 claude가
`task-notification`을 새 user 프롬프트로 주입해 새 턴을 열어 버리고, 그
프롬프트가 다음 사용량 스캔의 경계가 되기 때문이다(실전사로 확인). 해결은
사용량을 알림 경로에서 완전히 분리한 것이다(§11.1) — `turn-usage`는 hub의
게이트·dedup보다 먼저 방출되므로 억제된 Stop에서도 유실되지 않는다.

**"서브에이전트 몫이 통째로 빠진다"도 해소됐다(2026-08-29).** CLI가 서브에이전트
전사를 세션 파일 밖(`<session>/subagents/`)으로 옮긴 뒤로 Claude는 세션 토큰의
약 2/3, Codex는 약 39%가 집계에서 빠져 있었다. 지금은 Claude가 서브 전사들을,
Codex가 서브스레드 rollout들을 함께 합산한다(§9.2·§9.3). 요약 바와 분석 패널은
같은 원천을 쓰므로 함께 낫는다.

**잔존 한계**는 다음과 같이 재정의된다.

- **죽은/미지 세션**: hub가 `resolve_agent`로 agentId를 못 찾는 session_id의
  Stop은 usage도 알림도 둘 다 폐기된다 — 붙일 agentId 자체가 없다.
- **앱 재시작 후 첫 Stop의 워터마크 부재**(Claude, §9.2): 워터마크는 프로세스
  메모리에만 있어 앱을 재시작하면 사라진다. 재시작 후 첫 Stop이 잡는 턴 창이
  실제보다 한 턴만큼 과대 집계될 수 있다.
- **세션 마지막 서브에이전트의 꼬리**: 서브 전사는 메인 Stop 이후에도 계속
  자라고 그 몫은 **다음 Stop**이 이어 센다 — 그래서 세션의 마지막 턴 뒤에 더 이상
  Stop이 오지 않으면 그 구간은 집계되지 않는다.
- **Codex 장수 세션의 꼬리 상한**(§9.3): rollout이 꼬리 읽기 상한(1MB)보다
  커진 세션은 `token_count`를 못 찾아도 "누계가 진짜 0"이라는 증명이 안 되므로
  기준 0을 심지 않는다 — 신선한 세션과 달리 그 첫 턴은 여전히 생략될 수 있다.
- **죽은/미지 세션에서 usage가 폐기돼도 워터마크는 이미 전진해 있다**
  (Claude, §9.2): `map_hook`이 워터마크를 전진시키는 건 `ClaudeAdapter`가
  전사를 스캔한 부수효과일 뿐, hub가 그 `ObserverEvent::Stop`을 실제로
  `turn_usage`로 이어 붙이는 데 성공했는지와 무관하다. hub가 `resolve_agent`
  실패로 그 Stop을 버려도(위 첫 항목) 워터마크는 이미 다음 `message.id`로
  갱신된 뒤다 — 입양/재시작 직후 registry 등록 전에 훅이 먼저 도착하는 경로가
  실재해서, 그 구간의 토큰은 다음 Stop이 와도 워터마크 뒤라 다시 셀 방법이
  없이 영구 유실된다.
- **요약 바의 "턴 수" 의미가 바뀌었다**: 억제된 Stop도 이제 usage를 낸다
  (§11.1). 그래서 사용자가 한 번 지시한 요청이 백그라운드 서브에이전트를
  띄우면, 그 진행 중 Stop과 서브 완료 후 Stop이 각각 usage를 하나씩 내
  요약 바의 턴 수가 2로 잡힌다 — 예전의 "턴 = 억제되지 않은 완료 알림"과는
  다른 의미다. 다만 분석 패널의 `reconstructTurns`가 세는 턴 경계(프롬프트~
  다음 프롬프트/session_state)와는 오히려 더 가까워졌다.

### 11.5 설정 게이트 — 하이드레이션을 기다린다

`AppSettings.sessionCostEnabled`(기본 **true**, opt-out) — 이미 수집 중인 턴
토큰을 재사용할 뿐 추가 API 호출이 없어 기본 켜짐으로 잡았다(구독 한도 게이지
쪽 `usageFloatEnabled`와 같은 결). 꺼져 있으면 `useSessionUsageSeed`가 아예
`loadSessionEvents`를 부르지 않고(불필요한 파일 읽기 자체를 생략), 요약 바도
사용량 스팬을 렌더하지 않는다.

다만 "꺼져 있으면"을 판정하려면 **진짜 설정값이 스토어에 와 있어야** 한다.
`appStore`는 생성 시점에 `appSettings`를 `DEFAULT_APP_SETTINGS`(sessionCostEnabled
= true) 플레이스홀더로 채우고, `bootApp`이 `getAppSettings` IPC 왕복을 마친
뒤에야 `hydrateSettings`로 실제 값을 심는다. React의 첫 커밋에서 도는 effect는
그 IPC 왕복보다 먼저 실행되므로, `enabled` 값만 보고 판정하면 **설정을 꺼 놓은
사용자에게도 부팅마다 시드 조회가 나갔다**(코드 리뷰 A). 그래서 스토어에 런타임
전용 `settingsHydrated: boolean`(초기 false)을 두고 `hydrateSettings`가 이걸
true로 세운다 — `AppSettings`의 필드는 아니다(영속 대상도, 계약도 아니라서
Rust/픽스처 churn이 없다). `useSessionUsageSeed`는 `settingsHydrated &&
sessionCostEnabled`가 **처음 참이 되는 시점**에만 시도하고, 그 전에는 모듈
스코프 `attempted` 플래그도 세우지 않는다 — 하이드레이션이 끝내 실패해
`hydrateSettings`가 안 불리면(부팅 IPC 실패 등) 이 훅은 영영 시도하지 않는데,
시드 없이 실시간 누계만 쓰는 것이 그 경우의 의도된 강등이다.

### 11.6 표시 형식과 바 높이 불변식

형식은 `1.2M · $0.42` — 토큰은 `input+output`을 `formatTokens`로 축약(분석 패널과
같은 규칙), 캐시 읽기/쓰기는 셀에 안 넣고 `title` 툴팁에만(입력/출력/캐시읽기/
캐시쓰기/턴 수/대표 모델 + "공개 API 요율 기준 추정치" 각주). `costUnknownTurns > 0`
(§9.4처럼 단가를 모르는 모델이 섞였을 때)이면 비용 앞에 `~`를 붙여 부분 집계임을
드러낸다. 토큰이 실린 턴이 하나도 없으면(`turns === 0`) 스팬 자체를 렌더하지 않는다.

세 표시 강등 규칙(코드 리뷰 D, 분석 패널 `AnalyticsDialog`와 같은 결):
- **비용을 하나도 모를 때**(`costUnknownTurns === turns`) — `~$0.0000`처럼 백만
  토큰짜리 세션이 공짜로 보이는 대신 비용 자리를 `—`로 떨군다. 토큰 자리는
  그대로 의미 있게 보여준다.
- **일부만 모를 때**(`0 < costUnknownTurns < turns`) — `~` 접두는 그대로 두되,
  `title` 툴팁에 `summary.usage.costUnknownHint`(unknown 턴 수를 넣은 한 줄,
  activity 네임스페이스의 같은 키와 같은 톤)를 더해 `~`의 뜻을 남긴다. 이 키는
  `{ko,en,ja,fr,zh-Hans,zh-Hant}/terminal.json` 여섯 곳 모두에 있다.
- **캐시만 있는 턴**(`input + output === 0`인데 `cacheRead > 0`) — 토큰 자리를
  `0`이 아니라 `—`로 보여준다(그대로 두면 "쓴 토큰 없음"으로 오해).

(참고: `AnalyticsDialog`에도 "비용을 하나도 모를 때" 문제가 원래부터 있지만,
그 컴포넌트는 이번 §11 확장 범위 밖이라 손대지 않았다.)

`TerminalSummaryBar`는 **라벨이 없어도 바 자체는 자리를 지키는 불변식**이 있다
(높이가 바뀌면 xterm rows가 바뀌어 PTY resize가 나가고, pi 기본 TUI가 resize마다
스크롤백을 지운다 — 파일 헤더 주석 참조). 사용량 스팬을 더해도 바 높이(22px)는
그대로다: `margin-left: auto`로 오른쪽에 붙이는 `flex: none` 스팬 하나를 얹었을
뿐 바의 `height`는 손대지 않았다. 그리고 이 스팬은 라벨과 독립이라, 라벨이 없고
사용량만 있는 상태에서도 바가 (숨김이 아니라) 보인다 — "숨김"은 라벨도 사용량도
둘 다 없을 때만이다.

### 11.7 kind 필터와 부팅 크리티컬 패스 이탈

실측(최근 4일치 JSONL)으로는 15,480줄/약 4.5MB 중 시딩이 실제로 쓰는
`kind=stop ∧ tokens 존재` 줄은 156줄(1.0%)뿐이었다 — 나머지 99%는 렌더러가
받아서 바로 버리는 tool/notification/prompt 등이다. `load_session_events`
리더에 `kinds: Option<&[SessionEventKind]>` 필터를 더해 그 버림을 파싱 직후
Rust 쪽에서 끝낸다(`None`이면 현행 그대로 전부 — 분석 패널은 인자를 안 써서
동작이 그대로다). IPC 커맨드도 같은 모양의 옵셔널 `kinds`를 받고,
`loadSessionEvents(fromAt, toAt, kinds?)`가 5접점 계약
(`ipc/commands.rs`/`lib.rs`/`shared/ipc.ts`/`renderer/ipc/tauriApi.ts`/
`shared/types/api.ts`)을 그대로 통과한다. `useSessionUsageSeed`는 지금은
`["stop", "usage"]`를 넘긴다(§9.1의 kind 분리 이후 — 실측 당시엔 `kind=stop`뿐이었다).
(이 5접점은 **커맨드** 계약이다. **이벤트**를 추가할 때 손대는 접점은 다르다 —
`turn-usage` 신설이 실제로 고친 7곳은 §11.1 끝의 표 참고.)

시딩 자체도 부팅 크리티컬 패스에서 뺐다 — 게이트(§11.5)를 통과한 뒤
`setTimeout(..., 0)`으로 한 틱 미루고 언마운트 시 타이머를 정리한다. §11.3의
`firstAt` 기반 컷오프 덕에 이 지연이 안전하다: 시딩이 늦게 끝나서 그 사이
실시간 사용량이 먼저 몇 턴을 반영해도, 컷오프를 스토어에서 **실행 시점에 직접**
읽어(훅 클로저에 캡처해 둔 값이 아니라) `firstAt`을 최신값으로 잡으므로 이중
계산이 생기지 않는다.
