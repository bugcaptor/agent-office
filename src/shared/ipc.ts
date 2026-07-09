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
} as const;

export type CommandName = (typeof Commands)[keyof typeof Commands];

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

export type EventName = (typeof Events)[keyof typeof Events];
