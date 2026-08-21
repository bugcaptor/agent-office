// src/renderer/office/scenes/valleyScene.ts
//
// 산 계곡 풍경. 해변 씬과 같은 규칙(의미 타일 공유, 어휘만 교체):
//   Floor=풀밭·자갈, Wall=산능선(ty0)/침엽수림(ty1)/바위(측면)/계곡 물줄기(하단),
//   Rug=캠핑 돗자리, DeskTop=캠핑 테이블, Plant=침엽수·바위,
//   Counter=모닥불·주전자, Table=평상, BossDesk=통나무 오두막.
//
// 레이아웃은 해변과 일부러 다르게 잡았다 — 오두막(보스 자리)이 좌측에 서고
// 줄서기 레인이 동쪽으로 뻗으며, 라운지는 우측에 있다(줄 슬롯 방향은
// `buildSceneMap`이 보스 위치에서 유도한다).
import type { TileRect } from "../map/mapData";
import { L, Tile, buildSceneMap } from "../map/mapData";
import { adaptColor, adaptPalette, sceneColorMode } from "./sceneColor";
import type { SceneDef, TileDrawFn } from "./sceneTypes";

const GRID: Tile[][] = [
  L('WWWWWWWWWWWWWWWWWWWW'), // ty0  - 설산 능선
  L('WWWWWWWWWWWWWWWWWWWW'), // ty1  - 침엽수림
  L('WFFFFFFFFFFFFFFFFFFW'), // ty2  - 좌석 행 1
  L('WFDDFFDDFFDDFFDDFFPW'), // ty3  - 캠핑 테이블 4쌍 + 침엽수(tx18)
  L('WFFFFFFFFFFFFFFFFFFW'), // ty4
  L('WFFFFFFFFFFFFFFFFFFW'), // ty5  - 좌석 행 2
  L('WPDDFFDDFFDDFFDDFFFW'), // ty6  - 바위(tx1) + 캠핑 테이블 4쌍
  L('WFFFFFFFFFFFFFFFFFFW'), // ty7
  L('WFBFFFFFFFFFFFFFFFFW'), // ty8  - 통나무 오두막 상단(tx2)
  L('WFBFFFFFFFFFFFFFFFFW'), // ty9  - 오두막 하단 + 줄서기 레인(동쪽)
  L('WFFFFFFRRRRRRRRRRRFW'), // ty10 - 캠핑 돗자리
  L('WFFFFFFRRRRTTRRRRRFW'), // ty11 - 돗자리 + 평상(2칸)
  L('WFFCCCCCFFFFFFFFFFFW'), // ty12 - 모닥불·주전자
  L('WWWWWWWWWWWWWWWWWWWW'), // ty13 - 계곡 물줄기
];

/** 돗자리를 편 쉼터 — 오피스 러그 라운지와 같은 역할. */
const BREAK_ROOM: TileRect = { x: 13, y: 10, w: 5, h: 2 };

export const VALLEY_MAP = buildSceneMap(GRID, BREAK_ROOM);

const VALLEY_PALETTE = {
  grassA: 0x6faf57,
  grassB: 0x63a04d,
  grassDot: 0x86c46b,
  pebble: 0xa8a49a,
  pebbleHi: 0xc6c2b8,
  flower: 0xf2d05a,
  ridgeSky: 0x455467,
  ridgeRock: 0x5f7085,
  ridgeSnow: 0xe6eef4,
  forestDark: 0x1d4530,
  forestMid: 0x2c6a45,
  forestHi: 0x3f8a58,
  rockWall: 0x7b7367,
  rockTop: 0x9b9284,
  moss: 0x5f8f4a,
  streamDeep: 0x2f7fa8,
  streamMid: 0x49a0c6,
  streamFoam: 0xd8f0f7,
  mat: 0x3f6ea8,
  matPlaid: 0xe4e0d0,
  tableWood: 0xc0894f,
  tableTop: 0xdbab72,
  tableShade: 0x8f6335,
  laptopLid: 0x4c5a6e,
  laptopBody: 0x36414f,
  coniferTrunk: 0x6b4a2c,
  coniferDark: 0x24583a,
  coniferHi: 0x3f8a58,
  boulder: 0x8b8377,
  boulderHi: 0xaba398,
  fireLog: 0x7a4f2c,
  fireFlame: 0xf2803a,
  fireCore: 0xf7d154,
  ember: 0xd9542f,
  kettle: 0x3d4148,
  kettleHi: 0x6d727c,
  deckWood: 0xb98652,
  deckTop: 0xd7a774,
  cabinLog: 0x8f6335,
  cabinLogHi: 0xb28150,
  cabinRoof: 0x5a4030,
  cabinDoor: 0x3f2c1e,
  cabinWindow: 0xf2d05a,
  smoke: 0xc3c7c9,
};

type ValleyPalette = typeof VALLEY_PALETTE;

/** 레터박스(맵 밖) 배경 — 풀밭보다 어두운 숲 그늘색. */
const VALLEY_BACKGROUND = 0x3f5a3c;

/** 결정적 흩뿌리기(자갈/들꽃/나무 배치). 베이크된 정적 텍스처라 난수 금지. */
const scatter = (tx: number, ty: number, mod: number): number => (tx * 97 + ty * 41) % mod;

function valleyTileDraw(pal: ValleyPalette): TileDrawFn {
  return (g, { t, tx, ty, s, map }) => {
    const grass = (tx + ty) % 2 === 0 ? pal.grassA : pal.grassB;
    switch (t) {
      case Tile.Floor: {
        g.rect(0, 0, s, s).fill(grass);
        g.rect(2, 4, 1, 2).fill(pal.grassDot); // 풀포기 1px
        g.rect(s - 4, s - 6, 1, 2).fill(pal.grassDot);
        // 해변과 같은 이유로 드물게 — 풀밭이 잡동사니로 보이지 않을 만큼만.
        const k = scatter(tx, ty, 13);
        if (k === 0) {
          // 자갈길 조각
          g.rect(4, 7, 3, 2).fill(pal.pebble);
          g.rect(4, 7, 2, 1).fill(pal.pebbleHi);
          g.rect(9, 10, 3, 2).fill(pal.pebble);
          g.rect(9, 10, 2, 1).fill(pal.pebbleHi);
          g.rect(8, 5, 2, 1).fill(pal.pebble);
        } else if (k === 6) {
          // 들꽃 두 송이
          g.rect(5, 9, 1, 3).fill(pal.forestHi);
          g.rect(4, 8, 3, 1).fill(pal.flower);
          g.rect(10, 6, 1, 3).fill(pal.forestHi);
          g.rect(9, 5, 3, 1).fill(pal.flower);
        }
        break;
      }
      case Tile.Wall: {
        if (ty === 0) {
          // 먼 설산 능선: 하늘 → 암벽 → 눈 덮인 봉우리
          g.rect(0, 0, s, s).fill(pal.ridgeSky);
          const peak = scatter(tx, ty, 4); // 봉우리 높이를 칸마다 흔든다
          g.rect(0, 4 + peak, s, s - 4 - peak).fill(pal.ridgeRock);
          g.rect(0, 4 + peak, s, 2).fill(pal.ridgeSnow);
          g.rect(3, 6 + peak, 2, 2).fill(pal.ridgeSnow);
          g.rect(10, 7 + peak, 3, 1).fill(pal.ridgeSnow);
          break;
        }
        if (ty === 1) {
          // 침엽수림 실루엣: 칸마다 두 그루씩 어긋나게
          g.rect(0, 0, s, s).fill(pal.forestDark);
          const off = scatter(tx, ty, 3);
          g.rect(2, 3 + off, 4, 9).fill(pal.forestMid);
          g.rect(3, 1 + off, 2, 3).fill(pal.forestMid);
          g.rect(3, 4 + off, 2, 2).fill(pal.forestHi);
          g.rect(9, 5 - off, 5, 9).fill(pal.forestMid);
          g.rect(10, 3 - off, 3, 3).fill(pal.forestMid);
          g.rect(10, 6 - off, 2, 2).fill(pal.forestHi);
          break;
        }
        if (ty === map.height - 1) {
          // 계곡 물줄기(하단): 물 + 흰 포말 + 돌 턱
          g.rect(0, 0, s, s).fill(pal.streamDeep);
          g.rect(0, 0, s, 3).fill(pal.rockWall); // 물가 돌 턱
          g.rect(0, 3, s, 1).fill(pal.rockTop);
          g.rect(0, 6, s, 2).fill(pal.streamMid);
          g.rect(scatter(tx, ty, 6) + 1, 9, 5, 1).fill(pal.streamFoam);
          g.rect(scatter(tx, ty + 1, 5) + 7, 12, 4, 1).fill(pal.streamFoam);
          break;
        }
        // 측면 바위벽 + 이끼
        g.rect(0, 0, s, s).fill(pal.rockWall);
        g.rect(0, 0, s, 3).fill(pal.rockTop);
        g.rect(2, 6, 4, 1).fill(pal.rockTop);
        g.rect(9, 10, 4, 1).fill(pal.rockTop);
        if (scatter(tx, ty, 3) === 0) {
          g.rect(3, 3, 3, 2).fill(pal.moss);
          g.rect(10, 4, 2, 2).fill(pal.moss);
        }
        break;
      }
      case Tile.Rug: {
        // 캠핑 돗자리: 격자 체크
        g.rect(0, 0, s, s).fill(pal.mat);
        g.rect(0, 3, s, 1).fill(pal.matPlaid);
        g.rect(0, 11, s, 1).fill(pal.matPlaid);
        g.rect(3, 0, 1, s).fill(pal.matPlaid);
        g.rect(11, 0, 1, s).fill(pal.matPlaid);
        g.rect(0, 0, s, 1).fill(pal.matPlaid);
        break;
      }
      case Tile.DeskTop: {
        const isLeft = map.tiles[ty][tx - 1] !== Tile.DeskTop;
        g.rect(0, 0, s, s).fill(grass); // 가구 타일은 바닥 베이크에서 빠지므로 스스로 풀을 깐다
        // 접이식 캠핑 테이블: 상판 + 앞치마 + X 다리
        g.rect(0, 8, s, 3).fill(pal.tableTop);
        g.rect(0, 11, s, 2).fill(pal.tableWood);
        g.rect(0, 13, s, 1).fill(pal.tableShade);
        g.rect(2, 13, 2, 3).fill(pal.tableWood);
        g.rect(12, 13, 2, 3).fill(pal.tableWood);
        g.rect(4, 14, 8, 1).fill(pal.tableShade); // 다리 사이 크로스바
        if (isLeft) {
          // 랩탑(뒷모습) — 오피스와 같은 이디엄
          g.rect(4, 7, 8, 1).fill(pal.laptopBody);
          g.rect(4, 3, 8, 4).fill(pal.laptopLid);
          g.rect(4, 6, 8, 1).fill(pal.laptopBody);
          g.rect(7, 4, 2, 2).fill(pal.laptopBody);
        } else {
          g.rect(5, 4, 5, 4).fill(pal.kettleHi); // 짝 칸에는 머그 두 개
          g.rect(5, 4, 5, 1).fill(pal.kettle);
          g.rect(10, 5, 1, 2).fill(pal.kettle);
        }
        break;
      }
      case Tile.Plant: {
        g.rect(0, 0, s, s).fill(grass);
        // 열 기준으로 번갈아 — 이 맵의 Plant 두 자리(tx1/tx18)가 각각 다른
        // 모습이 되도록(합 기준이면 둘 다 같은 쪽으로 떨어진다).
        if (tx % 2 === 0) {
          // 침엽수: 3단 삼각 + 줄기
          g.rect(7, 12, 2, 4).fill(pal.coniferTrunk);
          g.rect(3, 9, 10, 3).fill(pal.coniferDark);
          g.rect(4, 6, 8, 3).fill(pal.coniferDark);
          g.rect(5, 3, 6, 3).fill(pal.coniferDark);
          g.rect(6, 1, 4, 2).fill(pal.coniferHi);
          g.rect(5, 7, 3, 1).fill(pal.coniferHi);
          g.rect(4, 10, 3, 1).fill(pal.coniferHi);
        } else {
          // 이끼 낀 바위
          g.rect(2, 7, 12, 8).fill(pal.boulder);
          g.rect(3, 5, 9, 3).fill(pal.boulder);
          g.rect(4, 5, 6, 2).fill(pal.boulderHi);
          g.rect(3, 9, 4, 1).fill(pal.boulderHi);
          g.rect(9, 11, 4, 2).fill(pal.moss);
          g.rect(2, 14, 12, 1).fill(pal.tableShade);
        }
        break;
      }
      case Tile.Counter: {
        g.rect(0, 0, s, s).fill(grass);
        // 돌로 두른 화덕은 모든 칸 공통
        g.rect(1, 10, 14, 5).fill(pal.rockWall);
        g.rect(1, 10, 14, 1).fill(pal.rockTop);
        g.rect(3, 12, 10, 2).fill(pal.ember);
        if (tx % 2 === 0) {
          // 장작 + 불꽃
          g.rect(3, 9, 10, 2).fill(pal.fireLog);
          g.rect(5, 4, 6, 6).fill(pal.fireFlame);
          g.rect(7, 2, 3, 4).fill(pal.fireCore);
          g.rect(6, 6, 2, 3).fill(pal.fireCore);
        } else {
          // 삼각대에 건 주전자
          g.rect(2, 1, 1, 9).fill(pal.fireLog); // 삼각대 다리
          g.rect(13, 1, 1, 9).fill(pal.fireLog);
          g.rect(2, 1, 12, 1).fill(pal.fireLog); // 가로대
          g.rect(7, 2, 2, 2).fill(pal.kettle); // 걸이 고리
          g.rect(5, 4, 6, 5).fill(pal.kettleHi); // 주전자 몸통(밝게 — 어두우면 상자로 보인다)
          g.rect(6, 4, 4, 1).fill(pal.kettle); // 뚜껑
          g.rect(5, 8, 6, 1).fill(pal.kettle); // 바닥 그을음
          g.rect(11, 5, 3, 1).fill(pal.kettleHi); // 주둥이
          g.rect(3, 5, 2, 1).fill(pal.kettle); // 손잡이
          g.rect(3, 9, 10, 1).fill(pal.fireFlame); // 잔불
        }
        break;
      }
      case Tile.Table: {
        // 평상: 널판 데크 + 굵은 다리
        g.rect(0, 0, s, s).fill(grass);
        g.rect(0, 4, s, 8).fill(pal.deckWood);
        g.rect(0, 4, s, 2).fill(pal.deckTop);
        g.rect(0, 7, s, 1).fill(pal.tableShade); // 널판 이음매
        g.rect(0, 10, s, 1).fill(pal.tableShade);
        g.rect(1, 12, 3, 3).fill(pal.tableShade);
        g.rect(12, 12, 3, 3).fill(pal.tableShade);
        break;
      }
      case Tile.BossDesk: {
        // 통나무 오두막(세로 1×2): 위 칸이 지붕·굴뚝, 아래 칸이 통나무 벽·문.
        const isLower = map.tiles[ty - 1]?.[tx] === Tile.BossDesk;
        g.rect(0, 0, s, s).fill(grass);
        if (!isLower) {
          g.rect(11, 0, 3, 5).fill(pal.cabinRoof); // 굴뚝
          g.rect(11, 0, 3, 1).fill(pal.cabinLogHi);
          g.rect(12, 0, 2, 1).fill(pal.smoke); // 연기 한 줄기
          g.rect(1, 5, 14, 4).fill(pal.cabinRoof); // 박공 지붕
          g.rect(0, 9, s, 2).fill(pal.cabinLogHi); // 처마
          g.rect(2, 11, 12, 5).fill(pal.cabinLog); // 통나무 벽 상단
          g.rect(2, 13, 12, 1).fill(pal.cabinRoof); // 통나무 이음매
          g.rect(4, 6, 3, 3).fill(pal.cabinLogHi); // 지붕 하이라이트
        } else {
          g.rect(2, 0, 12, 14).fill(pal.cabinLog);
          g.rect(2, 3, 12, 1).fill(pal.cabinRoof); // 통나무 이음매
          g.rect(2, 8, 12, 1).fill(pal.cabinRoof);
          g.rect(3, 1, 4, 4).fill(pal.cabinWindow); // 창
          g.rect(3, 1, 4, 1).fill(pal.cabinRoof);
          g.rect(8, 5, 5, 9).fill(pal.cabinDoor); // 문
          g.rect(11, 9, 1, 2).fill(pal.cabinLogHi); // 손잡이
          g.rect(2, 14, 12, 2).fill(pal.tableShade); // 바닥 그림자
        }
        break;
      }
    }
  };
}

export const VALLEY_SCENE: SceneDef = {
  id: "valley",
  label: "산 계곡",
  map: VALLEY_MAP,
  resolve: (theme) => {
    const mode = sceneColorMode(theme.id);
    return {
      background: adaptColor(VALLEY_BACKGROUND, mode),
      drawTile: valleyTileDraw(adaptPalette(VALLEY_PALETTE, mode)),
    };
  },
};
