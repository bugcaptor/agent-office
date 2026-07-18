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
