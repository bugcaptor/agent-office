# 태스크 대화·ID 복사 설계

상태: 계획 (2026-10-27) — 구현 전. 구현을 마치면 이 문서를 "현재 구조" 매뉴얼로 고쳐 정본으로 올린다.

자동 진행 보고를 받고도 태스크가 어떤 상태인지 이해가 안 될 때, 태스크 상세 팝업에서 바로 **그 태스크를 주제로 한 대화 세션**을 연다. 세션은 캐릭터에 묶이지 않은 앱 안 터미널이라 진행자와 판단자를 건드리지 않고, 자동 진행 중에도 열 수 있다. 수동으로 대화할 때 태스크를 가리킬 수 있게 **태스크 ID 복사**도 함께 둔다.

관련 문서: [work-panels-design.md](work-panels-design.md)(상세 팝업·보내기·`office-task`), [auto-run-design.md](auto-run-design.md)(진행자·판단자·작업자 보고·기록), [session-log-design.md](session-log-design.md)(세션별 전사 파일), [subsystem-c-ui.md](subsystem-c-ui.md)(터미널 오버레이·탭).

## 결정 (2026-10-27 사용자 확인)

| 항목 | 결정 |
|---|---|
| 대화 상대 | 새 CLI 세션. 진행자 세션·판단자 세션에 끼어들지 않는다 |
| 세션 위치 | 앱 안의 **캐릭터 없는 터미널**(대화 탭). 같은 캐릭터 터미널·외부 터미널은 쓰지 않는다 |
| ID 복사 | 상세 팝업 머리에 ID와 복사 버튼. ID만 복사한다 |

## 왜 이 구조인가

- **같은 캐릭터를 쓰지 않는다.** 세션은 캐릭터마다 하나다(`appStore.sessions`, 네이티브 `SessionManager.sessions` 모두 agentId 키). 같은 캐릭터로 대화를 열면 지금 터미널을 갈아치운다. 자동 진행 중이면 진행자 세션이 죽고, 일시정지 중이어도 현재 태스크 세션이 사라져 계속을 누르면 `session-ended`로 멈춘다. 이 기능이 가장 필요한 순간이 바로 자동 진행 중이라 같은 캐릭터는 맞지 않는다.
- **진행자 터미널에 직접 묻지 않는다.** 그 턴이 끝나면 판단자가 내 질문의 답을 작업 결과로 읽는다.
- **판단자 세션을 갈라 쓰지 않는다**(`--fork-session`). 판단자는 보고와 응답 텍스트만 봤고 코드를 모른다. 필요하면 나중에 따로 더한다(아래 '하지 않는 것').
- **외부 터미널을 쓰지 않는다.** 오피스에서 보이지 않고, AppleScript·Windows Terminal별로 긴 명령 인용을 새로 다뤄야 한다. 앱 안 터미널은 기존 보내기 경로(기동 ACK·bracketed paste)를 그대로 쓸 수 있다.
- **네이티브는 이미 캐릭터 없이도 돈다.** `create_session`은 agentId 문자열만 받고 프로필을 찾지 않는다. 캐릭터에 묶인 것은 렌더러 쪽(`openTerminal`이 `agents`에 없는 id를 거절, 탭 줄이 `recentAgentIds`만 그림)과 관찰 훅이다.

## UI

### ID 복사

- 기존 업무의 상세 팝업 머리(`WorkDialog.tsx`의 `work-dialog-header`)에서 제목 옆에 ID를 고정폭 글꼴로 연하게 보이고, 그 옆에 복사 버튼을 둔다. 새 업무 초안에는 ID가 없으므로 보이지 않는다.
- 누르면 `navigator.clipboard.writeText(id)`로 ID만 복사하고, 버튼 툴팁·아이콘이 1.5초 동안 '복사됨'으로 바뀐다. 실패하면 '복사 못 함'을 같은 자리에 보인다(`memoStore`의 클립보드 가드와 같은 방식).
- ID 글자도 선택해서 복사할 수 있게 둔다(`user-select: text`).
- 자동 진행 기록(`AutoRunSection.tsx`)에서 태스크 제목을 누르면 그 태스크 팝업이 열리므로, 기록에 ID를 따로 보이지는 않는다.
- 구현됨: `WorkDialog`의 `id` 속성(`WorkItemsPanel`이 기존 업무에만 넘긴다). 화면 확인은 `npm run gui:shot -- --scenario workpanels --script scripts/gui-steps/work-id-copy.mjs`.

### 대화 열기

- 상세 팝업 실행 줄에 '대화' 버튼을 둔다(`터미널` 왼쪽). 기존 업무면 상태와 상관없이 누를 수 있고, 미저장 변경이 있으면 저장된 내용 기준으로 연다는 툴팁을 단다.
- 루트 팝업의 자동 진행 영역에서 현재 태스크(`currentId`)가 있으면 '현재 태스크에 대해 대화' 버튼을 둔다. 일시정지 사유를 읽고 바로 묻는 흐름이다.
- 누르면 팝업을 닫고(초안 유지) 대화 탭을 연다. 그 태스크의 대화 세션이 이미 살아 있으면 새로 만들지 않고 그 탭으로 간다.
- 막는 경우: 지원 셸이 아님(보내기와 같은 판정), 대화 세션 상한 초과(아래). 이유는 버튼 아래 문구로 보인다.
- 구현됨(`TaskChatButton.tsx`의 `useTaskChatButton`, `WorkItemsPanel.tsx` 실행 줄, `AutoRunSection.tsx`): 실행 줄은 `삭제 … 완료로 표시 · 대화 · 터미널 · 보내기` 순이다. 막힘 판정은 `taskChatBlockedReason` 하나로 한다 — 이미 살아 있는 대화면 막지 않고, 여는 중·상한(`task-chat-limit`, 가장 오래 안 본 대화 제목을 넣은 문구)·셸 순이다. 셸은 렌더러가 담당 프로필 셸 id만 알므로 `wsl`만 미리 `work-send-shell-unsupported`로 막고, 기본 셸이 bash/zsh가 아닌 경우(예: fish)는 네이티브 `open_task_chat`이 같은 코드로 거절해 버튼 아래 오류 문구로 보인다. 팝업은 열기 결과를 받은 뒤에 닫는다(실패하면 열어 두어 오류를 보인다, 보내기와 같은 흐름). 여는 동안 버튼 이름이 '여는 중…'으로 바뀐다. 기다리는 사이 다른 팝업으로 옮겼으면 그 팝업은 닫지 않는다. 루트 팝업의 '현재 태스크에 대해 대화'는 미저장 툴팁을 달지 않는다(현재 태스크의 초안은 그 태스크 팝업에만 있다). 화면 확인: `scripts/gui-steps/task-chat-buttons.mjs`(`--scenario workpanels`).

### 대화 탭

- 터미널 오버레이의 탭 줄에서 캐릭터 탭 뒤에 대화 탭을 따로 묶어 둔다. 얼굴 대신 말풍선 아이콘, 이름 대신 태스크 표시 제목(말줄임)을 쓴다. 툴팁에 태스크 ID와 작업 폴더를 보인다.
- 캐릭터 전용 메뉴(퇴근·이어하기·VS Code·작업 폴더·일기·메모·실행 레시피)는 대화 탭에서 숨긴다. 출력 내보내기·뷰 모드는 그대로 쓴다.
- 대화 탭에는 닫기(×)가 있다. CLI가 돌고 있으면 "대화를 끝낸다"는 확인 뒤 세션을 버린다. 캐릭터 탭과 달리 닫으면 세션도 끝난다.
- 오피스 씬에는 아무것도 그리지 않는다(캐릭터·미니미·머리 위 라벨 없음).
- 구현됨(`taskChatStore.ts`, `AgentTabStrip.tsx`, `TerminalHost.tsx`): 탭 목록·세션 상태는 `useTaskChatStore`(`chats`·`order`·`opening`)만 쥐고 `appStore.sessions`·`recentAgentIds`에는 넣지 않는다. `appStore.openTerminal`은 `task-chat-` id를 받으면 활성만 바꾼다(TopBar 대상·캐릭터 탭 순서가 그대로다). 탭 이름은 태스크의 지금 표시 제목(지워졌으면 연 시점 제목), 툴팁은 `태스크 <id>\n작업 폴더: <cwd|홈 폴더>`. 우클릭 메뉴는 출력 내보내기와 대화 끝내기뿐이고, 활성 탭이 대화면 포스트잇 버튼과 위젯을 숨기며 문서 버튼은 대화의 작업 폴더로 연다. Cmd/Ctrl+숫자는 캐릭터 탭 다음 번호로 대화 탭에 간다. 닫기 확인은 `confirm-close-task-chat` 모달(세션이 `starting`·`running`일 때만 — 셸 안 CLI가 끝났는지는 관찰을 껐으므로 따로 보지 않는다), 확인하면 `disposeSession` 뒤 이웃 대화 탭 → 최근 캐릭터 탭 → 오버레이 닫기 순으로 옮긴다. 끝난 대화의 터미널에는 '다시 띄우기' 없이 안내만 보인다(다시 열면 맥락 묶음을 새로 만들어야 하므로 태스크에서 연다).
- 구현됨(열기 `openTaskChat(taskId)`): 이미 살아 있으면 그 탭으로 간다. 아니면 렌더러 상한(`MAX_TASK_CHATS = 4`, `starting`·`running`만 센다)·여는 중을 먼저 막고 오류 코드를 돌려준다(`taskChatErrorText`가 문구로 바꾸고, 상한이면 가장 오래 안 본 대화 제목을 넣는다 — 본 시각은 활성 탭이 바뀔 때 갱신). 첫 크기는 지금 보이는 터미널 크기를 빌린다. IPC 전에 `terminalRegistry.ensure`로 출력을 구독해 첫 화면을 놓치지 않고, 실패하면 그 터미널을 버린다. 끝난 대화를 다시 열면 이전 화면을 버리고 새로 시작한다. 태스크 5는 `taskChatBlockedReason`·`openTaskChat`·`taskChatErrorText`를 쓰면 된다.

## 세션

### 식별자

- agentId 자리에 `task-chat-<taskId>`를 쓴다. 태스크 id는 nanoid(`[A-Za-z0-9_-]`)라 세션 로그 경로 검증(`diary_store.rs` 규약)을 통과한다. 프로필 id는 `nanoid()`(21자)라 이 접두사와 겹치지 않는다(아래 '열린 질문'). 프로필 생성 쪽에 따로 막는 장치는 두지 않고, 네이티브가 `taskChat` 옵션과 접두사가 어긋난 요청을 `task-chat-id-mismatch`로 거절한다. 그래서 핸드오프 제외·셸 작업중 감시 끄기는 접두사(`is_task_chat_id`)만 보고 정한다.
- 태스크 하나에 대화 세션 하나. 동시에 살아 있는 대화 세션은 4개까지로 한다(PTY·스레드 수 보호). 넘으면 가장 오래 안 본 대화를 닫으라고 안내한다.

### 관찰을 끈다

대화 세션은 업무 상태·통계에 섞이면 안 된다. 그래서 네이티브 세션 생성에 `SessionOpts.taskChat: true`를 더해 다음을 바꾼다.

- **관찰 훅 주입을 하지 않는다.** 셸 래퍼가 `claude`에 붙이는 `--settings $AGENT_OFFICE_SETTINGS`(훅)와 Codex·Pi·Kilo 훅 배선을 대화 세션에서는 빼고, 대신 훅 없이 권한 조각만 담은 대화용 설정 파일을 붙인다(아래 '권한'). 훅 이벤트가 없으므로 턴 시간·알림·세션 요약·분석·일기·마스코트 어디에도 들어가지 않는다. 셸 작업중 감지(`shell_activity`)도 끈다.
- **`AGENT_OFFICE_AGENT_ID`를 넣지 않는다.** `office-task`로 만든 업무가 가짜 담당으로 들어가지 않게 미배정으로 둔다. shim(`office-task`, `office-md`)과 PATH는 그대로 두어 `office-task --show`를 쓸 수 있게 한다.
- **페르소나 프롬프트를 넣지 않는다.** 사무실 공통 안내문(`office-task`·`office-md` 사용법)은 넣는다.
- 구현됨(`prepare_session_plan`의 `task_chat`): 관찰 URL을 없는 것으로 보고 준비하므로 Claude·Codex 훅, Pi 확장(`-e`), Kilo 플러그인, agy 래퍼, `AGENT_OFFICE_HOOK_URL`이 모두 빠진다(URL이 없으면 forwarder가 아무것도 보내지 않으므로 전역 agy 훅도 조용하다). `AGENT_OFFICE_SESSION`·`AGENT_OFFICE_APP_DATA`·`AGENT_OFFICE_WORK_INBOX`·`AGENT_OFFICE_TASK`·`AGENT_OFFICE_MD`·PATH는 그대로다. 공통 안내문은 Claude(`--append-system-prompt`)·Codex(`developer_instructions`)·Pi(`--append-system-prompt`)에 그대로 실린다.
- 세션 로그(PTY 전사)는 남긴다. `session-logs/v1/task-chat-<taskId>/`에 쌓여 30일 규칙으로 지워진다. 세션 로그 보기의 캐릭터 목록에는 나오지 않는다.

렌더러도 방어한다. `task-chat-` 접두사의 세션 이벤트는 캐릭터 중심 스토어(턴 시간·알림·요약 수집·사용량)에서 무시한다. 훅이 꺼져 있어도 세션 상태 이벤트(`session-state`)는 오기 때문이다.

- 구현됨: `isTaskChatId`(`src/shared/types/session.ts`)로 `sessionBridge`의 상태·알림·알림 지움·사용량·활동 다섯 갈래(오피스 씬 릴레이·배지·OS 알림 포함), `workMonitor`의 `observe`, `sessionSummaryStore`의 세션 상태 수집이 처음부터 버린다. 대화 세션 상태는 `installTaskChatTracking`(부팅 시 설치)이 따로 받아 `taskChatStore`에 반영한다(`disposed`는 `exited`로 흡수, 다른 sessionId의 늦은 이벤트는 무시).

### 앱 종료와 재실행

대화 세션은 핸드오프하지 않는다. 앱을 끄면 `handoff_all`에서 `task-chat-` 세션을 빼고 버린다(구현됨: 넘기기 전에 `dispose`, Windows의 no-op 경로도 같다). 대화는 일회성이고, 입양 경로(`adoptDetachedSessions`)가 캐릭터 없는 세션을 받을 준비가 없기 때문이다. 종료 확인창에 살아 있는 대화 수를 함께 알린다.

- 구현됨: `ConfirmQuitDialog`가 살아 있는 대화 수를 한 줄로 알린다. 근무 중인 캐릭터가 없어도 살아 있는 대화가 있으면 `quitGuard`가 확인창을 띄우고, 그때는 캐릭터 문구 없이 대화 안내만 보인다.

### 기동

- 작업 폴더: 그 태스크의 최신 연결 세션 요약의 `cwd` → 담당 캐릭터 프로필 `cwd` → 홈 폴더 순.
- CLI·모델·노력: 태스크의 에이전트 지정 → 담당 프로필 `defaultAgent` → Claude 기본값. 보내기와 같은 해석이다.
- 기존 보내기 네이티브 경로(`WorkSendRuntime`: 새 PTY의 기동 ACK 확인 → 기동 대기 → bracketed paste + CR)를 대화 세션 id로 그대로 쓴다. 렌더러는 성공해도 업무의 `sessionIds`·상태를 바꾸지 않는다. 새 IPC `open_task_chat({taskId, cwd, agent, prompt})`가 세션 생성과 첫 프롬프트 제출을 한 번에 하고 `{agentId, sessionId}`를 돌려준다.
- 자동 진행의 진행자 차단(`work-send-auto-run`)은 진행자 agentId 기준이라 대화 세션에는 걸리지 않는다.
- 구현됨(`task_chat::open`, IPC `open_task_chat({request})`): 요청은 `TaskChatRequest{taskId, cwd?, agent, shell?, cols?, rows?, context}`이고 `context`가 렌더러 스냅샷(아래 맥락 묶음의 2~4번 재료와 `sessions[{sessionId, agentId, sentAt}]`)이다. 모르는 칸은 거절한다(`deny_unknown_fields`). 검증 실패는 `task-chat-invalid-task`·`task-chat-invalid-cwd`(상대 경로)·`work-send-model-invalid`. 같은 태스크의 열기가 겹치면 `task-chat-busy`, 그 대화 세션이 이미 돌고 있으면 `task-chat-running`, 살아 있는 대화가 4개면 `task-chat-limit`로 거절한다(렌더러가 먼저 막고 네이티브는 방어선). 세션은 `taskChat: true`·persona 없이 만들고, 기동 ACK·첫 프롬프트 중 하나라도 실패하면 **만든 세션을 버리고** 오류를 돌려준다(렌더러에는 아직 탭이 없다). 렌더러는 결과를 받은 뒤에 탭을 연다.

## 첫 프롬프트 — 맥락 묶음

앱이 맥락을 모아 첫 프롬프트에 바로 넣는다. 파일을 따로 읽게 하면 Claude가 작업 폴더 밖 읽기 권한을 묻기 때문이다. 전체 상한은 보내기와 같은 256 KiB이고, 넘치면 아래 순서의 뒤쪽 칸부터 줄인다.

1. **역할과 규칙**: 이 세션은 태스크 하나에 대해 사람과 이야기하는 질의응답용이다. 사람이 시키지 않으면 파일을 고치거나, 커밋하거나, `office-task`로 상태를 바꾸지 않는다. 먼저 아래 맥락을 읽고 지금 상태를 쉬운 말로 요약한 뒤 질문을 기다린다.
2. **태스크**: ID, 제목, 상태, 담당, 에이전트, 본문, 선행 작업·후속 작업(제목과 상태).
3. **플랜**: 이 태스크가 속한 자동 진행의 루트 제목·본문, 실행 상태(진행 중·일시정지·끝남), 일시정지 사유 코드와 `pauseDetail`(판단자 메시지).
4. **자동 진행 기록**: 루트 `autoRun.log`에서 이 태스크의 항목(send·followup 지시문·next 인계 메모·human-done)을 시간순으로. 앞 태스크가 남긴 인계 메모 중 이 태스크에 넘어온 것.
5. **작업자 보고**: `work-inbox/reports/<id>.md` 최신본(`since` 없이 읽는다). 없으면 없다고 쓴다.
6. **세션 전사 위치**: 이 태스크의 `sessionIds` 각각에 대해 세션 로그 파일 절대 경로(`session-logs/v1/<agentId>/*-<sessionId 앞 8자>.log`)와 시작 시각. 필요하면 읽으라고만 쓰고 내용은 넣지 않는다(크다).
7. **최근 커밋**: 작업 폴더가 Git 저장소면 태스크를 처음 보낸 시각 이후의 `git log --oneline` 최대 30줄.

모으는 일은 네이티브가 한다(`task_chat.rs`의 `build_context`). 렌더러가 태스크·루트 스냅샷을 넘기고, 네이티브가 보고 파일·세션 로그 경로·Git 기록을 붙인다. 프롬프트 문구는 ko/en 두 벌이다(i18n 규약).

- 구현됨: 칸마다 `## 제목` 줄로 시작하고, 넘치면 7번부터 앞으로 칸 끝을 잘라 `…(길어서 잘림)`을 단다(칸 제목은 남긴다, 1번 역할은 자르지 않는다). 붙여넣기에 실을 수 없는 제어 문자는 빼고 `\r\n`은 `\n`으로 바꾼다. 문구는 설정 UI 언어(`i18n::ui_lang`)로 고르고 한국어 외에는 영어다.
- 5번은 `auto_run::read_report(…, since 0)`라 판단자와 같은 16,000자 상한이 걸린다. 6번은 `session_log::store::find_session_log`로 찾고 시작 시각은 로그 헤더(없으면 `sentAt`)다. 파일이 없으면 `로그 파일 없음 <agentId>: <sessionId>`로 적는다.
- 7번은 `sentAt` 중 가장 이른 값 이후(`git log --oneline -n 30 --since=@<초>`)다. 작업 폴더가 없거나, 보낸 적이 없거나, Git 저장소가 아니면(`rev-parse --is-inside-work-tree` 실패) 그렇다고 한 줄 적는다. git은 5초 시간 제한.

### 권한

- Claude: 대화용 설정 조각에 세션 로그 폴더와 `work-inbox/reports/` 읽기, shim 명령 사전 승인을 넣는다. 그래서 6번 경로를 권한 창 없이 읽는다. 쓰기 도구는 사전 승인하지 않는다(물으면 사람이 정한다). 훅은 없다.
  - 구현됨: `office_instructions::task_chat_permissions_fragment`가 `{"permissions":{"allow":["Bash(<shim>:*)", "Read(//<세션 로그 루트>/**)", "Read(//<work-inbox>/reports/**)"]}}`를 만들고, `lib.rs`의 공급자가 `observer/claude/<sid>.settings.json`(기존 GC 대상)에 쓴다. Claude 권한 규칙에서 `/x`는 설정 파일 기준 상대 경로라 절대 경로는 `//x`로, Windows 경로는 `//c/Users/...` 꼴로 쓴다(`claude_absolute_path`). shim 목록은 사무실 권한 조각과 같다(`office-task`는 업무 패널이 켜졌을 때만).
- Codex: 기본 샌드박스가 읽기는 어디든 허용하므로 따로 하지 않는다. `writable_roots`에 아무것도 더하지 않는다(수신함·문서 뷰어 캐시도 넣지 않는다 — Codex 대화에서 `office-task` 쓰기나 `office-md`를 쓰려면 샌드박스 승인을 거친다).

## 구현 위치 (계획)

| 역할 | 코드 |
|---|---|
| ID 표시·복사, 대화 버튼 | `src/renderer/workpanels/WorkDialog.tsx`, `WorkEditor.tsx`, `AutoRunSection.tsx` |
| 대화 세션 목록·열기·닫기 | 새 `src/renderer/workpanels/taskChatStore.ts` |
| 대화 탭 | `src/renderer/terminal/AgentTabStrip.tsx`, `TerminalHost.tsx`, `src/renderer/store/appStore.ts`(`openTerminal`이 대화 id를 받게) |
| 캐릭터 중심 스토어의 대화 이벤트 무시 | 세션 브리지·`workMonitor.ts`·`sessionSummaryStore.ts`·사용량 스토어 |
| 세션 생성 옵션(`taskChat`) | `src-tauri/src/session/manager.rs`(`prepare_session_plan`), `src-tauri/src/office_instructions.rs`, `src-tauri/src/lib.rs`(권한 조각) |
| 핸드오프 제외 | `SessionManager::handoff_all` |
| 맥락 묶음·IPC `open_task_chat` | 새 `src-tauri/src/task_chat.rs`, `src-tauri/src/ipc/commands/work_panels.rs`, `src/shared/ipc.ts` |
| 문구 | `src/shared/i18n/locales/` 6개 언어 |

## 검증 계획

- 프런트: ID 복사 성공·실패 표시, 대화 버튼 활성 조건, 같은 태스크 재열기는 기존 탭으로, 상한 초과 안내, 대화 탭 닫기 확인, `task-chat-` 이벤트가 요약·알림·턴 시간에 안 들어감, 자동 진행 중 대화를 열어도 실행 상태·진행자 세션이 그대로임.
- 네이티브: `taskChat` 옵션에서 훅 설정·`AGENT_OFFICE_AGENT_ID`·셸 작업중 감지가 빠짐, 맥락 묶음의 칸 순서와 상한 자르기, 보고 없음·Git 아님·세션 로그 없음, 핸드오프에서 대화 세션 제외.
- `npm run gui:shot`: 상세 팝업 머리의 ID·복사 표시, 대화 버튼, 탭 줄의 대화 탭(가짜 백엔드에 `openTaskChat` 스텁).
- 실기: 설치 앱에서 자동 진행 중 대화를 열어 질문하고, 진행자의 다음 판단이 영향받지 않음을 확인. Codex 태스크로도 한 번.

## 하지 않는 것

- 같은 캐릭터 터미널·외부 터미널에서 대화 열기.
- 판단자 세션을 `--fork-session`으로 갈라 대화하기. 필요해지면 대화 탭의 한 종류로 더한다(작업 폴더 `auto-run/judge`, `claude --resume <judgeSessionId> --fork-session`).
- 대화 내용을 태스크나 자동 진행 기록에 되돌려 붙이기. 사람이 정한 것을 반영하려면 루트 본문(플랜 메모)을 고치거나 태스크를 수정한다.
- 대화 세션의 앱 재시작 뒤 이어받기.

## 열린 질문 (2026-10-27 코드로 확인)

- **프로필 id 형식이 `task-chat-` 접두사와 겹칠 수 있는지** — 사실상 겹치지 않는다. 프로필 id는 `src/renderer/profile/generate.ts`의 `nanoid()`(21자, `[A-Za-z0-9_-]`)이고, 21자 nanoid가 10자 접두사 `task-chat-`로 시작할 확률은 64⁻¹⁰(약 10⁻¹⁸)이다. 다만 `profile_store.rs`는 id 형식을 검사하지 않아 손으로 고친 `profiles.json`이나 ctl `create`는 아무 id나 쓸 수 있다. 그래서 프로필 생성 쪽 장치 대신 네이티브가 옵션과 접두사가 어긋나면 세션 생성을 거절한다(위 '식별자').
- **셸 래퍼가 `AGENT_OFFICE_SETTINGS` 없이도 정상 동작하는지** — 그렇다. 래퍼는 고정 스크립트가 아니라 세션마다 `PreparedPlan.wrappers`(`CommandWrapperSpec`)에서 렌더되고, `claude --settings $AGENT_OFFICE_SETTINGS`는 그 목록에 그 항목이 있을 때만 생긴다. 관찰이 꺼진 세션이 이미 쓰는 경로(권한 전용 설정 파일을 `AGENT_OFFICE_SETTINGS`에 싣고, 파일이 사라지면 인자만 빼는 `skip_prefix_if_env_file_missing` 가드)가 있으므로, 대화 세션은 그 파일만 **대화용 설정 파일로 바꿔 끼운다**. 승인할 shim이 없어 파일을 못 만들면 claude 래퍼가 아예 없어 원본 `claude`가 뜬다.
- **Windows PowerShell·Git Bash에서 같은 방식으로 붙는지** — 같다. `shells::resolve_observed`가 PowerShell/pwsh(`-EncodedCommand` 함수, `Test-Path` 가드), Git Bash(`--rcfile`), zsh(ZDOTDIR) 모두 같은 래퍼 목록을 렌더한다(`wrapper_script.rs`의 `render_powershell`/`render_posix`). shim 경로와 PATH는 셸과 무관한 env다. WSL은 지금도 래퍼를 지원하지 않는다(보내기와 같은 판정으로 막는다). Windows에서 다른 점은 Claude 권한 규칙의 경로 꼴(`//c/...`)뿐이라 `claude_absolute_path`가 맞춘다. Windows 실기 확인은 아직 안 했다.
