// src/shared/types.ts
//
// FROZEN CONTRACT — this file is the source of truth for the renderer<->backend
// boundary. `src-tauri/src/types.rs` is a serde mirror and must stay in exact
// field-by-field agreement with the types below.
//
// Field mapping rule: TS camelCase <-> Rust struct #[serde(rename_all =
// "camelCase")] snake_case fields; TS string-literal unions <-> Rust enum
// #[serde(rename_all = "lowercase")] PascalCase variants; `T | undefined`
// fields <-> Rust `Option<T>` with `skip_serializing_if`.

/** Opaque id types. Both are plain strings on the wire. */
export type AgentId = string;
export type SessionId = string;

/**
 * Session lifecycle state as tracked by the Rust backend.
 * Mirrors Rust `SessionState` (serde lowercase).
 */
export type SessionState = "starting" | "running" | "exited" | "disposed";

/**
 * Renderer-facing session status: `SessionState` plus `idle`, meaning no
 * session has been created yet for this agent.
 *
 * Note: there is no separate "needs_input" or "error" status here.
 * `needs_input` is derived from notification presence (not session state),
 * and `error` is absorbed into `exited` (see `SessionExitInfo.intentional`,
 * false = unexpected exit).
 */
export type SessionStatus = SessionState | "idle";

/**
 * Session exit reason. Accompanies transitions into `exited`/`disposed`.
 * Mirrors Rust `SessionExitInfo`.
 */
export interface SessionExitInfo {
  sessionId: SessionId;
  /** portable-pty ExitStatus.exit_code() as i32, if available. */
  exitCode?: number;
  /** portable-pty does not expose signals cross-platform -> always undefined. */
  signal?: number;
  /** true = app intentionally killed the process (dispose/quit), false = unexpected exit. */
  intentional: boolean;
}

/**
 * Session state transition broadcast. Event name: "session-state".
 * Mirrors Rust `SessionStateEvent`.
 */
export interface SessionStateEvent {
  sessionId: SessionId;
  agentId: AgentId;
  state: SessionState;
  exit?: SessionExitInfo;
  at: number;
}

/**
 * Notification source. Mirrors Rust `NotificationSource` (serde lowercase).
 */
export type NotificationSource = "hook" | "stop" | "bell";

/**
 * Renderer-facing notification display type, derived from `NotificationSource`
 * Never stored/transmitted directly — always computed via
 * `notificationType()`.
 */
export type NotificationType = "question" | "done" | "info";

/**
 * Derives the renderer display type from a notification's source:
 * hook -> "question", stop -> "done", bell -> "info".
 */
export function notificationType(source: NotificationSource): NotificationType {
  switch (source) {
    case "hook":
      return "question";
    case "stop":
      return "done";
    case "bell":
      return "info";
  }
}

/**
 * Normalized notification event. hook POST and BEL fallback both converge to
 * this shape. Event name: "notification-new". Mirrors Rust `NotificationEvent`.
 *
 * `id` is issued by the Rust `NotificationHub` (uuid v4) — the renderer
 * must never reissue/regenerate ids, so that `clearNotifications` stays in
 * sync with the backend.
 */
export interface NotificationEvent {
  id: string;
  sessionId: SessionId;
  agentId: AgentId;
  source: NotificationSource;
  message: string;
  dedupKey: string;
  at: number;
}

/**
 * Activity kind for the time-tracking pipeline. Mirrors Rust `ActivityKind`
 * (serde lowercase). `prompt` = UserPromptSubmit (turn start), `tool` =
 * PostToolUse (heartbeat / waiting→working signal).
 */
export type ActivityKind = "prompt" | "tool";

/**
 * Activity signal for session time tracking. Emitted as the `activity-event`
 * Tauri event, bypassing the notification dedup/queue entirely. Mirrors
 * Rust `ActivityEvent`. `at` is the backend `now_ms()`
 * epoch-ms timestamp — the renderer must settle turns from this, never from
 * its own clock.
 */
export interface ActivityEvent {
  agentId: AgentId;
  sessionId: SessionId;
  kind: ActivityKind;
  at: number;
  /** kind="prompt"일 때 사용자 프롬프트 원문(최대 2,000자 절단). 파싱 실패/부재 시 undefined. */
  text?: string;
}

/**
 * Notifications-cleared broadcast. Event name: "notification-cleared".
 * Mirrors Rust `NotificationClearedEvent`.
 */
export interface NotificationClearedEvent {
  agentId: AgentId;
  ids: string[];
}

/**
 * Options accepted by `AgentOfficeApi.createSession`'s frozen `opts?` param.
 */
export interface CreateSessionOptions {
  cols?: number;
  rows?: number;
  cwd?: string;
  /** 셸 id(예: "pwsh", "git-bash", "wsl", "powershell"). 부재 = 자동/기본 셸. */
  shell?: string;
  /** 세션이 뜬 뒤 셸 stdin에 `{command}\n`으로 주입할 시작 명령어. 부재/공백 = 미주입.
   * 셸 문법(bat/sh/pwsh 등)은 선택한 셸에 맞게 사용자가 작성. */
  startupCommand?: string;
}

/**
 * Wire payload for the `create_session` command (camelCase args).
 * Mirrors Rust `CreateSessionRequest`.
 *
 * `autostartClaude` is not part of the frozen `AgentOfficeApi.createSession`
 * signature — the renderer adapter never sets it, so the backend
 * defaults to `false` when omitted: sessions start a plain shell with no
 * auto-launch. The shell still defines a `claude` wrapper, so typing plain
 * `claude` transparently becomes `claude --settings "$AGENT_OFFICE_SETTINGS"`
 * and time-tracking hooks still fire: on Windows via a PowerShell wrapper
 * function (`session::manager::CLAUDE_WRAPPER_PS`), on macOS/Linux zsh via a
 * ZDOTDIR shim (`session::zsh_wrapper`). Other shells (bash, fish, ...) are
 * not covered yet (see the TODO in `session::manager::default_shell`).
 */
export interface CreateSessionRequest extends CreateSessionOptions {
  agentId: AgentId;
  autostartClaude?: boolean;
}

/**
 * `createSession` response. Mirrors Rust `CreateSessionResult`.
 */
export interface CreateSessionResult {
  sessionId: SessionId;
  state: SessionState;
}

/**
 * PTY output batch. backend -> webview via `tauri::ipc::Channel<OutputChunk>`.
 * Mirrors Rust `OutputChunk`.
 */
export interface OutputChunk {
  sessionId: SessionId;
  /** Included so the renderer can filter/route by agent. */
  agentId: AgentId;
  data: string;
  /** Number of raw PTY read events folded into this batch (diagnostic). */
  frames: number;
  /** Monotonically increasing per-session sequence number. */
  seq: number;
}

/**
 * Agent profile (single definition). Mirrors Rust `AgentProfile`.
 */
export interface AgentProfile {
  /** nanoid */
  id: string;
  name: string;
  role: string;
  note: string;
  /** Sprite seed. */
  seed: string;
  createdAt: number;
  /** Reference only; actual seating is decided by B's assignDesks (deterministic hash). */
  deskIndex: number;
  /** 사용자가 책상 클릭으로 수동 지정한 책상 인덱스. 부재 = 자동(해시) 배정.
   * 지정된 책상은 자동 배정 풀에서 제외된다(주인 전용). */
  assignedDeskIndex?: number;
  /** Session working directory. Absent/undefined = backend falls back to the home dir. */
  cwd?: string;
  /** 셸 id(예: "pwsh", "git-bash", "wsl", "powershell"). 부재 = 자동/기본 셸. */
  shell?: string;
  /** 새 세션이 뜰 때마다 셸 stdin에 주입할 시작 명령어. 부재/공백 = 미주입.
   * 예: "source ./init.sh", "mysetup.bat". 셸 문법은 사용자 책임. */
  startupCommand?: string;
  /** 외모 묘사 힌트(자유 텍스트). 이미지 프롬프트에 반영. */
  appearance?: string;
  /** 초상 존재 표시 + 프론트 캐시 무효화 키(epoch ms). undefined = 초상 없음. */
  portraitUpdatedAt?: number;
  /** 픽셀아트 프롬프트 의뢰 문구(자유 텍스트). 비면 appearance로 폴백. */
  spriteRequest?: string;
  /** 커스텀 스프라이트 존재 표시 + 프론트 캐시 무효화 키(epoch ms). undefined = 절차 생성 사용. */
  spriteUpdatedAt?: number;
  /** 캐릭터 아키타입(종족) id. 부재/알 수 없음 = "human" 폴백, "auto" = 시드 추첨(저장 시 확정). */
  archetype?: string;
}

/**
 * Persisted app state. Mirrors Rust `PersistedState`.
 * Stored by Rust `ProfileStore` (profiles.json in Tauri app data dir);
 * sessionId is never persisted (runtime-only).
 */
export interface PersistedState {
  agents: AgentProfile[];
  version: 1;
}

/** PixelLab 생성 결과 (Rust pixellab::GeneratedImage 미러, camelCase). */
export interface GeneratedSpriteImage {
  pngBase64: string;
  /** usage.type=="usd"일 때만 존재. 구독(generations) 차감이면 없음. */
  costUsd?: number;
}

/** 앱 전역 opt-in 설정 — Rust `persistence::settings_store::AppSettings` 미러. */
export interface AppSettings {
  version: number;
  /** 머리 위 라벨 요약용 로컬 `claude` CLI 호출 허용(구독 크레딧 소모). */
  claudeCliEnabled: boolean;
  /** 세션에 Claude Code 훅 주입 + 로컬 훅 서버 기동(알림·시간측정). */
  claudeHooksEnabled: boolean;
}

/** `get_app_settings` 응답. firstRun = settings.json 부재(첫 실행). */
export interface GetAppSettingsResult {
  settings: AppSettings;
  firstRun: boolean;
}

/**
 * `list_available_shells` 응답 엔트리. Windows에서만 실제 목록을 반환하고,
 * 그 외 플랫폼은 빈 배열. Mirrors Rust `AvailableShell` (camelCase).
 */
export interface AvailableShell {
  id: string;
  label: string;
  path: string;
  /** false면 시간 추적(hook) 미지원 셸. */
  hooksSupported: boolean;
}

/**
 * Renderer-facing API surface (frozen). Implemented by
 * `src/renderer/ipc/tauriApi.ts` via Tauri commands (invoke) + events
 * (listen) + a dedicated output `Channel` (exact command/event names are
 * in `src/shared/ipc.ts`).
 *
 * sessionId is a Rust-backend-internal concept (hook routing, settings file
 * naming) and never crosses this boundary — every method here is keyed by
 * `agentId`.
 */
export interface AgentOfficeApi {
  createSession(agentId: string, opts?: CreateSessionOptions): Promise<CreateSessionResult>;
  disposeSession(agentId: string): Promise<void>;
  /** fire-and-forget */
  writeInput(agentId: string, data: string): void;
  resize(agentId: string, cols: number, rows: number): void;
  clearNotifications(agentId: string, ids?: string[]): void;
  listNotifications(agentId: string): Promise<NotificationEvent[]>;
  loadState(): Promise<PersistedState>;
  saveState(state: PersistedState): Promise<void>;
  setBadgeCount(n: number): void;
  /** 초상 PNG(base64, data: prefix 없음) 저장. */
  savePortrait(agentId: string, pngBase64: string): Promise<void>;
  /** 초상 base64를 반환, 파일 없으면 null. */
  loadPortrait(agentId: string): Promise<string | null>;
  /** 초상 파일 삭제(없어도 성공). */
  deletePortrait(agentId: string): Promise<void>;
  /** 커스텀 스프라이트 시트 PNG(base64, data: prefix 없음, 64×16) 저장. */
  saveSprite(agentId: string, pngBase64: string): Promise<void>;
  /** 스프라이트 시트 base64를 반환, 파일 없으면 null. */
  loadSprite(agentId: string): Promise<string | null>;
  /** 스프라이트 파일 삭제(없어도 성공). */
  deleteSprite(agentId: string): Promise<void>;
  /** 머리 위 라벨 요약: `claude -p`(haiku) 헤드리스 호출. 호출마다 사용자의 Claude 구독/크레딧을 소모한다. */
  summarizeText(instruction: string, text: string): Promise<string>;
  /** PixelLab로 64×64 스프라이트 1장 생성. 동기 HTTP — 수십 초 걸릴 수 있다. */
  generateSpriteImage(description: string): Promise<GeneratedSpriteImage>;
  /** 앱 전역 opt-in 설정 로드. 인자 없음. */
  getAppSettings(): Promise<GetAppSettingsResult>;
  /** 앱 전역 opt-in 설정 저장. */
  setAppSettings(settings: AppSettings): Promise<void>;
  /** 사용 가능한 셸 목록. Windows 외 플랫폼은 빈 배열. */
  listAvailableShells(): Promise<AvailableShell[]>;
  /** 디렉터리를 Visual Studio Code로 연다. VS Code 미설치/경로 부재 시 reject. */
  openInVscode(path: string): Promise<void>;
  /** Returns an unsubscribe function. */
  onData(agentId: string, cb: (data: string) => void): () => void;
  onSessionState(cb: (e: SessionStateEvent) => void): () => void;
  onNotification(cb: (n: NotificationEvent) => void): () => void;
  onNotificationCleared(cb: (p: { agentId: string; ids: string[] }) => void): () => void;
  /** activity-event(prompt/tool) 구독. Returns an unsubscribe function. */
  onActivity(cb: (e: ActivityEvent) => void): () => void;
}
