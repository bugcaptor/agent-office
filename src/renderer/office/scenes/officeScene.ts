// src/renderer/office/scenes/officeScene.ts
//
// 기본 풍경: 사무실. 씬 축이 생기기 전 `TileRenderer.drawTile`에 있던 드로잉을
// 그대로 옮겨 온 것이라 **비주얼은 도입 이전과 100% 동일**해야 한다 —
// 좌표·색 키·드로우 순서를 손대지 말 것(TileRenderer의 랩탑 테스트가
// 왼쪽/오른쪽 상판의 드로우 명령 수 차이를 본다).
//
// 맵(GRID)은 `map/mapData.ts`에 그대로 남겨 뒀다: `OFFICE_MAP`을 비롯한
// 파생 상수들의 소비처가 많고, 이리로 옮기면 mapData ↔ scenes 순환 참조가
// 생긴다. 씬은 그 맵을 가리키기만 한다.
import type { OfficeTilePalette } from "../../theme/themes";
import { OFFICE_MAP, Tile } from "../map/mapData";
import type { SceneDef, TileDrawFn } from "./sceneTypes";

/** 오피스 타일 드로잉을 팔레트에 바인딩한다. */
export function officeTileDraw(pal: OfficeTilePalette): TileDrawFn {
  return (g, { t, tx, ty, s, map }) => {
    switch (t) {
      case Tile.Floor: {
        const checker = (tx + ty) % 2 === 0 ? pal.floorA : pal.floorB;
        g.rect(0, 0, s, s).fill(checker);
        // 1px pixel detail: corner dots
        g.rect(1, 1, 1, 1).fill(pal.floorDot);
        g.rect(s - 2, s - 2, 1, 1).fill(pal.floorDot);
        break;
      }
      case Tile.Wall:
        g.rect(0, 0, s, s).fill(pal.wall);
        g.rect(0, 0, s, 3).fill(pal.wallTop); // 3px top highlight
        break;
      case Tile.DeskTop:
        g.rect(0, 0, s, s).fill(pal.desk);
        g.rect(0, 0, s, 4).fill(pal.deskTop); // bright top face
        g.rect(0, s - 2, s, 2).fill(pal.deskEdge); // bottom shadow
        g.rect(2, 6, s - 4, 1).fill(pal.deskEdge); // 1px wood grain
        // 랩탑(뒷모습): 좌석과 정렬된 왼쪽 타일에만. 캐릭터가 책상 위쪽에
        // 앉으므로 화면은 북쪽을 향하고, 뷰어에게는 뚜껑 등판이 보인다.
        if (map.tiles[ty][tx - 1] !== Tile.DeskTop) {
          g.rect(s * 0.2, s * 0.25, s * 0.6, 2).fill(pal.laptopBody); // 본체(키보드) 슬리버 — 뚜껑 뒤로 살짝
          g.rect(s * 0.25, s * 0.3, s * 0.5, s * 0.45).fill(pal.laptopLid); // 뚜껑 등판
          g.rect(s * 0.25, s * 0.3 + s * 0.45 - 1, s * 0.5, 1).fill(pal.laptopBody); // 하단 힌지 라인
          g.rect(s * 0.45, s * 0.42, 2, 2).fill(pal.laptopBody); // 로고 도트
        }
        break;
      case Tile.Rug:
        g.rect(0, 0, s, s).fill(pal.rug);
        g.rect(0, 0, s, 1).fill(pal.rugEdge);
        g.rect(0, 0, 1, s).fill(pal.rugEdge);
        break;
      case Tile.Plant: {
        // Pot (bottom half) + a few foliage clusters (top), pixel-art style.
        const potH = Math.round(s * 0.35);
        g.rect(s * 0.25, s - potH, s * 0.5, potH).fill(pal.plantPot);
        g.rect(s * 0.5 - 1, s - potH - 1, 2, 1).fill(pal.plantPot); // pot rim
        g.rect(s * 0.3, s * 0.15, s * 0.4, s * 0.35).fill(pal.plant); // main foliage mass
        g.rect(s * 0.12, s * 0.35, s * 0.22, s * 0.22).fill(pal.plant); // left cluster
        g.rect(s * 0.66, s * 0.35, s * 0.22, s * 0.22).fill(pal.plant); // right cluster
        break;
      }
      case Tile.Counter:
        g.rect(0, 0, s, s).fill(pal.counter);
        g.rect(0, 0, s, 4).fill(pal.counterTop); // countertop face
        g.rect(0, s - 2, s, 2).fill(pal.counter); // base shadow
        // Alternate top decoration by tile position: coffee machine vs cup/sink block.
        if (tx % 2 === 0) {
          g.rect(s * 0.3, s * 0.55, s * 0.4, s * 0.3).fill(pal.counterTop); // coffee machine body
          g.rect(s * 0.4, s * 0.48, s * 0.2, s * 0.1).fill(pal.counterTop); // spout
        } else {
          g.rect(s * 0.25, s * 0.6, s * 0.2, s * 0.2).fill(pal.counterTop); // cup
          g.rect(s * 0.55, s * 0.6, s * 0.2, s * 0.2).fill(pal.counterTop); // cup
        }
        break;
      case Tile.Table:
        g.rect(0, 0, s, s).fill(pal.table);
        g.rect(0, 0, s, 4).fill(pal.tableTop); // bright top face
        g.rect(0, s - 2, s, 2).fill(pal.table); // bottom shadow (legs)
        g.rect(2, 6, s - 4, 1).fill(pal.tableTop); // 1px wood grain
        break;
      case Tile.BossDesk: {
        // 세로 책상(우측 벽 등짐) = 일반 책상의 90° 회전 — 가로 배치로
        // 되돌리면 이 케이스도 다시 눕혀야 한다.
        const north = map.tiles[ty - 1]?.[tx] === Tile.BossDesk;
        g.rect(0, 0, s, s).fill(pal.desk);
        g.rect(s - 4, 0, 4, s).fill(pal.deskTop); // 밝은 상판 얼굴(좌석 쪽 세로 밴드)
        g.rect(0, 0, 2, s).fill(pal.deskEdge); // 전면 그림자(줄 쪽 세로 밴드)
        g.rect(s - 7, 2, 1, s - 4).fill(pal.deskEdge); // 1px wood grain
        if (!north) {
          g.rect(s * 0.45, s * 0.25, 3, s * 0.5).fill(pal.counterTop); // 명패
        }
        break;
      }
    }
  };
}

export const OFFICE_SCENE: SceneDef = {
  id: "office",
  label: "사무실",
  map: OFFICE_MAP,
  // 테마 팔레트 직결 — 씬 색 변환(sceneColor)을 거치지 않는 유일한 씬이다.
  resolve: (theme) => ({
    background: theme.pixi.background,
    drawTile: officeTileDraw(theme.pixi),
  }),
};
