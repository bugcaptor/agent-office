// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { DISCUSSION_STATES, type Discussion } from "@shared/types";
import { discussionStageEquals, isStagedDiscussion, officeDiscussionStage } from "../officeStage";
import { isActiveDiscussion } from "../discussionRun";

const now = 1_800_000_000_000;
function discussion(patch: Partial<Discussion> = {}): Discussion {
  return {
    id: "d1", topic: "t", context: "", moderatorAgentId: "m",
    participants: [
      { agentId: "a", agent: { cli: "claude" } },
      { agentId: "m", agent: { cli: "claude" } },
      { agentId: "b", agent: { cli: "codex" } },
    ],
    limits: { maxRounds: 4, maxMinutes: 30, maxCostUsd: 5, usageGuard: true }, state: "running", round: 1, transcript: [],
    costUsd: 0, createdAt: now, updatedAt: now, ...patch,
  };
}

describe("officeDiscussionStage", () => {
  it("연출 대상 상태는 실행기의 '끝나지 않은 토의' 판정과 같다", () => {
    for (const state of DISCUSSION_STATES) expect(isStagedDiscussion({ state }), state).toBe(isActiveDiscussion({ state }));
  });

  it("준비·결론 확인·끝남이면 null", () => {
    for (const state of ["ready", "review", "finished"] as const) expect(officeDiscussionStage([discussion({ state })])).toBeNull();
    expect(officeDiscussionStage([])).toBeNull();
  });

  it("사회자를 맨 앞(0번 의자)에 두고 토론자는 저장 순서를 따른다", () => {
    const stage = officeDiscussionStage([discussion()])!;
    expect(stage.discussionId).toBe("d1");
    expect(stage.members.map((m) => [m.agentId, m.moderator])).toEqual([["m", true], ["a", false], ["b", false]]);
  });

  it("동시 첫 라운드: 차례를 받고 아직 내지 않은 토론자 전원이 말하는 중", () => {
    const stage = officeDiscussionStage([discussion({
      run: { phase: "opening", seen: {}, turns: [{ agentId: "a", sentAt: now }, { agentId: "b", sentAt: now, text: "done" }] },
    })])!;
    expect(stage.members.filter((m) => m.speaking).map((m) => m.agentId)).toEqual(["a"]);
  });

  it("사회자 차례와 강제 정리 차례도 말하는 중으로 본다", () => {
    const moderator = officeDiscussionStage([discussion({ run: { phase: "moderator", seen: {}, turns: [{ agentId: "m", sentAt: now }] } })])!;
    expect(moderator.members.find((m) => m.agentId === "m")!.speaking).toBe(true);
    const wrap = officeDiscussionStage([discussion({ state: "wrapping_up", run: { phase: "wrap_up", forced: "time", seen: {}, turns: [{ agentId: "m", sentAt: now }] } })])!;
    expect(wrap.members.find((m) => m.agentId === "m")!.speaking).toBe(true);
  });

  it("아직 보내지 않은 차례, 멈춤·사용자 대기 중에는 아무도 말하지 않지만 회의실엔 남는다", () => {
    const unsent = officeDiscussionStage([discussion({ run: { phase: "speakers", seen: {}, turns: [{ agentId: "a", question: "q" }] } })])!;
    expect(unsent.members.some((m) => m.speaking)).toBe(false);
    for (const state of ["paused", "waiting_user"] as const) {
      const stage = officeDiscussionStage([discussion({ state, run: { phase: "speakers", seen: {}, turns: [{ agentId: "a", sentAt: now }] } })]);
      expect(stage?.members).toHaveLength(3);
      expect(stage!.members.some((m) => m.speaking)).toBe(false);
    }
  });

  it("끝나지 않은 토의가 여럿 보이면(깨진 데이터) 가장 최근에 바뀐 것", () => {
    const stage = officeDiscussionStage([discussion({ id: "old" }), discussion({ id: "new", updatedAt: now + 1 }), discussion({ id: "done", state: "finished", updatedAt: now + 2 })]);
    expect(stage?.discussionId).toBe("new");
  });

  it("discussionStageEquals: 내용이 같으면 새 객체여도 같다", () => {
    const a = officeDiscussionStage([discussion()]);
    expect(discussionStageEquals(a, officeDiscussionStage([discussion({ updatedAt: now + 5, costUsd: 1 })]))).toBe(true);
    expect(discussionStageEquals(a, null)).toBe(false);
    expect(discussionStageEquals(null, null)).toBe(true);
    const speaking = officeDiscussionStage([discussion({ run: { phase: "moderator", seen: {}, turns: [{ agentId: "m", sentAt: now }] } })]);
    expect(discussionStageEquals(a, speaking)).toBe(false);
  });
});
