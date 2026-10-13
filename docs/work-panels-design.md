# 업무 패널 — 현재 구현 명세

설정으로 켜는 좌우 고정 패널 — 좌측 업무 목록(배정·선행 작업·보내기·모니터링·수신함), 우측 세션 로그(전 캐릭터, 필터·검색)의 정본 명세다.

상태: 정본 (2026-10-14) — 1·2차 기능과 업무 수신함, 선행 작업·상세 팝업 구현 완료. 설치 Tauri 앱의 저장·복원과 인증된 CLI 5종의 실기 인수는 남아 있다. 보내기는 직접 생성한 Unix bash/zsh와 Windows PowerShell·Git Bash PTY를 지원한다.

관련 문서: [터미널 자동화](archive/terminal-automation-design.md)(제거됨 — CLI 기동 receipt의 출처), [세션 활동 분석](session-analytics-design.md)(턴·토큰·비용 집계), [세션 로그](session-log-design.md)(전사 보기), [동료 대화](archive/agent-talk-design.md) §6(shim·권한 사전 승인 배선의 출처, 기능은 제거됨 — 배선은 `office_instructions.rs`). 수신함의 구현 계획과 결정 근거는 [archive/work-inbox-design.md](archive/work-inbox-design.md)에 있다.

## 개요

설정에서 켜면 오피스 화면 좌우에 고정 사이드바가 생긴다. 좌측은 사람이 관리하는 **업무 목록**이고, 업무를 캐릭터에게 보내면 앱이 에이전트 CLI를 띄우고 첫 프롬프트까지 입력한 뒤 상태를 지켜본다. 우측은 업무와 무관하게 실제로 돌았던 **세션 로그**를 모든 캐릭터에 걸쳐 보여준다.

- 좌측 = 계획(사람이 정한 할 일), 우측 = 사실(실제로 일어난 세션).
- 둘은 '보내기'로 만든 세션에서만 연결된다. 사람이 터미널에서 직접 한 작업도 우측에는 모두 나온다.

## 활성화 방식

시스템 설정의 토글 하나(`AppSettings.workPanelsEnabled`, 기본 `false`)로 켜고 끈다. 로컬 데스크톱의 오피스 화면에만 표시하며 마스코트에는 노출하지 않는다.

- 꺼져 있으면 패널을 만들지 않고 레이아웃은 지금과 같다.
- 꺼도 업무 데이터와 세션 요약 레코드는 지우지 않는다. 다시 켜면 그대로 돌아온다.
- 세션 요약 레코드(아래 데이터 모델)는 설정과 무관하게 항상 쌓는다. 나중에 켜도 그동안의 세션을 볼 수 있게 하기 위해서다(세션 로그의 opt-out 근거와 같음). 크기는 세션당 수 KB다.
- 설정은 Rust `AppSettings`와 TS 미러에 함께 저장한다.

## 좌측 패널: 업무 목록

업무는 앱 전역 목록 하나다. 추가·수정·삭제하고, 담당 캐릭터에게 보내면 앱이 에이전트를 띄워 시작시킨다.

### 업무 항목

| 필드 | 내용 |
| --- | --- |
| 제목 | 한 줄, 선택. 비우면 본문 앞 40자(공백·줄바꿈 합침, Unicode 문자 기준)를 연한 색으로 표시 |
| 본문 | 여러 줄 메모. 보낼 때 제목과 함께 첫 프롬프트가 된다 |
| 담당 캐릭터 | 프로필 1명 또는 미배정. 카드에 얼굴(스프라이트/초상)과 이름 |
| 에이전트·모델 | CLI(claude/codex/agy/kilo/pi) + 모델 + 노력(effort). 비우면 담당 캐릭터의 기본값 |
| 상태 | 할 일 / 진행 중 / 검토 대기 / 완료 |
| 연결 세션 | 이 업무로 보낸 세션 id 목록(다시 보내면 늘어남) |
| 선행 작업 | 먼저 완료돼야 하는 업무 목록. 아래 [선행 작업](#선행-작업) |

### 목록 구성

- 상태별 묶음을 검토 대기 → 진행 중 → 할 일 → 완료 순으로 보여준다. 완료 묶음은 기본 접힘.
- 묶음 안 순서는 드래그로 바꾼다.
- 카드는 제목·담당·미저장 표시·선행 작업 대기·후속 작업 수·모니터링만 보여준다. 상세 보기와 편집은 화면 가운데 팝업(모달)에서 한다. 패널 폭과 상관없이 입력란을 넓게 쓰기 위해서다.
- 목록 위의 '새 태스크 만들기' 버튼으로 제목·본문·담당·에이전트·상태·선행 작업을 입력하는 초안 팝업을 연다. '추가'를 누를 때만 업무를 생성하고 팝업을 닫는다. 제목과 본문 중 하나는 있어야 한다.
- 기존 카드를 누르면 상세 팝업에 편집 초안·모니터링·보내기·터미널·삭제를 보여주고 '저장'을 눌러야 반영한다. 필드 입력과 포커스 이동은 저장하지 않는다. 헤더 ×/Esc/배경 클릭은 초안을 유지한 채 닫는다. 새 업무도 닫았다가 '새 태스크 작성 계속'으로 돌아올 수 있다.
- 팝업 버튼은 두 줄로 역할을 나눈다. 편집 줄은 왼쪽 끝 '변경 되돌리기'(새 업무는 '초안 버리기')와 오른쪽 끝 주 버튼 '저장'(새 업무는 '추가')이다. 되돌리기는 변경이 있을 때만 보이되 자리는 비워 둬 저장 버튼이 움직이지 않으며, 팝업을 닫지 않는다. 바뀐 것이 없으면 저장을 막는다. 구분선 아래 실행 줄은 왼쪽 끝에 떼어 둔 위험 버튼 '삭제'(확인창 있음)와 오른쪽의 '완료로 표시'·'터미널'·주 버튼 '보내기'다. '완료로 표시'는 상태만 즉시 완료로 저장하고 다른 초안은 남긴 채 닫는다. 비활성 버튼은 흐리게 표시하고, 보내기를 막은 이유(선행 작업·담당·출근·다른 업무 전송 중 등)는 버튼 아래 문구로 보여준다. 재시작·삭제 확인창이 뜨는 동안 상세 팝업은 잠시 내려간다. 패널 접기는 초안을 보존하지만 업무 패널 기능을 끄거나 앱을 종료하면 임시 초안은 버린다.
- 편집 중에는 저장한 제목·담당·상태를 카드에 계속 표시하고 미저장 표시를 추가한다. 변경이 있으면 보내기가 '저장 후 보내기'로 바뀌어 초안 기준으로 판정·저장한 뒤 보낸다. 보내기에 성공하면 팝업을 닫아 열린 터미널을 보여주고, 실패나 재시작 확인 취소는 오류를 볼 수 있게 팝업을 남긴다. 자동 모니터링 상태·세션 이력은 초안과 별개로 갱신하며 저장은 사용자가 편집한 필드만 반영한다.
- 빈 제목을 그대로 저장해 본문을 수정하면 표시 제목도 함께 갱신한다. 제목 입력란의 연한 placeholder·삭제/재시작 확인·세션 기록의 연결 업무에도 같은 본문 앞부분을 사용한다.
- 삭제는 확인을 받는다. 진행 중 업무를 지워도 터미널 세션은 건드리지 않는다.

### 선행 작업

업무는 다른 업무를 선행 작업으로 가질 수 있다(`dependsOn`). 큰 일을 순서 있는 단계로 나눌 때 쓴다.

- 편집 팝업의 '선행 작업'에서 고르거나 뺀다. 후보는 자신·이미 고른 업무·완료된 업무·순환을 만드는 업무(이 업무를 직간접으로 기다리는 업무)를 뺀 나머지다.
- 선행 작업 중 완료되지 않은 것이 있으면 카드에 '선행 작업 대기: 제목들'을 경고색으로 표시하고 보내기를 막는다(`work-send-blocked`). 막지 않으려면 선행 작업을 빼거나 완료로 바꾼다. 자동으로 이어서 보내지는 않는다.
- 이 업무를 기다리는 업무가 있으면 '후속 작업 N개가 기다림'을 표시한다.
- 저장할 때 목록에 없는 id·자기 자신·중복·순환을 걸러낸다. 선행 업무를 지우면 후속 업무에서 그 조건만 빠진다.

### 에이전트 설정

- 프로필의 `defaultAgent`를 캐릭터 편집의 터미널 영역에서 저장한다. 업무의 에이전트 지정이 비어 있으면 프로필 설정 전체를 사용하며, 프로필도 비어 있으면 Claude 기본값이다. 업무에서 CLI를 명시하면 모델/effort는 그 CLI의 기본값으로 시작한다. CLI를 바꾸면 이전 모델/effort를 지운다.
- Claude/Codex/agy/Pi 모델 후보는 `workAgentModels`(`work_agent_models`) 조회를 쓴다. 수동 입력·저장된 모델은 조회 실패에 보존한다. agy의 명시 모델은 실행 직전 실제 목록과 정확히 대조하고, 모델·effort 검증과 POSIX 인용은 기존 기동 코드를 재사용한다.

### 보내기

카드의 '보내기' 버튼은 담당 캐릭터가 있고 지원되는 프로필일 때 누를 수 있다. 같은 캐릭터에는 한 번에 한 업무만 보낸다.

1. 담당 캐릭터의 터미널 상태를 본다.
   - 세션이 없으면 새로 만들고, 미제출 입력이 없는 빈 셸로 확인되면 기존 세션을 사용한다.
   - CLI가 돌거나 미제출 입력이 있거나 빈 셸임을 확인할 수 없으면 재시작 확인을 표시한다. 미제출 입력은 추가 경고로 보여준다. 취소하면 세션·입력·업무 상태를 바꾸지 않는다.
2. 필요하면 세션을 재시작한다(기존 재시작 경로 재사용).
3. 고른 CLI를 모델·노력 인자와 함께 기동한다.
4. 기동 ACK 확인 후 기본 10초를 기다리고 제목+본문을 첫 프롬프트로 제출한다.
5. 첫 프롬프트 제출 성공 후 업무 상태를 진행 중으로 바꾸고 사용한 세션 id를 업무에 연결한다.

기동·입력 중 실패하면 기존 상태와 연결 세션 이력을 유지하고 카드에 오류 한 줄을 띄운다. 할 일에서 보냈다면 할 일로 남는다. 실패한 보내기를 자동 재전송하지 않는다.

#### 지원 대상

- 앱이 직접 생성한 Unix bash/zsh PTY와 Windows PowerShell(5.1·7)·Git Bash PTY만 보낼 수 있다. 프로필에 시작 명령이 있거나 퇴근 중이면 보내기를 막는다. 입양·WSL·미지원 셸은 막는다. tmux 호스팅·외부 연결·봇/터미널 자동화는 제거됨(2026-10-14, [archive/health-check-cleanup-plan.md](archive/health-check-cleanup-plan.md) §7.G).
- 셸 판정은 세션의 실제 셸 실행 파일 이름으로 한다(`LaunchShell::for_shell_path`, `work_send/cli_transition.rs`).
- 기동 문장은 셸별로 만든다. bash/zsh는 `. '<launch.sh>'`, Git Bash는 같은 POSIX 스크립트에 `C:/…` 경로를 쓴다. PowerShell은 BOM 없는 UTF-8로 ACK·반환 기록을 쓰는 `launch.ps1`을 `& ([scriptblock]::Create([System.IO.File]::ReadAllText('…')))`로 실행한다. 실행 정책은 스크립트 파일에만 적용되므로 정책과 무관하고, 자식 스코프라 변수가 새지 않으며 현재 위치는 Push/Pop-Location으로 되돌린다. CLI 인자는 PowerShell 작은따옴표로 인용하고, PowerShell 5.1이 네이티브 인자의 큰따옴표를 지우므로 codex effort는 `-c 'model_reasoning_effort=high'`로 넘긴다(codex는 TOML로 못 읽는 값을 문자열로 받는다).

#### 입력과 재시작 규약

- 기존 PTY는 원래 셸의 foreground pgid와 그룹 구성원이 셸 하나인지 확인하며, 미제출 입력이 없을 때 바로 보낸다. 다른 프로세스가 있거나 조회가 불가능하면 재시작을 확인한다. Windows ConPTY에는 foreground 그룹이 없으므로 셸 프로세스에 살아 있는 자손이 없을 때만 빈 셸로 본다(Git Bash는 `bin\bash.exe` 런처가 띄운 내부 `bash.exe` 하나만 허용, `session/win_process.rs`).
- 재시작 확인은 세션과 사람 입력 revision을 고정한다. 확인 후 키 입력/세션 교체가 있으면 폐기하지 않는다. 미제출 입력은 별도 경고로 표시한다. 재시작 위치는 관측된 현재 cwd를 우선하며 프로필 시작 폴더는 바꾸지 않는다.
- 새 PTY의 입력 대상이 불명확해도 기동 ACK를 확인하기 전에는 첫 프롬프트를 보내지 않는다. 이미 다른 프로세스가 foreground인 것으로 확인된 새 PTY는 거절한다. ACK는 최대 10초 기다리고, 성공 시점부터 기본 10초 기동 대기를 센다.
- `WorkSendRuntime`은 캐릭터별 동시 보내기를 거절하고, InjectGate의 전환 소유권/strict 제출로 다른 자동 생산자를 보류한다. 사람 입력은 계속 전달되며 기동/대기/본문/CR 전에 세션·세대·revision을 재검사한다. 이 경로에는 미제출 입력의 2분 강행 규칙을 적용하지 않는다.
- 제목이 있으면 제목+빈 줄+본문, 제목이 없으면 본문 전체만 bracketed paste로 제출해 줄바꿈·코드 블록을 보존한다. 제목은 한 줄, 본문은 LF/탭만 허용하며 터미널 제어문자는 거절한다. 전체 프롬프트 상한은 256 KiB다. 기존 `Automation` 출처/자동 프롬프트 분류를 재사용한다.
- 기동 ACK가 없거나 준비 대기 중 반환 receipt가 생기면 프롬프트를 보내지 않는다. 실패한 기동·부분 제출은 자동 재전송하지 않으며 다시 보내려면 재시작 확인을 거친다. 본문 뒤 개입으로 CR을 보내지 못한 경우 카드에서 터미널 확인을 안내한다.
- 첫 프롬프트의 CR 제출 성공 후에만 `sessionIds`·세션 요약의 `workItemId`를 연결하고 진행 중으로 바꾼다. 실패는 기존 상태/세션 이력을 보존하며 오류 코드를 저장해 현재 언어로 표시한다. 보내기 도중 삭제한 업무를 복구하지 않는다.

### 모니터링

보낸 뒤 카드는 연결 세션의 실시간 상태를 보여준다.

- 최신 연결 세션의 작업 중 / 질문 대기 / 턴 종료 / 세션 종료 + 경과 시간 + 턴 수·토큰·추정 비용.
- 질문 대기(훅 알림)면 카드를 강조한다. 단 턴 종료·세션 종료 뒤의 훅 알림(Claude `idle_prompt` 입력 대기 리마인더)은 질문으로 보지 않고 다음 prompt까지 턴 종료를 유지한다.
- 완료 업무는 세션 신호와 무관하게 "완료"로 표시하고 강조하지 않는다. 세션이 살아 있으면 경과 시간을 숨기고, 끝난 세션이면 확정된 경과 시간을 보인다.
- 상세 팝업의 터미널 버튼을 누르면 팝업을 닫고(초안 유지) 연결 세션을 소유한 캐릭터의 터미널 탭을 연다. 카드 클릭은 상세 팝업만 연다. 업무 담당을 바꿔도 기존 연결 세션의 소유자를 따른다.
- 모니터링은 패널 토글과 무관하게 구독한다. 비용·토큰은 기존 실시간 사용량과 복원 시드를 합치고 비용 표시 설정을 따른다. 복원 시드는 기존 3일 조회 한계를 따른다.

### 상태 전이

최신 연결 세션의 Stop 알림 또는 최종 usage는 진행 중→검토 대기, 후속 prompt는 검토 대기→진행 중으로 바꾼다. 부분 usage, 셸 idle, 이전 연결 세션, 늦은 이벤트로 상태를 바꾸지 않는다.

```text
            보내기 성공                턴 종료
  [할 일] ─────────────▶ [진행 중] ─────────────▶ [검토 대기]
     ▲                     ▲                         │
     │ 보내기 실패          │   같은 세션에 후속 프롬프트 │
     └─(상태 유지)          └─────────────────────────┘
                                                     │ 사람이 체크
                                                     ▼
                                                  [완료]
```

완료는 사람이 체크하거나, 사람이 지시해 에이전트가 `office-task --done`을 실행할 때만 된다. 사람은 어느 상태에서나 상태를 직접 바꿀 수 있다.

보내기가 실패하면 상태는 바뀌지 않는다. 사람이 할 일 또는 완료로 바꾼 업무는 자동 상태 전이 대상에서 제외한다.

### 내보내기와 가져오기

목록 위 '새 태스크 만들기' 옆의 '내보내기·가져오기' 버튼이 팝업을 연다. 업무 목록을 불러온 뒤에만 누를 수 있다.

- 내보내기는 상태(검토 대기·진행 중·할 일·완료)와 담당 캐릭터를 체크해 둘 다 맞는 업무만 네이티브 저장 대화상자로 고른 위치에 쓴다. 담당 선택지는 업무가 있는 담당만 미배정·캐릭터(생성 순)·'삭제된 캐릭터'(지금 없는 캐릭터 전부를 하나로 묶음) 순으로 보이고, 괄호 수는 고른 상태 안에서 센다. 처음에는 모두 체크되어 있고(담당은 뺀 것만 기억해 새로 생긴 담당도 체크된다), 고른 업무가 없으면 버튼을 막는다. 기준은 메모리의 저장된 업무 목록이며 팝업에서 편집 중인 미저장 초안은 넣지 않는다. 수신함(`work-inbox/`) 파일은 대상이 아니다.
- 가져오기는 열기 대화상자로 파일을 고르고, 추가할 수와 건너뛸 수를 보여준 뒤 '추가'를 눌러야 반영한다. 파일의 담당 중 이 컴퓨터에 없는 캐릭터가 있으면 '찾을 수 없는 캐릭터' 묶음에 캐릭터별(파일의 `assigneeName`, 없으면 id와 업무 수)로 대체 선택 상자를 보여주고, 고른 캐릭터로 그 캐릭터의 업무를 한꺼번에 바꾼다. 기본값은 이름이 같은 캐릭터가 하나뿐이면 그 캐릭터, 아니면 미배정이다. 확인 전에 대체 캐릭터가 지워지면 미배정으로 둔다. 항상 새 id로 추가하고 기존 업무는 바꾸지 않는다. 업무 불러오기·저장 오류가 있으면 막는다.
- 대화상자 취소는 안내 없이 그대로 둔다. 저장 성공은 수와 경로, 실패·형식 오류는 경고로 팝업 안에 보여준다.

파일은 사람이 편집기에서 항목별로 지우거나 고칠 수 있게 2칸 들여쓴 UTF-8 JSON이다. 기본 이름은 `agent-office-work-items-YYYYMMDD.json`.

```json
{
  "format": "agent-office-work-items",
  "version": 1,
  "exportedAt": "2026-10-13T00:00:00.000Z",
  "items": [
    {
      "id": "…",
      "title": "제목",
      "status": "todo",
      "assigneeAgentId": "…",
      "assigneeName": "캐릭터 이름",
      "agent": { "cli": "codex", "model": "…", "effort": "high" },
      "dependsOn": ["…"],
      "body": ["본문 첫 줄", "둘째 줄"],
      "order": 0,
      "sessionIds": [],
      "lastError": "…",
      "createdAt": "2026-10-01T00:00:00.000Z",
      "updatedAt": "2026-10-02T00:00:00.000Z"
    }
  ]
}
```

- 내보내기는 `WorkItem` 저장 필드 전부를 쓴다. 담당 캐릭터가 지금 있으면 그 이름을 `assigneeName`으로 덧붙인다(다른 컴퓨터에서 대체할 때 보여주는 참고 값이며 가져온 업무에는 저장하지 않는다). 본문은 줄 배열, 시각은 ISO 문자열이다. 항목은 상태 묶음 순서·묶음 안 순서대로 놓는다. 값이 없는 선택 필드는 생략한다.
- 가져오기는 손으로 고친 파일을 너그럽게 받는다. 본문은 줄 배열·문자열 모두, 시각은 ISO·밀리초 숫자 모두 받는다. 모르는 상태는 할 일, 모르는 CLI의 `agent`는 버린다. 제목과 본문이 모두 빈 항목이나 객체가 아닌 항목은 건너뛰고 수를 센다. 앱 데이터의 `work-items.json`(배열)도 그대로 받는다. `format`이 다르거나 `version`이 더 크거나 JSON이 아니거나 남는 업무가 없으면 오류로 알린다.
- 가져온 업무의 제목·본문·상태·담당(없는 캐릭터는 고른 대체)·에이전트·`createdAt`은 파일 값을 쓴다. `order`는 파일의 배열 순서대로 각 상태 묶음 끝에 다시 매기고 `updatedAt`은 가져온 시각이다. `sessionIds`·`lastError`는 원래 카드의 실행 기록이라 잇지 않는다.
- `dependsOn`은 파일 안 다른 항목의 `id`를 가리키면 그 항목의 새 id로 다시 잇는다. 파일 밖 id는 현재 목록에 있으면 그대로, 없으면 버린다. 자기 참조·중복·순환은 선행 작업 규칙대로 뺀다.

## 우측 패널: 세션 로그

한 줄 = 터미널 세션 하나(시작~종료)다. 모든 캐릭터의 세션을 최신순으로 보여주고, 진행 중인 세션은 맨 위에 실시간으로 갱신한다.

### 한 줄에 보이는 것

| 항목 | 출처 |
| --- | --- |
| 캐릭터(얼굴+이름) | `session_started`의 agentId·agentName 스냅샷 |
| 시작~종료 시각, 소요, 실작업 시간 | session-events, session-times. 실작업 시간이 있으면 폴더 옆에 표시 |
| 무엇을 했나(한두 줄) | 머리 위 라벨의 목표(goal). 없으면 첫 프롬프트 앞부분 |
| 작업 폴더 | `session_started.cwd`의 폴더 이름 |
| 턴 수·토큰·추정 비용 | prompt/usage 이벤트 합산(기존 비용 단가 재사용) |
| 연결 업무 태그 | 업무에서 보낸 세션이면 업무 제목 |

요약 문구는 추가 LLM 호출 없이 기존 라벨 목표와 프롬프트만 쓴다. 요약기가 꺼져 있으면 첫 프롬프트로 대체한다.

### 펼친 상세

항목을 클릭하면 패널 안에서 펼쳐진다.

- 프롬프트 목록(시각 + 앞부분, 기계 입력은 출처 표시)
- 턴별 소요와 모델별 토큰·비용(이벤트별이 아니라 모델별 합계 한 줄씩)
- 연결 업무(있으면) — 클릭하면 좌측 카드로 스크롤
- '세션 로그 보기' 링크 — 전사가 남아 있을 때만(30일 보존), 기존 SessionLogDialog를 연다

### 검색·필터

- 텍스트 검색: 요약 문구, 프롬프트, 폴더 이름, 캐릭터 이름, 연결 업무 제목에서 부분 일치.
- 캐릭터: 여러 명 선택. 기본은 전원.
- 기간: 오늘 / 7일(기본) / 30일 / 전체(최대 300일).
- 작업 폴더: 기간 안에 나온 폴더 목록에서 여러 개 선택.

### 보존과 과거 세션

- 조회는 최대 300일이고 기본 표시는 최근 7일이다. 요약 월 파일의 만료 처리는 아래 저장 규약을 따른다.
- opt-in 이전 세션도 보존된 session-events를 300일 범위에서 재구성해 보여준다. 캐릭터·시각·폴더·턴·토큰은 나오지만 요약 레코드가 없는 과거 세션은 요약 문구와 프롬프트 본문이 없을 수 있다.
- 우측 패널은 읽기 전용이다. 로그 항목을 지우거나 고치는 기능은 없다.
- 패널 마운트 시 필요한 이벤트 종류만 300일 범위에서 읽고 이후 5초마다 새 구간을 중복 키(`runId:seq`)로 병합한다. 상세 전사 존재 확인은 펼친 세션에서만 실행한다.
- 이름·폴더·시작 시각은 `session_started` 스냅샷을 우선한다. 종료 시각이 없으면 현재 시각으로 대체하지 않고 부재로 표시한다.
- 앱 종료는 PTY를 kill한 뒤 `disposed` 기록 전에 프로세스가 끝나는 경우가 많다. 종료 이벤트가 없고 진행 중도 아닌 세션이 지금보다 앞선 실행(runId)에 속하면, 그 실행이 마지막으로 남긴 이벤트 시각을 추정 종료로 쓰고 "~경(앱 종료로 추정)"으로 표시한다. 열린 턴은 기존 규칙대로 세션의 마지막 이벤트에서 마감한다.
- 프롬프트·턴·사용량·연결 업무가 모두 없는 세션(앱을 켤 때 캐릭터마다 열리는 빈 셸 등)은 기본으로 숨긴다. 목록 아래 '빈 세션 보기 (N)'를 켜면 함께 보인다. 필터 후보(캐릭터·폴더)도 보이는 줄 기준이다.

## 업무 수신함

에이전트나 외부 프로그램이 업무 목록에 할 일을 남기는 입력 규약이다. 앱 데이터 폴더의 `work-inbox/`에 정해진 형식의 파일을 넣으면 `todo` 업무로 들어간다. `done` 키가 있는 파일은 새 업무가 아니라 기존 업무의 완료 요청이다. 누가 넣었는지는 따지지 않는다.

- 가져온 업무는 항상 `todo`이고 자동으로 보내지 않는다. 보내기는 사람이 누른다.
- 발신자 인증은 하지 않는다. 같은 사용자 권한의 프로세스는 누구나 넣을 수 있고, 할 일 한 줄이 추가될 뿐 실행되지 않는다.
- 마스코트에서는 처리하지 않는다. 화면 문자열은 추가하지 않았다. 가져온 업무는 기존 카드로 보이고 저장 실패는 기존 미저장 표시·재시도를 쓴다.

### 위치와 파일 형식

- `<app_data>/work-inbox/`. 앱이 시작할 때 만든다. 앱이 띄운 세션에는 `AGENT_OFFICE_WORK_INBOX`로 절대 경로를 알려준다. OS별 앱 데이터 경로는 [사용 안내](guide.md#업무-수신함)에 있다.
- 파일 하나가 업무 하나다. 확장자 `.md`, UTF-8(BOM 허용), LF/CRLF 모두 받아 LF로 바꾼다. 64 KiB까지.
- 선택 frontmatter: 첫 줄이 정확히 `---`이면 다음 `---` 줄까지를 `key: value`로 읽는다(YAML 전체는 지원하지 않는다). `assignee`(담당 캐릭터의 프로필 id 또는 이름), `id`(업무 id, `[A-Za-z0-9_-]` 1~64자. 형식이 틀리면 무시), `after`(선행 업무 id를 쉼표·공백으로 구분. 형식이 틀린 id는 버림), `done`(완료할 업무 id. 구분은 `after`와 같다)만 쓰고 그 외 키는 무시한다.
- frontmatter 다음의 첫 비어 있지 않은 줄이 제목이다. 앞의 `# `는 떼고, `#`만 있으면 빈 제목이다. 나머지가 본문이며 앞뒤 빈 줄만 다듬고 들여쓰기는 보존한다. 제목과 본문 중 하나만 있어도 된다.

```markdown
---
assignee: 하나
---
# 로그인 화면 회귀 테스트 추가
세션 만료 뒤 재로그인 흐름이 테스트에 없다.
```

담당은 가져오는 시점의 프로필로 판정한다. id가 정확히 일치하면 그 캐릭터, 아니면 이름(앞뒤 공백 무시, 대소문자 구분)이 **정확히 하나** 일치할 때만 배정한다. 없거나 여럿이면 미배정으로 받고 `warnings.log`에 `unknown-assignee`·`ambiguous-assignee`를 남긴다. 에이전트·모델은 비워 두고 보낼 때 담당의 `defaultAgent`를 따른다.

### 쓰기 규약 (생산자)

- 다른 이름으로 끝까지 쓴 뒤 같은 폴더 안에서 `.md`로 rename한다. 앱은 `.`으로 시작하는 파일과 `.md`가 아닌 파일, 디렉터리·심볼릭 링크를 읽지 않는다. 권장 임시 이름은 `.<최종 이름>.tmp`.
- 파일 이름순으로 가져오므로 `20261012T091500123Z-<임의값>.md`처럼 시각을 앞에 둔다.

### 처리 규약 (앱)

- 업무 패널이 켜져 있고 업무 목록 로드에 성공한 동안 3초마다 확인한다. 한 주기에 최대 50개, 주기는 겹치지 않는다. 종료 확인창이 떠 있는 동안은 멈춘다.
- 이름순으로 `todo` 묶음 끝에 추가한다. 선택·편집 초안·접힘 상태는 바꾸지 않는다.
- `done`에 유효한 id가 하나라도 있으면 완료 요청이다. 제목·본문은 비어도 되고 무시한다. 같은 주기의 새 업무를 모두 추가한 뒤 적용하므로 방금 들어온 업무도 완료할 수 있다. 해당 업무를 `done` 묶음 끝으로 옮기며 이미 완료된 업무는 건너뛴다. 목록(같은 주기 추가 포함)에 없는 id가 하나라도 있으면 그 파일은 아무것도 완료하지 않고 `rejected/`로 간다(`unknown-task`, 둘째 줄에 없는 id들). `office-task --done`과 같은 원자성이다. 사람이 편집 중인 초안은 건드리지 않는다(저장하면 초안의 필드가 이긴다). 업무 저장이 성공한 뒤에만 파일을 지운다.
- `after`는 기존 업무나 같은 주기에 함께 들어온 업무만 남기고 나머지는 버린다. 버린 선행은 업무를 막지 않으므로 업무는 받고 `warnings.log`에 사유를 남긴다: 없는 id `unknown-prerequisite`, 자기 자신 `self-prerequisite`, 순환을 만드는 id `cyclic-prerequisite`(같은 주기에 서로를 기다리면 먼저 처리한 쪽의 선행이 빠진다). 중복은 조용히 합친다. 생산자는 선행 업무 파일을 먼저(이름순 앞에) 써야 한다. `office-task`는 시각을 이름 앞에 두므로 차례로 부르면 된다.
- 업무 id는 frontmatter `id`가 있으면 그 값, 없으면 `inbox-` + (파일 이름과 원본 바이트의 SHA-256 앞 16자)다. 같은 id가 이미 있을 때 제목·본문까지 같으면 재수신으로 보고 추가 없이 파일만 지운다(경고도 다시 남기지 않는다). 그래서 저장 성공과 삭제 사이에 앱이 죽어도 두 번 생기지 않는다. 내용이 다르면 기존 업무를 덮지 않고 `rejected/`로 옮긴다(`duplicate-id`, 둘째 줄에 id). 같은 주기 안의 중복 id도 같다.
- 업무 스냅샷 저장이 성공한 뒤에만 원본을 지운다. 저장에 실패하면 파일을 남기고 다음 주기에 먼저 저장을 재시도한다.
- UTF-8이 아니거나, 64 KiB를 넘거나, frontmatter가 닫히지 않았거나, 제목·본문이 모두 비거나, 새 업무의 제목·본문에 탭·줄바꿈 외 제어문자(lone CR 포함)가 있으면 `rejected/`로 옮기고 같은 이름 + `.error` 파일에 사유 코드(`invalid-utf8`·`too-large`·`unclosed-frontmatter`·`empty`·`control-char`) 한 줄을 남긴다. `control-char`는 둘째 줄에 처음 찾은 문자(`U+001B` 형식)를 적는다. 업무 보내기와 같은 기준이라 받아 놓고 보낼 수 없는 카드가 생기지 않는다. 이름이 겹치면 앞에 uuid를 붙여 이전 거부 파일을 보존한다. `rejected/`는 자동으로 비우지 않는다.
- 의미 오류는 조용히 버리지 않는다. 기준: **반영하면 기존 업무를 잘못 바꾸거나 요청의 뜻을 이룰 수 없으면 거부**(`duplicate-id`·`unknown-task` — `.error` 첫 줄 코드, 둘째 줄 대상 id), **업무 자체는 성립하고 일부 입력만 버리면 받고 경고**(담당·선행). 거부는 업무 저장 성공 뒤 ack와 같은 시점에 옮긴다.
- 경고는 `work-inbox/warnings.log`에 `시각(UTC)⇥파일 이름⇥업무 id⇥코드⇥값` 한 줄로 덧붙인다(제어 문자는 공백으로). 256 KiB를 넘으면 `warnings.log.1` 하나만 남기고 새로 쓴다. 경고 기록에 실패해도 업무 파일은 ack한다.
- 패널이 꺼져 있거나 앱이 꺼져 있으면 파일은 쌓이고, 켜지면 순서대로 들어간다. 수정된 지 24시간이 지난 `.`으로 시작하는 `.tmp` 파일은 확인할 때 지운다.
- 종료할 때는 루프를 즉시 멈추고, 진행 중인 주기를 기존 업무 flush와 함께 기다린다.

### `office-task` 명령

앱이 띄운 세션의 에이전트가 수신함 규약을 쓰는 도구다. OS temp의 `agent-office/bin/office-task`(Windows는 `.cmd`) shim이 앱 바이너리의 오프라인 서브커맨드 `agent-office task`를 실행한다. GUI·control 서버·토큰이 필요 없어 CLI 제어가 꺼져 있어도 동작한다. shim은 세션 준비마다 다시 쓰고, 앱이 떠 있는 동안 5초마다 확인해 없거나 내용이 다르면 되살린다(OS temp 청소로 떠 있는 세션이 경로를 잃지 않게).

```
office-task [--to <이름|id> | --unassigned] [--after <id>]... <제목> [본문 | -]
office-task --list [--all]
office-task --done <id>...
```

- 본문 자리의 `-`는 stdin을 읽는다. 여러 줄은 heredoc으로 넘긴다. `--` 뒤는 모두 제목·본문이다.
- 수신함 경로는 `AGENT_OFFICE_WORK_INBOX` → `AGENT_OFFICE_APP_DATA/work-inbox` 순서로 찾고 둘 다 없으면 1로 끝난다.
- 담당: `--to` 값, `--unassigned`면 비움, 둘 다 없으면 `AGENT_OFFICE_AGENT_ID`(세션 캐릭터의 프로필 id). 그 변수가 없으면 미배정으로 쓰고 stderr에 알린다.
- 업무 id를 `t-` + 임의 8자로 정해 frontmatter `id`에 쓰고 stdout에 **id만** 출력한다. 셸에서 `A=$(office-task …)`로 받아 다음 업무의 `--after "$A"`에 넘겨 순서 있는 단계를 만든다.
- `--after`는 여러 번 쓸 수 있고 값 하나에 쉼표로 여러 id를 넣어도 된다. 형식이 틀린 id가 있으면 쓰지 않고 2로 끝난다. `work-items.json`과 수신함 미처리 파일 어디에도 없는 id는 stderr에 `warning: --after <id>: no such task`를 알리되 업무는 쓴다(0) — 앱은 그 선행을 빼고 받는다.
- `--to`는 `profiles.json`을 읽어 id나 유일한 이름과 맞지 않으면 stderr에 경고(없는 이름·이름 여럿)하고 그대로 쓴다(0). 앱이 미배정으로 받는다. 기본 담당(`AGENT_OFFICE_AGENT_ID`)은 확인하지 않는다. 앱 데이터 파일을 읽을 수 없으면 두 확인을 건너뛴다.
- 제목의 줄바꿈은 공백으로 합치고, 제목 줄은 항상 `# `로 써서 본문이 frontmatter·제목으로 잘못 읽히지 않게 한다. 쓰기 규약대로 임시 파일 → rename한다. 업무 목록 반영(앱·패널 켜짐)은 확인하지 않는다.
- 제목·본문이 모두 비거나, 탭·줄바꿈 외 제어문자가 있거나(수신함이 거절할 파일), 64 KiB를 넘으면 쓰지 않고 1, 인자 오류(`--to`·`--after` 값 누락·형식 오류·줄바꿈 포함 등)는 2로 끝난다.
- `--list`는 앱 데이터 폴더(`AGENT_OFFICE_APP_DATA`, 없으면 수신함의 부모)의 `work-items.json`을 읽어 완료가 아닌 업무를 검토 대기 → 진행 중 → 할 일, 묶음 안 순서대로 한 줄씩 출력한다: `id  [status]  @담당 이름  제목  after: id들 (blocked)`. 담당 이름은 `profiles.json`에서 찾고, 제목이 비면 본문 앞 40자를 쓴다. `--all`이면 완료도 포함한다. 저장된 값의 제어문자는 터미널에 해석되지 않게 `\x1b`처럼 이스케이프해 찍는다. 앱이 아직 가져가지 않은 수신함 파일은 개수만 알린다. 디바운스 저장 전의 최신 편집은 보이지 않을 수 있다.
- `--done`은 id(여러 개·쉼표 구분 가능)를 `work-items.json`과 수신함의 미처리 파일 `id`에서 찾는다. 하나라도 없으면 아무것도 쓰지 않고 1(`no such task`)로 끝난다. 이미 완료된 것은 stderr에 알리고 뺀다. 남은 id로 `done:` frontmatter만 있는 완료 요청 파일을 쓰고 완료 요청한 id를 한 줄씩 출력한다. 남은 게 없으면 쓰지 않고 0이다. 형식이 틀린 id·값 없음은 2다.

세션 셸에는 업무 패널 설정과 무관하게 다음 env를 싣는다(`session/manager.rs`의 `prepare_session_plan`).

| 변수 | 값 |
| --- | --- |
| `AGENT_OFFICE_AGENT_ID` | 세션 캐릭터의 프로필 id |
| `AGENT_OFFICE_WORK_INBOX` | 수신함 절대 경로 |
| `AGENT_OFFICE_TASK` | `office-task` shim 절대 경로. 안내문 채널이 없는 CLI에서 사용법을 알려줄 때 쓴다 |

### 공통 안내문 주입

CLI별 스킬 대신 짧은 영어 안내문(`src-tauri/src/office_instructions.rs`, shim 절대 경로를 끼워 넣음)을 시스템 프롬프트에 **덧붙인다**. 후속 작업을 발견했거나 사용자가 나중 할 일로 남기라고 할 때 쓰는 법, 기본 담당은 본인이고 넘길 때만 `--to`, 이번 턴에 끝낼 일은 남기지 않기, 다른 사람이 읽을 맥락 쓰기, 출력된 id와 `--after`로 의존성을 걸어 계획을 단계별로 구조화하는 법, `--list`로 기존 업무를 보고 잇는 법, 사용자가 완료하라고 지시했을 때만 `--done`으로 완료 처리하기(스스로 끝났다고 판단해 완료하지 않기), 남긴 뒤 한 줄로 알리기를 담는다.

- 업무 패널(`workPanelsEnabled`)이 켜졌을 때만 넣는다. 세션을 띄울 때 고정되므로 실행 중 설정 변경은 이미 떠 있는 세션에 반영되지 않는다.
- Claude·Codex는 "persona + 빈 줄 + 안내문"(있는 것만)을 하나의 값으로 넣는다. env 이름 `AGENT_OFFICE_PERSONA`는 래퍼 호환 때문에 유지한다.

| CLI | 채널 |
| --- | --- |
| Claude | `--append-system-prompt "$AGENT_OFFICE_PERSONA"` |
| Codex | `-c developer_instructions="…"`(`AGENT_OFFICE_CODEX_PERSONA`). codex-cli 0.162에서 developer 메시지로 들어감을 `codex debug prompt-input`으로 확인 |
| Pi | `--append-system-prompt "$AGENT_OFFICE_OFFICE_PROMPT"`(pi 1.1.0). persona는 싣지 않는다. 관찰 확장 파일이 사라져 prefix를 포기하는 경우 안내문도 빠진다 |
| Kilo | 앱 소유 플러그인의 `experimental.chat.system.transform` 훅이 `AGENT_OFFICE_OFFICE_PROMPT`를 `output.system`에 덧붙인다(kilo 7.8.3 바이너리에서 훅 이름·인자 확인). 플러그인은 관찰이 켜져 있고 사용자가 `KILO_CONFIG`를 쓰지 않을 때만 실린다 |
| agy | 덧붙이기 플래그가 없다(1.3.3). `AGENT_OFFICE_TASK` 등 env만 제공하므로 프로젝트 AGENTS.md에 사용법을 적는다 |

### 권한과 샌드박스

- **Claude 권한**: 세션 설정 조각의 `permissions.allow`에 켜진 기능의 shim만 넣는다 — 업무 패널이 켜지면 `Bash(<office-task>:*)`. 관찰이 켜져 있으면 훅 설정 파일에 합치고, 꺼져 있으면 권한 조각만 담은 세션 설정 파일을 `--settings`로 물린다. 대화 플러그인(`--plugin-dir`)은 대화가 켜졌을 때만 붙는다.
- **Codex 샌드박스**: 안내문을 실은 세션에 `-c sandbox_workspace_write.writable_roots=[…]`로 수신함을 더한다. 이 `-c`는 사용자 설정의 같은 배열을 **덮어쓰므로**(0.162 실측) `CODEX_HOME`(또는 `~/.codex`)`/config.toml` 최상위 값을 읽어 합친 배열을 넘긴다. 설정이 없거나 깨졌으면 수신함만 넣는다. 사용자 파일은 쓰지 않는다. 사용자가 `read-only` 샌드박스면 승인이 뜨고, `-p` 프로필 레이어의 값은 합치지 않는다.
- **Claude Bash 샌드박스**를 사용자가 켰다면 수신함 쓰기에 승인이 뜰 수 있다. 받아들인다.
- Pi·Kilo·agy의 샌드박스·승인 동작은 아래 실기 인수에서 확인한다.

## 데이터 모델과 저장

업무와 세션 요약은 별도 저장소에, 기본 에이전트는 프로필의 `defaultAgent`에 저장한다. 모두 앱 데이터 폴더 아래다.

| 저장소 | 위치·필드 | 내용 | 쓰는 쪽 |
| --- | --- | --- | --- |
| 업무 | `work-items.json` | `WorkItem[]` 전체. 변경 시 디바운스 저장(메모 저장과 같은 규약) | 렌더러 → IPC |
| 세션 요약 | `session-summaries/v1/YYYY-MM.jsonl` | 세션당 레코드. 같은 sessionId는 마지막 줄이 이긴다 | 렌더러 → IPC |
| 프로필 기본값 | `AgentProfile.defaultAgent` | `{ cli, model, effort }`. 비어 있으면 claude 기본 | 캐릭터 편집 화면 |

### WorkItem

```ts
interface WorkItem {
  id: string;            // nanoid
  title: string;         // 빈 문자열 허용. 표시 제목은 본문에서 생성
  body: string;
  assigneeAgentId?: string;
  agent?: { cli: "claude" | "codex" | "agy" | "kilo" | "pi"; model?: string; effort?: string };
  status: "todo" | "in_progress" | "review" | "done";
  order: number;         // 상태 묶음 안 정렬
  sessionIds: string[];  // 보낸 세션들, 최신이 마지막
  lastError?: string;    // 오류 코드. 표시할 때 현재 언어로 번역
  dependsOn?: string[];  // 선행 업무 id. 비면 생략
  createdAt: number;
  updatedAt: number;
}
```

### SessionSummary

```ts
interface SessionSummary {
  sessionId: string;
  agentId: string;
  agentName: string;     // 시작 당시 스냅샷
  cwd?: string;
  startedAt: number;
  endedAt?: number;      // 진행 중 또는 종료 시각 유실 시 없음
  goal?: string;         // 라벨 목표
  prompts: { at: number; text: string; origin?: string }[]; // 최대 50개, 각 300자
  workItemId?: string;
}
```

- 턴 수·토큰·비용·작업 시간은 저장하지 않고 session-events에서 계산한다(분석 패널과 같은 집계 코드 재사용).
- 요약 레코드는 렌더러가 세션 시작·종료와 `taskLabels` 변화를 구독해 만든다. 패널·일기·요약기 설정과 무관하게 항상 수집한다.
- 과거 세션은 session-events만으로 줄을 만들고 요약 레코드가 있으면 합친다.
- 부팅 복원 시 만료 경계보다 앞선 월 파일을 지운다. 경계 월 안의 300일 초과 레코드는 읽기·표시에서 제외하고 그 월이 완전히 만료되면 파일을 제거한다.

### 저장과 종료 규약

- 업무의 추가/저장 확정은 즉시 전체 스냅샷을 flush하고, 상태 자동 전이·정렬 등은 1초 디바운스로 저장한다. 동시 로드·저장을 직렬화하고 실패 시 미저장 표시와 재시도를 제공한다.
- 요약은 변경된 세션을 1초 디바운스로 직렬 append하며 저장 실패 시 재시도한다. 프롬프트는 **처음 50개**를 남겨 첫 프롬프트 폴백을 보존한다. 각 본문은 Unicode 문자 기준 300자다. 이후 프롬프트도 세션 이벤트의 턴·사용량 집계에는 계속 포함된다.
- 정상 종료에서는 재시작 확인 대기를 취소하고 이미 시작한 보내기 결과를 기다린 뒤 업무·요약을 flush한다. 저장에 실패하면 종료를 보류해 재시도할 수 있도록 한다. OS 강제 종료에 대한 마지막 디바운스 보장은 하지 않는다.
- 세션 폐기·핸드오프·앱 종료는 네이티브 보내기를 취소한다.

## 레이아웃과 상호작용

좌우 패널은 고정 사이드바다. 켜면 오피스 캔버스가 가운데로 좁아지고, 오버레이처럼 캔버스를 가리지 않는다.

- TopBar·BottomBar 사이의 높이를 쓴다. 터미널 오버레이와 다이얼로그는 지금처럼 패널 위에 뜬다.
- 폭은 각각 기본 280px, 경계를 드래그하거나 경계의 좌우 방향키로 220~480px 안에서 조절한다.
- 각 패널은 머리의 접기 버튼으로 24px 띠로 접힌다. 폭과 접힘 상태는 로컬(localStorage)에 기억한다.
- 캔버스 폭이 줄면 오피스 씬은 기존 리사이즈 경로로 다시 맞춘다. 티커 칼럼(SessionTimePanel·알림)은 우측 패널 안쪽으로 밀린다.
- 화면 문자열은 모두 `src/shared/i18n/locales/`의 6개 지원 언어(ko·en·fr·ja·zh-Hans·zh-Hant)에 둔다. 픽셀 테마 토큰을 따른다.
- 키보드: 팝업을 열면 첫 입력란에 포커스. 제목 입력란 Enter = 추가/저장(변경이 있을 때), 본문 Enter = 줄바꿈, Esc = 초안을 유지하며 팝업 닫기. 전역 단축키는 이번 범위가 아니다.

## 구현 위치와 IPC

| 역할 | 현재 코드 |
| --- | --- |
| 영속 타입·보내기 계약 | `src/shared/types/workPanels.ts`, `src-tauri/src/types.rs` |
| 설정·패널 배치 | `src/shared/types/settings.ts`, `src-tauri/src/persistence/settings_store.rs`, `src/renderer/settings/`, `src/renderer/layout/UIChrome.tsx` |
| 패널 UI·폭·접힘·상세 팝업 | `src/renderer/workpanels/WorkItemsPanel.tsx`, `WorkDialog.tsx`, `WorkEditor.tsx`, `workItemDraft.ts`, `SessionHistoryPanel.tsx`, `WorkPanelFrame.tsx`, `panelLayoutStore.ts`, `workPanels.css` |
| 프로필 기본값·업무 오버라이드 | `src/renderer/workpanels/WorkAgentFields.tsx`, `src/renderer/profile/` |
| 선행 작업 정리·후보·대기 판정 | `src/renderer/workpanels/workDependencies.ts` |
| 업무·요약 수집과 저장 | `src/renderer/workpanels/workItemStore.ts`, `sessionSummaryStore.ts`, `sessionRows.ts`, `src-tauri/src/persistence/work_panels_store.rs` |
| 보내기 조정·오류 표시 | `src/renderer/workpanels/workSendStore.ts`, `sendErrorText.ts` |
| 네이티브 보내기·CLI 기동 | `src-tauri/src/work_send.rs`, `src-tauri/src/work_send/{cli_transition,launch}.rs`(기동 receipt·모델/effort 검증·기동 문장, 예전 `automation/`에서 이전), `src-tauri/src/ipc/commands/work_panels.rs` |
| strict 입력·세션 재시작 | `src-tauri/src/session/inject.rs`, `src/renderer/agent/restartAgentSession.ts`. `InjectSource::WorkSend` 사용 |
| Windows 빈 셸 판정 | `src-tauri/src/session/win_process.rs`(Toolhelp 스냅숏), `pty_factory.rs`(`PtyControl::process_id`), `manager.rs`(`is_shell_prompt`) |
| 모니터링·자동 상태 전이 | `src/renderer/workpanels/workMonitor.ts`, `WorkCardMonitor.tsx`. activity·notification·최종 usage 이벤트 구독 |
| 종료 저장 | `src/renderer/workpanels/lifecycle.ts` |
| 수신함 파서·scan·ack | `src-tauri/src/persistence/work_inbox.rs`, `src-tauri/src/ipc/commands/work_panels.rs` |
| 내보내기·가져오기 팝업·파일 형식 | `src/renderer/workpanels/WorkTransferDialog.tsx`, `workItemFile.ts`, `workItemStore.ts`(`importFile`), `src-tauri/src/ipc/commands/work_panels.rs`, `misc.rs`(`save_text_with_dialog`·`open_text_with_dialog`) |
| 수신 루프·담당 판정·가져오기 | `src/renderer/workpanels/workInbox.ts`, `workItemStore.ts`(`importInbox`), `lifecycle.ts`(`installWorkInboxLifecycle`) |
| `office-task` 서브커맨드·shim | `src-tauri/src/work_inbox_cli.rs`, `src-tauri/src/lib.rs`(`maybe_run_cli`의 `task` 분기) |
| 세션 env·안내문·권한·Codex 쓰기 경로 | `src-tauri/src/session/manager.rs`(`prepare_session_plan`), `src-tauri/src/office_instructions.rs`, `src-tauri/src/lib.rs`(권한 조각), `src-tauri/src/session/kilo_plugin.rs` |
| 턴·토큰·비용·전사 보기 | `src/renderer/analytics/`, `src/renderer/usage/`, `src/renderer/sessionlog/SessionLogDialog.tsx` |

IPC 계약은 `src/shared/ipc.ts`와 `src/renderer/ipc/tauriApi.ts`의 로컬 어댑터를 통해 사용한다.

| IPC | 동작 |
| --- | --- |
| `load_work_items` / `save_work_items` | 업무 전체 스냅샷 읽기·쓰기 |
| `load_session_summaries` / `append_session_summaries` | 월별 요약 복원·append |
| `inspect_work_send_target` | 세션 id·사람 입력 revision·재시작 필요·미제출 입력을 읽기 전용으로 점검 |
| `send_work_item` | 고정된 세션·revision에 CLI와 프롬프트 제출. 성공하면 `sessionId`·`submittedAt` 반환 |
| `scan_work_inbox` | 수신함을 이름순 최대 50개 읽어 `InboxEntry[]`(`fileName`·`key`·`title`·`body`·`assignee?`) 반환. 거부 파일 이동과 오래된 임시 파일 정리를 함께 한다 |
| `export_work_items_file` | `defaultName`·`filterName`(현재 언어의 형식 이름)·`content`를 받아 저장 대화상자로 고른 경로에 쓴다. 저장한 경로, 취소 시 `null` |
| `import_work_items_file` | 열기 대화상자로 고른 JSON 파일의 텍스트, 취소 시 `null`. 파싱·검증은 렌더러가 한다 |
| `ack_work_inbox` | `fileNames`를 지운다. 경로 구분자·`..`·`.`으로 시작·`.md`가 아닌 이름은 무시하고, 이미 없으면 성공으로 본다 |
| `reject_work_inbox` | `rejections`(`fileName`·`reason`·`detail?`)를 `rejected/`로 옮기고 `.error`에 사유 코드와 detail을 쓴다. 이름 규칙은 ack와 같고, 코드는 `[a-z-]` 1~32자만 받으며, 이미 없는 파일은 건너뛴다 |
| `warn_work_inbox` | `warnings`(`fileName`·`taskId`·`code`·`detail`)를 `warnings.log`에 덧붙인다 |

기존 `dispose_session`의 선택적 `expectedSessionId`·`expectedHumanInputRevision`으로 확인했던 세션만 재시작한다. 업무 실행은 터미널 자동화의 결과 파일 감시와 CLI 종료 흐름을 사용하지 않는다.

## 검증 상태 (2026-10-13)

구현 완료 범위는 opt-in 설정·좌우 패널·업무 편집과 영속화·세션 요약과 검색·프로필 기본 에이전트·업무 보내기·실시간 모니터링과 자동 상태 전이다.

완료한 검증:

- 태스크 작성 플로우 변경: 새 작성 취소·접기/재개, 편집 초안의 카드 전환·패널 접기 보존, 모든 필드의 수정 취소, 빈 제목의 저장·복원·확인창/세션 검색, 본문 전체 전송, 편집 중 자동 상태 전이 보존을 회귀 검사했다. `npx tsc --noEmit`, 프런트 전체 2,926개, 네이티브 전체 1,675개, `npm run build` 통과. Safari의 독립 로컬 프리뷰에서 기본 280px 폭의 연한 제목·본문 placeholder·추가/저장·취소·접기 한 줄 배치와 취소/재개 동작을 확인했다. 합성 데이터·IPC만 사용했다.

- `npx tsc --noEmit`, `npx vitest run --dir src`, `cargo test --manifest-path src-tauri/Cargo.toml`, `npm run build`: 타입 검사·프런트 전체 회귀·네이티브 전체 회귀·프로덕션/웹 빌드 통과. UI 문자열은 i18n 테스트로 검사한다.
- `cargo test --manifest-path src-tauri/Cargo.toml work_send_multiline_in_real_pty_with_cli_double -- --ignored`: 실제 PTY에서 CLI 대역의 모델·effort 인자, 기본 10초 대기, Unicode·빈 줄·코드 블록 그대로 수신을 확인했다. 설치된 CLI나 LLM 계정은 사용하지 않는다.
- Safari 로컬 프리뷰에서 합성 데이터·IPC로 기본 설정 표시, 모델 입력 배치, 질문 강조·경과·턴·비용, 재시작 확인·미제출 경고·취소를 확인했다. 설치된 원본 앱은 교체하지 않았다.

- 업무 수신함(2026-10-13): 파서·scan·ack·`office-task`·세션 env·안내문 조합·권한 조각·Codex 배열 합치기·Kilo 훅 단위 테스트와 수신 루프(저장 실패 시 보류·건너뛴 파일 ack·정지 뒤 늦은 응답 폐기) 회귀를 추가했다. `npx tsc --noEmit`, 프런트 전체 2,936개, 네이티브 전체 1,653개 통과. `codex debug prompt-input`(모델 호출 없음)으로 `-c writable_roots`의 덮어쓰기 동작과 `developer_instructions` 주입을 확인했다.
- 에이전트 완료 처리(2026-10-13): 수신함 `done` 파싱, `office-task --done`(없는 id 거절·이미 완료 건너뜀·수신함 미처리 업무 허용), 같은 주기 추가 후 완료 적용·저장 실패 시 재수신 회귀를 추가했다. `npx tsc --noEmit`, 프런트 전체 2,945개, 네이티브 전체 1,662개 통과.
- 수신함 의미 오류(2026-10-14, 점검 F4): id 충돌(재수신과 구분)·없는 완료 대상 거부, 버린 선행·모르는 담당 경고 기록, `office-task --to`·`--after` 즉시 경고 회귀를 추가했다.
- 선행 작업·상세 팝업(2026-10-13): 선행 작업 정리(없는 id·자기 자신·중복·순환)·대기 판정·후보, 저장·삭제·수신함 연결, 보내기 차단, 팝업 편집·선행 작업 선택 UI 회귀와 `office-task --after`·`--list`·수신함 `id`/`after` 파싱 단위 테스트를 추가했다. `npx tsc --noEmit`, 프런트 전체 2,944개, 네이티브 전체 1,660개, `npm run build` 통과. Chrome의 독립 로컬 프리뷰(합성 데이터·IPC)에서 상세 팝업 배치, 선행 작업 표시·순환 후보 제외, 막힌 보내기를 확인했다.
- Windows 보내기(2026-10-13): Windows 11에서 셸 종류 판정·PowerShell 인용·경로 접두사 정리 단위 테스트, PowerShell 5.1·7과 Git Bash가 생성된 기동 스크립트를 실행해 ACK·반환 기록(BOM 없음, 종료 코드, 미존재 명령은 1)을 남기는지, 실제 PowerShell로 실행하는 기존 보내기 회귀(`work_send` 전체)를 확인했다. `cargo test --manifest-path src-tauri/Cargo.toml work_send_multiline_in_real_conpty_with_cli_double -- --ignored`: 실제 ConPTY에서 세 셸 모두 빈 셸 판정 → 작은따옴표 포함 모델 인자 기동 → ACK → 10초 대기 → 한글·이모지·탭·빈 줄·코드 블록 본문이 bracketed paste 한 번으로 그대로 도착함을 node 대역(raw 모드, bracketed paste 켬)으로 확인했다. 설치된 CLI나 LLM 계정은 사용하지 않는다.
- 내보내기·가져오기(2026-10-13): 상태 선택 직렬화·왕복 보존·손으로 고친 파일 허용·형식 오류, 새 id 추가와 선행 작업 재연결·순환 제거·로드 실패 차단, 팝업의 상태 선택·취소·실패·성공 안내와 확인 후 추가 회귀를 추가했다. `npx tsc --noEmit`, 프런트 전체 2,954개, 네이티브 전체 1,662개 통과. 네이티브 대화상자는 테스트에서 대역으로 바꿨다.
- 담당 캐릭터 선택·대체(2026-10-13): 담당 키 묶음·담당 이름 왕복, 없는 캐릭터 수집·같은 이름 기본 대체·일괄 교체 단위 테스트, 가져온 업무에 이름을 저장하지 않는 회귀, 팝업에서 담당 체크로 고른 업무만 내보내고 다른 캐릭터 목록에서 대체를 골라 추가하는 회귀를 추가했다. `npx tsc --noEmit`, 프런트 전체 2,958개 통과.

남은 실기 인수:

- [ ] 설치 Tauri 앱에서 설정·프로필 기본 에이전트·업무 저장과 재실행 후 복원 확인.
- [ ] 설치 Tauri 앱의 Windows PowerShell·pwsh·Git Bash 세션에서 인증된 Claude·Codex로 보내기(새 세션·기존 빈 셸·CLI 실행 중 재시작 확인) 확인. 관찰 래퍼(`-EncodedCommand`)와 사용자 프로필이 있는 새 PowerShell이 10초 ACK 안에 기동 문장을 읽는지 확인.
- [ ] 인증된 실제 Claude·Codex·agy·Kilo·Pi에서 CLI 기동, 첫 프롬프트 제출, 질문·턴 종료·후속 프롬프트의 표시와 상태 전이 확인.
- [ ] 설치 앱(macOS·Windows)에서 내보내기 저장 대화상자의 형식 이름·기본 파일명·취소, 내보낸 파일을 편집기로 고친 뒤 가져오기 확인. 다른 컴퓨터(캐릭터 id가 다른 앱)에서 가져와 없는 캐릭터 대체 선택과 팝업 배치 확인.
- [ ] 설치 앱에서 외부 셸로 수신함에 파일을 넣어 업무 카드가 생기고 파일이 지워지는지, 패널을 끈 상태에서 쌓인 파일이 켤 때 들어오는지 확인.
- [ ] 인증된 Claude 세션에서 "이 작업을 단계별 태스크로 나눠 줘"에 `--after`로 이어진 태스크가 생기고, "그 태스크 완료해 줘"에 `--done`이 실행돼 후속 태스크의 대기가 풀리는지, 지시 없이 스스로 완료하지 않는지 확인.
- [ ] 인증된 Claude 세션에서 "다음 할 일로 남겨 줘"에 안내문대로 `office-task`가 권한 확인 없이 실행되고 본인 캐릭터에게 배정되는지, persona가 있는 캐릭터에서 말투가 유지되는지 확인.
- [ ] Codex 세션에서 같은 요청이 샌드박스 승인 없이 들어오는지 확인.
- [ ] Pi·Kilo에서 안내문 주입, agy에서 env 폴백(AGENTS.md 안내)으로 동작하는지와 각 CLI의 실행 승인 동작 확인.

## 알려진 제한

- 기동 ACK와 기본 10초 대기는 CLI 인증·승인·TUI 준비를 증명하지 않는다. 실제 CLI 준비 상태는 위 실기 인수에서 확인해야 한다.
- 외부 CLI의 자연 종료와 PTY 쓰기를 완전히 원자화하는 보장은 하지 않는다.
- 렌더러 reload/crash 뒤 보내기를 자동 재개·재전송하지 않는다. 이미 제출했지만 IPC 응답을 렌더러가 받기 전에 사라진 경우 업무 연결은 남지 않을 수 있으며 실제 세션 기록은 별도로 수집된다.

## 범위 밖

현재 제공하지 않는 기능:

- 턴 종료만으로 자동 완료 처리. 완료는 사람이 정한다(직접 바꾸거나 에이전트에게 지시).
- 세션 종료 시 LLM 요약 생성(크레딧 소모).
- 선행 작업이 끝나면 후속 업무를 자동으로 보내는 큐, 하위 업무(계층), 마감일.
- 외부 이슈 트래커(Gitea·kbm) 동기화.
- 우측 로그 항목 편집·삭제, 내보내기.
- 마스코트 창에 패널 노출.
- 수신함으로 기존 업무의 완료 외 갱신(제목·본문·담당·상태 변경), 출처(`source`) 표시, 도착 알림(토스트·말풍선). 사용자가 직접 치는 `/task` 같은 CLI별 명령·스킬.
