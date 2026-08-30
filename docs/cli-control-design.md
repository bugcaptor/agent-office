# CLI 제어 설계 (이슈 #55)

상태: 정본 — 구현 완료(이슈 #55 닫음, 2026-07-20 확인). 구현: `src-tauri/src/control/{mod,protocol,client}.rs`. 리팩터 후 렌더러 커맨드는 `ipc/commands/settings.rs`(`control_status`/`control_approve`/`control_revoke`)에 있다. 이후 외부 터미널 attach(`/v1/attach`·`/v1/detach`)가 추가됐다(§외부 터미널 attach).

실행 중인 Agent Office 인스턴스를 다른 AI/스크립트가 프로그래밍 방식으로
조종하기 위한 로컬 제어 인터페이스. `--observer-forward`/`--sessiond`와 동일한
"같은 바이너리의 인자 분기 + 로컬 IPC 서버" 패턴을 미러링한다.

## 목표와 범위

**포함(v1)**

- 앱 바이너리 서브커맨드 `agent-office ctl <명령>`(GUI를 띄우지 않는 단명
  클라이언트).
- 명령 표면: `status` `ping` `list` `create` `attach` `detach` `send` `dispose`
  `notifications` `clear` `settings get` `settings set`.
- 앱 안의 로컬 control 서버(axum, `127.0.0.1`, 임의 포트).
- **2단계 옵트인**: 설정 `cli_enabled`(기본 OFF)로 서버를 켜고, 앱에서 **명시적
  승인**(토큰 발급)이 있어야 명령이 실행된다.

**제외(후속)**

- 실시간 터미널 출력 스트리밍(`tail`) — 요청/응답만이다. (`attach`는 이름과
  달리 스트리밍이 아니라 **외부 터미널을 캐릭터에 붙이는** 명령이다 — 아래
  §외부 터미널 attach.)
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
| `/v1/attach` | `{ agentId, pid?, cwd?, tmux? }` → `{ sessionId, mode, script }` | `attach_external_with_profile`(tmux면 `create_with_profile`) |
| `/v1/detach` | `{ agentId }` → `{ detached }` | `detach_external` |
| `/v1/send` | `{ agentId, data }` | `write_input` |
| `/v1/dispose` | `{ agentId }` | `dispose_session` |
| `/v1/notifications` | `{ agentId }` | `list_notifications` |
| `/v1/clear` | `{ agentId, ids? }` | `clear_notifications` |
| `/v1/talk/roster` | `{}` → `RosterEntry[]` | 동료 대화(docs/agent-talk-design.md) |
| `/v1/talk/send` | `{ to, text, waitMs?, convId? }` | 〃 |
| `/v1/talk/reply` | `{ convId, text, waitMs? }` | 〃 |
| `/v1/talk/inbox` | `{ waitMs? }` | 〃 |
| `/v1/talk/end` | `{ convId, reason? }` | 〃 |
| `/v1/settings/get` | `{}` | `get_app_settings` |
| `/v1/settings/set` | `{ <설정필드>: <값>, … }` | `set_app_settings`(cliEnabled·talkEnabled 제외) |

## 외부 터미널 attach

앱 밖에서 시작한 터미널(iTerm, tmux pane 등)을 캐릭터에 붙인다 — 화면 미러링은
없고 **훅 알림 + 성격(persona) 주입**만 제공한다.

```sh
eval "$(agent-office ctl attach 캐릭터ID)"   # 붙이기
agent-office ctl detach 캐릭터ID              # 끊기(셸을 닫아도 자동 정리됨)
```

- **왜 eval인가**: 이미 떠 있는 셸의 env(`AGENT_OFFICE_SESSION` 등)와 `claude`
  래퍼 함수를 그 셸 안에 심어야 하기 때문이다. 자식 프로세스로는 부모 셸을
  바꿀 수 없다.
- **출력 계약**: 성공하면 **stdout에는 셸 스크립트 원문만** 나가고, 그 밖의
  안내·오류는 전부 stderr로 간다. 실패하면 stdout이 비어 `eval ""`이 되므로
  안전하다(종료 코드는 §종료 코드와 동일).
- **셸 지원**: zsh/bash 등 POSIX 셸 전용이다. fish는 문법이 달라 미지원
  (`export A=v`/함수 정의가 유효하지 않다).
- **observer OFF**: 훅 URL이 없으므로 알림은 오지 않고 성격만 적용된다. 이때
  스크립트 상단에 경고 코멘트가 붙는다(`# 경고: 관측이 비활성입니다 …`).
- **끊김 감지**: `ctl`이 자기 부모 프로세스 PID(= 그 터미널의 셸)를 함께
  보내고, 앱이 **5초 주기**로 `kill(pid, 0)` 스윕을 돌려 셸이 사라지면 자동으로
  detach한다(unix 전용). EXIT trap은 사용자의 기존 trap을 덮어쓸 위험이 있어
  쓰지 않는다.
- **1캐릭터 1세션**: 그 캐릭터의 앱 내 PTY 세션이 살아 있으면 새 논리 세션을
  만들지 않고 **그 sid에 합류**한다(`mode: "bind"`). 새로 발급하면
  `mode: "new"`다. 반대로 앱에서 그 캐릭터의 세션을 새로 만들면 붙어 있던 외부
  세션은 자동으로 끊긴다.
- **미존재 캐릭터**: `ok:false`로 거절한다(캐릭터 자동 생성은 범위 밖).
- **앱 재시작**: 논리 세션은 메모리에만 있으므로 사라진다 — 터미널에서 다시
  `eval "$(… attach …)"`을 실행해야 한다(영속화는 후속 과제).

## tmux 세션 attach(풀 기능)

일반 외부 터미널과 달리 tmux는 **출력 미러링 + 입력 주입까지** 된다. 방식은
"앱이 자기 PTY로 tmux 클라이언트를 하나 더 여는 것"이다 — 서버가 일반 세션을
`startup_command = exec tmux attach-session -t '<target>'`로 만들면 출력·입력·
resize·`on_exit`·세션 로그·봇 inject가 **기존 PTY 파이프라인 그대로** 동작한다
(pipe-pane + send-keys 대안은 가짜 SpawnedPty·이스케이프·크기 불일치 때문에
사실상 병렬 파이프라인 신설이라 기각했다).

```sh
agent-office ctl attach 캐릭터ID --tmux 세션이름   # 앱이 그 tmux에 붙는다
# 그 tmux의 각 pane 안에서(훅·성격 붙이기):
eval "$(agent-office ctl attach 캐릭터ID)"
```

- **검증**: 대상 이름이 비었거나 개행/제어문자를 포함하면 거절한다. 이어서
  `tmux has-session -t <target>`를 돌려 tmux 미설치(spawn 실패)와 세션 미존재를
  각각 다른 `ok:false` 메시지로 알린다. 앱이 GUI로 떠 PATH가 최소값이면 tmux를
  못 찾을 수 있다(부팅 시 로그인 셸 env 캡처가 PATH를 채워 준다, `env_capture`).
- **응답**: `mode: "tmux"`, `script`는 **코멘트 두 줄뿐**이다 — 출력 계약(성공 시
  stdout = eval 대상)을 깨지 않으면서 `eval`해도 요청한 셸은 그대로다. 두 번째
  줄이 pane 안에서 할 일(위 `eval`)을 안내한다.
- **pane의 훅·성격**: `exec tmux`로 셸을 갈아치우므로 클라이언트 셸의 env는
  pane에 전달되지 않는다(pane은 tmux **서버**에서 갈라진다). pane 안에서
  `eval "$(… ctl attach …)"`을 하면 그 캐릭터의 앱 내 PTY 세션(= tmux
  클라이언트)이 살아 있으므로 **BindExisting** — 같은 sid로 훅·성격이 합류한다.
- **`pid`는 쓰지 않는다**: 세션 수명은 tmux 클라이언트의 종료(`on_exit`)가
  결정한다. `ctl`을 부른 셸이 죽어도 무관하다.
- **이미 세션이 있으면 거절**: 1캐릭터 1세션이라 `create`는 살아 있는 세션을
  재사용한다 — 그러면 tmux 클라이언트가 뜨지 않으므로 성공으로 위장하지 않고
  `ok:false`로 알린다(탭을 닫거나 `ctl dispose <id>` 후 재시도).
- **창 크기**: 앱 탭이 tmux 클라이언트이므로 앱 창 크기가 tmux window 크기에
  영향을 준다(tmux 기본 `window-size latest` = 가장 최근 클라이언트 기준).
  다른 실제 터미널과 같은 세션을 함께 보면 작은 쪽에 맞춰지는 등 tmux의 평소
  규칙을 그대로 따른다.
- **dispose는 비파괴적**: 탭을 닫으면 tmux **클라이언트만** 죽는다. tmux 서버와
  세션·pane은 그대로 남아 다시 attach할 수 있다(`tmux kill-session`을 하면 그때
  클라이언트도 끝나 세션이 `Exited`가 된다).
- **"다시 띄우기" 주의**: `Exited` 후 탭의 재시작은 **프로필의**
  `startupCommand`를 기준으로 하므로 tmux 재접속이 아니다. 다시 붙이려면
  `ctl attach … --tmux …`를 한 번 더 실행한다(프로필에 저장하는 옵션은 후속 과제).
- **봇 inject**: `write_input`은 tmux 클라이언트의 stdin으로 가므로 **활성
  pane**에 입력된다. 봇 모드로 쓸 tmux 세션은 pane 하나만 두는 것을 권한다.
- **프로필의 tmux 자동 호스팅과는 별개**: 여기(`--tmux <target>`)는 사용자가
  이미 손으로 만들어 둔 임의의 tmux 세션에 즉석으로 붙는 용도라서 `target`은
  사용자가 손으로 치는 이름이고, 그래서 tmux 기본값인 접두/fnmatch 퍼지
  매칭(`attach_command`, `-t <target>`)을 그대로 쓴다. 반대로 프로필에서
  "tmux 호스팅"을 켜서 **앱이 직접 만드는** 세션은 이름이 전부 `ao-` 접두로
  시작한다는 것만이 사용자와 맺는 유일한 계약이고, 그 이름을 다루는 쪽
  (`tmux_host.rs`의 생성·gc·kill·attach)은 전부 `-t '=<name>'` 정확일치를
  쓴다 — 접두 매칭으로 `ao-nova-ab12cd`가 `ao-nova-ab12cdef`를 잘못 잡는
  사고를 막기 위해서다. 두 매칭 방식이 공존하는 건 대상 이름을 누가
  짓느냐가 다르기 때문이지 결함이 아니다. 상세는
  `docs/tmux-hosting-design.md`.

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
- `src-tauri/src/control/client.rs` — `ctl` 파서·발견·요청·출력(attach는 raw
  stdout).
- `src-tauri/src/control/tmux.rs` — `--tmux` 대상 검증, `exec tmux
  attach-session` 시작 명령 렌더, `tmux has-session` 확인기(테스트 주입 가능).
- `src-tauri/src/session/attach_script.rs` — attach 응답 스크립트 렌더러
  (`PreparedPlan` → export + `wrapper_script::render_posix`), tmux 모드의
  코멘트 전용 안내(`render_tmux_notice`).
- `src-tauri/src/session/external.rs` — 외부(논리) 세션 등록/해제·PID 스윕.
- `src-tauri/src/lib.rs` — `maybe_run_cli` 분기, setup에서 opt-in 기동, 종료
  훅 정리, 렌더러 커맨드 등록.
- `src-tauri/src/ipc/commands.rs` — `control_status`/`control_approve`/
  `control_revoke` 렌더러 커맨드, `apply_settings_effects`(GUI/CLI 공유).
- 렌더러 `settings/SettingsDialog.tsx` — CLI 제어 토글 + 2단계 승인 UI.
