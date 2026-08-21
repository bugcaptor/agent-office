// src/renderer/office/scenes/castleScene.ts
//
// 중세 성 대전당(great hall) 풍경. 해변/계곡 씬과 같은 규칙(의미 타일 공유,
// 어휘만 교체):
//   Floor=플래그스톤 석재 바닥, Wall=성벽(ty0 총안·ty1 스테인드글라스 창·
//   그 외 석축+휘장), Rug=붉은 카펫(금색 테두리), DeskTop=필경대,
//   Plant=횃불 스탠드 옆 화분 관목, Counter=연회 술통 카운터,
//   Table=연회 목재 테이블, BossDesk=단상 위 왕좌.
//
// 레이아웃은 해변/계곡과 일부러 다르게 잡았다 — 왕좌가 대전당 안쪽(위쪽)
// 한가운데 서고, 거기서 붉은 카펫이 두 칸 폭 통로로 곧장 남쪽으로 뻗어
// 아래쪽 연회장까지 이어진다. 필경대는 그 통로 좌우로 4쌍씩 두 줄(= 좌석 8개,
// 오피스와 동일). 알현 줄은 왕좌 하단 행을 따라 서쪽으로 늘어선다
// (줄 슬롯 방향은 `buildSceneMap`이 보스 위치에서 유도한다 — 왕좌가 맵
// 오른쪽 절반의 첫 칸 tx10이라 서쪽으로 뻗는다).
import type { TileRect } from "../map/mapData";
import { L, Tile, buildSceneMap } from "../map/mapData";
import { adaptColor, adaptPalette, sceneColorMode } from "./sceneColor";
import type { SceneDef, TileDrawFn } from "./sceneTypes";

const GRID: Tile[][] = [
  L('WWWWWWWWWWWWWWWWWWWW'), // ty0  - 성벽 총안(성가퀴) + 밤하늘
  L('WWWWWWWWWWWWWWWWWWWW'), // ty1  - 석축 + 스테인드글라스 창
  L('WFFFFFFFFPBPFFFFFFFW'), // ty2  - 왕좌 상단(tx10) + 좌우 횃불(tx9/tx11)
  L('WFFFFFFFFFBFFFFFFFFW'), // ty3  - 왕좌 하단 + 알현 줄 레인(서쪽)
  L('WFFFFFFFFRRFFFFFFFFW'), // ty4  - 좌석 행 1 + 카펫 통로 시작
  L('WFDDFDDFFRRFFDDFDDFW'), // ty5  - 필경대 4쌍(통로 좌우 2쌍씩)
  L('WFFFFFFFFRRFFFFFFFFW'), // ty6
  L('WPFFFFFFFRRFFFFFFFPW'), // ty7  - 좌석 행 2 + 횃불·관목(tx1/tx18)
  L('WFDDFDDFFRRFFDDFDDFW'), // ty8  - 필경대 4쌍
  L('WFFFFFFFFRRFFFFFFFFW'), // ty9
  L('WFRRRRRRRRRRRRRRRRFW'), // ty10 - 카펫이 펼쳐진 연회장
  L('WFRRRRTTRRRRRRTTRRFW'), // ty11 - 카펫 + 연회 테이블 2벌
  L('WFCCCCCCFFFFFFFFFFFW'), // ty12 - 술통 카운터
  L('WWWWWWWWWWWWWWWWWWWW'), // ty13 - 대전당 안쪽 벽
];

/** 카펫 위에서 잔을 드는 자리 — 오피스 러그 라운지와 같은 역할. */
const BREAK_ROOM: TileRect = { x: 9, y: 10, w: 5, h: 2 };

export const CASTLE_MAP = buildSceneMap(GRID, BREAK_ROOM);

/** 성가퀴(총안)가 서는 행. 이 위는 하늘, 그 아래 행은 창이 뚫린 석축. */
const CRENEL_ROW = 0;
const WINDOW_ROW = 1;

const CASTLE_PALETTE = {
  stoneA: 0x8e8a83,
  stoneB: 0x847f78,
  stoneSeam: 0x9d9890,
  stoneShade: 0x6b6660,
  stoneCrack: 0x5f5a55,
  stoneStain: 0x9a958c,
  skyNight: 0x2b3550,
  wallStone: 0x6f6a64,
  wallStoneHi: 0x8a847c,
  wallMortar: 0x565049,
  glassLead: 0x3a3630,
  glassBlue: 0x3f74b8,
  glassRed: 0xc4453f,
  glassGold: 0xe8b74a,
  bannerCloth: 0x8c2a33,
  bannerTrim: 0xe0b957,
  moss: 0x5d7a44,
  carpetRed: 0xa82f36,
  carpetDark: 0x86232b,
  carpetGold: 0xd9ac4c,
  carpetGoldHi: 0xf0d089,
  deskWood: 0x6d4a2c,
  deskTop: 0x8d6238,
  deskShade: 0x4a3120,
  parchment: 0xece0c0,
  parchmentHi: 0xf7f0da,
  ink: 0x3a3226,
  quill: 0xf3ead6,
  quillHi: 0xd9c9a6,
  candleStand: 0xb9902f,
  candleWax: 0xf2ead2,
  candleFlame: 0xf5b33c,
  inkBottle: 0x2f4a63,
  inkBottleHi: 0x50708c,
  torchPost: 0x4f4a44,
  torchBowl: 0x7a6a3c,
  flame: 0xe8752c,
  flameCore: 0xf7d154,
  potClay: 0xa5643c,
  potRim: 0xc48054,
  shrubDark: 0x2f6b3c,
  shrubHi: 0x4a9455,
  barrelWood: 0x7a4e2a,
  barrelTop: 0x9c6a3c,
  barrelHoop: 0x4a4a50,
  barrelHoopHi: 0x767680,
  mug: 0xc2a066,
  mugFoam: 0xf5efdc,
  feastWood: 0x8a5c33,
  feastTop: 0xac7844,
  cloth: 0xe4dcc2,
  bread: 0xd2a35c,
  meat: 0xa8552f,
  goblet: 0xd9ac4c,
  throneGold: 0xc9a03e,
  throneGoldHi: 0xf0d089,
  throneCushion: 0x9c2730,
  daisStone: 0x9a948b,
  daisStep: 0xb4ada2,
};

type CastlePalette = typeof CASTLE_PALETTE;

/** 레터박스(맵 밖) 배경 — 석재 바닥보다 훨씬 어두운 성벽 그늘색. */
const CASTLE_BACKGROUND = 0x33302e;

/** 결정적 흩뿌리기(바닥 균열·돌 얼룩·창·휘장 배치). 베이크된 정적 텍스처라 난수 금지. */
const scatter = (tx: number, ty: number, mod: number): number => (tx * 61 + ty * 113) % mod;

function castleTileDraw(pal: CastlePalette): TileDrawFn {
  return (g, { t, tx, ty, s, map }) => {
    const stone = (tx + ty) % 2 === 0 ? pal.stoneA : pal.stoneB;
    /** 이 칸이 카펫으로 읽히는가 — 카펫 위에 얹힌 연회 테이블도 카펫 취급해야
     * 테이블 둘레에 금테가 끼어들지 않는다. */
    const onCarpet = (x: number, y: number): boolean => {
      const n = map.tiles[y]?.[x];
      return n === Tile.Rug || n === Tile.Table;
    };
    switch (t) {
      case Tile.Floor: {
        // 플래그스톤: 체커 두 톤 + 위/왼쪽 줄눈, 아래/오른쪽 그늘 → 돌판 격자
        g.rect(0, 0, s, s).fill(stone);
        g.rect(0, 0, s, 1).fill(pal.stoneSeam);
        g.rect(0, 0, 1, s).fill(pal.stoneSeam);
        g.rect(0, s - 1, s, 1).fill(pal.stoneShade);
        g.rect(s - 1, 0, 1, s).fill(pal.stoneShade);
        // 장식은 드물게 — 촘촘하면 대전당 바닥이 아니라 폐허로 보인다.
        const k = scatter(tx, ty, 11);
        if (k === 0) {
          // 균열: 꺾인 1px 선
          g.rect(4, 4, 1, 3).fill(pal.stoneCrack);
          g.rect(5, 7, 1, 2).fill(pal.stoneCrack);
          g.rect(6, 9, 3, 1).fill(pal.stoneCrack);
          g.rect(9, 10, 1, 2).fill(pal.stoneCrack);
        } else if (k === 6) {
          // 물때 얼룩: 뭉툭한 두 덩이
          g.rect(4, 6, 5, 3).fill(pal.stoneStain);
          g.rect(6, 9, 3, 1).fill(pal.stoneStain);
          g.rect(10, 3, 3, 2).fill(pal.stoneStain);
        }
        break;
      }
      case Tile.Wall: {
        if (ty === CRENEL_ROW) {
          // 성가퀴: 밤하늘 위로 총안(빈 칸)과 성돌(merlon)이 번갈아 선다.
          g.rect(0, 0, s, s).fill(pal.skyNight);
          g.rect(0, 6, s, s - 6).fill(pal.wallStone);
          g.rect(0, 6, s, 1).fill(pal.wallStoneHi); // 총안 바닥 갓돌
          if (tx % 2 === 0) {
            g.rect(2, 1, 12, 5).fill(pal.wallStone);
            g.rect(2, 1, 12, 1).fill(pal.wallStoneHi);
            g.rect(2, 4, 12, 1).fill(pal.wallMortar);
          }
          g.rect(0, 10, s, 1).fill(pal.wallMortar); // 벽돌 줄눈
          g.rect(tx % 2 === 0 ? 5 : 11, 11, 1, 5).fill(pal.wallMortar);
          break;
        }
        if (ty === WINDOW_ROW) {
          // 석축 + 네 칸마다 한 번씩 뚫린 좁고 긴 스테인드글라스 창.
          g.rect(0, 0, s, s).fill(pal.wallStone);
          g.rect(0, 5, s, 1).fill(pal.wallMortar);
          g.rect(0, 11, s, 1).fill(pal.wallMortar);
          g.rect(tx % 2 === 0 ? 5 : 11, 0, 1, 5).fill(pal.wallMortar);
          g.rect(tx % 2 === 0 ? 11 : 5, 6, 1, 5).fill(pal.wallMortar);
          if (scatter(tx, ty, 4) === 0) {
            g.rect(4, 1, 8, 14).fill(pal.glassLead); // 창틀(납선)
            g.rect(6, 1, 4, 1).fill(pal.glassBlue); // 아치 꼭대기
            g.rect(5, 2, 6, 4).fill(pal.glassBlue);
            g.rect(5, 6, 6, 3).fill(pal.glassRed);
            g.rect(5, 9, 6, 3).fill(pal.glassGold);
            g.rect(5, 12, 6, 2).fill(pal.glassBlue);
            g.rect(7, 1, 2, 13).fill(pal.glassLead); // 세로 납선
            g.rect(5, 5, 6, 1).fill(pal.glassLead); // 가로 납선
            g.rect(5, 11, 6, 1).fill(pal.glassLead);
          }
          break;
        }
        // 측면·하단 석축: 벽돌 엇쌓기 + 드문드문 걸린 문장 휘장, 이끼.
        g.rect(0, 0, s, s).fill(pal.wallStone);
        g.rect(0, 0, s, 1).fill(pal.wallStoneHi);
        g.rect(0, 5, s, 1).fill(pal.wallMortar);
        g.rect(0, 11, s, 1).fill(pal.wallMortar);
        g.rect(tx % 2 === 0 ? 4 : 12, 0, 1, 5).fill(pal.wallMortar);
        g.rect(tx % 2 === 0 ? 12 : 4, 6, 1, 5).fill(pal.wallMortar);
        g.rect(tx % 2 === 0 ? 4 : 12, 12, 1, 4).fill(pal.wallMortar);
        if (scatter(tx, ty, 4) === 0) {
          // 문장 휘장: 가로 걸대 + 아래가 뾰족한 천
          g.rect(3, 2, 10, 1).fill(pal.bannerTrim);
          g.rect(4, 3, 8, 9).fill(pal.bannerCloth);
          g.rect(4, 3, 8, 1).fill(pal.bannerTrim);
          g.rect(5, 12, 6, 1).fill(pal.bannerCloth);
          g.rect(7, 13, 2, 1).fill(pal.bannerCloth);
          g.rect(6, 6, 4, 3).fill(pal.bannerTrim); // 문장
        } else if (scatter(tx, ty, 5) === 0) {
          g.rect(2, 12, 4, 2).fill(pal.moss);
          g.rect(11, 6, 3, 2).fill(pal.moss);
        }
        break;
      }
      case Tile.Rug: {
        // 붉은 카펫: 금색 테두리는 "카펫이 아닌 이웃" 쪽 변에만 그린다 —
        // 그래야 두 칸 폭 통로와 넓은 연회장이 각각 하나의 융단으로 읽힌다.
        g.rect(0, 0, s, s).fill(pal.carpetRed);
        // 짜여진 결(1px) + 가운데 마름모 무늬
        g.rect(0, 4, s, 1).fill(pal.carpetDark);
        g.rect(0, 12, s, 1).fill(pal.carpetDark);
        g.rect(6, 6, 4, 4).fill(pal.carpetDark);
        g.rect(7, 5, 2, 6).fill(pal.carpetDark);
        g.rect(5, 7, 6, 2).fill(pal.carpetDark);
        g.rect(7, 7, 2, 2).fill(pal.carpetGold);
        if (!onCarpet(tx, ty - 1)) {
          g.rect(0, 0, s, 2).fill(pal.carpetGold);
          g.rect(0, 0, s, 1).fill(pal.carpetGoldHi);
        }
        if (!onCarpet(tx, ty + 1)) {
          g.rect(0, s - 2, s, 2).fill(pal.carpetGold);
          g.rect(0, s - 1, s, 1).fill(pal.carpetGoldHi);
        }
        if (!onCarpet(tx - 1, ty)) {
          g.rect(0, 0, 2, s).fill(pal.carpetGold);
          g.rect(0, 0, 1, s).fill(pal.carpetGoldHi);
        }
        if (!onCarpet(tx + 1, ty)) {
          g.rect(s - 2, 0, 2, s).fill(pal.carpetGold);
          g.rect(s - 1, 0, 1, s).fill(pal.carpetGoldHi);
        }
        break;
      }
      case Tile.DeskTop: {
        const isLeft = map.tiles[ty][tx - 1] !== Tile.DeskTop;
        g.rect(0, 0, s, s).fill(stone); // 가구 타일은 바닥 베이크에서 빠지므로 스스로 돌바닥을 깐다
        // 필경대: 두꺼운 참나무 상판 + 앞판 + 굵은 다리
        g.rect(0, 5, s, 8).fill(pal.deskWood);
        g.rect(0, 5, s, 2).fill(pal.deskTop);
        g.rect(0, 9, s, 1).fill(pal.deskShade); // 상판/앞판 이음매
        g.rect(0, 12, s, 1).fill(pal.deskShade);
        g.rect(1, 13, 3, 3).fill(pal.deskWood);
        g.rect(12, 13, 3, 3).fill(pal.deskWood);
        g.rect(4, 15, 8, 1).fill(pal.deskShade); // 다리 사이 바닥 그림자
        if (isLeft) {
          // 펼친 양피지 + 깃펜 — 오피스 랩탑과 같은 자리(쌍의 왼쪽 칸)
          g.rect(2, 1, 9, 4).fill(pal.parchment);
          g.rect(2, 1, 9, 1).fill(pal.parchmentHi);
          g.rect(3, 3, 7, 1).fill(pal.ink); // 필사한 글줄
          g.rect(3, 4, 5, 1).fill(pal.ink);
          g.rect(12, 0, 1, 5).fill(pal.quill); // 깃펜대
          g.rect(11, 1, 1, 3).fill(pal.quillHi);
        } else {
          // 짝 칸에는 촛대와 잉크병
          g.rect(3, 4, 4, 1).fill(pal.candleStand);
          g.rect(4, 1, 2, 3).fill(pal.candleWax);
          g.rect(4, 0, 2, 1).fill(pal.candleFlame);
          g.rect(9, 2, 4, 3).fill(pal.inkBottle);
          g.rect(9, 2, 4, 1).fill(pal.inkBottleHi); // 유리 하이라이트
          g.rect(10, 1, 2, 1).fill(pal.inkBottle); // 병목
        }
        break;
      }
      case Tile.Plant: {
        // 횃불 스탠드(왼쪽) + 화분 관목(오른쪽) — 식물 요소는 유지하되
        // 성답게 불을 곁들인다.
        g.rect(0, 0, s, s).fill(stone);
        g.rect(3, 7, 2, 8).fill(pal.torchPost); // 쇠기둥
        g.rect(1, 14, 6, 2).fill(pal.torchPost); // 삼발이 받침
        g.rect(1, 5, 6, 3).fill(pal.torchBowl); // 불 그릇
        g.rect(1, 5, 6, 1).fill(pal.flameCore); // 그릇 테 반사
        g.rect(2, 2, 4, 3).fill(pal.flame);
        g.rect(3, 0, 2, 3).fill(pal.flameCore);
        g.rect(9, 11, 6, 5).fill(pal.potClay); // 토분
        g.rect(9, 11, 6, 1).fill(pal.potRim);
        g.rect(9, 5, 6, 6).fill(pal.shrubDark); // 관목
        g.rect(10, 3, 4, 3).fill(pal.shrubDark);
        g.rect(10, 6, 3, 2).fill(pal.shrubHi);
        g.rect(12, 9, 2, 1).fill(pal.shrubHi);
        break;
      }
      case Tile.Counter: {
        // 술통 카운터: 눕힌 통을 잇대 만든 바. 칸마다 맥주잔/세운 통을 번갈아.
        g.rect(0, 0, s, s).fill(stone);
        g.rect(0, 7, s, 8).fill(pal.barrelWood);
        g.rect(0, 7, s, 2).fill(pal.barrelTop); // 상판 널
        g.rect(0, 10, s, 1).fill(pal.barrelHoop); // 통 테
        g.rect(0, 13, s, 1).fill(pal.barrelHoop);
        g.rect(0, 10, s, 1).fill(tx % 2 === 0 ? pal.barrelHoop : pal.barrelHoopHi);
        g.rect(0, 15, s, 1).fill(pal.deskShade);
        if (tx % 2 === 0) {
          // 거품 넘치는 맥주잔 둘
          g.rect(3, 3, 4, 4).fill(pal.mug);
          g.rect(3, 2, 4, 1).fill(pal.mugFoam);
          g.rect(7, 4, 1, 2).fill(pal.mug); // 손잡이
          g.rect(9, 4, 4, 3).fill(pal.mug);
          g.rect(9, 3, 4, 1).fill(pal.mugFoam);
        } else {
          // 세워 둔 술통 + 놋 마개
          g.rect(4, 1, 8, 6).fill(pal.barrelWood);
          g.rect(4, 1, 8, 1).fill(pal.barrelTop);
          g.rect(4, 3, 8, 1).fill(pal.barrelHoop);
          g.rect(4, 6, 8, 1).fill(pal.barrelHoop);
          g.rect(12, 4, 2, 2).fill(pal.goblet); // 꼭지
        }
        break;
      }
      case Tile.Table: {
        // 연회 목재 테이블: 카펫 위에 놓이면 바닥도 카펫으로 깔아 융단이
        // 테이블 밑에서 끊겨 보이지 않게 한다.
        const overCarpet = onCarpet(tx - 1, ty) || onCarpet(tx + 1, ty);
        g.rect(0, 0, s, s).fill(overCarpet ? pal.carpetRed : stone);
        g.rect(0, 5, s, 3).fill(pal.feastTop); // 상판
        g.rect(0, 8, s, 3).fill(pal.feastWood); // 앞치마
        g.rect(0, 11, s, 1).fill(pal.deskShade);
        g.rect(2, 12, 3, 4).fill(pal.feastWood); // 다리
        g.rect(11, 12, 3, 4).fill(pal.feastWood);
        g.rect(0, 5, s, 1).fill(pal.cloth); // 식탁보 자락
        if (tx % 2 === 0) {
          g.rect(3, 1, 6, 4).fill(pal.meat); // 구운 고기
          g.rect(3, 1, 6, 1).fill(pal.bread);
          g.rect(10, 2, 3, 3).fill(pal.bread); // 빵
        } else {
          g.rect(4, 2, 3, 3).fill(pal.goblet); // 술잔
          g.rect(4, 5, 3, 1).fill(pal.goblet);
          g.rect(9, 1, 4, 4).fill(pal.bread); // 빵 바구니
          g.rect(9, 1, 4, 1).fill(pal.cloth);
        }
        break;
      }
      case Tile.BossDesk: {
        // 왕좌(세로 1×2): 위 칸이 휘장·높은 등받이, 아래 칸이 좌석·팔걸이·단상.
        const isLower = map.tiles[ty - 1]?.[tx] === Tile.BossDesk;
        g.rect(0, 0, s, s).fill(stone);
        if (!isLower) {
          g.rect(2, 0, 12, 3).fill(pal.bannerCloth); // 머리 위 휘장
          g.rect(2, 0, 12, 1).fill(pal.bannerTrim);
          g.rect(6, 1, 4, 2).fill(pal.throneGoldHi); // 문장
          g.rect(6, 3, 4, 1).fill(pal.throneGoldHi); // 등받이 꼭대기 장식
          g.rect(4, 4, 8, 12).fill(pal.throneGold); // 등받이
          g.rect(4, 4, 8, 1).fill(pal.throneGoldHi);
          g.rect(5, 6, 6, 10).fill(pal.throneCushion); // 등쿠션
          g.rect(5, 6, 6, 1).fill(pal.bannerTrim);
        } else {
          g.rect(3, 0, 10, 5).fill(pal.throneCushion); // 앉는 쿠션
          g.rect(3, 5, 10, 2).fill(pal.throneGold); // 좌판 앞단
          g.rect(3, 5, 10, 1).fill(pal.throneGoldHi);
          g.rect(1, 0, 2, 7).fill(pal.throneGold); // 팔걸이
          g.rect(13, 0, 2, 7).fill(pal.throneGold);
          g.rect(1, 7, 14, 4).fill(pal.daisStone); // 단상 윗단
          g.rect(1, 7, 14, 1).fill(pal.daisStep);
          g.rect(0, 11, s, 5).fill(pal.daisStone); // 단상 아랫단
          g.rect(0, 11, s, 1).fill(pal.daisStep);
          g.rect(0, 15, s, 1).fill(pal.stoneShade); // 바닥 접지 그림자
        }
        break;
      }
    }
  };
}

export const CASTLE_SCENE: SceneDef = {
  id: "castle",
  label: "중세 성",
  map: CASTLE_MAP,
  resolve: (theme) => {
    const mode = sceneColorMode(theme.id);
    return {
      background: adaptColor(CASTLE_BACKGROUND, mode),
      drawTile: castleTileDraw(adaptPalette(CASTLE_PALETTE, mode)),
    };
  },
};
