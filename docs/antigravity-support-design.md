# Antigravity CLI(agy) 작업 상태 감지 지원 — 설계 문서

작성: 2026-09-06 (agy 1.1.27 실측 기준)
상태: 설계 확정, 구현 착수 전. 이슈는 kbm 너른바다/프로젝트관리/AgentOffice 참고.

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
- 실측한 전사 파일 위치: `~/.gemini/antigravity-cli/brain/<conversationId>/.system_generated/logs/transcript.jsonl`.
  한 줄이 한 스텝이고 `{"step_index","source","type","status","created_at","content"}` 꼴이다.
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
| `prompt` (턴 시작) | UserPromptSubmit | `PreInvocation` 이면서 `invocationNum == 1` | 프롬프트 원문이 없으므로 `transcriptPath` 의 마지막 `USER_INPUT` 에서 `<USER_REQUEST>` 안쪽만 잘라 `{"prompt": …, "cwd": workspacePaths[0]}` 로 보낸다 |
| `tool` (하트비트) | PostToolUse | `PostToolUse` (`matcher: "*"`) | `{"tool_name": name, "tool_input": {…}}`. Pi 와 같은 갈래 |
| `stop` (idle 정산) | Stop | `Stop` | `{"message": "Antigravity finished a task"}`. `terminationReason` 이 `error` 면 메시지를 바꿔 준다 |
| `hook` (waiting) | Notification | **v1 없음** | 아래 결정 D1 |
| `sub-start/stop` | SubagentStart/Stop | **v1 제외** | agy 에 서브에이전트 개념(`subagent_info`)은 있지만 훅 이벤트가 없다 |

`PreInvocation` 을 쓰는 이유: 턴 시작에 딱 한 번 오는 이벤트가 이것뿐이다.
`invocationNum` 이 2 이상이면 같은 턴 안의 반복 호출이므로 버린다.

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

1. 첫 줄에서 이벤트별 무해 출력을 stdout 에 찍는다.
   PreToolUse 는 안 걸지만 혹시 걸리면 `{"decision":"allow"}`, 나머지는 `{}`.
   Stop 은 `{"decision":""}`.
2. `AGENT_OFFICE_HOOK_URL` 또는 `AGENT_OFFICE_SESSION` 이 비어 있으면 종료.
3. stdin 을 읽고, 이벤트 이름(env `AGENT_OFFICE_AGY_EVENT` 로 hooks.json 에서 넘김)에 따라
   `source` 를 정한다. PreInvocation 은 `invocationNum` 이 1 일 때만.
4. `curl --connect-timeout 0.5 --max-time 1.5` 로 POST. 실패해도 exit 0.

전사 파일 읽기는 스크립트가 아니라 **서버 쪽에서** 한다. 스크립트는 페이로드를 그대로
넘기고, `ingest_agy_source` 가 `transcriptPath` 를 열어 마지막 USER_INPUT 을 뽑는다.
셸에서 JSON 을 파싱하는 짓은 하지 않는다.

Windows 는 v1 에서 뺀다. `cmd /c` 용 스크립트를 따로 써야 하고 지금 Windows 에서 agy 를
쓰는 사용자가 없다.

### 3.2 전역 hooks.json 병합 기록기

`session/agy_hooks_file.rs`.

- 대상: `~/.gemini/config/hooks.json`. 폴더가 없으면 만든다.
- 파일을 JSON 으로 읽어 최상위 키 `agent-office` 만 우리 것으로 바꿔 쓴다.
  **다른 키(orca-status 등)는 바이트 하나 건드리지 않는다.** 파싱에 실패하면 손대지 않고
  경고만 남긴다. 사용자 파일을 망가뜨리는 것보다 관찰을 포기하는 쪽이 낫다.
- 쓰는 내용:

```json
"agent-office": {
  "PreInvocation": [{"type":"command","command":"AGENT_OFFICE_AGY_EVENT=PreInvocation sh '<hook.sh>'","timeout":5}],
  "PostToolUse": [{"matcher":"*","hooks":[{"type":"command","command":"AGENT_OFFICE_AGY_EVENT=PostToolUse sh '<hook.sh>'","timeout":5}]}],
  "Stop": [{"type":"command","command":"AGENT_OFFICE_AGY_EVENT=Stop sh '<hook.sh>'","timeout":5}]
}
```

- 시점: 관찰 서버가 켜질 때(`hooks_on`) 한 번. 끌 때 지우지 않는다. env 가 없으면
  스크립트가 스스로 빠지므로 남아 있어도 무해하고, hook.sh 경로가 사라지면 sh 가
  파일 없음 오류를 내지만 timeout 5 초 안에 끝나고 stdout 이 비어 agy 는 기본 동작을 한다.
  이 마지막 항목은 스파이크 S2 에서 확인한다.
- 워크스페이스 `.agents/hooks.json` 은 쓰지 않는다. 저장소에 파일을 남기는 것은 사용자
  git 을 더럽힌다.

### 3.3 셸 래퍼

`agy()` 함수를 `CommandWrapperSpec` 으로 추가한다. 넘길 인자는 없다. 훅은 전역 파일이 다
하므로 래퍼가 할 일은 사실상 없지만, `skip_prefix_if_env_file_missing` 가드와 통일성을
위해 같은 틀에 둔다. env 로 `AGENT_OFFICE_AGY_HOOK` (hook.sh 경로)을 넣는다.

### 3.4 서버와 이벤트 매핑

- `observer/server.rs`: `query.agent == "agy"` 이면 `ingest_agy_source` 로 보낸다.
  Pi 갈래 바로 옆이다.
- `observer/mod.rs`: `ingest_agy_source(session_id, source, body)`.
  - `prompt`: `event::agy_prompt_text(body)` 가 `transcriptPath` 를 열어 마지막
    `USER_INPUT` 의 `<USER_REQUEST>` 안쪽을 돌려준다. 파일이 크면 뒤에서 64KB 만 읽는다.
    cwd 는 `workspacePaths[0]`.
  - `tool`: `{"name", …}` 을 `tool_name` / `tool_input` 으로 옮긴다.
  - `stop`: 메시지. `terminationReason == "error"` 면 "Antigravity stopped with an error".
- `ObserverProvider` 에 `Antigravity` 를 더한다. `as_str` 은 `"agy"`. UI 아이콘·세션 종류
  표시가 이 값을 쓴다.

### 3.5 이어하기

Claude 재개 설계(`docs/claude-session-resume-design.md`)를 그대로 따른다.

- 모든 이벤트에 `conversationId` 가 실려 온다. `ClaudeSessionSink::record` 와 같은
  sink 에 provider 를 넣어 기록한다. 스토어 파일은 provider 별로 나눈다.
- 재개 명령은 `agy --conversation <id>`. `--continue` 는 "가장 최근" 이라 우리 용도가 아니다.
- v1 은 기록까지만. 재개 UI 는 Claude 것을 provider 분기로 넓히는 후속 이슈로 뺀다.

### 3.6 사용량

이미 `usage/antigravity_live.rs` 가 `agy -p /usage` 로 실시간 조회를 한다. 세션 단위
토큰 집계는 전사 파일에 토큰 수가 없어(실측) 이번에는 하지 않는다. Pi 와 같은 `tokens: None`.

## 4. 스파이크 (구현 전 반나절)

- **S1** `PreInvocation` 의 `invocationNum` 이 사용자 턴마다 1 로 돌아오는지.
  agy 가 "execution" 과 "invocation" 을 나누고 있어(`executionNum` 이 Stop 에 따로 있다)
  turn 경계가 어느 쪽인지 확인해야 한다. 아니면 `PreInvocation` 대신 Stop 뒤 첫
  `PreInvocation` 을 턴 시작으로 삼는 상태를 서버가 들고 있어야 한다.
- **S2** hook.sh 가 없거나 exit 1 일 때 agy 가 어떻게 하는지. Stop 이 걸려 멈추지 못하면
  안 된다. stdout 이 비었을 때 "decision 없음 = 정상 종료" 인지 실측.
- **S3** 전사 파일이 PreInvocation 시점에 이미 사용자 입력을 담고 있는지.
  Claude 처럼 훅 뒤에 쓰인다면 D2 의 읽기 시점을 PostToolUse 첫 회로 미룬다.
- **S4** `agy -p` 프린트 모드에서도 훅이 도는지. 돈다면 사용량 조회(`/usage`) 가
  가짜 턴을 만들지 않게 `AGENT_OFFICE_HOOK_URL` 을 빼고 호출하도록 antigravity_live 를 고친다.

## 5. 하지 않는 것

- waiting 상태 (D1)
- 서브에이전트 표시
- Windows 훅 스크립트
- 재개 UI (기록만)
- 워크스페이스 hooks.json 기록

## 6. 검증

- 네이티브: hooks.json 병합이 다른 키를 보존하는 테스트, 깨진 JSON 을 건드리지 않는
  테스트, `ingest_agy_source` 매핑 테스트, 전사 꼬리에서 USER_REQUEST 추출 테스트.
- 눈검증: 앱에서 agy 세션을 열어 프롬프트 → 캐릭터 working, 도구 사용 중 하트비트,
  완료 → idle 과 라벨 갱신. 앱 밖 터미널에서 agy 를 띄워 훅이 조용히 빠지는지.
- `npx tsc --noEmit`, `npx vitest run --dir src`, `cargo test --manifest-path src-tauri/Cargo.toml`.
