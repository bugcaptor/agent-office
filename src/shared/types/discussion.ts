// src/shared/types/discussion.ts
//
// Domain slice: 팀 토의(`discussions.json`). Rust `Discussion`/`DiscussionsFile` 미러.
// 저장은 토의 하나씩 upsert·삭제한다(렌더러 실행기와 CLI가 함께 쓴다).

import type { WorkAgent } from './workPanels';

/** 토의 참가자 CLI 종류. */
type WorkCli = WorkAgent['cli'];
import { DISCUSSION_SESSION_PREFIX } from './session';

/** 이 빌드가 읽는 최대 스키마 버전. Rust `DISCUSSIONS_SCHEMA_VERSION` 미러. */
export const DISCUSSIONS_SCHEMA_VERSION = 1;
/** 사회자를 뺀 토론자 수 상한(사회자 포함 최대 5명). Rust 미러. */
export const DISCUSSION_MAX_DEBATERS = 4;
/** 상한 범위(포함)와 기본값. Rust `MAX_*_RANGE`·`DiscussionLimits::default` 미러. */
export const DISCUSSION_LIMIT_RANGES = {
  maxRounds: { min: 1, max: 12, defaultValue: 4 },
  maxMinutes: { min: 5, max: 240, defaultValue: 30 },
  maxCostUsd: { min: 0.5, max: 50, defaultValue: 5 },
} as const;

/** 진행 상태. 준비(`ready`)에서만 내용을 고칠 수 있다. */
export type DiscussionState = "ready" | "running" | "waiting_user" | "wrapping_up" | "finished" | "paused";
export const DISCUSSION_STATES: readonly DiscussionState[] = ["ready", "running", "waiting_user", "wrapping_up", "finished", "paused"];

/** 참가자 한 명. 사회자도 이 목록에 들어 있다. */
export interface DiscussionParticipant {
  agentId: string;
  /** 이 토의에서 맡은 역할(`roles.json` id). 없거나 지워진 역할이면 역할 없음. */
  roleId?: string;
  /** 토의용 CLI·모델. 열 때 역할 → 프로필 기본값 순으로 채워 확정한다. */
  agent: WorkAgent;
}

/** 안전장치. 닿으면 사회자가 강제 정리한다. */
export interface DiscussionLimits {
  maxRounds: number;
  maxMinutes: number;
  maxCostUsd: number;
  /** 구독 사용량이 모자라면 한도 창 리셋을 기다린다. */
  usageGuard: boolean;
}

export type DiscussionUtteranceKind = "opening" | "statement" | "moderator" | "question" | "answer" | "notice";

/** 발언 하나. 실행용 기록이다. */
export interface DiscussionUtterance {
  at: number;
  round: number;
  kind: DiscussionUtteranceKind;
  /** 말한 참가자. 사용자 답·시스템 알림이면 없다. */
  agentId?: string;
  text: string;
}

export interface Discussion {
  id: string;
  topic: string;
  /** 맥락(본문). */
  context: string;
  /** 작업 폴더. 없으면 홈 폴더. */
  cwd?: string;
  participants: DiscussionParticipant[];
  moderatorAgentId: string;
  limits: DiscussionLimits;
  state: DiscussionState;
  /** 지금 라운드. 시작 전 0. */
  round: number;
  transcript: DiscussionUtterance[];
  /** 결정 메모 파일(절대 경로). 쓰기 전에는 없다. */
  decisionMemoPath?: string;
  /** 참가자 CLI 비용 누계(USD). */
  costUsd: number;
  createdAt: number;
  updatedAt: number;
  startedAt?: number;
  endedAt?: number;
}

export interface DiscussionsFile {
  version: number;
  discussions: Discussion[];
}

/** 저장을 막는 구조 문제. Rust `discussion_problem` 미러. */
export type DiscussionProblem = "id" | "topic" | "duplicate" | "moderator" | "debaters" | "too-many" | "limits";

export function discussionLimitsValid(limits: DiscussionLimits): boolean {
  const within = (key: keyof typeof DISCUSSION_LIMIT_RANGES, integer: boolean) => {
    const v = limits[key], { min, max } = DISCUSSION_LIMIT_RANGES[key];
    return typeof v === "number" && Number.isFinite(v) && (!integer || Number.isInteger(v)) && v >= min && v <= max;
  };
  return within("maxRounds", true) && within("maxMinutes", true) && within("maxCostUsd", false);
}

/** 저장할 수 있으면 null. 지워진 팀원·역할 id는 문제로 보지 않는다. */
export function discussionProblem(d: Discussion): DiscussionProblem | null {
  if (!d.id.trim()) return "id";
  if (!d.topic.trim()) return "topic";
  const ids = new Set<string>();
  for (const p of d.participants) {
    if (!p.agentId.trim() || ids.has(p.agentId)) return "duplicate";
    ids.add(p.agentId);
  }
  if (!ids.has(d.moderatorAgentId)) return "moderator";
  const debaters = d.participants.length - 1;
  if (debaters === 0) return "debaters";
  if (debaters > DISCUSSION_MAX_DEBATERS) return "too-many";
  if (!discussionLimitsValid(d.limits)) return "limits";
  return null;
}

// ── 토의 참가자 세션(전용 탭). Rust `discussion_session.rs` 미러. ──

/** 동시에 살아 있는 토의 세션 상한(사회자 포함 5명 × 동시 토의 1개). Rust `MAX_DISCUSSION_SESSIONS`. */
export const DISCUSSION_MAX_SESSIONS = 5;

/** 토의 세션 agentId. 두 id가 `[A-Za-z0-9_-]{1,64}`가 아니면 null(Rust `session_agent_id`). */
export function discussionSessionAgentId(discussionId: string, participantAgentId: string): string | null {
  const ok = (id: string) => /^[A-Za-z0-9_-]{1,64}$/.test(id);
  return ok(discussionId) && ok(participantAgentId) ? `${DISCUSSION_SESSION_PREFIX}${discussionId}.${participantAgentId}` : null;
}
/** 토의 세션 agentId → 토의 id·참가자 id. 형식이 아니면 null. */
export function parseDiscussionSessionAgentId(agentId: string): { discussionId: string; participantAgentId: string } | null {
  const match = /^discuss-([A-Za-z0-9_-]{1,64})\.([A-Za-z0-9_-]{1,64})$/.exec(agentId);
  return match ? { discussionId: match[1], participantAgentId: match[2] } : null;
}

/** 읽기 전용을 어떻게 지키는가. flags = CLI 플래그가 막음, partial = 편집 도구만 막음, prompt = 프롬프트 제한뿐. */
export type DiscussionReadOnly = "flags" | "partial" | "prompt";
/** 발언을 받는 길. submit = `office-discuss say`, last-reply = 턴의 마지막 응답. */
export type DiscussionCollection = "submit" | "last-reply";

/** Rust `read_only_mode` 미러. 근거는 그쪽 주석(CLI `--help` 실측). */
export function discussionReadOnlyMode(cli: WorkCli): DiscussionReadOnly {
  if (cli === "claude" || cli === "codex") return "flags";
  if (cli === "pi") return "partial";
  return "prompt";
}
/** Rust `collection_mode` 미러. Codex 읽기 전용 샌드박스는 제출 명령의 쓰기도 막는다. */
export function discussionCollectionMode(cli: WorkCli): DiscussionCollection {
  return cli === "codex" ? "last-reply" : "submit";
}

export interface DiscussionSessionRequest {
  discussionId: string;
  /** 참가자(캐릭터) agentId. */
  agentId: string;
  cols?: number;
  rows?: number;
  /** 첫 프롬프트 끝에 붙일 이번 차례 지시. 없으면 대기 지시로 끝난다. */
  turn?: string;
}
export interface DiscussionSessionResult {
  /** 토의 세션 agentId. */
  agentId: string;
  sessionId: string;
  readOnly: DiscussionReadOnly;
  collection: DiscussionCollection;
  /** 관찰 훅이 붙었는가. 거짓이면 턴 종료 신호가 오지 않는다. */
  observed: boolean;
}
export interface DiscussionCollectRequest {
  /** 토의 세션 agentId. */
  agentId: string;
  sessionId: string;
  cli: WorkCli;
  /** 그 차례를 보낸 시각(epoch ms, `sendDiscussionTurn`의 submittedAt). */
  since: number;
}
export type DiscussionUtteranceSource = "submitted" | "last-reply";
export interface CollectedUtterance { text: string; source: DiscussionUtteranceSource }
