// src/renderer/office/world/pathing.ts
//
// Grid <-> pixel conversion and wander-target selection. Pure — no
// Pixi/DOM dependency. `pickWanderTarget` takes an injected `rand`
// function; determinism is not required at runtime, but tests inject a
// fixed sequence for reproducibility.

import { BREAK_ROOM_RECT, OfficeMap, Tile, TILE_SIZE } from "../map/mapData";

export interface GridPos {
  tx: number;
  ty: number;
}

export const tileCenterPx = (p: GridPos) => ({
  x: p.tx * TILE_SIZE + TILE_SIZE / 2,
  y: p.ty * TILE_SIZE + TILE_SIZE / 2,
});

/** 타일 점유 집합(Set<string>)에서 쓰는 키. */
export const tileKey = (tx: number, ty: number): string => `${tx},${ty}`;

// Floor와 Rug(러그 = 바닥 마감재)는 걸을 수 있고, 그 외(벽/데스크/화분/
// 카운터/테이블)는 통행 불가.
export const isWalkable = (m: OfficeMap, tx: number, ty: number): boolean =>
  ty >= 0 &&
  ty < m.height &&
  tx >= 0 &&
  tx < m.width &&
  (m.tiles[ty][tx] === Tile.Floor || m.tiles[ty][tx] === Tile.Rug);

/** seat 주변 걷기 가능한 임의 타일(배회 목적지) 선택. 결정성 불필요(런타임). */
export function pickWanderTarget(m: OfficeMap, near: GridPos, rand: () => number, radius = 3): GridPos {
  for (let i = 0; i < 12; i++) {
    const tx = near.tx + Math.round((rand() * 2 - 1) * radius);
    const ty = near.ty + Math.round((rand() * 2 - 1) * radius);
    if (isWalkable(m, tx, ty)) return { tx, ty };
  }
  return near; // 실패 시 제자리
}

/**
 * 탕비실(휴게 공간) 사각형(BREAK_ROOM_RECT) 내부에서 걸을 수 있는 임의
 * 타일을 골라 그 픽셀 중심을 반환한다. 사각형 내부 전체가 걸을 수 있는
 * 타일이라는 계약이 있으므로 보통 첫 시도에 성공하지만, 계약이 깨진
 * 경우를 대비해 pickWanderTarget과 같은 재시도(rejection sampling) 방식을
 * 쓰고 실패 시 null을 반환한다.
 *
 * `occupied`(tileKey 집합)에 든 타일은 다른 캐릭터가 예약한 자리이므로
 * 후보에서 제외한다 — 쉬는 캐릭터끼리 같은 타일에 겹쳐 서지 않게 하는
 * 유일한 방어선. 전부 점유라 실패하면 null(호출자가 다음 틱에 재시도).
 */
export function pickBreakTarget(
  m: OfficeMap,
  rand: () => number,
  occupied?: ReadonlySet<string>,
): { x: number; y: number } | null {
  const rect = BREAK_ROOM_RECT;
  for (let i = 0; i < 12; i++) {
    const tx = rect.x + Math.floor(rand() * rect.w);
    const ty = rect.y + Math.floor(rand() * rect.h);
    if (isWalkable(m, tx, ty) && !occupied?.has(tileKey(tx, ty))) return tileCenterPx({ tx, ty });
  }
  return null;
}
