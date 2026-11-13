// src/renderer/office/map/__tests__/mapData.test.ts
//
// The break room (탕비실) rectangle must remain valid: its
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

});
