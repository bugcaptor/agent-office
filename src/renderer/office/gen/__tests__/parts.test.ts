// src/renderer/office/gen/__tests__/parts.test.ts
//
// Tests for pixel part data.
//
// Coverage:
// - Every PixelRows constant is a 16x16 grid (16 rows, 16 chars each) —
//   required by the compositor's fixed CELL size and by hand-edited pixel
//   art staying in bounds.
// - Variant registries expose the documented MVP counts (hair 4, clothes 3,
//   accessory 4 incl. "none").

import { describe, expect, it } from "vitest";

import {
  ACCESSORY_KEYS,
  ACCESSORY_VARIANTS,
  BODY_BASE_FRONT,
  BODY_VARIANTS_COUNT,
  CLOTHES_KEYS,
  CLOTHES_VARIANTS,
  EMPTY16,
  HAIR_KEYS,
  HAIR_VARIANTS,
  LEGS_WALK_A,
  LEGS_WALK_B,
  type PixelRows,
} from "../parts";

function expect16x16(rows: PixelRows, label: string) {
  expect(rows.length, `${label}: row count`).toBe(16);
  for (const row of rows) {
    expect(row.length, `${label}: row width`).toBe(16);
  }
}

describe("EMPTY16", () => {
  it("returns a fully transparent 16x16 grid", () => {
    const rows = EMPTY16();
    expect16x16(rows, "EMPTY16");
    expect(rows.every((r) => r === "................")).toBe(true);
  });
});

describe("body/legs pixel data", () => {
  it("BODY_BASE_FRONT is 16x16", () => {
    expect16x16(BODY_BASE_FRONT, "BODY_BASE_FRONT");
  });
  it("LEGS_WALK_A / LEGS_WALK_B are 16x16", () => {
    expect16x16(LEGS_WALK_A, "LEGS_WALK_A");
    expect16x16(LEGS_WALK_B, "LEGS_WALK_B");
  });
  it("BODY_VARIANTS_COUNT matches the MVP spec (1)", () => {
    expect(BODY_VARIANTS_COUNT).toBe(1);
  });
});

describe("hair variants", () => {
  it("has the 4 documented MVP variants, each 16x16", () => {
    expect(HAIR_KEYS.sort()).toEqual(["bald", "bob", "short", "spiky"].sort());
    for (const key of HAIR_KEYS) expect16x16(HAIR_VARIANTS[key], `hair.${key}`);
  });
});

describe("clothes variants", () => {
  it("has the 3 documented MVP variants, each 16x16", () => {
    expect(CLOTHES_KEYS.sort()).toEqual(["plain", "stripe", "vest"].sort());
    for (const key of CLOTHES_KEYS) expect16x16(CLOTHES_VARIANTS[key], `clothes.${key}`);
  });
  it("'plain' is fully transparent (base shirt shows through unmodified)", () => {
    expect(CLOTHES_VARIANTS.plain.every((r) => r === "................")).toBe(true);
  });
});

describe("accessory variants", () => {
  it("has the documented MVP variants incl. 'none', each 16x16", () => {
    expect(ACCESSORY_KEYS.sort()).toEqual(["cap", "glasses", "headset", "none"].sort());
    for (const key of ACCESSORY_KEYS) expect16x16(ACCESSORY_VARIANTS[key], `accessory.${key}`);
  });
  it("'none' is fully transparent", () => {
    expect(ACCESSORY_VARIANTS.none.every((r) => r === "................")).toBe(true);
  });
});
