// src/renderer/office/map/TileRenderer.ts
//
// Procedural tile rendering: colored rectangles + 1px pixel-art detail,
// no embedded tilesheet assets (no art pipeline; a code-drawn palette swap
// is easy and 16x16 tiles are simple enough to draw in code).
//
// `build()` bakes the static floor/wall layer into a single cached texture
// (nearest-neighbor, per the project's pixel-art-sharpness requirement — the
// a naive `cacheAsTexture(true)` defaults to 'linear' scaling,
// which would blur this baked texture, so we pass `{ scaleMode: 'nearest' }`
// explicitly). `buildFurniture()` returns individual Graphics for every
// y-sort target (desks, plants, break-room counter/table) for the
// `sortableLayer`, each pre-tagged with `zIndex`.
//
// 한 칸을 *무엇으로* 그릴지는 풍경(scene)이 정한다(scenes/sceneTypes.ts의
// `TileDrawFn`; 오피스 드로잉은 scenes/officeScene.ts). 이 클래스는 베이크/
// 가구 분리/zIndex 같은 씬 공용 기계만 소유하므로 씬이 늘어나도 그대로다.

import { Container, Graphics } from "pixi.js";
import { Tile, TILE_SIZE } from "./mapData";
import type { OfficeMap } from "./mapData";
import { THEMES } from "../../theme/themes";
import { officeTileDraw } from "../scenes/officeScene";
import type { TileDrawFn } from "../scenes/sceneTypes";

export class TileRenderer {
  // 드로잉 함수는 씬 레지스트리(scenes/*)에서 주입된다. 기본값은 오피스 씬 ×
  // 테마 도입 이전의 원본 색(midnight) — 팔레트/드로잉 없이 쓰던 기존
  // 호출부·테스트의 외형을 보존한다. OfficeScene은 항상 현재 씬×테마로
  // 확정된 함수를 명시적으로 넘긴다.
  constructor(
    private map: OfficeMap,
    private tile = TILE_SIZE,
    private draw: TileDrawFn = officeTileDraw(THEMES.midnight.pixi),
  ) {}

  /** Tile types drawn in the y-sorted furniture layer instead of the baked floor layer. */
  private static readonly FURNITURE_TILES: ReadonlySet<Tile> = new Set([
    Tile.DeskTop,
    Tile.Plant,
    Tile.Counter,
    Tile.Table,
    Tile.BossDesk,
  ]);

  /** Static floor+wall layer. Checkerboard + 1px dot detail, baked into one texture. */
  build(): Container {
    const root = new Container();
    for (let ty = 0; ty < this.map.height; ty++) {
      for (let tx = 0; tx < this.map.width; tx++) {
        const t = this.map.tiles[ty][tx];
        if (TileRenderer.FURNITURE_TILES.has(t)) continue; // drawn in the furniture (y-sort) layer
        const g = this.drawTile(t, tx, ty);
        g.position.set(tx * this.tile, ty * this.tile);
        root.addChild(g);
      }
    }
    root.cacheAsTexture({ scaleMode: "nearest" }); // static -> bake to one texture (Pixi v8 API)
    return root;
  }

  /** Desks/plants/counters/tables are y-sort targets, so they stay individual display objects. zIndex = bottom y. */
  buildFurniture(): Container[] {
    const out: Container[] = [];
    for (let ty = 0; ty < this.map.height; ty++) {
      for (let tx = 0; tx < this.map.width; tx++) {
        const t = this.map.tiles[ty][tx];
        if (!TileRenderer.FURNITURE_TILES.has(t)) continue;
        const g = this.drawTile(t, tx, ty);
        g.position.set(tx * this.tile, ty * this.tile);
        g.zIndex = (ty + 1) * this.tile; // sort by furniture's bottom edge
        out.push(g);
      }
    }
    return out;
  }

  private drawTile(t: Tile, tx: number, ty: number): Graphics {
    const g = new Graphics();
    this.draw(g, { t, tx, ty, s: this.tile, map: this.map });
    return g;
  }
}
