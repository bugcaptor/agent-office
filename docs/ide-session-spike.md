# 기존 IDE 세션 관찰 연결 스파이크

상태: 실험 구현. KBM [2w6](https://zgpdlx.tailc90d0d.ts.net:8444/t/2w6). 2026-09-12.

Agent Office는 여러 작업을 오케스트레이션하는 사람을 돕는다. 앱 밖에서 이미
실행 중인 Codex·Claude 작업도 캐릭터에 연결할 수 있어야 한다. 이 스파이크는
기존 VS Code 확장을 그대로 사용하고, 확장이 기록하는 로컬 JSONL에 새로 추가된
활동만 작은 Node 연결기로 읽어 앱에 전달한다. 새 VS Code 확장은 필요 없다.

## 사용하기

Node.js 18 이상과 **이 변경을 포함해 빌드한 Agent Office**가 필요하다.
기존 설치 버전에는 관찰 API가 없으므로 `observed-api-unavailable` 오류가 난다.
저장소에서 `npm run tauri dev`로 개발 앱을 실행할 수 있다. 기존 앱과 같은 데이터
디렉터리를 사용하는 두 인스턴스를 동시에 띄우지 않는다.

1. VS Code에서 Codex 또는 Claude 확장을 평소처럼 사용한다. 기존 세션을
   재시작하거나 확장 설정을 바꿀 필요는 없다.
2. Agent Office에서 연결할 캐릭터의 **작업 폴더를 세션의 `cwd`와 동일하게**
   설정한다. 현재 PTY나 다른 외부 연결이 있는 캐릭터는 선택할 수 없다.
   캐릭터 생성 시 앱이 터미널을 열었다면 그 앱 터미널을 종료한 후 연결한다.
3. 앱 설정에서 CLI 제어를 켜고 승인한다. 연결기는 기존 `control-port`와
   `control-token`을 사용하며 이 설정을 자동으로 바꾸지 않는다.
4. 저장소에서 후보 세션과 캐릭터 ID를 확인한다.

   ```sh
   npm run spike:ide -- list --provider codex
   npm run spike:ide -- list --provider claude
   npm run spike:ide -- agents
   ```

   출력은 JSON이다. `file`, `sourceSessionId`, `cwd`, `updatedAt`, `source`를
   보고 연결할 기록을 선택한다. **목록은 기록 후보이며 실행 중인 프로세스 목록이
   아니다.** 현재 열린 대화인지는 VS Code에서 확인한다. 기본 목록은 기록에
   VS Code 출처가 확인된 것만 표시한다. `--all-sources`로 다른 출처도 볼 수 있다.

5. 선택한 파일을 캐릭터에 연결한다. `FILE`과 `AGENT_ID`를 실제 값으로 바꾼다.

   ```sh
   npm run spike:ide -- watch --provider codex --file "FILE" --agent "AGENT_ID"
   # Claude는 --provider claude
   ```

   여러 작업은 연결기를 각각 실행하고 서로 다른 캐릭터를 지정한다. 기존 세션의
   시작·도구·완료 이벤트가 새로 기록되면 해당 캐릭터의 활동/완료 알림으로 흐른다.
   연결 직후 상태는 `unknown`이며 과거 프롬프트·완료를 재생하지 않는다.

6. 캐릭터 탭의 **VS Code에서 작업 폴더 열기**로 돌아간다. 정확한 대화 탭 선택은
   VS Code에서 한다. 연결기에서 Ctrl+C 또는 앱의 **연결 해제**로 관찰을 끝낸다.
   연결 해제는 원본 Codex·Claude·VS Code 프로세스를 종료하지 않는다.

앱 없이 파싱만 점검하려면 `watch ... --dry-run`을 사용한다. 이 경우에도
연결 시점 이후의 이벤트만 JSON으로 출력하며, 앱이나 원본 확장에는 쓰지 않는다.

```sh
npm run spike:ide -- watch --provider claude --file "FILE" --dry-run
```

## 발견 범위와 옵션

- 기본 저장 위치는 `$CODEX_HOME/sessions`(미지정 시 `~/.codex/sessions`),
  `$CLAUDE_CONFIG_DIR/projects`(미지정 시 `~/.claude/projects`)이다.
- `--cwd PATH`는 후보의 작업 폴더를 걸러낸다. `--limit N`은 출력 개수(1–200).
- 스파이크는 공급자마다 최대 2,000개 디렉터리 항목을 방문하고, 그 안에서
  수정 시각이 최근인 파일 200개까지만 검사한다. 전체 기록 검색을 보장하지 않는다.
  하위 에이전트 디렉터리와 디렉터리 내부의 심볼릭 링크는 탐색하지 않는다.
- 오래된 파일이나 다른 위치는 `list --provider claude --file "FILE"`로 직접
  확인하고 `watch --file`로 연결할 수 있다.
- 출처 필드가 없는 옛 형식은 `source: "unknown"`으로 남긴다. 파일을 직접
  확인한 경우에만 `watch ... --allow-unknown-source`로 명시적으로 선택한다.
  서브에이전트 기록은 이 옵션으로도 연결하지 않는다.
- `--app-data PATH` 또는 `AGENT_OFFICE_APP_DATA`로 다른 앱 데이터 디렉터리를
  선택한다. 기본은 기존 `ctl`/`vscode-ext`와 같은 OS별 앱 데이터 경로다.
- 도움말: `npm run spike:ide -- --help` (`--lang ko|en|ja|fr|zh-Hans|zh-Hant`).

## 관찰하는 신호

| 공급자 | 기록의 VS Code 출처 | 시작 | 도구 | 완료 |
|---|---|---|---|---|
| Codex | `session_meta.payload`: `originator=codex_vscode`, `source=vscode`, `thread_source=user` | `event_msg/task_started` | `response_item/function_call` 또는 `custom_tool_call` | `event_msg/task_complete` |
| Claude | `entrypoint=claude-vscode`, `isSidechain!=true` | `user`, `promptId`, `message.role=user`, 도구 결과 제외 | `assistant`의 `tool_use` 블록 | `assistant.message.stop_reason=end_turn` |

공급자의 native session ID와 파일 위치를 연결의 대상으로 삼는다. Codex는
turn/call ID, Claude는 prompt/message ID로 같은 이벤트를 중복 제거한다.
전사 전체를 JSON으로 파싱하지만, 출력·전송하는 것은 공급자/세션/작업 폴더 등의
메타데이터와 활동 종류뿐이다. 프롬프트, 답변, 도구 인자, 토큰 사용량은 전송하지 않는다.

파일 수정 시각은 후보 정렬에만 사용한다. 일정 시간 조용하다는 이유로 완료나
승인 대기라고 판단하지 않는다. `turn_aborted`, 임의 assistant 텍스트,
`stop_hook_summary`, 큐 기록 등은 완료로 바꾸지 않는다.

## 앱 연결 계약

기존 인증을 적용하는 로컬 control 서버에 세 라우트를 추가했다.

| 라우트 | 핵심 입력 | 결과 |
|---|---|---|
| `POST /v1/observed/attach` | `agentId`, `provider`, `sourceSessionId`, `cwd`, UUID `ownerId`, 연결기 `pid` | `{sessionId}` |
| `POST /v1/observed/event` | `agentId`, `sessionId`, `ownerId`, 양의 `sequence`, `kind` | `{accepted}` |
| `POST /v1/observed/detach` | `agentId`, `sessionId`, `ownerId` | `{detached}` |

각 결과는 기존 `{ok:true,data:...}` 봉투를 따른다. `kind`는
`prompt|tool|stop|attention|heartbeat`이며 연결기는 `attention`을 보내지 않는다.
앱의 observer 설정이 꺼져 있으면 이벤트가 거절된다.

- 관찰 attach는 기존 PTY/외부 연결을 교체하거나 합류하지 않는다. 같은 소유자와
  공급자/native ID의 재요청은 멱등이다. 하나의 native ID를 여러 캐릭터에
  중복 연결하지 않는다.
- 세션/소유자가 다른 이벤트와 해제 요청은 현재 연결을 건드리지 않는다.
  이미 수신한 sequence는 다시 알리지 않는다. 연결기의 전송 재시도는 같은
  sequence를 유지한다.
- 관찰 세션은 `external=true`인 논리 세션이다. 셸을 생성하거나 persona,
  훅 설정, 확장 설정을 주입하지 않는다. 기존 외부 연결 UI와 알림 허브를 재사용한다.
- 연결기는 5초마다 heartbeat를 보낸다. 앱은 30초 lease 만료를 5초 주기로
  정리하며, Unix에서는 연결기 PID 종료도 감지한다. 이는 **연결기**의 생존
  확인이며 원본 IDE 프로세스의 생존 확인은 아니다.
- 원본 파일 교체/축소는 관찰을 중단한다. 파일 검사와 tail 연결 사이 교체도
  파일 identity로 거절한다. 과거 기록을 다시 읽어 새 완료 알림으로 만들지 않는다.
  2 MiB 초과 한 줄은 건너뛰고 읽기는 한 tick 최대 4 MiB로 제한한다.

## 스파이크의 한계와 다음 판단

- 기록 형식은 공개된 안정적 구독 API가 아니다. 공급자 버전 변경에 맞춘 검증이
  필요하다. 실제 로컬 VS Code 기록 샘플에서 메타데이터와 이벤트 필드를
  확인했지만, 현재 설치 버전에서 새 작업을 실행하는 GUI 실기 인수는 별도다.
  자동·통합 검증은 macOS에서 수행했으며 Windows/Linux 실기는 수행하지 않았다.
- 현재 IDE가 살아 있는지, 승인을 기다리는지, 작업이 중단됐는지는 보장하지 않는다.
  최초 연결 시 진행 중이던 작업도 다음 명시적 이벤트 전까지 현재 활동을 모른다.
  출처 필드는 기록의 출처이며 그 대화를 나중에 어느 UI로 재개했는지 보장하지 않는다.
- 대화 본문/사용량/서브에이전트 집계/봇 입력/승인/정확한 대화 포커스는 제공하지 않는다.
  작업 폴더 열기 버튼은 기존 `openInVscode(cwd)`이며 채팅 탭을 지정하지 않는다.
- 연결기의 자동 재연결·재부팅 복구·GUI 후보 선택은 범위 밖이다. 앱 또는 연결기가
  종료되면 명시적으로 다시 연결한다. 다음 단계에서 원본 IDE의 세션 식별 API와
  사용자 수동 매핑을 비교하고, 필요한 경우에만 기존 `vscode-ext`를 브리지로 확장한다.

## 검증

```sh
npm run spike:ide:test
npx tsc --noEmit
npx vitest run --dir src
cargo test --manifest-path src-tauri/Cargo.toml
# 실제 Node 연결기 → Rust loopback 서버 → 캐릭터 알림 → SIGTERM 해제
cargo test --manifest-path src-tauri/Cargo.toml node_codex_watch_forwards_new_records_without_replaying_history_then_detaches -- --ignored
```

Node 테스트는 두 전사 형식, 과거 미재생, 부분 UTF-8/JSONL, 중복/다른 세션/서브에이전트,
파일 교체/축소, 인증 오류와 sequence 재시도, 실제 연결기 프로세스의 HTTP 전송과
SIGTERM 해제를 검사한다. Rust 테스트는 실제 loopback 서버에서 인증된 attach와
이벤트 전달, 기존 세션 보존, 멱등/소유권/해제/lease를 검사한다. 실제 원본 기록은
읽기 전용으로 출처 판별과 최초 EOF에서 0건 반환을 확인했다.
별도 ignored 통합 스모크는 실제 Node 연결기와 Rust 서버를 함께 실행해
Codex 형식의 새 기록만 캐릭터 활동·완료 알림으로 전달되고 해제되는 것을 검증한다.

## 구현 위치

- `scripts/ide-session-spike.mjs`: 실행 명령과 연결기 수명
- `scripts/ide-session-spike/`: 전사 파서, 후보 발견, HTTP 클라이언트, 테스트
- `src-tauri/src/control/observed.rs`: 인증된 관찰 API
- `src-tauri/src/session/external.rs`: 원본 프로세스를 소유하지 않는 논리 세션
- `src/renderer/terminal/TerminalHost.tsx`: 연결 해제와 VS Code 작업 폴더 이동

배경: [외부 터미널 attach](external-session-attach-design.md),
[기존 VS Code 로그 뷰어](vscode-character-extension-plan.md).
