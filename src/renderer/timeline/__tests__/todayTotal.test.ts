// src/renderer/timeline/__tests__/todayTotal.test.ts
//
// Pure-function tests for the "오늘 일한 시간" headline helpers. No
// React/store/IPC involved — see todayTotal.ts and the design doc
// docs/superpowers/specs/2026-07-11-today-worked-total-design.md.
import { describe, expect, it } from "vitest";
import { msUntilNextLocalMidnight, startOfLocalDay, sumWorkedSince } from "../todayTotal";
import type { SessionTurnRecord } from "@shared/types";

function mkRecord(overrides: Partial<SessionTurnRecord> = {}): SessionTurnRecord {
  return {
    agentId: "a1",
    startedAt: 0,
    endedAt: 1000,
    totalMs: 1000,
    workedMs: 800,
    waitedMs: 200,
    ...overrides,
  };
}

describe("sumWorkedSince", () => {
  it("filters by endedAt >= sinceMs and sums workedMs of the included records", () => {
    const records = [
      mkRecord({ endedAt: 500, workedMs: 100 }), // excluded (before sinceMs)
      mkRecord({ endedAt: 1500, workedMs: 300 }), // included
      mkRecord({ endedAt: 2000, workedMs: 700 }), // included
    ];
    expect(sumWorkedSince(records, 1000)).toBe(1000);
  });

  it("includes a record whose endedAt exactly equals sinceMs (boundary)", () => {
    const records = [mkRecord({ endedAt: 1000, workedMs: 250 })];
    expect(sumWorkedSince(records, 1000)).toBe(250);
  });

  it("returns 0 for an empty record list", () => {
    expect(sumWorkedSince([], 1000)).toBe(0);
  });

  it("never returns negative when sinceMs excludes every record (음수 방어)", () => {
    const records = [mkRecord({ endedAt: 100, workedMs: 999 })];
    expect(sumWorkedSince(records, 100_000)).toBe(0);
  });
});

describe("startOfLocalDay", () => {
  it("zeroes the time-of-day, keeping the same local calendar date", () => {
    const now = new Date(2026, 6, 11, 15, 30, 45, 123).getTime(); // 2026-07-11 15:30:45.123 local
    const expected = new Date(2026, 6, 11, 0, 0, 0, 0).getTime();
    expect(startOfLocalDay(now)).toBe(expected);
  });

  it("is idempotent when nowMs is already local midnight", () => {
    const midnight = new Date(2026, 6, 11, 0, 0, 0, 0).getTime();
    expect(startOfLocalDay(midnight)).toBe(midnight);
  });
});

describe("msUntilNextLocalMidnight", () => {
  it("returns exactly 24h when nowMs is exactly local midnight", () => {
    const midnight = new Date(2026, 6, 11, 0, 0, 0, 0).getTime();
    expect(msUntilNextLocalMidnight(midnight)).toBe(24 * 60 * 60 * 1000);
  });

  it("equals startOfLocalDay(now) + 24h - now (DST 무시 스모크)", () => {
    const now = new Date(2026, 6, 11, 9, 15, 0, 0).getTime();
    const expected = startOfLocalDay(now) + 24 * 60 * 60 * 1000 - now;
    expect(msUntilNextLocalMidnight(now)).toBe(expected);
  });

  it("is always positive for any time within the day", () => {
    const lateNight = new Date(2026, 6, 11, 23, 59, 59, 999).getTime();
    expect(msUntilNextLocalMidnight(lateNight)).toBeGreaterThan(0);
  });
});
