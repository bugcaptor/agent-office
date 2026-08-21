// src/renderer/office/scenes/zombieScene.ts
//
// 좀비 아포칼립스 생존자 기지 풍경. 해변/계곡과 같은 규칙(의미 타일은 오피스와
// 공유하고 어휘만 교체):
//   Floor=금 간 아스팔트·흙바닥(잡초 틈새), Wall=바리케이드(판자 X자 못질·
//   철조망·폐타이어)/맨 윗 행은 흐린 하늘과 폐건물 실루엣, Rug=낡은 캠핑 매트,
//   DeskTop=판자 작업대(무전기·랜턴), Plant=말라죽은 나무(까마귀),
//   Counter=보급소 카운터(통조림·물통), Table=드럼통 모닥불, BossDesk=감시탑.
//
// 무섭게가 아니라 아기자기하게 — 핏자국·시체 같은 잔혹 묘사는 쓰지 않고,
// "사람이 붙어살며 고쳐 쓴 흔적"(덧댄 널판, 못, 헝겊 패치, 주워 온 통조림)으로
// 폐허감을 낸다.
//
// 레이아웃도 해변/계곡과 일부러 다르게 잡았다 — 감시탑(보스 자리)이 정문 쪽인
// 위쪽 좌측에 서고 배급 줄이 그 아래 행을 따라 동쪽으로 뻗으며, 작업대 두 줄은
// 기지 한가운데, 매트 라운지와 드럼통 모닥불은 아래쪽, 보급소는 우측 하단이다.
import type { TileRect } from "../map/mapData";
import { L, Tile, buildSceneMap } from "../map/mapData";
import { adaptColor, adaptPalette, sceneColorMode } from "./sceneColor";
import type { SceneDef, TileDrawFn } from "./sceneTypes";

const GRID: Tile[][] = [
  L('WWWWWWWWWWWWWWWWWWWW'), // ty0  - 흐린 하늘 + 먼 폐건물 실루엣
  L('WWWWWWWWWWWWWWWWWWWW'), // ty1  - 철조망을 두른 바리케이드(정문 담장)
  L('WFFBFFFFFFFFFFFFFFPW'), // ty2  - 감시탑 망루(tx3) + 말라죽은 나무(tx18)
  L('WFFBFFFFFFFFFFFFFFFW'), // ty3  - 감시탑 다리 + 배급 줄 레인(동쪽으로)
  L('WFFFFFFFFFFFFFFFFFFW'), // ty4  - 좌석 행 1
  L('WFDDFFDDFFDDFFDDFFFW'), // ty5  - 판자 작업대 4쌍
  L('WFFFFFFFFFFFFFFFFFFW'), // ty6  - 좌석 행 2
  L('WPDDFFDDFFDDFFDDFFFW'), // ty7  - 말라죽은 나무(tx1) + 판자 작업대 4쌍
  L('WFFFFFFFFFFFFFFFFFFW'), // ty8
  L('WFFFFFFFFFFFFFCCCCCW'), // ty9  - 보급소 카운터(우측)
  L('WFRRRRRRRRRRFFFFFFFW'), // ty10 - 낡은 캠핑 매트 라운지
  L('WFRRRRTTRRRRFFFFFFFW'), // ty11 - 매트 + 드럼통 모닥불(2칸)
  L('WFFFFFFFFFFFFFFFFFFW'), // ty12
  L('WWWWWWWWWWWWWWWWWWWW'), // ty13 - 바리케이드(뒤쪽 담장)
];

/** 모닥불 옆에 깔개를 펴 둔 쉼터 — 오피스 러그 라운지와 같은 역할. */
const BREAK_ROOM: TileRect = { x: 8, y: 10, w: 4, h: 2 };

export const ZOMBIE_MAP = buildSceneMap(GRID, BREAK_ROOM);

const ZOMBIE_PALETTE = {
  asphaltA: 0x5c584f,
  asphaltB: 0x545045,
  crack: 0x3b3830,
  weed: 0x6c8b46,
  weedHi: 0x90b05e,
  leaf: 0xbe8b45,
  leafDim: 0x8f6a35,
  sky: 0x8d95a2,
  skyHaze: 0xa9b0ba,
  ruinDark: 0x3f444e,
  ruinMid: 0x565c68,
  ruinWindow: 0x2c303a,
  plankA: 0x8f6a41,
  plankB: 0x74522f,
  plankTop: 0xb08857,
  plankSeam: 0x5a3f24,
  plankNail: 0xd6d0c1,
  wire: 0xb4aea0,
  tire: 0x34332f,
  tireHi: 0x4c4a45,
  mat: 0x7b6b53,
  matStripe: 0xb0a084,
  matEdge: 0x5b4d3a,
  matPatch: 0x8e7f96,
  benchTop: 0xc19660,
  benchWood: 0x9a7040,
  benchSeam: 0x6d4c2a,
  benchShade: 0x503722,
  radioBody: 0x4a5348,
  radioGrill: 0x2e352e,
  radioKnob: 0xe0a83f,
  radioAntenna: 0xa8a496,
  lanternMetal: 0x59605f,
  lanternGlass: 0xcfd6cf,
  lanternGlow: 0xf3cc63,
  deadTrunk: 0x6b5942,
  deadBranch: 0x8a7457,
  crow: 0x24262c,
  crowEye: 0xe8d8a6,
  crowBeak: 0xc9a24a,
  crateWood: 0x9d7746,
  crateTop: 0xc39a63,
  crateSeam: 0x6b4b29,
  crateShade: 0x422e1c,
  can: 0xb9b3a3,
  canLid: 0xd8d3c4,
  canLabel: 0xc4593c,
  jug: 0x5aa2c0,
  jugCap: 0xe6e2d4,
  jugLabel: 0xdfe7ea,
  drum: 0x9c5537,
  drumRim: 0xc07a4d,
  drumRust: 0x6f3a24,
  flame: 0xef8b3a,
  flameCore: 0xf8d45f,
  ember: 0xd45230,
  towerRoof: 0x6d7076,
  towerRoofHi: 0x9a9da3,
  towerBeam: 0x7a5733,
  towerWood: 0xa87b47,
  towerDark: 0x2b2419,
};

type ZombiePalette = typeof ZOMBIE_PALETTE;

/** 레터박스(맵 밖) 배경 — 아스팔트보다 한 단계 어두운 잿빛 그늘. */
const ZOMBIE_BACKGROUND = 0x3c3a34;

/** 결정적 흩뿌리기(균열/잡초/낙엽/폐타이어 배치). 베이크된 정적 텍스처라
 * 난수·시각(Date.now)을 쓰면 재베이크마다 무늬가 바뀐다. */
const scatter = (tx: number, ty: number, mod: number): number => (tx * 59 + ty * 113) % mod;

function zombieTileDraw(pal: ZombiePalette): TileDrawFn {
  return (g, { t, tx, ty, s, map }) => {
    const ground = (tx + ty) % 2 === 0 ? pal.asphaltA : pal.asphaltB;
    switch (t) {
      case Tile.Floor: {
        g.rect(0, 0, s, s).fill(ground);
        // 금 간 자국 — 칸마다 어긋나게 밀어 격자로 읽히지 않게 한다.
        const c = scatter(tx, ty, 4);
        g.rect(3 + c, 5, 6, 1).fill(pal.crack);
        g.rect(8 + c, 6, 1, 3).fill(pal.crack);
        g.rect(s - 5, s - 4, 3, 1).fill(pal.crack);
        // 장식은 해변·계곡과 같은 이유로 드물게 — 촘촘하면 폐허가 아니라 쓰레기장이 된다.
        const k = scatter(tx, ty, 15);
        if (k === 0) {
          // 갈라진 틈에서 올라온 잡초
          g.rect(3, 13, 7, 1).fill(pal.crack);
          g.rect(4, 10, 1, 4).fill(pal.weed);
          g.rect(6, 8, 1, 6).fill(pal.weedHi);
          g.rect(8, 11, 1, 3).fill(pal.weed);
        } else if (k === 7) {
          // 바람에 몰린 낙엽 몇 장
          g.rect(10, 4, 3, 2).fill(pal.leaf);
          g.rect(11, 3, 1, 1).fill(pal.leaf);
          g.rect(4, 11, 2, 2).fill(pal.leafDim);
        }
        break;
      }
      case Tile.Wall: {
        if (ty === 0) {
          // 흐린 하늘 + 무너진 고층 실루엣 두 채. 칸마다 높이를 흔들어
          // 스카이라인이 한 줄로 평평해지지 않게 한다.
          g.rect(0, 0, s, s).fill(pal.sky);
          g.rect(0, 0, s, 4).fill(pal.skyHaze);
          const h = scatter(tx, ty, 5);
          g.rect(1, 5 + h, 6, s - 5 - h).fill(pal.ruinMid);
          g.rect(1, 5 + h, 6, 1).fill(pal.ruinDark); // 부서진 옥상 라인
          g.rect(2, 7 + h, 2, 2).fill(pal.ruinWindow);
          g.rect(4, 10 + h, 2, 2).fill(pal.ruinWindow);
          const h2 = scatter(tx, ty + 1, 3);
          g.rect(9, 8 - h2, 6, s - 8 + h2).fill(pal.ruinDark);
          g.rect(9, 8 - h2, 3, 1).fill(pal.ruinMid);
          g.rect(10, 10 - h2, 2, 2).fill(pal.ruinWindow);
          g.rect(13, 12 - h2, 1, 2).fill(pal.ruinWindow);
          break;
        }
        // 바리케이드. 맨 윗 행(정문 담장)만 판자를 한 단 내려 세우고 그 위로
        // 철조망을 두른다 — 담장 너머 하늘이 보여야 기지 안이라는 게 읽힌다.
        const top = ty === 1 ? 5 : 0;
        if (top > 0) {
          g.rect(0, 0, s, s).fill(pal.sky);
          g.rect(0, 0, s, 3).fill(pal.skyHaze);
          g.rect(0, 1, s, 1).fill(pal.wire); // 철조망 두 줄
          g.rect(0, 4, s, 1).fill(pal.wire);
          g.rect(3, 0, 1, 5).fill(pal.wire); // 가시(꼬아 묶은 매듭)
          g.rect(11, 0, 1, 5).fill(pal.wire);
        }
        // 주워 온 널판을 세로로 세워 박은 담장
        g.rect(0, top, s, s - top).fill(pal.plankA);
        g.rect(0, top, s, 2).fill(pal.plankTop); // 위쪽 단면(빛 받는 면)
        g.rect(4, top + 2, 1, s - top - 2).fill(pal.plankSeam);
        g.rect(9, top + 2, 1, s - top - 2).fill(pal.plankSeam);
        g.rect(14, top + 2, 1, s - top - 2).fill(pal.plankSeam);
        // X자로 덧댄 보강 널 — 대각선을 4단 계단으로 흉내낸다.
        for (let i = 0; i < 4; i++) {
          g.rect(1 + i * 4, top + 2 + i * 2, 3, 2).fill(pal.plankB);
          g.rect(12 - i * 4, top + 2 + i * 2, 3, 2).fill(pal.plankB);
        }
        g.rect(0, s - 4, s, 2).fill(pal.plankB); // 아래쪽 가로 띠장
        g.rect(2, top + 3, 1, 1).fill(pal.plankNail); // 못 두 방
        g.rect(13, top + 3, 1, 1).fill(pal.plankNail);
        // 담장에 기대 쌓아 둔 폐타이어 — 드물게(측면·뒤쪽 담장에만).
        if (top === 0 && scatter(tx, ty, 4) === 0) {
          g.rect(3, 9, 10, 6).fill(pal.tire);
          g.rect(3, 9, 10, 1).fill(pal.tireHi);
          g.rect(6, 11, 4, 3).fill(pal.plankB); // 타이어 가운데 구멍
        }
        break;
      }
      case Tile.Rug: {
        // 낡은 캠핑 매트/천막 깔개: 바랜 줄무늬 + 해진 가장자리 + 덧댄 헝겊
        g.rect(0, 0, s, s).fill(pal.mat);
        g.rect(0, 3, s, 2).fill(pal.matStripe);
        g.rect(0, 9, s, 2).fill(pal.matStripe);
        g.rect(0, 0, s, 1).fill(pal.matEdge);
        g.rect(0, s - 1, s, 1).fill(pal.matEdge);
        if (scatter(tx, ty, 5) === 0) {
          g.rect(5, 6, 5, 4).fill(pal.matPatch);
          g.rect(5, 6, 5, 1).fill(pal.matEdge); // 기운 실밥
        }
        break;
      }
      case Tile.DeskTop: {
        const isLeft = map.tiles[ty][tx - 1] !== Tile.DeskTop;
        g.rect(0, 0, s, s).fill(ground); // 가구 타일은 바닥 베이크에서 빠지므로 스스로 노면을 깐다
        // 널판을 얹은 작업대 — 주워 온 자재라 상판과 다리 색이 제각각이다.
        g.rect(0, 7, s, 4).fill(pal.benchTop);
        g.rect(0, 9, s, 1).fill(pal.benchSeam); // 널판 이음매
        g.rect(0, 11, s, 2).fill(pal.benchWood);
        g.rect(0, 13, s, 1).fill(pal.benchShade);
        g.rect(2, 13, 2, 3).fill(pal.benchWood); // 각목 다리
        g.rect(12, 13, 2, 3).fill(pal.benchWood);
        if (isLeft) {
          // 무전기: 안테나 + 스피커 그릴 + 다이얼(오피스 랩탑 자리)
          g.rect(11, 0, 1, 5).fill(pal.radioAntenna);
          g.rect(4, 2, 8, 5).fill(pal.radioBody);
          g.rect(5, 3, 4, 3).fill(pal.radioGrill);
          g.rect(10, 4, 1, 1).fill(pal.radioKnob);
          g.rect(4, 6, 8, 1).fill(pal.benchShade); // 상판에 닿는 그림자
        } else {
          // 짝 칸에는 랜턴 하나 — 밤샘 근무의 유일한 불빛.
          g.rect(6, 0, 1, 2).fill(pal.lanternMetal); // 손잡이
          g.rect(9, 0, 1, 2).fill(pal.lanternMetal);
          g.rect(6, 1, 4, 1).fill(pal.lanternMetal);
          g.rect(5, 2, 6, 2).fill(pal.lanternMetal);
          g.rect(5, 4, 6, 3).fill(pal.lanternGlass);
          g.rect(7, 4, 2, 3).fill(pal.lanternGlow); // 심지 불빛
          g.rect(5, 7, 6, 1).fill(pal.lanternMetal);
        }
        break;
      }
      case Tile.Plant: {
        // 말라죽은 나무: 잎 하나 없이 가지만 앙상하게.
        g.rect(0, 0, s, s).fill(ground);
        g.rect(7, 2, 2, 14).fill(pal.deadTrunk);
        g.rect(6, 12, 1, 4).fill(pal.deadTrunk); // 갈라진 밑동
        g.rect(9, 13, 2, 3).fill(pal.deadTrunk);
        g.rect(3, 5, 4, 1).fill(pal.deadBranch);
        g.rect(2, 3, 1, 3).fill(pal.deadBranch);
        g.rect(9, 4, 5, 1).fill(pal.deadBranch);
        g.rect(13, 1, 1, 4).fill(pal.deadBranch);
        g.rect(5, 8, 3, 1).fill(pal.deadBranch);
        g.rect(9, 9, 3, 1).fill(pal.deadBranch);
        // 열 기준 번갈이 — 이 맵의 Plant 두 자리(tx1/tx18)가 서로 다른 모습이
        // 되도록(합 기준이면 둘 다 같은 쪽으로 떨어진다).
        if (tx % 2 === 0) {
          // 가지 끝에 앉은 까마귀 한 마리 — 이 씬의 유일한 살아 있는 것.
          g.rect(9, 2, 2, 1).fill(pal.crow); // 꼬리
          g.rect(10, 1, 4, 3).fill(pal.crow); // 몸통
          g.rect(13, 0, 2, 2).fill(pal.crow); // 머리
          g.rect(14, 1, 1, 1).fill(pal.crowEye);
          g.rect(15, 1, 1, 1).fill(pal.crowBeak);
        }
        break;
      }
      case Tile.Counter: {
        // 보급소 카운터: 궤짝을 쌓아 만든 배급대 + 통조림/물통을 번갈아.
        g.rect(0, 0, s, s).fill(ground);
        g.rect(0, 6, s, 8).fill(pal.crateWood);
        g.rect(0, 6, s, 2).fill(pal.crateTop); // 상판
        g.rect(0, 10, s, 1).fill(pal.crateSeam); // 궤짝 단 사이
        g.rect(3, 11, 1, 3).fill(pal.crateSeam);
        g.rect(11, 11, 1, 3).fill(pal.crateSeam);
        g.rect(0, 14, s, 2).fill(pal.crateShade);
        if (tx % 2 === 0) {
          // 통조림 두 무더기
          g.rect(3, 2, 4, 4).fill(pal.can);
          g.rect(3, 2, 4, 1).fill(pal.canLid);
          g.rect(3, 4, 4, 1).fill(pal.canLabel);
          g.rect(8, 3, 4, 3).fill(pal.can);
          g.rect(8, 3, 4, 1).fill(pal.canLid);
        } else {
          // 배급용 물통
          g.rect(4, 1, 7, 5).fill(pal.jug);
          g.rect(4, 1, 7, 1).fill(pal.jugCap);
          g.rect(5, 3, 5, 2).fill(pal.jugLabel);
          g.rect(11, 2, 1, 2).fill(pal.jugCap); // 손잡이
        }
        break;
      }
      case Tile.Table: {
        // 드럼통 모닥불 — 둘러앉아 몸을 녹이는 곳(오피스 탕비실 테이블 자리).
        g.rect(0, 0, s, s).fill(ground);
        g.rect(3, 6, 10, 9).fill(pal.drum);
        g.rect(3, 6, 10, 1).fill(pal.drumRim); // 통 아가리
        g.rect(3, 9, 10, 1).fill(pal.drumRust); // 녹 띠
        g.rect(3, 12, 10, 1).fill(pal.drumRust);
        g.rect(2, 15, 12, 1).fill(pal.crateShade); // 바닥 그림자
        g.rect(5, 2, 6, 5).fill(pal.flame);
        g.rect(4, 4, 2, 2).fill(pal.flame);
        g.rect(7, 0, 3, 4).fill(pal.flameCore);
        g.rect(6, 5, 4, 2).fill(pal.flameCore);
        g.rect(11, 4, 2, 2).fill(pal.ember); // 튀는 불티
        break;
      }
      case Tile.BossDesk: {
        // 감시탑(세로 1×2): 위 칸이 망루, 아래 칸이 다리·사다리.
        const isLower = map.tiles[ty - 1]?.[tx] === Tile.BossDesk;
        g.rect(0, 0, s, s).fill(ground);
        if (!isLower) {
          g.rect(2, 0, 12, 3).fill(pal.towerRoof); // 함석 지붕
          g.rect(2, 0, 12, 1).fill(pal.towerRoofHi);
          g.rect(1, 3, 14, 1).fill(pal.towerBeam); // 처마
          g.rect(2, 4, 12, 7).fill(pal.towerWood); // 망루 벽
          g.rect(3, 5, 10, 4).fill(pal.towerDark); // 사방이 트인 관측창
          g.rect(11, 6, 2, 2).fill(pal.lanternGlow); // 걸어 둔 랜턴
          g.rect(2, 11, 12, 2).fill(pal.towerBeam); // 난간
          g.rect(4, 11, 1, 2).fill(pal.towerWood); // 난간 살
          g.rect(8, 11, 1, 2).fill(pal.towerWood);
          g.rect(2, 13, 12, 3).fill(pal.towerDark); // 망루 밑 그늘
          g.rect(3, 13, 2, 3).fill(pal.towerBeam); // 아래 칸 기둥과 이어지는 자리
          g.rect(11, 13, 2, 3).fill(pal.towerBeam);
        } else {
          g.rect(3, 0, 2, 14).fill(pal.towerBeam); // 다리
          g.rect(11, 0, 2, 14).fill(pal.towerBeam);
          g.rect(3, 3, 10, 1).fill(pal.towerWood); // 가새(버팀대)
          g.rect(6, 0, 1, 14).fill(pal.towerWood); // 사다리 세로대
          g.rect(9, 0, 1, 14).fill(pal.towerWood);
          g.rect(5, 1, 6, 1).fill(pal.towerWood); // 사다리 발판
          g.rect(5, 5, 6, 1).fill(pal.towerWood);
          g.rect(5, 9, 6, 1).fill(pal.towerWood);
          g.rect(2, 14, 12, 2).fill(pal.crateShade); // 바닥 그림자
        }
        break;
      }
    }
  };
}

export const ZOMBIE_SCENE: SceneDef = {
  id: "zombie",
  label: "좀비 마을",
  map: ZOMBIE_MAP,
  resolve: (theme) => {
    const mode = sceneColorMode(theme.id);
    return {
      background: adaptColor(ZOMBIE_BACKGROUND, mode),
      drawTile: zombieTileDraw(adaptPalette(ZOMBIE_PALETTE, mode)),
    };
  },
};
