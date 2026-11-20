// src/renderer/office/scenes/meetingTiles.ts
//
// 토의 회의실(5×3, `meetingRoomAt`) 그림 — 모든 풍경이 같은 구도를 쓴다.
//
// 회의실 바닥(MeetingFloor)은 한 장짜리 깔개로 그리고 의자는 그 위에 굽는다. 회의 테이블은 GRID에서
// 평범한 Table 타일이라, 각 풍경의 Table 그림(화로·비치 테이블·홀로테이블 같은 2칸짜리 소품)이 그대로
// 쓰이면 3칸 회의 테이블로 읽히지 않는다. 그래서 회의실 사각형 안의 Table은 여기서 긴 테이블로 그린다.
// 풍경이 정하는 것은 색(`MeetingPalette`)뿐이다.
import type { Graphics } from "pixi.js";

import { Tile } from "../map/mapData";
import type { OfficeMap } from "../map/mapData";
import type { TileDrawContext } from "./sceneTypes";

/** 회의실 바닥·의자 색. 오피스는 테마 팔레트에서, 다른 풍경은 저작 팔레트의 `meet*` 키에서 온다. */
export interface MeetingFloorColors {
  floor: number;
  edge: number;
  grain: number;
  chair: number;
  /** 의자 등받이 윗변 하이라이트. 없으면 긋지 않는다. */
  chairHi?: number;
}

/** 풍경 저작 팔레트가 갖추는 회의실 색 키. 테이블까지 포함한다. */
export interface MeetingPalette {
  meetFloor: number;
  meetEdge: number;
  meetGrain: number;
  meetChair: number;
  meetChairHi: number;
  meetTableTop: number;
  meetTable: number;
  meetTableShade: number;
  meetPaper: number;
  meetCup: number;
}

function inMeetingRoom(map: OfficeMap, tx: number, ty: number): boolean {
  const r = map.meetingRoom?.rect;
  return !!r && tx >= r.x && tx < r.x + r.w && ty >= r.y && ty < r.y + r.h;
}

/** 깔개 바탕: 회의실 바깥과 맞닿은 변에만 테두리를 긋고, 안쪽에는 가로 결을 넣는다. */
function drawMeetingBase(g: Graphics, { tx, ty, s, map }: TileDrawContext, c: MeetingFloorColors): void {
  g.rect(0, 0, s, s).fill(c.floor);
  const room = (x: number, y: number) => {
    const n = map.tiles[y]?.[x];
    return n === Tile.MeetingFloor || n === Tile.Table;
  };
  if (!room(tx, ty - 1)) g.rect(0, 0, s, 1).fill(c.edge);
  if (!room(tx, ty + 1)) g.rect(0, s - 1, s, 1).fill(c.edge);
  if (!room(tx - 1, ty)) g.rect(0, 0, 1, s).fill(c.edge);
  if (!room(tx + 1, ty)) g.rect(s - 1, 0, 1, s).fill(c.edge);
  g.rect(3, 5, s - 6, 1).fill(c.grain);
  g.rect(3, 11, s - 6, 1).fill(c.grain);
}

/** 회의실 바닥 한 칸 + 그 칸이 의자 자리면 빈 의자. 앉은 캐릭터가 대부분을 덮고 등받이만 머리 뒤로 비친다. */
export function drawMeetingFloor(g: Graphics, ctx: TileDrawContext, c: MeetingFloorColors): void {
  const { tx, ty, s, map } = ctx;
  drawMeetingBase(g, ctx, c);
  const seat = map.meetingRoom?.seats.find((x) => x.tx === tx && x.ty === ty);
  if (seat?.facing === "down") {
    g.rect(3, 1, s - 6, 4).fill(c.chair); // 높은 등받이(테이블 반대편 = 북쪽)
    g.rect(4, 8, s - 8, 3).fill(c.chair); // 좌석
    if (c.chairHi !== undefined) g.rect(3, 1, s - 6, 1).fill(c.chairHi);
  } else if (seat) {
    const back = seat.facing === "right" ? 1 : s - 4; // 등받이는 테이블 반대쪽 변
    g.rect(back, 3, 3, s - 5).fill(c.chair);
    g.rect(seat.facing === "right" ? 4 : 3, 9, s - 7, 3).fill(c.chair);
    if (c.chairHi !== undefined) g.rect(back, 3, 3, 1).fill(c.chairHi);
  }
}

/** 이어 붙는 긴 회의 테이블 한 칸. 다리는 양 끝 칸에만, 위에는 서류와 잔을 번갈아 둔다. */
function drawMeetingTable(g: Graphics, ctx: TileDrawContext, pal: MeetingPalette): void {
  const { tx, ty, s, map } = ctx;
  drawMeetingBase(g, ctx, meetingFloorColors(pal));
  const west = map.tiles[ty][tx - 1] !== Tile.Table;
  const east = map.tiles[ty][tx + 1] !== Tile.Table;
  const x0 = west ? 1 : 0;
  const w = (east ? s - 1 : s) - x0;
  g.rect(x0, 3, w, 7).fill(pal.meetTableTop); // 상판
  g.rect(x0, 10, w, 2).fill(pal.meetTable); // 앞판
  g.rect(x0, 12, w, 1).fill(pal.meetTableShade);
  if (west) g.rect(2, 12, 2, 3).fill(pal.meetTable);
  if (east) g.rect(s - 4, 12, 2, 3).fill(pal.meetTable);
  if (tx % 2 === 0) {
    g.rect(4, 4, 7, 5).fill(pal.meetPaper); // 펼친 서류
    g.rect(5, 6, 5, 1).fill(pal.meetTableShade);
  } else {
    g.rect(3, 5, 4, 3).fill(pal.meetPaper);
    g.rect(10, 4, 3, 4).fill(pal.meetCup); // 잔
  }
}

export function meetingFloorColors(pal: MeetingPalette): MeetingFloorColors {
  return { floor: pal.meetFloor, edge: pal.meetEdge, grain: pal.meetGrain, chair: pal.meetChair, chairHi: pal.meetChairHi };
}

/**
 * 회의실 칸이면 그리고 true를 낸다(회의실 바닥, 회의실 안의 Table). 풍경의 타일 드로잉이 맨 앞에서
 * 부르고, false면 원래 그림으로 넘어간다.
 */
export function drawMeetingTile(g: Graphics, ctx: TileDrawContext, pal: MeetingPalette): boolean {
  if (ctx.t === Tile.MeetingFloor) {
    drawMeetingFloor(g, ctx, meetingFloorColors(pal));
    return true;
  }
  if (ctx.t === Tile.Table && inMeetingRoom(ctx.map, ctx.tx, ctx.ty)) {
    drawMeetingTable(g, ctx, pal);
    return true;
  }
  return false;
}
