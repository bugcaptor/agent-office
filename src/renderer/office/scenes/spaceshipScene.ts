// src/renderer/office/scenes/spaceshipScene.ts
//
// 우주선 내부 풍경. 해변/계곡과 같은 규칙(의미 타일은 오피스와 공유하고
// 어휘만 교체):
//   Floor=금속 데크 패널, Wall=창밖 우주(위 2줄)/선체 격벽(그 외),
//   Rug=홀로그램 라운지 패드, DeskTop=조종 콘솔 스테이션,
//   Plant=수경재배 포드, Counter=보급·급식 카운터, Table=홀로테이블,
//   BossDesk=함장석(브리지 콘솔).
//
// 레이아웃은 해변/계곡과 일부러 다르게 잡았다 — 함장석이 맵 **위쪽 한가운데**
// (뷰포트를 등지는 브리지 자리)에 서고, 줄서기 레인은 그 하단 행을 따라
// 서쪽으로 뻗는다(`buildSceneMap`이 보스 위치에서 유도한다). 콘솔 스테이션은
// 두 행이 서로 엇갈리게 배치돼 가운데 통로가 지그재그로 열린다.
//
// 팔레트는 "강철 + 청록 발광" 한 벌만 두고, 어두운/모노크롬 테마에서는
// sceneColor.ts가 자동 변환한다.
import type { TileRect } from "../map/mapData";
import { L, Tile, buildSceneMap } from "../map/mapData";
import { adaptColor, adaptPalette, sceneColorMode } from "./sceneColor";
import type { SceneDef, TileDrawFn } from "./sceneTypes";

// 위 2줄은 뷰포트(창밖 우주), ty4/ty7이 좌석 행, ty5/ty8이 콘솔 스테이션
// 4쌍씩(= 좌석 8개, 오피스와 동일). 함장석은 세로 1×2로 tx10에 서고,
// 줄서기 레인은 그 하단 행(ty3)을 따라 서쪽(tx9→tx2)으로 뻗는다.
const GRID: Tile[][] = [
  L('WWWWWWWWWWWWWWWWWWWW'), // ty0  - 뷰포트: 먼 우주 + 행성
  L('WWWWWWWWWWWWWWWWWWWW'), // ty1  - 뷰포트 하단 + 창틀
  L('WFFFFFFFFFBFFFFFFFFW'), // ty2  - 함장석 상단(tx10) = 브리지 콘솔 뱅크
  L('WPFFFFFFFFBFFFFFFFPW'), // ty3  - 함장석 하단 + 줄서기 레인(서쪽) + 수경 포드
  L('WFFFFFFFFFFFFFFFFFFW'), // ty4  - 좌석 행 1
  L('WFDDFFDDFFFFDDFFDDFW'), // ty5  - 콘솔 스테이션 4쌍(가운데 통로 tx8~11)
  L('WFFFFFFFFFFFFFFFFFFW'), // ty6  - 통로
  L('WFFFFFFFFFFFFFFFFFFW'), // ty7  - 좌석 행 2
  L('WFFDDFFDDFFDDFFDDFFW'), // ty8  - 콘솔 스테이션 4쌍(위 행과 한 칸 엇갈림)
  L('WFPFFFFFFFFFFFFFFPFW'), // ty9  - 라운지 진입부 + 수경 포드
  L('WFRRRRRRRRRRRRRRRRFW'), // ty10 - 홀로그램 라운지 패드
  L('WFRRRTTRRRRRRRRRRRFW'), // ty11 - 패드 + 홀로테이블(2칸)
  L('WFFFFFFFFFFCCCCCCFFW'), // ty12 - 보급·급식 카운터
  L('WWWWWWWWWWWWWWWWWWWW'), // ty13 - 선미 격벽
];

/** 홀로그램 패드를 깐 승무원 라운지 — 오피스 러그 라운지와 같은 역할.
 * 홀로테이블(tx5~6)을 피해 패드 오른쪽에 잡는다. */
const BREAK_ROOM: TileRect = { x: 12, y: 10, w: 5, h: 2 };

export const SPACESHIP_MAP = buildSceneMap(GRID, BREAK_ROOM);

/** 뷰포트(창밖 우주)로 그리는 위쪽 행 수. 그보다 아래의 Wall은 선체 격벽. */
const VIEW_ROWS = 2;

// 뷰포트에 걸치는 행성 — 2×2 타일(tx14~15 × ty0~1) 한 덩어리로 그린다.
// 칸마다 통짜 원을 그리면 행성이 네 개로 보이므로, 32×32 블록 좌표계에서
// 원을 자른 뒤 각 칸이 자기 몫만 그리는 방식.
const PLANET_TX = 14;
const PLANET_TY = 0;
const PLANET_R = 13;

// 홀로테이블 위에 뜨는 구(球)의 행별 반폭(8줄). 쌍의 이음매에 중심을 두어
// 왼쪽 칸이 오른쪽 절반을, 오른쪽 칸이 왼쪽 절반을 그리면 구 하나가 된다.
const HOLO_SPHERE = [2, 3, 4, 4, 4, 4, 3, 2] as const;

const SPACESHIP_PALETTE = {
  deckA: 0x5a6472,
  deckB: 0x515b69,
  deckSeam: 0x3d4552,
  deckStain: 0x454e5b,
  rivet: 0x7d8797,
  hatch: 0x6e7887,
  guideLamp: 0x4fd8d0,
  spaceVoid: 0x0a0f1e,
  spaceVoidLo: 0x0d1426,
  nebula: 0x2a2350,
  star: 0xf2f6ff,
  starDim: 0x9fb4d8,
  planet: 0x3f7fa8,
  planetBand: 0x64a8c6,
  planetShade: 0x24506e,
  frame: 0x3a4350,
  frameHi: 0x6c7788,
  bulkhead: 0x49525f,
  bulkheadHi: 0x616c7b,
  bulkheadShade: 0x333b46,
  warnStripe: 0xe0b23c,
  warnDark: 0x2c333d,
  holoPad: 0x123241,
  holoGrid: 0x2f8fa0,
  holoGlow: 0x7ff0e4,
  consoleBody: 0x4a5464,
  consoleTop: 0x6a7484,
  consoleShade: 0x2f3743,
  screen: 0x0f2c3a,
  screenGlow: 0x5fe0c8,
  btnRed: 0xe0563f,
  btnGreen: 0x62d06a,
  btnAmber: 0xe6b23f,
  podFrame: 0x6a7484,
  podFrameHi: 0x8b95a5,
  podGlass: 0x2f6f7e,
  podGlassHi: 0x8fd8de,
  podLeaf: 0x3f9e5e,
  podLeafHi: 0x63c47c,
  podStem: 0x2f7346,
  podFluid: 0x59c9d8,
  rackBody: 0x3f4854,
  rackShade: 0x2b323c,
  crate: 0xa8763f,
  crateHi: 0xc9974f,
  counterBody: 0x59636f,
  counterTop: 0x818c99,
  counterShade: 0x363e49,
  tray: 0xd6dce4,
  dispenser: 0x9aa4b2,
  captainSeat: 0x3a4250,
  captainSeatHi: 0x59647a,
  captainRail: 0x7d8797,
  captainScreen: 0x0e2f42,
};

type SpaceshipPalette = typeof SPACESHIP_PALETTE;

/** 레터박스(맵 밖) 배경 — 데크보다 어두운 선체 그늘색. */
const SPACESHIP_BACKGROUND = 0x141a24;

/** 결정적 흩뿌리기(별·패널 얼룩·경고 스트라이프). 베이크된 정적 텍스처라
 * 난수를 쓰면 재베이크마다 무늬가 바뀐다 — 해변/계곡과 같은 이유, 계수만 다르다. */
const scatter = (tx: number, ty: number, mod: number): number => (tx * 131 + ty * 59) % mod;

function spaceshipTileDraw(pal: SpaceshipPalette): TileDrawFn {
  return (g, { t, tx, ty, s, map }) => {
    // 데크 패널은 체커로 깐다 — 오피스 바닥과 같은 이디엄이라 패널 이음선이
    // 칸 경계와 맞아떨어져 보인다.
    const deck = (tx + ty) % 2 === 0 ? pal.deckA : pal.deckB;
    switch (t) {
      case Tile.Floor: {
        g.rect(0, 0, s, s).fill(deck);
        // 패널 이음선(위·왼쪽 1px) + 네 귀퉁이 리벳
        g.rect(0, 0, s, 1).fill(pal.deckSeam);
        g.rect(0, 0, 1, s).fill(pal.deckSeam);
        g.rect(2, 2, 1, 1).fill(pal.rivet);
        g.rect(s - 3, 2, 1, 1).fill(pal.rivet);
        g.rect(2, s - 3, 1, 1).fill(pal.rivet);
        g.rect(s - 3, s - 3, 1, 1).fill(pal.rivet);
        // 장식은 드물게 — 촘촘하면 격납고가 아니라 고물상으로 보인다.
        const k = scatter(tx, ty, 17);
        if (k === 0) {
          // 점검 해치: 볼트 링 두른 사각 뚜껑
          g.rect(4, 5, 8, 7).fill(pal.hatch);
          g.rect(4, 5, 8, 1).fill(pal.rivet);
          g.rect(5, 6, 6, 5).fill(pal.deckStain);
          g.rect(5, 6, 1, 1).fill(pal.rivet);
          g.rect(10, 6, 1, 1).fill(pal.rivet);
          g.rect(5, 10, 1, 1).fill(pal.rivet);
          g.rect(10, 10, 1, 1).fill(pal.rivet);
        } else if (k === 6) {
          // 유도등 스트립: 데크에 박힌 발광 표시선
          g.rect(3, 8, 10, 1).fill(pal.deckSeam);
          g.rect(4, 8, 3, 1).fill(pal.guideLamp);
          g.rect(9, 8, 3, 1).fill(pal.guideLamp);
        } else if (k === 12) {
          // 오래된 그리스 얼룩
          g.rect(5, 4, 5, 2).fill(pal.deckStain);
          g.rect(4, 6, 3, 1).fill(pal.deckStain);
          g.rect(9, 10, 4, 2).fill(pal.deckStain);
        }
        break;
      }
      case Tile.Wall: {
        // 위 2줄이라도 좌우 끝 칸은 창이 아니라 선체 기둥이다 — 뷰포트가
        // 맵 가장자리까지 뚫려 있으면 배가 아니라 하늘처럼 보인다.
        const isViewport = ty < VIEW_ROWS && tx > 0 && tx < map.width - 1;
        if (isViewport) {
          g.rect(0, 0, s, s).fill(ty === 0 ? pal.spaceVoid : pal.spaceVoidLo);
          // 성운 얼룩 — 아주 가끔, 옅게
          if (scatter(tx, ty, 7) === 0) {
            g.rect(2, 4, 9, 5).fill(pal.nebula);
            g.rect(4, 3, 5, 1).fill(pal.nebula);
            g.rect(3, 9, 6, 1).fill(pal.nebula);
          }
          // 별 두 점(밝은 것 하나, 흐린 것 하나) — 좌표까지 해시로 결정한다
          const k = scatter(tx, ty, 11);
          g.rect((k * 5) % 13 + 1, (k * 3) % 13 + 1, 1, 1).fill(pal.star);
          const k2 = scatter(tx + 5, ty + 3, 9);
          g.rect((k2 * 7) % 12 + 2, (k2 * 2) % 12 + 2, 1, 1).fill(pal.starDim);
          if (k === 4) {
            // 큰 별: 십자 반짝임
            g.rect(6, 5, 3, 1).fill(pal.star);
            g.rect(7, 4, 1, 3).fill(pal.star);
          }
          // 행성: 2×2 블록 좌표계(32×32, 중심 16,16)에서 자기 몫만 잘라 그린다
          if (tx >= PLANET_TX && tx <= PLANET_TX + 1 && ty >= PLANET_TY && ty <= PLANET_TY + 1) {
            const ox = (tx - PLANET_TX) * s;
            const oy = (ty - PLANET_TY) * s;
            for (let y = 0; y < s; y++) {
              const dy = oy + y + 0.5 - 16;
              if (Math.abs(dy) >= PLANET_R) continue;
              const hw = Math.sqrt(PLANET_R * PLANET_R - dy * dy);
              const x0 = Math.max(Math.round(16 - hw), ox);
              const x1 = Math.min(Math.round(16 + hw), ox + s);
              if (x1 <= x0) continue;
              g.rect(x0 - ox, y, x1 - x0, 1).fill(pal.planet);
              // 구름 띠 두 줄 — 원반 왼쪽 60%만 덮어 자전하는 결을 준다
              const gy = oy + y;
              if (gy === 11 || gy === 12 || gy === 21) {
                const bx1 = Math.min(x1, Math.round(16 - hw + hw * 1.2));
                if (bx1 > x0) g.rect(x0 - ox, y, bx1 - x0, 1).fill(pal.planetBand);
              }
              // 명암 경계 — 행별 반폭에 비례해 휘므로 구처럼 보인다
              const term = Math.max(x0, Math.round(16 + hw * 0.35));
              if (x1 > term) g.rect(term - ox, y, x1 - term, 1).fill(pal.planetShade);
            }
          }
          // 창틀은 맨 위에 얹는다(별·행성을 가로지르는 세로 멀리언 + 하단 프레임).
          // 행성이 앉은 tx14~15는 5칸 주기에서 비껴 있어 가려지지 않는다.
          if (tx % 6 === 4) {
            g.rect(7, 0, 2, s).fill(pal.frame);
            g.rect(7, 0, 1, s).fill(pal.frameHi);
          }
          if (ty === 0) g.rect(0, 0, s, 2).fill(pal.frame);
          if (ty === VIEW_ROWS - 1) {
            g.rect(0, s - 3, s, 3).fill(pal.frame);
            g.rect(0, s - 3, s, 1).fill(pal.frameHi);
          }
          break;
        }
        // 선체 격벽: 가로 리브 + 리벳, 가끔 경고 스트라이프
        g.rect(0, 0, s, s).fill(pal.bulkhead);
        g.rect(0, 0, s, 2).fill(pal.bulkheadHi);
        g.rect(0, s - 2, s, 2).fill(pal.bulkheadShade);
        g.rect(0, 5, s, 1).fill(pal.bulkheadShade);
        g.rect(0, 12, s, 1).fill(pal.bulkheadShade);
        g.rect(2, 3, 1, 1).fill(pal.rivet);
        g.rect(s - 3, 3, 1, 1).fill(pal.rivet);
        g.rect(2, 10, 1, 1).fill(pal.rivet);
        g.rect(s - 3, 10, 1, 1).fill(pal.rivet);
        if (scatter(tx, ty, 4) === 0) {
          // 경고 스트라이프: 검은 띠 위에 노란 사선 세 개(전부 칸 안쪽에서 끝난다)
          g.rect(0, 6, s, 6).fill(pal.warnDark);
          for (let i = 0; i < s; i += 6) {
            g.rect(i, 7, 2, 1).fill(pal.warnStripe);
            g.rect(i + 1, 8, 2, 1).fill(pal.warnStripe);
            g.rect(i + 2, 9, 2, 1).fill(pal.warnStripe);
          }
        }
        break;
      }
      case Tile.Rug: {
        // 홀로그램 라운지 패드: 어두운 유리판 위에 발광 격자
        g.rect(0, 0, s, s).fill(pal.holoPad);
        g.rect(0, 0, s, 1).fill(pal.holoGrid); // 칸 경계가 격자선이 된다
        g.rect(0, 0, 1, s).fill(pal.holoGrid);
        g.rect(0, 8, s, 1).fill(pal.holoGrid); // 칸 안쪽 반 칸 격자
        g.rect(8, 0, 1, s).fill(pal.holoGrid);
        // 교차점 발광 — 칸마다 자리를 바꿔 패드 전체가 은은하게 깜빡이는 결
        if ((tx + ty) % 2 === 0) {
          g.rect(8, 8, 1, 1).fill(pal.holoGlow);
          g.rect(3, 3, 2, 1).fill(pal.holoGlow);
        } else {
          g.rect(0, 0, 1, 1).fill(pal.holoGlow);
          g.rect(11, 12, 2, 1).fill(pal.holoGlow);
        }
        break;
      }
      case Tile.DeskTop: {
        const isLeft = map.tiles[ty][tx - 1] !== Tile.DeskTop;
        g.rect(0, 0, s, s).fill(deck); // 가구 타일은 바닥 베이크에서 빠지므로 스스로 데크를 깐다
        // 스크린 뱅크(뒤로 젖혀진 계기판). 화면을 쌍의 바깥쪽 끝까지만 채우고
        // 이음매는 비우지 않아, 2칸이 하나의 콘솔로 이어져 읽힌다.
        g.rect(0, 3, s, 7).fill(pal.consoleBody);
        g.rect(0, 3, s, 1).fill(pal.consoleTop);
        g.rect(0, 9, s, 1).fill(pal.consoleShade);
        const sx = isLeft ? 2 : 0;
        g.rect(sx, 4, s - 2, 4).fill(pal.screen);
        if (isLeft) {
          // 항법 파형
          g.rect(sx + 1, 6, 3, 1).fill(pal.screenGlow);
          g.rect(sx + 4, 5, 2, 1).fill(pal.screenGlow);
          g.rect(sx + 6, 7, 3, 1).fill(pal.screenGlow);
          g.rect(sx + 9, 5, 3, 1).fill(pal.screenGlow);
        } else {
          // 궤도 스코프 + 표적
          g.rect(sx + 3, 4, 6, 1).fill(pal.screenGlow);
          g.rect(sx + 2, 5, 1, 2).fill(pal.screenGlow);
          g.rect(sx + 9, 5, 1, 2).fill(pal.screenGlow);
          g.rect(sx + 3, 7, 6, 1).fill(pal.screenGlow);
          g.rect(sx + 5, 5, 2, 2).fill(pal.btnAmber);
        }
        // 조작대 상판 + 버튼 불빛
        g.rect(0, 10, s, 5).fill(pal.consoleBody);
        g.rect(0, 10, s, 2).fill(pal.consoleTop);
        g.rect(0, 15, s, 1).fill(pal.consoleShade);
        g.rect(2, 13, 10, 1).fill(pal.consoleShade); // 키패드 홈
        g.rect(2, 12, 2, 1).fill(isLeft ? pal.btnGreen : pal.btnRed);
        g.rect(6, 12, 2, 1).fill(pal.btnAmber);
        g.rect(10, 12, 2, 1).fill(isLeft ? pal.btnRed : pal.btnGreen);
        break;
      }
      case Tile.Plant: {
        // 수경재배 포드: 금속 받침 + 유리 돔 + 배양액에 뜬 식물
        g.rect(0, 0, s, s).fill(deck);
        g.rect(3, 12, 10, 3).fill(pal.podFrame);
        g.rect(3, 12, 10, 1).fill(pal.podFrameHi);
        g.rect(2, 15, 12, 1).fill(pal.consoleShade); // 데크에 닿는 그림자
        g.rect(3, 3, 10, 9).fill(pal.podGlass); // 돔
        g.rect(4, 2, 8, 1).fill(pal.podGlass);
        g.rect(3, 3, 1, 9).fill(pal.podGlassHi); // 유리 하이라이트
        g.rect(5, 2, 3, 1).fill(pal.podGlassHi);
        // 열 기준으로 번갈아 — 이 맵의 포드 네 자리가 다 같은 모습이 되지 않게
        // (합 기준이면 나란한 자리끼리 같은 쪽으로 떨어진다).
        if (tx % 2 === 0) {
          g.rect(7, 7, 2, 5).fill(pal.podStem); // 다 자란 포기
          g.rect(4, 6, 3, 2).fill(pal.podLeaf);
          g.rect(9, 5, 3, 2).fill(pal.podLeaf);
          g.rect(6, 4, 4, 2).fill(pal.podLeafHi);
        } else {
          g.rect(7, 8, 2, 4).fill(pal.podStem); // 막 올라온 새싹
          g.rect(5, 7, 2, 1).fill(pal.podLeaf);
          g.rect(9, 6, 2, 1).fill(pal.podLeaf);
          g.rect(6, 5, 4, 2).fill(pal.podLeafHi);
        }
        g.rect(4, 11, 8, 1).fill(pal.podFluid); // 배양액 수면
        break;
      }
      case Tile.Counter: {
        // 보급·급식 카운터: 뒷벽 랙 + 배식대. 칸마다 보급 상자/식판을 번갈아.
        g.rect(0, 0, s, s).fill(deck);
        g.rect(0, 0, s, 5).fill(pal.rackBody);
        g.rect(0, 4, s, 1).fill(pal.rackShade);
        if (tx % 2 === 0) {
          g.rect(2, 1, 5, 3).fill(pal.crate); // 보급 상자
          g.rect(2, 1, 5, 1).fill(pal.crateHi);
          g.rect(9, 2, 4, 2).fill(pal.crate);
          g.rect(9, 2, 4, 1).fill(pal.crateHi);
        } else {
          g.rect(3, 1, 3, 3).fill(pal.dispenser); // 음료 디스펜서
          g.rect(3, 1, 3, 1).fill(pal.podFluid);
          g.rect(9, 1, 4, 3).fill(pal.rackShade); // 보관함
          g.rect(10, 2, 2, 1).fill(pal.btnGreen);
        }
        g.rect(0, 8, s, 6).fill(pal.counterBody); // 배식대
        g.rect(0, 8, s, 2).fill(pal.counterTop);
        g.rect(0, 14, s, 2).fill(pal.counterShade);
        if (tx % 2 === 0) {
          g.rect(4, 6, 8, 2).fill(pal.tray); // 상판에 얹힌 식판
          g.rect(5, 6, 2, 1).fill(pal.crateHi);
        } else {
          g.rect(3, 6, 4, 2).fill(pal.dispenser); // 보온 통
          g.rect(9, 7, 4, 1).fill(pal.btnAmber); // 배식 표시등
        }
        break;
      }
      case Tile.Table: {
        // 홀로테이블: 낮은 슬래브 + 이음매 위에 뜬 행성 홀로그램.
        const isLeft = map.tiles[ty][tx - 1] !== Tile.Table;
        g.rect(0, 0, s, s).fill(deck);
        // 구는 슬래브 위(y0~7)에 뜬다 — 쌍의 두 칸이 각각 절반씩 그린다.
        for (let i = 0; i < HOLO_SPHERE.length; i++) {
          const w = HOLO_SPHERE[i];
          g.rect(isLeft ? s - w : 0, i, w, 1).fill(pal.holoGrid);
        }
        g.rect(isLeft ? s - 6 : 0, 4, 6, 1).fill(pal.holoGlow); // 구를 두른 궤도 링
        g.rect(0, 8, s, 3).fill(pal.counterTop); // 슬래브 상판
        g.rect(isLeft ? s - 4 : 0, 8, 4, 1).fill(pal.holoGlow); // 투사기 발광
        g.rect(0, 11, s, 2).fill(pal.consoleBody);
        g.rect(0, 13, s, 1).fill(pal.consoleShade);
        g.rect(isLeft ? 3 : 10, 13, 3, 3).fill(pal.consoleBody); // 다리
        g.rect(isLeft ? 2 : 9, 15, 5, 1).fill(pal.consoleShade);
        break;
      }
      case Tile.BossDesk: {
        // 함장석(세로 1×2): 위 칸이 브리지 콘솔 뱅크, 아래 칸이 함장 좌석.
        const isLower = map.tiles[ty - 1]?.[tx] === Tile.BossDesk;
        g.rect(0, 0, s, s).fill(deck);
        if (!isLower) {
          g.rect(7, 0, 2, 2).fill(pal.captainRail); // 통신 마스트
          g.rect(1, 2, 14, 3).fill(pal.captainRail); // 상단 캐노피
          g.rect(1, 2, 14, 1).fill(pal.consoleTop);
          g.rect(2, 5, 12, 8).fill(pal.consoleBody); // 콘솔 뱅크
          g.rect(3, 6, 10, 5).fill(pal.captainScreen); // 메인 스크린
          g.rect(4, 7, 8, 1).fill(pal.screenGlow);
          g.rect(4, 9, 5, 1).fill(pal.screenGlow);
          g.rect(10, 9, 2, 1).fill(pal.btnAmber);
          g.rect(3, 11, 3, 1).fill(pal.btnGreen); // 상태등 열
          g.rect(7, 11, 2, 1).fill(pal.btnAmber);
          g.rect(10, 11, 3, 1).fill(pal.btnRed);
          g.rect(2, 13, 12, 2).fill(pal.consoleTop); // 콘솔 앞 턱
        } else {
          g.rect(4, 0, 8, 7).fill(pal.captainSeat); // 등받이
          g.rect(5, 0, 6, 2).fill(pal.captainSeatHi); // 헤드레스트
          g.rect(2, 4, 2, 5).fill(pal.captainSeat); // 팔걸이
          g.rect(12, 4, 2, 5).fill(pal.captainSeat);
          g.rect(2, 4, 2, 1).fill(pal.btnGreen); // 팔걸이 조작 패널
          g.rect(12, 4, 2, 1).fill(pal.btnRed);
          g.rect(4, 7, 8, 4).fill(pal.captainSeatHi); // 좌면
          g.rect(6, 11, 4, 3).fill(pal.captainRail); // 지지 기둥
          g.rect(3, 14, 10, 2).fill(pal.consoleShade); // 바닥 원판 그림자
        }
        break;
      }
    }
  };
}

export const SPACESHIP_SCENE: SceneDef = {
  id: "spaceship",
  label: "우주선",
  map: SPACESHIP_MAP,
  resolve: (theme) => {
    const mode = sceneColorMode(theme.id);
    return {
      background: adaptColor(SPACESHIP_BACKGROUND, mode),
      drawTile: spaceshipTileDraw(adaptPalette(SPACESHIP_PALETTE, mode)),
    };
  },
};
