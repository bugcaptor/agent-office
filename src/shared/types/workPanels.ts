/** 업무 패널의 영속 계약. docs/work-panels-design.md, Rust types.rs 미러. */
export interface WorkAgent {
  cli: "claude" | "codex" | "agy" | "kilo" | "pi";
  model?: string;
  effort?: string;
}
export type WorkStatus = "todo" | "in_progress" | "review" | "done";
export interface WorkItem {
  id: string;
  /** 빈 문자열이면 표시 제목은 본문 앞부분에서 만든다. */
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

/** 읽기 전용 보내기 사전 점검. 확인창의 응답은 이 세션/revision에만 유효하다. */
export interface WorkSendTarget {
  sessionId?: string;
  humanInputRevision: number;
  restartRequired: boolean;
  hasUncommitted: boolean;
}
export interface WorkSendRequest {
  agentId: string;
  sessionId: string;
  humanInputRevision: number;
  freshSession: boolean;
  agent: WorkAgent;
  title: string;
  body: string;
}
export interface WorkSendResult { sessionId: string; submittedAt: number }

/** 로컬 수신함 파일. key는 파일 이름과 원본 바이트의 해시 앞 16자. */
export interface InboxEntry {
  fileName: string;
  key: string;
  title: string;
  body: string;
  assignee?: string;
}
