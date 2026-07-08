// src/renderer/timeline/turnReducer.ts
//
// Pure per-agent turn state machine for session time tracking.
// `(state, input) → state`, with no React/zustand/clock dependencies so it's
// deterministically testable and trivially snapshot-able for a future
// persistence layer. Every duration is computed from the backend `at`
// timestamps carried by inputs — the renderer's wall clock is never read here.

export type TurnPhase = "idle" | "working" | "waiting";

export interface AgentTurnState {
  phase: TurnPhase;
  /** Backend ms when the current open turn started; null when idle. */
  turnStartedAt: number | null;
  /** Backend ms when the current waiting stretch began; null when not waiting. */
  waitingSince: number | null;
  /** Waited ms accumulated *within the current open turn* (settled into waitedMs on turn close). */
  waitedInTurnMs: number;
  // ---- session-run lifetime accumulators (per agent) ----
  /** Total turn time = sum of (close - start) over settled turns. */
  totalMs: number;
  /** Worked time = totalMs - waitedMs. */
  workedMs: number;
  /** Waited time across settled turns. */
  waitedMs: number;
  /** Count of settled turns. */
  turns: number;
}

export type TurnInputKind = "prompt" | "tool" | "notification" | "stop" | "settle";
export interface TurnInput {
  kind: TurnInputKind;
  /** Backend now_ms() epoch ms. */
  at: number;
}

export function initialTurnState(): AgentTurnState {
  return {
    phase: "idle",
    turnStartedAt: null,
    waitingSince: null,
    waitedInTurnMs: 0,
    totalMs: 0,
    workedMs: 0,
    waitedMs: 0,
    turns: 0,
  };
}

/**
 * Closes the currently-open turn at `at`, folding its accounting into the
 * lifetime accumulators and returning the reset (idle) accumulator fields.
 * Caller guarantees phase is working|waiting (turnStartedAt non-null).
 */
function settleOpenTurn(s: AgentTurnState, at: number): AgentTurnState {
  const start = s.turnStartedAt ?? at;
  // If we were waiting, the waitingSince→at gap is waited time too.
  const waitedInTurn =
    s.phase === "waiting" && s.waitingSince !== null
      ? s.waitedInTurnMs + (at - s.waitingSince)
      : s.waitedInTurnMs;
  const turnTotal = at - start;
  return {
    ...s,
    phase: "idle",
    turnStartedAt: null,
    waitingSince: null,
    waitedInTurnMs: 0,
    totalMs: s.totalMs + turnTotal,
    waitedMs: s.waitedMs + waitedInTurn,
    workedMs: s.workedMs + (turnTotal - waitedInTurn),
    turns: s.turns + 1,
  };
}

/** Opens a fresh turn at `at` (idle → working). */
function openTurn(s: AgentTurnState, at: number): AgentTurnState {
  return { ...s, phase: "working", turnStartedAt: at, waitingSince: null, waitedInTurnMs: 0 };
}

export function reduceTurn(s: AgentTurnState, input: TurnInput): AgentTurnState {
  const { kind, at } = input;

  // prompt always closes any open turn and starts a new one — regardless of
  // phase. The only waiting→working signal is `tool`.
  if (kind === "prompt") {
    const closed = s.phase === "idle" ? s : settleOpenTurn(s, at);
    return openTurn(closed, at);
  }

  if (s.phase === "idle") {
    // idle ignores tool/notification/stop/settle (no half-turn accounting).
    return s;
  }

  switch (kind) {
    case "tool":
      // waiting → working: fold the waited gap. working: no-op heartbeat.
      if (s.phase === "waiting" && s.waitingSince !== null) {
        return {
          ...s,
          phase: "working",
          waitingSince: null,
          waitedInTurnMs: s.waitedInTurnMs + (at - s.waitingSince),
        };
      }
      return s;
    case "notification":
      // working → waiting. Already waiting: keep the first waitingSince.
      if (s.phase === "working") {
        return { ...s, phase: "waiting", waitingSince: at };
      }
      return s;
    case "stop":
    case "settle":
      return settleOpenTurn(s, at);
    default:
      return s;
  }
}
