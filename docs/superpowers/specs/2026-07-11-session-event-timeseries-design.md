# 분석용 세션 이벤트 시계열 — 설계 스펙

- 날짜: 2026-07-11
- 상태: 승인됨 (구현 전)
- 브랜치: `feature/session-event-timeseries`
- 워크트리: `/private/tmp/agent-office-session-event-timeseries`

## 1. 배경과 목표

현재 `session-times.jsonl`은 완료된 턴마다 `totalMs`, `workedMs`, `waitedMs`를 한 번 기록한다. 장기 누적 합계에는 적합하지만 세션 경계, 작업·대기 전이, 이벤트 순서를 복원할 수 없다.

이 기능은 Rust 백엔드에서 캐릭터별 세션 원천 이벤트를 손실이 적은 시계열로 영속화한다. 이후 `jq`, Python, DuckDB 같은 외부 도구나 별도 분석 기능이 기간·캐릭터·세션별 지표와 상태 구간을 재구성할 수 있어야 한다.

## 2. 범위

### 목표

- 세션 수명주기와 Claude 훅 이벤트를 발생 순서대로 저장한다.
- `agentId`와 `sessionId`를 모두 저장해 캐릭터 및 개별 세션 단위 분석을 지원한다.
- 앱 실행과 렌더러 수명에 무관하게 로컬 디스크에 보존한다.
- 날짜별·버전별 JSONL로 사람이 직접 읽고 범용 도구로 처리할 수 있게 한다.
- 사용자 프롬프트 원문과 터미널 내용을 기록하지 않는다.

### 비목표

- 분석 UI, 차트, 집계 API 또는 CSV 내보내기.
- 기존 `session-times.jsonl`의 마이그레이션이나 제거.
- 토큰·비용·터미널 출력 분석.
- 로그 자동 삭제, 압축 또는 보존 기간 정책.
- 이미 저장된 과거 턴 요약에서 원천 이벤트를 역생성하는 작업.

## 3. 선택한 접근

`<app-data>/session-events/v1/` 아래에 UTC 날짜별 JSONL 파일을 둔다.

```text
session-events/
└── v1/
    ├── 2026-07-11.jsonl
    ├── 2026-07-12.jsonl
    └── ...
```

SQLite는 현재 수집 전용 범위에 비해 의존성·마이그레이션·잠금 설계 비용이 크다. 단일 JSONL은 장기 사용 시 파일 크기, 날짜별 처리와 손상 격리 문제가 있다. 날짜별 JSONL은 기존 프로젝트의 append-only 저장 패턴을 재사용하면서 장기 분석과 스키마 진화를 지원한다.

`v1` 디렉터리와 레코드의 `schemaVersion`을 함께 둔다. 호환되지 않는 변경은 `v2`에 기록하고, 호환 가능한 필드 추가는 기존 버전 안에서 선택 필드로 처리한다.

## 4. 이벤트 스키마

모든 줄은 독립적으로 파싱 가능한 JSON 객체이며 다음 공통 envelope를 가진다.

```json
{
  "schemaVersion": 1,
  "runId": "83cd95a1-b7c2-4c1e-9ef8-c9d5d1297ab0",
  "seq": 42,
  "at": 1783732299469,
  "agentId": "kfJ7r_Kub6Vg6uh0xowBl",
  "sessionId": "session-uuid",
  "kind": "tool"
}
```

| 필드 | 의미 |
|---|---|
| `schemaVersion` | 정수 스키마 버전. v1에서는 항상 `1`. |
| `runId` | 앱 프로세스 시작마다 생성하는 UUID. |
| `seq` | 해당 `runId` 안에서 1부터 증가하는 순번. |
| `at` | 백엔드가 부여한 Unix epoch milliseconds. |
| `agentId` | 캐릭터의 안정 ID. |
| `sessionId` | PTY/Claude 세션의 런타임 ID. |
| `kind` | 아래 이벤트 종류. |

`runId + seq`가 실행 내 유일 키다. 여러 앱 실행의 이벤트는 `at`으로 정렬하고, 같은 `runId` 안에서 같은 millisecond가 겹치면 `seq`로 순서를 확정한다. append 실패가 발생한 순번은 재사용하지 않아 로그의 `seq` 공백으로 유실 가능성을 관찰할 수 있게 한다.

### 4.1 이벤트 종류

| `kind` | 발생 시점 | 추가 필드 |
|---|---|---|
| `session_started` | 세션 ID와 실제 실행 컨텍스트가 확정된 직후 | `agentName`, `agentRole?`, `cwd?`, `shell` |
| `session_state` | 기존 `SessionStateEvent` 방출 직전 | `state`: `starting` \| `running` \| `exited` \| `disposed` |
| `prompt` | `UserPromptSubmit` 훅 수신 | 없음 |
| `tool` | `PostToolUse` 훅 수신 | 없음 |
| `notification` | Claude Notification 훅 또는 일반 대기 알림 수신 | 없음 |
| `bell` | 터미널 bell 수신 | 없음 |
| `stop` | Claude Stop 훅 수신 | 없음 |

`session_started`의 캐릭터 이름·역할은 세션 시작 당시 프로필 스냅샷이다. 프로필이 수정되거나 삭제돼도 과거 로그를 사람이 해석할 수 있게 한다. `cwd`는 실제 세션 작업 디렉터리, `shell`은 자동 선택까지 끝난 실제 실행 셸을 기록한다.

### 4.2 개인정보 경계

v1 직렬화 타입에는 다음 필드를 정의하지 않는다.

- 프롬프트 원문 또는 요약
- 알림 메시지와 dedup key
- 터미널 입출력
- 도구 이름, 입력 인자 또는 결과
- 환경 변수와 API 키

기존 `ActivityEvent.text`와 `NotificationEvent.message`는 렌더러 기능에는 계속 전달하지만, 시계열 레코드로 변환할 때 폐기한다. 문자열 필드를 사후 삭제하는 방식이 아니라 별도의 강타입 레코드를 직렬화해 실수로 민감 필드가 추가되는 것을 막는다.

## 5. 컴포넌트와 책임

### 5.1 `SessionEventStore`

Rust 영속화 컴포넌트다.

- 루트 경로: `<app-data>/session-events/v1`
- 앱 시작 시 `runId` 한 개 생성
- 내부 mutex 안에서 `seq` 할당과 append를 직렬화
- 이벤트 `at`의 UTC 날짜를 `YYYY-MM-DD.jsonl`로 변환
- 부모 디렉터리가 없으면 생성
- 레코드를 JSON 한 줄과 `\n`으로 직렬화해 append 모드로 한 번의 `write_all`
- 파일을 읽거나 집계하지 않음

mutex는 같은 프로세스의 여러 이벤트 생산 스레드가 줄을 섞거나 순번을 중복 배정하지 못하게 한다. 파일 핸들은 이벤트마다 열고 닫아 날짜 전환과 종료 처리를 단순하게 유지한다. 이벤트 빈도가 낮으므로 연결 유지나 비동기 배치는 도입하지 않는다.

### 5.2 이벤트 정규화 경계

기존 `AppEvents` 출력 경계 앞에 기록 계층을 둔다.

- `ActivityEvent`를 `prompt` 또는 `tool` 레코드로 변환
- `NotificationEvent.source`를 `notification`, `bell`, `stop`으로 변환
- `SessionStateEvent`를 `session_state`로 변환
- 저장을 먼저 시도하고 성공 여부와 무관하게 기존 `TauriEvents` 전달 수행
- output chunk와 notification-cleared는 기록 대상이 아니므로 그대로 통과

`session_started`는 실제 cwd와 shell을 아는 세션 생성 경로에서 별도로 기록한다. 해당 레코드를 위해 `SessionEventStore` 전체를 노출하지 않고, 필요한 강타입 입력만 받는 recorder 인터페이스를 세션 계층에 주입한다.

### 5.3 기존 시간 추적

렌더러 `turnReducer`, Zustand `timeTracking`, `SessionTimePanel`, `session-times.jsonl` 기록은 변경하지 않는다. 새 이벤트 로그는 분석용 원천 데이터이며 현재 UI의 누적값 계산 경로와 독립적이다.

## 6. 데이터 흐름

```text
세션 생성 ────────────────> session_started ──┐
SessionStateEvent ────────> session_state ────┤
ActivityEvent ────────────> prompt/tool ──────┼─> SessionEventStore
NotificationEvent ────────> notification/     │      │
                              bell/stop ──────┘      └─> v1/YYYY-MM-DD.jsonl

각 기존 이벤트 ── 저장 시도 ──> 기존 Tauri emit 계속 ──> renderer
```

1. 앱 setup이 app-data 경로로 store를 생성한다.
2. 세션 생성이 성공해 런타임 컨텍스트가 확정되면 `session_started`를 기록한다.
3. 이후 백엔드 이벤트가 발생할 때마다 정규화 레코드를 만든다.
4. store가 mutex를 잡고 다음 `seq`를 소비한다.
5. `at` 기준 UTC 파일을 선택해 한 줄을 append한다.
6. append 결과와 무관하게 기존 이벤트 전달과 세션 동작을 계속한다.

## 7. 오류 처리와 내구성

- 분석 로그 실패는 PTY 생성, 훅 처리, 알림 또는 렌더러 이벤트를 실패시키지 않는다.
- append 오류는 `eprintln!` 경고로 남긴다. 오류 메시지에는 대상 파일과 OS 오류를 포함하되 이벤트 내용은 출력하지 않는다. 일반 영속 앱 로그 신설은 이번 범위에 포함하지 않으며, 이후 정상 기록의 `seq` 공백도 누락 관측 수단이 된다.
- JSON 직렬화, 디렉터리 생성, 파일 open, write 실패 모두 동일한 비차단 정책을 적용한다.
- 비정상 종료로 마지막 줄이 부분 기록될 수 있다. 향후 reader는 빈 줄과 파싱 불가능한 줄만 건너뛰어야 한다.
- 자동 재시도와 메모리 큐는 도입하지 않는다. 디스크가 계속 실패할 때 무제한 메모리 증가나 세션 지연을 만들지 않기 위해서다.
- 로그 삭제와 보존은 사용자의 명시적 작업으로 남긴다.

## 8. 테스트 전략

### Store 단위 테스트

- 첫 append가 디렉터리와 UTC 날짜 파일을 생성한다.
- UTC 자정 전후 이벤트가 서로 다른 파일로 분리된다.
- 같은 run에서 `seq`가 1부터 증가하고 중복되지 않는다.
- 여러 스레드 동시 append 결과가 모두 완전한 JSON 줄이고 `(runId, seq)`가 유일하다.
- write 실패 후 다음 성공 레코드의 `seq`에 공백이 남는다.
- 생성된 각 줄을 v1 레코드로 역직렬화할 수 있다.

### 정규화·개인정보 테스트

- Activity의 `prompt`/`tool` 매핑.
- Notification의 `hook`/`bell`/`stop` 매핑.
- 모든 session state 매핑.
- `ActivityEvent.text`, `NotificationEvent.message`, 터미널 데이터가 직렬화 JSON에 존재하지 않는다.
- `session_started`에 당시 이름·역할·cwd·실제 shell이 들어간다.

### 통합·회귀 테스트

- 저장 성공과 실패 모두에서 기존 이벤트가 정확히 한 번 전달된다.
- 저장 실패가 세션 생성·상태 전이·훅 응답을 실패시키지 않는다.
- 기존 `session-times.jsonl`, turn reducer, SessionTimePanel 테스트가 변경 없이 통과한다.
- 전체 TypeScript와 Rust 테스트를 실행한다.

기준선에서 TypeScript는 839개 테스트가 통과한다. Rust는 샌드박스 밖 실행 기준 261개가 통과하고 `session::bash_wrapper::tests::is_bash_matches_bare_and_full_paths` 1건이 macOS의 Windows 역슬래시 경로 해석 차이로 실패한다. 이 기존 실패는 본 기능 범위 밖이며 구현 후에도 실패 수가 증가하지 않아야 한다.

## 9. 완료 조건

- 새 세션의 모든 대상 이벤트가 `<app-data>/session-events/v1/YYYY-MM-DD.jsonl`에 기록된다.
- 레코드만으로 앱 실행·캐릭터·세션·이벤트 종류·시간·실행 내 순서를 식별할 수 있다.
- 세션 시작 당시 프로필과 실행 컨텍스트를 확인할 수 있다.
- 승인된 민감 데이터가 JSONL에 포함되지 않는다.
- 저장소 오류가 기존 앱 기능을 중단시키지 않는다.
- 기존 요약 로그와 UI 동작이 유지된다.
- 새 테스트는 모두 통과하고 기존 기준선보다 실패가 늘지 않는다.
