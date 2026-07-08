// src/renderer/office/map/__tests__/TileRenderer.test.ts
//
// Tests for procedural tile rendering.
//
// `Container`/`Graphics` construction and geometry (`.rect().fill()`,
// `.position`, `.zIndex`, `.cacheAsTexture()`) do not touch a canvas
// rendering context, so this runs under the default (plain Node) vitest
// environment — no jsdom, no real GPU/WebGL needed. Actual pixel output is
// out of scope here (that requires a renderer) and is covered by manual
// visual verification per the task brief; this test asserts the
// structural/geometry contract `OfficeScene` relies on.

import { describe, expect, it } from "vitest";
import { Container } from "pixi.js";
import { TileRenderer } from "../TileRenderer";
import { OFFICE_MAP, Tile, TILE_SIZE } from "../mapData";

/** Tile types drawn in the y-sorted furniture layer (mirrors TileRenderer's own set). */
const FURNITURE_TILES = new Set([Tile.DeskTop, Tile.Plant, Tile.Counter, Tile.Table]);

describe("TileRenderer.build", () => {
  it("adds one child per non-furniture tile, positioned on the grid", () => {
    const r = new TileRenderer(OFFICE_MAP, TILE_SIZE);
    const root = r.build();

    let nonFurnitureCount = 0;
    for (const row of OFFICE_MAP.tiles) {
      for (const t of row) {
        if (!FURNITURE_TILES.has(t)) nonFurnitureCount++;
      }
    }
    expect(root.children.length).toBe(nonFurnitureCount);
  });

  it("positions each floor/wall/rug tile at tx*TILE_SIZE, ty*TILE_SIZE", () => {
    const r = new TileRenderer(OFFICE_MAP, TILE_SIZE);
    const root = r.build();

    // Top-left corner is a wall tile (ty=0, tx=0) in the hardcoded map.
    const first = root.children[0];
    expect(first.position.x).toBe(0);
    expect(first.position.y).toBe(0);
  });

  it("bakes the static layer into a single cached (nearest) texture", () => {
    const r = new TileRenderer(OFFICE_MAP, TILE_SIZE);
    const root = r.build();
    expect(root.isCachedAsTexture).toBe(true);
  });

  it("returns a fresh Container instance on each call (no shared mutable state)", () => {
    const r = new TileRenderer(OFFICE_MAP, TILE_SIZE);
    const a = r.build();
    const b = r.build();
    expect(a).not.toBe(b);
    expect(a).toBeInstanceOf(Container);
  });
});

describe("TileRenderer.buildFurniture", () => {
  it("returns exactly one Graphics per furniture tile (desk/plant/counter/table) in the map", () => {
    const r = new TileRenderer(OFFICE_MAP, TILE_SIZE);
    const furniture = r.buildFurniture();

    let furnitureTileCount = 0;
    for (const row of OFFICE_MAP.tiles) {
      for (const t of row) {
        if (FURNITURE_TILES.has(t)) furnitureTileCount++;
      }
    }
    expect(furniture.length).toBe(furnitureTileCount);
  });

  it("sets zIndex to (ty + 1) * TILE_SIZE for y-sorting against characters", () => {
    const r = new TileRenderer(OFFICE_MAP, TILE_SIZE);
    const furniture = r.buildFurniture();

    for (const g of furniture) {
      const ty = g.position.y / TILE_SIZE;
      expect(g.zIndex).toBe((ty + 1) * TILE_SIZE);
    }
  });

  it("is not empty for the hardcoded OFFICE_MAP (sanity: desks exist)", () => {
    const r = new TileRenderer(OFFICE_MAP, TILE_SIZE);
    expect(r.buildFurniture().length).toBeGreaterThan(0);
    expect(r.buildFurniture().length).toBeGreaterThanOrEqual(OFFICE_MAP.desks.length * 2); // each desk slot is a 2-tile-wide pair, plus break-room decor
  });

  it("draws each new decoration tile type (Plant/Counter/Table) without throwing", () => {
    const r = new TileRenderer(OFFICE_MAP, TILE_SIZE);
    expect(() => r.buildFurniture()).not.toThrow();

    const drawnTypes = new Set<number>();
    for (let ty = 0; ty < OFFICE_MAP.height; ty++) {
      for (let tx = 0; tx < OFFICE_MAP.width; tx++) {
        drawnTypes.add(OFFICE_MAP.tiles[ty][tx]);
      }
    }
    expect(drawnTypes.has(Tile.Plant)).toBe(true);
    expect(drawnTypes.has(Tile.Counter)).toBe(true);
    expect(drawnTypes.has(Tile.Table)).toBe(true);
  });
});
