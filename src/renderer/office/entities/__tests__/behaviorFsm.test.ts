// src/renderer/office/entities/__tests__/behaviorFsm.test.ts
//
// TDD for the Phase B office redesign: "sit at desk only while the session
// is active, otherwise hang out in the break room".
//
// `stepBehavior` is a pure transition function — no clock, no RNG owned
// internally. Every tick's elapsed time (`dtMs`), accumulated state timer
// (`c.timerMs`) and random draw (`c.rand`) are injected by the caller, so
// tests fully control time and randomness (no Math.random / real timers).
//
// Coverage:
// - sitting: an active session or a pending notification pins the
//   character at the desk forever (no linger/timer/rand can override it).
// - sitting: once inactive+no-pending, a short linger (SIT_LINGER_MS) must
//   elapse before the character gets up (deterministically — no rand
//   check), then it transitions to walking with a break-target request.
// - walking: arrival is owned by the movement controller — always stays.
// - breakIdle: session/pending going active triggers an immediate,
//   deterministic return-to-desk request, no matter the timer/rand.
// - breakIdle: otherwise, a small per-second probability (scaled by dtMs)
//   sends the character on a break-room stroll (requestBreakWander).

import { describe, expect, it } from "vitest";
import { stepBehavior, type FsmContext } from "../behaviorFsm";

const ctx = (overrides: Partial<FsmContext> = {}): FsmContext => ({
  hasPending: false,
  sessionActive: false,
  timerMs: 0,
  rand: 0.999, // 기본값: "전이 안 함" 쪽으로 치우친 난수
  ...overrides,
});

describe("stepBehavior: sitting", () => {
  it("stays sitting while the session is active, no matter the timer or rand", () => {
    const r = stepBehavior("sitting", ctx({ sessionActive: true, timerMs: 999_999, rand: 0 }), 1000);
    expect(r).toEqual({ next: "sitting" });
  });

  it("stays sitting while a notification is pending, no matter the timer or rand", () => {
    const r = stepBehavior("sitting", ctx({ hasPending: true, timerMs: 999_999, rand: 0 }), 1000);
    expect(r).toEqual({ next: "sitting" });
  });

  it("stays sitting before the linger time elapses, even once inactive", () => {
    const r = stepBehavior("sitting", ctx({ timerMs: 1999 }), 16);
    expect(r).toEqual({ next: "sitting" });
  });

  it("transitions to walking with a break-target request once the linger time has passed while inactive", () => {
    const r = stepBehavior("sitting", ctx({ timerMs: 2000 }), 16);
    expect(r).toEqual({ next: "walking", requestBreakTarget: true });
  });

  it("the sitting -> walking transition is deterministic (does not depend on rand)", () => {
    const r = stepBehavior("sitting", ctx({ timerMs: 5000, rand: 0.999999 }), 16);
    expect(r).toEqual({ next: "walking", requestBreakTarget: true });
  });
});

describe("stepBehavior: walking", () => {
  it("stays walking regardless of inputs — arrival is owned by the movement controller", () => {
    const r = stepBehavior(
      "walking",
      ctx({ timerMs: 999_999, rand: 0, hasPending: true, sessionActive: true }),
      16,
    );
    expect(r).toEqual({ next: "walking" });
  });
});

describe("stepBehavior: breakIdle", () => {
  it("stays breakIdle while inactive with no pending notification and unfavorable rand", () => {
    const r = stepBehavior("breakIdle", ctx({ timerMs: 1000, rand: 0.999999 }), 16);
    expect(r).toEqual({ next: "breakIdle" });
  });

  it("returns to desk immediately once the session becomes active, no matter the timer/rand", () => {
    const r = stepBehavior("breakIdle", ctx({ sessionActive: true, timerMs: 0, rand: 0.999999 }), 16);
    expect(r).toEqual({ next: "walking", requestReturnToDesk: true });
  });

  it("returns to desk immediately when a notification becomes pending, no matter the timer/rand", () => {
    const r = stepBehavior("breakIdle", ctx({ hasPending: true, timerMs: 0, rand: 0.999999 }), 16);
    expect(r).toEqual({ next: "walking", requestReturnToDesk: true });
  });

  it("the return-to-desk transition is deterministic (does not consult rand)", () => {
    const r = stepBehavior("breakIdle", ctx({ sessionActive: true, rand: 0 }), 16);
    expect(r).toEqual({ next: "walking", requestReturnToDesk: true });
  });

  it("strolls to another break-room tile when rand is favorable while still inactive", () => {
    const r = stepBehavior("breakIdle", ctx({ timerMs: 500, rand: 0 }), 1000);
    expect(r).toEqual({ next: "walking", requestBreakWander: true });
  });

  it("scales the per-tick stroll probability with dtMs (larger dt -> larger chance of transition)", () => {
    // BREAK_WANDER_CHANCE_PER_SEC = 0.06. At dt=1000ms the per-tick
    // probability equals 0.06, so rand=0.05 should trigger but the same
    // rand should not trigger at dt=1ms (per-tick prob ~= 0.00006).
    const long = stepBehavior("breakIdle", ctx({ rand: 0.05 }), 1000);
    expect(long.next).toBe("walking");
    expect(long.requestBreakWander).toBe(true);

    const short = stepBehavior("breakIdle", ctx({ rand: 0.05 }), 1);
    expect(short).toEqual({ next: "breakIdle" });
  });
});
