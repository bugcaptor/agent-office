# 사용자 정의 터미널 입력 자동화

상태: **v1 단일 CLI 및 v2 순차 CLI 전환 구현** (2026-09-08).
이슈: KBM [#2t9](https://zgpdlx.tailc90d0d.ts.net:8444/t/2t9).

같은 탭·PTY에서 CLI를 유지하는 v1과, CLI 반환을 확인한 뒤 다음 CLI를 기동하는 v2를 지원한다. 자동 반환은 직접 생성한 빈 bash/zsh 세션에서 지원한다. 검증 환경과 실행 제한은 아래 [CLI 전환과 반환 확인](#cli-전환과-반환-확인)을 따른다.

이 문서는 자동화의 동작 계약과 현재 구현을 함께 관리하는 정본이다. 과거의 파일별 제작 순서와 임시 진행 보고는 KBM 진행에 남긴다.

## 사용자 흐름

탭 메뉴의 `자동화…`는 저장 목록과 편집기를 연다. 메뉴를 여는 것, 예제를 선택하는 것, 저장하거나 JSON을 가져오는 것만으로 터미널에 입력하지 않는다.

1. 새 자동화를 만들거나 저장한 정의를 선택한다.
2. 이름, 입력 변수, 단계, 반복 횟수를 편집한다. 작업 폴더는 편집 대상이 아니다 — 실행을 누른 탭의 작업 폴더를 그대로 읽기 전용으로 보여준다.
3. 단계를 추가·복제·삭제하거나 위아래로 옮긴다. 프롬프트의 개행과 코드블록을 보존한다.
4. 정의를 저장하고 실행 미리보기에서 실제 단계와 기동·종료 입력을 확인한다.
5. 명시적으로 실행을 누른다. 실행은 그 시점의 정의와 입력값을 복사해 사용한다. 실행 중 원본을 편집해도 이미 시작한 실행에는 반영하지 않는다.

미리보기의 시간은 1,000ms 미만이면 `ms`, 이상이면 소수점 한 자리 `sec`로 표시한다(예: `999ms`, `1.0sec`, `1.5sec`). 편집 입력과 저장 값의 단위는 ms를 유지한다.

목록은 복사·삭제·JSON 가져오기/내보내기와 최근 실행 기록을 제공한다. 기본 예제는 단일 작업, 작성→검수, 구현→검수 반복, Claude 설계→agy 구현이다. 단계 수와 역할명은 사용자가 정한다.

편집기 상단의 제목·닫기와 하단의 저장·실행·내보내기는 고정한다. 목록과 설정을 포함한 가운데 영역만 스크롤하며, 작은 창에서 목록이 설정 위로 배치되어도 양쪽 조작부는 계속 보인다. 저장 실패 안내도 스크롤 영역 밖에 표시한다.

## 시간과 직접 입력 계약

자동화는 사용자가 평소 쓰는 터미널에 입력을 대신 넣는다. 사용자는 같은 터미널에서 언제든 직접 타이핑하고, 붙여넣고, CLI 승인에 응답할 수 있다.

- CLI 기동 뒤 기본 10초 기다린다. 시간 지연이며 CLI 준비 완료를 판별하는 신호는 아니다.
- LLM 작업의 결과 대기는 기본 30분이다. 작업 단계마다 조정한다.
- 시간이 지나면 비차단 선택 패널과 알림으로 더 기다리기/중단을 제시한다. 자동 입력, 강제 종료, 자동 선택을 하지 않는다.
- 연장한 deadline은 사용자가 선택한 시각에 해당 단계의 설정 대기시간을 더한다. 연장 횟수 제한은 없다.
- 선택 대기 중 결과가 도착해도 다음 단계로 자동 진행하지 않는다. 완료 신호를 표시하고 선택을 유지한다.
- 완료 마커 뒤 기본 10초 마무리 대기를 거친다. 마커 이후 사람이 직접 제출하면 후속 자동 제출을 보류하고 명시적인 계속/중단 선택을 기다린다.
- 자동화 중단은 자동 입력과 감시를 멈춘다. CLI 종료, Ctrl-C, 입력줄 삭제는 보내지 않는다. 종료 명령은 사용자가 ExitCli 단계를 넣었을 때만 실행한다.
- 미제출 입력의 판정, 30초 알림과 2분 보류 상한은 [입력 조율](#입력-조율)를 따른다.

## 입력 조율

### 공용 관문

`src-tauri/src/session/inject.rs`의 InjectGate가 Bot, Talk, WebRemote, Automation의 자동 입력을 처리한다. 각 생산자는 자기 메시지를 보존하고 제출이 보류되면 다시 시도하거나 호출자에게 보류를 알린다.

본문 → 150ms → CR을 탭별로 직렬화한다. 다른 생산자가 제출 중이면 AnotherProducer로 돌려준다. 네 생산자 모두 출처 표식을 붙여 자동 입력을 사람 프롬프트로 집계하지 않는다.

터미널 키, 외부 ctl 입력, 웹 원격 raw 키는 사람 입력이다. xterm이 PTY 질의에 답하는 커서 위치·상태·장치 속성·색상 응답과 활성화된 포커스 보고는 `terminalResponse` 출처로 구분한다. 이 응답은 PTY에 즉시 전달하지만 사람 제출 epoch, 입력 revision, 미제출 표시를 바꾸지 않는다. 실제 키·IME·붙여넣기는 사람 입력으로 유지하며 임의 ESC 문자열 전체를 제외하지 않는다.

사람 입력은 평소에는 즉시 보내고 자동 제출 구간에 도착한 입력은 CR 뒤에 순서대로 보낸다. 제출 퓨처가 취소돼도 RAII 가드가 표식을 해제해 입력이 큐에 영구히 갇히지 않게 한다.

기존 세션 ID를 본문 직전과 150ms 뒤 CR 직전에 다시 확인한다. 새 세션으로 바뀌었으면 Enter나 기존 세션의 대기 입력을 보내지 않는다.

### 미제출 입력 판정

아래 규칙은 사람 입력에만 적용한다. 터미널 프로토콜 자동응답은 적용 대상이 아니다.

TUI 화면을 읽지 않고 들어온 바이트를 순서대로 반영한다.

| 입력 | 처리 |
|---|---|
| CR 또는 LF | 미제출 표시 해제, 사람 제출 epoch 증가 |
| Ctrl-C 또는 Ctrl-U | 미제출 표시 해제 |
| 그 밖의 데이터 | 미제출 표시 설정 |
| 빈 문자열 | 변경 없음 |

한 청크가 `first\rsecond`라면 second가 남아 있으므로 보류한다. CR이 한 번 포함됐다는 이유로 뒤의 미제출 조각을 무시하지 않는다. 백스페이스만으로 입력줄이 비었다고 추측하지 않는다.

미제출 표시가 있으면 HumanTyping으로 보류한다. 배너의 계속 버튼은 표시만 해제하며 입력줄을 지우거나 대신 Enter를 보내지 않는다.

### 보류 시간

- 배너에 자동 입력 대기를 즉시 표시한다.
- 보류 30초 후 창이 비포커스이면 OS 알림을 한 번 보낸다.
- 미제출 표시가 선 뒤 2분이 지나면 표시를 내리고 자동 제출을 진행한다.
- 이때 사용자 조각이 자동 지시 앞에 붙어 함께 제출될 수 있다. Ctrl-U로 지우지 않는다. `submitted-over-human-fragment`를 실행 이력에 남긴다.

v2의 기동·종료 strict 제출에는 2분 상한을 적용하지 않는다. 일반 LLM/v1/Bot/Talk 제출의 2분 상한은 미제출 조각에만 적용한다. 마커 뒤 사용자가 직접 제출한 경우에는 별도의 사람 개입 선택에서 명시적 계속을 기다린다.

### 마커 이후 사람 개입과 중단

마커 관측 시 사람 제출 epoch를 저장하고 다음 자동 단계 제출 전에 다시 비교한다. 증가했다면 후속 제출을 보류하고 비차단 계속/중단 선택을 표시한다. 완료 마커는 지우지 않는다.

중단은 cancelling → 태스크 정지 → cancelled다. 제출 중에도 취소 신호를 처리하여 아직 보내지 않은 Enter를 보내지 않는다. 종료 입력이나 입력줄 삭제를 부수 효과로 넣지 않는다. 앱 종료 시 자동화 정지를 먼저 수행한다.

## 저장 정의

Rust와 TypeScript는 camelCase JSON 계약을 공유한다. 새 필드는 양쪽 타입과 계약 픽스처로 검증한다.

```text
AutomationDefinition {
  schemaVersion: 1 | 2, id, revision, name,
  inputs: [{ key, label, default? }],
  inputValues?: { [key]: string },
  steps: AutomationStep[],
  repeat?: { maxCycles }
}
```

| kind | 단계 | 주요 설정 |
|---|---|---|
| launchCli | CLI 기동 | cliProfileId, model?, effort?, startupWaitMs (기본 10,000) |
| llmTask | LLM 작업 | label, promptTemplate, waitTimeoutMs (기본 1,800,000), completionGraceMs (기본 10,000), allowEarlyComplete (기본 false) |
| wait | 대기 | durationMs |
| confirm | 사용자 확인 | message |
| exitCli | CLI 종료 | command; v2 전용 returnMode? (auto/manual, 기본 auto), exitWaitMs? (기본 30,000) |

모든 단계는 고유 id를 갖는다. v1·v2의 cliProfileId는 내장 `claude`, `codex`, `agy`, `kilo`, `pi`를 가리킨다. 별도 CLI 프로필 관리 기능은 없다. 모델은 각 CLI의 `--model`, 추론 강도는 Claude·agy의 `--effort`, Codex의 `-c model_reasoning_effort`, Pi의 `--thinking` 인수로 전달하며 미리보기도 같은 인용 규칙을 쓴다.

자동화의 agy 기동에는 `--dangerously-skip-permissions`를 항상 전달해 도구 권한 확인으로 실행이 멈추지 않게 한다. 명령 미리보기에도 같은 옵션을 표시한다.

Kilo Code는 `kilo --auto`로 대화형 CLI를 기동한다. 명시적으로 거절된 권한을 제외한 도구 권한을 자동 승인하며 미리보기에도 옵션을 표시한다. 모델을 지정하면 `--model 'provider/model'`을 추가한다. 대화형 기동에는 추론 강도 인수가 없으므로 편집기에서 해당 입력을 숨기고, 저장·가져오기에서도 `effort`가 지정된 Kilo 단계를 거절한다. `kilo run`의 일회성 실행은 사용하지 않는다. CLI 인수 계약은 [Kilo 공식 CLI 명령 문서](https://kilo.ai/docs/code-with-ai/platforms/cli-reference)를 따른다.

Kilo 모델 후보는 설치된 `kilo models`에서 조회한다. 기존 저장 모델과 수동 입력은 목록 조회 실패에도 유지하며, 후보 목록으로 사용자 지정 모델을 실행 단계에서 차단하지 않는다. CLI는 미리 설치하고 인증해 두어야 한다.

Pi는 `pi --approve`로 대화형 CLI를 기동해 새 프로젝트의 신뢰 질문 때문에 자동화가 멈추지 않게 한다. 모델을 지정하면 `--model 'provider/id'`, 추론 강도를 지정하면 `--thinking 'high'`를 추가한다. Pi 세션은 `/quit`로 종료한다. 편집기가 Pi launch에 대응하는 기본 Exit CLI 명령을 `/quit`로 설정하고, 다른 CLI로 바꾸면 기본값을 `/exit`로 되돌린다. 사용자가 입력한 다른 종료 명령은 바꾸지 않는다. Pi 모델 후보는 `automation_cli_models('pi')`로 설치된 CLI에서 조회하며, 기존 저장 모델과 수동 입력은 목록 조회 실패에도 유지한다.

agy 모델은 현재 `agy models`가 반환하는 정확한 ID를 사용한다. 편집기에서 동적 후보를 제공하고 실행 전에도 명시한 모든 agy 모델을 실제 CLI 목록과 대조한다. 목록을 얻지 못하거나 ID가 없으면 터미널에 기동 명령을 넣기 전에 원인을 표시한다. 모델을 생략하면 CLI 기본값을 사용하므로 목록 조회를 요구하지 않는다. `--effort`는 모델 ID와 별개이며 agy에는 `low`/`medium`/`high`만 허용한다. 예를 들어 모델 목록에 `gemini-3.8-flash-medium`이 있다고 해서 `gemini-3.8-flash`를 같은 모델로 추측해 바꾸지 않는다. 목록 조회 전에 확인한 세션 ID를 런타임 시작과 모든 제출까지 고정한다. 목록 조회 중 또는 검증 직후 세션이 교체되면 새 터미널에 기동하지 않는다.

Claude와 Codex도 `automation_cli_models(cliProfileId)`로 설치된 CLI의 후보를 조회한다. Claude는 Agent SDK와 같은 control initialize 응답의 `models[].value`를 사용한다. 이 값은 `opus` 같은 유효한 CLI 별칭일 수 있으며 하드코딩한 후보가 아니다. Codex는 기존 `codex debug models` 카탈로그의 visible 항목과 우선순위 정렬을 재사용한다. 현재 설치본에서 공식 app-server `model/list`와 같은 목록임을 확인했으나 debug 명령의 형식이 바뀌면 조회 실패로 처리한다.

목록 조회는 프롬프트를 보내지 않으며 시간·출력 크기 상한을 둔다. Claude 조회 자식에만 safe-mode와 빈 MCP 설정을 적용해 훅·플러그인·MCP가 기동되지 않게 하고, 사용자 전역 설정은 바꾸지 않는다. 로딩·조회 실패·재시도를 CLI별로 표시하고, 늦게 도착한 이전 CLI의 응답으로 현재 후보를 덮지 않는다. 수동 입력과 기존 저장 모델은 조회 실패에도 보존한다. Claude/Codex의 목록은 선택 후보이며 사용자 지정 모델·별칭을 실행 단계에서 차단하는 허용 목록으로 쓰지 않는다. CLI를 바꾸면 이전 CLI의 모델·추론 강도는 초기화한다.

반복 횟수는 유한한 양의 정수다. v1에서 LaunchCli는 첫 위치에 최대 하나, ExitCli는 마지막 위치에 최대 하나만 허용한다. 선두 LaunchCli는 처음 한 번, 후미 ExitCli는 마지막 또는 조기 완료 후 한 번 실행한다. 그 사이의 단계만 maxCycles만큼 반복한다. 진행 중 LLM을 반복 상한 때문에 끊지 않는다.

잘못된 버전, 빈 이름/단계, 중복 id/입력 키, 잘못된 시간과 반복 횟수, 알 수 없는 단계/필드는 저장·가져오기 단계에서 거절한다. 가져온 정의를 실행하려면 다시 명시적으로 실행해야 한다.

저장 실패 시 실패 문구와 함께 백엔드 오류 코드 또는 파일 시스템 오류 상세를 표시하고 편집 중인 값은 유지한다.

정의는 앱 데이터 폴더의 `automations/`에 JSON으로 원자 저장한다. 저장 버튼은 실행 입력값(`inputValues`)을 함께 보관하며 다시 선택하거나 편집기를 다시 열 때 복원한다. 입력 변수의 `default`는 템플릿 기본값으로 별도 유지한다. 실행 입력값이 없는 기존 정의는 변수 기본값을 사용한다. 실행 입력값만 바꾸어도 미저장 변경으로 처리한다.

작업 폴더(workspace)는 정의에 저장하지 않는다. 실행 버튼을 누른 그 순간, 실행을 요청한 탭이 세션을 시작한 폴더를 네이티브가 직접 읽어 확정한다. 그래서 같은 정의라도 어느 탭에서 실행하느냐에 따라 작업 폴더가 달라진다. 탭의 세션 시작 폴더를 못 읽으면(세션이 없거나 외부 attach로 붙어 시작 폴더 정보가 비어 있으면) `automation-cwd-unknown`으로 실행을 거절한다. 작업 폴더는 절대경로로 고정하며 셸의 현재 위치를 바꾸지 않는다. 사용자가 셸에서 수행한 `cd`와 세션 생성 시 작업 폴더는 다를 수 있다.

옛 정의 파일에 남아 있던 `workspace` 필드는 읽을 때만 인식하고 버린다. 그 정의를 한 번이라도 다시 저장하면 파일에서 사라진다.

## CLI 전환과 반환 확인

편집기의 흐름 유형을 “CLI 전환”으로 명시적으로 선택하면 schemaVersion 2로 저장한다. 저장·가져오기·재열기만으로 v1을 변환하지 않는다. v1의 기동 생략형, 기동 한 번/본문 반복/종료 한 번 의미도 유지한다.

v2는 순서 배열 전체를 maxCycles회 실행한다(repeat 생략은 1회). compiler가 shell/cli 상태를 추적해 각 LaunchCli와 ExitCli의 대응 구간을 만든다. shell에서만 기동, cli에서만 LLM 작업·종료가 가능하며 wait/confirm은 양쪽에서 허용한다. 최소 하나의 완결 CLI 구간이 필요하고 마지막 상태는 shell이다. 중첩 기동·고아 종료·CLI 밖 작업·미종료 구간은 단계 index/ID/원인 코드와 함께 저장 단계에서 거절한다.

allowEarlyComplete가 켜진 작업의 complete는 현재 구간의 대응 ExitCli만 한 번 실행하고 반환을 확인한 뒤 전체 실행을 완료한다. 다음 CLI를 기동하거나 다른 구간의 종료 문자열을 보내지 않는다. 마무리 대기와 마커 뒤 사람 개입 확인은 유지하며, 명시적 중단은 종료 문자열을 보내지 않는다.

### 최초 확인과 지원 환경

첫 기동 전 비차단 shellReady 선택으로 사용자가 원래 로컬 셸이며 입력줄이 비어 있음을 확인한다. 확인 자체는 터미널에 입력하지 않는다. 읽기 전용 명령 미리보기와 실행 검증은 실제 세션에 기록한 셸 경로를 사용한다.

직접 생성한 Unix bash/zsh의 동기 호출·POSIX 인용·원자 receipt 경로를 지원한다. 임의 startup command/autostart가 있었거나 tmux 호스팅·외부 attach·입양 세션은 원래 셸이 현재 입력 대상임을 증명하지 못하므로 v2를 거절한다. PowerShell/pwsh/Git Bash/WSL/미식별 셸도 현재 렌더링 미지원 오류로 거절한다. manual은 지원하는 직접 bash/zsh 세션에서 반환 확인을 수동화하는 옵션이며, 다른 셸의 명령 인용을 해결하지 않는다. 미검증 플랫폼에 auto를 광고하지 않는다.

### 실행별 반환 기록

긴 기동·반환 기록 문장은 앱 소유 `launch.sh`로 원자 저장하고, PTY에는 `. '절대경로/launch.sh'`라는 짧은 source 호출만 보낸다. 인용 처리 후 UTF-8 바이트와 Enter의 합이 512를 넘으면 주입 전에 거절한다. 이전 CLI의 반환 기록이 생성되어도 셸의 입력 편집기가 아직 준비되지 않았을 수 있으므로, 고정 sleep을 추가하는 대신 canonical 입력 큐에도 온전히 들어가는 길이로 제한한다. macOS에서 긴 한 줄 입력이 1024바이트에서 잘리는 문제를 방지한다.

스크립트는 현재 셸의 subshell에서 원래 CLI/observer wrapper 함수를 동기 호출한다. 현재 셸의 함수와 비export 변수를 상속하며 내부 변수·작업 폴더 변경은 부모 셸에 남기지 않는다. 셸 전역 설정은 바꾸지 않는다.

```text
<app-data>/automation-cli-returns/<runId>/<launchId>/return.json
{ version: 1, sessionId, runId, launchId, launchStepId, stepExecutionId, nonce, exitCode }

<app-data>/automation-cli-returns/<runId>/<launchId>/started.json
{ version: 1, sessionId, runId, launchId, launchStepId, stepExecutionId, nonce }
```

각 실제 기동마다 launchId/nonce가 새로 생긴다. 앱 소유 경로를 준비하고 매 poll에 부모 실경로·심링크·파일 형식·4KiB 상한 및 모든 식별자를 검증한다. 부분 JSON은 다시 읽고 오래된 식별자는 거절한다. 반환 기록은 LLM result.json과 구별하고 프롬프트나 UI 명령 미리보기에 nonce/내부 셸 문장을 노출하지 않는다.

스크립트가 시작되면 먼저 `started.json`을 원자 기록하고, 기록 실패 시 CLI도 실행하지 않는다. 런너는 이 응답을 확인한 뒤에만 CLI 컨텍스트로 전환하고 기존 `startupWaitMs`를 센다. 시작 응답은 제출 후 최대 10초 기다리며, 시간 초과·검증 실패는 기동 재전송 없이 실패로 끝나며 LLM 작업을 보내지 않는다. 시작 응답은 셸이 해당 스크립트를 실행했음을 증명하며 CLI의 인증·신뢰·도구 승인이나 TUI 입력 준비까지 증명하지는 않는다.

정상 exitCode 0만 자동 전환 근거다. idle, observer Stop, PTY Running이나 단순 sleep은 반환 근거가 아니다. startup 대기와 LLM 제출 재시도 직전에도 receipt를 확인하며 CLI가 이미 돌아왔으면 실패로 중단해 LLM 프롬프트를 셸에 보내지 않는다. ExitCli 전에 이미 반환 기록이 있으면 종료 문자열을 보내지 않고 cli-already-returned로 기록한다. 이 신호는 해당 동기 호출의 반환만 나타내며 프로세스 트리 전체 종료나 같은 사용자 권한의 악성 프로세스에 대한 보안 경계는 아니다.

### 종료 선택

- auto: 종료 명령 한 번 제출 후 exiting으로 반환을 기다린다. exitWaitMs는 양의 정수이며 기본 30초다. 시간이 지나면 cliExitTimeout에서 더 기다리기/중단만 허용한다. 연장은 종료 명령을 재전송하지 않는다.
- manual: 종료 명령 한 번 제출 후 cliExitUnconfirmed에서 셸 복귀와 빈 입력줄의 명시적 확인을 기다린다.
- 비정상 코드·잘못된 receipt는 원인을 표시하고 수동 확인/중단으로 전환한다. 수동 확인은 cli-return-manual 이력으로 남긴다.
- 타임아웃 선택 중 정상 receipt가 도착하면 cliReturnObservedAtMs만 표시한다. 자동으로 패널을 닫거나 다음 CLI를 기동하지 않는다. 사용자가 더 기다리기를 선택해야 진행한다.

status.cli는 실제 기동 제출 대상에 맞춰 갱신한다. v2의 optional cliContext는 unknown/shell/cli(cli, launchStepId)를 구분하며, 필드가 없는 v1은 기존 표시를 유지한다. 실행 이력은 cli-launch, cli-exit-submitted, cli-returned, cli-return-manual, cli-already-returned, early-complete와 cli/cycle/launchStepId/exitStepId/launchId/종료 이유를 남긴다.

### 전환 중 입력 보호

InjectGate는 CR/LF 전용 epoch 외에 모든 비어 있지 않은 사람 입력의 revision을 관리한다. 최초 셸 확인·종료 시작부터 다음 기동의 준비 대기가 끝날 때까지 같은 gate의 RAII 전환 소유권을 유지하며, 사이의 wait/confirm도 포함한다. 마지막 반환 확인·중단·오류·세션 교체·drop 때 소유권을 해제한다. 다른 자동 생산자는 보류하고 사람 입력은 계속 전달한다.

strict 기동·종료 제출은 admission/본문/CR 직전에 sessionId, 실행 generation, 취소, 기대 revision을 검사한다. 사람 revision 검사와 각 쓰기는 같은 임계구역으로 묶는다. 2분이 지나도 사람의 미제출 조각 위에 강행하지 않는다. 확인 응답 뒤 새 입력이 있으면 새 decisionId로 다시 확인한다.

기동 CR 성공 전 개입은 셸 확인을 다시 요구한다. 성공 후 startup 개입은 현재 CLI 확인을 요구하며 Launch를 재전송하지 않는다. 본문 이후 CR 전에 취소·개입·세션/세대 변경이 있으면 부분 제출 오류로 끝내고 자동 재시도하지 않는다. 큐의 사람 Enter가 이미 나간 본문 조각을 제출할 수 있으므로 “명령 실행 취소 보장”으로 설명하지 않는다. 사용자가 직접 터미널을 정리한 뒤 새 실행을 시작한다.

CLI의 자연 반환과 PTY 쓰기를 외부 프로세스까지 원자화하지는 않는다. 종료 직전 receipt 재확인 및 gate 검사를 수행하지만 모든 외부 프로세스 변화에 대한 완전한 대상 고정을 보장하지 않는다.

### Claude → agy 핸드오프

예제는 기동/작업/종료 2개 구간으로 구성한다. Claude는 아래 실행·회차별 경로에 목표, 변경 범위, 구현 순서, 영향 파일, 검증 명령, 미해결 항목을 작성하고 done과 경로·요약을 보고한다.

```text
{{workspace}}/.agent-office/automation-runs/{{run}}/handoff/cycle-{{cycle}}/design.md
```

agy는 같은 문서와 previousResult를 읽어 구현하며, 문서가 없거나 불충분하면 추측하지 않고 blocked로 보고한다. previousResult는 CLI 전환·대기·회차 경계에서 지우지 않는다. 대화 기록을 자동 공유하거나 문서를 엔진이 암묵 전송하지 않는다. 최종 보존 문서는 프롬프트에서 docs 정본 반영을 별도로 지시한다.

## 프롬프트와 완료 신호

각 LLM 작업은 실행별 폴더를 사용한다.

```text
<workspace>/.agent-office/automation-runs/<runId>/<stepExecutionId>/
  prompt.md
  result.json
```

치환을 끝낸 본문을 prompt.md에 쓰고, 터미널에는 이 파일을 읽으라는 한 줄 지시와 결과 파일 경로·규격만 넣는다. 변수는 선언한 입력과 `{{workspace}}`, `{{run}}`, `{{cycle}}`, `{{previousResult}}`만 치환한다. previousResult는 직전 완료 LLM 단계의 summary이며 최초에는 빈 문자열이고 회차가 바뀌어도 이어진다. 입력값이나 결과 안에 있는 변수 표기는 다시 평가하지 않는다. 셸 치환식이나 코드를 평가하지 않는다.

```json
{ "version": 1, "runId": "...", "stepExecutionId": "...", "status": "done", "summary": "작업 완료" }
```

- `done`: 현재 작업 완료.
- `complete`: allowEarlyComplete가 켜진 작업에서는 전체 반복을 조기 완료한다. 그 밖의 작업에서는 일반 완료로 취급한다.
- `blocked`: 비차단 사용자 선택으로 전환한다. 더 기다리기는 같은 실행의 결과를 재검증하며, 계속은 해당 작업을 명시적으로 건너뛴다. 프롬프트를 다시 보내지 않는다.

runId와 stepExecutionId가 다른 결과는 현재 실행의 완료로 인정하지 않는다. 읽는 중인 부분 JSON은 다시 확인한다. 현재 실행의 프로토콜 오류는 숨기지 않고 사용자에게 보여준다. 수정 후 같은 실행에서 기다릴 수 있어야 한다.

실행 폴더의 실경로가 workspace 내부인지 확인하고 심링크를 통한 외부 경로 이탈을 거절한다. 결과 파일 자체가 심링크이면 읽지 않는다. 결과 파일 크기 제한은 64KiB다. 제어 파일이 사용자 저장소의 커밋 후보로 올라오지 않도록 `.agent-office/.gitignore`를 둔다.

## 실행 상태와 복원

실행은 runId/sessionId, 불변 정의 스냅샷, 입력값과 workspace, 회차와 stepIndex/stepExecutionId, 현재 phase/deadline/decisionId를 가진다. 배너는 자동화 이름·회차·단계·현재 대기와 남은 시간을 보여준다.

선택은 runId·stepExecutionId·decisionId가 모두 맞을 때 한 번만 적용한다. 중복 클릭과 오래된 창의 응답은 무시한다. Confirm, blocked, 사람 개입, 타임아웃 패널은 터미널 포커스를 빼앗지 않는다.

- 렌더러 reload는 백엔드 상태를 조회해 실행과 선택 패널을 복원한다. 창이 닫혔다고 자동 진행하지 않는다.
- 실행 이력은 단계 전이, 마커, 시간 만료, 연장/계속/중단 선택, 오류와 시각을 저장한다.
- 앱 종료는 자동화 태스크를 먼저 정지시키고 기존 세션 종료/핸드오프 정책을 따른다.
- 크래시 후 미완료 기록은 interrupted로 복원한다. 자동으로 프롬프트를 재전송하거나 실행을 재시작하지 않는다.
- 세션이 사라지거나 같은 탭에 새 세션이 생기면 옛 실행은 그 세션에 입력하지 않는다. 모델 카탈로그처럼 실행 전 비동기 점검이 있으면 점검 전의 sessionId를 고정하고, 런타임 진입 직전에도 같은 실행 세션인지 다시 확인한다. 다르면 이력·태스크·입력을 만들지 않고 시작을 거절한다.
- 중단·실패 배너는 `터미널 재시작` 버튼으로 기존 재시작 흐름을 연다. 자동화 중단 자체는 CLI를 종료하지 않으며, 사용자가 재시작을 선택했을 때 기존 PTY와 스크롤백을 폐기하고 새 터미널을 만든다.
- 세션 폐기 시 해당 자동화를 취소하고 런타임 스냅샷과 입력 관문을 정리한다. 실행 이력과 저장된 자동화 정의는 보존한다. 재시작은 프런트 자동화 상태도 제거하고 응답 세대를 바꾸므로, 이전 폴링·시작·중단·결정 응답이 배너나 새 실행 상태를 되살리거나 덮지 못한다. 재시작·세션 이어하기 전체를 탭별 생성 예약으로 보호해, 기존 PTY 종료 이벤트 뒤 클릭이나 중복 재시작이 별도 세션을 먼저 만들지 못하게 한다.

## 구현 위치

| 영역 | 위치 |
|---|---|
| 정의·저장·실행·결과·이력 | `src-tauri/src/automation/` |
| IPC | `src-tauri/src/ipc/commands/automation.rs` |
| 자동/사람 입력 조율 | `src-tauri/src/session/inject.rs` |
| 목록·편집기 | `src/renderer/automation/` |
| 배너·선택·메뉴 | `src/renderer/terminal/` |
| 공유 타입 | `src/shared/types/automation.ts`, `src-tauri/src/types.rs` |
| UI 문구 | `src/shared/i18n/locales/*/terminal.json` |

## 검증

필수 자동 검증은 `npx tsc --noEmit`, `npx vitest run --dir src`, `cargo test --manifest-path src-tauri/Cargo.toml`이다. 편집/저장/가져오기만으로 실행되지 않는 것, 명시 실행, 단계 순서/반복, 타임아웃 동안 입력 0회, 중단 뒤 입력 0회, 중복 결정 거절, 크래시 후 자동 재실행 0회를 회귀로 확인한다.

실 PTY 회귀는 다음 명령으로 별도 실행한다. 첫 테스트는 v1 Launch 1회/Task N회/Exit 1회를, 둘째는 Claude→agy 대역의 2 CLI×2회와 지연 반환·문서 핸드오프를 검증한다.

```sh
rtk cargo test --manifest-path src-tauri/Cargo.toml definition_roundtrip_in_real_shell_with_cli_double -- --ignored
rtk cargo test --manifest-path src-tauri/Cargo.toml v2_two_cli_two_cycles_roundtrip_in_real_shell_with_cli_double -- --ignored
rtk cargo test --manifest-path src-tauri/Cargo.toml v2_safety_tests
```

인증된 실제 CLI 왕복은 선택적이다. 기존 단일 CLI는 `AGENT_OFFICE_REAL_AUTOMATION_CLI`에 claude/codex/agy/kilo/pi를 지정한 `definition_roundtrip_with_installed_cli -- --ignored`, v2 실제 핸드오프는 `v2_handoff_with_installed_claude_and_agy -- --ignored`로 실행한다. 임시 작업 폴더만 사용하고 인증을 우회하거나 전역 설정을 바꾸지 않는다. agy는 자동화 기본 기동 옵션인 `--dangerously-skip-permissions`, Kilo는 `--auto`를 사용한다. 실제 CLI 검증이 인증/신뢰/도구 승인에서 멈추면 대역 테스트 통과와 구분하여 KBM에 기록한다.

지원 플랫폼의 자동 검사 외에 미검증 Windows 경로는 활성화하지 않는다. 최신 실행 결과·실 CLI 제약·커밋은 KBM 진행에 남긴다.

## 범위 밖

예약 실행, 자동화 전용 PTY/세션, 실행 중 사람 입력 잠금, 병렬 그래프, 새 원격 실행 API, 플러그인 실행기는 후속 범위다. 실행 전체 시간 상한으로 LLM을 강제 종료하지 않는다.
