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
  listAvailableShells: "list_available_shells",
  openInVscode: "open_in_vscode",
  openInTerminal: "open_in_terminal",
  pickDirectory: "pick_directory",
  appendSessionTurn: "append_session_turn",
  loadSessionTurns: "load_session_turns",
  loadSessionEvents: "load_session_events",
  // 세션 핸드오프(docs/session-handoff-design.md) — unix 전용, 종료 시 PTY를
  // sessiond 데몬으로 넘기고 재시작 시 되찾는다.
  handoffSupported: "handoff_supported",
  handoffSessions: "handoff_sessions",
  adoptDetachedSessions: "adopt_detached_sessions",
  // Claude 세션 이어하기(docs/claude-session-resume-design.md) — 캡처된
  // native 세션 ID를 agentId별로 돌려준다.
  listClaudeResumeSessions: "list_claude_resume_sessions",
  // 구독 사용량(rate limit) 스냅샷(docs/usage-limits-design.md) — 홈 디렉터리의
  // Claude/Codex 로컬 캐시를 읽어 정규화한 원시 스냅샷.
  loadUsageSnapshot: "load_usage_snapshot",
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
