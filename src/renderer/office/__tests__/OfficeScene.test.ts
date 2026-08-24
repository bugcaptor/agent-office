// @vitest-environment jsdom
//
// src/renderer/office/__tests__/OfficeScene.test.ts
//
// Tests for the integer-scale camera
// calculation, and dispose-safety around the async `init()`. Also covers
// `syncAgents`/`destroy` delegating to `OfficeWorld`,
// and `init()` wiring `world.update()` onto the Pixi ticker.
//
// Only `pixi.js`'s `Application` is mocked (its `init()` needs a real
// WebGL/canvas-2d context, unavailable in jsdom) so this stays a focused
// unit test of `OfficeScene`'s own orchestration logic rather than an
// integration test of Pixi rendering — per the task brief's guidance to
// minimize Pixi-Application-requiring test surface. `Container`/`Graphics`
// (used by `TileRenderer`) are real: they don't need a render context to
// construct. The 3H tests below spy on the real `OfficeWorld.prototype`
// (rather than mocking the module) and only ever pass it an empty profile
// list, so they never touch `createCharacterAssets`'s canvas dependency —
// entity-creation behavior itself is `OfficeWorld.test.ts`'s job, not this
// suite's.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// `vi.mock` factories are hoisted above the rest of the file, so mutable
// state they close over must come from `vi.hoisted` (plain top-level
// `const`/`let` would be a TDZ violation at hoist time).
const state = vi.hoisted(() => ({
  destroySpy: vi.fn(),
  tickerAddSpy: vi.fn(),
  tickerRemoveSpy: vi.fn(),
  initResolvers: [] as Array<() => void>,
  // `background`는 setTheme/setScene의 라이브 배경색 갱신이 만지는 유일한
  // 렌더러 표면이라 페이크에도 둔다(실제 Pixi는 여기에 색을 쓴다).
  rendererSize: { width: 320, height: 224, background: { color: 0 } },
}));

// OfficeScene이 수상자 초상을 직접 불러오므로(office/**에서 유일하게 tauriApi를
// 직접 호출 — 설계 §7 "소비할 데이터"), 이 모듈만 목킹한다. 기본은 초상 없음.
const tauri = vi.hoisted(() => ({
  loadAwardPortrait: vi.fn(async () => null as string | null),
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: tauri }));

// 트로피 배치 테스트가 실제 캐릭터를 syncAgents하므로, OfficeWorld.test.ts와
// 같은 이유(createCharacterAssets는 Texture.from에 실제 <canvas>가 필요 —
// 이 vitest jsdom 환경엔 없다)로 캔버스를 만지는 팩토리만 목킹한다.
const charFactory = vi.hoisted(() => ({
  createCharacterAssetsSpy: vi.fn(),
  createMinimiAssetsSpy: vi.fn(() => null),
}));
vi.mock("../gen/characterFactory", () => ({ createCharacterAssets: charFactory.createCharacterAssetsSpy }));
vi.mock("../gen/minimiFactory", () => ({ createMinimiAssets: charFactory.createMinimiAssetsSpy }));

vi.mock("pixi.js", async (importOriginal) => {
  const actual = await importOriginal<typeof import("pixi.js")>();

  class FakeApplication {
    stage = new actual.Container();
    renderer: typeof state.rendererSize | undefined;
    // `OfficeScene.init()` wires `world.update()` onto `ticker.add`; real
    // `Application.init()` needs a WebGL/canvas-2d context this jsdom-free
    // unit test doesn't provide, so the ticker is faked out here too.
    ticker = { add: state.tickerAddSpy, remove: state.tickerRemoveSpy };
    init = vi.fn(() => {
      return new Promise<void>((resolve) => {
        state.initResolvers.push(() => {
          this.renderer = state.rendererSize;
          resolve();
        });
      });
    });
    destroy = state.destroySpy;
  }

  return { ...actual, Application: FakeApplication };
});

// Imported after the mock is registered so OfficeScene picks up the fake Application.
const { OfficeScene, computeIntegerScale, bindHoverGate } = await import("../OfficeScene");
const { createMockOfficeBus } = await import("../bus");
const { OfficeWorld } = await import("../world/OfficeWorld");
const { BufferImageSource, Texture } = await import("pixi.js");
const { i18n, initI18nForTest } = await import("../../i18n");

// `createCharacterAssets`가 실제로 하는 일(cellSize=16의 4프레임 텍스처 세트)을
// 최소한으로 흉내낸다 — OfficeWorld.test.ts의 makeFakeAssets와 같은 이유·같은 모양.
function makeFakeAssets() {
  const solid = (label: string) =>
    new Texture({
      source: new BufferImageSource({ resource: new Uint8Array([255, 255, 255, 255]), width: 1, height: 1, label }),
      label,
    });
  const idle0 = solid("idle0");
  const idle1 = solid("idle1");
  const walk0 = solid("walk0");
  const walk1 = solid("walk1");
  return {
    base: idle0,
    frames: { idle0, idle1, walk0, walk1 },
    idle: [idle0, idle1],
    walk: [walk0, walk1],
    cellSize: 16,
    descriptor: { archetype: "test", hair: "test", clothes: "test", accessory: "test" },
  };
}

class FakeResizeObserver {
  observe(): void {}
  disconnect(): void {}
}

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", FakeResizeObserver);
  state.destroySpy.mockClear();
  state.tickerAddSpy.mockClear();
  state.tickerRemoveSpy.mockClear();
  state.initResolvers = [];
  state.rendererSize = { width: 320, height: 224, background: { color: 0 } };
  tauri.loadAwardPortrait.mockClear();
  tauri.loadAwardPortrait.mockResolvedValue(null);
  charFactory.createCharacterAssetsSpy.mockReset().mockImplementation(() => makeFakeAssets());
  charFactory.createMinimiAssetsSpy.mockReset().mockImplementation(() => null);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("computeIntegerScale", () => {
  it("picks the largest integer scale that fits both dimensions", () => {
    // map is 320x224; viewport 700x500 -> width allows 2.18x, height allows 2.23x -> floor to 2
    expect(computeIntegerScale(700, 500, 320, 224)).toBe(2);
  });

  it("is bounded by whichever dimension is tighter", () => {
    // width allows 10x but height only allows 1.5x -> clamp to 1
    expect(computeIntegerScale(3200, 336, 320, 224)).toBe(1);
  });

  it("never returns less than 1, even if the viewport is smaller than the map", () => {
    expect(computeIntegerScale(100, 80, 320, 224)).toBe(1);
  });
});

describe("OfficeScene destroy() dispose-safety", () => {
  it("is a safe no-op if called before init() resolves (does not touch the Pixi app)", () => {
    const canvas = document.createElement("canvas");
    const scene = new OfficeScene({ canvas, bus: createMockOfficeBus() });

    void scene.init(); // fire and forget, deliberately not resolved yet
    expect(() => scene.destroy()).not.toThrow();
    expect(state.destroySpy).not.toHaveBeenCalled();
  });

  it("performs the real teardown once init() has resolved", async () => {
    const canvas = document.createElement("canvas");
    const scene = new OfficeScene({ canvas, bus: createMockOfficeBus() });

    const initPromise = scene.init();
    state.initResolvers.forEach((resolve) => resolve());
    await initPromise;

    // init() wires world.update() onto the Pixi ticker (drives entity
    // FSM/movement/animation off Pixi's own frame clock).
    expect(state.tickerAddSpy).toHaveBeenCalledTimes(1);

    scene.destroy();
    expect(state.destroySpy).toHaveBeenCalledTimes(1);
    expect(state.destroySpy).toHaveBeenCalledWith(true, { children: true, texture: true });
    expect(state.tickerRemoveSpy).toHaveBeenCalledTimes(1);
    expect(state.tickerRemoveSpy).toHaveBeenCalledWith(state.tickerAddSpy.mock.calls[0][0]);
  });

  it("is idempotent: calling destroy() twice after init only tears down once", async () => {
    const canvas = document.createElement("canvas");
    const scene = new OfficeScene({ canvas, bus: createMockOfficeBus() });

    const initPromise = scene.init();
    state.initResolvers.forEach((resolve) => resolve());
    await initPromise;

    scene.destroy();
    scene.destroy();
    expect(state.destroySpy).toHaveBeenCalledTimes(1);
  });

  it("matches the exact StrictMode double-mount race: destroy() before init, then init resolves", async () => {
    // Simulates the hook's `disposed` flag pattern: cleanup fires (destroy
    // before init resolves) and *then* the init promise settles.
    const canvas = document.createElement("canvas");
    const scene = new OfficeScene({ canvas, bus: createMockOfficeBus() });

    const initPromise = scene.init();
    scene.destroy(); // cleanup fires first, synchronously, before init resolves
    expect(state.destroySpy).not.toHaveBeenCalled();

    state.initResolvers.forEach((resolve) => resolve());
    await initPromise;
    // Mirrors useOfficeScene's init().then(() => { if (disposed) scene.destroy(); })
    scene.destroy();

    expect(state.destroySpy).toHaveBeenCalledTimes(1);
  });
});

describe("OfficeScene <-> OfficeWorld wiring (Task 3H)", () => {
  it("does not delegate to OfficeWorld.syncAgents before init() resolves", () => {
    const spy = vi.spyOn(OfficeWorld.prototype, "syncAgents");
    const canvas = document.createElement("canvas");
    const scene = new OfficeScene({ canvas, bus: createMockOfficeBus() });

    void scene.init(); // deliberately not resolved
    scene.syncAgents([]);

    expect(spy).not.toHaveBeenCalled();
    spy.mockRestore();
  });

  it("delegates syncAgents to OfficeWorld.syncAgents once init() has resolved", async () => {
    const spy = vi.spyOn(OfficeWorld.prototype, "syncAgents");
    const canvas = document.createElement("canvas");
    const scene = new OfficeScene({ canvas, bus: createMockOfficeBus() });

    const initPromise = scene.init();
    state.initResolvers.forEach((resolve) => resolve());
    await initPromise;

    scene.syncAgents([]);
    expect(spy).toHaveBeenCalledWith([]);
    spy.mockRestore();
  });

  it("calls OfficeWorld.destroy() even if destroy() runs before init() resolves (no bus-listener leak)", () => {
    const spy = vi.spyOn(OfficeWorld.prototype, "destroy");
    const canvas = document.createElement("canvas");
    const scene = new OfficeScene({ canvas, bus: createMockOfficeBus() });

    void scene.init();
    scene.destroy();

    expect(spy).toHaveBeenCalledTimes(1);
    spy.mockRestore();
  });
});

describe("desk click hit areas (책상 지정 메뉴)", () => {
  it("init() 후 데스크 슬롯마다 인터랙티브 히트영역이 생기고, pointertap이 emitDeskClicked(index, 화면좌표)로 나간다", async () => {
    const { OFFICE_MAP, TILE_SIZE } = await import("../map/mapData");
    const bus = createMockOfficeBus();
    const seen: Array<[number, number, number]> = [];
    bus.onDeskClicked((i, x, y) => seen.push([i, x, y]));

    const canvas = document.createElement("canvas");
    const scene = new OfficeScene({ canvas, bus });
    const initPromise = scene.init();
    state.initResolvers.forEach((resolve) => resolve());
    await initPromise;

    // floorLayer(최하단 레이어)에 있는 인터랙티브 자식들 = 데스크 히트영역 + 보스 책상 히트영역.
    // 캐릭터/가구보다 아래에 두어 캐릭터 클릭이 항상 우선한다.
    const floorLayer = (
      scene as unknown as { floorLayer: { children: Array<Record<string, unknown>> } }
    ).floorLayer;
    const hits = floorLayer.children.filter((c) => c.eventMode === "static");
    expect(hits.length).toBe(OFFICE_MAP.desks.length + 1); // +1 for boss desk

    // 첫 데스크의 히트영역을 찾아 탭 → (index, 화면좌표) 발행 확인.
    const d0 = OFFICE_MAP.desks[0];
    const hit = hits.find(
      (c) =>
        (c.position as { x: number; y: number }).x === d0.seat.tx * TILE_SIZE &&
        (c.position as { x: number; y: number }).y === (d0.seat.ty + 1) * TILE_SIZE,
    ) as unknown as { cursor: string; emit(ev: string, payload: unknown): boolean };
    expect(hit).toBeDefined();
    expect(hit.cursor).toBe("pointer");

    hit.emit("pointertap", { global: { x: 33, y: 44 } });
    expect(seen).toEqual([[d0.index, 33, 44]]);
  });
});

describe("setScene (풍경 교체)", () => {
  /** floorLayer의 인터랙티브 자식 = 좌석 히트영역 + 보스 책상 히트영역. */
  const hitsOf = (s: unknown) =>
    (s as { floorLayer: { children: Array<Record<string, unknown>> } }).floorLayer.children.filter(
      (c) => c.eventMode === "static",
    );

  async function makeStartedScene() {
    const canvas = document.createElement("canvas");
    const officeScene = new OfficeScene({ canvas, bus: createMockOfficeBus() });
    const initPromise = officeScene.init();
    state.initResolvers.forEach((resolve) => resolve());
    await initPromise;
    return officeScene;
  }

  it("바닥 재베이크 + 가구 교체 + 히트영역/보스 책상 재구축 + 월드 맵 전파를 한 번에 한다", async () => {
    const { SCENES } = await import("../scenes/scenes");
    const { TILE_SIZE } = await import("../map/mapData");
    const setMapSpy = vi.spyOn(OfficeWorld.prototype, "setMap");
    const officeScene = await makeStartedScene();

    const internals = officeScene as unknown as {
      floorTiles: { destroyed: boolean };
      furnitureTiles: unknown[];
    };
    const officeFloor = internals.floorTiles;
    const officeFurniture = internals.furnitureTiles;
    expect(hitsOf(officeScene)).toHaveLength(SCENES.office.map.desks.length + 1);

    officeScene.setScene(SCENES.valley);

    // 바닥: 이전 베이크 컨테이너는 파기되고 새 인스턴스로 교체된다.
    expect(officeFloor.destroyed).toBe(true);
    expect(internals.floorTiles).not.toBe(officeFloor);
    // 가구: Graphics 전량 교체.
    expect(internals.furnitureTiles).not.toBe(officeFurniture);
    expect(internals.furnitureTiles.length).toBeGreaterThan(0);
    // 월드: 새 맵 전파(좌석 재배정 + 엔티티 리타깃은 OfficeWorld의 몫).
    expect(setMapSpy).toHaveBeenCalledWith(SCENES.valley.map);
    // 히트영역: 개수는 새 맵의 좌석 수 + 보스 1, 위치는 새 맵의 좌석 좌표.
    const hits = hitsOf(officeScene);
    expect(hits).toHaveLength(SCENES.valley.map.desks.length + 1);
    const bossRect = SCENES.valley.map.bossDesk!;
    const bossHit = hits.find(
      (c) =>
        (c.position as { x: number; y: number }).x === bossRect.x * TILE_SIZE &&
        (c.position as { x: number; y: number }).y === bossRect.y * TILE_SIZE,
    );
    expect(bossHit).toBeDefined(); // 오피스(tx17)와 다른 좌표(tx2)에 다시 생겼다

    setMapSpy.mockRestore();
    officeScene.destroy();
  });

  it("같은 풍경을 다시 넘기면 아무것도 재구축하지 않는다", async () => {
    const { SCENES } = await import("../scenes/scenes");
    const setMapSpy = vi.spyOn(OfficeWorld.prototype, "setMap");
    const officeScene = await makeStartedScene();
    const before = (officeScene as unknown as { floorTiles: unknown }).floorTiles;

    officeScene.setScene(SCENES.office); // 생성 시 기본값과 동일

    expect((officeScene as unknown as { floorTiles: unknown }).floorTiles).toBe(before);
    expect(setMapSpy).not.toHaveBeenCalled();
    setMapSpy.mockRestore();
    officeScene.destroy();
  });

  it("풍경을 바꿔도 휴가 팻말 표시 상태가 유지되고 구독이 중복되지 않는다", async () => {
    const { SCENES } = await import("../scenes/scenes");
    const bus = createMockOfficeBus();
    const canvas = document.createElement("canvas");
    const officeScene = new OfficeScene({ canvas, bus });
    const initPromise = officeScene.init();
    state.initResolvers.forEach((resolve) => resolve());
    await initPromise;

    bus.triggerVacationModeChanged(true);
    const signOf = () => (officeScene as unknown as { bossSign?: { visible: boolean } }).bossSign!;
    expect(signOf().visible).toBe(true);

    officeScene.setScene(SCENES.beach);
    // 새로 만든 팻말이 휴가 상태를 그대로 이어받는다(bus는 값이 바뀔 때만 발화).
    expect(signOf().visible).toBe(true);

    // 이전 씬의 구독이 남아 있었다면 파기된 팻말을 만져 예외가 났을 것이다.
    bus.triggerVacationModeChanged(false);
    expect(signOf().visible).toBe(false);

    officeScene.destroy();
  });
});

describe("휴가 팻말 문구의 언어 전환 (Pixi Text는 React 밖이다)", () => {
  /** 팻말 글씨(Pixi `Text`). 씬이 직접 들고 있는 참조를 그대로 본다. */
  const labelOf = (s: unknown) => (s as { bossSignLabel?: { text: string } }).bossSignLabel!;

  /** i18next EventEmitter의 `languageChanged` 구독자 수(내부 `observers` Map).
   *  리스너 누수는 곧 파기된 Pixi 객체 접근이라 개수 자체를 계약으로 본다. */
  const languageListeners = (): number => {
    const obs = (i18n as unknown as { observers?: Record<string, Map<unknown, number>> })
      .observers;
    return obs?.languageChanged?.size ?? 0;
  };

  async function makeStartedScene() {
    const canvas = document.createElement("canvas");
    const officeScene = new OfficeScene({ canvas, bus: createMockOfficeBus() });
    const initPromise = officeScene.init();
    state.initResolvers.forEach((resolve) => resolve());
    await initPromise;
    return officeScene;
  }

  // 언어는 파일 전역 상태다 — 정본(ko)으로 되돌려 다른 테스트에 새지 않게 한다.
  afterEach(async () => {
    await initI18nForTest();
  });

  it("언어를 바꾸면 씬 재구축 없이 팻말 글씨만 바뀐다", async () => {
    const officeScene = await makeStartedScene();
    const sign = (officeScene as unknown as { bossSign?: unknown }).bossSign;
    const label = labelOf(officeScene);
    expect(label.text).toBe("휴가중");

    await initI18nForTest("en");

    expect(label.text).toBe("ON LEAVE");
    // 같은 표시객체를 그대로 쓴다(글씨만 교체 — 씬/팻말 재구축 없음).
    expect((officeScene as unknown as { bossSign?: unknown }).bossSign).toBe(sign);
    expect(labelOf(officeScene)).toBe(label);

    officeScene.destroy();
  });

  it("destroy()가 언어 리스너를 떼어 낸다(파기된 Text 접근 방지)", async () => {
    const before = languageListeners();
    const officeScene = await makeStartedScene();
    expect(languageListeners()).toBe(before + 1);

    officeScene.destroy();

    expect(languageListeners()).toBe(before);
    // 리스너가 남았다면 파기된 Text의 `.text`를 건드려 여기서 터진다.
    await initI18nForTest("en");
  });

  it("풍경을 바꿔도 리스너가 하나만 유지된다(팻말 재구축 시 중복 구독 방지)", async () => {
    const { SCENES } = await import("../scenes/scenes");
    const before = languageListeners();
    const officeScene = await makeStartedScene();

    officeScene.setScene(SCENES.beach);
    expect(languageListeners()).toBe(before + 1);

    await initI18nForTest("en");
    expect(labelOf(officeScene).text).toBe("ON LEAVE");

    officeScene.destroy();
    expect(languageListeners()).toBe(before);
  });
});

describe("label anchor publishing (overhead-task-label)", () => {
  it("worldToScreen: 카메라 offset + scale을 적용한다", async () => {
    const { worldToScreen } = await import("../OfficeScene");
    expect(worldToScreen(50, 40, 2, 7, 11)).toEqual({ x: 107, y: 91 });
  });

  it("ticker 콜백이 매 tick collectLabelAnchors 결과를 화면좌표로 bus에 발행한다", async () => {
    const collectSpy = vi
      .spyOn(OfficeWorld.prototype, "collectLabelAnchors")
      .mockImplementation(function (out: Map<string, { x: number; y: number }>) {
        out.clear();
        out.set("a1", { x: 50, y: 40 });
      });
    const bus = createMockOfficeBus();
    const seen: Array<Map<string, { x: number; y: number }>> = [];
    bus.onLabelAnchorsChanged((m) => seen.push(new Map(m)));

    const canvas = document.createElement("canvas");
    const scene = new OfficeScene({ canvas, bus });
    const initPromise = scene.init();
    state.initResolvers.forEach((resolve) => resolve());
    await initPromise;

    // init()이 ticker에 건 콜백을 직접 호출해 한 프레임을 흉내낸다.
    const tickerCb = state.tickerAddSpy.mock.calls[0][0] as (t: { deltaMS: number }) => void;
    tickerCb({ deltaMS: 16 });

    expect(seen).toHaveLength(1);
    // renderer 320x224, 맵 320x224 → scale 1, offset (0,0) → 월드좌표 그대로.
    expect(seen[0].get("a1")).toEqual({ x: 50, y: 40 });
    collectSpy.mockRestore();
  });
});

describe("award trophy + frame (이 달의 우수사원, docs/employee-of-the-month-design.md §7)", () => {
  type Internals = {
    trophyOverlay?: { root: { visible: boolean; x: number; y: number; zIndex: number } };
    awardFrame?: { root: { visible: boolean } };
    // floorLayer 부착 순서(액자가 벽 타일 위에 남는지) 검증용.
    floorLayer: { children: unknown[] };
  };

  async function makeStartedScene(bus = createMockOfficeBus()) {
    const canvas = document.createElement("canvas");
    const officeScene = new OfficeScene({ canvas, bus });
    const initPromise = officeScene.init();
    state.initResolvers.forEach((resolve) => resolve());
    await initPromise;
    return { officeScene, bus };
  }

  it("확정 수상자가 없으면 트로피와 액자 콘텐츠 모두 숨겨진다", async () => {
    const { officeScene } = await makeStartedScene();
    const internals = officeScene as unknown as Internals;

    expect(internals.trophyOverlay?.root.visible).toBe(false);
    expect(internals.awardFrame?.root.visible).toBe(false);

    officeScene.destroy();
  });

  it("수상자가 발화되면 좌석 위치에 트로피가, 벽에 액자 콘텐츠가 표시된다", async () => {
    const { resolveAwardeeSeat } = await import("../awardee");
    const { OFFICE_MAP } = await import("../map/mapData");
    const { tileCenterPx } = await import("../world/pathing");
    const { officeScene, bus } = await makeStartedScene();
    const profiles = [{ id: "a1", name: "하나", role: "eng", seed: "a1", assignedDeskIndex: 0 }];

    officeScene.syncAgents(profiles);
    bus.triggerAwardeeChanged({ agentId: "a1", name: "하나", month: "2026-07", hasPortrait: false });

    const seat = resolveAwardeeSeat(OFFICE_MAP, "a1", profiles)!;
    const expected = tileCenterPx({ tx: seat.tx + 1, ty: seat.ty + 1 });
    const internals = officeScene as unknown as Internals;

    expect(internals.trophyOverlay?.root.visible).toBe(true);
    expect(internals.trophyOverlay?.root.x).toBe(expected.x);
    expect(internals.trophyOverlay?.root.y).toBe(expected.y - 2);
    // zIndex는 y가 아니라 **얹힌 책상 상판의 정렬값 + 1**이다. 가구는 자기 아래
    // 모서리((ty+1)*TILE_SIZE)로 정렬하므로 y를 그대로 쓰면 트로피가 자기가
    // 올라앉은 책상 뒤로 들어가 통째로 가려진다(회귀: "트로피 안 보이는데?").
    const deskSortKey = (seat.ty + 1 + 1) * 16;
    expect(internals.trophyOverlay?.root.zIndex).toBeGreaterThan(deskSortKey);
    // 책상 남쪽 타일에 선 캐릭터(그 타일 중심 = deskSortKey + 8)보다는 뒤에 온다.
    expect(internals.trophyOverlay?.root.zIndex).toBeLessThan(deskSortKey + 8);
    expect(internals.awardFrame?.root.visible).toBe(true);
    // hasPortrait:false -> 초상 IPC를 아예 타지 않는다.
    expect(tauri.loadAwardPortrait).not.toHaveBeenCalled();

    officeScene.destroy();
  });

  it("수상자가 오피스에 없으면(프로필 삭제/미배정) 트로피를 숨긴다", async () => {
    const { officeScene, bus } = await makeStartedScene();
    officeScene.syncAgents([]); // a1 프로필이 로스터에 없음
    bus.triggerAwardeeChanged({ agentId: "a1", name: "하나", month: "2026-07", hasPortrait: false });

    const internals = officeScene as unknown as Internals;
    expect(internals.trophyOverlay?.root.visible).toBe(false);
    // 액자는 "확정 수상자가 있으면 항상 보인다"(좌석 유무와 무관).
    expect(internals.awardFrame?.root.visible).toBe(true);

    officeScene.destroy();
  });

  it("수상자가 교체되면 트로피 위치가 새 좌석으로 갱신된다", async () => {
    const { resolveAwardeeSeat } = await import("../awardee");
    const { OFFICE_MAP } = await import("../map/mapData");
    const { tileCenterPx } = await import("../world/pathing");
    const { officeScene, bus } = await makeStartedScene();
    const profiles = [
      { id: "a1", name: "하나", role: "eng", seed: "a1", assignedDeskIndex: 0 },
      { id: "a2", name: "두리", role: "eng", seed: "a2", assignedDeskIndex: 1 },
    ];
    officeScene.syncAgents(profiles);

    bus.triggerAwardeeChanged({ agentId: "a1", name: "하나", month: "2026-07", hasPortrait: false });
    const internals = officeScene as unknown as Internals;
    const seat1 = resolveAwardeeSeat(OFFICE_MAP, "a1", profiles)!;
    const p1 = tileCenterPx({ tx: seat1.tx + 1, ty: seat1.ty + 1 });
    expect(internals.trophyOverlay?.root.x).toBe(p1.x);

    bus.triggerAwardeeChanged({ agentId: "a2", name: "두리", month: "2026-08", hasPortrait: false });
    const seat2 = resolveAwardeeSeat(OFFICE_MAP, "a2", profiles)!;
    const p2 = tileCenterPx({ tx: seat2.tx + 1, ty: seat2.ty + 1 });
    expect(internals.trophyOverlay?.root.x).toBe(p2.x);
    expect(p2.x).not.toBe(p1.x); // 실제로 다른 책상으로 옮겨졌는지 확인(픽스처가 우연히 같은 좌표를 고르지 않도록)

    officeScene.destroy();
  });

  it("같은 수상자로 재발화되면 초상을 다시 불러오지 않는다(불필요한 재조회 없음)", async () => {
    const { officeScene, bus } = await makeStartedScene();
    const awardee = { agentId: "a1", name: "하나", month: "2026-07", hasPortrait: true };
    tauri.loadAwardPortrait.mockResolvedValue("YWJj"); // base64("abc") — 디코드는 실패해도 IPC 호출 자체만 검증

    bus.triggerAwardeeChanged(awardee);
    await Promise.resolve(); // loadAwardPortrait의 첫 then까지 flush
    await Promise.resolve();
    expect(tauri.loadAwardPortrait).toHaveBeenCalledTimes(1);

    bus.triggerAwardeeChanged({ ...awardee }); // 내용은 같고 참조만 다른 객체
    await Promise.resolve();
    await Promise.resolve();
    expect(tauri.loadAwardPortrait).toHaveBeenCalledTimes(1); // 재조회 없음

    officeScene.destroy();
  });

  it("setScene으로 씬을 재구축해도 트로피/액자 표시 상태가 유지된다(휴가 팻말과 동일 패턴)", async () => {
    const { SCENES } = await import("../scenes/scenes");
    const { officeScene, bus } = await makeStartedScene();
    const profiles = [{ id: "a1", name: "하나", role: "eng", seed: "a1", assignedDeskIndex: 0 }];
    officeScene.syncAgents(profiles);
    bus.triggerAwardeeChanged({ agentId: "a1", name: "하나", month: "2026-07", hasPortrait: false });

    officeScene.setScene(SCENES.valley);

    const internals = officeScene as unknown as Internals;
    expect(internals.trophyOverlay?.root.visible).toBe(true);
    // valley 씬은 액자 아트가 없다(현재 오피스 씬 전용) — 콘텐츠는 숨겨진다.
    expect(internals.awardFrame?.root.visible).toBe(false);

    officeScene.setScene(SCENES.office);
    expect((officeScene as unknown as Internals).awardFrame?.root.visible).toBe(true);

    officeScene.destroy();
  });

  it("액자는 overlayLayer가 아니라 floorLayer에 있다(벽 타일과 같은 레이어 — sortableLayer의 캐릭터·가구가 항상 앞에 온다)", async () => {
    const { officeScene } = await makeStartedScene();
    const internals = officeScene as unknown as Internals;
    expect(internals.floorLayer.children).toContain(internals.awardFrame?.root);
    officeScene.destroy();
  });

  it("setTheme으로 재도색해도 액자가 floorLayer에서 벽(floorTiles) 위에 남는다", async () => {
    const { THEMES } = await import("../../theme/themes");
    const { officeScene, bus } = await makeStartedScene();
    bus.triggerAwardeeChanged({ agentId: "a1", name: "하나", month: "2026-07", hasPortrait: false });

    const internals = officeScene as unknown as Internals;
    const frameRoot = internals.awardFrame?.root;
    expect(frameRoot?.visible).toBe(true);

    officeScene.setTheme(THEMES.pipboy);

    // repaint()가 floorTiles를 통째로 새로 만든다 — buildMapLayers가
    // addChild(append)가 아니라 addChildAt(…, 0)으로 맨 아래에 꽂아야, 이미
    // floorLayer에 있던 액자(및 데스크/보스 히트영역)가 새 floorTiles보다
    // 항상 위에 남는다. append였다면 재도색마다 액자가 벽 밑으로 깔린다.
    const children = internals.floorLayer.children;
    expect(children).toContain(frameRoot);
    expect(children.indexOf(frameRoot)).toBeGreaterThan(0);
    expect(internals.awardFrame?.root.visible).toBe(true); // 재도색이 가시성을 건드리지 않는다

    officeScene.destroy();
  });

  it("풍경 전환 후 오피스로 돌아오면 액자가 floorLayer에 다시 부착되고 벽 위에 남는다", async () => {
    const { SCENES } = await import("../scenes/scenes");
    const { officeScene, bus } = await makeStartedScene();
    bus.triggerAwardeeChanged({ agentId: "a1", name: "하나", month: "2026-07", hasPortrait: false });

    officeScene.setScene(SCENES.valley); // 액자 아트가 없는 씬으로 나갔다가
    officeScene.setScene(SCENES.office); // 다시 오피스로 복귀

    const internals = officeScene as unknown as Internals;
    const frameRoot = internals.awardFrame?.root;
    expect(frameRoot?.visible).toBe(true);
    const children = internals.floorLayer.children;
    expect(children).toContain(frameRoot);
    expect(children.indexOf(frameRoot)).toBeGreaterThan(0);

    officeScene.destroy();
  });
});

describe("bindHoverGate (오버레이 뒤 캐릭터 hover 차단)", () => {
  it("초기에는 move를 끄고, 캔버스 enter/leave로 토글한다", () => {
    const features = { move: true };
    const canvas = document.createElement("canvas");
    const off = bindHoverGate(features, canvas);

    // 첫 pointerenter 전에는 포인터 위치를 모르므로 차단 상태로 시작.
    expect(features.move).toBe(false);

    canvas.dispatchEvent(new Event("pointerenter"));
    expect(features.move).toBe(true);

    // 커서가 캔버스를 덮은 오버레이로 넘어가면 leave → 히트테스트 차단.
    canvas.dispatchEvent(new Event("pointerleave"));
    expect(features.move).toBe(false);

    off();
  });

  it("dispose 후에는 enter가 와도 move를 건드리지 않는다", () => {
    const features = { move: true };
    const canvas = document.createElement("canvas");
    const off = bindHoverGate(features, canvas);
    off();

    canvas.dispatchEvent(new Event("pointerenter"));
    expect(features.move).toBe(false);
  });
});
