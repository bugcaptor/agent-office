// src/renderer/office/scenes/beachScene.ts
//
// 바캉스 해변 풍경. 의미 타일은 오피스와 같고(같은 `Tile` enum, 같은 20×14),
// 어휘만 바뀐다:
//   Floor=모래사장, Wall=바다(위 2줄)/모래언덕(그 외), Rug=비치타월,
//   DeskTop=파라솔 아래 작업대, Plant=야자수, Counter=티키 바,
//   Table=비치 테이블, BossDesk=라이프가드 타워.
//
// 팔레트는 "한낮의 원색" 한 벌만 두고, 어두운/모노크롬 테마에서는
// sceneColor.ts가 자동 변환한다.
import type { TileRect } from "../map/mapData";
import { L, Tile, buildSceneMap } from "../map/mapData";
import type { QuietGroup } from "./sceneColor";
import { defineScene } from "./defineScene";
import type { SceneDef, TileDrawFn } from "./sceneTypes";

// 위 2줄은 바다(수평선+파도), ty2/ty5는 좌석 행, ty3/ty6은 파라솔 작업대
// 4쌍씩(= 좌석 8개, 오피스와 동일). 라이프가드 타워는 오피스 보스 책상처럼
// 우측에 세로 1×2로 서고, 줄서기 레인은 그 하단 행을 따라 서쪽으로 뻗는다.
const GRID: Tile[][] = [
  L('WWWWWWWWWWWWWWWWWWWW'), // ty0  - 먼 바다 + 수평선
  L('WWWWWWWWWWWWWWWWWWWW'), // ty1  - 파도 거품이 밀려오는 물가
  L('WFFFFFFFFFFFFFFFFFFW'), // ty2  - 좌석 행 1
  L('WFDDFFDDFFDDFFDDFFPW'), // ty3  - 파라솔 작업대 4쌍 + 야자수(tx18)
  L('WFFFFFFFFFFFFFFFFFFW'), // ty4
  L('WFFFFFFFFFFFFFFFFFFW'), // ty5  - 좌석 행 2
  L('WPDDFFDDFFDDFFDDFFFW'), // ty6  - 야자수(tx1) + 파라솔 작업대 4쌍
  L('WFFFFFFFFFFFFFFFFFFW'), // ty7
  L('WFFFFFFFFFFFFFFFFBFW'), // ty8  - 라이프가드 타워 상단(tx17)
  L('WFFFFFFFFFFFFFFFFBFW'), // ty9  - 타워 하단 + 줄서기 레인
  L('WFRRRRRRRRRRRRRRRRFW'), // ty10 - 비치타월 라운지
  L('WFRRRRRRRTTRRRRRRRPW'), // ty11 - 비치타월 + 비치 테이블 + 야자수(tx18)
  L('WFFCCCCCCFFFFFFFFFFW'), // ty12 - 티키 바
  L('WWWWWWWWWWWWWWWWWWWW'), // ty13 - 모래언덕
];

/** 파라솔 그늘 아래 쉬는 자리 — 오피스 러그 라운지와 같은 역할. */
const BREAK_ROOM: TileRect = { x: 11, y: 10, w: 5, h: 2 };

export const BEACH_MAP = buildSceneMap(GRID, BREAK_ROOM);

/** 바다가 시작되는 행 — 이 위(포함)는 Wall을 바다로, 아래는 모래언덕으로 그린다. */
const SEA_ROWS = 2;

const BEACH_PALETTE_RAW = {
  sandA: 0xf2dfae,
  sandB: 0xe9d19e,
  sandDot: 0xd8bc86,
  shell: 0xfff4e0,
  starfish: 0xf2896a,
  seaDeep: 0x17608f,
  seaHorizon: 0x0f4670,
  seaMid: 0x2a92c9,
  seaFoam: 0xdff4fb,
  dune: 0xdcc08a,
  duneTop: 0xf0daa8,
  duneGrass: 0x77a35a,
  towel: 0xef6f6c,
  towelStripe: 0xfbe8d3,
  canopyA: 0xe8524f,
  canopyB: 0xfbf3e2,
  parasolPole: 0xb98652,
  deskWood: 0xd9a86a,
  deskTop: 0xf0cd94,
  deskShade: 0xa87742,
  laptopLid: 0x4c5a6e,
  laptopBody: 0x36414f,
  palmTrunk: 0xa9743f,
  palmFrond: 0x3f9e5e,
  palmFrondHi: 0x5cbf78,
  coconut: 0x6b4526,
  // 티키 바(탕비실): 황금 짚 + 갈색 통나무는 모래 위에서 유일하게 짙은
  // 덩어리라 시선을 끌어당겼다. 모래와 같은 계열의 흰색으로 올려 모래보다
  // 밝은 쪽으로 분리한다(어둡게가 아니라 밝게 떨어뜨린다).
  tikiThatch: 0xf5eeda,
  tikiThatchHi: 0xfdfaf0,
  tikiWood: 0xdad0bb,
  tikiTop: 0xfbf6ea,
  tikiDeco: 0xc9bfa9,
  tableWood: 0xc98f57,
  tableTop: 0xe8be86,
  towerWood: 0xe0b070,
  towerBeam: 0xa9743f,
  towerFlag: 0xe8524f,
  towerWindow: 0x2f4a63,
};

// 모래사장 잔무늬 죽이기 — 체커 두 톤과 잔물결 자국을 모래 바탕으로 당긴다.
// 바다·타월·파라솔(해변을 해변이게 하는 색)은 그대로 둔다.
const BEACH_QUIET: readonly QuietGroup<typeof BEACH_PALETTE_RAW>[] = [
  { base: "sandA", keys: ["sandB", "sandDot"], amount: 0.6 },
];

/** 캐릭터가 읽히도록 배경 잔무늬를 죽인 실사용 팔레트. */
// 바다는 맵 밖 경치다 — 캐릭터가 밟는 무대가 아니라 그 너머라 진해도 된다.
// 파라솔 천막은 무대 안이지만 해변의 표식이라 절반만 깎아 붉은 기를 남긴다.
const BEACH_KEEP = [
  { keys: ["seaDeep", "seaHorizon", "seaMid", "seaFoam"], amount: 0.05 },
  { keys: ["canopyA", "canopyB"], amount: 0.15 },
] as const;

type BeachPalette = typeof BEACH_PALETTE_RAW;

/** 타일 좌표에서 나오는 결정적 해시 — 조개/불가사리/풀포기를 흩뿌리는 데 쓴다.
 * (베이크된 정적 텍스처라 난수를 쓰면 재베이크마다 무늬가 바뀐다.) */
const scatter = (tx: number, ty: number, mod: number): number => (tx * 73 + ty * 151) % mod;

function beachTileDraw(pal: BeachPalette): TileDrawFn {
  return (g, { t, tx, ty, s, map }) => {
    // 타일 체커 대신 넓은 모래 결을 쓴다. 가구가 있는 칸도 같은 바탕을 받아
    // 해변이 잘린 격자가 아니라 한 장의 백사장으로 읽힌다.
    const sand = ty % 5 === 0 ? pal.sandB : pal.sandA;
    switch (t) {
      case Tile.Floor: {
        g.rect(0, 0, s, s).fill(sand);
        // 세계 좌표 기준의 완만한 바람결. 여섯 칸마다 한 번만 꺾여 반복감이 약하다.
        const ripple = (tx * s + ty * 5) % 72;
        if (ripple < 16) g.rect(0, 4 + (ripple % 3), s, 1).fill(pal.sandDot);
        // 물가가 닿는 첫 바닥 칸에는 이어지는 포말을 얹는다.
        if (ty === SEA_ROWS) {
          // 젖은 모래와 포말 경계는 픽셀마다 완만하게 오르내린다.
          for (let x = 0; x < s; x++) {
            const worldX = tx * s + x;
            const shore = 2 + Math.abs((worldX % 48) - 24) / 12;
            g.rect(x, 0, 1, Math.floor(shore)).fill(pal.sandB);
            if (worldX % 48 < 28) g.rect(x, Math.floor(shore), 1, 1).fill(pal.seaFoam);
          }
        }
        // 장식은 드물게 — 촘촘하면 모래사장이 아니라 잡동사니로 보인다.
        const k = scatter(tx, ty, 23);
        if (k === 0) {
          // 조개: 부채꼴 두 줄
          g.rect(6, 8, 4, 1).fill(pal.shell);
          g.rect(5, 9, 6, 2).fill(pal.shell);
          g.rect(7, 10, 2, 1).fill(pal.sandDot);
        } else if (k === 9) {
          // 불가사리: 십자 + 대각 팔
          g.rect(7, 5, 2, 6).fill(pal.starfish);
          g.rect(4, 7, 8, 2).fill(pal.starfish);
          g.rect(5, 10, 2, 1).fill(pal.starfish);
          g.rect(9, 10, 2, 1).fill(pal.starfish);
        }
        break;
      }
      case Tile.Wall: {
        if (ty < SEA_ROWS) {
          const nearShore = ty === SEA_ROWS - 1;
          g.rect(0, 0, s, s).fill(nearShore ? pal.seaMid : pal.seaDeep);
          if (!nearShore) {
            g.rect(0, 0, s, 3).fill(pal.seaHorizon); // 수평선 쪽이 더 짙다
            // 물결선도 전역 x의 완만한 굽이로 이어진다.
            for (let x = 0; x < s; x++) {
              const worldX = tx * s + x;
              const waveY = 8 + Math.floor(Math.abs((worldX % 56) - 28) / 14);
              g.rect(x, waveY, 1, 1).fill(pal.seaMid);
              if (worldX % 72 < 34) g.rect(x, 13 - Math.floor(Math.abs((worldX % 40) - 20) / 10), 1, 1).fill(pal.seaMid);
            }
          } else {
            // 밀려오는 파도 거품: 아래쪽 두 줄 + 1px 물보라
            for (let x = 0; x < s; x++) {
              const worldX = tx * s + x;
              const crestY = 10 + Math.floor(Math.abs((worldX % 64) - 32) / 16);
              g.rect(x, crestY, 1, s - crestY).fill(pal.seaFoam);
              if (worldX % 64 < 40) g.rect(x, crestY - 1, 1, 1).fill(pal.seaFoam);
            }
          }
          break;
        }
        // 모래언덕(측면·하단): 밝은 능선 + 사초 포기
        g.rect(0, 0, s, s).fill(pal.dune);
        for (let x = 0; x < s; x++) {
          const worldX = tx * s + x;
          const duneLip = 3 + Math.floor(Math.abs((worldX % 96) - 48) / 24);
          g.rect(x, 0, 1, duneLip).fill(pal.duneTop);
          g.rect(x, duneLip, 1, 1).fill(pal.sandDot);
        }
        if (scatter(tx, ty, 3) === 0) {
          g.rect(6, 5, 1, 5).fill(pal.duneGrass);
          g.rect(8, 6, 1, 4).fill(pal.duneGrass);
          g.rect(10, 4, 1, 6).fill(pal.duneGrass);
        }
        break;
      }
      case Tile.Rug: {
        // 하나의 긴 비치타월: 타일마다 되풀이하던 줄무늬 대신 넓은 세로 띠.
        g.rect(0, 0, s, s).fill(pal.towel);
        if ((tx - 2) % 6 < 2) g.rect(6, 0, 4, s).fill(pal.towelStripe);
        if (map.tiles[ty - 1]?.[tx] !== Tile.Rug) g.rect(0, 0, s, 1).fill(pal.towelStripe);
        if (map.tiles[ty + 1]?.[tx] !== Tile.Rug) g.rect(0, s - 1, s, 1).fill(pal.towelStripe);
        break;
      }
      case Tile.DeskTop: {
        const isLeft = map.tiles[ty][tx - 1] !== Tile.DeskTop;
        g.rect(0, 0, s, s).fill(sand); // 가구 타일은 바닥 베이크에서 빠지므로 스스로 모래를 깐다
        // 파라솔 캐노피: 4px 줄무늬. 줄 위상을 tx로 이어 붙여 2칸짜리 쌍이
        // 하나의 파라솔로 읽히게 한다(칸 단위로 통짜 색을 칠하면 홍백 블록처럼 보인다).
        g.rect(0, 0, s, 4).fill(pal.canopyB);
        for (let x = tx % 2 === 0 ? 0 : 4; x < s; x += 8) {
          g.rect(x, 0, 4, 4).fill(pal.canopyA);
        }
        g.rect(0, 0, s, 1).fill(pal.canopyB); // 꼭대기 하이라이트
        g.rect(0, 4, s, 1).fill(pal.deskShade); // 캐노피 그림자 끝단
        if (isLeft) g.rect(s - 1, 4, 1, 8).fill(pal.parasolPole); // 기둥은 쌍의 이음매에
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
        }
        break;
      }
      case Tile.Plant: {
        // 야자수: 굽은 줄기 + 사방으로 뻗은 잎 + 코코넛
        g.rect(0, 0, s, s).fill(sand);
        g.rect(7, 6, 2, 10).fill(pal.palmTrunk);
        g.rect(6, 11, 1, 5).fill(pal.palmTrunk);
        g.rect(3, 3, 10, 2).fill(pal.palmFrond);
        g.rect(5, 1, 6, 2).fill(pal.palmFrondHi);
        g.rect(1, 5, 4, 1).fill(pal.palmFrond);
        g.rect(11, 5, 4, 1).fill(pal.palmFrond);
        g.rect(2, 6, 2, 1).fill(pal.palmFrondHi);
        g.rect(12, 6, 2, 1).fill(pal.palmFrondHi);
        g.rect(6, 5, 2, 2).fill(pal.coconut);
        g.rect(9, 5, 1, 1).fill(pal.coconut);
        break;
      }
      case Tile.Counter: {
        // 티키 바: 짚 지붕 + 통나무 카운터. 칸마다 유리잔/파인애플을 번갈아.
        g.rect(0, 0, s, s).fill(sand);
        g.rect(0, 0, s, 4).fill(pal.tikiThatch);
        g.rect(0, 0, s, 1).fill(pal.tikiThatchHi);
        g.rect(0, 4, s, 1).fill(pal.tikiWood); // 지붕 처마
        g.rect(0, 8, s, 6).fill(pal.tikiWood);
        g.rect(0, 8, s, 2).fill(pal.tikiTop); // 바 상판
        g.rect(0, 14, s, 2).fill(pal.tikiWood);
        if (tx % 2 === 0) {
          g.rect(5, 5, 1, 3).fill(pal.tikiWood); // 지지 기둥
          g.rect(6, 5, 4, 3).fill(pal.tikiDeco); // 칵테일 잔
          g.rect(7, 4, 1, 1).fill(pal.towelStripe); // 우산 장식
        } else {
          g.rect(10, 5, 1, 3).fill(pal.tikiWood);
          g.rect(4, 5, 3, 3).fill(pal.palmFrond); // 파인애플 잎
          g.rect(4, 6, 3, 2).fill(pal.tikiThatchHi);
        }
        break;
      }
      case Tile.Table: {
        // 비치 테이블: 낮은 상판 + 벌어진 다리
        g.rect(0, 0, s, s).fill(sand);
        g.rect(1, 5, 14, 3).fill(pal.tableTop);
        g.rect(1, 8, 14, 2).fill(pal.tableWood);
        g.rect(3, 10, 2, 4).fill(pal.tableWood);
        g.rect(11, 10, 2, 4).fill(pal.tableWood);
        g.rect(2, 14, 3, 1).fill(pal.deskShade);
        g.rect(11, 14, 3, 1).fill(pal.deskShade);
        break;
      }
      case Tile.BossDesk: {
        // 라이프가드 타워(세로 1×2): 위 칸이 망대, 아래 칸이 다리·사다리.
        const isLower = map.tiles[ty - 1]?.[tx] === Tile.BossDesk;
        g.rect(0, 0, s, s).fill(sand);
        if (!isLower) {
          g.rect(8, 0, 1, 4).fill(pal.towerBeam); // 깃대
          g.rect(9, 0, 5, 3).fill(pal.towerFlag); // 깃발
          g.rect(1, 4, 14, 2).fill(pal.towerBeam); // 지붕
          g.rect(2, 6, 12, 9).fill(pal.towerWood); // 망대 벽
          g.rect(3, 8, 10, 4).fill(pal.towerWindow); // 창
          g.rect(2, 13, 12, 2).fill(pal.towerBeam); // 난간
        } else {
          g.rect(3, 0, 2, 14).fill(pal.towerBeam); // 다리
          g.rect(11, 0, 2, 14).fill(pal.towerBeam);
          g.rect(5, 2, 6, 1).fill(pal.towerWood); // 사다리 발판
          g.rect(5, 6, 6, 1).fill(pal.towerWood);
          g.rect(5, 10, 6, 1).fill(pal.towerWood);
          g.rect(2, 14, 12, 1).fill(pal.deskShade); // 모래에 박힌 그림자
        }
        break;
      }
    }
  };
}

export const BEACH_SCENE: SceneDef = defineScene({
  id: "beach",
  labelKey: "office:scene.beach",
  map: BEACH_MAP,
  raw: BEACH_PALETTE_RAW,
  quiet: BEACH_QUIET,
  keep: BEACH_KEEP,
  /** 레터박스(맵 밖) 배경 — 모래보다 한 단계 어둡게 해 맵이 떠 보이게 한다. */
  background: 0xc9ae83,
  draw: beachTileDraw,
});
