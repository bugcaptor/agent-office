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
  /** 외부(논리) 세션의 전이인가 — 앱 밖 터미널/tmux에 붙인 세션은 PTY가 없어
   * 렌더러가 터미널 대신 placeholder를 그린다. PTY 세션은 Rust가 `None`으로
   * 두어 JSON에서 통째로 빠진다(additive, 기존 계약 무변경). */
  external?: boolean;
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
  /** tmux 자동 호스팅 여부. 유닉스 전용, 부재/false = 기존처럼 직접 셸을 띄운다. */
  tmuxHost?: boolean;
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
  | "stop"
  | "usage";

/**
 * 한 턴에서 소비한 토큰 사용량. 과거 파일은 `kind="stop"` 레코드에(그것도 추출에
 * 성공한 경우에만) 실렸고, 신규 파일은 `kind="usage"` 레코드에 실린다(`kind="stop"`엔
 * 더 이상 안 실린다) — 소비자는 **kind가 아니라 tokens 유무**로 합산해야 신구
 * 파일을 모두 커버한다. Rust `SessionEventTokens`(camelCase) 미러.
 *
 * 모든 필드가 옵션이다 — 제공자/버전마다 실어 주는 항목이 다르고, 추출에
 * 실패한 항목은 조용히 생략한다. 값이 하나도 없으면 `tokens` 자체를 싣지
 * 않는다. `input`은 **캐시를 제외한** 순수 입력 토큰이다(Claude
 * `input_tokens`, Codex `input_tokens - cached_input_tokens`).
 */
export interface SessionEventTokens {
  /** 캐시 히트/기록을 제외한 입력 토큰. */
  input?: number;
  /** 출력 토큰(추론 토큰 포함). */
  output?: number;
  /** 캐시에서 읽은 입력 토큰(할인 단가). */
  cacheRead?: number;
  /** 캐시에 기록한 입력 토큰(할증 단가). Codex는 구분이 없어 항상 생략. */
  cacheWrite?: number;
  /** 그 턴의 대표 모델 ID(예: "claude-opus-5", "gpt-5.4"). 비용 환산 키. */
  model?: string;
}

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
  /**
   * 그 턴이 쓴 토큰(추출 성공 시에만). 과거 파일은 kind="stop"에, 신규 파일은
   * kind="usage"에 실린다 — 집계는 kind가 아니라 tokens 유무로 판단해야
   * 신구 파일을 모두 커버한다. 추출 실패/부재는 undefined.
   */
  tokens?: SessionEventTokens;
  /**
   * kind="prompt"일 때 그 프롬프트의 출처. **봇 주입만** 표식이 붙고 사람이 친
   * 프롬프트는 필드 자체가 없다. 봇은 별도 세션을 띄우지 않고 이미 떠 있는
   * 터미널에 프롬프트를 밀어넣으므로(백엔드 `bot/runner.rs::inject`) 세션·
   * agentId로는 구분할 수 없고, 구분선이 턴 단위에만 있다.
   *
   * 이 필드가 생기기 전의 과거 이벤트에는 당연히 없다 — 전부 사람 몫으로
   * 집계되는 것이 의도된 동작이다.
   */
  origin?: "bot";
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
  /**
   * 훅이 알려 준 전사(JSONL) 절대 경로. 세션 로그가 이 경로로 대화를 끌어온다
   * (`CLAUDE_CONFIG_DIR`을 옮긴 환경에서 경로 추측이 통하지 않으므로).
   * 옛 기록에는 없을 수 있다.
   */
  transcriptPath?: string;
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

/**
 * 세션 로그 파일 하나의 요약 정보. Mirrors Rust `SessionLogItem` (camelCase).
 * 설계: docs/session-log-design.md §6. 본문은 담지 않는다 — 파일이 수십 MB일 수
 * 있어 목록에는 메타데이터만 싣는다.
 */
export interface SessionLogItem {
  /** 로그 파일 절대 경로. 후속 동작(열기·학습자료)의 키다. */
  path: string;
  /** 헤더에 기록된 sessionId. 헤더가 없으면 빈 문자열. */
  sessionId: string;
  /** 세션 시작 시각(epoch ms). */
  startedAt: number;
  /** 마지막 기록 시각(epoch ms). startedAt과의 차이가 대략의 지속 시간이다. */
  modifiedAt: number;
  bytes: number;
  /** 세션의 시작 작업 폴더. 없으면 빈 문자열. */
  cwd: string;
}

/** `list_session_logs`의 한 페이지. `total`은 페이징 전 전체 개수. */
export interface SessionLogPage {
  total: number;
  items: SessionLogItem[];
}

/**
 * `generate_study_material`의 결과. `dir`/`fileName`을 그대로 마크다운
 * 뷰어(`markdownReadFile`)에 넘기면 인앱 미리보기가 열린다.
 */
export interface StudyMaterialResult {
  path: string;
  dir: string;
  fileName: string;
}
