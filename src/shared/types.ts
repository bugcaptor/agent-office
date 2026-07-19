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
 * PostToolUse (heartbeat / waiting→working signal). `resume` = the backend's
 * post-completion output heuristic (이슈 #39) deciding the agent is still
 * working after a Stop; the renderer treats it like `tool` for turn purposes.
 */
export type ActivityKind =
  | "prompt"
  | "tool"
  | "sub-start"
  | "sub-stop"
  | "sub-count"
  | "resume";

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
  /** kind="prompt"일 때 사용자 프롬프트 원문(최대 2,000자 절단), kind="tool"일 때
   * 도구 요약("Bash: npm test" 등, 최대 60자). 파싱 실패/부재 시 undefined. */
  text?: string;
  /** kind="tool"일 때 턴 중간 assistant 내레이션(claude transcript 꼬리, 스로틀
   * 적용). 그 외 kind/codex/부재는 undefined. */
  assistantText?: string;
  /** kind="prompt"일 때 훅 body top-level cwd(세션 실제 작업 디렉터리, 라벨
   * 프로젝트명 표시용, 이슈 #44 작업 D). 그 외 kind/부재는 undefined. */
  cwd?: string;
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
  /**
   * Raw stream bytes this batch carried (§#49 offset accounting). May differ
   * from `data.length`: the renderer accumulates this on write to derive the
   * snapshot offset. Adopt restore snapshots carry `bytes === 0` so they are
   * excluded from that accounting.
   */
  bytes: number;
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

/**
 * 세션 이벤트 시계열 레코드의 종류. Rust `SessionEventKind`(serde snake_case)
 * 미러. 수집 설계 docs/session-event-timeseries-design.md §4.1 참조.
 */
export type SessionEventKind =
  | "session_started"
  | "session_state"
  | "prompt"
  | "tool"
  | "notification"
  | "bell"
  | "stop";

/**
 * 세션 원천 이벤트 1건. `<app-data>/session-events/v1/YYYY-MM-DD.jsonl`에
 * 한 줄씩 쌓인 레코드를 `loadSessionEvents`가 그대로 돌려준다(집계는 렌더러가
 * 한다). Rust `SessionEventRecord`(camelCase envelope + 옵션 필드 +
 * snake_case `kind`/`state`) 미러. 모든 시각은 백엔드 epoch ms.
 *
 * 옵션 필드는 `kind`에 따라만 존재한다: `agentName`/`agentRole`/`cwd`/`shell`은
 * `session_started`에서만, `state`는 `session_state`에서만. 나머지 종류는
 * envelope만 갖는다(선행 설계 §4.1).
 */
export interface SessionEventRecord {
  /** 정수 스키마 버전. v1에서는 항상 1. */
  schemaVersion: number;
  /** 앱 프로세스 시작마다 생성하는 UUID. */
  runId: string;
  /** 해당 runId 안에서 1부터 증가하는 순번. */
  seq: number;
  /** 백엔드가 부여한 epoch ms. */
  at: number;
  agentId: AgentId;
  sessionId: SessionId;
  kind: SessionEventKind;
  /** kind="session_started"일 때 세션 시작 당시 프로필 이름 스냅샷. */
  agentName?: string;
  /** kind="session_started"일 때 세션 시작 당시 역할 스냅샷. */
  agentRole?: string;
  /** kind="session_started"일 때 실제 세션 작업 디렉터리. */
  cwd?: string;
  /** kind="session_started"일 때 자동 선택까지 끝난 실제 실행 셸. */
  shell?: string;
  /** kind="session_state"일 때 전이한 세션 상태. */
  state?: SessionState;
}

/**
 * Claude 세션 이어하기(resume) 엔트리 — 에이전트당 최신 1건. observer가 훅
 * body의 native `session_id`(그리고 `cwd`)를 캡처해 저장한 값으로,
 * `claude --resume <sessionId>` 재개에 쓴다. 설계: docs/claude-session-resume-design.md.
 * Rust `ClaudeResumeEntry`(camelCase) 미러.
 */
export interface ClaudeResumeEntry {
  /** Claude Code native 세션 ID(agent-office 자체 UUID가 아님). */
  sessionId: SessionId;
  /** 캡처 시점의 작업 디렉터리(참고용 — resume은 같은 프로젝트에서만 찾는다). */
  cwd?: string;
  /** 마지막으로 갱신된 백엔드 epoch ms. */
  updatedAt: number;
}

/** 라벨 요약에 사용할 로컬 CLI provider. Rust `SummaryProvider` 미러. */
export type SummaryProvider = "claude" | "codex";

/** "OS 터미널로 열기"가 사용할 외부 터미널 앱 — Rust `ExternalTerminal` 미러.
 * macOS에서만 의미가 있다(다른 OS는 무시). */
export type ExternalTerminalApp = "terminal" | "iterm";

/** 셸 출력 내보내기(.txt)를 열 외부 에디터 — Rust `ExternalEditor` 미러.
 * 기본은 OS 기본 연결(system). */
export type ExternalEditorApp = "system" | "vscode";

/** 앱 전역 opt-in 설정 — Rust `persistence::settings_store::AppSettings` 미러. */
export interface AppSettings {
  version: number;
  /** 머리 위 라벨 요약용 로컬 CLI 호출 허용. */
  summarizerEnabled: boolean;
  /** 라벨 요약에 사용할 로컬 CLI provider. */
  summaryProvider: SummaryProvider;
  /** 실험(옵트인): Claude 요약기가 읽기 전용 툴(Read/Glob/Grep)로 세션 작업
   * 폴더를 훑어 목표를 추론하도록 허용. Claude provider일 때만 효과. 기본 꺼짐. */
  summarizerToolCalls: boolean;
  /** 세션 observer 주입 + 로컬 observer 서버 기동(알림·시간측정). */
  observerEnabled: boolean;
  /** 사무실 앰비언스 사운드(타이핑·효과음·공조음) 재생 여부. 기본 켜짐. */
  soundEnabled: boolean;
  /** 마스터 볼륨 0.0~1.0. 기본 0.5. */
  soundVolume: number;
  /** "OS 터미널로 열기"가 사용할 터미널 앱. 기본 Terminal.app(macOS 전용). */
  externalTerminal: ExternalTerminalApp;
  /** 셸 출력 내보내기(.txt)를 열 에디터. 기본 OS 기본 연결. */
  externalEditor: ExternalEditorApp;
  /** 질문(Hook) 알림을 방출 전 보류하는 시간(ms). 그 사이 세션이 계속
   * 일하면(오토모드 자동 승인 등) 알림을 조용히 폐기한다. 0이면 즉시 알림. 기본 5000. */
  attentionHoldMs: number;
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
 * `adopt_detached_sessions` 응답 엔트리 — 재시작 시 `sessiond` 데몬에서
 * 되찾은 세션 1건. Mirrors Rust `AdoptedSessionInfo` (camelCase).
 * 세션 핸드오프 설계: docs/session-handoff-design.md §커맨드.
 */
export interface AdoptedSessionInfo {
  agentId: AgentId;
  sessionId: SessionId;
  rows: number;
  cols: number;
}

/**
 * 구독 사용량(rate limit) 한도 윈도 종류. Rust `UsageWindowKind`
 * (serde snake_case) 미러. 설계: docs/usage-limits-design.md §3.
 * `unknown`은 미래 확장 대비 폴백(예: 매핑 안 된 codex window_minutes).
 */
export type UsageWindowKind = "session" | "weekly" | "weekly_model" | "unknown";

/**
 * 한도 윈도 1개. Rust `UsageWindow`(camelCase) 미러. 단위는 전부 백엔드에서
 * 정규화됨: `resetsAtMs`는 epoch ms(Claude ISO·Codex 초 모두 변환), 백분율은
 * `usedPercent`. nullable 필드는 `T | null`(optional 아님).
 */
export interface UsageWindow {
  kind: UsageWindowKind;
  /** weekly_model일 때 모델 표시명 등. 없으면 null. */
  label: string | null;
  usedPercent: number;
  /** epoch ms로 정규화. 파싱 불가/부재 시 null. */
  resetsAtMs: number | null;
  windowMinutes: number | null;
  /**
   * "지금 구속 중인 윈도"인지(Claude `limits[]`에만 있음). **유효성이
   * 아니다** — 실측(`~/.claude.json`)상 weekly_all/weekly_scoped도 살아
   * 있는 한도인데 false로 온다. 걸러내는 용도로 쓰지 말 것, 표시용 보조
   * 정보로만 쓴다. Codex와 Claude five_hour/seven_day 폴백 경로는 항상 null.
   */
  isActive: boolean | null;
}

/**
 * provider별 사용량. Rust `ProviderUsage`(camelCase) 미러.
 * `windows`는 가변 배열 — UI가 "5시간+주간 둘 다 있음"을 하드코딩하지 않는다.
 */
export interface ProviderUsage {
  provider: "claude" | "codex";
  /** 신선도 기준 시각(epoch ms). 로컬 CLI가 실제로 돌 때만 갱신되는 캐시. */
  fetchedAtMs: number;
  /** codex plan_type, claude organizationRateLimitTier 등. 없으면 null. */
  planLabel: string | null;
  windows: UsageWindow[];
}

/**
 * `load_usage_snapshot` 응답. Rust `UsageSnapshot` 미러. 파싱에 실패한 소스는
 * 해당 provider가 null이며, 커맨드 자체는 항상 성공한다.
 */
export interface UsageSnapshot {
  claude: ProviderUsage | null;
  codex: ProviderUsage | null;
}

/**
 * 마크다운 문서 탐색·편집(이슈 #10)의 renderer<->backend 계약.
 * `version`은 렌더러가 해석하지 않는 불투명 토큰(백엔드가 발급, 낙관적 잠금용)이라
 * 왕복만 한다 — 값 형식(해시·mtime 등)은 백엔드 소관이므로 `string`으로만 다룬다.
 */
export interface MarkdownFileEntry {
  /** root 기준 상대 경로(POSIX 구분자). 목록/열기의 키. */
  relPath: string;
  /** 표시·퍼지 매칭 가중치용 파일명(경로 마지막 세그먼트). */
  name: string;
}

/** `markdown_list_files` 응답. `truncated`면 상한을 넘어 일부만 담겼다. */
export interface MarkdownListResult {
  files: MarkdownFileEntry[];
  truncated: boolean;
}

/** `markdown_read_file` 응답. `version`은 이후 쓰기의 `expectedVersion`으로 되돌려준다. */
export interface MarkdownReadResult {
  content: string;
  version: string;
}

/** `markdown_write_file` 응답. 저장 성공 시 갱신된 `version`을 돌려준다. */
export interface MarkdownWriteResult {
  version: string;
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
  /** 머리 위 라벨 요약: 캡처한 provider의 로컬 CLI를 호출한다. 호출마다 사용자
   * 구독/크레딧을 소모할 수 있다. cwd가 있으면(실험 툴 모드) Claude가 그 폴더에서
   * 읽기 전용 툴로 목표를 추론한다 — 실제 툴 모드 여부는 백엔드가 설정으로 판단. */
  summarizeText(
    provider: SummaryProvider,
    instruction: string,
    text: string,
    cwd?: string,
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
  /** 디렉터리를 OS 기본 터미널 앱으로 연다. 경로 부재/실행 실패 시 reject. */
  openInTerminal(path: string): Promise<void>;
  /** 셸 출력(터미널 버퍼 plain text)을 임시 .txt로 쓰고 설정한 외부 에디터로
   * 연다. 쓴 파일의 절대 경로를 반환, 쓰기/실행 실패 시 reject. */
  exportTerminalOutput(agentName: string, content: string): Promise<string>;
  /** 네이티브 폴더 선택 다이얼로그. 선택한 절대 경로, 취소 시 null.
   * `initialDir`이 실존 디렉터리면 거기서 시작한다(`~` 확장 포함). */
  pickDirectory(initialDir?: string): Promise<string | null>;
  /** Returns an unsubscribe function. `bytes` is the raw stream byte count of
   * this batch (§#49); the renderer accumulates it on write to derive snapshot
   * offsets. Restore snapshots deliver `bytes === 0`. */
  onData(agentId: string, cb: (data: string, bytes: number) => void): () => void;
  onSessionState(cb: (e: SessionStateEvent) => void): () => void;
  onNotification(cb: (n: NotificationEvent) => void): () => void;
  onNotificationCleared(cb: (p: { agentId: string; ids: string[] }) => void): () => void;
  /** activity-event(prompt/tool) 구독. Returns an unsubscribe function. */
  onActivity(cb: (e: ActivityEvent) => void): () => void;
  /** 완료된 턴 1건을 로컬 시계열 로그에 append (fire-and-forget). */
  appendSessionTurn(record: SessionTurnRecord): void;
  /** 누적된 세션 턴 기록 전체를 읽는다(통계용). 손상된 줄은 건너뛴다. */
  loadSessionTurns(): Promise<SessionTurnRecord[]>;
  /** 세션 이벤트 시계열에서 `fromAt..=toAt`(epoch ms) 범위를 읽는다(분석 패널용).
   * 없는 파일·손상 줄은 건너뛰며 항상 성공한다. `(at, runId, seq)` 정렬. */
  loadSessionEvents(fromAt: number, toAt: number): Promise<SessionEventRecord[]>;
  /** 세션 핸드오프(unix 전용) 지원 여부. Windows 등 미지원 플랫폼은 false. */
  handoffSupported(): Promise<boolean>;
  /** 종료 시 살아있는 세션들을 `sessiond` 데몬으로 넘긴다. `snapshots`는
   * agentId -> 직렬화된 터미널 화면(스크롤백 포함, xterm SerializeAddon
   * 출력) -- 데몬이 핸드오프 이전 화면을 보관할 방법이 이것뿐이므로 실어
   * 보낸다. `renderedBytes`(agentId -> 렌더러가 실제 렌더한 raw 스트림 바이트
   * 누적치)로 스냅샷 offset(=base+누적치)을 확정해 재입양 시 유실을 없앤다(§#49).
   * 넘긴 세션 수를 반환. */
  handoffSessions(
    snapshots: Record<string, string>,
    renderedBytes: Record<string, number>
  ): Promise<number>;
  /** 부팅 시 1회 — 데몬에 남아있던 세션을 되찾는다. 미지원/데몬 없음이면 빈 배열. */
  adoptDetachedSessions(): Promise<AdoptedSessionInfo[]>;
  /** v2 상시 브로커 모드(docs/session-broker-v2-design.md)가 켜져 있는지.
   * true일 때만 렌더러가 주기 스냅샷 업로드를 활성화한다. 미지원/기본은 false. */
  sessionBrokerMode(): Promise<boolean>;
  /** 브로커 모드 주기 스냅샷 업로드 — agentId -> 직렬화된 xterm 화면. 데몬이
   * 세션별 최신 것만 보관해 앱 크래시 후 화면 복원에 대비한다. 브로커 모드가
   * 아니거나 데몬에 못 닿으면 백엔드에서 no-op. `renderedBytes`(agentId ->
   * 렌더러가 실제 렌더한 raw 스트림 바이트 누적치)로 스냅샷 offset을 확정한다(§#49). */
  uploadSessionSnapshots(
    snapshots: Record<string, string>,
    renderedBytes: Record<string, number>
  ): Promise<void>;
  /** Claude 세션 이어하기 후보 목록(agentId → 최신 1건). 메뉴를 열 때 조회한다.
   * 캡처된 적 없는 에이전트는 키가 없다(빈 객체 가능). */
  listClaudeResumeSessions(): Promise<Record<AgentId, ClaudeResumeEntry>>;
  /** 구독 사용량(rate limit) 스냅샷을 홈 디렉터리 로컬 캐시에서 읽는다(인자 없음).
   * 파싱 실패한 provider는 null이며 호출 자체는 항상 성공한다. */
  loadUsageSnapshot(): Promise<UsageSnapshot>;
  /** `root` 하위의 마크다운(.md) 파일 목록(이슈 #10). 상한 초과 시 `truncated=true`. */
  markdownListFiles(root: string): Promise<MarkdownListResult>;
  /** `root` 기준 `relPath` 파일 내용과 버전을 읽는다. 부재/범위 밖이면 reject. */
  markdownReadFile(root: string, relPath: string): Promise<MarkdownReadResult>;
  /** `expectedVersion`이 현재 버전과 다르면 "CONFLICT"로 시작하는 메시지로 reject.
   * 성공 시 갱신된 버전을 돌려준다. */
  markdownWriteFile(
    root: string,
    relPath: string,
    content: string,
    expectedVersion: string,
  ): Promise<MarkdownWriteResult>;
}
