// src/renderer/office/map/__tests__/mapData.test.ts
//
// Phase A (office redesign): 8-desk work zone + a break room (탕비실) whose
// BREAK_ROOM_RECT interior must be fully walkable so pathing.pickBreakTarget
// can safely rejection-sample inside it. Also pins the Tile enum's numeric
// values, since they're baked into the hardcoded GRID and any persisted data.

import { describe, expect, it } from "vitest";
import { BREAK_ROOM_RECT, OFFICE_MAP, Tile } from "../mapData";

describe("Tile enum stability", () => {
  it("keeps existing members at their original numeric values", () => {
    expect(Tile.Floor).toBe(0);
    expect(Tile.Wall).toBe(1);
    expect(Tile.DeskTop).toBe(2);
    expect(Tile.Rug).toBe(3);
  });

  it("appends new members without renumbering existing ones", () => {
    expect(Tile.Plant).toBe(4);
    expect(Tile.Counter).toBe(5);
    expect(Tile.Table).toBe(6);
  });
});

describe("BREAK_ROOM_RECT", () => {
  it("is a non-empty rectangle fully inside the map bounds", () => {
    expect(BREAK_ROOM_RECT.w).toBeGreaterThan(0);
    expect(BREAK_ROOM_RECT.h).toBeGreaterThan(0);
    expect(BREAK_ROOM_RECT.x).toBeGreaterThanOrEqual(0);
    expect(BREAK_ROOM_RECT.y).toBeGreaterThanOrEqual(0);
    expect(BREAK_ROOM_RECT.x + BREAK_ROOM_RECT.w).toBeLessThanOrEqual(OFFICE_MAP.width);
    expect(BREAK_ROOM_RECT.y + BREAK_ROOM_RECT.h).toBeLessThanOrEqual(OFFICE_MAP.height);
  });

  it("every tile inside the rect is walkable (Floor or Rug)", () => {
    for (let ty = BREAK_ROOM_RECT.y; ty < BREAK_ROOM_RECT.y + BREAK_ROOM_RECT.h; ty++) {
      for (let tx = BREAK_ROOM_RECT.x; tx < BREAK_ROOM_RECT.x + BREAK_ROOM_RECT.w; tx++) {
        const t = OFFICE_MAP.tiles[ty][tx];
        expect([Tile.Floor, Tile.Rug], `tile (${tx},${ty}) = ${t}`).toContain(t);
      }
    }
  });

  it("sits within the bottom (break room) portion of the map, below the desk rows", () => {
    expect(BREAK_ROOM_RECT.y).toBeGreaterThanOrEqual(9);
  });
});

describe("office map decoration tiles", () => {
  it("contains at least one Plant, Counter and Table tile", () => {
    const counts = { plant: 0, counter: 0, table: 0 };
    for (const row of OFFICE_MAP.tiles) {
      for (const t of row) {
        if (t === Tile.Plant) counts.plant++;
        if (t === Tile.Counter) counts.counter++;
        if (t === Tile.Table) counts.table++;
      }
    }
    expect(counts.plant).toBeGreaterThanOrEqual(2);
    expect(counts.plant).toBeLessThanOrEqual(4);
    expect(counts.counter).toBeGreaterThan(0);
    expect(counts.table).toBeGreaterThan(0);
  });

  it("has exactly 8 desks (2 rows x 4 pairs)", () => {
    expect(OFFICE_MAP.desks.length).toBe(8);
  });
});
