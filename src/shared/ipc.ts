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
  summarizeText: "summarize_text",
  generateSpriteImage: "generate_sprite_image",
  getAppSettings: "get_app_settings",
  setAppSettings: "set_app_settings",
  // CLI 제어(이슈 #55, docs/cli-control-design.md) — 2단계 승인 상태 조회/승인/취소.
  controlStatus: "control_status",
  controlApprove: "control_approve",
  controlRevoke: "control_revoke",
  // 캐릭터 봇 모드(이슈 #57, docs/bot-mode-design.md) — 탭 단위 시작/중단/상태.
  botStart: "bot_start",
  botStop: "bot_stop",
  botStatus: "bot_status",
  listAvailableShells: "list_available_shells",
  openInVscode: "open_in_vscode",
  openInTerminal: "open_in_terminal",
  exportTerminalOutput: "export_terminal_output",
  pickDirectory: "pick_directory",
  appendSessionTurn: "append_session_turn",
  loadSessionTurns: "load_session_turns",
  // 캐릭터 일기(이슈 #56) — per-agent 일기 append/load.
  appendDiaryEntry: "append_diary_entry",
  loadDiary: "load_diary",
  // 캐릭터 일기(이슈 #60) — 작업 로그 스냅샷 save/load(영속 보존·부팅 복원).
  saveWorkLog: "save_work_log",
  loadWorkLogs: "load_work_logs",
  loadSessionEvents: "load_session_events",
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
  // git 상태(porcelain v2)를 돌려준다. git 조회는 거대 저장소 대비 3초 타임아웃.
  workdirListFiles: "workdir_list_files",
  workdirGitStatus: "workdir_git_status",
  // 변경점(diff)·이력(history) 확인(이슈 #11 후속). 전부 읽기 전용이며
  // difftool만 외부 GUI 도구를 fire-and-forget으로 띄운다.
  workdirDiffFile: "workdir_diff_file",
  workdirFileHistory: "workdir_file_history",
  workdirDiffCommit: "workdir_diff_commit",
  // 이슈 #54: 한 커밋이 바꾼 파일 목록(인라인 확장·페이징)과, 파일 지목 없는
  // 저장소 전체 로그(검색·전체브랜치).
  workdirCommitFiles: "workdir_commit_files",
  workdirRepoLog: "workdir_repo_log",
  workdirDifftool: "workdir_difftool",
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
} as const;
