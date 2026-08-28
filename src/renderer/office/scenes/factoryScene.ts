// src/renderer/office/scenes/factoryScene.ts
//
// 공장 작업장 풍경. 해변/계곡과 같은 규칙(의미 타일 공유, 어휘만 교체):
//   Floor=콘크리트 바닥(균열·기름 얼룩·볼트 자국), Wall=벽돌 벽(ty0 채광창 /
//   ty1 배관 / 하단 셔터 문 / 측면 벽돌+소화기), Rug=노란 빗금 안전구역,
//   DeskTop=작업 벤치(공구·부품 + 롤러 컨베이어), Plant=드럼통 옆 화분,
//   Counter=공구 대여 카운터, Table=검수 테이블, BossDesk=관리실 제어반.
//
// 레이아웃은 해변·계곡과 일부러 다르게 잡았다 — 벤치를 두 줄 다 5쌍씩(=좌석
// 10개) 반 칸 어긋나게 깔아 생산 라인처럼 보이게 하고, 제어반(보스 자리)은
// 벽을 등지지 않고 작업장 한가운데(tx11)에 섬처럼 선다. 줄서기 레인은 그
// 하단 행을 따라 서쪽으로 뻗고(`buildSceneMap`이 보스 위치에서 유도),
// 안전구역 라운지는 좌측 하단·공구 카운터는 우측 하단에 둔다.
import type { Graphics } from "pixi.js";

import type { TileRect } from "../map/mapData";
import { L, Tile, buildSceneMap } from "../map/mapData";
import type { QuietGroup } from "./sceneColor";
import { defineScene } from "./defineScene";
import type { SceneDef, TileDrawFn } from "./sceneTypes";

const GRID: Tile[][] = [
  L('WWWWWWWWWWWWWWWWWWWW'), // ty0  - 벽돌 벽 상단 + 채광창
  L('WWWWWWWWWWWWWWWWWWWW'), // ty1  - 배관·덕트가 지나는 벽 하단
  L('WFFFFFFFFFFFFFFFFFFW'), // ty2  - 좌석 행 1
  L('WFDDFDDFDDFDDFDDFFPW'), // ty3  - 작업 벤치 라인 A 5쌍 + 드럼통 화분(tx18)
  L('WFFFFFFFFFFFFFFFFFFW'), // ty4  - 라인 사이 통로
  L('WFFFFFFFFFFFFFFFFFFW'), // ty5  - 좌석 행 2
  L('WPFDDFDDFDDFDDFDDFFW'), // ty6  - 드럼통 화분(tx1) + 벤치 라인 B 5쌍(반 칸 어긋남)
  L('WFFFFFFFFFFFFFFFFFFW'), // ty7  - 통로
  L('WFTTFFFFFFFBFFFFFFFW'), // ty8  - 검수 테이블(tx2·3) + 관리실 제어반 상단(tx11)
  L('WFFFFFFFFFFBFFFFFFFW'), // ty9  - 제어반 하단 + 줄서기 레인(서쪽)
  L('WRRRRRRRRRFFFFFFFFFW'), // ty10 - 노란 빗금 안전구역(휴게 존)
  L('WRRRRRRRRRFFFFFFFFPW'), // ty11 - 안전구역 + 드럼통 화분(tx18)
  L('WFFFFFFFFFFFCCCCCCFW'), // ty12 - 공구 대여 카운터
  L('WWWWWWWWWWWWWWWWWWWW'), // ty13 - 하단 벽 + 셔터 문
];

/** 노란 빗금 안전구역 안의 쉼터 — 오피스 러그 라운지와 같은 역할. */
const BREAK_ROOM: TileRect = { x: 2, y: 10, w: 5, h: 2 };

export const FACTORY_MAP = buildSceneMap(GRID, BREAK_ROOM);

const FACTORY_PALETTE_RAW = {
  concreteA: 0x9a9a94,
  concreteB: 0x93938c,
  concreteSeam: 0x7f7f78,
  crack: 0x6d6d66,
  oilStain: 0x5b564e,
  bolt: 0xb4b4ac,
  brick: 0xa8563c,
  brickDark: 0x8c4530,
  brickHi: 0xc0684a,
  mortar: 0xd6c9b6,
  glass: 0xa9d8e8,
  glassHi: 0xe6f6fb,
  frame: 0x4a5058,
  pipe: 0x8d949c,
  pipeHi: 0xc2c8ce,
  pipeJoint: 0x5f666e,
  hazardYellow: 0xf2c437,
  hazardDark: 0x33302a,
  hazardEdge: 0xd8a520,
  steel: 0x9aa1a9,
  steelHi: 0xc7cdd3,
  steelDark: 0x5c636b,
  benchTop: 0xb6bcc2,
  benchEdge: 0x7e858d,
  benchLeg: 0x525860,
  termLid: 0x4c5a6e,
  termBody: 0x36414f,
  crate: 0xc08b4e,
  crateHi: 0xdcae72,
  crateStrap: 0x8a5f30,
  drum: 0x3f7fb0,
  drumHi: 0x63a4d2,
  drumDark: 0x2c5c85,
  potClay: 0xc2663f,
  potClayHi: 0xdb8258,
  soil: 0x5a4030,
  leaf: 0x3f9e5e,
  leafHi: 0x5cbf78,
  pegboard: 0xb9884e,
  pegHole: 0x8a5f30,
  fireExt: 0xd2402f,
  shadow: 0x71716a,
  console: 0x646b73,
  consoleFace: 0x3d434a,
  screen: 0x1c3a4a,
  screenGlow: 0x53d2c8,
  buttonRed: 0xe0503f,
  buttonGreen: 0x5fd07a,
  beacon: 0xf2803a,
};

// 작업장 잔무늬 죽이기 — 슬래브 체커·이음매·균열·기름때·볼트를 콘크리트
// 바탕으로, 벽돌 격자를 벽돌 바탕으로 당긴다. 위험 스트라이프·드럼통·비상등
// (공장을 공장이게 하는 색)은 그대로.
const FACTORY_QUIET: readonly QuietGroup<typeof FACTORY_PALETTE_RAW>[] = [
  {
    base: "concreteA",
    keys: ["concreteB", "concreteSeam", "crack", "oilStain", "bolt"],
    amount: 0.58,
  },
  { base: "brick", keys: ["brickDark", "brickHi", "mortar"], amount: 0.45 },
  // 노랑-검정 빗금은 공장의 특징이라 없애지 않되, 라운지 두 줄을 통째로 덮는
  // 최대 대비 무늬라 그대로 두면 그 위의 캐릭터가 빗금에 썰려 보인다.
  { base: "hazardYellow", keys: ["hazardDark", "hazardEdge"], amount: 0.5 },
];

type FactoryPalette = typeof FACTORY_PALETTE_RAW;

/** 결정적 흩뿌리기(균열/기름 얼룩/볼트 자국/벽 설비). 베이크된 정적 텍스처라
 * 난수를 쓰면 재베이크마다 무늬가 바뀐다 — 해변·계곡과 같은 이유. */
const scatter = (tx: number, ty: number, mod: number): number => (tx * 61 + ty * 113) % mod;

/** 셔터 문이 난 하단 벽의 칸 범위(양끝 포함) — 벽 드로잉이 이 안에서만 레일을 세운다. */
const SHUTTER_X0 = 8;
const SHUTTER_X1 = 11;

/**
 * 45° 안전 빗금 한 벌. 줄 간격 8px가 타일 폭 16px를 정확히 나누므로 칸마다
 * 위상을 계산하지 않아도 이웃 칸과 저절로 이어진다. 2px 계단으로 끊어
 * 픽셀아트다운 굵기를 남긴다(1px씩 그리면 rect 수만 두 배가 된다).
 */
function hazardStripes(g: Graphics, s: number, color: number): void {
  for (let y = 0; y < s; y += 2) {
    for (let x0 = (y % 8) - 8; x0 < s; x0 += 8) {
      const x = Math.max(0, x0);
      const w = Math.min(x0 + 4, s) - x;
      if (w > 0) g.rect(x, y, w, 2).fill(color);
    }
  }
}

/** 4px 단 벽돌 쌓기(줄눈 1px, 단마다 반 칸 어긋남) — 모든 Wall 변종의 바탕. */
function brickBase(g: Graphics, s: number, pal: FactoryPalette, ty: number): void {
  g.rect(0, 0, s, s).fill(pal.brick);
  for (let y = 0; y < s; y += 4) {
    g.rect(0, y, s, 1).fill(pal.brickHi); // 벽돌 윗면 빛
    g.rect(0, y + 3, s, 1).fill(pal.mortar); // 가로 줄눈
    // 세로 줄눈: 단(course)마다 8px 어긋나게 — ty를 섞어 칸 경계에서도 이어진다.
    const off = ((ty * 4 + y / 4) % 2) * 4;
    g.rect(off, y, 1, 3).fill(pal.mortar);
    g.rect(off + 8, y, 1, 3).fill(pal.mortar);
  }
}

function factoryTileDraw(pal: FactoryPalette): TileDrawFn {
  return (g, { t, tx, ty, s, map }) => {
    const concrete = (tx + ty) % 2 === 0 ? pal.concreteA : pal.concreteB;
    switch (t) {
      case Tile.Floor: {
        g.rect(0, 0, s, s).fill(concrete);
        // 콘크리트 슬래브 이음매 — 칸 경계와 맞아떨어져 격자 바닥으로 읽힌다.
        g.rect(0, s - 1, s, 1).fill(pal.concreteSeam);
        g.rect(s - 1, 0, 1, s).fill(pal.concreteSeam);
        // 장식은 드물게 — 촘촘하면 작업장이 아니라 폐허로 보인다.
        const k = scatter(tx, ty, 19);
        if (k === 0) {
          // 균열: 1px 계단
          g.rect(3, 4, 3, 1).fill(pal.crack);
          g.rect(6, 5, 2, 1).fill(pal.crack);
          g.rect(8, 6, 4, 1).fill(pal.crack);
          g.rect(11, 7, 2, 1).fill(pal.crack);
        } else if (k === 4) {
          // 기름 얼룩: 가장자리가 번진 검은 웅덩이
          g.rect(5, 8, 6, 3).fill(pal.oilStain);
          g.rect(4, 9, 8, 1).fill(pal.oilStain);
          g.rect(6, 7, 3, 1).fill(pal.oilStain);
          g.rect(9, 11, 2, 1).fill(pal.oilStain);
        } else if (k === 7) {
          // 앵커 볼트 자국 두 개(설비를 뜯어낸 자리)
          g.rect(4, 5, 2, 2).fill(pal.bolt);
          g.rect(4, 6, 2, 1).fill(pal.crack);
          g.rect(10, 10, 2, 2).fill(pal.bolt);
          g.rect(10, 11, 2, 1).fill(pal.crack);
        }
        break;
      }
      case Tile.Wall: {
        if (ty === 0) {
          // 벽 상단 채광창(하이 윈도): 벽돌에 낸 창틀 + 유리 2쪽
          brickBase(g, s, pal, ty);
          g.rect(1, 2, 14, 9).fill(pal.frame);
          g.rect(2, 3, 12, 7).fill(pal.glass);
          g.rect(7, 3, 2, 7).fill(pal.frame); // 중간 문설주
          g.rect(2, 3, 5, 2).fill(pal.glassHi); // 들이치는 빛
          g.rect(9, 3, 3, 1).fill(pal.glassHi);
          if (scatter(tx, ty, 5) === 0) g.rect(3, 7, 3, 1).fill(pal.frame); // 금 간 유리
          break;
        }
        if (ty === 1) {
          // 배관·덕트: 굵은 관 + 가는 관, 칸마다 플랜지/밸브를 번갈아
          brickBase(g, s, pal, ty);
          g.rect(0, 2, s, 5).fill(pal.pipe);
          g.rect(0, 2, s, 1).fill(pal.pipeHi);
          g.rect(0, 6, s, 1).fill(pal.pipeJoint);
          g.rect(0, 11, s, 3).fill(pal.pipe);
          g.rect(0, 11, s, 1).fill(pal.pipeHi);
          if (tx % 3 === 0) {
            g.rect(5, 1, 3, 7).fill(pal.pipeJoint); // 플랜지 이음
            g.rect(5, 1, 3, 1).fill(pal.pipeHi);
          } else if (tx % 3 === 1) {
            g.rect(10, 7, 2, 4).fill(pal.pipeJoint); // 굵은 관 → 가는 관 분기
            g.rect(9, 8, 4, 1).fill(pal.hazardYellow); // 밸브 핸들
          }
          break;
        }
        if (ty === map.height - 1) {
          // 하단 벽: 벽돌 + 바닥 접선 안전 띠, 가운데엔 셔터 문
          brickBase(g, s, pal, ty);
          const inShutter = tx >= SHUTTER_X0 && tx <= SHUTTER_X1;
          if (inShutter) {
            g.rect(0, 2, s, s - 2).fill(pal.steel);
            // 셔터 슬랫 — 4px 주기라 칸을 넘어 이어진다.
            for (let y = 3; y < s; y += 4) g.rect(0, y, s, 1).fill(pal.steelDark);
            g.rect(0, 2, s, 1).fill(pal.steelHi); // 상인방
            if (tx === SHUTTER_X0) g.rect(0, 2, 2, s - 2).fill(pal.frame); // 좌측 레일
            if (tx === SHUTTER_X1) g.rect(s - 2, 2, 2, s - 2).fill(pal.frame); // 우측 레일
          }
          // 안전 띠(노랑/검정 4px 교대) — 8px 주기라 이웃 칸과 이어진다.
          for (let x = 0; x < s; x += 8) {
            g.rect(x, 0, 4, 2).fill(pal.hazardYellow);
            g.rect(x + 4, 0, 4, 2).fill(pal.hazardDark);
          }
          break;
        }
        // 측면 벽돌 벽 + 드문 설비(소화기 / 배전함)
        brickBase(g, s, pal, ty);
        const k = scatter(tx, ty, 4);
        if (k === 0) {
          g.rect(6, 4, 4, 8).fill(pal.fireExt); // 소화기
          g.rect(7, 2, 2, 2).fill(pal.steelDark); // 손잡이
          g.rect(6, 6, 4, 1).fill(pal.steelHi); // 라벨
          g.rect(6, 12, 4, 1).fill(pal.shadow);
        } else if (k === 2) {
          g.rect(4, 3, 8, 9).fill(pal.frame); // 배전함
          g.rect(5, 4, 6, 7).fill(pal.steelDark);
          g.rect(5, 4, 6, 2).fill(pal.hazardYellow); // 감전 주의 라벨
          g.rect(10, 7, 1, 2).fill(pal.steelHi); // 걸쇠
        }
        break;
      }
      case Tile.Rug: {
        // 노란 빗금 안전구역: 바탕 노랑 + 검정 사선 + 구역 경계 1px
        g.rect(0, 0, s, s).fill(pal.hazardYellow);
        hazardStripes(g, s, pal.hazardDark);
        g.rect(0, 0, s, 1).fill(pal.hazardEdge);
        g.rect(0, s - 1, s, 1).fill(pal.hazardEdge);
        break;
      }
      case Tile.DeskTop: {
        const isLeft = map.tiles[ty][tx - 1] !== Tile.DeskTop;
        g.rect(0, 0, s, s).fill(concrete); // 가구 타일은 바닥 베이크에서 빠지므로 스스로 콘크리트를 깐다
        // 스테인리스 작업 벤치 — 상판을 통짜로 칠해 2칸짜리 쌍이 하나의 긴
        // 벤치로 읽히게 한다(칸마다 테두리를 두르면 사물함처럼 보인다).
        g.rect(0, 6, s, 5).fill(pal.benchTop);
        g.rect(0, 6, s, 1).fill(pal.steelHi);
        g.rect(0, 11, s, 2).fill(pal.benchEdge); // 앞치마
        g.rect(0, 13, s, 1).fill(pal.steelDark);
        // 벤치 아래 롤러 컨베이어 — 4px 주기 이음매라 칸을 넘어 이어진다.
        g.rect(0, 14, s, 2).fill(pal.steel);
        for (let x = 0; x < s; x += 4) g.rect(x, 14, 1, 2).fill(pal.steelDark);
        if (isLeft) {
          // 산업용 단말기(뒷모습) — 오피스 랩탑과 같은 이디엄(등판이 뷰어를 향한다)
          g.rect(4, 5, 8, 1).fill(pal.termBody);
          g.rect(4, 1, 8, 4).fill(pal.termLid);
          g.rect(4, 4, 8, 1).fill(pal.termBody);
          g.rect(7, 2, 2, 2).fill(pal.termBody);
        } else {
          // 짝 칸에는 부품 상자 + 렌치
          g.rect(2, 2, 6, 4).fill(pal.crate);
          g.rect(2, 2, 6, 1).fill(pal.crateHi);
          g.rect(4, 2, 2, 4).fill(pal.crateStrap); // 결속 밴드
          g.rect(9, 4, 5, 1).fill(pal.steelHi); // 렌치 자루
          g.rect(13, 3, 2, 3).fill(pal.steelHi); // 렌치 머리
          g.rect(14, 4, 1, 1).fill(pal.benchEdge); // 물림턱
        }
        break;
      }
      case Tile.Plant: {
        // 기름 드럼통 옆에 관리자가 들여놓은 화분 — 공장에서도 식물은 산다.
        g.rect(0, 0, s, s).fill(concrete);
        g.rect(1, 4, 7, 11).fill(pal.drum);
        g.rect(2, 4, 2, 11).fill(pal.drumHi); // 세로 하이라이트
        g.rect(1, 4, 7, 1).fill(pal.drumHi); // 뚜껑 림
        g.rect(1, 7, 7, 2).fill(pal.drumDark); // 보강 띠 두 줄
        g.rect(1, 12, 7, 2).fill(pal.drumDark);
        g.rect(1, 15, 7, 1).fill(pal.shadow);
        // 열 기준으로 번갈아 — 이 맵의 화분 자리(tx1/tx18)가 서로 다르게 보이도록.
        if (tx % 2 === 0) {
          g.rect(3, 9, 3, 3).fill(pal.hazardYellow); // 경고 라벨
        } else {
          g.rect(4, 4, 2, 2).fill(pal.steelDark); // 주입구 마개
        }
        // 화분 + 잎
        g.rect(9, 10, 6, 5).fill(pal.potClay);
        g.rect(9, 10, 6, 1).fill(pal.potClayHi);
        g.rect(10, 9, 4, 1).fill(pal.soil);
        g.rect(11, 5, 2, 5).fill(pal.leaf); // 줄기
        g.rect(8, 3, 4, 2).fill(pal.leaf);
        g.rect(12, 2, 4, 2).fill(pal.leafHi);
        g.rect(9, 6, 3, 1).fill(pal.leafHi);
        g.rect(13, 5, 2, 1).fill(pal.leaf);
        g.rect(9, 15, 6, 1).fill(pal.shadow);
        break;
      }
      case Tile.Counter: {
        // 공구 대여 카운터: 뒤편 유공보드에 공구를 걸고, 앞면엔 안전 띠.
        g.rect(0, 0, s, s).fill(concrete);
        g.rect(0, 0, s, 7).fill(pal.pegboard);
        g.rect(0, 0, s, 1).fill(pal.steelDark); // 보드 상단 프레임
        for (let y = 2; y < 7; y += 3) {
          for (let x = 1; x < s; x += 4) g.rect(x, y, 1, 1).fill(pal.pegHole);
        }
        if (tx % 2 === 0) {
          g.rect(3, 1, 1, 5).fill(pal.steelHi); // 드라이버 두 자루
          g.rect(2, 5, 3, 1).fill(pal.crateStrap);
          g.rect(7, 1, 1, 4).fill(pal.steelHi);
          g.rect(6, 4, 3, 1).fill(pal.crateStrap);
        } else {
          g.rect(3, 2, 5, 3).fill(pal.hazardYellow); // 안전모
          g.rect(3, 4, 5, 1).fill(pal.hazardEdge);
          g.rect(10, 2, 4, 4).fill(pal.steelHi); // 장갑 상자
          g.rect(10, 2, 4, 1).fill(pal.steel);
        }
        g.rect(0, 7, s, 3).fill(pal.benchTop); // 카운터 상판
        g.rect(0, 7, s, 1).fill(pal.steelHi);
        g.rect(0, 10, s, 6).fill(pal.benchEdge); // 앞면
        for (let x = 0; x < s; x += 8) {
          g.rect(x, 12, 4, 2).fill(pal.hazardYellow);
          g.rect(x + 4, 12, 4, 2).fill(pal.hazardDark);
        }
        g.rect(0, s - 1, s, 1).fill(pal.shadow);
        break;
      }
      case Tile.Table: {
        // 검수 테이블: 낮은 강철 상판 + 완성 부품 상자
        g.rect(0, 0, s, s).fill(concrete);
        g.rect(3, 1, 5, 4).fill(pal.crate); // 부품 상자
        g.rect(3, 1, 5, 1).fill(pal.crateHi);
        g.rect(5, 1, 1, 4).fill(pal.crateStrap);
        g.rect(9, 3, 5, 2).fill(pal.steelHi); // 검수 대기 부품 더미
        g.rect(10, 2, 3, 1).fill(pal.steel);
        g.rect(1, 5, 14, 3).fill(pal.benchTop);
        g.rect(1, 5, 14, 1).fill(pal.steelHi);
        g.rect(1, 8, 14, 2).fill(pal.benchEdge);
        g.rect(2, 10, 2, 4).fill(pal.benchLeg);
        g.rect(12, 10, 2, 4).fill(pal.benchLeg);
        g.rect(1, 14, 3, 1).fill(pal.shadow);
        g.rect(12, 14, 3, 1).fill(pal.shadow);
        break;
      }
      case Tile.BossDesk: {
        // 관리실 제어반(세로 1×2): 위 칸이 모니터 뱅크·경광등, 아래 칸이
        // 레버·버튼이 달린 경사 콘솔.
        const isLower = map.tiles[ty - 1]?.[tx] === Tile.BossDesk;
        g.rect(0, 0, s, s).fill(concrete);
        if (!isLower) {
          g.rect(7, 2, 2, 2).fill(pal.steelDark); // 경광등 기둥
          g.rect(5, 0, 6, 2).fill(pal.beacon); // 경광등
          g.rect(6, 0, 4, 1).fill(pal.hazardYellow);
          g.rect(1, 4, 14, 2).fill(pal.steelDark); // 상단 차양
          g.rect(2, 6, 12, 8).fill(pal.console); // 캐비닛
          g.rect(3, 7, 4, 5).fill(pal.screen); // 모니터 3면
          g.rect(8, 7, 5, 5).fill(pal.screen);
          g.rect(3, 7, 4, 1).fill(pal.consoleFace);
          g.rect(8, 7, 5, 1).fill(pal.consoleFace);
          g.rect(4, 9, 2, 1).fill(pal.screenGlow); // 그래프 한 줄기
          g.rect(9, 8, 3, 1).fill(pal.screenGlow);
          g.rect(9, 10, 2, 1).fill(pal.screenGlow);
          g.rect(2, 14, 12, 2).fill(pal.steelDark); // 하단 몰딩
        } else {
          g.rect(2, 0, 12, 9).fill(pal.console);
          g.rect(3, 1, 10, 4).fill(pal.consoleFace); // 경사 조작 패널
          g.rect(4, 2, 2, 2).fill(pal.buttonRed);
          g.rect(7, 2, 2, 2).fill(pal.buttonGreen);
          g.rect(10, 2, 2, 2).fill(pal.hazardYellow);
          g.rect(5, 5, 1, 4).fill(pal.steelHi); // 레버
          g.rect(4, 5, 3, 1).fill(pal.buttonRed); // 레버 손잡이
          g.rect(9, 5, 4, 3).fill(pal.screen); // 계기판
          g.rect(10, 6, 1, 1).fill(pal.screenGlow);
          g.rect(2, 9, 12, 2).fill(pal.steelDark); // 받침대
          g.rect(3, 11, 2, 4).fill(pal.benchLeg); // 다리
          g.rect(11, 11, 2, 4).fill(pal.benchLeg);
          g.rect(2, 15, 12, 1).fill(pal.shadow); // 바닥 그림자
        }
        break;
      }
    }
  };
}

export const FACTORY_SCENE: SceneDef = defineScene({
  id: "factory",
  labelKey: "office:scene.factory",
  map: FACTORY_MAP,
  raw: FACTORY_PALETTE_RAW,
  quiet: FACTORY_QUIET,
  /** 레터박스(맵 밖) 배경 — 바닥보다 어두운 공장 그늘색. */
  background: 0x4a4a46,
  draw: factoryTileDraw,
});
