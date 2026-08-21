// @vitest-environment jsdom
//
// src/renderer/office/scenes/__tests__/scenes.test.ts
//
// 풍경(scene) 축 단위 테스트:
// - 레지스트리 무결성: 3씬 모두 같은 의미 타일 계약(20×14, 좌석/라운지/보스/
//   줄 슬롯)을 지키는가 — 이게 깨지면 FSM·좌석배정·줄서기가 조용히 오작동한다
// - 씬×테마 전 조합에서 타일 드로잉이 예외 없이 도는가
// - sceneColor: 순수성·결정성·모드별 방향성
// - sceneStorage: 라운드트립 + 오염값 폴백
//
// `Graphics` 생성/드로우는 렌더 컨텍스트가 필요 없어 여기서 그대로 돌린다
// (TileRenderer.test.ts와 같은 판단). jsdom은 localStorage 스텁 때문에 쓴다.

import { afterEach, describe, expect, it, vi } from "vitest";
import { Graphics } from "pixi.js";

import { DEFAULT_SCENE_ID, SCENES, SCENE_ORDER, isSceneId, nextSceneId } from "../scenes";
import { adaptColor, adaptPalette, sceneColorMode } from "../sceneColor";
import { OFFICE_MAP, Tile } from "../../map/mapData";
import { isWalkable } from "../../world/pathing";
import { THEMES, THEME_ORDER } from "../../../theme/themes";
import type { SceneDef } from "../sceneTypes";

// theme.test.ts와 같은 이유: 이 프로젝트의 jsdom 환경은 localStorage 전역을
// 노출하지 않는다. 모듈 최상단에서 스텁해야 동적 import도 같은 스텁을 본다.
const storage = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (k: string) => storage.get(k) ?? null,
  setItem: (k: string, v: string) => void storage.set(k, String(v)),
  removeItem: (k: string) => void storage.delete(k),
  clear: () => storage.clear(),
});

afterEach(() => {
  localStorage.clear();
});

const ALL_SCENES: SceneDef[] = Object.values(SCENES);

describe("SCENES 레지스트리 무결성", () => {
  it("SCENE_ORDER가 레지스트리의 모든 풍경을 정확히 한 번씩 순회한다", () => {
    expect([...SCENE_ORDER].sort()).toEqual(Object.keys(SCENES).sort());
    expect(new Set(SCENE_ORDER).size).toBe(SCENE_ORDER.length);
    expect(SCENE_ORDER[0]).toBe(DEFAULT_SCENE_ID);
  });

  it("모든 풍경이 id 일치 + 비어있지 않은 한국어 라벨을 가진다", () => {
    for (const [id, scene] of Object.entries(SCENES)) {
      expect(scene.id).toBe(id);
      expect(scene.label.trim().length).toBeGreaterThan(0);
    }
  });

  it("isSceneId / nextSceneId", () => {
    expect(isSceneId("beach")).toBe(true);
    expect(isSceneId("bogus")).toBe(false);
    expect(isSceneId(null)).toBe(false);
    expect(nextSceneId("office")).toBe("beach");
    expect(nextSceneId(SCENE_ORDER[SCENE_ORDER.length - 1])).toBe(SCENE_ORDER[0]);
  });

  it.each(ALL_SCENES.map((s) => [s.id, s] as const))("[%s] 맵이 20×14이고 행 길이가 균일하다", (_id, scene) => {
    expect(scene.map.width).toBe(OFFICE_MAP.width);
    expect(scene.map.height).toBe(OFFICE_MAP.height);
    expect(scene.map.tiles.length).toBe(scene.map.height);
    for (const row of scene.map.tiles) expect(row.length).toBe(scene.map.width);
  });

  it.each(ALL_SCENES.map((s) => [s.id, s] as const))(
    "[%s] 좌석 수가 오피스 이상이고, 모든 좌석이 걸을 수 있으며 바로 아래가 책상이다",
    (_id, scene) => {
      expect(scene.map.desks.length).toBeGreaterThanOrEqual(OFFICE_MAP.desks.length);
      for (const desk of scene.map.desks) {
        const { tx, ty } = desk.seat;
        expect(isWalkable(scene.map, tx, ty)).toBe(true);
        expect(scene.map.tiles[ty + 1][tx]).toBe(Tile.DeskTop);
      }
      // index는 0..N-1로 빈틈없이 매겨져야 한다(assignDesks가 인덱스를 키로 쓴다).
      const indices = scene.map.desks.map((d) => d.index).sort((a, b) => a - b);
      expect(indices).toEqual(Array.from({ length: scene.map.desks.length }, (_, i) => i));
    },
  );

  it.each(ALL_SCENES.map((s) => [s.id, s] as const))(
    "[%s] 휴게 공간이 맵 안에 있고 내부 전 타일이 걸을 수 있다",
    (_id, scene) => {
      const rect = scene.map.breakRoom;
      expect(rect).toBeDefined();
      expect(rect!.w).toBeGreaterThan(0);
      expect(rect!.h).toBeGreaterThan(0);
      expect(rect!.x + rect!.w).toBeLessThanOrEqual(scene.map.width);
      expect(rect!.y + rect!.h).toBeLessThanOrEqual(scene.map.height);
      for (let ty = rect!.y; ty < rect!.y + rect!.h; ty++) {
        for (let tx = rect!.x; tx < rect!.x + rect!.w; tx++) {
          expect(isWalkable(scene.map, tx, ty)).toBe(true);
        }
      }
    },
  );

  it.each(ALL_SCENES.map((s) => [s.id, s] as const))(
    "[%s] 보스 책상이 존재하고 그 사각형이 전부 BossDesk 타일이다",
    (_id, scene) => {
      const rect = scene.map.bossDesk;
      expect(rect).toBeDefined();
      expect(rect!.w).toBeGreaterThan(0);
      expect(rect!.h).toBeGreaterThan(0);
      for (let ty = rect!.y; ty < rect!.y + rect!.h; ty++) {
        for (let tx = rect!.x; tx < rect!.x + rect!.w; tx++) {
          expect(scene.map.tiles[ty][tx]).toBe(Tile.BossDesk);
          expect(isWalkable(scene.map, tx, ty)).toBe(false); // 보스 자리로는 못 걸어 들어간다
        }
      }
    },
  );

  it.each(ALL_SCENES.map((s) => [s.id, s] as const))(
    "[%s] 줄서기 슬롯이 존재하고 전부 걸을 수 있는 서로 다른 타일이다",
    (_id, scene) => {
      const slots = scene.map.queueSlots;
      expect(slots).toBeDefined();
      expect(slots!.length).toBeGreaterThan(0);
      const seen = new Set<string>();
      for (const s of slots!) {
        expect(isWalkable(scene.map, s.tx, s.ty)).toBe(true);
        seen.add(`${s.tx},${s.ty}`);
      }
      expect(seen.size).toBe(slots!.length); // 두 캐릭터가 같은 칸에 겹쳐 서지 않는다
    },
  );

  it("모든 풍경 × 모든 테마 조합이 유효한 배경색과 예외 없는 드로잉을 낸다", () => {
    for (const scene of ALL_SCENES) {
      for (const themeId of THEME_ORDER) {
        const render = scene.resolve(THEMES[themeId]);
        expect(Number.isInteger(render.background)).toBe(true);
        expect(render.background).toBeGreaterThanOrEqual(0);
        expect(render.background).toBeLessThanOrEqual(0xffffff);
        // 맵 전 칸을 실제로 그려 본다 — 좌표 계산이 깨지면 여기서 터진다.
        for (let ty = 0; ty < scene.map.height; ty++) {
          for (let tx = 0; tx < scene.map.width; tx++) {
            const g = new Graphics();
            expect(() =>
              render.drawTile(g, { t: scene.map.tiles[ty][tx], tx, ty, s: 16, map: scene.map }),
            ).not.toThrow();
            expect(g.context.instructions.length).toBeGreaterThan(0);
          }
        }
      }
    }
  });

  it("씬마다 8종 의미 타일 전부를 쓴다(어휘가 통째로 바뀌어도 빠진 종류가 없다)", () => {
    for (const scene of ALL_SCENES) {
      const used = new Set<Tile>();
      for (const row of scene.map.tiles) for (const t of row) used.add(t);
      for (const t of [
        Tile.Floor,
        Tile.Wall,
        Tile.DeskTop,
        Tile.Rug,
        Tile.Plant,
        Tile.Counter,
        Tile.Table,
        Tile.BossDesk,
      ]) {
        expect(used.has(t)).toBe(true);
      }
    }
  });
});

describe("sceneColor", () => {
  it("테마 → 모드 매핑(밝은 테마는 원색, midnight은 황혼, pipboy는 초록)", () => {
    expect(sceneColorMode("daylight")).toBe("identity");
    expect(sceneColorMode("sakura")).toBe("identity");
    expect(sceneColorMode("midnight")).toBe("dusk");
    expect(sceneColorMode("pipboy")).toBe("green");
  });

  it("결정적이다: 같은 입력이면 몇 번을 불러도 같은 값", () => {
    for (const mode of ["identity", "dusk", "green"] as const) {
      for (const c of [0x000000, 0x123456, 0xf2dfae, 0xffffff]) {
        expect(adaptColor(c, mode)).toBe(adaptColor(c, mode));
      }
    }
  });

  it("항상 유효한 0xRRGGBB를 낸다", () => {
    for (const mode of ["identity", "dusk", "green"] as const) {
      for (const c of [0x000000, 0x0f4670, 0xef6f6c, 0xffffff]) {
        const out = adaptColor(c, mode);
        expect(Number.isInteger(out)).toBe(true);
        expect(out).toBeGreaterThanOrEqual(0);
        expect(out).toBeLessThanOrEqual(0xffffff);
      }
    }
  });

  it("identity는 입력을 그대로 돌려준다", () => {
    expect(adaptColor(0x123456, "identity")).toBe(0x123456);
    expect(adaptColor(0xffffff, "identity")).toBe(0xffffff);
  });

  it("dusk는 어둡게 만든다(원색보다 휘도가 낮다)", () => {
    const luma = (c: number) =>
      0.299 * ((c >> 16) & 0xff) + 0.587 * ((c >> 8) & 0xff) + 0.114 * (c & 0xff);
    for (const c of [0xf2dfae, 0x2a92c9, 0x6faf57, 0xffffff]) {
      expect(luma(adaptColor(c, "dusk"))).toBeLessThan(luma(c));
    }
  });

  it("green은 초록이 지배적인 색으로 보낸다", () => {
    for (const c of [0xf2dfae, 0x2a92c9, 0xef6f6c, 0x333333]) {
      const out = adaptColor(c, "green");
      const [r, g, b] = [(out >> 16) & 0xff, (out >> 8) & 0xff, out & 0xff];
      expect(g).toBeGreaterThanOrEqual(r);
      expect(g).toBeGreaterThanOrEqual(b);
    }
  });

  it("adaptPalette는 원본을 변형하지 않고 새 객체를 만든다", () => {
    const pal = { a: 0x112233, b: 0xff8800 };
    const snapshot = { ...pal };
    for (const mode of ["identity", "dusk", "green"] as const) {
      const out = adaptPalette(pal, mode);
      expect(out).not.toBe(pal);
      expect(Object.keys(out).sort()).toEqual(Object.keys(pal).sort());
      expect(pal).toEqual(snapshot); // 입력 불변
    }
    expect(adaptPalette(pal, "identity")).toEqual(pal);
  });
});

describe("sceneStorage", () => {
  it("저장한 풍경을 그대로 복원한다(라운드트립)", async () => {
    const { SCENE_STORAGE_KEY, loadStoredSceneId, persistSceneId } = await import("../sceneStorage");
    for (const id of SCENE_ORDER) {
      persistSceneId(id);
      expect(localStorage.getItem(SCENE_STORAGE_KEY)).toBe(id);
      expect(loadStoredSceneId()).toBe(id);
    }
  });

  it("부재/오염된 값이면 기본 풍경으로 폴백한다", async () => {
    const { SCENE_STORAGE_KEY, loadStoredSceneId } = await import("../sceneStorage");
    expect(loadStoredSceneId()).toBe(DEFAULT_SCENE_ID); // 부재
    for (const junk of ["", "  ", "OFFICE", "beach ", "{}", "null"]) {
      localStorage.setItem(SCENE_STORAGE_KEY, junk);
      expect(loadStoredSceneId()).toBe(DEFAULT_SCENE_ID);
    }
  });

  it("store.setScene: 상태 갱신 + 영속을 한 번에 수행한다", async () => {
    const { SCENE_STORAGE_KEY } = await import("../sceneStorage");
    const { useAppStore } = await import("../../../store/appStore");
    useAppStore.getState().setScene("valley");
    expect(useAppStore.getState().scene).toBe("valley");
    expect(localStorage.getItem(SCENE_STORAGE_KEY)).toBe("valley");
    useAppStore.getState().setScene(DEFAULT_SCENE_ID); // 다른 테스트에 새지 않게 되돌린다
  });
});
