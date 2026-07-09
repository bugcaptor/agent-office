// src/renderer/office/world/__tests__/pathing.test.ts
//
// Tests for grid <-> pixel conversion,
// walkability checks and wander-target selection.
//
// `pickWanderTarget` takes an injected `rand: () => number` — tests supply
// deterministic sequences instead of Math.random, so both the "found a
// walkable tile" and "gave up after 12 tries, fell back to `near`" paths
// are exercised deterministically.

import { describe, expect, it } from "vitest";
import { isWalkable, pickBreakTarget, pickWanderTarget, tileCenterPx, tileKey } from "../pathing";
import { BREAK_ROOM_RECT, OFFICE_MAP, Tile, TILE_SIZE, type OfficeMap } from "../../map/mapData";

const makeMap = (rows: string[]): OfficeMap => {
  const tiles = rows.map((row) =>
    [...row].map((ch) => (ch === "F" ? Tile.Floor : Tile.Wall)),
  );
  return { width: tiles[0].length, height: tiles.length, tiles, desks: [] };
};

describe("tileCenterPx", () => {
  it("converts a grid position to the pixel center of that tile", () => {
    expect(tileCenterPx({ tx: 0, ty: 0 })).toEqual({ x: TILE_SIZE / 2, y: TILE_SIZE / 2 });
    expect(tileCenterPx({ tx: 3, ty: 2 })).toEqual({
      x: 3 * TILE_SIZE + TILE_SIZE / 2,
      y: 2 * TILE_SIZE + TILE_SIZE / 2,
    });
  });
});

describe("isWalkable", () => {
  const m = makeMap(["WWW", "WFW", "WWW"]);

  it("is true only for Floor tiles inside bounds", () => {
    expect(isWalkable(m, 1, 1)).toBe(true);
  });

  it("is false for non-floor tiles (e.g. walls)", () => {
    expect(isWalkable(m, 0, 0)).toBe(false);
  });

  it("is false outside the map bounds in every direction", () => {
    expect(isWalkable(m, -1, 1)).toBe(false);
    expect(isWalkable(m, 1, -1)).toBe(false);
    expect(isWalkable(m, 3, 1)).toBe(false);
    expect(isWalkable(m, 1, 3)).toBe(false);
  });

  it("is true for Rug tiles too (a floor covering, not an obstacle)", () => {
    const withRug: OfficeMap = {
      width: 3,
      height: 3,
      tiles: [
        [Tile.Wall, Tile.Wall, Tile.Wall],
        [Tile.Wall, Tile.Rug, Tile.Wall],
        [Tile.Wall, Tile.Wall, Tile.Wall],
      ],
      desks: [],
    };
    expect(isWalkable(withRug, 1, 1)).toBe(true);
  });
});

describe("pickWanderTarget", () => {
  it("returns a walkable tile near the given position when one exists within the retry budget", () => {
    const m = makeMap(["WWWWW", "WFFFW", "WFFFW", "WFFFW", "WWWWW"]);
    // rand() * 2 - 1 == 0 -> offset 0 each axis -> stays at `near`, which is walkable.
    const rand = () => 0.5;
    const target = pickWanderTarget(m, { tx: 2, ty: 2 }, rand, 1);
    expect(isWalkable(m, target.tx, target.ty)).toBe(true);
  });

  it("falls back to the original position after exhausting the retry budget with no walkable candidate", () => {
    const m = makeMap(["WWW", "WFW", "WWW"]);
    // rand() * 2 - 1 == 1 -> offset == +radius each axis -> always lands out of
    // bounds / on a wall for this tiny map, every one of the 12 attempts.
    const rand = () => 1;
    const near = { tx: 1, ty: 1 };
    const target = pickWanderTarget(m, near, rand, 3);
    expect(target).toEqual(near);
  });

  it("stays within the requested radius of `near` when it succeeds", () => {
    const m = makeMap([
      "WWWWWWWWW",
      "WFFFFFFFW",
      "WFFFFFFFW",
      "WFFFFFFFW",
      "WFFFFFFFW",
      "WFFFFFFFW",
      "WFFFFFFFW",
      "WFFFFFFFW",
      "WWWWWWWWW",
    ]);
    const rand = () => 0.9; // offset -> round((0.9*2-1)*3) = round(2.4) = 2
    const near = { tx: 4, ty: 4 };
    const target = pickWanderTarget(m, near, rand, 3);
    expect(Math.abs(target.tx - near.tx)).toBeLessThanOrEqual(3);
    expect(Math.abs(target.ty - near.ty)).toBeLessThanOrEqual(3);
  });
});

describe("pickBreakTarget", () => {
  it("returns a pixel point whose tile lies inside BREAK_ROOM_RECT on the real office map", () => {
    const rand = () => 0.4; // deterministic mid-range pick
    const target = pickBreakTarget(OFFICE_MAP, rand);
    expect(target).not.toBeNull();
    const tx = Math.floor(target!.x / TILE_SIZE);
    const ty = Math.floor(target!.y / TILE_SIZE);
    expect(tx).toBeGreaterThanOrEqual(BREAK_ROOM_RECT.x);
    expect(tx).toBeLessThan(BREAK_ROOM_RECT.x + BREAK_ROOM_RECT.w);
    expect(ty).toBeGreaterThanOrEqual(BREAK_ROOM_RECT.y);
    expect(ty).toBeLessThan(BREAK_ROOM_RECT.y + BREAK_ROOM_RECT.h);
    expect(isWalkable(OFFICE_MAP, tx, ty)).toBe(true);
  });

  it("returns points covering the rect's tiles for a spread of rand() values", () => {
    const seen = new Set<string>();
    for (let i = 0; i < 20; i++) {
      const r = i / 20;
      const target = pickBreakTarget(OFFICE_MAP, () => r);
      expect(target).not.toBeNull();
      seen.add(`${Math.floor(target!.x / TILE_SIZE)},${Math.floor(target!.y / TILE_SIZE)}`);
    }
    expect(seen.size).toBeGreaterThan(1);
  });

  it("skips tiles present in the occupied set and lands on a free one", () => {
    // First attempt draws (0, 0) -> (11, 10), which is occupied; second
    // attempt draws (0.2, 0) -> (12, 10), free -> returned.
    const occupied = new Set([tileKey(11, 10)]);
    const q = [0, 0, 0.2, 0];
    const rand = () => (q.length ? q.shift()! : 0.999);
    const target = pickBreakTarget(OFFICE_MAP, rand, occupied);
    expect(target).not.toBeNull();
    expect(Math.floor(target!.x / TILE_SIZE)).toBe(12);
    expect(Math.floor(target!.y / TILE_SIZE)).toBe(10);
  });

  it("returns null when every break-room tile is occupied", () => {
    const occupied = new Set<string>();
    for (let ty = BREAK_ROOM_RECT.y; ty < BREAK_ROOM_RECT.y + BREAK_ROOM_RECT.h; ty++) {
      for (let tx = BREAK_ROOM_RECT.x; tx < BREAK_ROOM_RECT.x + BREAK_ROOM_RECT.w; tx++) {
        occupied.add(tileKey(tx, ty));
      }
    }
    let i = 0;
    const rand = () => ((i += 7) % 20) / 20; // spread of values, all attempts hit occupied tiles
    expect(pickBreakTarget(OFFICE_MAP, rand, occupied)).toBeNull();
  });

  it("falls back to null when the rect has no walkable tile within the retry budget", () => {
    const allWalls: OfficeMap = {
      width: 5,
      height: 5,
      tiles: Array.from({ length: 5 }, () => Array(5).fill(Tile.Wall)),
      desks: [],
    };
    const rand = () => 0.5;
    expect(pickBreakTarget(allWalls, rand)).toBeNull();
  });
});
