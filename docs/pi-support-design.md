# Pi(pi.dev) CLI 작업 상태 감지 지원

상태: 정본 (2026-10-14) — 구현 완료(이슈 #8). 초기 설계·버전별 실측·완료 기록은 [archive/pi-support-design-snapshot.md](archive/pi-support-design-snapshot.md)에 보존한다. 아래는 현재 코드의 계약이다. 이번 H에서는 외부 Pi 버전이나 설치 앱 화면을 새로 검증하지 않았다.

## 구조와 구현 근거

| 파일 + 심볼 | 역할 |
|---|---|
| `src-tauri/src/session/pi_extension.rs` — `PI_EXTENSION_TS`, `write_extension`, `ensure_extension` | 정적 확장을 앱 데이터 `observer/pi/`에 배포. 앱 밖에서는 env 가드로 no-op |
| `src-tauri/src/session/manager.rs` — `prepare_session_plan` | 관찰 설정에 따라 `AGENT_OFFICE_PI_EXT`·훅 URL·세션 ID와 `CommandWrapperSpec` 준비 |
| `src-tauri/src/session/wrapper_script.rs` — `render_posix`, `render_powershell` | `pi()`/PowerShell 함수가 `-e` 확장 경로를 주입. 중복 인자 가드 |
| `src-tauri/src/observer/server.rs` — `HookQuery`, `handle_hook` | `/hook?session=<id>&agent=pi&source=<kind>`를 Pi 갈래로 라우팅 |
| `src-tauri/src/observer/mod.rs` — `ObserverRuntime::ingest_pi_source` | prompt/tool/stop을 `ObserverEvent`로 매핑. Claude/Codex 어댑터와 같은 hub에 전달 |
| `src-tauri/src/notification/hub.rs` — `NotificationHub::ingest_observer` | 알림 dedup·hold·activity 방출 |
| `src/renderer/timeline/turnReducer.ts` — `reduceTurn` | prompt로 턴 열기, tool로 작업 재개, stop으로 정산 |
| `src/renderer/store/appStore.ts` — `applyActivityEvent`, `pushNotification` | 턴 상태·라벨·알림 표시 |

관찰 opt-in은 `AppSettings.observerEnabled`(`src/shared/types/settings.ts`, Rust `persistence/settings_store.rs`의 `observer_enabled`)이다. 예전 `claude_hooks_enabled` 명칭은 사용하지 않는다. 앱 종료 시 v1 핸드오프를 거친 세션도 확장 POST의 포트 재발견 경로로 복구한다.

## 이벤트 계약

확장의 `PI_EXTENSION_TS` 안 `agentOffice`가 등록하는 이벤트가 정본이다.

| Pi 이벤트 | source / 의미 |
|---|---|
| `before_agent_start` | `prompt`: 사용자 prompt + `process.cwd()`, 턴 시작 |
| `tool_execution_start` | `tool`: `tool_name`·`tool_input`, 도구 시작 시 하트비트 |
| `message_end` | assistant 텍스트만 `tool`의 `assistant`로 전달, 5초 스로틀 |
| `agent_settled` | `stop`: 요청 전체 종료. `finishRun`의 `runOpen` 가드로 중복 방지 |
| `agent_end` | `agent_settled`를 본 적 없는 버전에만 1.5초 지연 완료 폴백 |
| `session_shutdown` | 열린 요청만 정산하는 안전망 |

초기 설계의 `agent_end` 즉시 완료·`tool_execution_end` 매핑은 이력이다. 한 요청 중 반복되는 `agent_end`를 완료로 처리하면 재시도·압축·큐 소진 때 턴을 일찍 닫는다. 현재 매핑은 이를 피한다.

## 실패·순서·제약

- POST는 Promise 체인으로 직렬화해 prompt와 tool의 수신 순서가 뒤집히는 것을 막는다. 타임스탬프는 백엔드가 부여한다.
- 요청 제한 시간은 2초. 연결 실패 시 `AGENT_OFFICE_APP_DATA/observer-port`를 읽고 루프백 HTTP URL의 포트만 바꿔 한 번 재시도한다. 성공한 포트를 다음 요청에도 사용한다.
- 전역 중복 등록 가드가 `-e` 중복 주입을 방어한다. 이벤트 등록·핸들러·통신 실패는 Pi 작업을 깨뜨리지 않는다.
- 권한 질문을 알려 주는 별도 hook source는 없다. Pi 관찰은 working/idle을 기본으로 하며 대기 상태를 추측하지 않는다(초기 결정 D1).
- 토큰 집계는 현재 `ingest_pi_source`의 tool/stop 모두 `tokens: None`이다(초기 결정 D2). 서브에이전트 훅은 주입하지 않는다(초기 결정 D3).
- WSL 관찰은 지원하지 않는다. 중첩 셸의 래퍼 보존과 Pi 외부 버전별 호환은 별도 실기 범위다.

## 검증 근거와 잔여

확장 배포·완료 판정·스로틀·포트 재시도 검사는 `src-tauri/src/session/pi_extension.rs`의 `tests`, Pi 수신 배선 검사는 `src-tauri/src/observer/server.rs`의 `tests`에 있다. 셸 래퍼 검사는 `src-tauri/src/session/wrapper_script.rs`의 `tests`에 있다. 과거 눈검증 완료 기록은 archive에 보존하며 현재 설치 Pi 버전의 재검증으로 간주하지 않는다.
