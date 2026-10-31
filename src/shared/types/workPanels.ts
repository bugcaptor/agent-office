/** 업무 패널의 영속 계약. Rust types.rs 미러. */
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
  /** 플랜 루트(플랜 메모 전용). 자동 진행은 이 업무를 실행하지 않는다. 아니면 생략한다. */
  plan?: boolean;
  /** 자동 진행 상태. 이 업무를 루트로 시작했을 때만 있다. */
  autoRun?: AutoRun;
  createdAt: number;
  updatedAt: number;
}
export type AutoRunState = "running" | "paused" | "finished";
export type AutoRunPauseReason =
  | "human-input" | "session-ended" | "send-failed" | "judge-failed" | "followup-limit"
  | "judge-stop" | "task-busy" | "task-changed" | "app-restarted" | "user";
export type AutoRunLogKind = "start" | "send" | "next" | "followup" | "pause" | "resume" | "finish" | "human-done" | "root-done";
export interface AutoRunLogEntry { at: number; kind: AutoRunLogKind; taskId?: string; text?: string }
/** 루트 업무 하나의 자동 진행. 렌더러가 소유하고 네이티브는 보존만 한다. */
export interface AutoRun {
  state: AutoRunState;
  /** 판단자 claude 세션 uuid. 첫 호출은 --session-id, 이후는 --resume. */
  judgeSessionId: string;
  judgeStarted: boolean;
  /** 판단자에게 마지막으로 보낸 루트 본문의 해시. 바뀌었을 때만 다시 보낸다. */
  judgedBodyHash?: string;
  /** 지금 진행 중인 태스크. */
  currentId?: string;
  /** 현재 태스크에 넣은 후속 지시 수. */
  followUps: number;
  /** 현재 태스크에 마지막으로 프롬프트를 넣은 시각. 그 전에 끝난 턴의 늦은 신호는 판단하지 않는다. */
  sentAt?: number;
  pauseReason?: AutoRunPauseReason;
  /** 판단자 메시지나 오류 코드. */
  pauseDetail?: string;
  /** 판단자 누적 비용(USD). */
  costUsd: number;
  log: AutoRunLogEntry[];
  startedAt: number;
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
/** 실행 중인 CLI에 넣는 후속 프롬프트. */
export interface WorkFollowUpRequest { agentId: string; sessionId: string; text: string }
/** 태스크 대화 열기. 네이티브가 보고·세션 로그 경로·Git 기록을 붙여 맥락 묶음을 만든다. */
export interface TaskChatRequest {
  taskId: string;
  /** 작업 폴더. 없으면 세션 기본(홈)이고 Git 기록은 찾지 않는다. */
  cwd?: string;
  agent: WorkAgent;
  /** 담당 프로필의 셸 선택 id(Windows 전용). */
  shell?: string;
  cols?: number;
  rows?: number;
  context: TaskChatContext;
}
export interface TaskChatContext {
  task: {
    /** 표시 제목(`workItemTitle`). */
    title: string;
    status: WorkStatus;
    /** 담당 캐릭터 이름. */
    assignee?: string;
    body: string;
    dependsOn?: TaskChatLink[];
    dependents?: TaskChatLink[];
    /** 업무 sessionIds를 보낸 순서대로. sentAt은 그 세션에 보낸 시각(epoch ms). */
    sessions?: { sessionId: string; agentId: string; sentAt?: number }[];
  };
  /** 이 태스크가 속한 자동 진행의 루트. */
  plan?: {
    id: string;
    title: string;
    body: string;
    state?: AutoRunState;
    pauseReason?: AutoRunPauseReason;
    pauseDetail?: string;
    /** 루트 autoRun.log에서 이 태스크의 항목만, 시간순. */
    log?: { at: number; kind: AutoRunLogKind; text?: string }[];
    /** 앞 태스크가 남긴 인계 메모 중 이 태스크에 넘어온 것. */
    handoffNotes?: { taskId: string; title: string; text: string }[];
  };
}
export interface TaskChatLink { id: string; title: string; status: WorkStatus }
export interface TaskChatResult { agentId: string; sessionId: string }
export interface AutoRunJudgeRequest { sessionId: string; resume: boolean; prompt: string; model?: string }
export interface AutoRunJudgeResult { decision: "next" | "followup" | "stop"; message: string; note: string; costUsd?: number }

/** 로컬 수신함 파일. key는 파일 이름과 원본 바이트의 해시 앞 16자. */
/** 수신함 파일을 `rejected/`로 옮길 의미 오류. `reason`은 `.error` 첫 줄의 사유 코드. */
export interface InboxRejection { fileName: string; reason: "duplicate-id" | "unknown-task" | "empty-task"; detail?: string }
/** 업무는 만들었지만 입력 일부를 버리거나 바꾼 경우. `work-inbox/warnings.log`에 한 줄로 남는다. */
export interface InboxWarning {
  fileName: string;
  taskId: string;
  code: "unknown-assignee" | "ambiguous-assignee" | "unknown-prerequisite" | "self-prerequisite" | "cyclic-prerequisite";
  detail: string;
}
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
  /** 플랜 루트(`office-task --plan`). */
  plan?: boolean;
  /** 완료 요청 파일이면 완료로 바꿀 업무 id. 이때 제목·본문은 쓰지 않는다. */
  done?: string[];
  /** 삭제 요청 파일이면 지울 업무 id. */
  delete?: string[];
  /** 수정 요청 파일이면 고칠 업무와 바꿀 필드. */
  edit?: InboxEdit;
}
/** 기존 업무 하나의 수정 요청. 있는 필드만 바꾼다. */
export interface InboxEdit {
  id: string;
  title?: string;
  body?: string;
  /** 담당 캐릭터의 프로필 id 또는 이름. 빈 문자열이면 미배정. */
  assignee?: string;
  /** 선행 업무 전체를 이 목록으로 바꾼다. 비면 모두 뺀다. */
  after?: string[];
  addAfter?: string[];
  removeAfter?: string[];
  status?: WorkStatus;
  /** 플랜 루트 표시를 켜거나 끈다. */
  plan?: boolean;
}
