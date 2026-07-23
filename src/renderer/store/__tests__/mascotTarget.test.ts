// pickMascotTarget(이슈 #72) — 마스코트가 보여줄 캐릭터 1명 선정 규칙.
import { describe, expect, it } from "vitest";
import { pickMascotTarget } from "../selectors";
import type { TurnPhase } from "../../timeline/turnReducer";

const turn = (phase: TurnPhase, turnStartedAt: number | null = null) => ({
  phase,
  turnStartedAt,
});

const agents = (...ids: string[]) =>
  Object.fromEntries(ids.map((id) => [id, {}])) as Record<string, { clockedOut?: boolean }>;

describe("pickMascotTarget", () => {
  it("활동이 없으면 아무도 고르지 않는다", () => {
    expect(
      pickMascotTarget({
        notifications: [],
        timeTracking: { a: turn("idle") },
        agents: agents("a"),
        prevAgentId: null,
      }),
    ).toEqual({ agentId: null, hasPending: false, working: false });
  });

  it("알림 대기가 working보다 우선하고, 가장 최근 알림이 이긴다", () => {
    const pick = pickMascotTarget({
      // newest-first (appStore.pushNotification이 앞에 붙인다)
      notifications: [{ agentId: "b" }, { agentId: "c" }],
      timeTracking: { a: turn("working", 100) },
      agents: agents("a", "b", "c"),
      prevAgentId: null,
    });
    expect(pick).toEqual({ agentId: "b", hasPending: true, working: false });
  });

  it("알림 대기 중인 캐릭터가 동시에 working이면 working도 함께 보고한다", () => {
    const pick = pickMascotTarget({
      notifications: [{ agentId: "a" }],
      timeTracking: { a: turn("working", 10) },
      agents: agents("a"),
      prevAgentId: null,
    });
    expect(pick).toEqual({ agentId: "a", hasPending: true, working: true });
  });

  it("waiting(질문 대기)만으로는 활동으로 치지 않는다", () => {
    expect(
      pickMascotTarget({
        notifications: [],
        timeTracking: { a: turn("waiting", 50) },
        agents: agents("a"),
        prevAgentId: null,
      }).agentId,
    ).toBeNull();
  });

  it("working끼리는 sticky — 더 최근에 턴을 시작한 캐릭터가 있어도 안 뺏는다", () => {
    const pick = pickMascotTarget({
      notifications: [],
      timeTracking: { a: turn("working", 100), b: turn("working", 999) },
      agents: agents("a", "b"),
      prevAgentId: "a",
    });
    expect(pick).toEqual({ agentId: "a", hasPending: false, working: true });
  });

  it("sticky 대상이 일을 멈추면 turnStartedAt이 가장 최근인 working으로 넘어간다", () => {
    const pick = pickMascotTarget({
      notifications: [],
      timeTracking: { a: turn("idle"), b: turn("working", 5), c: turn("working", 42) },
      agents: agents("a", "b", "c"),
      prevAgentId: "a",
    });
    expect(pick).toEqual({ agentId: "c", hasPending: false, working: true });
  });

  it("알림은 sticky를 즉시 인터럽트한다", () => {
    const pick = pickMascotTarget({
      notifications: [{ agentId: "b" }],
      timeTracking: { a: turn("working", 100), b: turn("idle") },
      agents: agents("a", "b"),
      prevAgentId: "a",
    });
    expect(pick.agentId).toBe("b");
  });

  it("퇴근했거나 프로필이 사라진 에이전트는 모든 단계에서 제외된다", () => {
    const pick = pickMascotTarget({
      notifications: [{ agentId: "gone" }, { agentId: "off" }],
      timeTracking: { off: turn("working", 100), a: turn("working", 1) },
      agents: { off: { clockedOut: true }, a: {} },
      prevAgentId: "off",
    });
    expect(pick).toEqual({ agentId: "a", hasPending: false, working: true });
  });
});
