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
| 예약 명세 파싱(`office-task --at`·`--every`, 수신함 `schedule:`) | `persistence/work_inbox.rs`의 `normalize_schedule` |
| 업무 수신함 `office-task`, 공통 안내문 | `work_inbox_cli.rs`, `persistence/work_inbox.rs`, `office_instructions.rs` |
| 문서 뷰어 `office-md` | `doc_view.rs`, `doc_view_cli.rs` |
| 외부 CLI 제어 `ctl`, 플랜 위임(`autorun`·`ask`·턴 상태) | `control/` (`autorun.rs`·`turns.rs`), 렌더러 쪽 `workpanels/autoRunControl.ts` |
| 작업 폴더·Git·파일 색인 | `workdir/`, `file_index/`, `file_manager.rs` |
| 실행 레시피 | `run_recipes/` |

### 렌더러 (`src/renderer/`)

| 영역 | 어디 |
|---|---|
| 진입·부팅 | `main.tsx`, `bootstrap.ts`, `App.tsx` |
| 전역 상태 | `store/appStore.ts`, 파생값은 `store/selectors.ts` |
| 백엔드 이벤트 → 스토어 | `ipc/sessionBridge.ts` |
| 오피스 씬(PixiJS)·캐릭터 생성 | `office/` (`OfficeScene.ts`, 생성기 `office/gen/`, 맵 `office/map/`) |
| 터미널 탭·xterm | `terminal/` (`TerminalHost.tsx`, `TerminalRegistry.ts`, `AgentTabStrip.tsx`) |
| 업무 패널·자동 진행·태스크 대화 | `workpanels/` |
| 예약(규칙 계산·실행기·팝업 영역) | `workpanels/workSchedule.ts`, `scheduleRunner.ts`, `WorkScheduleSection.tsx` |
| 팀 토의(역할 목록·프리셋, 토의 카드·열기 대화상자, 사회자 주도 진행, 라운드·시간·비용 상한과 사용량 대기, 결정 메모와 결론 확인) | `discussion/` (`roleStore.ts`, `RolesDialog.tsx`, `discussionStore.ts`, `discussionModel.ts`, `DiscussionSection.tsx`, `DiscussionEditor.tsx`, 진행 루프·안전장치 `discussionRun.ts`, 결정 메모 작성 `discussionMemo.ts`), 저장은 `persistence/role_store.rs`·`persistence/discussion_store.rs`(메모 `discussions/memos/<id>.md`), 메모 브라우저 열기는 `ipc/commands/doc_view.rs`. 에이전트·CLI 경로: 만들기 `office-task --discuss`(`discussion_cli.rs` → 수신함 `persistence/discussion_inbox.rs`, 렌더러 `discussionInbox.ts`), `ctl discuss list·status·memo·start·answer`(`control/discussion.rs`, 쓰기는 렌더러 `discussionControl.ts`) |
| 마스코트 창·신호등 | `mascot/`, `store/mascotLights.ts`, `ipc/mascotBridge.ts` |
| 그 밖의 기능 | 기능 이름의 폴더(`usage/`, `analytics/`, `sessionlog/`, `awards/`, `workdir/`, `markdown/`, `diary/`, `settings/` …) |
| 다국어 | 카탈로그 `src/shared/i18n/locales/`, 하드코딩 검사 `__tests__/noHardcodedHangul.test.ts` |

### 도구

| 무엇 | 어디 |
|---|---|
| 헤드리스 GUI 확인 `npm run gui:shot` | `scripts/gui-shot.mjs`, 가짜 백엔드·시나리오 `src/renderer/guiHarness/`, 스텝 스크립트 `scripts/gui-steps/` |
| 그 밖의 스크립트 | [scripts/README.md](../scripts/README.md) |
