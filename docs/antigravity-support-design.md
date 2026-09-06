# Antigravity CLI(agy) 작업 상태 감지 지원 — 설계 문서

작성: 2026-09-06 (agy 1.1.27 실측 기준)
상태: 구현 완료. 이슈는 kbm 너른바다/프로젝트관리/AgentOffice 참고.

> 구현 중 스파이크 실측으로 아래 본문과 달라진 점(§2/§3에 반영 완료):
> `invocationNum`은 1이 아니라 **0부터** 세고, 턴 시작은 `PreInvocation &&
> invocationNum == 0`이다. 전사 파일은 `~/.gemini/antigravity-cli/brain/...`가
> 아니라 페이로드의 `transcriptPath`를 그대로 열면 되고(실제 파일명은
> `transcript_full.jsonl`), PreInvocation 시점에 이미 그 턴의 사용자 입력이
> 쓰여 있다(S3 확정 — PostToolUse로 미룰 필요 없음). `agy -p`(print 모드)에서도
> 훅이 돌아 `usage/antigravity_live.rs`가 `agy`를 부를 때
> `AGENT_OFFICE_HOOK_URL`/`AGENT_OFFICE_SESSION`을 env_remove한다. cwd는
> `workspacePaths`가 빈 배열로 올 수 있어 `agy()` 셸 래퍼가 호출 시점의 `$PWD`를
> 실어 보내는 폴백이 필요했는데, 쿼리 문자열이 아니라 **헤더**
> (`X-Agent-Office-Cwd`)로 넘긴다 — POSIX sh에는 표준 percent-encoding 도구가
> 없어 쿼리에 실으면 공백·비ASCII 경로가 깨지기 때문이다.

## 0. 요약

Pi 때와 같은 길로 간다. **agy 는 Claude 와 거의 같은 셸 훅 체계를 갖고 있어서**,
훅 명령 하나가 기존 `127.0.0.1:<port>/hook?session=<id>&agent=agy&source=<kind>`
로 POST 하면 hub → turnReducer → UI 는 한 줄도 손대지 않아도 된다.

Pi 와 다른 점은 하나다. Pi 는 확장 파일을 우리 폴더에 두고 `-e` 로 넘겼지만,
agy 는 세션별로 훅 파일을 넘길 방법이 없다. 그래서 **사용자 전역 훅 파일에 우리
항목 하나를 병합해서 심는다.** 앱 밖에서 실행한 agy 에 영향이 없도록 훅 명령이
env 를 보고 스스로 빠진다.

이번 작업의 전부:

- 전역 `~/.gemini/config/hooks.json` 에 `agent-office` 키 병합 기록기
- 훅 명령이 실행할 셸 스크립트 1개 (app_data 에 정적 배포)
- 셸 래퍼 `agy()` 함수 (env 파일 없으면 강등)
- 서버의 `agent=agy` 갈래와 이벤트 매핑
- 대화 ID 기억해 두었다가 `agy --conversation <id>` 로 이어하기

## 1. agy 훅 체계 실측 (1.1.27)

바이너리에 들어 있는 훅 가이드(`docs/hooks.md`)와 `agy changelog` 를 읽어 확인했다.

### 1.1 파일 위치와 병합 규칙

- 전역: `~/.gemini/config/hooks.json`. 예전에는 `~/.gemini/antigravity-cli/hooks.json`
  에 쓰던 버그가 있었고 지금은 config 쪽이 맞다(changelog).
- 워크스페이스: `<workspace>/.agents/hooks.json`. 폴더를 신뢰한 뒤에 읽힌다.
- 플러그인: `plugins/<name>/hooks.json`.
- 최상위 키가 **훅 묶음 이름**이고, 같은 이벤트에 여러 묶음이 있으면 순서대로 모두 실행한다.
  묶음마다 `"enabled": false` 로 끌 수 있다.
- 이미 다른 도구(orca)가 `orca-status` 키로 같은 방식의 상태 훅을 심어 두었다.
  우리 방식이 통한다는 산 증거다.

### 1.2 이벤트 다섯 개

| 이벤트 | 언제 | 구조 |
|---|---|---|
| `PreInvocation` | 모델을 부르기 직전. 한 턴에 도구 호출 횟수만큼 반복 | 평면 배열 |
| `PostInvocation` | 도구 호출이 끝난 뒤 | 평면 배열 |
| `PreToolUse` | 도구 실행 직전. `matcher` 정규식으로 도구 이름 필터 | `{matcher, hooks:[…]}` 묶음 |
| `PostToolUse` | 도구 실행 직후. 실패하면 `error` 필드 | 묶음 |
| `Stop` | 실행 루프가 끝날 때 | 평면 배열 |

**프롬프트 제출 이벤트는 없다.** 바이너리에 "prompt hooks are not currently supported"
문자열이 박혀 있다. Claude 의 UserPromptSubmit, Notification 등가물이 둘 다 없다.

### 1.3 명령 실행 방식과 페이로드

- 명령은 `sh -c` 로 돈다(Windows 는 `cmd /c`). **작업 디렉터리가 hooks.json 이 있는
  폴더로 바뀐다.** 그래서 cwd 는 페이로드에서 읽어야 한다.
- 부모 프로세스의 env 를 그대로 물려받는다. `AGENT_OFFICE_SESSION`, `AGENT_OFFICE_HOOK_URL`
  을 훅 안에서 읽을 수 있다. Pi 확장이 `process.env` 를 읽던 것과 같은 자리다.
- stdin 으로 JSON 을 주고 stdout 으로 JSON 을 받는다. 키는 camelCase.
- 공통 필드: `conversationId`, `workspacePaths`(배열), `transcriptPath`,
  `artifactDirectoryPath`, `modelName`.
- 이벤트별 추가 필드:
  - PreToolUse: `name`, 도구 인자, `stepIdx`
  - PostToolUse: 위 + `error`(실패 시)
  - PreInvocation / PostInvocation: `invocationNum`, `initialNumSteps`
  - Stop: `executionNum`, `terminationReason`(`model_stop` / `max_steps_exceeded` / `error`), `error`, `fullyIdle`
- 전사 파일 경로는 페이로드의 `transcriptPath` 를 그대로 쓴다(실제로는
  `transcript_full.jsonl` 로 온다 — `~/.gemini/antigravity-cli/brain/...` 추측 경로가
  아니라 body가 주는 절대 경로가 정본이다). 한 줄이 한 스텝이고
  `{"step_index","source","type","status","created_at","content"}` 꼴이다.
  사용자 입력은 `type: "USER_INPUT"` 이고 `content` 안에 `<USER_REQUEST>…</USER_REQUEST>` 로
  싸여 있다. 뒤에 `<ADDITIONAL_METADATA>` 같은 시스템 덧붙임이 따라온다.

### 1.4 출력이 실행을 바꾼다

이게 Claude 훅과 결정적으로 다른 위험이다.

- PreToolUse 출력의 `decision` 은 **필수**다. `allow` / `deny` / `ask` / `force_ask`.
- Stop 출력의 `decision` 이 `continue` 면 **에이전트가 멈추지 못하고 루프를 다시 돈다.**
- PreInvocation / PostInvocation 의 `injectSteps` 는 대화에 메시지를 끼워 넣고,
  PostInvocation 의 `terminationBehavior: "force_continue"` 는 루프를 강제로 잇는다.

우리는 관찰만 하므로 **항상 무해한 출력을 먼저 찍고** 그다음에 POST 한다.

## 2. 이벤트 매핑

| 앱 source | Claude 훅 | agy 이벤트 | 비고 |
|---|---|---|---|
| `prompt` (턴 시작) | UserPromptSubmit | `PreInvocation` 이면서 `invocationNum == 0`(실측: 0부터 센다) | 프롬프트 원문이 없으므로 `transcriptPath` 의 마지막 `USER_INPUT` 에서 `<USER_REQUEST>` 안쪽만 잘라 `{"prompt": …, "cwd": workspacePaths[0]}` 로 보낸다. `workspacePaths` 가 빈 배열이면 `agy()` 래퍼가 실은 `X-Agent-Office-Cwd` 헤더로 강등 |
| `tool` (하트비트) | PostToolUse | `PostToolUse` (`matcher: "*"`) | `{"tool_name": name, "tool_input": {…}}`. Pi 와 같은 갈래 |
| `stop` (idle 정산) | Stop | `Stop` | `{"message": "Antigravity finished a task"}`. `terminationReason` 이 `error` 면 메시지를 바꿔 준다 |
| `hook` (waiting) | Notification | **v1 없음** | 아래 결정 D1 |
| `sub-start/stop` | SubagentStart/Stop | **v1 제외** | agy 에 서브에이전트 개념(`subagent_info`)은 있지만 훅 이벤트가 없다 |

`PreInvocation` 을 쓰는 이유: 턴 시작에 딱 한 번 오는 이벤트가 이것뿐이다.
`invocationNum` 은 0부터 세고(실측), 같은 턴 안에서 1, 2… 로 오르는 반복 호출이므로
0 이 아니면 버린다.

`PostInvocation` 을 안 쓰는 이유: PostToolUse 가 하트비트를 이미 주고, Stop 이 정산을
준다. 하나 더 쏘면 dedup 만 늘어난다.

`PreToolUse` 를 안 쓰는 이유: 출력이 실행을 게이트한다(1.4). 관찰용으로는 위험만 크고
얻는 게 없다. PostToolUse 하나면 충분하다.

> **결정 D1: v1 에서 agy 세션은 waiting 상태가 없다 (idle ↔ working 2상태).**
> Pi 와 같은 사정이다. 권한 질문을 알리는 이벤트가 없다. PreToolUse 에 `decision: "ask"`
> 를 돌려주면 강제로 물어보게 만들 수는 있지만, 그건 사용자의 권한 설정을 우리가
> 뒤엎는 일이라 하지 않는다. 훗날 agy 에 알림 이벤트가 생기면 source=hook POST 한 줄이다.

> **결정 D2: 턴 라벨은 전사 파일에서 읽는다.**
> 페이로드에 프롬프트가 없으니 대안은 둘이다. 전사 파일 꼬리를 읽거나, 라벨 없이 간다.
> 전사 파일은 페이로드가 절대 경로를 주고 형식이 단순해서 읽는 비용이 작다. 읽기에
> 실패하면 `prompt` 를 비운 채 보내 턴만 연다. 라벨 파이프라인은 빈 프롬프트를 이미
> 견딘다(Pi 에서 확인).

## 3. 구현 설계

### 3.1 훅 스크립트 (정적 배포)

`session/agy_hook.rs` 가 `<app_data>/observer/agy/hook.sh` 를 부팅 때 blind overwrite 한다.
Pi 확장 파일과 같은 방식이다. 하는 일:

1. 첫 줄에서 stdin 을 읽기도 전에 무해 출력 `{}` 를 stdout 에 찍는다(세 이벤트
   모두 동일 — Stop 의 `{"decision":""}` 은 `{}` 와 동등하게 취급되므로 굳이
   나누지 않는다. 실측 결과 훅 실패/부재에도 agy 는 정상 종료하므로(S2) 이
   출력은 순수히 "우리가 루프를 방해하지 않는다"는 방어용이다).
2. `AGENT_OFFICE_HOOK_URL` 또는 `AGENT_OFFICE_SESSION` 이 비어 있으면(=
   agent-office 밖) stdin 을 마저 읽지 않고 즉시 종료.
3. 이벤트 이름(env `AGENT_OFFICE_AGY_EVENT` 로 hooks.json 이 넘김)에 따라
   `source` 를 정한다: PreInvocation→prompt, PostToolUse→tool, Stop→stop.
   `invocationNum` 필터링(0 인지)은 셸이 아니라 **서버**가 한다 — 셸에서
   JSON 을 파싱하지 않는다는 원칙을 지키기 위해서다.
4. stdin 을 임시 파일(`mktemp`)에 담아 `curl --data-binary @<파일>` 로 스트리밍
   forwarding 한다(셸 변수에 body 전체를 담으면 ARG_MAX 근처 큰 전사에서
   위험하다). `curl --connect-timeout 0.5 --max-time 1.5` 로 POST. cwd 는
   `AGENT_OFFICE_AGY_CWD`(agy() 셸 래퍼가 호출 한 번에만 붙는 앞자리 대입으로
   실어 둔 env) 가 있으면 `X-Agent-Office-Cwd` 헤더로 함께 보낸다. curl 이
   "연결 실패"(종료 코드 7 — 스테일 포트, §핵심 5)로 끝나면
   `AGENT_OFFICE_APP_DATA/observer-port` 를 읽어(정수 검증 후) 그 포트로 1회
   재시도한다(forwarder.rs/pi 확장과 같은 계약). `AGENT_OFFICE_APP_DATA` 는
   session/manager.rs 가 세션 env 에 항상 심어 두므로 래퍼가 따로 넘길 필요는
   없다. 실패해도 exit 0.

전사 파일 읽기는 스크립트가 아니라 **서버 쪽에서** 한다. 스크립트는 페이로드를 그대로
넘기고, `ingest_agy_source` 가 `transcriptPath` 를 열어 마지막 USER_INPUT 을 뽑는다.
셸에서 JSON 을 파싱하는 짓은 하지 않는다.

Windows 는 v1 에서 뺀다. `cmd /c` 용 스크립트를 따로 써야 하고 지금 Windows 에서 agy 를
쓰는 사용자가 없다. 훅 파일 배포·hooks.json 병합은 `#[cfg(unix)]` 로 막았지만,
`agy()` 셸 래퍼 자체(PowerShell/Git Bash 포함 모든 플랫폼 렌더러)는 그대로 정의된다 —
prefix 인자가 없어 `command agy "$@"` 와 동등해 무해하다.

### 3.2 전역 hooks.json 병합 기록기

`session/agy_hooks_file.rs`.

- 대상: `~/.gemini/config/hooks.json`. 폴더가 없으면 만든다.
- 파일을 JSON 으로 읽어 최상위 키 `agent-office` 만 우리 것으로 바꿔 쓴다.
  **다른 키(orca-status 등)는 값을 보존한다(포맷·키 순서는 바뀔 수 있다)** —
  파싱 후 재직렬화(`serde_json::to_vec_pretty`) + tmp→rename 원자 쓰기라 값은
  그대로지만 들여쓰기·키 순서 같은 표현은 바뀔 수 있다. 파싱에 실패하면
  손대지 않고 경고만 남긴다. 사용자 파일을 망가뜨리는 것보다 관찰을 포기하는
  쪽이 낫다.
- 쓰는 내용:

```json
"agent-office": {
  "PreInvocation": [{"type":"command","command":"AGENT_OFFICE_AGY_EVENT=PreInvocation sh '<hook.sh>'","timeout":5}],
  "PostToolUse": [{"matcher":"*","hooks":[{"type":"command","command":"AGENT_OFFICE_AGY_EVENT=PostToolUse sh '<hook.sh>'","timeout":5}]}],
  "Stop": [{"type":"command","command":"AGENT_OFFICE_AGY_EVENT=Stop sh '<hook.sh>'","timeout":5}]
}
```

- 시점: 앱 부팅 때 한 번(`lib.rs`, pi 확장 파일과 같은 자리) — 관찰 토글과 무관하게
  항상 다시 쓴다. Pi 확장과 같은 이유: 훅 명령이 env(`AGENT_OFFICE_HOOK_URL`/
  `AGENT_OFFICE_SESSION`) 가드로 스스로 빠지므로 관찰이 꺼져 있어도 무해하다.
  끌 때 지우지 않는다. hook.sh 경로가 사라지면 sh 가 파일 없음 오류를 내지만
  timeout 5 초 안에 끝나고 stdout 이 비어 agy 는 기본 동작을 한다(S2 확인).
- 워크스페이스 `.agents/hooks.json` 은 쓰지 않는다. 저장소에 파일을 남기는 것은 사용자
  git 을 더럽힌다.

### 3.3 셸 래퍼

`agy()` 함수를 `CommandWrapperSpec` 으로 추가한다. prefix 인자는 없다 — 훅은 전역
hooks.json 이 다 하므로 래퍼가 명령줄에 더할 것이 없다. 유일한 역할은 §4 스파이크
실측(`workspacePaths` 가 빈 배열로 올 수 있음)의 cwd 폴백으로, 호출 한 번에만 붙는
앞자리 대입(`AGENT_OFFICE_AGY_CWD="$PWD" command agy "$@"`)으로 호출 시점의 `$PWD`
를 넘기는 것이다(`CommandWrapperSpec::export_cwd_env`, POSIX 렌더러만 지원). `export`
가 아니라 앞자리 대입을 쓰는 이유: `export` 는 함수가 끝난 뒤에도 같은 셸에서 도는
다른 명령에 값이 새는데, 앞자리 대입은 그 호출 하나의 환경에만 적용되고 사라진다.
hook.sh 는 이 값을 `X-Agent-Office-Cwd` 헤더로 서버에 얹는다.

### 3.4 서버와 이벤트 매핑

- `observer/server.rs`: `query.agent == "agy"` 이면 `ingest_agy_source` 로 보낸다.
  Pi 갈래 바로 옆이다. `X-Agent-Office-Cwd` 헤더를 읽어 cwd 폴백으로 넘긴다.
- `observer/mod.rs`: `ingest_agy_source(session_id, source, body, cwd_override)`.
  - `prompt`: `invocationNum == 0` 이 아니면 버린다. `event::agy_prompt_text(body)` 가
    `transcriptPath` 를 열어 마지막 `USER_INPUT` 의 `<USER_REQUEST>` 안쪽을 돌려준다.
    파일이 크면 뒤에서 64KB 만 읽는다. cwd 는 `workspacePaths[0]`, 없으면 `cwd_override`.
  - `tool`: top-level `name` 을 도구 이름으로, 흔한 인자 키(command/path/pattern/…)를
    detail 로 삼아 라벨을 만든다(agy 훅 가이드가 도구 인자 스키마를 명시하지 않아
    최선 추정 — 실제 필드명이 다르면 `event::agy_tool_activity_text` 만 고치면 된다).
  - `stop`: `terminationReason == "error"` 면 "Antigravity stopped with an error",
    아니면 "Antigravity finished a task"(전사에 완료 서술 텍스트가 없어 고정 문구).
- `ObserverProvider` 열거형에는 추가하지 **않았다** — Pi 도 이 열거형에 없다(전용
  어댑터가 없고 `agent=pi`/`agent=agy` 쿼리 갈래로 바이패스하는 구조라서). 설계
  원문의 §3.4 항목과 어긋나는 지점이니 기록해 둔다: 실제로 `provider=` 파라미터를
  쓰는 것은 어댑터(Claude/Codex)뿐이라 여기 추가해도 소비하는 곳이 없다.

### 3.5 이어하기

Claude 재개 설계(`docs/claude-session-resume-design.md`)를 그대로 따르되, 트레잇은
`ClaudeSessionSink` 를 그대로 재사용하지 않고 `AgySessionSink` 로 나눴다(provider 가
다르고 스토어 파일도 분리하므로 트레잇 이름에 Claude 가 들어가면 오해를 부른다).

- 모든 이벤트에 `conversationId` 가 실려 온다. `AgyResumeRecorder`(=
  `AgySessionSink` 구현)가 `AgyResumeStore`(`agy-resume.json`)에 기록만 한다 —
  `ClaudeResumeRecorder`/`ClaudeResumeStore` 와 같은 구조.
- 재개 명령은 `agy --conversation <id>`. `--continue` 는 "가장 최근" 이라 우리 용도가 아니다.
- v1 은 기록까지만. 재개 UI 는 Claude 것을 provider 분기로 넓히는 후속 이슈로 뺀다.

### 3.6 사용량 (kbm #2se, 2026-09-07 갱신)

이미 `usage/antigravity_live.rs` 가 `agy -p /usage` 로 한도(잔여 비율) 실시간 조회를
한다. S4 실측대로 `cli_command`/`fallback_cli_command` 가 자식 `agy` 에
`AGENT_OFFICE_HOOK_URL`/`AGENT_OFFICE_SESSION` 을 물려주지 않게 `env_remove` 한다 —
안 그러면 `/usage` 조회 한 번이 가짜 턴(prompt/stop)을 만든다. 이건 그대로다.

**턴 단위 토큰 집계는 처음엔 안 하는 것으로 남겼으나(전사 파일엔 토큰 수가 없다는
실측), 재실측으로 뒤집혔다.** 전사(`transcriptPath`, `transcript_full.jsonl`)에는
여전히 없지만, agy 가 대화마다 별도로 남기는 SQLite 파일에는 있다.

- **원천**: `~/.gemini/antigravity-cli/conversations/<conversationId>.db`.
  SQLite, **WAL 모드**다. 스키마는 `steps(idx integer PK, step_type integer,
  status integer, metadata blob, …)`.
- **오픈 순서(2단계 폴백, `observer/event.rs::agy_open_readonly`)**: 우선 순정
  `OpenFlags::SQLITE_OPEN_READ_ONLY`로 연다 — agy 가 대화를 열어 둔 채라면
  (`-wal`/`-shm` 사이드카가 있음) 이게 진행 중인 최신 WAL 프레임까지 읽는
  유일한 방법이다. sqlite 는 파일 접근을 지연시켜 `open` 자체는 사이드카가
  없어도 거의 항상 성공하고, 진짜 실패(`SQLITE_CANTOPEN`)는 첫 쿼리에서야
  드러난다(실측) — 그래서 가벼운 확인 쿼리로 실제로 열리는지 먼저 검증한다.
  실패하면 `file:<path>?immutable=1` URI 로 한 번 더 연다. agy 가 완전히
  종료해 체크포인트까지 끝난(사이드카가 없는) 상태에서 순정 오픈이
  `SQLITE_CANTOPEN` 으로 실패하는 게 실측으로 확인됐는데, 사이드카가 없다는
  것 자체가 "놓칠 WAL 프레임이 없다"는 뜻이라 이 경로에서는 immutable 로
  읽어도 최신 턴이 빠질 위험이 없다 — 그래서 순서를 read-only → immutable 로
  둬서 열려 있는 대화는 WAL 프레임을 읽고, 닫힌 대화도 읽히게 했다. 두
  시도 다 실패하면(파일 자체가 없음 등) 실패로 보고 조용히 넘어간다(§ 아래
  "실패 시" 참고).
- **행 판정**: `step_type = 15` 인 행이 모델 응답이다. `metadata` 컬럼이
  protobuf 블롭이고 `.proto` 정의는 없다(공개 스키마 없음) — 크레이트 추가 없이
  직접 만든 40줄짜리 최소 디코더(varint/length-delimited/64-bit/32-bit
  와이어타입만)로 읽는다(`observer/event.rs::pb_parse_fields`). 최상위 필드
  9(length-delimited)가 사용량 서브메시지이고, 그 안의 varint 필드
  **2=input_tokens, 3=output_tokens, 5=cache_read_tokens, 9=thinking_tokens**
  (6 은 있지만 무시)이다. 실측: `9.2=5571, 9.3=21, 9.5=8130, 9.9=19` 이며 `agy -p
  --output-format json` 의 usage(`total_tokens=5592=5571+21`, `cache_read`는
  별도 필드 8130)와 일치했다. 모델 스텝이라도 최상위 필드 9 자체가 없는 행(예:
  턴 끝의 빈 응답)이 있어 그런 행은 건너뛴다.
  - `input` = agy 가 이미 캐시를 제외하고 주는 순수 입력이므로 그대로 쓴다
    (`ClaudeAdapter`가 `input_tokens`를 캐시 제외로 정규화하는 것과 같은
    규약, §session-analytics-design.md §9.1).
  - `output` = `output_tokens + thinking_tokens`.
  - `cache_write`/`model` 은 db 쪽엔 없다 — `cache_write`는 항상 None, `model`
    은 훅 페이로드 top-level `modelName`(Stop 이벤트에서)을 그대로 쓴다.
- **워터마크(이중 계산 방지)**: `ClaudeAdapter::transcript_usage_watermark` 와
  같은 자리 — `ObserverRuntime` 이 `ao_session_id → 마지막으로 합산한 steps.idx`
  맵(`agy_usage_watermark: Mutex<HashMap<String, i64>>`)을 든다. Stop 마다
  `idx > watermark` 인 `step_type=15` 행을 합산하고, 새 워터마크로 그 구간의
  마지막 idx(사용량을 못 뽑은 행이어도 스캔했으면 포함, 재스캔 낭비 방지)를
  남긴다(`observer/event.rs::agy_turn_usage`).
  - **첫 프롬프트에서 워터마크를 미리 찍어 둔다.** `PreInvocation &&
    invocationNum == 0`(턴 시작) 시점에, 이 `ao_session_id` 로 아직 워터마크가
    없으면(앱 안에서 이 세션을 처음 본다) 그 순간 db 의 `max(idx)`
    (`agy_db_max_idx`)를 워터마크로 찍는다. 그래야 **리줌·입양된 대화**(db 에
    이미 이전 턴들의 행이 쌓여 있는 경우)의 첫 Stop 이 그 전체 히스토리를
    이번 턴 몫으로 잘못 합산하는 사고를 막는다. 브랜드 뉴 대화는 이 시점에
    db 가 아직 없거나 비어 있어 아무것도 안 찍히고, 그때는
    `agy_turn_usage` 의 "워터마크 없으면 전체 합산" 이 곧 이번 턴 전체와
    같으므로 그대로 옳다.
  - 워터마크 맵은 Claude 쪽과 마찬가지로 세션이 끝나도 지워지지 않는 누수가
    있지만 세션당 정수 하나 규모라 무시할 만하다.
- **실패 시 조용히 None**: db 부재, `steps` 테이블에 새 행 없음, 2단계 오픈
  폴백이 둘 다 실패하는 경우(파일 자체가 없는 등) 모두
  `agy_turn_usage`/`agy_db_max_idx`가 `None`을 돌려주고, 호출부는 기존
  워터마크를 그대로 둔 채 다음 Stop에서 재시도한다 — Claude 어댑터의 "합산
  실패는 조용한 폴백" 원칙과 같다.
- **의존성**: `rusqlite`를 추가했다. macOS/Linux는 시스템 `libsqlite3`(SDK에
  항상 있다)에 링크하고, Windows만 시스템 sqlite3 개발 라이브러리가 보통 없어
  `bundled` 피처로 정적 링크한다(`Cargo.toml`의 `[target.'cfg(windows)'.dependencies]`).
- **프런트 단가표**: `renderer/analytics/pricing.ts`에 agy 훅 `modelName`(예:
  `gemini-3.8-flash-medium`, `gemini-3.1-pro-preview`) 전용 요율을 추가했다.
  `modelName`에 마이너 버전·이펙트 티어가 섞여 고정 패턴으로는 pro/flash를
  다 못 잡으므로(패턴을 늘릴 때마다 새 마이너 버전 조합이 또 빠진다), 별도
  함수 `geminiThreeRateFor`가 `RATES` 순회보다 먼저 "id에 `gemini-3` 포함
  && `pro` 포함"이면 Pro 요율, 그 외 `gemini-3` 포함이면 Flash 요율로 가른다.
  출처·확인일은 그 파일 주석 참고.

## 4. 스파이크 (구현 전 반나절, 완료 — 실측 결과를 §2/§3 본문에 반영함)

- **S1** `PreInvocation` 의 `invocationNum` 이 사용자 턴마다 어떻게 도는지.
  **실측: 0부터 센다.** 턴 시작은 `PreInvocation && invocationNum == 0`, 같은 턴 안에서
  도구 호출마다 1, 2… 로 오른다. 원문 설계가 가정한 "1부터"는 틀렸다 — §2에 반영.
- **S2** hook.sh 가 없거나 exit 1 일 때 agy 가 어떻게 하는지. **실측: 훅 실패/부재에도
  agy 는 정상 종료한다.** Stop 이 걸려 멈추지 않는 문제 없음 — 그래도 방어적으로 무해
  출력은 유지한다(§3.1).
- **S3** 전사 파일이 PreInvocation 시점에 이미 사용자 입력을 담고 있는지.
  **실측: 그렇다.** PostToolUse 로 미룰 필요 없이 PreInvocation 즉시 읽어도 된다.
  전사 파일 경로도 페이로드의 `transcriptPath` 를 그대로 쓰면 되고(실측 파일명은
  `transcript_full.jsonl`), 파일이 크면 뒤 64KB 만 읽는다.
- **S4** `agy -p` 프린트 모드에서도 훅이 도는지. **실측: 돈다.** `usage/antigravity_live.rs`
  가 `agy` 를 Command 로 부를 때 `AGENT_OFFICE_HOOK_URL`/`AGENT_OFFICE_SESSION` 을
  env_remove 하도록 고쳤다(§3.6).
- **추가 실측(원 설계에 없던 것)**: `workspacePaths` 가 빈 배열로 올 수 있다 — 셸 래퍼
  `agy()` 가 `AGENT_OFFICE_AGY_CWD="$PWD"` 를 export 하고 hook.sh 가 이를
  `X-Agent-Office-Cwd` 헤더로 서버에 얹는 cwd 폴백을 추가했다(§3.2/§3.3).

## 5. 하지 않는 것

- waiting 상태 (D1)
- 서브에이전트 표시
- Windows 훅 스크립트
- 재개 UI (기록만)
- 워크스페이스 hooks.json 기록

## 6. 검증

- 네이티브(`session/agy_hooks_file.rs`, `session/agy_hook.rs`, `observer/event.rs`,
  `observer/mod.rs`, `observer/server.rs`, `session/wrapper_script.rs`,
  `usage/antigravity_live.rs`): hooks.json 병합이 다른 키(`orca-status`)를
  보존하는 테스트, 깨진 JSON·비객체 최상위를 건드리지 않는 테스트,
  `ingest_agy_source` 매핑 테스트(턴 시작 invocationNum==0 필터, cwd 폴백, tool/stop
  매핑, conversationId 캡처), 전사 꼬리에서 USER_REQUEST 추출 테스트, `agy()` 래퍼의
  `$PWD` export 순서 테스트, agy 자식 프로세스의 훅 env 제거 테스트. 완료.
- 눈검증(미실시 — 실제 agy 설치·워크스페이스 필요): 앱에서 agy 세션을 열어
  프롬프트 → 캐릭터 working, 도구 사용 중 하트비트, 완료 → idle 과 라벨 갱신.
  앱 밖 터미널에서 agy 를 띄워 훅이 조용히 빠지는지.
- `npx tsc --noEmit`, `npx vitest run --dir src`, `cargo test --manifest-path src-tauri/Cargo.toml`
  모두 통과 확인.
