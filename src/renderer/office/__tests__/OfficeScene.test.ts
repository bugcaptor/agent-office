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
  rendererSize: { width: 320, height: 224 },
}));

vi.mock("pixi.js", async (importOriginal) => {
  const actual = await importOriginal<typeof import("pixi.js")>();

  class FakeApplication {
    stage = new actual.Container();
    renderer: { width: number; height: number } | undefined;
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
const { OfficeScene, computeIntegerScale } = await import("../OfficeScene");
const { createMockOfficeBus } = await import("../bus");
const { OfficeWorld } = await import("../world/OfficeWorld");

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
  state.rendererSize = { width: 320, height: 224 };
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
