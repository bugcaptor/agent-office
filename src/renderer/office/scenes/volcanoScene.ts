// src/renderer/office/scenes/volcanoScene.ts
//
// 화산지대 대장간 캠프. 해변/계곡 씬과 같은 규칙(의미 타일 공유, 어휘만 교체):
//   Floor=현무암 지대, Wall=용암 호수(위 2줄)/화산 암벽(그 외),
//   Rug=흑요석 판(내열 매트), DeskTop=모루 작업대(화구·연장),
//   Plant=내화 식물(붉은 잎 관목/용암꽃), Counter=용암석 바,
//   Table=바위 테이블, BossDesk=용암 왕좌.
//
// 레이아웃은 해변·계곡과 일부러 다르게 잡았다 — 모루가 두 칸 간격 대신 한 칸
// 간격으로 촘촘히 늘어서고(작업장다운 밀도), 아래 행은 위 행과 한 칸 엇갈리게
// 배치해 격자가 아니라 캠프처럼 읽히게 했다. 왕좌는 좌우 끝이 아니라 한가운데
// (tx11)에 서고 줄서기 레인은 그 하단 행을 따라 서쪽으로 뻗는다(방향은
// `buildSceneMap`이 보스 위치에서 유도한다). 라운지는 좌측, 바는 우측 하단.
//
// 팔레트는 "낮의 원색" 한 벌만 두고, 어두운/모노크롬 테마에서는
// sceneColor.ts가 자동 변환한다.
import type { TileRect } from "../map/mapData";
import { L, Tile, buildSceneMap } from "../map/mapData";
import type { QuietGroup } from "./sceneColor";
import {
  SCENE_CHROMA_CUT,
  adaptColor,
  adaptPalette,
  desaturateColor,
  desaturatePalette,
  quietPalette,
  sceneColorMode,
} from "./sceneColor";
import type { SceneDef, TileDrawFn } from "./sceneTypes";

const GRID: Tile[][] = [
  L('WWWWWWWWWWWWWWWWWWWW'), // ty0  - 먼 용암 호수(건너편 암흑 능선)
  L('WWWWWWWWWWWWWWWWWWWW'), // ty1  - 끓는 물가 + 굳은 크러스트 가장자리
  L('WFFFFFFFFFFFFFFFFFFW'), // ty2  - 좌석 행 1
  L('WFDDFDDFDDFDDFFFFPFW'), // ty3  - 모루 작업대 4쌍 + 붉은 잎 관목(tx17)
  L('WFFFFFFFFFFFFFFFFFFW'), // ty4  - 통로
  L('WFFFFFFFFFFFFFFFFFFW'), // ty5  - 좌석 행 2
  L('WFFDDFDDFDDFDDFFFFFW'), // ty6  - 모루 작업대 4쌍(위 행과 한 칸 엇갈림)
  L('WFFFFFFFFFFFFFFFFFPW'), // ty7  - 통로 + 용암꽃(tx18)
  L('WPFFFFFFFFFBFFFFFFFW'), // ty8  - 관목(tx1) + 용암 왕좌 상단(tx11)
  L('WFFFFFFFFFFBFFFFFFFW'), // ty9  - 왕좌 하단 + 줄서기 레인(서쪽)
  L('WFRRRRRRRRRRFFFFFFFW'), // ty10 - 흑요석 판 라운지
  L('WFRRTTRRRRRRFFFFFFFW'), // ty11 - 라운지 + 바위 테이블(2칸)
  L('WFFFFFFFFFFFFCCCCCFW'), // ty12 - 용암석 바
  L('WWWWWWWWWWWWWWWWWWWW'), // ty13 - 화산 암벽
];

/** 흑요석 판을 깔아 둔 쉼터 — 오피스 러그 라운지와 같은 역할.
 * 바위 테이블(tx4/tx5)을 피해 오른쪽 절반을 잡는다(내부 전 타일이
 * 걸을 수 있어야 한다는 계약). */
const BREAK_ROOM: TileRect = { x: 6, y: 10, w: 5, h: 2 };

export const VOLCANO_MAP = buildSceneMap(GRID, BREAK_ROOM);

/** 용암 호수가 차지하는 행 수 — 이 위(포함)의 Wall은 호수로, 아래는 암벽으로 그린다. */
const LAVA_ROWS = 2;

const VOLCANO_PALETTE_RAW = {
  basaltA: 0x2f2a2c,
  basaltB: 0x37312f,
  basaltSeam: 0x1f1a1b,
  crack: 0xd8531e,
  crackHot: 0xf2a03c,
  ash: 0x585052,
  ashHi: 0x7c7072,
  lavaDeep: 0xc23a12,
  lavaMid: 0xf05a1a,
  lavaHot: 0xffb43c,
  lavaCrust: 0x4a3230,
  lavaCrustHi: 0x6d4741,
  rockWall: 0x453b3a,
  rockTop: 0x5f5250,
  rockShade: 0x2a2222,
  obsidian: 0x241f2b,
  obsidianHi: 0x3d3550,
  obsidianEdge: 0x6b5f86,
  anvilBody: 0x3a3438,
  anvilHi: 0x6e646a,
  toolSteel: 0x939ba6,
  toolHandle: 0x5a3f2a,
  forgeGlow: 0xff8a2b,
  forgeCore: 0xffd06a,
  smoke: 0x8c8286,
  leafDark: 0x8c2620,
  leafRed: 0xb8322a,
  leafHi: 0xe0563a,
  stem: 0x4a2f26,
  bloom: 0xf2a03c,
  barStone: 0x4e4240,
  barTop: 0x6f5f5a,
  barGlow: 0xff7a2a,
  mugMetal: 0x9aa2ad,
  tableRock: 0x554a48,
  tableRockHi: 0x786964,
  throneRock: 0x3a2f31,
  throneHi: 0x5e4c4d,
  throneLava: 0xf05a1a,
  throneCore: 0xffd06a,
};

// 화산지대 잔무늬 죽이기 — 현무암 체커·잔금·화산재를 암반 바탕으로 당긴다.
// 용암·발광 균열·화로(화산을 화산이게 하는 불빛)는 절대 건드리지 않는다 —
// 어두운 바닥이 조용해질수록 그 빛이 오히려 또렷해진다.
const VOLCANO_QUIET: readonly QuietGroup<typeof VOLCANO_PALETTE_RAW>[] = [
  { base: "basaltA", keys: ["basaltB", "basaltSeam", "ash", "ashHi"], amount: 0.55 },
  { base: "obsidian", keys: ["obsidianEdge", "obsidianHi"], amount: 0.5 },
];

/** 캐릭터가 읽히도록 배경 잔무늬를 죽인 실사용 팔레트. */
// 불빛만은 색을 남긴다 — 화산에서 주황은 장식이 아니라 "여기가 뜨겁다"는
// 정보다. 회색으로 만들면 용암이 그냥 갈색 진흙이 된다.
const VOLCANO_FIRE = {
  keys: [
    "crack",
    "crackHot",
    "lavaDeep",
    "lavaMid",
    "lavaHot",
    "forgeGlow",
    "forgeCore",
    "barGlow",
    "throneLava",
    "throneCore",
    "bloom",
  ],
  amount: 0.08,
} as const;

const VOLCANO_PALETTE = desaturatePalette(
  quietPalette(VOLCANO_PALETTE_RAW, VOLCANO_QUIET),
  SCENE_CHROMA_CUT,
  [VOLCANO_FIRE],
);

type VolcanoPalette = typeof VOLCANO_PALETTE;

/** 레터박스(맵 밖) 배경 — 재가 내려앉은 하늘. 현무암보다 한 단계 어둡다. */
const VOLCANO_BACKGROUND = desaturateColor(0x1b1517, SCENE_CHROMA_CUT);

/** 타일 좌표에서 나오는 결정적 해시 — 균열 발광/화산재/기포를 흩뿌리는 데 쓴다.
 * (베이크된 정적 텍스처라 난수를 쓰면 재베이크마다 무늬가 바뀐다.) */
const scatter = (tx: number, ty: number, mod: number): number => (tx * 53 + ty * 131) % mod;

function volcanoTileDraw(pal: VolcanoPalette): TileDrawFn {
  return (g, { t, tx, ty, s, map }) => {
    const basalt = (tx + ty) % 2 === 0 ? pal.basaltA : pal.basaltB;
    switch (t) {
      case Tile.Floor: {
        g.rect(0, 0, s, s).fill(basalt);
        // 굳은 용암의 잔금 — 칸마다 하나만. 두 갈래를 다 그으면 바닥이
        // 술렁여서 그 위를 걷는 캐릭터가 묻힌다.
        g.rect(2, 4, 4, 1).fill(pal.basaltSeam);
        // 장식은 드물게 — 촘촘하면 화산지대가 아니라 불꽃놀이가 된다.
        const k = scatter(tx, ty, 29);
        if (k === 0) {
          // 발광 균열: 갈라진 틈 사이로 마그마가 비친다
          g.rect(2, 9, 6, 1).fill(pal.crack);
          g.rect(7, 8, 5, 1).fill(pal.crack);
          g.rect(11, 10, 3, 1).fill(pal.crack);
          g.rect(4, 9, 2, 1).fill(pal.crackHot);
          g.rect(9, 8, 2, 1).fill(pal.crackHot);
        } else if (k === 7) {
          // 화산재 무더기
          g.rect(5, 10, 6, 2).fill(pal.ash);
          g.rect(6, 9, 4, 1).fill(pal.ashHi);
          g.rect(11, 4, 2, 1).fill(pal.ash);
        } else if (k === 13) {
          // 굴러온 화산탄 두 덩이
          g.rect(4, 6, 4, 3).fill(pal.rockWall);
          g.rect(4, 6, 3, 1).fill(pal.rockTop);
          g.rect(10, 10, 3, 2).fill(pal.rockWall);
          g.rect(10, 10, 2, 1).fill(pal.rockTop);
        }
        break;
      }
      case Tile.Wall: {
        if (ty < LAVA_ROWS) {
          const nearShore = ty === LAVA_ROWS - 1;
          g.rect(0, 0, s, s).fill(nearShore ? pal.lavaMid : pal.lavaDeep);
          if (!nearShore) {
            g.rect(0, 0, s, 3).fill(pal.rockShade); // 호수 건너편 암흑 능선
            g.rect(0, 3, s, 1).fill(pal.lavaCrust);
            // 먼 용암 흐름(1px 대시)
            g.rect(scatter(tx, ty, 6) + 1, 7, 5, 1).fill(pal.lavaMid);
            g.rect(scatter(tx, ty + 1, 5) + 8, 12, 3, 1).fill(pal.lavaHot);
            // 표면에 떠다니는 크러스트 판
            if (scatter(tx, ty, 3) === 0) {
              g.rect(4, 9, 7, 3).fill(pal.lavaCrust);
              g.rect(5, 9, 5, 1).fill(pal.lavaCrustHi);
            }
          } else {
            // 물가: 부글거리는 기포 + 식어 굳은 가장자리(아래쪽 두 줄)
            const b = scatter(tx, ty, 5);
            g.rect(b + 2, 4, 3, 3).fill(pal.lavaHot);
            g.rect(b + 3, 3, 1, 1).fill(pal.forgeCore);
            g.rect(11, 8, 2, 2).fill(pal.lavaHot);
            g.rect(0, s - 5, s, 2).fill(pal.lavaCrustHi);
            g.rect(0, s - 3, s, 3).fill(pal.lavaCrust);
            g.rect(scatter(tx, ty, 7) + 2, s - 2, 3, 1).fill(pal.crack); // 굳은 틈 사이 잔열
          }
          break;
        }
        // 화산 암벽(측면·하단): 밝은 상단 능선 + 균열 잔열 + 김
        g.rect(0, 0, s, s).fill(pal.rockWall);
        g.rect(0, 0, s, 3).fill(pal.rockTop);
        g.rect(2, 6, 4, 1).fill(pal.rockShade);
        g.rect(9, 10, 4, 1).fill(pal.rockShade);
        if (scatter(tx, ty, 4) === 0) {
          g.rect(6, 4, 1, 7).fill(pal.crack);
          g.rect(6, 6, 2, 2).fill(pal.crackHot);
        } else if (scatter(tx, ty, 5) === 0) {
          g.rect(3, 3, 2, 2).fill(pal.smoke); // 바위 틈에서 새는 김
          g.rect(11, 5, 2, 1).fill(pal.smoke);
        }
        break;
      }
      case Tile.Rug: {
        // 흑요석 판: 유리질 검정 + 판 이음매 + 비스듬한 반사 두 줄
        g.rect(0, 0, s, s).fill(pal.obsidian);
        g.rect(0, 0, s, 1).fill(pal.obsidianEdge); // 판 이음매(위·왼쪽)
        g.rect(0, 0, 1, s).fill(pal.obsidianEdge);
        g.rect(2, 4, 6, 1).fill(pal.obsidianHi);
        g.rect(3, 5, 3, 1).fill(pal.obsidianHi);
        g.rect(9, 10, 5, 1).fill(pal.obsidianHi);
        g.rect(11, 11, 3, 1).fill(pal.obsidianHi);
        break;
      }
      case Tile.DeskTop: {
        const isLeft = map.tiles[ty][tx - 1] !== Tile.DeskTop;
        g.rect(0, 0, s, s).fill(basalt); // 가구 타일은 바닥 베이크에서 빠지므로 스스로 현무암을 깐다
        // 돌 작업대(두 칸 공통): 상판 + 어두운 앞면. 쌍의 이음매를 끊지 않으려고
        // 칸 전체 폭으로 깐다 — 칸마다 여백을 두면 작업대가 토막나 보인다.
        g.rect(0, 9, s, 5).fill(pal.rockWall);
        g.rect(0, 9, s, 2).fill(pal.rockTop);
        g.rect(0, 14, s, 1).fill(pal.rockShade);
        if (isLeft) {
          // 모루: 뿔 + 잘록한 허리 + 받침. 오른쪽에 망치를 세워 둔다.
          g.rect(4, 4, 9, 2).fill(pal.anvilHi); // 모루 상판
          g.rect(1, 5, 3, 1).fill(pal.anvilHi); // 뿔
          g.rect(4, 6, 9, 1).fill(pal.anvilBody);
          g.rect(6, 7, 4, 1).fill(pal.anvilBody); // 잘록한 허리
          g.rect(4, 8, 9, 1).fill(pal.anvilBody); // 받침
          g.rect(13, 2, 1, 6).fill(pal.toolHandle); // 망치 자루
          g.rect(12, 1, 4, 2).fill(pal.toolSteel); // 망치 머리
        } else {
          // 화구: 달군 석탄 + 위로 오르는 불티. 상판에는 집게 한 자루.
          g.rect(2, 4, 12, 5).fill(pal.rockWall);
          g.rect(2, 4, 12, 1).fill(pal.rockTop);
          g.rect(3, 5, 10, 3).fill(pal.forgeGlow);
          g.rect(5, 6, 6, 2).fill(pal.forgeCore);
          g.rect(6, 2, 2, 2).fill(pal.crackHot); // 불티
          g.rect(9, 1, 1, 1).fill(pal.forgeCore);
          g.rect(2, 8, 7, 1).fill(pal.toolSteel); // 집게
          g.rect(2, 9, 3, 1).fill(pal.toolHandle);
        }
        break;
      }
      case Tile.Plant: {
        g.rect(0, 0, s, s).fill(basalt);
        // 열 기준으로 번갈아 — 이 맵의 Plant 세 자리(tx1/tx17/tx18)가 한쪽으로
        // 몰리지 않게(합 기준이면 행에 따라 전부 같은 쪽으로 떨어진다).
        if (tx % 2 === 0) {
          // 용암꽃: 굳은 용암 줄기 끝에 발광하는 꽃 한 송이
          g.rect(7, 8, 2, 8).fill(pal.stem);
          g.rect(6, 12, 1, 4).fill(pal.stem);
          g.rect(9, 10, 2, 1).fill(pal.stem);
          g.rect(5, 4, 6, 4).fill(pal.leafRed); // 꽃받침
          g.rect(6, 3, 4, 2).fill(pal.bloom); // 꽃잎
          g.rect(7, 2, 2, 1).fill(pal.forgeCore); // 꽃술 발광
          g.rect(3, 6, 2, 1).fill(pal.leafHi);
          g.rect(11, 6, 2, 1).fill(pal.leafHi);
          g.rect(2, 14, 12, 1).fill(pal.rockShade); // 바닥 그림자
        } else {
          // 붉은 잎 관목: 그을린 밑동 + 세 뭉치 잎
          g.rect(7, 11, 2, 5).fill(pal.stem);
          g.rect(3, 8, 10, 3).fill(pal.leafDark);
          g.rect(4, 5, 8, 3).fill(pal.leafRed);
          g.rect(5, 3, 6, 2).fill(pal.leafRed);
          g.rect(6, 4, 3, 2).fill(pal.leafHi);
          g.rect(4, 9, 3, 1).fill(pal.leafHi);
          g.rect(10, 6, 2, 1).fill(pal.bloom); // 여문 열매
          g.rect(2, 14, 12, 1).fill(pal.rockShade);
        }
        break;
      }
      case Tile.Counter: {
        // 용암석 바: 돌 카운터 + 달궈진 이음매. 칸마다 잔/냄비를 번갈아.
        g.rect(0, 0, s, s).fill(basalt);
        g.rect(0, 6, s, 8).fill(pal.barStone);
        g.rect(0, 6, s, 2).fill(pal.barTop); // 바 상판
        g.rect(0, 10, s, 1).fill(pal.barGlow); // 달궈진 돌 이음매
        g.rect(0, 14, s, 2).fill(pal.rockShade);
        if (tx % 2 === 0) {
          // 쇠잔 두 개 — 김이 오른다
          g.rect(3, 2, 4, 4).fill(pal.mugMetal);
          g.rect(3, 2, 4, 1).fill(pal.toolSteel);
          g.rect(7, 3, 1, 2).fill(pal.mugMetal); // 손잡이
          g.rect(10, 3, 3, 3).fill(pal.mugMetal);
          g.rect(4, 0, 1, 2).fill(pal.smoke);
          g.rect(11, 1, 1, 2).fill(pal.smoke);
        } else {
          // 불에 올린 냄비: 아래에서 잔불이 올라온다
          g.rect(4, 1, 8, 5).fill(pal.anvilBody);
          g.rect(4, 1, 8, 1).fill(pal.anvilHi); // 테두리
          g.rect(5, 2, 6, 2).fill(pal.forgeGlow); // 끓는 속
          g.rect(6, 2, 3, 1).fill(pal.forgeCore);
          g.rect(2, 3, 2, 1).fill(pal.anvilHi); // 손잡이
          g.rect(12, 3, 2, 1).fill(pal.anvilHi);
          g.rect(5, 6, 6, 1).fill(pal.barGlow); // 냄비 밑 잔불
        }
        break;
      }
      case Tile.Table: {
        // 바위 테이블: 두꺼운 석판 + 굵은 돌기둥 다리, 아래로 잔열이 샌다
        g.rect(0, 0, s, s).fill(basalt);
        g.rect(0, 4, s, 3).fill(pal.tableRockHi);
        g.rect(0, 7, s, 3).fill(pal.tableRock);
        g.rect(0, 10, s, 1).fill(pal.rockShade); // 석판 아랫면
        g.rect(2, 11, 4, 4).fill(pal.tableRock);
        g.rect(10, 11, 4, 4).fill(pal.tableRock);
        g.rect(2, 11, 3, 1).fill(pal.tableRockHi);
        g.rect(10, 11, 3, 1).fill(pal.tableRockHi);
        g.rect(7, 12, 2, 1).fill(pal.crack); // 다리 사이 균열 발광
        break;
      }
      case Tile.BossDesk: {
        // 용암 왕좌(세로 1×2): 위 칸이 등받이·불꽃 관, 아래 칸이 좌석·용암 받침.
        const isLower = map.tiles[ty - 1]?.[tx] === Tile.BossDesk;
        g.rect(0, 0, s, s).fill(basalt);
        if (!isLower) {
          g.rect(6, 0, 4, 2).fill(pal.throneCore); // 등받이 꼭대기 불꽃
          g.rect(3, 1, 2, 2).fill(pal.throneLava);
          g.rect(11, 1, 2, 2).fill(pal.throneLava);
          g.rect(2, 3, 12, 13).fill(pal.throneRock); // 등받이 본체
          g.rect(2, 3, 12, 1).fill(pal.throneHi);
          g.rect(4, 5, 2, 10).fill(pal.throneLava); // 갈라진 틈으로 흐르는 용암
          g.rect(10, 5, 2, 10).fill(pal.throneLava);
          g.rect(4, 7, 2, 3).fill(pal.throneCore);
          g.rect(10, 9, 2, 3).fill(pal.throneCore);
          g.rect(7, 6, 2, 6).fill(pal.throneHi); // 등받이 중앙 돌기둥
        } else {
          g.rect(2, 0, 12, 6).fill(pal.throneRock); // 등받이 아랫단
          g.rect(4, 0, 2, 3).fill(pal.throneLava);
          g.rect(10, 0, 2, 3).fill(pal.throneLava);
          g.rect(1, 6, 14, 3).fill(pal.throneHi); // 좌판
          g.rect(1, 6, 14, 1).fill(pal.throneCore); // 달궈진 앞턱
          g.rect(1, 9, 14, 4).fill(pal.throneRock); // 받침대
          g.rect(3, 10, 3, 2).fill(pal.throneLava); // 받침 사이로 새는 용암
          g.rect(10, 10, 3, 2).fill(pal.throneLava);
          g.rect(2, 13, 12, 2).fill(pal.crack); // 바닥에 고인 용암 웅덩이
          g.rect(5, 14, 5, 1).fill(pal.crackHot);
        }
        break;
      }
    }
  };
}

export const VOLCANO_SCENE: SceneDef = {
  id: "volcano",
  labelKey: "office:scene.volcano",
  map: VOLCANO_MAP,
  resolve: (theme) => {
    const mode = sceneColorMode(theme.id);
    return {
      background: adaptColor(VOLCANO_BACKGROUND, mode),
      drawTile: volcanoTileDraw(adaptPalette(VOLCANO_PALETTE, mode)),
    };
  },
};
