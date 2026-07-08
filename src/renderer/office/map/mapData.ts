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
}

export interface DeskSlot {
  index: number; // 0..N-1
  // 캐릭터가 앉는 타일(의자 위치) — 그리드 좌표
  seat: { tx: number; ty: number };
  // 바라보는 방향 (데스크가 위에 있으므로 보통 'up')
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

// F=Floor, W=Wall, D=DeskTop, R=Rug, P=Plant, C=Counter, T=Table 로 읽기 쉽게
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
  L('WFFFFFFFFFFFFFFFFFFW'), // ty7
  L('WFFFFFFFFFFFFFFFFFFW'), // ty8
  L('WFPFFFFFFFFFFFFFFPFW'), // ty9  - 탕비실 진입부 + 화분(모서리)
  L('WFRRRRRRRRRRRRRRRRFW'), // ty10 - 러그 라운지
  L('WFRRRRRRRTTRRRRRRRFW'), // ty11 - 러그 라운지 + 테이블(2칸)
  L('WFFCCCCCCFFFFFFFFFFW'), // ty12 - 카운터(하단 벽 쪽)
  L('WWWWWWWWWWWWWWWWWWWW'), // ty13
];

/** 휴게 공간(탕비실) 내부의 걸을 수 있는 러그 라운지 사각형(타일 좌표). */
export const BREAK_ROOM_RECT: TileRect = { x: 11, y: 10, w: 7, h: 2 };

// 데스크 상판(ty=2,5)의 각 DeskTop 쌍마다 그 아래 타일을 seat으로 생성
function deriveDesks(grid: Tile[][]): DeskSlot[] {
  const desks: DeskSlot[] = [];
  let idx = 0;
  for (let ty = 0; ty < grid.length; ty++) {
    for (let tx = 0; tx < grid[ty].length; tx++) {
      // 데스크 쌍의 왼쪽 타일에서만 슬롯 생성 (오른쪽은 짝)
      if (grid[ty][tx] === Tile.DeskTop && grid[ty][tx - 1] !== Tile.DeskTop) {
        desks.push({ index: idx++, seat: { tx, ty: ty + 1 }, facing: 'up' });
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
