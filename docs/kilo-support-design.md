# Kilo Code CLI(kilo/kilocode) 작업 상태 감지 지원 — 설계 문서

작성: 2026-09-10 (kilo 7.6.0, OpenCode 포크 기준 스파이크 실측)
상태: 구현 완료. 이슈는 kbm 너른바다/프로젝트관리/AgentOffice 참고.

## 0. 요약

Pi·Antigravity 때와 같은 길을 갑니다. Kilo Code CLI 프로세스 안에 플러그인 하나를
심어서, 세션 이벤트를 기존 `127.0.0.1:<port>/hook?session=<id>&agent=kilo&source=<kind>`
엔드포인트로 POST합니다. `observer/server.rs` 아래(hub, turnReducer, appStore)는
한 줄도 건드리지 않습니다.

Pi와 다른 점은 주입 방식입니다. Pi는 `-e <경로>` 인자로 확장 파일을 직접
가리켰지만, Kilo는 인자로 플러그인을 못 넘기고 `KILO_CONFIG=<json 경로>` env로
설정 파일을 가리켜야 합니다. 그 설정 파일의 `plugin` 배열(`file://` URL)에 우리
플러그인을 넣어 두는 식입니다. 파일 두 개(플러그인 본체 .ts + 설정 .json) 모두
세션과 무관한 정적 내용이라, pi 확장처럼 앱 부팅·세션 준비 시점마다 그대로
덮어씁니다.

이번 작업의 전부:

- 플러그인 본체(.ts)와 그걸 가리키는 설정(.json) 두 파일을 app_data에 정적 배포
  (`session/kilo_plugin.rs`)
- 셸 래퍼 `kilo()`/`kilocode()` 함수 — 그 호출 한 번에만 `KILO_CONFIG` env를
  설정 경로로 채워 넣고, 원래 값으로 되돌립니다
- 서버의 `agent=kilo` 갈래와 이벤트 매핑(`observer/mod.rs::ingest_kilo_source`)
- 사용자가 이미 자기 `KILO_CONFIG`를 쓰고 있으면 관찰을 접고 원본 명령을
  그대로 실행하는 안전장치

## 1. 스파이크 실측(2026-09-10, kilo 7.6.0)

### 1.1 플러그인을 심는 방법

- env `KILO_CONFIG=<json 경로>`를 주면, 그 JSON의 `plugin` 배열에 적힌
  파일이 로드됩니다. 내용은 `{"plugin": ["file:///abs/agent-office-kilo.ts"]}`
  하나면 됩니다.
- 이 설정은 사용자 전역 설정(`~/.config/kilo/kilo.jsonc`)과 **병합**됩니다.
  `kilo debug config`로 직접 확인했습니다. 전역 설정에 있는 플러그인 목록에
  우리 항목이 얹히는 식이라, 사용자가 이미 쓰던 플러그인이 사라지거나 덮이지
  않습니다.
- 전역 파일 자체는 절대 건드리지 않습니다. 우리 쪽 값은 매번 이 세션 하나의
  호출에만 붙는 env로 전달합니다(§3).

### 1.2 플러그인 모듈 모양

실제로 로드해서 확인한 형태입니다.

```ts
export const AgentOffice = async (ctx: any) => ({
  event: async ({ event }: any) => { /* ... */ },
  "chat.message": async (input: any, output: any) => { /* ... */ },
  "tool.execute.before": async (input: any, output: any) => { /* ... */ },
});
```

`export default`도 같이 둬도 무방합니다(로더가 함수인지만 확인합니다).
`process.env`는 pi 확장 때와 마찬가지로 그대로 읽힙니다.

### 1.3 이벤트 페이로드

- `chat.message` — 루트 세션은 `input = { sessionID, model? }`, 자식(서브에이전트)
  세션은 `input = { sessionID, agent, messageID, model }`(`agent` 필드로
  자식임을 구분할 수 있습니다). 공통으로 `output = { message: {id, role,
  sessionID, ...}, parts: [{type:"text", text:"<프롬프트 원문>"}] }`.
- `tool.execute.before` — `input = { tool: "read"|"write"|"bash"|"task"|...,
  sessionID, callID }`, `output = { args: {...} }`. args 키는 도구별로
  다릅니다(§2 표).
- `tool.execute.after` — `input = { tool, sessionID, callID, args }`,
  `output = { title }`. 우리 플러그인에는 넣지 않았습니다(§6).
- `session.created` — `properties = { sessionID, info: { id, parentID?,
  title } }`. `parentID`가 있으면 task 도구가 만든 자식(서브에이전트)
  세션입니다.
- `session.status` — `properties.status.type`이 `"busy"|"retry"|"idle"`로
  옵니다. 한 턴 안에서 `busy`가 여러 번 반복되므로, 완료 판정에는 쓰지
  않습니다.
- `session.idle` — `properties = { sessionID }`. **정상 종료면 한 세션당
  정확히 한 번**옵니다. 하지만 오류나 중단으로 끝난 턴에서는 **같은
  세션에 두 번** 옵니다 — 실측으로는 `SessionProcessor.halt`와 러너의
  `onIdle`이 각각 idle 상태를 세팅하면서 둘 다 이벤트를 냅니다. 그래서
  "idle이 오면 곧 끝"이라고만 보지 않고, 루트 세션 ID와 진행 중 표시
  (`runOpen`)로 걸러야 합니다(§4). 또한 **자식 세션의 idle이 부모 세션의
  idle보다 먼저 옵니다** — 자식이 끝나야 부모가 다음 단계로 넘어가니
  당연한 순서지만, 회계 코드가 이 순서를 가정하고 있다는 점은 적어
  둡니다(§4).
- `session.error` — `properties = { sessionID, ... }`. 루트 세션이 오류로
  끝나면 옵니다(§4).
- `permission.asked` — `properties = { id, sessionID, permission: "bash"|...,
  patterns: ["echo x"], metadata, always, tool: {messageID, callID} }`.
  (`permission.ask` 훅 자체는 발화하지 않습니다. 이벤트로만 받을 수 있습니다.)
- `permission.replied` — `properties = { sessionID, requestID, reply }`.

`session.turn.open`/`session.turn.close` 이벤트는 kilo 바이너리 안에 정의는
돼 있지만, 스파이크 3회 모두 플러그인의 `event` 훅으로 전달되지 않았습니다.
턴 경계를 이 이벤트로 잡는 방법으로 갈아탈까 고민했지만, 실려 오지 않는
이벤트를 기준으로 설계할 수는 없어서 `chat.message`/`session.idle` 조합을
그대로 씁니다.

## 2. 이벤트 → source 매핑

`observer/mod.rs::ingest_kilo_source`가 받는 source 문자열과 그 의미입니다.
플러그인이 훅 이벤트를 이 source들로 번역해 POST합니다.

| source | 언제 | body | 만드는 ObserverEvent |
| --- | --- | --- | --- |
| `prompt` | 루트 세션의 `chat.message` | `{prompt, cwd}` | `Prompt` |
| `tool` | `tool.execute.before`, 자식 세션의 `chat.message`(하트비트용), `permission.replied` | `{tool_name, tool_input}` 또는 빈 객체 | `Tool` |
| `hook` | `permission.asked` | `{message}` | `Attention`(권한 알림) |
| `sub-start` | `session.created`(parentID 있음) | `{}` | `SubStart` |
| `sub-stop` | 자식 세션의 `session.idle` | `{}` | `SubStop` |
| `stop` | 루트 세션 ID와 일치하고 `runOpen`인 `session.idle` | `{message, running}` | `Stop` |

`permission.replied`를 `tool`로 보내는 이유는 완료 알림 억제가 아니라
반대쪽입니다 — hub는 권한을 묻는 `hook`(Attention) 알림을 세션이 계속
일한다는 신호(프롬프트 제출·도구 사용 등)가 오면 조용히 폐기하는데
(`notification/hub.rs`의 hold, 이슈 #41), `Tool` 활동도 그 신호 중
하나입니다. 권한 응답을 받았는데도 그 hold가 안 풀리면 사용자가 승인/거부한
뒤에도 "권한이 필요합니다" 알림이 화면에 남아 있을 수 있어, `permission.replied`
시점에 `Tool` 활동 하나를 흘려보내 그 알림을 정리합니다.

도구 인자 키는 OpenCode 표준을 따릅니다. `kilo_tool_activity_detail`
(`observer/event.rs`)이 도구 이름별로 이렇게 읽습니다.

| tool | 인자 키 | 라벨에 쓰는 값 |
| --- | --- | --- |
| `bash` | `command` | 첫 줄 |
| `read`/`write`/`edit` | `filePath` | 파일 이름만(경로 마지막 조각) |
| `list` | `path` | 경로 그대로 |
| `glob`/`grep` | `pattern` | 그대로 |
| `webfetch` | `url` | 그대로 |
| `task` | `description` | 그대로 |

`list`/`glob`/`grep`/`webfetch`는 스파이크에서 직접 호출을 못 잡아서
OpenCode 표준 필드명으로 추정했습니다(§6). `bash`/`read`/`write`/`edit`/
`task`는 실제 호출로 확인했습니다.

자식 세션 판정은 두 가지 중 하나입니다. `childSessions`에 등록된
`sessionID`이거나, `input.agent`가 있으면(자식 세션에만 실리는 필드)
자식으로 봅니다. 자식의 `chat.message`는 완료 판정에 영향을 주지 않는
단순 하트비트로만 쓰고 `tool`로 보냅니다 — 자식이 프롬프트를 새로 받았다고
루트 턴이 끝난 걸로 착각하면 안 되기 때문입니다.

`stop`의 `running`은 아직 도는 자식 세션 수입니다. hub는 `running > 0`이면
완료 알림을 억제합니다(`notification/hub.rs`, "백그라운드 서브에이전트가
아직 도는 중의 Stop은 완료가 아니다" 원칙 — pi/agy와 동일).

## 3. KILO_CONFIG 배선과 전역 설정 불간섭

Kilo는 pi처럼 인자로 확장을 넘길 방법이 없어서 env를 씁니다. 문제는
사용자가 이미 자기 `KILO_CONFIG`를 쓰고 있을 수 있다는 겁니다 — 그걸 우리
값으로 덮어쓰면 사용자 플러그인 설정이 깨집니다.

그래서 `CommandWrapperSpec`(`observer/event.rs`)에 필드 두 개를 추가했습니다.

- `set_env_from_env: Vec<(String, String)>` — `(대상 env, 값을 가져올 env)`.
  kilo는 `[("KILO_CONFIG", "AGENT_OFFICE_KILO_CONFIG")]`를 씁니다. 렌더된
  래퍼는 원본 명령을 실행하는 **그 한 번의 호출에만** 대상 env를 세팅합니다.
  POSIX는 앞자리 대입(`KILO_CONFIG="${AGENT_OFFICE_KILO_CONFIG}" command
  kilo "$@"`)으로, PowerShell은 호출 전에 세팅했다가 `try`/`finally`로
  원래 값을 복원하는 식으로 구현했습니다. 둘 다 그 호출이 끝나면 값이
  원래대로 돌아옵니다 — `export`처럼 셸에 값이 눌러앉지 않습니다.
- `skip_if_env_set: Option<String>` — 그 env가 호출 시점에 이미 값을 갖고
  있으면 관찰을 접고 원본 명령을 그대로 실행합니다. kilo는 `KILO_CONFIG`를
  이 이름으로 씁니다. 사용자 설정과 충돌하면 관찰 하나를 포기하는 쪽을
  택한 겁니다 — 반대로 우리 값으로 덮어쓰면 사용자가 쓰던 플러그인이
  통째로 안 뜨는 더 큰 사고가 됩니다.

기존 `skip_prefix_if_env_file_missing`(이슈 #40, claude/pi가 쓰던 파일-부재
강등)도 그대로 겁니다. 설정 파일이 사라졌으면(app_data 없는 구성이 OS
temp를 쓰다 정리된 경우 등) env 대입 없이 원본 명령을 실행합니다.

렌더러 쪽 회귀는 하나 있었습니다. 기존 `skip_prefix_if_env_file_missing`
가드는 "prefix가 있을 때만" 렌더링됐는데, kilo 래퍼는 `prefix_args`가
비어 있고 `set_env_from_env`만 있습니다. 그래서 가드 조건을
`!prefix.is_empty() || !set_env_from_env.is_empty()`로 완화했습니다.
강등 경로(파일 부재 폴백, `skip_if_present` 조기 반환, `skip_if_env_set`
조기 반환)에는 어느 경우에도 env 대입을 붙이지 않습니다 — 강등이란 곧
관찰을 접는다는 뜻이니까요.

## 4. 자식 세션 회계와 턴 정산

Kilo의 task 도구가 서브에이전트를 만들면 `session.created`에 `parentID`가
실립니다. 플러그인은 그 세션을 `childSessions`에 넣고 `activeChildren`을
올리면서 `sub-start`를 보냅니다. 그 자식이 끝나면(`session.idle`,
`sessionID`가 `childSessions`에 있음) `activeChildren`을 내리고(0 미만으로는
안 내려갑니다) `sub-stop`을 보냅니다.

**루트 세션 판정은 소거법(자식이 아니면 루트)을 쓰지 않습니다.** `chat.message`
루트 분기에서 `rootSessionID`에 그 세션 ID를 명시적으로 기록해 두고,
`session.idle`이 오면 `sessionID === rootSessionID`로 정확히 비교합니다.
이렇게 한 이유는 §1.3의 실측 때문입니다 — 오류·중단으로 끝난 턴은 같은
세션에 `session.idle`이 **두 번** 옵니다. 소거법으로 "자식이 아니니 루트"라고
판정하면 이 두 번째 idle을 다른 세션의 이벤트와 헷갈릴 여지가 생깁니다.
명시적 ID 비교 + `runOpen`(사용자 요청 1건이 진행 중인가) 가드를 쓰면 훨씬
단순합니다 — 첫 번째 idle에서 `runOpen`을 끄고 `stop`을 보내면, 두 번째
idle은 `runOpen`이 이미 꺼져 있어 그냥 무시됩니다.

새 루트 프롬프트가 들어오면(`chat.message` 루트 분기) `childSessions`와
`activeChildren`, `lastError`를 전부 리셋합니다. Kilo의 task 도구는 부모가
자식을 `await`하므로, 새 요청이 시작되는 시점에 이전 요청의 자식 세션이
남아 있을 이유가 없습니다 — 남아 있다면 이전 턴에서 `sub-stop`을 놓친
회계 버그이므로, 여기서 강제로 0에서 다시 셉니다.

턴이 오류나 중단으로 끝나면 완료 메시지를 다르게 보냅니다. `session.error`가
루트 세션에 오면 `lastError`를 세우고, 그 뒤 루트 `session.idle`에서 stop을
보낼 때 `lastError`가 켜져 있으면 "Kilo stopped with an error"를, 아니면
"Kilo finished a task"를 씁니다. `lastError`도 새 루트 프롬프트 시작 시
리셋됩니다.

**자식의 idle이 부모보다 먼저 온다**는 실측(§1.3)이 회계 순서를 정당화합니다.
자식 정리가 부모 판정보다 먼저 일어나므로, 부모의 `stop`이 나갈 때
`activeChildren`은 이미 그 자식을 뺀 값입니다. 다만 형제 자식이 여럿이면
부모가 idle이 되는 시점에도 다른 자식이 아직 돌고 있을 수 있어, `stop`의
`running`이 0보다 클 수 있습니다 — 그 경우 hub가 완료 알림을 억제하는 게
정확한 동작입니다(§2).

## 5. 그 외 배선

- **스테일 포트 재시도** — 옵저버 서버는 앱을 켤 때마다 포트 0으로
  새로 잡히므로, 앱을 껐다 켜면 입양된 세션이 들고 있는
  `AGENT_OFFICE_HOOK_URL`은 죽은 포트를 가리킵니다. pi 확장과 똑같이,
  플러그인도 `AGENT_OFFICE_APP_DATA/observer-port` 파일을 읽어 한 번만
  재시도하고, 성공하면 그 URL을 계속 씁니다.
- **훅 OFF일 때** — `observer_url`이 없으면(관찰 기능 자체가 꺼져 있으면)
  `kilo`/`kilocode` 래퍼 자체를 안 만듭니다. 셸은 원래 PATH의 `kilo`를
  그대로 실행합니다.
- **PowerShell도 지원** — agy와 달리 kilo는 v1부터 Windows를 뺄 이유가
  없어서, `set_env_from_env`/`skip_if_env_set`을 POSIX와 PowerShell 양쪽
  렌더러에 구현했습니다.
- **파일 URL의 공백 인코딩** — macOS의 app_data 경로에 공백이 섞일 수
  있습니다. 문자열로 `file://` + 경로를 그냥 이어 붙이면 그 공백이 그대로
  남아 Kilo의 URL 파서가 깨집니다. `reqwest::Url::from_file_path`로 만들면
  공백이 `%20`으로 자동 인코딩됩니다(`kilo_plugin.rs` 테스트로 확인).
- **파일 쓰기 원자화** — 플러그인·설정 파일은 부팅/세션 준비마다 blind
  overwrite됩니다. 그냥 `fs::write`로 덮어쓰면 쓰는 도중 죽거나 다른 세션
  준비와 겹쳤을 때 절반만 쓰인 파일을 Kilo가 그대로 읽을 창이 생깁니다.
  같은 디렉터리에 `.tmp` 임시 파일로 쓴 뒤 `rename`으로 옮기는 식으로
  원자화했습니다(`observer/claude.rs`의 훅 설정 temp+rename과 같은 패턴).
  같은 문제가 있던 `pi_extension.rs`의 확장 파일 쓰기도 함께 고쳤습니다.

## 6. 실측하지 못한 것

- **glob/grep/list/webfetch 도구의 정확한 인자 키.** 스파이크에서 직접 호출을
  못 잡았습니다. OpenCode 표준(§2 표)으로 추정한 값이라, 실제로 다르면
  라벨이 도구 이름만 나오는 정도로 강등될 뿐 관찰 자체는 안 깨집니다
  (`kilo_tool_activity_detail`만 고치면 됩니다).
- **Windows에서 kilo/PowerShell 실기.** 렌더러 코드와 단위 테스트는
  마련했지만, 실제 Windows 환경에서 kilo를 띄워 플러그인이 로드되는지는
  확인하지 못했습니다.
- **`tool.execute.after`.** 우리 플러그인 모듈에는 넣지 않았습니다. `title`만
  오는 페이로드라 지금 라벨 체계로는 쓸 데가 마땅치 않았습니다(§1.3).
