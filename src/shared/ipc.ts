// src/shared/ipc.ts
//
// Frozen command/event name constants for the Tauri IPC boundary. Both the
// Rust backend (#[tauri::command] names, event emit() names) and the
// renderer adapter (`src/renderer/ipc/tauriApi.ts`) must use these constants
// rather than re-typing the literal strings, so the two sides can't silently
// drift.

/** `invoke()` command names (all commands use `rename_all = "camelCase"` args). */
export const Commands = {
  createSession: "create_session",
  disposeSession: "dispose_session",
  // 외부(논리) 세션 연결 해제 — 앱 밖 터미널에 붙여 둔 캐릭터를 떼어낸다.
  detachExternalSession: "detach_external_session",
  writeInput: "write_input",
  resize: "resize_session",
  clearNotifications: "clear_notifications",
  listNotifications: "list_notifications",
  loadState: "load_state",
  saveState: "save_state",
  setBadgeCount: "set_badge_count",
  subscribeOutput: "subscribe_output",
  unsubscribeOutput: "unsubscribe_output",
  savePortrait: "save_portrait",
  loadPortrait: "load_portrait",
  deletePortrait: "delete_portrait",
  saveSprite: "save_sprite",
  loadSprite: "load_sprite",
  deleteSprite: "delete_sprite",
  // 서브에이전트 미니미 전용 픽셀아트(단일 N×N). 없으면 부모 스프라이트 축소판.
  saveMinimi: "save_minimi",
  loadMinimi: "load_minimi",
  deleteMinimi: "delete_minimi",
  summarizeText: "summarize_text",
  // 설정 화면의 서비스별 모델 목록(요약기·TTS 리라이트 공용, kbm #2fc).
  // provider마다 조회 경로가 다르고(공개 카탈로그·API 키·로컬 CLI) 라이브
  // 소스가 아예 없는 provider도 있다 — 그래서 실패도 "빈 목록"도 정상이며
  // 호출측이 정적 프리셋으로 조용히 폴백한다.
  listProviderModels: "list_provider_models",
  // 로컬 codex CLI 내장 이미지 생성(kbm #2fa) — 설치 탐지 + 프롬프트 1건 생성.
  codexImageStatus: "codex_image_status",
  generateCodexImage: "generate_codex_image",
  getAppSettings: "get_app_settings",
  setAppSettings: "set_app_settings",
  // 작업 중 잠자기 방지(이슈 #68) — 렌더러가 "일하는 캐릭터 있음"을 통지.
  setKeepAwake: "set_keep_awake",
  // 데스크톱 마스코트(이슈 #72, docs/mascot-window-design.md) — 창 표시 토글과,
  // 마스코트 클릭 시 main 포커스 + 터미널 열기 요청.
  setMascotVisible: "set_mascot_visible",
  mascotActivate: "mascot_activate",
  // 알림 대사 TTS — 질문/완료 알림 문구를 캐릭터 대사로 리라이트 + 합성(백엔드
  // 전담: API 키는 웹뷰로 나오지 않고 오디오 바이트만 온다).
  ttsSpeak: "tts_speak",
  ttsListVoices: "tts_list_voices",
  ttsKeyStatus: "tts_key_status",
  ttsSetKeys: "tts_set_keys",
  // CLI 제어(이슈 #55, docs/cli-control-design.md) — 2단계 승인 상태 조회/승인/취소.
  controlStatus: "control_status",
  controlApprove: "control_approve",
  controlRevoke: "control_revoke",
  // 캐릭터 봇 모드(이슈 #57, docs/bot-mode-design.md) — 탭 단위 시작/중단/상태.
  botStart: "bot_start",
  botStop: "bot_stop",
  botStatus: "bot_status",
  // 동료 대화(docs/agent-talk-design.md) — 상태 스냅샷·감사 로그 열람.
  talkStatus: "talk_status",
  listTalkLogDates: "list_talk_log_dates",
  readTalkLog: "read_talk_log",
  listAvailableShells: "list_available_shells",
  openInVscode: "open_in_vscode",
  openInTerminal: "open_in_terminal",
  exportTerminalOutput: "export_terminal_output",
  pickDirectory: "pick_directory",
  // 캐릭터 내보내기/가져오기(이슈 #77) — 저장/열기 다이얼로그 경유 텍스트 파일 rw.
  exportCharacterFile: "export_character_file",
  importCharacterFile: "import_character_file",
  // 캐릭터 일기 내보내기(이슈 #65) — 저장 다이얼로그 경유 md/json 쓰기.
  exportDiaryFile: "export_diary_file",
  appendSessionTurn: "append_session_turn",
  loadSessionTurns: "load_session_turns",
  // 캐릭터 일기(이슈 #56) — per-agent 일기 append/load.
  appendDiaryEntry: "append_diary_entry",
  loadDiary: "load_diary",
  // 캐릭터 일기(이슈 #60) — 작업 로그 스냅샷 save/load(영속 보존·부팅 복원).
  saveWorkLog: "save_work_log",
  loadWorkLogs: "load_work_logs",
  // 에이전트별 포스트잇 메모(이슈 #79) — 현재 장 로드/저장, 한 장 넘기기,
  // 아카이브 목록/열람, 캐릭터 삭제 시 폴더 정리.
  loadMemo: "load_memo",
  saveMemo: "save_memo",
  archiveMemoSheet: "archive_memo_sheet",
  listMemoArchive: "list_memo_archive",
  readMemoSheet: "read_memo_sheet",
  deleteMemos: "delete_memos",
  loadSessionEvents: "load_session_events",
  // 세션 로그(docs/session-log-design.md) — 상시 기록된 터미널 전사 목록/열기와,
  // 그중 하나로 만드는 회고·학습자료(수동 트리거).
  listSessionLogs: "list_session_logs",
  openSessionLog: "open_session_log",
  generateStudyMaterial: "generate_study_material",
  // 세션 핸드오프(docs/session-handoff-design.md) — unix 전용, 종료 시 PTY를
  // sessiond 데몬으로 넘기고 재시작 시 되찾는다.
  handoffSupported: "handoff_supported",
  handoffSessions: "handoff_sessions",
  adoptDetachedSessions: "adopt_detached_sessions",
  // 세션 브로커 v2(docs/session-broker-v2-design.md) — 상시 브로커 모드 여부
  // 조회와, 브로커 모드에서 크래시 생존 화면 복원을 위한 주기 스냅샷 업로드.
  sessionBrokerMode: "session_broker_mode",
  uploadSessionSnapshots: "upload_session_snapshots",
  // Claude 세션 이어하기(docs/claude-session-resume-design.md) — 캡처된
  // native 세션 ID를 agentId별로 돌려준다.
  listClaudeResumeSessions: "list_claude_resume_sessions",
  // 구독 사용량(rate limit) 스냅샷(docs/usage-limits-design.md) — 홈 디렉터리의
  // Claude/Codex 로컬 캐시를 읽어 정규화한 원시 스냅샷.
  loadUsageSnapshot: "load_usage_snapshot",
  // 마크다운 문서 탐색·편집(이슈 #10) — 에이전트 cwd를 root로 하위 .md 목록/읽기/쓰기.
  // 쓰기는 낙관적 잠금(expectedVersion)이며 충돌 시 "CONFLICT"로 시작하는 메시지로 reject.
  markdownListFiles: "markdown_list_files",
  markdownReadFile: "markdown_read_file",
  markdownWriteFile: "markdown_write_file",
  // 작업 폴더 보기(이슈 #11) — 에이전트 cwd를 root로 전체 파일 목록과, 파일별
  // git 상태(porcelain v2)를 돌려준다. git 조회의 1차 탈출구는 사용자 취소
  // (`opId` + workdirGitCancel)이고, 타임아웃(status 120s/조회 300s)은 매달린
  // 자식을 결국 정리하는 백스톱이다.
  workdirListFiles: "workdir_list_files",
  // 서버사이드 검색(이슈 #67) — 목록의 5000개 상한 밖 파일도 Everything
  // 백엔드가 켜져 있으면 검색어로 다시 찾아온다. Walker 백엔드/빈 쿼리/es.exe
  // 실패는 모두 usedIndex=false로 조용히 폴백(프런트가 클라이언트 필터로 전환).
  workdirSearchFiles: "workdir_search_files",
  workdirGitStatus: "workdir_git_status",
  // 라벨 표면의 "프로젝트 (브랜치)" 표시용 경량 조회. status와 달리 취소(opId)가
  // 없고 타임아웃도 2초 — 30초 주기 폴링이라 실패하면 브랜치를 생략할 뿐이다.
  workdirGitBranch: "workdir_git_branch",
  // 변경점(diff)·이력(history) 확인(이슈 #11 후속). 전부 읽기 전용이며
  // difftool만 외부 GUI 도구를 fire-and-forget으로 띄운다.
  workdirDiffFile: "workdir_diff_file",
  workdirFileHistory: "workdir_file_history",
  workdirDiffCommit: "workdir_diff_commit",
  // 이슈 #54: 한 커밋이 바꾼 파일 목록(인라인 확장·페이징)과, 파일 지목 없는
  // 저장소 전체 로그(검색·전체브랜치).
  workdirCommitFiles: "workdir_commit_files",
  workdirRepoLog: "workdir_repo_log",
  // 진행 중인 git 조회 취소 — 조회 커맨드에 넘긴 `opId`로 자식 git을 죽인다.
  // 이미 끝났거나 없는 opId는 조용한 no-op(fire-and-forget으로 부른다).
  workdirGitCancel: "workdir_git_cancel",
  workdirDifftool: "workdir_difftool",
  // 웹 원격(docs/web-remote-design.md) — 앱은 호스트 역할만 한다
  // (페어링 승인·클라이언트 관리·스냅샷 응답).
  webRemoteStatus: "web_remote_status",
  webRemotePairApprove: "web_remote_pair_approve",
  webRemotePairReject: "web_remote_pair_reject",
  webRemoteRevoke: "web_remote_revoke",
  webRemoteSetPermission: "web_remote_set_permission",
  webRemoteSubmitSnapshot: "web_remote_submit_snapshot",
  // tailscale serve 대행(웹 원격 HTTPS 47443). 상태 정본은 tailscaled라
  // 앱 설정에 저장하지 않고 매번 조회한다.
  tailscaleServeStatus: "tailscale_serve_status",
  tailscaleServeEnable: "tailscale_serve_enable",
  tailscaleServeDisable: "tailscale_serve_disable",
} as const;

/**
 * `emit()`/`listen()` event names. PTY output (highest-traffic stream) uses a
 * `Channel` instead, for ordering guarantees and to avoid broadcast overhead;
 * these lower-frequency signals go through events because multiple listeners
 * (office scene, ticker, badge) need to hear them.
 */
export const Events = {
  sessionState: "session-state",
  notificationNew: "notification-new",
  notificationCleared: "notification-cleared",
  activityEvent: "activity-event",
  // 마스코트 창(이슈 #72). mascotState는 main→mascot 브로드캐스트(진실의 원천은
  // main의 스토어), mascotReady는 mascot 부팅 핸드셰이크(main이 현재 상태 재방출),
  // mascotOpenTerminal은 Rust가 main에만 emit_to하는 클릭 결과다.
  mascotState: "mascot-state",
  mascotReady: "mascot-ready",
  mascotOpenTerminal: "mascot-open-terminal",
  // 웹 원격. webRemoteSnapshotRequest는 호스트 렌더러에 화면 직렬화를 요청하는
  // 신호, webRemotePairRequest는 승인 다이얼로그를 띄우는 신호다.
  webRemoteSnapshotRequest: "web-remote-snapshot-request",
  webRemotePairRequest: "web-remote-pair-request",
  // 동료 대화(docs/agent-talk-design.md §7). 누가 말한 *순간* 발화한다 —
  // 실제 배달(주입)은 상대가 한가해질 때까지 늦춰질 수 있다.
  talkMessage: "talk-message",
} as const;
