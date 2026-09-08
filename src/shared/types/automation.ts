// src/shared/types/automation.ts
//
// Domain slice: automation check (kbm) — one-shot LLM CLI round-trip on a
// single tab, used to verify a session can still take a prompt and respond.
// See src/shared/types.ts for the frozen-contract overview.

/** LLM CLI to drive for the automation check. Rust `AutomationCli` mirror. */
export type AutomationCli = "claude" | "codex" | "agy" | "kilo" | "pi";

/** Current phase of the automation loop. Rust `AutomationPhase` mirror. */
export type AutomationPhase =
  | "launching"
  | "waitingStartup"
  | "injecting"
  | "watching"
  | "waitingHumanInput"
  | "confirming"
  | "timeoutDecision"
  | "settling"
  | "exiting"
  | "done"
  | "completed"
  | "interrupted"
  /** User requested cancellation, waiting for automation task shutdown. */
  | "cancelling"
  /** User stopped the run. Not a failure — the CLI and terminal are left as they are. */
  | "cancelled"
  | "failed";

/** Reason for deferring automatic prompt injection. Rust `PendingReason` mirror. */
export type PendingReason = "humanTyping" | "anotherProducer";

/** Decision choice on timeout. Rust `AutomationDecisionChoice` mirror. */
export type AutomationDecisionChoice = "extend" | "stop" | "continue";
export type AutomationRunDecisionChoice = AutomationDecisionChoice;

export interface AutomationInput { key: string; label: string; default?: string; }
export interface AutomationRepeat { maxCycles: number; }
/** How a v2 exit confirms that the shell has regained control. */
export type AutomationCliReturnMode = "auto" | "manual";
export type AutomationStep =
  | { id: string; kind: "launchCli"; cliProfileId: string; model?: string; effort?: string; startupWaitMs?: number }
  | { id: string; kind: "llmTask"; label: string; promptTemplate: string; waitTimeoutMs?: number; completionGraceMs?: number; allowEarlyComplete?: boolean }
  | { id: string; kind: "wait"; durationMs: number }
  | { id: string; kind: "confirm"; message: string }
  | { id: string; kind: "exitCli"; command: string; returnMode?: AutomationCliReturnMode; exitWaitMs?: number };
/** v1 keeps the original single-CLI scheduling; v2 repeats every paired CLI segment. */
export interface AutomationDefinition { schemaVersion: 1 | 2; id: string; revision: number; name: string; inputs: AutomationInput[]; steps: AutomationStep[]; repeat?: AutomationRepeat; workspace?: string; inputValues?: Record<string, string>; }
export interface AutomationRunEvent { at: number; kind: string; details?: string; }
export interface AutomationRunRecord { runId: string; definitionSnapshot: AutomationDefinition; inputs: Record<string, string>; workspace: string; agentId: string; status: string; outcome?: string; events: AutomationRunEvent[]; }

/** v2's explicit terminal state; absent from v1 statuses for compatibility. */
export type AutomationCliContext =
  | { state: "unknown" }
  | { state: "shell" }
  | { state: "cli"; cli: AutomationCli; launchStepId: string };

/** Runtime status of one tab's automation run. Rust `AutomationAgentStatus` mirror. */
export interface AutomationAgentStatus {
  /** Whether the automation task is still alive. */
  running: boolean;
  /** Current phase — drives the badge/banner copy. */
  phase: AutomationPhase;
  cli: AutomationCli;
  cliContext?: AutomationCliContext;
  /** Absolute path of the result file being watched (diagnostics/manual check). */
  filePath: string;
  /** Epoch ms when this run started. */
  startedAtMs: number;
  /** Unique run ID for this automation execution. */
  runId?: string;
  /** Unique ID for the currently executing step. */
  stepExecutionId?: string;
  /** Epoch ms deadline for watching phase timeout. */
  deadlineMs?: number;
  /** Unique decision ID when in timeoutDecision phase (used for atomic decide). */
  decisionId?: string;
  /** Number of times the timeout has been extended. */
  extensionCount?: number;
  /** Epoch ms when the completion marker was first observed. */
  markerObservedAtMs?: number;
  /** Reason why automatic injection is currently deferred. */
  pendingReason?: PendingReason;
  /** Epoch ms when injection deferral started. */
  pendingSinceMs?: number;
  /** Last error code, if the run ended in failure. */
  error?: string;
  definitionId?: string;
  definitionName?: string;
  sessionId?: string;
  workspace?: string;
  cycle?: number;
  stepIndex?: number;
  stepId?: string;
  outcome?: string;
  completedAtMs?: number;
  decisionMessage?: string;
  decisionReason?: "timeout" | "confirm" | "blocked" | "protocolError" | "humanInput" | "shellReady" | "cliExitTimeout" | "cliExitUnconfirmed";
  /** Epoch ms when a v2 CLI return receipt was observed. */
  cliReturnObservedAtMs?: number;
}

/** `automation_status` response — snapshot of tabs with an automation run
 * in progress or just finished. */
export interface AutomationStatus {
  agents: Record<string, AutomationAgentStatus>;
}
