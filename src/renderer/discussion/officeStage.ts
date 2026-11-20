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
import { isActiveDiscussion, participantsModeratorFirst } from "./discussionModel";

const SPEAKING: ReadonlySet<DiscussionState> = new Set<DiscussionState>(["running", "wrapping_up"]);

export function officeDiscussionStage(discussions: readonly Discussion[]): OfficeDiscussionStage | null {
  // 동시 토의는 하나뿐이지만, 깨진 데이터로 둘이 보이면 가장 최근에 바뀐 쪽을 쓴다.
  const d = discussions.filter(isActiveDiscussion).sort((a, b) => b.updatedAt - a.updatedAt)[0];
  if (!d) return null;
  const speaking = new Set(
    SPEAKING.has(d.state)
      ? (d.run?.turns ?? []).filter((t) => t.sentAt !== undefined && t.text === undefined).map((t) => t.agentId)
      : [],
  );
  const ids = participantsModeratorFirst(d).map((p) => p.agentId);
  const members: OfficeDiscussionMember[] = [...new Set(ids)].map((agentId) => ({
    agentId,
    moderator: agentId === d.moderatorAgentId,
    speaking: speaking.has(agentId),
  }));
  return { discussionId: d.id, members };
}

export function discussionStageEquals(a: OfficeDiscussionStage | null, b: OfficeDiscussionStage | null): boolean {
  if (a === b) return true;
  if (!a || !b || a.discussionId !== b.discussionId || a.members.length !== b.members.length) return false;
  return a.members.every((m, i) => {
    const n = b.members[i];
    return m.agentId === n.agentId && m.moderator === n.moderator && m.speaking === n.speaking;
  });
}
