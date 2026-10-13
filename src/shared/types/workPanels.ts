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
  /** 선행 업무 id. 모두 완료돼야 보낼 수 있다. 비면 생략한다. */
  dependsOn?: string[];
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
  /** 생산자가 정한 업무 id(`office-task`가 출력). 없으면 `inbox-<key>`. */
  id?: string;
  /** 선행 업무 id. */
  after?: string[];
  /** 완료 요청 파일이면 완료로 바꿀 업무 id. 이때 제목·본문은 쓰지 않는다. */
  done?: string[];
}
