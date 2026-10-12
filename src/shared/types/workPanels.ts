/** 업무 패널의 영속 계약. docs/work-panels-design.md, Rust types.rs 미러. */
export interface WorkAgent {
  cli: "claude" | "codex" | "agy" | "kilo" | "pi";
  model?: string;
  effort?: string;
}
export type WorkStatus = "todo" | "in_progress" | "review" | "done";
export interface WorkItem {
  id: string;
  title: string;
  body: string;
  assigneeAgentId?: string;
  agent?: WorkAgent;
  status: WorkStatus;
  order: number;
  sessionIds: string[];
  lastError?: string;
  createdAt: number;
  updatedAt: number;
}
export interface SessionSummary {
  sessionId: string;
  agentId: string;
  agentName: string;
  cwd?: string;
  startedAt: number;
  endedAt?: number;
  goal?: string;
  prompts: { at: number; text: string; origin?: string }[];
  workItemId?: string;
}
