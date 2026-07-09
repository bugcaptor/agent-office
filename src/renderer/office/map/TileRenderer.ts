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

import { Container, Graphics } from "pixi.js";
import { Tile, TILE_SIZE } from "./mapData";
import type { OfficeMap } from "./mapData";
import { THEMES } from "../../theme/themes";
import type { OfficeTilePalette } from "../../theme/themes";

export class TileRenderer {
  // 팔레트는 테마 시스템(theme/themes.ts)에서 주입된다. 기본값은 테마 도입
  // 이전의 원본 색(midnight) — 팔레트 없이 쓰던 기존 호출부/테스트의 외형을
  // 보존한다. OfficeScene은 항상 현재 테마의 팔레트를 명시적으로 넘긴다.
  constructor(
    private map: OfficeMap,
    private tile = TILE_SIZE,
    private pal: OfficeTilePalette = THEMES.midnight.pixi,
  ) {}

  /** Tile types drawn in the y-sorted furniture layer instead of the baked floor layer. */
  private static readonly FURNITURE_TILES: ReadonlySet<Tile> = new Set([
    Tile.DeskTop,
    Tile.Plant,
    Tile.Counter,
    Tile.Table,
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
    const s = this.tile;
    switch (t) {
      case Tile.Floor: {
        const checker = (tx + ty) % 2 === 0 ? this.pal.floorA : this.pal.floorB;
        g.rect(0, 0, s, s).fill(checker);
        // 1px pixel detail: corner dots
        g.rect(1, 1, 1, 1).fill(this.pal.floorDot);
        g.rect(s - 2, s - 2, 1, 1).fill(this.pal.floorDot);
        break;
      }
      case Tile.Wall:
        g.rect(0, 0, s, s).fill(this.pal.wall);
        g.rect(0, 0, s, 3).fill(this.pal.wallTop); // 3px top highlight
        break;
      case Tile.DeskTop:
        g.rect(0, 0, s, s).fill(this.pal.desk);
        g.rect(0, 0, s, 4).fill(this.pal.deskTop); // bright top face
        g.rect(0, s - 2, s, 2).fill(this.pal.deskEdge); // bottom shadow
        g.rect(2, 6, s - 4, 1).fill(this.pal.deskEdge); // 1px wood grain
        // 랩탑(뒷모습): 좌석과 정렬된 왼쪽 타일에만. 캐릭터가 책상 위쪽에
        // 앉으므로 화면은 북쪽을 향하고, 뷰어에게는 뚜껑 등판이 보인다.
        if (this.map.tiles[ty][tx - 1] !== Tile.DeskTop) {
          g.rect(s * 0.2, s * 0.25, s * 0.6, 2).fill(this.pal.laptopBody); // 본체(키보드) 슬리버 — 뚜껑 뒤로 살짝
          g.rect(s * 0.25, s * 0.3, s * 0.5, s * 0.45).fill(this.pal.laptopLid); // 뚜껑 등판
          g.rect(s * 0.25, s * 0.3 + s * 0.45 - 1, s * 0.5, 1).fill(this.pal.laptopBody); // 하단 힌지 라인
          g.rect(s * 0.45, s * 0.42, 2, 2).fill(this.pal.laptopBody); // 로고 도트
        }
        break;
      case Tile.Rug:
        g.rect(0, 0, s, s).fill(this.pal.rug);
        g.rect(0, 0, s, 1).fill(this.pal.rugEdge);
        g.rect(0, 0, 1, s).fill(this.pal.rugEdge);
        break;
      case Tile.Plant: {
        // Pot (bottom half) + a few foliage clusters (top), pixel-art style.
        const potH = Math.round(s * 0.35);
        g.rect(s * 0.25, s - potH, s * 0.5, potH).fill(this.pal.plantPot);
        g.rect(s * 0.5 - 1, s - potH - 1, 2, 1).fill(this.pal.plantPot); // pot rim
        g.rect(s * 0.3, s * 0.15, s * 0.4, s * 0.35).fill(this.pal.plant); // main foliage mass
        g.rect(s * 0.12, s * 0.35, s * 0.22, s * 0.22).fill(this.pal.plant); // left cluster
        g.rect(s * 0.66, s * 0.35, s * 0.22, s * 0.22).fill(this.pal.plant); // right cluster
        break;
      }
      case Tile.Counter:
        g.rect(0, 0, s, s).fill(this.pal.counter);
        g.rect(0, 0, s, 4).fill(this.pal.counterTop); // countertop face
        g.rect(0, s - 2, s, 2).fill(this.pal.counter); // base shadow
        // Alternate top decoration by tile position: coffee machine vs cup/sink block.
        if (tx % 2 === 0) {
          g.rect(s * 0.3, s * 0.55, s * 0.4, s * 0.3).fill(this.pal.counterTop); // coffee machine body
          g.rect(s * 0.4, s * 0.48, s * 0.2, s * 0.1).fill(this.pal.counterTop); // spout
        } else {
          g.rect(s * 0.25, s * 0.6, s * 0.2, s * 0.2).fill(this.pal.counterTop); // cup
          g.rect(s * 0.55, s * 0.6, s * 0.2, s * 0.2).fill(this.pal.counterTop); // cup
        }
        break;
      case Tile.Table:
        g.rect(0, 0, s, s).fill(this.pal.table);
        g.rect(0, 0, s, 4).fill(this.pal.tableTop); // bright top face
        g.rect(0, s - 2, s, 2).fill(this.pal.table); // bottom shadow (legs)
        g.rect(2, 6, s - 4, 1).fill(this.pal.tableTop); // 1px wood grain
        break;
    }
    return g;
  }
}
