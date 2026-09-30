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
import { Container, type Graphics } from "pixi.js";
import { TileRenderer } from "../TileRenderer";
import { BOSS_DESK_RECT, OFFICE_MAP, Tile, TILE_SIZE } from "../mapData";

/** Tile types drawn in the y-sorted furniture layer (mirrors TileRenderer's own set). */
const FURNITURE_TILES = new Set([Tile.DeskTop, Tile.Plant, Tile.Counter, Tile.Table, Tile.BossDesk]);

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

  it("draws a laptop (back of the lid toward the viewer) on the left tile of each desk pair", () => {
    // 좌석이 책상 위쪽이므로 랩탑 화면은 북쪽(캐릭터)을 향하고, 뷰어에게는
    // 뚜껑 등판이 보인다. 랩탑은 좌석과 정렬된 왼쪽 타일에만 그린다 —
    // 왼쪽 타일 Graphics는 오른쪽 짝보다 드로우 명령이 많아야 한다.
    const r = new TileRenderer(OFFICE_MAP, TILE_SIZE);
    const byTile = new Map<string, Graphics>();
    for (const g of r.buildFurniture()) {
      byTile.set(`${g.position.x / TILE_SIZE},${g.position.y / TILE_SIZE}`, g as Graphics);
    }
    for (const d of OFFICE_MAP.desks) {
      const deskTy = d.seat.ty + 1; // 좌석 바로 아래(남쪽)가 책상 상판
      const left = byTile.get(`${d.seat.tx},${deskTy}`)!;
      const right = byTile.get(`${d.seat.tx + 1},${deskTy}`)!;
      expect(left).toBeDefined();
      expect(right).toBeDefined();
      expect(left.context.instructions.length).toBeGreaterThan(right.context.instructions.length);
    }
  });

  it("renders boss desk tiles in the furniture (y-sort) layer", () => {
    const out = new TileRenderer(OFFICE_MAP, TILE_SIZE).buildFurniture();
    const atBossDesk = out.filter(
      (g) => g.position.y >= BOSS_DESK_RECT.y * TILE_SIZE &&
        g.position.y < (BOSS_DESK_RECT.y + BOSS_DESK_RECT.h) * TILE_SIZE &&
        g.position.x >= BOSS_DESK_RECT.x * TILE_SIZE &&
        g.position.x < (BOSS_DESK_RECT.x + BOSS_DESK_RECT.w) * TILE_SIZE,
    );
    expect(atBossDesk).toHaveLength(BOSS_DESK_RECT.w * BOSS_DESK_RECT.h);
    for (const g of atBossDesk) {
      expect(g.zIndex).toBe((g.position.y / TILE_SIZE + 1) * TILE_SIZE);
    }
  });
});
