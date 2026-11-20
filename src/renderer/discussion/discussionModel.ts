// src/renderer/discussion/discussionModel.ts
//
// 토의 열기 대화상자의 초안 ↔ 저장 형식 변환과 기본값 채우기(순수 함수).
//
// - 역할 기본값: 프로필 `defaultRoleId`. 사회자 칸은 moderator 스탠스 역할만,
//   토론자 칸은 moderator가 아닌 역할만 고른다(사회자는 중립).
// - CLI·모델 기본값: 역할 `agent` → 프로필 `defaultAgent` → claude.
//   역할을 바꾸면 사용자가 CLI 칸을 직접 고치지 않은 행만 다시 채운다.
import { IS_WINDOWS } from "../shared/platform";
import { nanoid } from "nanoid";
import {
  DISCUSSION_LIMIT_RANGES,
  discussionReadOnlyMode,
  type DiscussionReadOnly,
  type AgentProfile,
  type Discussion,
  type DiscussionLimits,
  type DiscussionParticipant,
  type DiscussionRole,
  type WorkAgent,
} from "@shared/types";
import { normalizeWorkAgent } from "../profile/generate";

export interface ParticipantDraft {
  agentId: string;
  roleId?: string;
  agent: WorkAgent;
  /** CLI 칸을 사람이 직접 고쳤다. 역할을 바꿔도 덮지 않는다. */
  agentTouched: boolean;
}

export interface DiscussionDraft {
  topic: string;
  context: string;
  cwd: string;
  moderator: ParticipantDraft | null;
  debaters: ParticipantDraft[];
  limits: DiscussionLimits;
}

type Profiles = Record<string, AgentProfile>;

export const defaultDiscussionLimits = (): DiscussionLimits => ({
  maxRounds: DISCUSSION_LIMIT_RANGES.maxRounds.defaultValue,
  maxMinutes: DISCUSSION_LIMIT_RANGES.maxMinutes.defaultValue,
  maxCostUsd: DISCUSSION_LIMIT_RANGES.maxCostUsd.defaultValue,
  usageGuard: true,
});

export const newDiscussionId = () => `disc-${nanoid(8)}`;

/** 그 칸에서 고를 수 있는 역할. */
export function roleChoices(roles: readonly DiscussionRole[], moderator: boolean): DiscussionRole[] {
  return roles.filter((r) => (r.stance === "moderator") === moderator);
}

/** 팀원을 그 칸에 처음 넣을 때의 역할. */
export function defaultRoleId(profile: AgentProfile | undefined, roles: readonly DiscussionRole[], moderator: boolean): string | undefined {
  const choices = roleChoices(roles, moderator);
  const own = choices.find((r) => r.id === profile?.defaultRoleId);
  if (own) return own.id;
  // 사회자는 역할이 곧 진행 지침이라 첫 사회자 역할을 넣는다. 토론자는 역할 없음(자유 의견).
  return moderator ? choices[0]?.id : undefined;
}

export function defaultAgent(roleId: string | undefined, profile: AgentProfile | undefined, roles: readonly DiscussionRole[]): WorkAgent {
  const role = roles.find((r) => r.id === roleId);
  return structuredClone(role?.agent ?? profile?.defaultAgent ?? { cli: "claude" });
}

export function participantDraft(agentId: string, profiles: Profiles, roles: readonly DiscussionRole[], moderator: boolean): ParticipantDraft {
  const profile = profiles[agentId];
  const roleId = defaultRoleId(profile, roles, moderator);
  return { agentId, roleId, agent: defaultAgent(roleId, profile, roles), agentTouched: false };
}

export function withRole(row: ParticipantDraft, roleId: string | undefined, profiles: Profiles, roles: readonly DiscussionRole[]): ParticipantDraft {
  return { ...row, roleId, agent: row.agentTouched ? row.agent : defaultAgent(roleId, profiles[row.agentId], roles) };
}

/** 새 토의 초안. 사회자는 기본 역할이 사회자인 첫 팀원, 없으면 첫 팀원. 토론자는 비워 둔다. */
export function newDiscussionDraft(order: readonly string[], profiles: Profiles, roles: readonly DiscussionRole[]): DiscussionDraft {
  const moderatorRoles = new Set(roleChoices(roles, true).map((r) => r.id));
  const ids = order.filter((id) => profiles[id]);
  const moderatorId = ids.find((id) => moderatorRoles.has(profiles[id].defaultRoleId ?? "")) ?? ids[0];
  return {
    topic: "",
    context: "",
    cwd: (moderatorId && profiles[moderatorId].cwd) || "",
    moderator: moderatorId ? participantDraft(moderatorId, profiles, roles, true) : null,
    debaters: [],
    limits: defaultDiscussionLimits(),
  };
}

/** 저장된 토의를 고치기 위한 초안. 저장된 CLI 값은 확정값이라 역할을 바꿔도 덮지 않는다. */
export function draftFromDiscussion(d: Discussion): DiscussionDraft {
  const row = (p: DiscussionParticipant): ParticipantDraft => ({ ...structuredClone(p), agentTouched: true });
  const moderator = d.participants.find((p) => p.agentId === d.moderatorAgentId);
  return {
    topic: d.topic,
    context: d.context,
    cwd: d.cwd ?? "",
    moderator: moderator ? row(moderator) : null,
    debaters: d.participants.filter((p) => p !== moderator).map(row),
    limits: { ...d.limits },
  };
}

/** 초안을 저장 형식으로. `base`가 있으면 그 토의를 고친다(실행 기록은 그대로 둔다). */
export function discussionFromDraft(draft: DiscussionDraft, now: number, base?: Discussion): Discussion {
  const participant = ({ agentId, roleId, agent }: ParticipantDraft): DiscussionParticipant =>
    ({ agentId, ...(roleId ? { roleId } : {}), agent: normalizeWorkAgent(agent) });
  const rows = [...(draft.moderator ? [draft.moderator] : []), ...draft.debaters];
  const cwd = draft.cwd.trim();
  const { cwd: _cwd, ...kept }: Partial<Discussion> = base ?? {};
  return {
    id: newDiscussionId(), state: "ready", round: 0, transcript: [], costUsd: 0, createdAt: now,
    ...kept,
    topic: draft.topic.trim(),
    context: draft.context.trim(),
    ...(cwd ? { cwd } : {}),
    participants: rows.map(participant),
    moderatorAgentId: draft.moderator?.agentId ?? "",
    limits: { ...draft.limits },
    updatedAt: now,
  };
}

/** 빈 작업 폴더는 홈을 사용한다. Windows에서는 드라이브 루트 또는 UNC 경로가 필요하다. */
export function validDiscussionCwd(cwd?: string, windows = IS_WINDOWS): boolean {
  if (!cwd) return true;
  if (!windows) return cwd.startsWith("/");
  return /^[A-Za-z]:[\\/]/.test(cwd) || /^[\\/]{2}[^\\/]+[\\/][^\\/]+/.test(cwd);
}

/** 사람이 고칠 수 있는 상태. 실행 중인 토의는 3·4단계 실행기가 쥐고 있다. */
export const discussionEditable = (d: Discussion) => d.state === "ready";
/** 지울 수 있는 상태. 진행·대기·정리·멈춤 중에는 실행기가 붙잡고 있을 수 있어 막는다. 결론 확인은 탭을 닫은 뒤다. */
export const discussionDeletable = (d: Discussion) => d.state === "ready" || d.state === "review" || d.state === "finished";

const READ_ONLY_RANK: Record<DiscussionReadOnly, number> = { flags: 0, partial: 1, prompt: 2 };
/** 토의 전체의 읽기 전용 수준: 참가자 중 가장 약한 것. 카드에 보인다. */
export function discussionReadOnlySummary(d: Pick<Discussion, "participants">): DiscussionReadOnly {
  return d.participants
    .map((p) => discussionReadOnlyMode(p.agent.cli))
    .reduce<DiscussionReadOnly>((worst, mode) => (READ_ONLY_RANK[mode] > READ_ONLY_RANK[worst] ? mode : worst), "flags");
}

/** 진행 시간이 흐르는 상태(진행 중·정리 중). 멈춤·사람 대기·사용량 대기는 세지 않는다. */
export function discussionTicking(state: Discussion["state"]): boolean {
  return state === "running" || state === "wrapping_up";
}

/** 지금까지의 진행 시간(ms). 누계에 지금 흐르는 구간을 더한다. */
export function discussionElapsedMs(d: Pick<Discussion, "state" | "activeMs" | "activeSince">, now = Date.now()): number {
  const open = discussionTicking(d.state) && d.activeSince !== undefined ? Math.max(0, now - d.activeSince) : 0;
  return (d.activeMs ?? 0) + open;
}
