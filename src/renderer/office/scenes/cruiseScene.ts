// src/renderer/office/scenes/cruiseScene.ts
//
// 호화 크루즈 선상 데크 풍경. 해변·계곡 씬과 같은 규칙(의미 타일 공유,
// 어휘만 교체):
//   Floor=티크 갑판, Wall=난간 너머 먼 바다(위 2줄)/흰 선체 벽(그 외),
//   Rug=수영장 물(얕은 풀 — walkable이라 캐릭터가 물에 들어가 쉰다),
//   DeskTop=차양 아래 선상 작업대, Plant=화분 야자, Counter=칵테일 바,
//   Table=풀사이드 라운드 테이블, BossDesk=선장 브리지.
//
// 레이아웃은 해변·계곡과 일부러 다르게 잡았다 — 맵 한가운데를 수영장이
// 차지하고(라운지=풀), 작업대 두 줄이 풀의 위/아래로 갈라서며, 칵테일 바는
// 좌우 현측에 붙는다. 선장 브리지는 우측 하단(선미)에 서고 줄서기 레인은
// 그 하단 행을 따라 서쪽으로 뻗는다(방향은 `buildSceneMap`이 유도한다).
//
// 팔레트는 "한낮의 원색" 한 벌만 두고, 어두운/모노크롬 테마에서는
// sceneColor.ts가 자동 변환한다.
import type { TileRect } from "../map/mapData";
import { L, Tile, buildSceneMap } from "../map/mapData";
import { adaptColor, adaptPalette, sceneColorMode } from "./sceneColor";
import type { SceneDef, TileDrawFn } from "./sceneTypes";

// 작업대는 ty3(4쌍)·ty10(4쌍) = 좌석 8개로 오피스와 같다. 두 줄의 열 위상을
// 한 칸 어긋나게 두어(ty3=tx2/6/10/14, ty10=tx3/7/11/15) 풀을 사이에 둔
// 좌우 비대칭 갑판처럼 읽히게 했다.
const GRID: Tile[][] = [
  L('WWWWWWWWWWWWWWWWWWWW'), // ty0  - 하늘 + 수평선 + 먼 바다
  L('WWWWWWWWWWWWWWWWWWWW'), // ty1  - 난간 너머 물결 + 흰 난간
  L('WFFFFFFFFFFFFFFFFFFW'), // ty2  - 선수 프롬나드(좌석 행 1)
  L('WPDDFFDDFFDDFFDDFFPW'), // ty3  - 차양 작업대 4쌍 + 야자 화분(tx1/tx18)
  L('WFFFFFFFFFFFFFFFFFFW'), // ty4  - 산책로
  L('WFFTFFRRRRRRRRFFTFFW'), // ty5  - 라운드 테이블 + 수영장 상단
  L('WCFFFFRRRRRRRRFFFFCW'), // ty6  - 칵테일 바(양 현측) + 수영장
  L('WCFFFFRRRRRRRRFFFFCW'), // ty7  - 칵테일 바 + 수영장
  L('WFFTFFRRRRRRRRFFTFFW'), // ty8  - 라운드 테이블 + 수영장 하단
  L('WFFFFFFFFFFFFFFFFFFW'), // ty9  - 산책로(좌석 행 2)
  L('WFFDDFFDDFFDDFFDDFFW'), // ty10 - 차양 작업대 4쌍(한 칸 어긋난 위상)
  L('WFFFFFFFFFFFFFFFFBFW'), // ty11 - 선장 브리지 상단(tx17)
  L('WPFFFFFFFFFFFFFFFBFW'), // ty12 - 브리지 하단 + 줄서기 레인(서쪽) + 화분
  L('WWWWWWWWWWWWWWWWWWWW'), // ty13 - 선체 벽
];

/** 수영장 = 이 씬의 라운지. 물 타일이 walkable이라 사각형 전체가 계약을 만족한다. */
const BREAK_ROOM: TileRect = { x: 6, y: 5, w: 8, h: 4 };

export const CRUISE_MAP = buildSceneMap(GRID, BREAK_ROOM);

/** 바다가 보이는 행 — 이 위(포함)는 Wall을 바다·난간으로, 아래는 선체 벽으로 그린다. */
const SEA_ROWS = 2;

const CRUISE_PALETTE = {
  teakA: 0xc9a06a,
  teakB: 0xbe9460,
  teakSeam: 0x8f6a3c,
  teakKnot: 0xa87c4a,
  ropeTan: 0xe2cca2,
  skyA: 0x9fd3ef,
  skyHi: 0xdcf1fb,
  seaDeep: 0x14507e,
  seaHorizon: 0x0d3a5e,
  seaMid: 0x2b86bb,
  seaFoam: 0xdff2fb,
  railWhite: 0xf4f6f8,
  railShadow: 0xbcc5cd,
  hullWhite: 0xe9edf1,
  hullShadow: 0xb3bdc7,
  hullStripe: 0x1d4f7c,
  brass: 0xd9a83f,
  brassHi: 0xf4d97e,
  poolWater: 0x2fb3c4,
  poolDeep: 0x1a7f97,
  poolFoam: 0xd9f6fb,
  poolTile: 0xf1f6f7,
  poolLine: 0x8fb6c2,
  awningA: 0x2b5fa8,
  awningB: 0xf4f6f8,
  awningTrim: 0x1d4270,
  deskWood: 0xb98652,
  deskTop: 0xdcb27e,
  deskShade: 0x8a6236,
  laptopLid: 0x4c5a6e,
  laptopBody: 0x36414f,
  potTerra: 0xc4653f,
  potHi: 0xdd8058,
  palmTrunk: 0xa9743f,
  palmFrond: 0x3f9e5e,
  palmFrondHi: 0x5cbf78,
  barWood: 0x7b4a2c,
  barTop: 0xa46a3f,
  barTopHi: 0xc98f57,
  glass: 0xd8f2f5,
  champagne: 0xf0d27a,
  bottleGreen: 0x2f6b46,
  cloth: 0xf4f2ea,
  clothShade: 0xd0c9b8,
  bridgeWall: 0xeceff2,
  bridgeRoof: 0x24547f,
  bridgeWindow: 0x2f5f86,
  bridgeWindowHi: 0x76b0d3,
  mast: 0xa9b3bb,
  helm: 0x8a5a30,
  helmHi: 0xc08a52,
};

type CruisePalette = typeof CRUISE_PALETTE;

/** 레터박스(맵 밖) 배경 — 갑판보다 훨씬 어두운 먼바다 남색. */
const CRUISE_BACKGROUND = 0x0d3352;

/** 타일 좌표에서 나오는 결정적 해시 — 갑판 옹이/물결 하이라이트를 흩뿌리는 데
 * 쓴다. (베이크된 정적 텍스처라 난수를 쓰면 재베이크마다 무늬가 바뀐다.) */
const scatter = (tx: number, ty: number, mod: number): number => (tx * 53 + ty * 113) % mod;

function cruiseTileDraw(pal: CruisePalette): TileDrawFn {
  /** 티크 갑판 바닥 — 가구 타일도 바닥 베이크에서 빠지므로 스스로 이걸 깐다.
   * 널빤지 이음선 위상은 tx와 무관하게 고정해 칸 경계를 넘어 결이 이어진다. */
  const deck = (g: Parameters<TileDrawFn>[0], tx: number, ty: number, s: number): void => {
    g.rect(0, 0, s, s).fill(pal.teakA);
    g.rect(5, 0, 5, s).fill(pal.teakB); // 널 하나만 톤을 살짝 달리해 결이 보이게
    g.rect(4, 0, 1, s).fill(pal.teakSeam); // 코킹(이음선)
    g.rect(10, 0, 1, s).fill(pal.teakSeam);
    g.rect(15, 0, 1, s).fill(pal.teakSeam);
    g.rect(0, 3 + scatter(tx, ty, 9), 4, 1).fill(pal.teakSeam); // 널 이음(버트 조인트)
  };

  return (g, { t, tx, ty, s, map }) => {
    switch (t) {
      case Tile.Floor: {
        deck(g, tx, ty, s);
        // 장식은 드물게 — 촘촘하면 잘 닦인 갑판이 아니라 잡동사니로 보인다.
        const k = scatter(tx, ty, 13);
        if (k === 0) {
          // 옹이: 가운데가 짙은 타원
          g.rect(6, 7, 3, 2).fill(pal.teakKnot);
          g.rect(7, 6, 1, 1).fill(pal.teakKnot);
          g.rect(7, 9, 1, 1).fill(pal.teakKnot);
          g.rect(7, 7, 1, 1).fill(pal.teakSeam);
        } else if (k === 7) {
          // 황동 데크 링에 사려 놓은 로프
          g.rect(11, 9, 4, 1).fill(pal.brass);
          g.rect(11, 13, 4, 1).fill(pal.brass);
          g.rect(10, 10, 1, 3).fill(pal.brass);
          g.rect(15, 10, 1, 3).fill(pal.brass);
          g.rect(12, 10, 2, 1).fill(pal.ropeTan);
          g.rect(11, 11, 4, 1).fill(pal.ropeTan);
          g.rect(12, 12, 2, 1).fill(pal.ropeTan);
        }
        break;
      }
      case Tile.Wall: {
        if (ty < SEA_ROWS) {
          if (ty === 0) {
            // 먼 바다: 하늘 → 수평선 → 물. 수평선은 1px로 딱 끊어야 선처럼 읽힌다.
            g.rect(0, 0, s, s).fill(pal.seaDeep);
            g.rect(0, 0, s, 5).fill(pal.skyA);
            g.rect(scatter(tx, ty, 8) + 1, 1, 5, 1).fill(pal.skyHi); // 엷은 구름
            g.rect(scatter(tx, ty + 1, 6) + 8, 3, 4, 1).fill(pal.skyHi);
            g.rect(0, 5, s, 1).fill(pal.seaHorizon);
            g.rect(scatter(tx, ty, 6) + 2, 9, 4, 1).fill(pal.seaMid); // 먼 물결(1px 대시)
            g.rect(scatter(tx, ty + 1, 5) + 9, 13, 3, 1).fill(pal.seaMid);
            break;
          }
          // 뱃전 바로 너머의 물 + 흰 난간(선체 가장자리)
          g.rect(0, 0, s, s).fill(pal.seaMid);
          g.rect(0, 0, s, 3).fill(pal.seaDeep);
          g.rect(scatter(tx, ty, 7) + 2, 4, 4, 1).fill(pal.seaFoam);
          g.rect(scatter(tx, ty + 1, 6) + 7, 6, 3, 1).fill(pal.seaFoam);
          g.rect(0, 8, s, 2).fill(pal.railWhite); // 손잡이 가로대
          g.rect(0, 10, s, 1).fill(pal.railShadow);
          g.rect(3, 10, 1, 3).fill(pal.railWhite); // 난간 지주
          g.rect(11, 10, 1, 3).fill(pal.railWhite);
          g.rect(0, 12, s, 1).fill(pal.railWhite); // 중간 가로대
          g.rect(0, 13, s, 3).fill(pal.hullWhite); // 갑판 가장자리 코밍
          g.rect(0, 13, s, 1).fill(pal.railWhite);
          break;
        }
        // 흰 선체 벽(측면·하단): 네이비 띠 + 드문드문 현창
        g.rect(0, 0, s, s).fill(pal.hullWhite);
        g.rect(0, 0, s, 2).fill(pal.railWhite);
        g.rect(0, 5, s, 3).fill(pal.hullStripe);
        g.rect(0, 8, s, 1).fill(pal.hullShadow);
        if (scatter(tx, ty, 3) === 0) {
          g.rect(5, 10, 6, 5).fill(pal.brass); // 현창 테
          g.rect(6, 11, 4, 3).fill(pal.bridgeWindow);
          g.rect(6, 11, 4, 1).fill(pal.bridgeWindowHi);
          g.rect(5, 10, 6, 1).fill(pal.brassHi);
        }
        break;
      }
      case Tile.Rug: {
        // 수영장 물. 가장자리(이웃이 물이 아닌 쪽)에는 흰 타일 코핑을 둘러
        // 풀의 윤곽이 칸 단위가 아니라 하나의 웅덩이로 읽히게 한다.
        const isPool = (x: number, y: number): boolean => map.tiles[y]?.[x] === Tile.Rug;
        g.rect(0, 0, s, s).fill(pal.poolWater);
        g.rect(scatter(tx, ty, 7) + 1, 4, 5, 1).fill(pal.poolFoam); // 물결 하이라이트
        g.rect(scatter(tx, ty + 1, 6) + 7, 10, 4, 1).fill(pal.poolFoam);
        g.rect(9, 6, 4, 1).fill(pal.poolDeep); // 깊은 쪽 그늘
        g.rect(2, 12, 3, 1).fill(pal.poolDeep);
        if (!isPool(tx, ty - 1)) {
          g.rect(0, 0, s, 3).fill(pal.poolTile);
          g.rect(0, 3, s, 1).fill(pal.poolLine);
        }
        if (!isPool(tx, ty + 1)) {
          g.rect(0, s - 3, s, 3).fill(pal.poolTile);
          g.rect(0, s - 4, s, 1).fill(pal.poolLine);
        }
        if (!isPool(tx - 1, ty)) {
          g.rect(0, 0, 3, s).fill(pal.poolTile);
          g.rect(3, 0, 1, s).fill(pal.poolLine);
        }
        if (!isPool(tx + 1, ty)) {
          g.rect(s - 3, 0, 3, s).fill(pal.poolTile);
          g.rect(s - 4, 0, 1, s).fill(pal.poolLine);
        }
        break;
      }
      case Tile.DeskTop: {
        const isLeft = map.tiles[ty][tx - 1] !== Tile.DeskTop;
        deck(g, tx, ty, s);
        // 차양(어닝): 4px 줄무늬. 줄 위상을 tx로 이어 붙여 2칸짜리 쌍이 하나의
        // 차양으로 읽히게 한다(칸 단위로 통짜 색을 칠하면 청백 블록처럼 보인다).
        g.rect(0, 0, s, 4).fill(pal.awningB);
        for (let x = tx % 2 === 0 ? 0 : 4; x < s; x += 8) {
          g.rect(x, 0, 4, 4).fill(pal.awningA);
        }
        g.rect(0, 0, s, 1).fill(pal.awningB); // 꼭대기 하이라이트
        g.rect(0, 4, s, 1).fill(pal.awningTrim); // 처마 끝단
        for (let x = 1; x < s; x += 4) g.rect(x, 5, 2, 1).fill(pal.awningB); // 스캘럽 프릴
        if (isLeft) g.rect(s - 1, 5, 1, 6).fill(pal.railShadow); // 차양 지주는 쌍의 이음매에
        // 작업대 상판
        g.rect(0, 10, s, 5).fill(pal.deskWood);
        g.rect(0, 10, s, 2).fill(pal.deskTop);
        g.rect(0, 14, s, 1).fill(pal.deskShade);
        if (isLeft) {
          // 랩탑(뒷모습) — 오피스와 같은 이디엄(뚜껑 등판이 뷰어를 향한다)
          g.rect(4, 9, 8, 1).fill(pal.laptopBody);
          g.rect(4, 5, 8, 4).fill(pal.laptopLid);
          g.rect(4, 8, 8, 1).fill(pal.laptopBody);
          g.rect(7, 6, 2, 2).fill(pal.laptopBody);
        } else {
          // 짝 칸에는 아이스 버킷에 꽂힌 샴페인
          g.rect(5, 4, 2, 5).fill(pal.champagne); // 병목
          g.rect(4, 6, 4, 3).fill(pal.bottleGreen);
          g.rect(4, 9, 8, 4).fill(pal.railWhite); // 버킷
          g.rect(4, 9, 8, 1).fill(pal.railShadow);
          g.rect(9, 6, 3, 3).fill(pal.glass); // 잔
          g.rect(10, 9, 1, 2).fill(pal.glass);
        }
        break;
      }
      case Tile.Plant: {
        deck(g, tx, ty, s);
        // 테라코타 화분에 심은 야자. 열 기준으로 잎 방향을 번갈아 — 이 맵의
        // 화분 세 자리가 전부 같은 모습이 되지 않게(합 기준이면 뭉친다).
        g.rect(3, 10, 10, 2).fill(pal.potTerra); // 화분 테두리
        g.rect(3, 10, 10, 1).fill(pal.potHi);
        g.rect(4, 12, 8, 4).fill(pal.potTerra);
        g.rect(4, 12, 1, 4).fill(pal.potHi);
        g.rect(7, 6, 2, 5).fill(pal.palmTrunk); // 줄기
        g.rect(3, 3, 10, 2).fill(pal.palmFrond); // 잎
        g.rect(5, 1, 6, 2).fill(pal.palmFrondHi);
        g.rect(1, 5, 4, 1).fill(pal.palmFrond);
        g.rect(11, 5, 4, 1).fill(pal.palmFrond);
        if (tx % 2 === 0) {
          g.rect(2, 6, 3, 1).fill(pal.palmFrondHi);
          g.rect(12, 7, 2, 1).fill(pal.palmFrondHi);
        } else {
          g.rect(2, 7, 2, 1).fill(pal.palmFrondHi);
          g.rect(11, 6, 3, 1).fill(pal.palmFrondHi);
        }
        break;
      }
      case Tile.Counter: {
        // 칵테일 바: 마호가니 카운터 + 백바 선반 + 황동 풋레일.
        // 이 맵의 Counter는 세로로 쌓이므로(현측 두 칸) ty 기준으로 번갈아 꾸민다.
        deck(g, tx, ty, s);
        g.rect(0, 1, s, 4).fill(pal.barWood); // 백바 선반
        g.rect(0, 4, s, 1).fill(pal.barTopHi);
        if (ty % 2 === 0) {
          g.rect(2, 1, 2, 3).fill(pal.bottleGreen); // 병 진열
          g.rect(5, 2, 2, 2).fill(pal.champagne);
          g.rect(8, 1, 2, 3).fill(pal.glass);
          g.rect(11, 2, 2, 2).fill(pal.bottleGreen);
        } else {
          g.rect(3, 1, 3, 3).fill(pal.champagne);
          g.rect(7, 2, 2, 2).fill(pal.glass);
          g.rect(10, 1, 3, 3).fill(pal.champagne);
        }
        g.rect(0, 8, s, 6).fill(pal.barWood); // 카운터
        g.rect(0, 8, s, 2).fill(pal.barTop);
        g.rect(0, 8, s, 1).fill(pal.barTopHi);
        g.rect(0, 13, s, 1).fill(pal.brass); // 황동 풋레일
        if (ty % 2 === 0) {
          // 샴페인 잔 세 개(역삼각 보울 + 스템)
          g.rect(2, 5, 3, 2).fill(pal.glass);
          g.rect(3, 7, 1, 1).fill(pal.glass);
          g.rect(7, 5, 3, 2).fill(pal.champagne);
          g.rect(8, 7, 1, 1).fill(pal.glass);
          g.rect(12, 5, 3, 2).fill(pal.glass);
          g.rect(13, 7, 1, 1).fill(pal.glass);
        } else {
          // 칵테일 잔 + 우산 장식
          g.rect(4, 5, 5, 2).fill(pal.glass);
          g.rect(5, 7, 3, 1).fill(pal.champagne);
          g.rect(6, 4, 1, 1).fill(pal.brassHi);
          g.rect(11, 5, 3, 3).fill(pal.bottleGreen);
          g.rect(11, 5, 3, 1).fill(pal.brassHi);
        }
        break;
      }
      case Tile.Table: {
        // 풀사이드 라운드 테이블: 흰 리넨을 덮은 원형 상판 + 황동 외다리.
        deck(g, tx, ty, s);
        g.rect(6, 1, 4, 2).fill(pal.glass); // 상판 위 음료
        g.rect(7, 3, 2, 1).fill(pal.champagne);
        g.rect(4, 4, 8, 1).fill(pal.cloth); // 원형 실루엣(계단식 4단)
        g.rect(2, 5, 12, 2).fill(pal.cloth);
        g.rect(1, 7, 14, 3).fill(pal.cloth);
        g.rect(2, 10, 12, 1).fill(pal.clothShade);
        g.rect(4, 11, 8, 1).fill(pal.clothShade); // 늘어진 리넨 자락
        g.rect(1, 7, 14, 1).fill(pal.railWhite); // 상판 하이라이트
        g.rect(7, 12, 2, 3).fill(pal.brass); // 기둥
        g.rect(5, 15, 6, 1).fill(pal.brassHi); // 받침
        break;
      }
      case Tile.BossDesk: {
        // 선장 브리지(세로 1×2): 위 칸이 레이더 마스트·파노라마 창,
        // 아래 칸이 조타륜이 보이는 선교 정면.
        const isLower = map.tiles[ty - 1]?.[tx] === Tile.BossDesk;
        deck(g, tx, ty, s);
        if (!isLower) {
          g.rect(7, 0, 2, 4).fill(pal.mast); // 마스트
          g.rect(3, 1, 10, 1).fill(pal.mast); // 회전 레이더 바
          g.rect(3, 0, 3, 1).fill(pal.brassHi);
          g.rect(1, 4, 14, 2).fill(pal.bridgeRoof); // 지붕
          g.rect(1, 4, 14, 1).fill(pal.railWhite);
          g.rect(2, 6, 12, 9).fill(pal.bridgeWall); // 선교 벽
          g.rect(3, 8, 10, 4).fill(pal.bridgeWindow); // 파노라마 창
          g.rect(3, 8, 10, 1).fill(pal.bridgeWindowHi);
          g.rect(7, 8, 1, 4).fill(pal.bridgeWall); // 창틀 분할
          g.rect(2, 13, 12, 2).fill(pal.hullStripe); // 네이비 띠
        } else {
          g.rect(2, 0, 12, 14).fill(pal.bridgeWall);
          g.rect(2, 0, 12, 1).fill(pal.hullShadow);
          g.rect(4, 3, 8, 8).fill(pal.helm); // 조타륜 바깥 링
          g.rect(5, 4, 6, 6).fill(pal.bridgeWall); // 링 속을 비운다
          g.rect(7, 3, 2, 8).fill(pal.helm); // 세로 스포크
          g.rect(4, 6, 8, 2).fill(pal.helm); // 가로 스포크
          g.rect(7, 1, 2, 2).fill(pal.helmHi); // 손잡이 넷
          g.rect(7, 12, 2, 2).fill(pal.helmHi);
          g.rect(2, 6, 2, 2).fill(pal.helmHi);
          g.rect(12, 6, 2, 2).fill(pal.helmHi);
          g.rect(5, 4, 1, 1).fill(pal.helmHi); // 림 하이라이트
          g.rect(10, 9, 1, 1).fill(pal.helmHi);
          g.rect(2, 14, 12, 2).fill(pal.hullShadow); // 갑판에 진 그림자
        }
        break;
      }
    }
  };
}

export const CRUISE_SCENE: SceneDef = {
  id: "cruise",
  label: "호화 크루즈",
  map: CRUISE_MAP,
  resolve: (theme) => {
    const mode = sceneColorMode(theme.id);
    return {
      background: adaptColor(CRUISE_BACKGROUND, mode),
      drawTile: cruiseTileDraw(adaptPalette(CRUISE_PALETTE, mode)),
    };
  },
};
