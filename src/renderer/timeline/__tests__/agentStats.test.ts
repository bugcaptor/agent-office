// src/renderer/timeline/__tests__/agentStats.test.ts
//
// Pure-function tests for the per-agent stats helpers. No React/store/IPC —
// see agentStats.ts and docs/superpowers/specs/2026-07-11-per-agent-stats-design.md.
import { describe, expect, it } from "vitest";
import { aggregateByAgent, buildAgentStatsRows } from "../agentStats";
import { sumWorkedSince } from "../todayTotal";
import type { SessionTurnRecord } from "@shared/types";

function mkRecord(overrides: Partial<SessionTurnRecord> = {}): SessionTurnRecord {
  return { agentId: "a1", startedAt: 0, endedAt: 1000, totalMs: 1000, workedMs: 800, waitedMs: 200, ...overrides };
}

describe("aggregateByAgent", () => {
  it("groups workedMs by agentId and separates today (endedAt >= sinceMs)", () => {
    const records = [
      mkRecord({ agentId: "a", endedAt: 500, workedMs: 100 }), // a: not today
      mkRecord({ agentId: "a", endedAt: 1500, workedMs: 200 }), // a: today
      mkRecord({ agentId: "b", endedAt: 3000, workedMs: 400 }), // b: today
    ];
    const agg = aggregateByAgent(records, 1000);
    expect(agg.a).toEqual({ totalWorkedMs: 300, todayWorkedMs: 200 });
    expect(agg.b).toEqual({ totalWorkedMs: 400, todayWorkedMs: 400 });
  });

  it("counts a record whose endedAt exactly equals sinceMs as today (boundary)", () => {
    const agg = aggregateByAgent([mkRecord({ agentId: "a", endedAt: 1000, workedMs: 250 })], 1000);
    expect(agg.a).toEqual({ totalWorkedMs: 250, todayWorkedMs: 250 });
  });

  it("returns {} for an empty record list", () => {
    expect(aggregateByAgent([], 1000)).toEqual({});
  });
});

describe("buildAgentStatsRows", () => {
  const agents = { a: { name: "Alice" }, b: { name: "Bob", clockedOut: true } };

  it("adds the clamped live delta to both total and today", () => {
    const disk = { a: { totalWorkedMs: 1000, todayWorkedMs: 400 } };
    const rows = buildAgentStatsRows(disk, { a: 700 }, { a: 200 }, agents);
    // delta = max(0, 700 - 200) = 500, added to both.
    expect(rows[0]).toEqual({ agentId: "a", label: "Alice", departed: false, totalWorkedMs: 1500, todayWorkedMs: 900 });
  });

  it("clamps a negative delta to 0 when memory < baseline (agent removed)", () => {
    const disk = { a: { totalWorkedMs: 1000, todayWorkedMs: 400 } };
    const rows = buildAgentStatsRows(disk, {}, { a: 500 }, agents);
    expect(rows[0]).toMatchObject({ totalWorkedMs: 1000, todayWorkedMs: 400 });
  });

  it("labels a disk-only agent missing from the roster as departed", () => {
    const disk = { deadbeefcafe1234: { totalWorkedMs: 300, todayWorkedMs: 0 } };
    const rows = buildAgentStatsRows(disk, {}, {}, {});
    expect(rows[0]).toMatchObject({ agentId: "deadbeefcafe1234", label: "deadbeef… (퇴사)", departed: true });
  });

  it("includes an in-memory-only new agent with no disk record", () => {
    const rows = buildAgentStatsRows({}, { a: 300 }, { a: 0 }, agents);
    expect(rows[0]).toMatchObject({ agentId: "a", label: "Alice", totalWorkedMs: 300, todayWorkedMs: 300 });
  });

  it("keeps the name for a clocked-out agent still in the roster", () => {
    const rows = buildAgentStatsRows({ b: { totalWorkedMs: 50, todayWorkedMs: 0 } }, {}, {}, agents);
    expect(rows[0]).toMatchObject({ label: "Bob", departed: false });
  });

  it("sorts by total desc, then today desc, then label", () => {
    const disk = {
      a: { totalWorkedMs: 100, todayWorkedMs: 10 },
      b: { totalWorkedMs: 300, todayWorkedMs: 10 },
      c: { totalWorkedMs: 100, todayWorkedMs: 50 },
    };
    const rows = buildAgentStatsRows(disk, {}, {}, { a: { name: "A" }, b: { name: "B" }, c: { name: "C" } });
    expect(rows.map((r) => r.agentId)).toEqual(["b", "c", "a"]);
  });

  it("holds the headline invariant: Σ today === sumWorkedSince(all, since) + Σ delta", () => {
    const records = [
      mkRecord({ agentId: "a", endedAt: 2000, workedMs: 200 }),
      mkRecord({ agentId: "b", endedAt: 500, workedMs: 300 }), // not today
      mkRecord({ agentId: "b", endedAt: 2000, workedMs: 100 }),
    ];
    const since = 1000;
    const disk = aggregateByAgent(records, since);
    const memory = { a: 500, b: 50 };
    const baseline = { a: 200, b: 20 };
    const rows = buildAgentStatsRows(disk, memory, baseline, { a: { name: "A" }, b: { name: "B" } });
    const sumToday = rows.reduce((s, r) => s + r.todayWorkedMs, 0);
    const sumDelta = Math.max(0, 500 - 200) + Math.max(0, 50 - 20);
    expect(sumToday).toBe(sumWorkedSince(records, since) + sumDelta);
  });
});
