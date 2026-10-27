# 태스크 대화·ID 복사

상태: 정본 (2026-10-27) — 구현 완료. 설치 앱에서 자동 진행 중 실제 CLI로 대화하는 실기 확인(Claude·Codex)과 Windows 실기는 남아 있다.

자동 진행 보고를 받고도 태스크가 어떤 상태인지 이해가 안 될 때, 태스크 상세 팝업에서 바로 **그 태스크를 주제로 한 대화 세션**을 연다. 세션은 캐릭터에 묶이지 않은 앱 안 터미널(대화 탭)이라 진행자와 판단자를 건드리지 않고, 자동 진행 중에도 열 수 있다. 수동으로 대화할 때 태스크를 가리킬 수 있게 상세 팝업 머리에서 **태스크 ID를 복사**할 수도 있다.

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
- **판단자 세션을 갈라 쓰지 않는다**(`--fork-session`). 판단자는 보고와 응답 텍스트만 봤고 코드를 모른다.
- **외부 터미널을 쓰지 않는다.** 오피스에서 보이지 않고, AppleScript·Windows Terminal별로 긴 명령 인용을 새로 다뤄야 한다. 앱 안 터미널은 기존 보내기 경로(기동 ACK·bracketed paste)를 그대로 쓴다.
- **네이티브는 캐릭터 없이도 돈다.** `create_session`은 agentId 문자열만 받고 프로필을 찾지 않는다. 캐릭터에 묶인 것은 렌더러 쪽(`appStore.openTerminal`, 탭 줄의 `recentAgentIds`)과 관찰 훅이라, 그 둘만 대화 세션을 따로 다룬다.

## UI

### ID 복사

- 기존 업무의 상세 팝업 머리(`WorkDialog.tsx`의 `work-dialog-header`)에서 제목 옆에 ID를 고정폭 글꼴로 연하게 보이고, 그 옆에 복사 버튼(⧉)을 둔다. `WorkDialog`의 선택 속성 `id`가 있을 때만 그린다. `WorkItemsPanel`은 기존 업무 팝업에만 `id`를 넘기고 새 초안·내보내기 팝업에는 넘기지 않는다. ID 영역은 머리 폭의 40%까지이고 넘치면 말줄임한다.
- 누르면 `navigator.clipboard.writeText(id)`로 ID만 복사하고, 버튼이 1.5초 동안 ✓·'복사됨'으로 바뀐다. 실패하면 같은 자리에 !·'복사 못 함'(`--accent-error`)을 보인다. 클립보드 가드는 `memoStore`와 같은 방식이지만 그쪽 함수가 export되지 않아 `WorkDialog.tsx` 안에 따로 둔다.
- ID 글자도 선택해서 복사할 수 있다(`user-select: text`).
- 자동 진행 기록(`AutoRunSection.tsx`)에서 태스크 제목을 누르면 그 태스크 팝업이 열리므로, 기록에 ID를 따로 보이지는 않는다.
- 문구 키: `workpanels.json`의 `copyId`·`idCopied`·`idCopyFailed`(6개 언어).

### 대화 열기

- **상세 팝업 실행 줄**(`WorkItemsPanel.tsx`): `삭제 … 완료로 표시 · 대화 · 터미널 · 보내기` 순이다. 기존 업무면 상태와 상관없이 누를 수 있고, 미저장 변경이 있으면 "저장된 내용으로 연다"는 툴팁을 단다. 대화는 저장본으로 열리고, 미저장 초안은 `WorkItemsPanel`의 `edits`에 그대로 남는다(같은 카드를 다시 열면 초안이 보인다).
- **루트 팝업**(`AutoRunSection.tsx`): 자동 진행에 현재 태스크(`currentId`)가 있으면 '현재 태스크에 대해 대화' 버튼을 둔다. 일시정지 사유를 읽고 바로 묻는 흐름이다. 현재 태스크의 초안은 그 태스크 팝업에만 있으므로 미저장 툴팁은 달지 않는다.
- 두 버튼 모두 `TaskChatButton.tsx`의 `useTaskChatButton`(막힘 판정, 대화 목록·여는 중 구독, 열기 결과 처리)을 쓴다. 막힘 판정은 `taskChatBlockedReason` 하나로 한다 — 이미 살아 있는 대화면 막지 않고(그 탭으로 간다), 그다음 여는 중·상한(`task-chat-limit`, 가장 오래 안 본 대화 제목을 넣은 문구)·셸 순이다. 막힘 사유와 열기 오류는 버튼 아래 `#work-chat-hint`에 보인다.
- 셸: 렌더러는 담당 프로필의 셸 id만 알므로 `wsl`만 미리 `work-send-shell-unsupported`로 막는다. 기본 셸이 bash/zsh가 아닌 경우(예: fish)는 네이티브 `open_task_chat`이 같은 코드로 거절해 오류 문구로 보인다.
- 팝업은 **열기 결과를 받은 뒤에** 닫는다. 실패하면 열어 둔 채 오류를 보인다(보내기와 같은 흐름). 여는 동안 버튼 이름이 '여는 중…'으로 바뀐다. 기다리는 사이 다른 팝업으로 옮겼으면 그 팝업은 닫지 않는다.

### 대화 탭

- 터미널 오버레이의 탭 줄(`AgentTabStrip.tsx`)에서 캐릭터 탭 뒤에 대화 탭을 따로 묶어 둔다. 얼굴 대신 말풍선(💬), 이름 대신 태스크의 지금 표시 제목(지워졌으면 연 시점 제목, 말줄임)을 쓴다. 툴팁은 `태스크 <id>` 줄과 `작업 폴더: <cwd|홈 폴더>` 줄이다. Cmd/Ctrl+숫자는 캐릭터 탭 다음 번호로 대화 탭에 간다.
- 우클릭 메뉴는 출력 내보내기와 대화 끝내기뿐이다(퇴근·이어하기·VS Code·작업 폴더·일기·메모·실행 레시피 없음). 활성 탭이 대화면 포스트잇 버튼과 위젯(`PostItWidget.tsx`)을 숨기고, 문서 버튼은 대화의 작업 폴더로 연다.
- 대화 탭에는 닫기(×)가 있다. 세션이 `starting`·`running`이면 `confirm-close-task-chat` 모달(`ConfirmCloseTaskChatDialog.tsx`)로 확인한 뒤 `disposeSession`한다. 셸 안 CLI가 끝났는지는 관찰을 껐으므로 따로 보지 않는다. 닫은 뒤에는 이웃 대화 탭 → 최근 캐릭터 탭 → 오버레이 닫기 순으로 옮긴다. 캐릭터 탭과 달리 **닫으면 세션도 끝난다**.
- 끝난 대화의 터미널(`TerminalHost.tsx`)에는 '다시 띄우기' 없이 안내만 보인다. 맥락 묶음을 새로 만들어야 하므로 태스크에서 다시 연다(이전 화면은 버린다).
- 오피스 씬에는 아무것도 그리지 않는다(캐릭터·미니미·머리 위 라벨 없음).

### 렌더러 상태 — `taskChatStore.ts`

- 탭 목록·세션 상태는 `useTaskChatStore`(`chats`·`order`·`opening`)만 쥔다. `appStore.sessions`·`recentAgentIds`에는 넣지 않으므로 오피스 씬·캐릭터 순회에 나타나지 않는다. `appStore.openTerminal`은 `task-chat-` id를 받으면 활성 탭만 바꾼다(TopBar 대상·캐릭터 탭 순서는 그대로).
- `openTaskChat(taskId)`: 이미 살아 있으면 그 탭으로 간다. 아니면 렌더러 상한(`MAX_TASK_CHATS = 4`, `starting`·`running`만 센다)·여는 중을 먼저 막고 오류 코드를 돌려준다(`taskChatErrorText`가 문구로 바꾼다. 본 시각은 활성 탭이 바뀔 때 갱신). 요청 재료는 `resolveTaskChatCwd`(작업 폴더)·`buildTaskChatContext`(태스크·플랜 스냅샷)로 만든다. 첫 크기는 지금 보이는 터미널 크기를 빌린다. IPC 전에 `terminalRegistry.ensure`로 출력을 구독해 첫 화면을 놓치지 않고, 실패하면 그 터미널을 버린다. 결과를 받은 뒤에 탭을 연다.
- `requestCloseTaskChat`/`closeTaskChat`: 위 닫기 흐름.
- `installTaskChatTracking`(부팅 때 `bootstrap.ts`가 설치): 대화 세션의 상태 이벤트만 받아 반영한다. `disposed`는 `exited`로 흡수하고, 다른 sessionId의 늦은 이벤트는 무시한다.
- 업무의 `sessionIds`·상태와 자동 진행 실행 상태는 바꾸지 않는다.

## 세션

### 식별자

- agentId 자리에 `task-chat-<taskId>`를 쓴다(`TASK_CHAT_PREFIX`·`isTaskChatId` — TS `src/shared/types/session.ts`, Rust `types.rs`의 `is_task_chat_id`). 태스크 id는 nanoid(`[A-Za-z0-9_-]`)라 세션 로그 경로 검증(`diary_store.rs` 규약)을 통과한다.
- 프로필 id는 `src/renderer/profile/generate.ts`의 `nanoid()`(21자)라 이 접두사로 시작할 확률이 64⁻¹⁰(약 10⁻¹⁸)이다. 다만 `profile_store.rs`는 id 형식을 검사하지 않아 손으로 고친 `profiles.json`이나 ctl `create`는 아무 id나 쓸 수 있다. 그래서 네이티브가 `taskChat` 옵션과 접두사가 어긋난 세션 생성을 `task-chat-id-mismatch`로 거절하고, 핸드오프 제외·셸 작업중 감시 끄기는 접두사만 보고 정한다.
- 태스크 하나에 대화 세션 하나. 동시에 살아 있는 대화 세션은 4개까지다(PTY·스레드 수 보호, `task_chat::MAX_TASK_CHATS`).

### 관찰을 끈다

대화 세션은 업무 상태·통계에 섞이면 안 된다. 세션 생성 옵션 `taskChat`(TS `CreateSessionOptions.taskChat`, Rust `CreateSessionRequest.task_chat`, 기본 false)이 켜지면 `SessionManager::prepare_session_plan`이 관찰 URL을 없는 것으로 보고 준비한다.

- 빠지는 것: Claude·Codex 훅, Pi 확장(`-e`), Kilo 플러그인, agy 래퍼, `AGENT_OFFICE_HOOK_URL`, `AGENT_OFFICE_AGENT_ID`(그래서 `office-task`로 만든 업무는 미배정), 페르소나 프롬프트, 셸 작업중 감시(`shell_activity`), Codex `writable_roots` 추가. URL이 없으면 forwarder가 아무것도 보내지 않으므로 전역 agy 훅도 조용하다. 훅 이벤트가 없으니 턴 시간·알림·세션 요약·분석·일기·마스코트 어디에도 들어가지 않는다.
- 남는 것: `AGENT_OFFICE_SESSION`·`AGENT_OFFICE_APP_DATA`·`AGENT_OFFICE_WORK_INBOX`·`AGENT_OFFICE_TASK`·`AGENT_OFFICE_MD`·PATH(shim `office-task`·`office-md`), 사무실 공통 안내문(Claude·Pi `--append-system-prompt`, Codex `developer_instructions`).
- 셸 래퍼는 세션마다 `PreparedPlan.wrappers`에서 렌더되고(`wrapper_script.rs`의 `render_posix`/`render_powershell`), `claude --settings $AGENT_OFFICE_SETTINGS`는 그 목록에 항목이 있을 때만 생긴다. 대화 세션은 관찰이 꺼진 세션의 권한 전용 설정 파일 자리에 **대화용 설정 파일**을 끼운다(파일이 사라지면 인자만 빼는 `skip_prefix_if_env_file_missing` 가드 그대로). 승인할 shim이 없어 파일을 못 만들면 claude 래퍼가 없어 원본 `claude`가 뜬다. PowerShell/pwsh·Git Bash·zsh 모두 같은 래퍼 목록을 렌더하고, WSL은 래퍼를 지원하지 않아 보내기와 같은 판정으로 막는다.
- 세션 로그(PTY 전사)는 남긴다. `session-logs/v1/task-chat-<taskId>/`에 쌓여 30일 규칙으로 지워진다. 세션 로그 보기의 캐릭터 목록에는 나오지 않는다.

렌더러도 방어한다. 훅이 꺼져 있어도 세션 상태 이벤트(`session-state`)는 오므로, `isTaskChatId`로 `sessionBridge.ts`의 상태·알림·알림 지움·사용량·활동 다섯 갈래(오피스 씬 릴레이·배지·OS 알림 포함), `workMonitor.ts`의 `observe`, `sessionSummaryStore.ts`의 세션 상태 수집이 처음부터 버린다.

### 권한

- **Claude**: `office_instructions::task_chat_permissions_fragment`가 `{"permissions":{"allow":["Bash(<shim>:*)", "Read(//<세션 로그 루트>/**)", "Read(//<work-inbox>/reports/**)"]}}`를 만들고, `lib.rs`의 대화 설정 공급자(`SessionManager::with_task_chat_settings`)가 `observer/claude/<sid>.settings.json`(기존 GC 대상)에 쓴다. 그래서 맥락 묶음 6번의 전사 경로를 권한 창 없이 읽는다. 쓰기 도구는 사전 승인하지 않고 훅도 없다. Claude 권한 규칙에서 `/x`는 설정 파일 기준 상대 경로라 절대 경로는 `//x`로, Windows 경로는 `//c/Users/...` 꼴로 쓴다(`claude_absolute_path`). shim 목록은 사무실 권한 조각과 같다(`office-task`는 업무 패널이 켜졌을 때만).
- **Codex**: 기본 샌드박스가 읽기는 어디든 허용하므로 따로 하지 않는다. `writable_roots`에 아무것도 더하지 않으므로 Codex 대화에서 `office-task` 쓰기나 `office-md`를 쓰려면 샌드박스 승인을 거친다.

### 열기 — IPC `open_task_chat`

`task_chat::open`(IPC `open_task_chat({request})`, `ipc/commands/work_panels.rs`, TS `api.openTaskChat`·`Commands.openTaskChat`)이 세션 생성과 첫 프롬프트 제출을 한 번에 하고 `TaskChatResult{agentId: task-chat-<taskId>, sessionId}`를 돌려준다.

- 요청 `TaskChatRequest{taskId, cwd?, agent, shell?, cols?, rows?, context}`(TS `src/shared/types/workPanels.ts`, Rust `types.rs`). `context`는 원문 프롬프트가 아니라 렌더러 스냅샷 `TaskChatContext{task, plan?}`이다 — 태스크(제목·상태·담당·본문·선행·후속·`sessions[{sessionId, agentId, sentAt}]`)와 플랜(루트 제목·본문·실행 상태·일시정지 사유·`pauseDetail`·이 태스크의 기록·넘어온 인계 메모). 모르는 칸은 거절한다(`deny_unknown_fields`). 계약 픽스처는 `src/shared/contract-fixtures/open-task-chat-request.json`·`open-task-chat-result.json`.
- 렌더러가 채우는 값: 작업 폴더(그 태스크의 최신 연결 세션 요약 `cwd` → 담당 프로필 `cwd` → 홈, 절대 경로만), 셸·크기, 해석이 끝난 실제 실행 agent(태스크의 에이전트 지정 → 담당 프로필 `defaultAgent` → Claude, 보내기와 같은 해석), `sentAt`(세션 요약의 `startedAt` — 보낸 시각을 따로 저장하지 않아 Git 기록 범위가 조금 넓을 수 있다), 인계 메모(처음 보내기 전의 다른 태스크 `next` 항목, 최대 20개).
- 거절: `task-chat-invalid-task`·`task-chat-invalid-cwd`(상대 경로)·`work-send-model-invalid`, 같은 태스크의 열기가 겹치면 `task-chat-busy`, 이미 살아 있으면 `task-chat-running`, 살아 있는 대화가 4개면 `task-chat-limit`. 렌더러가 먼저 막고 네이티브는 방어선이다.
- 세션은 `taskChat: true`·persona 없이 `spawn_session`으로 만들고, 기존 보내기 경로(`WorkSendRuntime`: 새 PTY의 기동 ACK 확인 → 기동 대기 → bracketed paste + CR)로 맥락 묶음을 넣는다. 세션 생성·기동 ACK·첫 프롬프트 중 하나라도 실패하면 **만든 세션을 버리고** 오류를 돌려준다(렌더러에는 아직 탭이 없다).
- 자동 진행의 진행자 차단(`work-send-auto-run`)은 진행자 agentId 기준이라 대화 세션에는 걸리지 않는다. 진행자·판단자는 대화 세션의 존재를 모른다.

### 첫 프롬프트 — 맥락 묶음

앱이 맥락을 모아 첫 프롬프트에 바로 넣는다(파일을 따로 읽게 하면 Claude가 작업 폴더 밖 읽기 권한을 묻는다). `task_chat::build_context`가 칸마다 `## 제목` 줄로 시작해 아래 순서로 붙인다. 문구는 설정 UI 언어(`i18n::ui_lang`)로 고르고 한국어 외에는 영어다.

1. **역할과 규칙**: 태스크 하나에 대한 질의응답용이다. 사람이 시키지 않으면 파일을 고치거나, 커밋하거나, `office-task`로 상태를 바꾸지 않는다. 먼저 맥락을 읽고 지금 상태를 쉬운 말로 요약한 뒤 질문을 기다린다.
2. **태스크**: ID, 제목, 상태, 담당, 에이전트, 본문, 선행 작업·후속 작업(제목과 상태).
3. **플랜**: 루트 제목·본문(플랜 메모), 실행 상태, 일시정지 사유 코드와 판단자 메시지. 플랜에 속하지 않으면 그렇다고 쓴다.
4. **자동 진행 기록**: 이 태스크의 send·followup·next·human-done 항목을 시간순으로, 앞 태스크에서 넘어온 인계 메모.
5. **작업자 보고**: `auto_run::read_report(…, since 0)`로 `work-inbox/reports/<id>.md` 최신본(판단자와 같은 16,000자 상한). 없으면 없다고 쓴다.
6. **세션 전사 위치**: `sessions` 각각의 세션 로그 절대 경로(`session_log::store::find_session_log`)와 시작 시각(로그 헤더, 없으면 `sentAt`). 내용은 넣지 않고 필요하면 읽으라고만 쓴다. 파일이 없으면 `로그 파일 없음 <agentId>: <sessionId>`.
7. **최근 커밋**: 가장 이른 `sentAt` 이후 `git log --oneline -n 30 --since=@<초>`(5초 시간 제한). 작업 폴더가 없거나, 보낸 적이 없거나, Git 저장소가 아니면 그렇다고 한 줄 쓴다.

전체 상한은 보내기와 같은 256 KiB다. 넘치면 7번부터 앞으로 칸 끝을 잘라 `…(길어서 잘림)`을 단다(칸 제목은 남기고 1번은 자르지 않는다). 붙여넣기에 실을 수 없는 제어 문자는 빼고 `\r\n`은 `\n`으로 바꾼다.

### 앱 종료와 재실행

- 대화 세션은 핸드오프하지 않는다. `SessionManager::handoff_all`(`session/handoff_v1.rs`)이 넘기기 전에 `task-chat-` 세션을 `dispose`한다(Windows의 no-op 경로도 같다). 대화는 일회성이고, 입양 경로(`adoptDetachedSessions`)가 캐릭터 없는 세션을 받을 준비가 없기 때문이다.
- `ConfirmQuitDialog`가 살아 있는 대화 수를 한 줄로 알린다. 근무 중인 캐릭터가 없어도 살아 있는 대화가 있으면 `quitGuard`가 확인창을 띄우고, 그때는 대화 안내만 보인다.

## 구현 위치

| 역할 | 코드 |
|---|---|
| ID 표시·복사 | `src/renderer/workpanels/WorkDialog.tsx`(`id` 속성), `WorkItemsPanel.tsx` |
| 대화 버튼 | `src/renderer/workpanels/TaskChatButton.tsx`(`useTaskChatButton`), `WorkItemsPanel.tsx` 실행 줄, `AutoRunSection.tsx` |
| 대화 목록·열기·닫기 | `src/renderer/workpanels/taskChatStore.ts`, `ConfirmCloseTaskChatDialog.tsx`, `src/renderer/bootstrap.ts`(`installTaskChatTracking`) |
| 대화 탭 | `src/renderer/terminal/AgentTabStrip.tsx`, `TerminalHost.tsx`, `src/renderer/store/appStore.ts`(`openTerminal`), `src/renderer/memo/PostItWidget.tsx` |
| 캐릭터 중심 스토어의 대화 이벤트 무시 | `src/renderer/ipc/sessionBridge.ts`, `src/renderer/workpanels/workMonitor.ts`, `sessionSummaryStore.ts` |
| 종료 확인 | `src/renderer/quitGuard.ts`, `src/renderer/agent/ConfirmQuitDialog.tsx` |
| 세션 옵션(`taskChat`)·접두사 검사 | `src-tauri/src/session/manager.rs`(`prepare_session_plan`, `create`, `live_task_chat_count`), `src-tauri/src/types.rs`, `src-tauri/src/ipc/commands/session.rs` |
| 대화용 권한 조각 | `src-tauri/src/office_instructions.rs`(`task_chat_permissions_fragment`, `claude_absolute_path`), `src-tauri/src/lib.rs`(공급자) |
| 핸드오프 제외 | `src-tauri/src/session/handoff_v1.rs`(`handoff_all`) |
| 맥락 묶음·IPC | `src-tauri/src/task_chat.rs`(`build_context`, `validate`, `open`), `src-tauri/src/ipc/commands/work_panels.rs`, `src/shared/ipc.ts`, `src/shared/types/api.ts`·`workPanels.ts`·`session.ts` |
| 문구 | `src/shared/i18n/locales/*/workpanels.json` 6개 언어, 프롬프트 ko/en은 `task_chat.rs` |
| 화면 확인 | `scripts/gui-steps/work-id-copy.mjs`, `task-chat.mjs`, `task-chat-buttons.mjs`, 가짜 백엔드 `src/renderer/guiHarness/fakeApi.ts`의 `openTaskChat` |

## 검증 (2026-10-27)

- 전체: `npx tsc --noEmit` 통과, `npx vitest run` 187개 파일·2,681개 테스트 통과, `cargo test`(src-tauri) 단위 1,274개 통과·22개 무시(실 PTY·실 CLI 스모크, 아래 대화 스모크 포함), `contract_fixtures` 26개·`observer_forwarder` 11개 통과.
- 프런트(vitest): ID 복사 성공·실패 표시, 대화 버튼 위치·툴팁·저장본으로 열기·초안 유지, 막힘(WSL·상한·여는 중)과 열기 실패, 같은 태스크 재열기는 기존 탭, 결과 뒤 탭 열기·실패 시 미리 구독한 터미널 버림, 대화 탭 닫기 확인과 이웃 탭 이동, 늦은 상태 이벤트 무시, `task-chat-` 이벤트가 요약·알림·턴 시간·업무 모니터에 안 들어감, 자동 진행 중 열어도 업무 상태·`sessionIds`·실행 상태가 그대로임, 종료 확인창의 대화 수.
- 네이티브(cargo): `taskChat`에서 훅 설정·`AGENT_OFFICE_AGENT_ID`·`AGENT_OFFICE_HOOK_URL`·셸 작업중 감시가 빠짐, 접두사·옵션 불일치 거절, 대화용 권한 조각과 `//` 경로 꼴, 맥락 묶음의 칸 순서·상한 자르기·제어 문자, 보고 없음·Git 아님·세션 로그 없음, `open_task_chat` 요청 검증, 핸드오프에서 대화 세션 제외.
- 실 PTY: `cargo test --lib real_shell_task_chat_session_runs_unobserved -- --ignored`(macOS zsh) 통과 — 관찰이 켜진 매니저에서도 대화 세션 셸에 `AGENT_OFFICE_AGENT_ID`·`AGENT_OFFICE_HOOK_URL`이 없고 `AGENT_OFFICE_SESSION`은 있으며, `claude` 래퍼가 대화용 설정 파일만 `--settings`로 붙이고 페르소나는 싣지 않으며, `sleep 3`이 activity를 내지 않는다.
- `npm run gui:shot -- --scenario workpanels --script scripts/gui-steps/<work-id-copy|task-chat|task-chat-buttons>.mjs`: ID·복사 표시, 대화 탭 두 개와 닫기 확인, 루트 팝업의 '현재 태스크에 대해 대화', 초안 유지, 상한 안내를 화면으로 확인.

### 남은 실기

- 설치 앱에서 자동 진행 중 대화를 열어 실제 Claude로 질문하고, 진행자의 다음 판단이 영향받지 않는지 확인. Codex 태스크로도 한 번(샌드박스 읽기·`office-md` 승인 흐름 포함). 위 검증은 가짜 백엔드·가짜 CLI까지라 `open_task_chat`의 기동 ACK·첫 프롬프트 붙여넣기를 실제 CLI로 끝까지 띄운 적은 없다.
- Windows(PowerShell·Git Bash)에서 대화 세션 래퍼·`//c/...` 권한 경로 확인.

## 알려진 제한

- 세션 env는 앱 프로세스 env 위에 덧쓰기만 한다. 앱을 Agent Office 터미널 안에서 띄우면(개발 모드 `npm run tauri dev` 등) 부모 세션의 `AGENT_OFFICE_AGENT_ID`·`AGENT_OFFICE_HOOK_URL`이 대화 세션에 그대로 상속된다. Finder·Dock으로 띄운 설치 앱에는 이 값이 없다. 위 실 PTY 스모크는 시작할 때 이 두 값을 지운다.
- 대화 세션도 `office-task --report`를 쓸 수 있고(Claude는 shim이 사전 승인), 그러면 판단자가 읽는 작업자 보고가 바뀐다. 역할 규칙(1번)으로만 막는다.

## 하지 않는 것

- 같은 캐릭터 터미널·외부 터미널에서 대화 열기.
- 판단자 세션을 `--fork-session`으로 갈라 대화하기. 필요해지면 대화 탭의 한 종류로 더한다(작업 폴더 `auto-run/judge`, `claude --resume <judgeSessionId> --fork-session`).
- 대화 내용을 태스크나 자동 진행 기록에 되돌려 붙이기. 사람이 정한 것을 반영하려면 루트 본문(플랜 메모)을 고치거나 태스크를 수정한다.
- 대화 세션의 앱 재시작 뒤 이어받기.
