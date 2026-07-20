# CLI 제어 설계 (이슈 #55)

상태: 정본 — 구현 완료(이슈 #55 닫음, 2026-07-20 확인). 구현: `src-tauri/src/control/{mod,protocol,client}.rs`. 리팩터 후 렌더러 커맨드는 `ipc/commands/settings.rs`(`control_status`/`control_approve`/`control_revoke`)에 있다.

실행 중인 Agent Office 인스턴스를 다른 AI/스크립트가 프로그래밍 방식으로
조종하기 위한 로컬 제어 인터페이스. `--observer-forward`/`--sessiond`와 동일한
"같은 바이너리의 인자 분기 + 로컬 IPC 서버" 패턴을 미러링한다.

## 목표와 범위

**포함(v1)**

- 앱 바이너리 서브커맨드 `agent-office ctl <명령>`(GUI를 띄우지 않는 단명
  클라이언트).
- 명령 표면: `status` `ping` `list` `create` `send` `dispose` `notifications`
  `clear` `settings get` `settings set`.
- 앱 안의 로컬 control 서버(axum, `127.0.0.1`, 임의 포트).
- **2단계 옵트인**: 설정 `cli_enabled`(기본 OFF)로 서버를 켜고, 앱에서 **명시적
  승인**(토큰 발급)이 있어야 명령이 실행된다.

**제외(후속)**

- 실시간 터미널 출력 스트리밍(`attach`/`tail`) — v1은 요청/응답만.
- 스프라이트/초상 생성, 프로필 전체 편집 패리티, GUI 전용 연출.
- 원격(네트워크) 접근.

## 통신 구조

```
[에이전트/스크립트]                         [실행 중인 agent-office GUI 앱]
  agent-office ctl send builder "npm test" --enter
        │  1. AGENT_OFFICE_APP_DATA(세션 자동 주입) 또는 OS app_data 경로 발견
        │  2. control-port / control-token 읽음
        ▼
  POST http://127.0.0.1:<port>/v1/send   ──►  axum control 서버(127.0.0.1)
  X-Agent-Office-Token: <token>               토큰 검증 → manager.write_input
        ◄── { "ok": true, "data": … }
```

- **`ctl`은 중복 앱 실행이 아니다.** `main.rs`가 `maybe_run_cli`로 인자를
  분기해 `run()`(Tauri GUI)에 도달하지 않는다 — 창·세션 매니저·두 번째 서버가
  뜨지 않는다. `ctl`은 포트/토큰을 읽어 **기존 GUI 앱**의 서버에 요청 1건을
  보내고 즉시 종료하는 얇은 클라이언트다.
- **앱이 떠 있어야 동작한다.** 서버를 소유하는 것은 GUI 앱이고, `ctl`은 그
  서버의 클라이언트다. 앱이 없거나 CLI 제어 OFF/미승인이면 `ctl`은 비영
  종료코드 + 명확한 에러로 즉시 실패하고, 절대 GUI를 대신 띄우지 않는다.
- **HTTP를 택한 이유**: 크로스플랫폼. sessiond의 UDS는 unix 전용이라 control은
  이미 의존성인 axum HTTP로 통일한다.

## 2단계 승인(보안 모델)

로컬 HTTP는 같은 사용자의 모든 로컬 프로세스·브라우저가 접근 가능하므로 반드시
인증이 필요하다.

1. **활성화(설정 `cli_enabled` ON)** — control 서버가 뜨고 `control-port`가
   기록된다. 하지만 토큰이 없으므로 **모든 요청이 401**이다.
2. **승인(앱에서 명시적 클릭)** — `control_approve` 커맨드가 128비트 랜덤 토큰을
   발급해 `control-token`(0600)에 기록한다. 이때부터 그 토큰을 제시하는 요청만
   인증된다. 승인은 지속되며(헤드리스/CI에서도 1회만), **승인 취소**
   (`control_revoke`)로 토큰을 폐기하면 이후 모든 요청이 다시 401이다.

서버는 **매 요청마다 `control-token` 파일 내용과 대조**하므로 승인(파일 생성)/
취소(파일 삭제)가 서버 재시작 없이 즉시 반영된다. 토큰 비교는 상수시간이다.

**추가 방어**

- `127.0.0.1`에만 바인딩(네트워크 노출 없음).
- 커스텀 헤더 `X-Agent-Office-Token` 필수 → 브라우저發 단순 폼 POST(토큰 없는
  CSRF 시도)를 프리플라이트 없이 차단(우리는 CORS 허용 헤더를 내보내지 않음).
- 모든 핸들러 `catch_unwind`(패닉이 요청을 매달지 않게, 기존 command와 동일).
- 서버가 없거나 CLI가 잘못 호출돼도 GUI 기능은 무영향(fail-open).
- `cli_enabled`는 **CLI로 바꿀 수 없다**(`settings set`에서 거부) — 자기 자신을
  켜고/끄는 권한 상승을 막는다. GUI에서만 토글한다.

## 와이어 프로토콜

- 요청: `POST http://127.0.0.1:<port>/v1/<command>`, 헤더
  `X-Agent-Office-Token: <token>`, JSON 본문(카멜케이스).
- 응답: `{ "ok": true, "data": … }` 또는 `{ "ok": false, "error": "…" }`.
- 토큰 불일치/누락: HTTP 401 + `{ "ok": false, "error": "unauthorized…" }`.

| 라우트 | 본문 | 대응 command |
| --- | --- | --- |
| `/v1/ping` | `{}` | (버전·세션 수) |
| `/v1/list` | `{}` | `load_state` + registry 스냅샷 |
| `/v1/create` | `{ agentId, cwd?, shell?, startupCommand?, name?, role?, cols?, rows? }` | `create_session` |
| `/v1/send` | `{ agentId, data }` | `write_input` |
| `/v1/dispose` | `{ agentId }` | `dispose_session` |
| `/v1/notifications` | `{ agentId }` | `list_notifications` |
| `/v1/clear` | `{ agentId, ids? }` | `clear_notifications` |
| `/v1/settings/get` | `{}` | `get_app_settings` |
| `/v1/settings/set` | `{ <설정필드>: <값>, … }` | `set_app_settings`(cliEnabled 제외) |

## 발견 순서와 오버라이드

1. `--app-data <경로>` / `--port <n>` / `--token <t>` 플래그(명시 최우선).
2. `AGENT_OFFICE_APP_DATA` env(세션 터미널엔 앱이 자동 주입).
3. OS별 표준 app_data 경로(`com.bugcaptor.agent-office`): macOS
   `~/Library/Application Support/…`, Linux `$XDG_DATA_HOME` 또는
   `~/.local/share/…`, Windows `%APPDATA%\…`.

`<app_data>/control-port`(현재 포트)와 `<app_data>/control-token`(승인 토큰,
0600)을 읽어 요청한다.

## 종료 코드

| 코드 | 의미 |
| --- | --- |
| 0 | 성공 |
| 1 | 서버가 `ok:false`로 거절(명령 오류) |
| 2 | 연결 실패(서버 없음/네트워크) |
| 3 | 포트 파일 없음(앱 미실행 또는 CLI 제어 OFF) |
| 4 | 토큰 없음(미승인) |
| 5 | 401(토큰 무효/취소됨) |
| 64 | 잘못된 사용법 |

## 다중 인스턴스

앱은 단일 인스턴스가 아니다. `control-port`는 last-writer 승 —
"마지막에 뜬 GUI 인스턴스를 제어"로 문서화한다(`ctl`을 몇 개 동시에 돌리든
무관). 인스턴스 선택은 후속 과제.

## 구현 맵

- `src-tauri/src/control/protocol.rs` — 와이어 타입·상수(`TOKEN_HEADER`,
  `control-port`/`control-token` 파일명).
- `src-tauri/src/control/mod.rs` — `ControlContext`(앱 상태 클론),
  `ControlServerState`(생명주기, ObserverServerState 미러), axum 라우터 +
  토큰 미들웨어 + 핸들러, 토큰/포트 파일 헬퍼(0600).
- `src-tauri/src/control/client.rs` — `ctl` 파서·발견·요청·출력.
- `src-tauri/src/lib.rs` — `maybe_run_cli` 분기, setup에서 opt-in 기동, 종료
  훅 정리, 렌더러 커맨드 등록.
- `src-tauri/src/ipc/commands.rs` — `control_status`/`control_approve`/
  `control_revoke` 렌더러 커맨드, `apply_settings_effects`(GUI/CLI 공유).
- 렌더러 `settings/SettingsDialog.tsx` — CLI 제어 토글 + 2단계 승인 UI.
