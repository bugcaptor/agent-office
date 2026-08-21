// src/renderer/office/scenes/sceneTypes.ts
//
// 풍경(scene) 축의 타입 계약 — 테마(색 축)와 직교한다.
//
// 계약의 핵심은 "의미 타일(Tile) 하나, 그림은 씬마다": 모든 씬이 같은
// `Tile` enum과 같은 맵 크기를 쓰므로 `deriveDesks`/좌석 배정/행동 FSM/
// 줄서기 로직은 씬을 전혀 모른다. 씬이 정하는 것은 두 가지뿐이다.
//   1) 자기 GRID(= `map`) — 어느 칸이 책상/라운지/보스 자리인가.
//   2) `resolve(theme)` — 그 칸들을 무슨 색으로 어떻게 그릴 것인가.
//
// TileRenderer의 베이크/가구 분리/zIndex 기계는 공용이고, 여기서 주입되는
// `drawTile`이 16px 한 칸의 픽셀아트만 책임진다. 팔레트는 `resolve`가
// 반환하는 클로저 안에 잡혀 있어(제네릭 대신 클로저) 레지스트리는
// 씬마다 다른 팔레트 타입을 몰라도 된다.
import type { Graphics } from "pixi.js";

import type { OfficeMap, Tile } from "../map/mapData";
import type { ThemeDef } from "../../theme/themes";

export type SceneId =
  | "office"
  | "beach"
  | "valley"
  | "spaceship"
  | "castle"
  | "steppe"
  | "cruise"
  | "factory"
  | "volcano"
  | "zombie";

/** `drawTile`에 넘어가는 한 칸의 문맥. `map`은 이웃 칸을 보고 디테일을
 * 바꾸는 용도(데스크 쌍의 왼쪽에만 랩탑, 보스 책상 상/하단 구분 등). */
export interface TileDrawContext {
  t: Tile;
  tx: number;
  ty: number;
  /** 타일 한 변(px) = TILE_SIZE. */
  s: number;
  map: OfficeMap;
}

/** 빈 Graphics에 타일 한 칸을 그린다(rect+fill 픽셀아트, 애니메이션 없음). */
export type TileDrawFn = (g: Graphics, ctx: TileDrawContext) => void;

/** 현재 테마로 확정된 씬의 렌더 바인딩. */
export interface SceneRender {
  /** 맵 밖 레터박스 배경색(0xRRGGBB). */
  background: number;
  /** 이 씬·이 테마의 팔레트가 묶인 타일 드로잉 함수. */
  drawTile: TileDrawFn;
}

export interface SceneDef {
  id: SceneId;
  /** 픽커 드롭다운에 그대로 노출되는 한국어 라벨. */
  label: string;
  /** 씬의 타일 맵. 좌석/보스 책상/줄 슬롯/라운지가 전부 여기서 유도된다. */
  map: OfficeMap;
  /**
   * 테마 → 렌더 바인딩. office 씬은 `theme.pixi`를 그대로 쓰고, 나머지 씬은
   * 자기 고유 팔레트에 테마별 변환(sceneColor.ts)을 걸어 테마와 어울리게 한다.
   */
  resolve(theme: ThemeDef): SceneRender;
}
