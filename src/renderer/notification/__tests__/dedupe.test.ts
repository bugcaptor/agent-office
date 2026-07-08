// src/renderer/notification/__tests__/dedupe.test.ts
//
// TDD for `dedupeLatestPerAgent`.
//
// Coverage:
// - T4: newest-first input dedupes to one card per agent, keeping the
//   newest occurrence and dropping older duplicates for the same agent.
// - Order of first-seen agents is preserved (no re-sorting here — the
//   caller is responsible for supplying an already newest-first list).
// - Empty input, already-deduped input, and single-agent-only input are
//   all no-ops (identity content, not necessarily identity reference).
// - Does not mutate the input array.
import { describe, expect, it } from "vitest";
import { dedupeLatestPerAgent } from "../dedupe";
import type { Notification } from "../../store/types";

let seq = 0;

function mkNotif(overrides: Partial<Notification> = {}): Notification {
  seq += 1;
  return {
    id: `n${seq}`,
    agentId: "a1",
    type: "info",
    message: "hello",
    excerpt: "hello",
    createdAt: 0,
    ...overrides,
  };
}

describe("dedupeLatestPerAgent", () => {
  it("dedupes to latest per agent, newest first (T4)", () => {
    const list = [
      mkNotif({ id: "n1", agentId: "a1", createdAt: 300 }),
      mkNotif({ id: "n2", agentId: "a2", createdAt: 250 }),
      mkNotif({ id: "n3", agentId: "a1", createdAt: 100 }), // stale a1
    ];

    const out = dedupeLatestPerAgent(list);

    expect(out.map((n) => n.agentId)).toEqual(["a1", "a2"]);
    expect(out[0].id).toBe("n1");
    expect(out[0].createdAt).toBe(300);
  });

  it("preserves first-seen order across more than two agents", () => {
    const list = [
      mkNotif({ id: "n1", agentId: "a3", createdAt: 400 }),
      mkNotif({ id: "n2", agentId: "a1", createdAt: 390 }),
      mkNotif({ id: "n3", agentId: "a2", createdAt: 380 }),
      mkNotif({ id: "n4", agentId: "a1", createdAt: 100 }),
      mkNotif({ id: "n5", agentId: "a3", createdAt: 90 }),
    ];

    const out = dedupeLatestPerAgent(list);

    expect(out.map((n) => n.id)).toEqual(["n1", "n2", "n3"]);
  });

  it("returns an empty array for empty input", () => {
    expect(dedupeLatestPerAgent([])).toEqual([]);
  });

  it("is a no-op (by content) when already deduped", () => {
    const list = [mkNotif({ id: "n1", agentId: "a1" }), mkNotif({ id: "n2", agentId: "a2" })];
    expect(dedupeLatestPerAgent(list)).toEqual(list);
  });

  it("does not mutate the input array", () => {
    const list = [
      mkNotif({ id: "n1", agentId: "a1", createdAt: 2 }),
      mkNotif({ id: "n2", agentId: "a1", createdAt: 1 }),
    ];
    const snapshot = [...list];

    dedupeLatestPerAgent(list);

    expect(list).toEqual(snapshot);
  });
});
