// src/renderer/office/gen/__tests__/parts.test.ts
//
// Tests for pixel part data.
//
// Coverage:
// - Every PixelRows constant is a 16x16 grid (16 rows, 16 chars each) —
//   required by the compositor's fixed CELL size and by hand-edited pixel
//   art staying in bounds.
// - Empty layers stay transparent so the base layer can show through.

import { describe, expect, it } from "vitest";

import {
  ACCESSORY_VARIANTS,
  BODY_BASE_FRONT,
  CLOTHES_VARIANTS,
  EMPTY16,
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
});

describe("hair variants", () => {
  it("every hair layer is 16x16", () => {
    for (const [key, rows] of Object.entries(HAIR_VARIANTS)) expect16x16(rows, `hair.${key}`);
  });
});

describe("clothes variants", () => {
  it("every clothes layer is 16x16", () => {
    for (const [key, rows] of Object.entries(CLOTHES_VARIANTS)) expect16x16(rows, `clothes.${key}`);
  });
  it("'plain' is fully transparent (base shirt shows through unmodified)", () => {
    expect(CLOTHES_VARIANTS.plain.every((r) => r === "................")).toBe(true);
  });
});

describe("accessory variants", () => {
  it("every accessory layer is 16x16", () => {
    for (const [key, rows] of Object.entries(ACCESSORY_VARIANTS)) expect16x16(rows, `accessory.${key}`);
  });
  it("'none' is fully transparent", () => {
    expect(ACCESSORY_VARIANTS.none.every((r) => r === "................")).toBe(true);
  });
});
