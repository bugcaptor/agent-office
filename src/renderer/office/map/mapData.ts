// src/renderer/office/map/mapData.ts
//
// Hardcoded office tile map + desk slot derivation.
//
// Pure data + pure derivation function — no Pixi/DOM dependency.

export const TILE_SIZE = 16;

export const enum Tile {
  Floor = 0,
  Wall = 1,
  DeskTop = 2, // 책상 상판 (캐릭터가 앉는 위쪽)
  Rug = 3, // 장식 러그
  Plant = 4, // 화분 (장식, 통행 불가)
  Counter = 5, // 탕비실 카운터 (장식, 통행 불가)
  Table = 6, // 탕비실 테이블 (장식, 통행 불가) — DeskTop과 구분되어 deriveDesks가 무시함
  BossDesk = 7, // 보스 책상 (통행 불가, 전용 렌더링) — deriveDesks가 무시함
}

export interface DeskSlot {
  index: number; // 0..N-1
  // 캐릭터가 앉는 타일(의자 위치) — 그리드 좌표
  seat: { tx: number; ty: number };
  // 바라보는 방향 (좌석이 책상 위쪽이므로 보통 'down' — 정면이 보인다)
  facing: 'up' | 'down' | 'left' | 'right';
}

export interface OfficeMap {
  width: number;
  height: number;
  tiles: readonly (readonly Tile[])[]; // [ty][tx]
  desks: readonly DeskSlot[];
}

/** 타일 좌표계 사각형(폭/높이는 타일 단위). */
export interface TileRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

// F=Floor, W=Wall, D=DeskTop, R=Rug, P=Plant, C=Counter, T=Table, B=BossDesk 로 읽기 쉽게
// 구성 후 숫자로 변환
const L = (row: string): Tile[] =>
  [...row].map(
    (ch) =>
      ({
        F: Tile.Floor,
        W: Tile.Wall,
        D: Tile.DeskTop,
        R: Tile.Rug,
        P: Tile.Plant,
        C: Tile.Counter,
        T: Tile.Table,
        B: Tile.BossDesk,
      })[ch] ?? Tile.Floor,
  );

// 위쪽: 데스크 2행 x 4쌍 = 8개. 아래쪽(ty=9..12): 탕비실(휴게 공간) —
// 러그 라운지 + 테이블 + 카운터(하단 벽 쪽) + 화분 장식.
const GRID: Tile[][] = [
  L('WWWWWWWWWWWWWWWWWWWW'), // ty0
  L('WFFFFFFFFFFFFFFFFFFW'), // ty1
  L('WFDDFFDDFFDDFFDDFFFW'), // ty2 - 데스크 상판 행 1
  L('WFFFFFFFFFFFFFFFFFFW'), // ty3 - 의자(seat) 행 1
  L('WFFFFFFFFFFFFFFFFFFW'), // ty4
  L('WFDDFFDDFFDDFFDDFFFW'), // ty5 - 데스크 상판 행 2
  L('WFFFFFFFFFFFFFFFFFFW'), // ty6 - 의자(seat) 행 2
  L('WFFFFFFFFFFFFFFFFBFW'), // ty7  - 보스 책상 상단(tx17) — 우측 벽에서 한 칸(tx18) 띄움
  L('WFFFFFFFFFFFFFFFFBFW'), // ty8  - 보스 책상 하단(tx17) + 줄서기 레인
  L('WFPFFFFFFFFFFFFFFPFW'), // ty9  - 탕비실 진입부 + 화분(모서리)
  L('WFRRRRRRRRRRRRRRRRFW'), // ty10 - 러그 라운지
  L('WFRRRRRRRTTRRRRRRRFW'), // ty11 - 러그 라운지 + 테이블(2칸)
  L('WFFCCCCCCFFFFFFFFFFW'), // ty12 - 카운터(하단 벽 쪽)
  L('WWWWWWWWWWWWWWWWWWWW'), // ty13
];

/** 휴게 공간(탕비실) 내부의 걸을 수 있는 러그 라운지 사각형(타일 좌표). */
export const BREAK_ROOM_RECT: TileRect = { x: 11, y: 10, w: 5, h: 2 };

/** GRID의 BossDesk 셀들을 감싸는 사각형 — deriveDesks처럼 지오메트리의
 * 소스는 GRID 하나. 책상을 옮기면 히트영역·표지판·줄 슬롯이 함께 따라온다. */
function deriveBossDeskRect(grid: Tile[][]): TileRect {
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (let ty = 0; ty < grid.length; ty++) {
    for (let tx = 0; tx < grid[ty].length; tx++) {
      if (grid[ty][tx] !== Tile.BossDesk) continue;
      minX = Math.min(minX, tx);
      minY = Math.min(minY, ty);
      maxX = Math.max(maxX, tx);
      maxY = Math.max(maxY, ty);
    }
  }
  return { x: minX, y: minY, w: maxX - minX + 1, h: maxY - minY + 1 };
}

/** 보스 책상 타일 영역(우측 벽을 등진 세로 1×2). 렌더링·히트영역·표지판 위치의 단일 출처. */
export const BOSS_DESK_RECT: TileRect = deriveBossDeskRect(GRID);

const QUEUE_MAX_SLOTS = 8;

/** 줄서기 슬롯(슬롯 0 = 맨 앞) — 책상 하단 행을 따라 서쪽으로 늘어선다. */
export const QUEUE_SLOTS: readonly { tx: number; ty: number }[] = Array.from(
  { length: QUEUE_MAX_SLOTS },
  (_, i) => ({ tx: BOSS_DESK_RECT.x - 1 - i, ty: BOSS_DESK_RECT.y + BOSS_DESK_RECT.h - 1 }),
);

// 데스크 상판(ty=2,5)의 각 DeskTop 쌍마다 그 *위* 타일을 seat으로 생성 —
// 캐릭터가 책상 뒤(북쪽)에 앉아 정면이 보이고, 랩탑은 뒷면이 보인다.
function deriveDesks(grid: Tile[][]): DeskSlot[] {
  const desks: DeskSlot[] = [];
  let idx = 0;
  for (let ty = 0; ty < grid.length; ty++) {
    for (let tx = 0; tx < grid[ty].length; tx++) {
      // 데스크 쌍의 왼쪽 타일에서만 슬롯 생성 (오른쪽은 짝)
      if (grid[ty][tx] === Tile.DeskTop && grid[ty][tx - 1] !== Tile.DeskTop) {
        desks.push({ index: idx++, seat: { tx, ty: ty - 1 }, facing: 'down' });
      }
    }
  }
  return desks;
}

export const OFFICE_MAP: OfficeMap = {
  width: GRID[0].length,
  height: GRID.length,
  tiles: GRID,
  desks: deriveDesks(GRID),
};
