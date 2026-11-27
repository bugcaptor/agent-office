// src/shared/types/discussion.ts
//
// Domain slice: 팀 토의(`discussions.json`). Rust `Discussion`/`DiscussionsFile` 미러.
// 저장은 토의 하나씩 upsert·삭제한다(렌더러 실행기와 CLI가 함께 쓴다).

import type { WorkAgent } from './workPanels';
import type { UsageWindowKind } from './usage';

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

/**
 * 진행 상태. 준비(`ready`)에서만 내용을 고칠 수 있다. `review`는 사회자가 정리해 결정 메모를 남기고 탭을 닫은 뒤
 * 사람이 결론을 확인하기를 기다리는 상태다(확인하면 `finished`).
 */
export type DiscussionState = "ready" | "running" | "waiting_user" | "wrapping_up" | "review" | "finished" | "paused";
export const DISCUSSION_STATES: readonly DiscussionState[] = ["ready", "running", "waiting_user", "wrapping_up", "review", "finished", "paused"];

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

/**
 * 진행 단계. opening = 토론자 전원 동시·비공개 발언, moderator = 사회자 지시, speakers = 사회자가 지목한 순서대로 발언,
 * user = 사회자가 사람에게 물었다(답을 기다림), wrap_up = 정리. 상한에 닿으면 사회자에게 정리 차례 하나를 준다(`forced`).
 */
export type DiscussionPhase = "opening" | "moderator" | "speakers" | "user" | "wrap_up";
export const DISCUSSION_PHASES: readonly DiscussionPhase[] = ["opening", "moderator", "speakers", "user", "wrap_up"];

/**
 * 멈춘 이유. user = 사람이 멈춤, restarted = 앱을 다시 열었다(토의 세션은 재시작 때 버려진다),
 * human-input = 탭에 쓰다 만 사람 입력이 있다, open-failed·send-failed = 세션 열기·차례 보내기 실패,
 * session-ended = 차례를 기다리던 세션이 끝났다, no-utterance = 재촉해도 발언이 없다,
 * moderator-invalid = 재촉해도 사회자 지시를 읽을 수 없다, usage-wait = 구독 사용량이 모자랄 것 같아 한도 창 리셋을 기다린다,
 * link-offline = 다른 머신 참가자의 머신이 끊겼다(다시 붙으면 스스로 이어 간다).
 */
export type DiscussionPauseReason =
  | "user" | "restarted" | "human-input" | "open-failed" | "send-failed" | "session-ended" | "no-utterance" | "moderator-invalid"
  | "usage-wait" | "link-offline";
export const DISCUSSION_PAUSE_REASONS: readonly DiscussionPauseReason[] = [
  "user", "restarted", "human-input", "open-failed", "send-failed", "session-ended", "no-utterance", "moderator-invalid", "usage-wait",
  "link-offline",
];

/** 강제 정리를 부른 상한. Rust `DiscussionLimitReason` 미러. */
export type DiscussionLimitReason = "rounds" | "time" | "cost";
export const DISCUSSION_LIMIT_REASONS: readonly DiscussionLimitReason[] = ["rounds", "time", "cost"];

/** 사용량 대기. 차례를 받을 참가자의 구독이 한도에 닿을 것 같아 기다린다. Rust `DiscussionUsageWait` 미러. */
export interface DiscussionUsageWait {
  /** 기다리게 한 참가자(캐릭터) agentId. */
  agentId: string;
  provider: "claude" | "codex";
  kind: UsageWindowKind;
  label: string | null;
  /** 판단 때의 지금 사용률 추정과 이번 차례가 더할 것으로 본 양(%p, 모르면 null). */
  nowPercent: number;
  addPercent: number | null;
  limit: number;
  /** 다시 볼 시각(epoch ms). */
  until: number;
  resetsAtMs: number | null;
}

/** 이번 단계의 차례 하나. 보내기 전에는 `sentAt`이 없고, 발언을 받으면 `text`가 생긴다. */
export interface DiscussionTurn {
  /** 말할 참가자(캐릭터) agentId. */
  agentId: string;
  /** 사회자가 이 사람에게 던진 질문(speakers 단계). */
  question?: string;
  /** 차례를 넣은 세션. */
  sessionId?: string;
  /** 차례를 넣은 시각(epoch ms). 발언 수집의 since이자 턴 종료 신호의 기준이다. */
  sentAt?: number;
  /** 발언이 없거나 지시를 못 읽어 재촉을 한 번 보냈다. */
  nudged?: boolean;
  /** 받은 발언. 동시 라운드는 전원이 낼 때까지 여기에만 두고 공개하지 않는다. */
  text?: string;
}

/** 참가자 세션이 지금까지 받은 기록 길이. 세션이 바뀌면(다시 열면) 처음부터 다시 전한다. */
export interface DiscussionSeen {
  sessionId: string;
  /** 이 세션에 전한 `transcript` 항목 수. */
  count: number;
}

/** 실행기 상태. 시작 전과 끝난 뒤에는 없다. 앱을 다시 열어도 이어 가도록 토의에 함께 저장한다. */
export interface DiscussionRun {
  phase: DiscussionPhase;
  /** opening = 토론자 전원, moderator = 사회자 한 명, speakers = 지목 순서. user·wrap_up은 비어 있다. */
  turns: DiscussionTurn[];
  /** 참가자 agentId → 그 세션이 본 기록. */
  seen: Record<string, DiscussionSeen>;
  /** user 단계에서 사회자가 사람에게 한 질문. */
  question?: string;
  pauseReason?: DiscussionPauseReason;
  /** 멈춘 이유의 덧붙임(오류 코드·참가자 이름 등). */
  pauseDetail?: string;
  /** 상한에 닿아 사회자에게 강제 정리를 맡겼다(wrap_up 단계). 없으면 사회자가 스스로 정리했다. */
  forced?: DiscussionLimitReason;
  /** 강제 정리를 시작한 때의 진행 시간(ms). 정리 차례의 유예를 여기서 잰다. */
  forcedAtMs?: number;
  usageWait?: DiscussionUsageWait;
  /** 사람이 사용량 대기를 건너뛰었다. 다음 차례 하나를 검사 없이 보낸다. */
  usageOverride?: boolean;
  /** 메모 저장 중 재시작해도 같은 본문으로 마무리한다. */
  pendingMemo?: string;
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
  /** 결정 메모 파일(절대 경로, `<app_data>/discussions/memos/<id>.md`). 쓰기 전에는 없다. */
  decisionMemoPath?: string;
  /** 참가자 CLI 비용 누계(USD). */
  costUsd: number;
  createdAt: number;
  updatedAt: number;
  startedAt?: number;
  endedAt?: number;
  /** 진행 시간 누계(ms). 진행 중·정리 중일 때만 흐르고, 멈춤·사람 대기·사용량 대기는 세지 않는다. */
  activeMs?: number;
  /** 지금 흐르는 구간의 시작(epoch ms). 진행 중·정리 중일 때만 있다. */
  activeSince?: number;
  /** 실행기 상태. 시작 전·끝난 뒤에는 없다. */
  run?: DiscussionRun;
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

/** 발언을 받는 길. submit = `office-discuss say`, last-reply = 턴의 마지막 응답. */
export type DiscussionCollection = "submit" | "last-reply";

/** Rust `collection_mode` 미러. 마지막 응답을 읽을 수 있는 CLI(Claude 전사·Codex Stop 훅)는 그것을 쓴다. */
export function discussionCollectionMode(cli: WorkCli): DiscussionCollection {
  return cli === "claude" || cli === "codex" ? "last-reply" : "submit";
}

export interface DiscussionSessionRequest {
  discussionId: string;
  /** 참가자(캐릭터) agentId. */
  agentId: string;
  cols?: number;
  rows?: number;
  /** 첫 프롬프트 끝에 붙일 이번 차례 지시. 차례를 받을 때만 세션을 연다. */
  turn: string;
  /** 첫 프롬프트 언어(차례 지시와 같게). 없으면 화면 언어 설정으로 정한다. */
  lang?: "ko" | "en";
}
export interface DiscussionSessionResult {
  /** 토의 세션 agentId. */
  agentId: string;
  sessionId: string;
  /** 첫 프롬프트를 넣은 시각(epoch ms). 첫 프롬프트에 차례 지시를 실었다면 그 발언 수집의 since다. */
  submittedAt: number;
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
