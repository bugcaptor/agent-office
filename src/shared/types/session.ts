// src/shared/types/session.ts
//
// Domain slice: session lifecycle, PTY output, session-turn/event timeseries,
// resume/handoff. See src/shared/types.ts for the frozen-contract overview.

import type { AgentId, SessionId } from './common';

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
