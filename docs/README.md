# docs/

설계·명세 문서는 두지 않는다. 동작은 코드와 테스트가 정본이다. 여기에는 코드로 알 수 없는 것과 코드 찾아가기 길잡이만 둔다.

| 문서 | 내용 |
|---|---|
| [guide.md](guide.md) | 사용 안내와 개발자 절(작업 규칙·빌드·테스트) |
| [macos-signing.md](macos-signing.md) | macOS 서명 원리, 인증서 관리, DMG 배포 절차 |
| 이 파일 | 코드 지도 |

## 코드 지도

### 경계: 렌더러 ↔ Rust

| 무엇 | 어디 |
|---|---|
| 커맨드·이벤트 이름 | `src/shared/ipc.ts` |
| 렌더러가 보는 API 계약 | `src/shared/types/api.ts`의 `AgentOfficeApi`, 도메인 타입은 `src/shared/types/*.ts` |
| 계약 구현(Tauri invoke) | `src/renderer/ipc/tauriApi.ts` |
| Rust 커맨드 | `src-tauri/src/ipc/commands/*.rs`, 등록은 `src-tauri/src/lib.rs`의 `generate_handler!` |
| Rust 미러 타입 | `src-tauri/src/types.rs` |
| TS·Rust 직렬화 대조 | `src/shared/contract-fixtures/` ↔ `src-tauri/tests/contract_fixtures.rs` |

### Rust (`src-tauri/src/`)

| 영역 | 어디 |
|---|---|
| 앱 진입·서브커맨드 분기(`ctl`·`task`·`md`·`--sessiond`)·부팅 배선 | `lib.rs`, `main.rs` |
| 앱 상태 | `state.rs` |
| PTY 세션, 셸 래퍼, CLI별 훅 주입(pi·kilo·agy) | `session/` (`manager.rs`가 중심) |
| 앱 종료 후 세션 존속(fd 핸드오프) | `session/handoff_v1.rs`, `sessiond/` |
| CLI 훅 수신·작업 상태 판정 | `observer/` (`server.rs`, CLI별 `claude.rs`·`codex.rs`·`event.rs`) |
| 알림·턴 사용량 | `notification/hub.rs` |
| 세션 이벤트 시계열(분석 패널 원천) | `session_events/` |
| 터미널 전사 기록·학습자료 | `session_log/` |
| 구독 사용량(rate limit) | `usage/` |
| 라벨 요약·일기 생성(LLM 호출) | `summarizer/` |
| 영속화(설정·프로필·업무 등) | `persistence/` |
| 업무 보내기·자동 진행·태스크 대화 | `work_send.rs`, `work_send/`, `auto_run.rs`, `task_chat.rs` |
| 일반 에이전트(캐릭터 없는 `office-agent` 곁 세션, 웹 상태판에서 지시) | `office_agent.rs`(열기·마지막 실패), 세션 준비는 `session/manager.rs`의 `prepare_session_plan`, 웹 라우트 `webdash/routes.rs`(`/api/office-agent`), 렌더러 `officeAgent/officeAgentStore.ts` |
| 예약 명세 파싱(`office-task --at`·`--every`, 수신함 `schedule:`) | `persistence/work_inbox.rs`의 `normalize_schedule` |
| 업무 수신함 `office-task`, 공통 안내문 | `work_inbox_cli.rs`, `persistence/work_inbox.rs`, `office_instructions.rs` |
| 문서 뷰어 `office-md` | `doc_view.rs`, `doc_view_cli.rs` |
| 외부 CLI 제어 `ctl`, 플랜 위임(`autorun`·`ask`·턴 상태) | `control/` (`autorun.rs`·`turns.rs`), 렌더러 쪽 `workpanels/autoRunControl.ts` |
| 팀 토의(저장·탭 세션·CLI) | 아래 [팀 토의](#팀-토의) 절 |
| 작업 폴더·Git·파일 색인 | `workdir/`, `file_index/`, `file_manager.rs` |
| 실행 레시피 | `run_recipes/` |
| 웹 상태판(tailnet·LAN HTTP 서버, 토큰, 전사 tail, 간접 명령) | `webdash/` (`mod.rs` 수명·설정 `web-dash.json`·토큰 `web-dash-token`, `net.rs` 바인드 주소, `routes.rs` API(`/api/image`·`/api/asks`), `log.rs` 대화 로그 버퍼·이미지 상자), 전사 이미지 파싱은 `session_log/agent_transcript/`(`ItemKind::Image`), 권한 요청·선택지 중계는 `observer/permission.rs`(Claude `PermissionRequest` 훅 → forwarder `forward_permission` → 옵저버 서버 `/permission` 장기 대기), 페이지 `src-tauri/web/dash.html`(가짜 API로 폰 크기 스크린샷: `node scripts/dash-shot.mjs`), 커맨드 `ipc/commands/web_dash.rs` |

### 렌더러 (`src/renderer/`)

| 영역 | 어디 |
|---|---|
| 진입·부팅 | `main.tsx`, `bootstrap.ts`, `App.tsx` |
| 전역 상태 | `store/appStore.ts`, 파생값은 `store/selectors.ts` |
| 백엔드 이벤트 → 스토어 | `ipc/sessionBridge.ts` |
| 오피스 씬(PixiJS)·캐릭터 생성 | `office/` (`OfficeScene.ts`, 생성기 `office/gen/`, 맵 `office/map/`) |
| 터미널 탭·xterm | `terminal/` (`TerminalHost.tsx`, `TerminalRegistry.ts`, `AgentTabStrip.tsx`) |
| 터미널 탭 대화 보기(전사 채팅·이미지·권한 카드·입력) | `chatView/` (`ChatView.tsx`, 순수 로직 `chatViewModel.ts`, 탭별 보기 `chatViewStore.ts`), Rust 커맨드 `ipc/commands/chat_view.rs`(수집기는 `WebDashState::app_logs`, 권한은 `observer/permission.rs`), gui 확인 `scripts/gui-steps/chat-view.mjs` |
| 업무 패널·자동 진행·태스크 대화 | `workpanels/` |
| 예약(규칙 계산·실행기·팝업 영역) | `workpanels/workSchedule.ts`, `scheduleRunner.ts`, `WorkScheduleSection.tsx` |
| 팀 토의 | `discussion/` — 아래 [팀 토의](#팀-토의) 절 |
| 마스코트 창·신호등·말풍선 | `mascot/`, `store/mascotLights.ts`, `store/mascotSpeech.ts`(말풍선 텍스트 선정), `ipc/mascotBridge.ts`(말풍선 표시 시간 포함) |
| 일반 에이전트 탭 | `officeAgent/officeAgentStore.ts`(탭·상태, 열기는 웹 상태판 요청 `office-agent-open`), gui 확인 `scripts/gui-steps/office-agent.mjs` |
| 웹 상태판 | `webdash/` — 쓰기 요청 처리 `webDashControl.ts`(`web-dash-control` 이벤트), 접속 표시·알림·라벨 넘기기 `webDashStore.ts`·`WebDashIndicator.tsx`, 설정 `settings/WebDashSection.tsx` |
| 그 밖의 기능 | 기능 이름의 폴더(`usage/`, `analytics/`, `sessionlog/`, `awards/`, `workdir/`, `markdown/`, `diary/`, `settings/` …) |
| 다국어 | 카탈로그 `src/shared/i18n/locales/`, 하드코딩 검사 `__tests__/noHardcodedHangul.test.ts` |

### 팀 토의

사용 안내는 [guide.md의 팀 토의](guide.md#팀-토의)에 있다.

| 무엇 | 어디 |
|---|---|
| 타입(역할·토의·진행 상태·탭 세션) | TS `src/shared/types/role.ts`·`discussion.ts`(상한 범위 `DISCUSSION_LIMIT_RANGES`, 검사 `discussionProblem`), Rust `src-tauri/src/types.rs`, 대조 fixture `src/shared/contract-fixtures/`의 `roles-file.json`·`discussions-file.json`·`discussion-run.forced-wrap.json`·`*-discussion-*.json` |
| Rust 커맨드 | 역할·토의 저장·메모·수신함 `ipc/commands/persistence.rs`(`load_roles`·`save_roles`·`load_discussions`·`save_discussion`·`delete_discussion`·`save_discussion_memo`·`load_discussion_memo`·`scan_discussion_inbox`·`scan_role_inbox`), 탭 세션·ctl 회신 `ipc/commands/work_panels.rs`(`open_discussion_session`·`send_discussion_turn`·`collect_discussion_utterance`·`discussion_control_reply`), 메모 열기·속기록 내보내기 `ipc/commands/doc_view.rs`(`open_discussion_memo`·`open_discussion_transcript`) |
| 역할 목록·프리셋·프로필 기본 역할 | 렌더러 `discussion/roleStore.ts`·`RolesDialog.tsx`, 저장 `persistence/role_store.rs`(`roles.json`), CLI `role_cli.rs`(`office-task --roles`·`--role-add`·`--role-edit`·`--role-delete`)와 수신함 `persistence/role_inbox.rs`·렌더러 `discussion/discussionInbox.ts`(`receiveRoleInbox`) |
| 토의 저장·카드·열기 대화상자 | `discussion/discussionStore.ts`·`discussionModel.ts`·`DiscussionSection.tsx`·`DiscussionEditor.tsx`, 저장 `persistence/discussion_store.rs`(`discussions.json`, 상한 기본값 `DiscussionLimits::default`) |
| 토의 탭 세션·첫 프롬프트(읽기 전용 규칙)·발언 수집 | Rust `discussion_session.rs`, 렌더러 `discussion/discussionSessionStore.ts`, 탭 표시 `terminal/AgentTabStrip.tsx` |
| 발언 제출 `office-discuss say` | `discussion_cli.rs`(`say`), Codex·Claude 마지막 응답은 Stop 훅 `last_assistant_message` → `observer/mod.rs`의 `StopReplySink`(Claude는 없으면 전사) |
| 사회자 주도 진행·상한·사용량 대기·마무리 | `discussion/discussionRun.ts`(부팅 배선 `bootstrap.ts`의 `installDiscussionRunner`) |
| 결정 메모 | 작성 `discussion/discussionMemo.ts`, 저장 `discussions/memos/<id>.md`(`persistence/discussion_store.rs`), 브라우저 열기 `ipc/commands/doc_view.rs`, 렌더링 `markdown/renderMarkdown.ts` |
| CLI에서 만들기 `office-task --discuss` | `discussion_cli.rs`(`create`) → 수신함 `persistence/discussion_inbox.rs` → 렌더러 `discussion/discussionInbox.ts`, 에이전트 안내 `office_instructions.rs` |
| `ctl discuss list·status·watch·memo·start·answer` | `control/discussion.rs`(참가자별 `speaking`·`last`), `control/client.rs`(`watch` 루프 `run_discuss_watch`), 쓰기 중계는 렌더러 `discussion/discussionControl.ts` |
| 오피스 회의실 연출 | 맵 `office/map/mapData.ts`의 `meetingRoom`(풍경마다 `meetingRoomAt`으로 둔다), 토의 → 씬 정보 `discussion/officeStage.ts`(`speakingSince`) → `officeBus.onDiscussionStageChanged`(`ipc/sessionBridge.ts`가 보냄) → `office/world/OfficeWorld.ts` → `office/entities/CharacterEntity.ts`·`behaviorFsm.ts`(`meeting` 상태), 분신 `office/entities/MeetingAvatar.ts`(`OfficeWorld.syncMeetingAvatars`), 말풍선·의사봉 `office/entities/DiscussionOverlay.ts`, 회의실 그림 `office/scenes/meetingTiles.ts`(색은 오피스 테마 팔레트, 다른 풍경은 저작 팔레트의 `meet*` 키) |
| 회의록 보드 | 행 파생 `discussion/meetingBoardModel.ts`, 화면 `discussion/MeetingBoard.tsx`(`App.tsx`에 마운트, 붙는 쪽 `boardLeft`), 회의실 화면 사각형 `OfficeScene.publishMeetingRoomRect` → `officeBus.onMeetingRoomRectChanged`, 행 클릭 → 속기록 탭(`openDialog`의 `tab`·`focus`) |
| 속기록 탭·내보내기 | 행·필터·접기·Markdown 엮기 `discussion/transcriptModel.ts`, 화면 `discussion/DiscussionTranscript.tsx`(보기 창 탭은 `DiscussionSection.tsx`의 `DiscussionDetail`, 대화상자는 업무 패널과 무관하게 `App.tsx`의 `DiscussionDialogs`), 임시 HTML은 Rust `open_discussion_transcript` → `doc-view/discussion-transcript-<id>.html` |
| gui 확인 | 시나리오 `discussions`·`meeting`, 스크립트 `scripts/gui-steps/discussion*.mjs` |

### 도구

| 무엇 | 어디 |
|---|---|
| 헤드리스 GUI 확인 `npm run gui:shot` | `scripts/gui-shot.mjs`, 가짜 백엔드·시나리오 `src/renderer/guiHarness/`, 스텝 스크립트 `scripts/gui-steps/` |
| 그 밖의 스크립트 | [scripts/README.md](../scripts/README.md) |
