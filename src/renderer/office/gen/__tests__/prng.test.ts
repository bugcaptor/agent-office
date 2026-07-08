// src/renderer/office/gen/__tests__/prng.test.ts
//
// Tests for the seeded PRNG core.
//
// Coverage required by the task brief:
// - hashStringToSeed + mulberry32/makeRng are fully deterministic: the same
//   seed (or seed string) produces the exact same sequence of values.
// - Different seeds produce different sequences (probabilistic sanity, not
//   a strict guarantee — but with 32-bit seeds a collision across many
//   trials would indicate a bug).
// - Distribution sanity: next() stays within [0,1), int()/range()/pick()/
//   bool() respect their documented bounds over many draws.

import { describe, expect, it } from "vitest";
import { hashStringToSeed, makeRng, mulberry32 } from "../prng";

describe("hashStringToSeed", () => {
  it("is deterministic for the same string", () => {
    expect(hashStringToSeed("agent-42")).toBe(hashStringToSeed("agent-42"));
  });

  it("returns an unsigned 32-bit integer", () => {
    const h = hashStringToSeed("some-agent-id");
    expect(Number.isInteger(h)).toBe(true);
    expect(h).toBeGreaterThanOrEqual(0);
    expect(h).toBeLessThanOrEqual(0xffffffff);
  });

  it("differs across distinct strings (sanity, not a hard guarantee)", () => {
    const seeds = new Set(
      ["alice", "bob", "carol", "dave", "erin", "frank", "grace", "heidi"].map(
        hashStringToSeed,
      ),
    );
    expect(seeds.size).toBe(8);
  });
});

describe("mulberry32", () => {
  it("produces the same sequence for the same seed", () => {
    const a = mulberry32(12345);
    const b = mulberry32(12345);
    const seqA = Array.from({ length: 10 }, () => a());
    const seqB = Array.from({ length: 10 }, () => b());
    expect(seqA).toEqual(seqB);
  });

  it("produces different sequences for different seeds", () => {
    const a = mulberry32(1);
    const b = mulberry32(2);
    const seqA = Array.from({ length: 5 }, () => a());
    const seqB = Array.from({ length: 5 }, () => b());
    expect(seqA).not.toEqual(seqB);
  });

  it("stays within [0,1)", () => {
    const r = mulberry32(999);
    for (let i = 0; i < 1000; i++) {
      const v = r();
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThan(1);
    }
  });
});

describe("makeRng", () => {
  it("gives identical sequences for the same seed across all helpers", () => {
    const rngA = makeRng(777);
    const rngB = makeRng(777);
    const drawA = [
      rngA.next(),
      rngA.int(10),
      rngA.range(-5, 5),
      rngA.pick([1, 2, 3, 4, 5]),
      rngA.bool(),
    ];
    const drawB = [
      rngB.next(),
      rngB.int(10),
      rngB.range(-5, 5),
      rngB.pick([1, 2, 3, 4, 5]),
      rngB.bool(),
    ];
    expect(drawA).toEqual(drawB);
  });

  it("gives different draws for different seeds (sanity)", () => {
    const rngA = makeRng(1);
    const rngB = makeRng(2);
    const drawA = Array.from({ length: 5 }, () => rngA.next());
    const drawB = Array.from({ length: 5 }, () => rngB.next());
    expect(drawA).not.toEqual(drawB);
  });

  it("int(max) stays within [0, max)", () => {
    const rng = makeRng(42);
    for (let i = 0; i < 500; i++) {
      const v = rng.int(7);
      expect(Number.isInteger(v)).toBe(true);
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThan(7);
    }
  });

  it("range(min, max) stays within [min, max)", () => {
    const rng = makeRng(43);
    for (let i = 0; i < 500; i++) {
      const v = rng.range(-3, 8);
      expect(v).toBeGreaterThanOrEqual(-3);
      expect(v).toBeLessThan(8);
    }
  });

  it("pick(arr) always returns an element of the array", () => {
    const rng = makeRng(44);
    const arr = ["a", "b", "c", "d"] as const;
    for (let i = 0; i < 200; i++) {
      expect(arr).toContain(rng.pick(arr));
    }
  });

  it("bool(p) respects the probability bound over many draws", () => {
    const rng = makeRng(45);
    let trueCount = 0;
    const total = 2000;
    for (let i = 0; i < total; i++) {
      if (rng.bool(0.25)) trueCount++;
    }
    const ratio = trueCount / total;
    // Loose bound — this is a sanity check on distribution, not a strict RNG test.
    expect(ratio).toBeGreaterThan(0.15);
    expect(ratio).toBeLessThan(0.35);
  });

  it("bool() defaults to p=0.5", () => {
    const rng = makeRng(46);
    let trueCount = 0;
    const total = 2000;
    for (let i = 0; i < total; i++) {
      if (rng.bool()) trueCount++;
    }
    const ratio = trueCount / total;
    expect(ratio).toBeGreaterThan(0.4);
    expect(ratio).toBeLessThan(0.6);
  });
});
