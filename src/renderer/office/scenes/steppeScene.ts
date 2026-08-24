// src/renderer/office/scenes/steppeScene.ts
//
// 몽골 초원 유목 캠프 풍경. 해변/계곡과 같은 규칙(의미 타일 공유, 어휘만 교체):
//   Floor=초원 풀밭(풀결·야생화), Wall=먼 산맥+하늘/구름(위 2줄)/완만한 언덕
//   능선(그 외), Rug=펠트 깔개(전통 기하 문양), DeskTop=융단 위 낮은 좌탁,
//   Plant=초원 관목·야생화 덤불, Counter=마유주(아이락) 통나무 카운터,
//   Table=무쇠 화로(솥을 건 모닥불), BossDesk=게르(유르트).
//
// 레이아웃은 해변/계곡을 베끼지 않고 캠프답게 새로 잡았다 — 게르가 맵 오른쪽
// 가운데(tx15)에 서고 그 앞(남쪽)으로 펠트 깔개 마당이 펼쳐지며, 좌탁은
// 북쪽 풀밭 두 줄(4쌍+3쌍)과 깔개 위(2쌍)에 흩어져 총 9쌍이다. 줄서기 레인은
// 게르 하단 행을 따라 서쪽으로 뻗는다(방향은 `buildSceneMap`이 유도한다).
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
  L('WWWWWWWWWWWWWWWWWWWW'), // ty0  - 하늘 + 구름 + 먼 산맥 봉우리
  L('WWWWWWWWWWWWWWWWWWWW'), // ty1  - 산자락 → 초원으로 내려오는 능선
  L('WFFFFFFFFFFFFFFFFFFW'), // ty2  - 좌석 행 1
  L('WPDDFFDDFFDDFFDDFFFW'), // ty3  - 좌탁 4쌍 + 야생화 덤불(tx1)
  L('WFFFFFFFFFFFFFFFFFFW'), // ty4
  L('WFFFFFFFFFFFFFFFFFFW'), // ty5  - 좌석 행 2
  L('WFDDFFDDFFDDFFFBFFPW'), // ty6  - 좌탁 3쌍 + 게르 상단(tx15) + 관목(tx18)
  L('WFFFFFFFFFFFFFFBFFFW'), // ty7  - 게르 하단 + 줄서기 레인(서쪽)
  L('WFFFFFFFFFFFFRRTTRRW'), // ty8  - 깔개 마당 + 화로 2칸
  L('WFFFFFFFFFFFFRRRRRRW'), // ty9  - 펠트 깔개(휴게)
  L('WFFFFFFFFFFFFRRRRRRW'), // ty10 - 펠트 깔개 + 깔개 좌탁의 좌석 행
  L('WFFPFFFFFFFFFRDDRDDW'), // ty11 - 깔개 위 좌탁 2쌍 + 야생화 덤불(tx3)
  L('WFFCCCCCCFFFFFFFFFPW'), // ty12 - 마유주 카운터 + 관목(tx18)
  L('WWWWWWWWWWWWWWWWWWWW'), // ty13 - 완만한 언덕 능선
];

/** 게르 앞에 편 펠트 깔개 마당 — 오피스 러그 라운지와 같은 역할.
 * 화로(ty8)는 통행 불가라 일부러 한 줄 아래에서 시작한다. */
const BREAK_ROOM: TileRect = { x: 13, y: 9, w: 6, h: 2 };

export const STEPPE_MAP = buildSceneMap(GRID, BREAK_ROOM);

/** 하늘·먼 산이 차지하는 위쪽 Wall 줄 수 — 그 아래 Wall은 언덕 능선으로 그린다. */
const SKY_ROWS = 2;

const STEPPE_PALETTE_RAW = {
  grassA: 0x86a94e,
  grassB: 0x7a9c46,
  grassBlade: 0x9dc165,
  grassDry: 0xc9b46a,
  stemGreen: 0x5f8a3c,
  flowerYellow: 0xf2d152,
  flowerPurple: 0xa06fc4,
  skyHigh: 0x8fc4e8,
  skyLow: 0xc6e2f2,
  cloud: 0xfdfdfa,
  cloudShade: 0xd9e6ef,
  mountainFar: 0x6d7f96,
  mountainNear: 0x8a9683,
  mountainSnow: 0xeef4f8,
  hillLight: 0x9cbb5f,
  hillDark: 0x789644,
  hillEdge: 0x5f7c38,
  feltBase: 0xf0e6cf,
  feltEdge: 0xd8c9a8,
  feltRed: 0xb8452f,
  feltBlue: 0x2f6ba8,
  matRed: 0xa8402c,
  matHi: 0xd06a4a,
  matShade: 0x6f2a1c,
  deskTop: 0xd7a460,
  deskWood: 0xb5813f,
  deskShade: 0x7d5426,
  laptopLid: 0x4c5a6e,
  laptopBody: 0x36414f,
  bowlWood: 0xd9b075,
  bowlHi: 0xf0dcb0,
  airagSkin: 0xc98f57,
  shrubDark: 0x3f6b32,
  shrubMid: 0x548544,
  shrubHi: 0x77a95a,
  shrubShade: 0x3d5326,
  branch: 0x8a6a41,
  logWood: 0xa9763f,
  logTop: 0xcf9a5c,
  logGrain: 0x8a5c2e,
  logShade: 0x6d4522,
  barrelWood: 0xb5854c,
  barrelTop: 0xd9b177,
  barrelHoop: 0x7c7a72,
  ladle: 0x8a6a41,
  milk: 0xf7f2e4,
  brazier: 0x4a4640,
  brazierHi: 0x6e6a62,
  ember: 0xd9542f,
  fireFlame: 0xf2803a,
  fireCore: 0xf7d154,
  cauldron: 0x3a3733,
  cauldronHi: 0x5d5850,
  cauldronRim: 0x7a746a,
  steam: 0xdfe6e6,
  gerFelt: 0xf2ece0,
  gerFeltHi: 0xfdfbf4,
  gerRoof: 0xe2d9c6,
  gerBand: 0x2f6ba8,
  gerCrown: 0xb8452f,
  gerSmoke: 0xcfd4d2,
  gerDoor: 0xe07a2f,
  gerDoorFrame: 0xb8452f,
  gerRope: 0xb59a6a,
  gerShade: 0x6c7a44,
};

// 초원 잔무늬 죽이기 — 체커 두 톤과 바람결(풀날)을 풀 바탕으로 당긴다.
// 야생화·게르·설산은 초원의 특징이라 손대지 않는다.
const STEPPE_QUIET: readonly QuietGroup<typeof STEPPE_PALETTE_RAW>[] = [
  { base: "grassA", keys: ["grassB", "grassBlade"], amount: 0.62 },
];

/** 캐릭터가 읽히도록 배경 잔무늬를 죽인 실사용 팔레트. */
// 지평선 너머 하늘·구름·설산.
const STEPPE_KEEP = [
  {
    keys: ["skyHigh", "skyLow", "cloud", "cloudShade", "mountainFar", "mountainSnow"],
    amount: 0.1,
  },
] as const;

const STEPPE_PALETTE = desaturatePalette(
  quietPalette(STEPPE_PALETTE_RAW, STEPPE_QUIET),
  SCENE_CHROMA_CUT,
  STEPPE_KEEP,
);

type SteppePalette = typeof STEPPE_PALETTE;

/** 레터박스(맵 밖) 배경 — 풀밭보다 한 단계 어두운 초원 그늘색. */
const STEPPE_BACKGROUND = desaturateColor(0x4e6b3c, SCENE_CHROMA_CUT);

/** 결정적 흩뿌리기(풀포기/야생화/구름/봉우리 높이). 베이크된 정적 텍스처라
 * 난수를 쓰면 재베이크마다 무늬가 바뀐다 — 해변/계곡과 같은 이유. */
const scatter = (tx: number, ty: number, mod: number): number => (tx * 61 + ty * 113) % mod;

function steppeTileDraw(pal: SteppePalette): TileDrawFn {
  return (g, { t, tx, ty, s, map }) => {
    const grass = (tx + ty) % 2 === 0 ? pal.grassA : pal.grassB;
    switch (t) {
      case Tile.Floor: {
        g.rect(0, 0, s, s).fill(grass);
        // 바람에 한쪽으로 눕는 풀결 — 한 갈래만. 두 갈래를 칸마다 찍으면
        // 초원 전체가 잔털로 뒤덮여 캐릭터가 파묻힌다.
        g.rect(3, 4, 1, 3).fill(pal.grassBlade);
        g.rect(4, 6, 1, 2).fill(pal.grassBlade);
        // 장식은 드물게 — 촘촘하면 초원이 아니라 화단으로 보인다.
        const k = scatter(tx, ty, 17);
        if (k === 0) {
          // 노란 야생화 두 송이
          g.rect(5, 10, 1, 3).fill(pal.stemGreen);
          g.rect(4, 9, 3, 1).fill(pal.flowerYellow);
          g.rect(9, 7, 1, 3).fill(pal.stemGreen);
          g.rect(8, 6, 3, 1).fill(pal.flowerYellow);
        } else if (k === 4) {
          // 보라 야생화 무리
          g.rect(6, 8, 1, 4).fill(pal.stemGreen);
          g.rect(5, 7, 3, 1).fill(pal.flowerPurple);
          g.rect(11, 9, 1, 3).fill(pal.stemGreen);
          g.rect(10, 8, 3, 1).fill(pal.flowerPurple);
        } else if (k === 7) {
          // 마른 억새 포기 — 초원 특유의 누런 결
          g.rect(7, 7, 1, 6).fill(pal.grassDry);
          g.rect(9, 8, 1, 5).fill(pal.grassDry);
          g.rect(8, 9, 1, 4).fill(pal.grassDry);
        }
        break;
      }
      case Tile.Wall: {
        if (ty < SKY_ROWS) {
          const isHorizon = ty === SKY_ROWS - 1;
          if (!isHorizon) {
            // 맨 윗줄: 하늘 + 구름 + 먼 산맥 봉우리(칸마다 높이를 흔든다)
            g.rect(0, 0, s, s).fill(pal.skyHigh);
            g.rect(0, 10, s, 6).fill(pal.skyLow); // 지평선 쪽이 옅다
            const c = scatter(tx, ty, 6);
            if (c === 0) {
              g.rect(2, 3, 8, 2).fill(pal.cloud);
              g.rect(4, 2, 5, 1).fill(pal.cloud);
              g.rect(3, 5, 4, 1).fill(pal.cloudShade);
            } else if (c === 3) {
              g.rect(9, 5, 6, 2).fill(pal.cloud);
              g.rect(11, 4, 3, 1).fill(pal.cloud);
              g.rect(10, 7, 3, 1).fill(pal.cloudShade);
            }
            const peak = scatter(tx, ty, 5);
            g.rect(0, 8 + peak, s, s - 8 - peak).fill(pal.mountainFar);
            g.rect(0, 8 + peak, s, 2).fill(pal.mountainSnow); // 만년설 능선
            g.rect(4, 10 + peak, 3, 1).fill(pal.mountainSnow);
            break;
          }
          // 둘째 줄: 산자락이 초원으로 내려앉는 구간
          g.rect(0, 0, s, s).fill(pal.mountainNear);
          g.rect(0, 0, s, 3).fill(pal.mountainFar); // 위쪽은 먼 산 색을 이어받는다
          const roll = scatter(tx, ty, 4);
          g.rect(0, 7 + roll, s, s - 7 - roll).fill(pal.hillDark);
          g.rect(0, 7 + roll, s, 1).fill(pal.hillLight);
          g.rect(3, 9 + roll, 1, 2).fill(pal.grassBlade);
          g.rect(11, 10 + roll, 1, 2).fill(pal.grassBlade);
          break;
        }
        // 측면·하단: 완만한 언덕 능선 + 바람에 눕는 풀
        g.rect(0, 0, s, s).fill(pal.hillDark);
        g.rect(0, 0, s, 4).fill(pal.hillLight);
        g.rect(0, 4, s, 1).fill(pal.hillEdge);
        if (scatter(tx, ty, 3) === 0) {
          g.rect(5, 6, 1, 4).fill(pal.grassBlade);
          g.rect(7, 7, 1, 3).fill(pal.grassBlade);
          g.rect(9, 5, 1, 5).fill(pal.grassBlade);
        } else if (scatter(tx, ty, 5) === 1) {
          // 마른 풀 무더기 한 점
          g.rect(4, 8, 4, 2).fill(pal.grassDry);
          g.rect(10, 11, 3, 1).fill(pal.grassDry);
        }
        break;
      }
      case Tile.Rug: {
        // 펠트 깔개(에스기): 크림색 바탕에 붉은/파란 기하 문양. 칸마다 두 문양을
        // 번갈아 놓아 여러 장을 이어 깐 것처럼 보이게 한다(통짜로 칠하면 장판).
        g.rect(0, 0, s, s).fill(pal.feltBase);
        g.rect(0, 0, s, 1).fill(pal.feltEdge);
        g.rect(0, s - 1, s, 1).fill(pal.feltEdge);
        if ((tx + ty) % 2 === 0) {
          // 울지(무한매듭) 계열 — 바깥 테를 십자로 도려내 네 귀 브래킷만 남긴다
          g.rect(3, 3, 10, 10).fill(pal.feltRed);
          g.rect(5, 5, 6, 6).fill(pal.feltBase);
          g.rect(7, 2, 2, 12).fill(pal.feltBase);
          g.rect(2, 7, 12, 2).fill(pal.feltBase);
          g.rect(6, 6, 4, 4).fill(pal.feltBlue);
          g.rect(7, 7, 2, 2).fill(pal.feltBase);
        } else {
          // 마름모 + 뿔(에베르) 문양 — 십자 위에 겹친 동심 사각
          g.rect(7, 3, 2, 10).fill(pal.feltBlue);
          g.rect(3, 7, 10, 2).fill(pal.feltBlue);
          g.rect(5, 5, 6, 6).fill(pal.feltRed);
          g.rect(6, 6, 4, 4).fill(pal.feltBase);
          g.rect(7, 7, 2, 2).fill(pal.feltRed);
        }
        break;
      }
      case Tile.DeskTop: {
        const isLeft = map.tiles[ty][tx - 1] !== Tile.DeskTop;
        // 좌석(바로 위 칸)이 깔개면 마당 안쪽 좌탁, 아니면 풀밭 위 좌탁이다.
        // 가구 타일은 바닥 베이크에서 빠지므로 스스로 바닥을 깐다.
        const onFelt = map.tiles[ty - 1]?.[tx] === Tile.Rug;
        g.rect(0, 0, s, s).fill(onFelt ? pal.feltBase : grass);
        if (onFelt) g.rect(0, 0, s, 1).fill(pal.feltEdge); // 위 깔개와 이음매를 맞춘다
        // 좌탁 밑에 깔린 붉은 융단 — 칸을 가로질러 그려 2칸 쌍이 한 자리로 읽힌다.
        g.rect(0, 6, s, 9).fill(pal.matRed);
        g.rect(0, 6, s, 1).fill(pal.matHi);
        g.rect(0, 14, s, 1).fill(pal.matShade);
        // 낮은 좌탁: 상판 + 앞치마 + 짧은 다리(앉아서 쓰는 높이)
        g.rect(1, 8, 14, 3).fill(pal.deskTop);
        g.rect(1, 11, 14, 2).fill(pal.deskWood);
        g.rect(1, 13, 14, 1).fill(pal.deskShade);
        g.rect(2, 13, 2, 2).fill(pal.deskWood);
        g.rect(12, 13, 2, 2).fill(pal.deskWood);
        if (isLeft) {
          // 랩탑(뒷모습) — 오피스와 같은 이디엄(뚜껑 등판이 뷰어를 향한다)
          g.rect(4, 7, 8, 1).fill(pal.laptopBody);
          g.rect(4, 3, 8, 4).fill(pal.laptopLid);
          g.rect(4, 6, 8, 1).fill(pal.laptopBody);
          g.rect(7, 4, 2, 2).fill(pal.laptopBody);
        } else {
          // 짝 칸에는 나무 사발과 아이락 가죽 주머니
          g.rect(4, 4, 5, 3).fill(pal.bowlWood);
          g.rect(4, 4, 5, 1).fill(pal.bowlHi);
          g.rect(10, 3, 3, 4).fill(pal.airagSkin);
          g.rect(10, 3, 3, 1).fill(pal.branch);
        }
        break;
      }
      case Tile.Plant: {
        g.rect(0, 0, s, s).fill(grass);
        // 열 기준으로 번갈아 — 이 맵의 Plant 네 자리가 두 모습으로 갈리도록
        // (합 기준이면 같은 열의 둘이 같은 쪽으로만 떨어진다).
        if (tx % 2 === 0) {
          // 초원 관목: 낮고 둥근 덤불 + 마른 가지
          g.rect(3, 7, 10, 7).fill(pal.shrubDark);
          g.rect(4, 5, 8, 3).fill(pal.shrubMid);
          g.rect(6, 4, 4, 2).fill(pal.shrubMid);
          g.rect(5, 6, 3, 2).fill(pal.shrubHi);
          g.rect(9, 8, 3, 2).fill(pal.shrubHi);
          g.rect(7, 13, 2, 3).fill(pal.branch);
          g.rect(2, 14, 12, 1).fill(pal.shrubShade);
        } else {
          // 야생화 덤불: 잎 무더기 + 노랑·보라 꽃대
          g.rect(4, 9, 8, 5).fill(pal.shrubMid);
          g.rect(3, 11, 10, 3).fill(pal.shrubDark);
          g.rect(5, 6, 1, 4).fill(pal.stemGreen);
          g.rect(4, 5, 3, 1).fill(pal.flowerYellow);
          g.rect(9, 5, 1, 5).fill(pal.stemGreen);
          g.rect(8, 4, 3, 1).fill(pal.flowerPurple);
          g.rect(11, 7, 1, 3).fill(pal.stemGreen);
          g.rect(10, 6, 3, 1).fill(pal.flowerYellow);
          g.rect(3, 14, 10, 1).fill(pal.shrubShade);
        }
        break;
      }
      case Tile.Counter: {
        // 마유주(아이락) 카운터: 통나무를 켜 얹은 상판. 칸마다 통과 사발을 번갈아.
        g.rect(0, 0, s, s).fill(grass);
        g.rect(0, 6, s, 7).fill(pal.logWood);
        g.rect(0, 6, s, 2).fill(pal.logTop); // 상판
        g.rect(0, 9, s, 1).fill(pal.logGrain); // 통나무 결
        g.rect(0, 13, s, 2).fill(pal.logShade);
        if (tx % 2 === 0) {
          // 아이락 통 + 국자
          g.rect(4, 1, 8, 5).fill(pal.barrelWood);
          g.rect(4, 1, 8, 1).fill(pal.barrelTop);
          g.rect(4, 3, 8, 1).fill(pal.barrelHoop); // 테
          g.rect(12, 2, 2, 4).fill(pal.ladle);
          g.rect(11, 1, 3, 1).fill(pal.ladle);
        } else {
          // 나무 사발 두 개 — 위쪽에 우유 거품 한 줄
          g.rect(3, 3, 5, 3).fill(pal.bowlWood);
          g.rect(3, 3, 5, 1).fill(pal.milk);
          g.rect(9, 2, 5, 4).fill(pal.bowlWood);
          g.rect(9, 2, 5, 1).fill(pal.milk);
          g.rect(9, 5, 5, 1).fill(pal.logGrain);
        }
        break;
      }
      case Tile.Table: {
        // 무쇠 화로(2칸): 왼 칸이 타오르는 불, 오른 칸이 그 위에 걸린 솥.
        const isLeft = map.tiles[ty][tx - 1] !== Tile.Table;
        g.rect(0, 0, s, s).fill(grass);
        // 다리 달린 낮은 화덕 상자는 두 칸 공통 — 이어 붙어 하나로 읽힌다.
        g.rect(0, 9, s, 5).fill(pal.brazier);
        g.rect(0, 9, s, 1).fill(pal.brazierHi);
        g.rect(1, 11, 14, 2).fill(pal.ember);
        g.rect(2, 14, 2, 2).fill(pal.brazier);
        g.rect(12, 14, 2, 2).fill(pal.brazier);
        if (isLeft) {
          // 장작 + 불꽃
          g.rect(3, 8, 10, 2).fill(pal.branch);
          g.rect(5, 3, 6, 6).fill(pal.fireFlame);
          g.rect(7, 1, 3, 4).fill(pal.fireCore);
          g.rect(6, 5, 2, 3).fill(pal.fireCore);
          g.rect(11, 6, 2, 3).fill(pal.fireFlame);
        } else {
          // 화덕에 얹은 무쇠 솥 + 김
          g.rect(2, 3, 12, 6).fill(pal.cauldron);
          g.rect(1, 2, 14, 2).fill(pal.cauldronRim); // 넓은 솥 전
          g.rect(3, 4, 10, 1).fill(pal.cauldronHi); // 안쪽 반사
          g.rect(3, 8, 10, 1).fill(pal.ember); // 밑불이 비친다
          g.rect(5, 0, 2, 2).fill(pal.steam);
          g.rect(9, 0, 2, 1).fill(pal.steam);
        }
        break;
      }
      case Tile.BossDesk: {
        // 게르(유르트, 세로 1×2): 위 칸이 지붕·토노(천창), 아래 칸이 흰 펠트 벽과 문.
        const isLower = map.tiles[ty - 1]?.[tx] === Tile.BossDesk;
        g.rect(0, 0, s, s).fill(grass);
        if (!isLower) {
          g.rect(7, 0, 2, 1).fill(pal.gerSmoke); // 천창으로 오르는 연기
          g.rect(6, 1, 4, 2).fill(pal.gerCrown); // 토노(천창 테)
          g.rect(2, 3, 12, 3).fill(pal.gerRoof); // 지붕 윗면
          g.rect(1, 6, 14, 4).fill(pal.gerFelt); // 지붕 아랫면(밝은 펠트)
          g.rect(4, 3, 1, 7).fill(pal.gerRoof); // 서까래(우니) 두 줄
          g.rect(11, 3, 1, 7).fill(pal.gerRoof);
          g.rect(0, 10, s, 2).fill(pal.gerBand); // 처마 장식띠
          g.rect(0, 12, s, 4).fill(pal.gerFelt); // 벽 상단으로 이어진다
          g.rect(0, 12, s, 1).fill(pal.gerFeltHi);
        } else {
          g.rect(0, 0, s, 11).fill(pal.gerFelt); // 흰 펠트 벽
          g.rect(0, 0, s, 1).fill(pal.gerFeltHi);
          g.rect(4, 1, 8, 10).fill(pal.gerDoorFrame); // 문틀
          g.rect(5, 2, 6, 9).fill(pal.gerDoor); // 주황 문
          g.rect(7, 2, 2, 9).fill(pal.gerDoorFrame); // 두 짝 이음매
          g.rect(6, 6, 1, 2).fill(pal.gerBand); // 손잡이
          g.rect(9, 6, 1, 2).fill(pal.gerBand);
          g.rect(0, 11, s, 2).fill(pal.gerRope); // 벽을 두른 밧줄
          g.rect(0, 13, s, 2).fill(pal.gerShade); // 땅에 닿는 그림자
        }
        break;
      }
    }
  };
}

export const STEPPE_SCENE: SceneDef = {
  id: "steppe",
  labelKey: "office:scene.steppe",
  map: STEPPE_MAP,
  resolve: (theme) => {
    const mode = sceneColorMode(theme.id);
    return {
      background: adaptColor(STEPPE_BACKGROUND, mode),
      drawTile: steppeTileDraw(adaptPalette(STEPPE_PALETTE, mode)),
    };
  },
};
