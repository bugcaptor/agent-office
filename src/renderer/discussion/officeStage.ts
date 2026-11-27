// src/renderer/discussion/officeStage.ts
//
// 토의 목록 → 오피스 씬 연출 정보(`OfficeDiscussionStage`). 순수 함수 — 스토어·Pixi 의존 없음.
// sessionBridge가 토의 스토어를 구독해 이 값이 실제로 바뀔 때만 씬에 전한다(실행기는 commit을 자주 한다).
//
// 규칙
// - 연출 대상은 끝나지 않은 토의(`isActiveDiscussion`: 준비·결론 확인·끝남 제외). 멈춘
//   토의(paused, 사용량 대기 포함)도 탭이 열려 있고 이어 갈 토의라 회의실에 남는다.
// - 순서 = 회의실 의자 순서: 사회자 먼저, 이어서 토론자(저장 순서).
// - 말하는 중 = 진행·정리 중에 차례를 보냈고(sentAt) 발언을 아직 받지 않은 참가자. 동시 첫 라운드면
//   토론자 전원이 함께 말한다.
import type { Discussion, DiscussionState } from "@shared/types";
import type { OfficeDiscussionMember, OfficeDiscussionStage } from "../office/bus";
import type { AgentProfile } from "../office/types";
import { isActiveDiscussion, participantsModeratorFirst } from "./discussionModel";

const SPEAKING: ReadonlySet<DiscussionState> = new Set<DiscussionState>(["running", "wrapping_up"]);

/** 연출할 토의. 동시 토의는 하나뿐이지만, 깨진 데이터로 둘이 보이면 가장 최근에 바뀐 쪽을 쓴다. */
export function stagedDiscussion(discussions: readonly Discussion[]): Discussion | null {
  return discussions.filter(isActiveDiscussion).sort((a, b) => b.updatedAt - a.updatedAt)[0] ?? null;
}

/** 말하는 중인 참가자 → 차례를 받은 시각(epoch ms). Rust `ctl discuss status`의 speakingSince와 같은 규칙. */
export function speakingSince(d: Discussion): Map<string, number> {
  const out = new Map<string, number>();
  if (!SPEAKING.has(d.state)) return out;
  for (const t of d.run?.turns ?? []) {
    if (t.sentAt !== undefined && t.text === undefined && !out.has(t.agentId)) out.set(t.agentId, t.sentAt);
  }
  return out;
}

/**
 * `remoteProfile`은 다른 머신 참가자(`기기id:캐릭터id`)의 외형을 준다(머신 연동 호스트). 이 머신 캐릭터는 씬이 이미
 * 프로필을 안다. 외형을 모르는 원격 참가자는 분신 없이 보드에만 나온다.
 */
export function officeDiscussionStage(
  discussions: readonly Discussion[],
  remoteProfile: (agentId: string) => AgentProfile | undefined = () => undefined,
): OfficeDiscussionStage | null {
  const d = stagedDiscussion(discussions);
  if (!d) return null;
  const speaking = speakingSince(d);
  const ids = participantsModeratorFirst(d).map((p) => p.agentId);
  const members: OfficeDiscussionMember[] = [...new Set(ids)].map((agentId) => {
    const profile = agentId.includes(":") ? remoteProfile(agentId) : undefined;
    return {
      agentId,
      moderator: agentId === d.moderatorAgentId,
      speaking: speaking.has(agentId),
      ...(profile ? { profile } : {}),
    };
  });
  return { discussionId: d.id, members };
}

const looksKey = (p: AgentProfile | undefined) => (p ? `${p.seed}|${p.archetype ?? ""}|${JSON.stringify(p.colors ?? null)}|${p.name}` : "");

export function discussionStageEquals(a: OfficeDiscussionStage | null, b: OfficeDiscussionStage | null): boolean {
  if (a === b) return true;
  if (!a || !b || a.discussionId !== b.discussionId || a.members.length !== b.members.length) return false;
  return a.members.every((m, i) => {
    const n = b.members[i];
    return m.agentId === n.agentId && m.moderator === n.moderator && m.speaking === n.speaking && looksKey(m.profile) === looksKey(n.profile);
  });
}
