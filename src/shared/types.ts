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
export type ActivityKind = "prompt" | "tool" | "sub-start" | "sub-stop" | "sub-count";

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
  /** kind="sub-count"일 때 현재 실행 중 서브에이전트 절대 수. */
  count?: number;
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
  /** Claude Code에 `--append-system-prompt`로 전달할 캐릭터 성격 프롬프트. */
  personalityPrompt?: string;
  /** Historical profile label copied into session_started analytics only. */
  agentName?: string;
  /** Historical profile role copied into session_started analytics only. */
  agentRole?: string;
}

/**
 * Wire payload for the `create_session` command (camelCase args).
 * Mirrors Rust `CreateSessionRequest`.
 *
 * `agentName` and `agentRole` are consumed by the Tauri command for the
 * session-start analytics snapshot; they are not part of Rust's PTY
 * `CreateSessionRequest`.
 *
 * `autostartClaude` is a frozen backward-compat wire field. It is not part of
 * the frozen `AgentOfficeApi.createSession` options, so the renderer never sets
 * it and omission defaults to `false`. Renderer-created sessions therefore do
 * not auto-launch a provider; `startupCommand` decides which CLI starts.
 *
 * When observation is enabled, newly created supported terminals define both
 * direct `claude` and `codex` wrappers: Windows PowerShell/`pwsh` functions,
 * a Git Bash `--rcfile`, or the supported zsh ZDOTDIR shim. WSL does not support
 * the observer wrapper.
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
  /** Claude Code 세션에 추가 시스템 프롬프트로 주입할 캐릭터 성격(멀티라인 가능). */
  personalityPrompt?: string;
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
  /** 퇴근(clock-out) 상태. true면 오피스/터미널에서 사라지고 소환 목록에만 남는다.
   * 부재/false = 근무 중. 되돌릴 수 있는 상태이며 프로필 자체는 보존된다. */
  clockedOut?: boolean;
  /** 키보드 사운드 팩 id (sound/packs.ts). 부재/무효 = 기본 팩. */
  keyboardSound?: string;
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

/**
 * 완료된 턴 1건의 시계열 기록. 턴이 종료(settle)될 때마다 append-only 로그
 * (session-times.jsonl)에 추가된다. 나중에 통계용으로 읽는다.
 * Mirrors Rust `SessionTurnRecord`. 모든 시각은 백엔드 epoch ms.
 */
export interface SessionTurnRecord {
  agentId: AgentId;
  /** 이 턴이 시작된 백엔드 epoch ms. */
  startedAt: number;
  /** 이 턴이 종료(settle)된 백엔드 epoch ms. */
  endedAt: number;
  /** 턴 전체 시간(endedAt - startedAt). */
  totalMs: number;
  /** 실작업 시간. */
  workedMs: number;
  /** 대기 시간. */
  waitedMs: number;
}

/** 라벨 요약에 사용할 로컬 CLI provider. Rust `SummaryProvider` 미러. */
export type SummaryProvider = "claude" | "codex";

/** 앱 전역 opt-in 설정 — Rust `persistence::settings_store::AppSettings` 미러. */
export interface AppSettings {
  version: number;
  /** 머리 위 라벨 요약용 로컬 CLI 호출 허용. */
  summarizerEnabled: boolean;
  /** 라벨 요약에 사용할 로컬 CLI provider. */
  summaryProvider: SummaryProvider;
  /** 세션 observer 주입 + 로컬 observer 서버 기동(알림·시간측정). */
  observerEnabled: boolean;
  /** 사무실 앰비언스 사운드(타이핑·효과음·공조음) 재생 여부. 기본 켜짐. */
  soundEnabled: boolean;
  /** 마스터 볼륨 0.0~1.0. 기본 0.5. */
  soundVolume: number;
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
  /** 머리 위 라벨 요약: 캡처한 provider의 로컬 CLI를 호출한다. 호출마다 사용자 구독/크레딧을 소모할 수 있다. */
  summarizeText(
    provider: SummaryProvider,
    instruction: string,
    text: string,
  ): Promise<string>;
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
  /** 완료된 턴 1건을 로컬 시계열 로그에 append (fire-and-forget). */
  appendSessionTurn(record: SessionTurnRecord): void;
  /** 누적된 세션 턴 기록 전체를 읽는다(통계용). 손상된 줄은 건너뛴다. */
  loadSessionTurns(): Promise<SessionTurnRecord[]>;
}
