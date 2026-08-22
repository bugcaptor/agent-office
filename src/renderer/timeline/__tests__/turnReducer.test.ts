// src/renderer/timeline/__tests__/turnReducer.test.ts
//
// TDD for the pure turn state machine. Every timestamp is a
// backend `at` value; the reducer never reads a wall clock.
import { describe, expect, it } from "vitest";
import { initialTurnState, reduceTurn, type AgentTurnState, type TurnInput } from "../turnReducer";

/** Fold a sequence of inputs from a fresh initial state. */
function run(...inputs: TurnInput[]): AgentTurnState {
  return inputs.reduce(reduceTurn, initialTurnState());
}

describe("reduceTurn", () => {
  it("normal turn: prompt → tool×N → stop accounts all time as worked", () => {
    const s = run(
      { kind: "prompt", at: 1000 },
      { kind: "tool", at: 3000 },
      { kind: "tool", at: 5000 },
      { kind: "stop", at: 9000 }
    );
    expect(s.phase).toBe("idle");
    expect(s.turns).toBe(1);
    expect(s.totalMs).toBe(8000); // 9000-1000
    expect(s.waitedMs).toBe(0);
    expect(s.workedMs).toBe(8000);
  });

  it("turn with waiting: prompt → notification → tool → stop splits waited", () => {
    const s = run(
      { kind: "prompt", at: 0 },
      { kind: "notification", at: 2000 }, // working→waiting
      { kind: "tool", at: 5000 }, // waiting→working, waited += 3000
      { kind: "stop", at: 6000 }
    );
    expect(s.totalMs).toBe(6000);
    expect(s.waitedMs).toBe(3000);
    expect(s.workedMs).toBe(3000); // 6000 - 3000
    expect(s.turns).toBe(1);
  });

  it("stop while waiting: notification→stop gap counts fully as waited", () => {
    const s = run(
      { kind: "prompt", at: 0 },
      { kind: "notification", at: 1000 }, // →waiting
      { kind: "stop", at: 4000 } // settle while waiting: waited += 4000-1000
    );
    expect(s.totalMs).toBe(4000);
    expect(s.waitedMs).toBe(3000);
    expect(s.workedMs).toBe(1000); // 4000 - 3000
    expect(s.turns).toBe(1);
  });

  it("lost stop + session settle force-closes the open turn", () => {
    const s = run(
      { kind: "prompt", at: 0 },
      { kind: "tool", at: 2000 },
      { kind: "settle", at: 5000 } // exited/disposed — same accounting as stop
    );
    expect(s.phase).toBe("idle");
    expect(s.totalMs).toBe(5000);
    expect(s.workedMs).toBe(5000);
    expect(s.turns).toBe(1);
  });

  it("duplicate prompt mid-turn settles the old turn and starts a new one", () => {
    const s = run(
      { kind: "prompt", at: 0 },
      { kind: "tool", at: 2000 },
      { kind: "prompt", at: 3000 } // closes turn1 (0..3000), opens turn2 at 3000
    );
    expect(s.phase).toBe("working");
    expect(s.turns).toBe(1); // one settled so far
    expect(s.totalMs).toBe(3000);
    expect(s.workedMs).toBe(3000);
    expect(s.turnStartedAt).toBe(3000);
    // closing turn2 too:
    const s2 = reduceTurn(s, { kind: "stop", at: 4000 });
    expect(s2.turns).toBe(2);
    expect(s2.totalMs).toBe(4000);
  });

  it("prompt while waiting also settles the old turn (waited gap counted)", () => {
    const s = run(
      { kind: "prompt", at: 0 },
      { kind: "notification", at: 1000 }, // →waiting
      { kind: "prompt", at: 4000 } // settle turn1 as waiting (waited 1000..4000), new turn
    );
    expect(s.phase).toBe("working");
    expect(s.turns).toBe(1);
    expect(s.waitedMs).toBe(3000);
    expect(s.workedMs).toBe(1000); // 4000 - 3000
    expect(s.turnStartedAt).toBe(4000);
  });

  it("idle ignores notification / stop / settle", () => {
    const base = initialTurnState();
    for (const kind of ["notification", "stop", "settle"] as const) {
      const s = reduceTurn(base, { kind, at: 1000 });
      expect(s).toEqual(base); // no change, no phantom turn
    }
  });

  it("idle + tool reopens a working turn (post-completion resume, 이슈 #39)", () => {
    // Stop settled the turn to idle; a later tool proves work resumed.
    const settled = run(
      { kind: "prompt", at: 0 },
      { kind: "stop", at: 1000 } // → idle, turns=1
    );
    expect(settled.phase).toBe("idle");
    expect(settled.turns).toBe(1);

    const resumed = reduceTurn(settled, { kind: "tool", at: 5000 });
    expect(resumed.phase).toBe("working");
    expect(resumed.turnStartedAt).toBe(5000);
    // The reopened turn hasn't settled yet — accumulators unchanged.
    expect(resumed.turns).toBe(1);
    expect(resumed.totalMs).toBe(1000);

    // The reopened turn settles normally on the next stop.
    const done = reduceTurn(resumed, { kind: "stop", at: 8000 });
    expect(done.turns).toBe(2);
    expect(done.totalMs).toBe(4000); // 1000 + (8000-5000)
    expect(done.workedMs).toBe(4000);
  });

  it("consecutive notifications keep the first waitingSince", () => {
    const s = run(
      { kind: "prompt", at: 0 },
      { kind: "notification", at: 1000 }, // waitingSince=1000
      { kind: "notification", at: 2000 }, // ignored (already waiting)
      { kind: "tool", at: 4000 } // waited += 4000-1000 = 3000
    );
    const done = reduceTurn(s, { kind: "stop", at: 4000 });
    expect(done.waitedMs).toBe(3000);
  });

  it("tool in working state is a no-op heartbeat (no accounting change)", () => {
    const s = run(
      { kind: "prompt", at: 0 },
      { kind: "tool", at: 1000 }
    );
    expect(s.phase).toBe("working");
    expect(s.waitedInTurnMs).toBe(0);
    expect(s.turnStartedAt).toBe(0);
  });
});

// pi(pi.dev) 확장이 보내는 실제 이벤트 시퀀스(pi v0.84.2 spy 확장 실측)를 그대로
// 리듀서에 먹여 상태 판정을 고정한다. pi에는 권한 게이트가 없어 notification이
// 없고(waiting 부재), 완료는 agent_settled 1회다.
describe("reduceTurn — pi(pi.dev) 이벤트 시퀀스", () => {
  it("before_agent_start → tool_execution_start×2 → agent_settled 를 한 턴으로 정산한다", () => {
    // 실측 시퀀스: session_start → input → before_agent_start → agent_start →
    // turn_start → tool_execution_start(read) → tool_execution_end →
    // turn_end → turn_start → turn_end → agent_end → agent_settled.
    // 확장이 훅으로 내보내는 것은 prompt / tool / tool / stop 넷뿐이다.
    const s = run(
      { kind: "prompt", at: 1_000 }, // before_agent_start
      { kind: "tool", at: 2_400 }, // tool_execution_start(read)
      { kind: "tool", at: 2_500 }, // tool_execution_start(bash)
      { kind: "stop", at: 4_000 } // agent_settled
    );
    expect(s.phase).toBe("idle");
    expect(s.turns).toBe(1);
    expect(s.totalMs).toBe(3_000);
    expect(s.workedMs).toBe(3_000);
    expect(s.waitedMs).toBe(0); // pi에는 입력 대기(notification) 신호가 없다
  });

  it("agent_end로 정산하면 재시도·압축·큐 소진 구간이 idle로 튄다 — agent_settled여야 하는 이유", () => {
    // pi의 _runAgentPrompt는 자동 재시도/컨텍스트 압축/큐잉된 후속 메시지가
    // 있으면 agent_end 후에도 agent.continue()로 루프를 더 돈다.
    // (a) 잘못된 매핑: agent_end마다 stop → 아직 일하는 중인데 턴이 닫힌다.
    const wrong = run(
      { kind: "prompt", at: 0 },
      { kind: "stop", at: 1_000 }, // agent_end #1 (실제로는 재시도 예정)
      { kind: "tool", at: 5_000 }, // 재시도 후 다시 도구 사용 → 새 턴이 열림
      { kind: "stop", at: 6_000 } // agent_end #2
    );
    expect(wrong.turns).toBe(2); // 한 요청이 두 턴으로 쪼개진다
    expect(wrong.totalMs).toBe(2_000); // 1_000~5_000 공백이 통째로 유실

    // (b) 올바른 매핑: agent_settled 1회만 stop.
    const right = run(
      { kind: "prompt", at: 0 },
      { kind: "tool", at: 5_000 },
      { kind: "stop", at: 6_000 } // agent_settled
    );
    expect(right.turns).toBe(1);
    expect(right.totalMs).toBe(6_000);
    expect(right.workedMs).toBe(6_000);
  });

  it("ESC 중단: agent_settled는 finally에서 항상 나므로 열린 턴이 남지 않는다", () => {
    const s = run(
      { kind: "prompt", at: 0 },
      { kind: "tool", at: 1_000 },
      { kind: "stop", at: 2_000 } // 중단이어도 agent_settled 발화
    );
    expect(s.phase).toBe("idle");
    expect(s.turnStartedAt).toBeNull();
    expect(s.turns).toBe(1);
  });

  it("정상 종료 후의 session_shutdown stop은 idle 상태를 건드리지 않는다", () => {
    const settled = run(
      { kind: "prompt", at: 0 },
      { kind: "stop", at: 1_000 }
    );
    // 확장은 열린 턴이 없으면 shutdown stop을 아예 보내지 않지만, 보내지더라도
    // 리듀서가 idle에서 stop을 무시하므로 헛턴이 생기지 않는다.
    const after = reduceTurn(settled, { kind: "stop", at: 9_000 });
    expect(after).toEqual(settled);
  });
});
