// src/renderer/office/world/__tests__/OfficeWorld.test.ts
//
// Tests for OfficeWorld: wires the A/C event
// bus to per-agent entities, diff-syncs entities against the profile list,
// and relays entity clicks back out through the bus.
//
// `createCharacterAssets` (gen/characterFactory.ts) needs `Texture.from()` on
// a real `<canvas>`, unavailable in this vitest environment (same reasoning
// as 3G's `entities/__tests__/helpers.ts`) — so the whole module is mocked
// to hand back `CharacterEntity`-compatible fake assets built from
// `BufferImageSource` (real Pixi `Texture`s, no canvas/WebGL). `bus.ts`'s
// `createMockOfficeBus` (already committed, dependency-free) drives the A->B
// direction and records the B->A direction. `Container`/`Sprite` are real
// per 3E/3G precedent — only the canvas-touching factory is mocked.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BufferImageSource, Container, Texture } from "pixi.js";

import type { CharacterAssets } from "../../gen/characterFactory";
import { setSpriteOverride, resetSpriteOverrides } from "../../gen/spriteOverrides";
import { Tile, type OfficeMap } from "../../map/mapData";
import { createMockOfficeBus } from "../../bus";
import type { AgentProfile } from "../../types";
import { CharacterEntity } from "../../entities/CharacterEntity";

const hoisted = vi.hoisted(() => ({ createCharacterAssetsSpy: vi.fn() }));

vi.mock("../../gen/characterFactory", () => ({
  createCharacterAssets: hoisted.createCharacterAssetsSpy,
}));

const { OfficeWorld, appearanceKey } = await import("../OfficeWorld");

const solidTexture = (label: string): Texture =>
  new Texture({
    source: new BufferImageSource({ resource: new Uint8Array([255, 255, 255, 255]), width: 1, height: 1, label }),
    label,
  });

function makeFakeAssets(): CharacterAssets {
  const idle0 = solidTexture("idle0");
  const idle1 = solidTexture("idle1");
  const walk0 = solidTexture("walk0");
  const walk1 = solidTexture("walk1");
  return {
    base: idle0,
    frames: { idle0, idle1, walk0, walk1 },
    idle: [idle0, idle1],
    walk: [walk0, walk1],
    cellSize: 16,
    descriptor: { archetype: "test", hair: "test", clothes: "test", accessory: "test" },
  };
}

/** 5x5 floor map with two desks, mirroring 3G's fixture + desk slots for assignDesks. */
function makeMap(deskCount = 2): OfficeMap {
  const row = (chars: string) => [...chars].map((c) => (c === "W" ? Tile.Wall : Tile.Floor));
  const tiles = [row("WWWWW"), row("WFFFW"), row("WFFFW"), row("WFFFW"), row("WWWWW")];
  const desks = Array.from({ length: deskCount }, (_, i) => ({
    index: i,
    seat: { tx: 1 + i, ty: 2 },
    facing: "up" as const,
  }));
  return { width: 5, height: 5, tiles, desks };
}

const profile = (id: string): AgentProfile => ({ id, name: id, role: "eng", seed: id });

/** `profile()` fixes seed = id; this variant lets appearance-key tests vary seed/spriteUpdatedAt/archetype independently. */
const mkProfile = (overrides: {
  id: string;
  seed: string;
  spriteUpdatedAt?: number;
  archetype?: string;
}): AgentProfile => ({
  id: overrides.id,
  name: overrides.id,
  role: "eng",
  seed: overrides.seed,
  ...(overrides.spriteUpdatedAt !== undefined ? { spriteUpdatedAt: overrides.spriteUpdatedAt } : {}),
  ...(overrides.archetype !== undefined ? { archetype: overrides.archetype } : {}),
});

/** `pointertap` needs a `FederatedPointerEvent`; the handler ignores it. */
const tap = (sprite: { emit(event: string, ...args: unknown[]): boolean }): boolean => sprite.emit("pointertap");

function makeWorld(map: OfficeMap = makeMap()) {
  const bus = createMockOfficeBus();
  const characterLayer = new Container();
  const overlayLayer = new Container();
  const world = new OfficeWorld({ bus, characterLayer, overlayLayer, map });
  return { bus, characterLayer, overlayLayer, world };
}

beforeEach(() => {
  hoisted.createCharacterAssetsSpy.mockReset();
  hoisted.createCharacterAssetsSpy.mockImplementation(() => makeFakeAssets());
});

afterEach(() => {
  vi.clearAllMocks();
  resetSpriteOverrides();
});

describe("OfficeWorld.syncAgents: add", () => {
  it("creates one entity per profile and adds its root to the character layer", () => {
    const { characterLayer, world } = makeWorld();
    world.syncAgents([profile("a"), profile("b")]);
    expect(characterLayer.children.length).toBe(2);
  });

  it("skips profiles that don't get a desk slot (more agents than desks)", () => {
    const { characterLayer, world } = makeWorld(makeMap(1)); // only 1 desk
    world.syncAgents([profile("a"), profile("b")]);
    expect(characterLayer.children.length).toBe(1);
  });
});

describe("OfficeWorld.syncAgents: keep", () => {
  it("does not recreate or re-add an entity that is still present in the next sync", () => {
    const { characterLayer, world } = makeWorld();
    world.syncAgents([profile("a"), profile("b")]);
    const before = [...characterLayer.children];
    hoisted.createCharacterAssetsSpy.mockClear();

    world.syncAgents([profile("a"), profile("b")]);

    expect(hoisted.createCharacterAssetsSpy).not.toHaveBeenCalled();
    expect(characterLayer.children).toEqual(before);
    expect(characterLayer.children[0]).toBe(before[0]);
    expect(characterLayer.children[1]).toBe(before[1]);
  });
});

describe("OfficeWorld.syncAgents: remove", () => {
  it("destroys and drops entities whose profile disappeared", () => {
    const { characterLayer, world } = makeWorld();
    world.syncAgents([profile("a"), profile("b")]);
    const removed = characterLayer.children[1];

    world.syncAgents([profile("a")]);

    expect(characterLayer.children.length).toBe(1);
    expect(removed.destroyed).toBe(true);
  });

  it("removes every entity when synced with an empty profile list", () => {
    const { characterLayer, world } = makeWorld();
    world.syncAgents([profile("a"), profile("b")]);
    world.syncAgents([]);
    expect(characterLayer.children.length).toBe(0);
  });
});

describe("OfficeWorld: bus -> entity (setPending)", () => {
  it("relays onNotificationChanged to the matching entity's exclamation overlay", () => {
    const { bus, characterLayer, world } = makeWorld();
    world.syncAgents([profile("a")]);
    const overlayRoot = characterLayer.children[0].children[1];
    expect(overlayRoot.visible).toBe(false);

    bus.triggerNotificationChanged("a", true);
    expect(overlayRoot.visible).toBe(true);

    bus.triggerNotificationChanged("a", false);
    expect(overlayRoot.visible).toBe(false);
  });

  it("ignores notifications for unknown/not-yet-synced agent ids", () => {
    const { bus, world } = makeWorld();
    world.syncAgents([profile("a")]);
    expect(() => bus.triggerNotificationChanged("ghost", true)).not.toThrow();
  });
});

describe("OfficeWorld: bus -> entity (setSessionActive)", () => {
  it("relays onSessionStateChanged to the matching entity's setSessionActive, mapping starting/running -> true and exited/disposed -> false", () => {
    const { bus, world } = makeWorld();
    world.syncAgents([profile("a")]);
    const spy = vi.spyOn(CharacterEntity.prototype, "setSessionActive");
    spy.mockClear();

    bus.triggerSessionStateChanged("a", "starting");
    expect(spy).toHaveBeenLastCalledWith(true);

    bus.triggerSessionStateChanged("a", "running");
    expect(spy).toHaveBeenLastCalledWith(true);

    bus.triggerSessionStateChanged("a", "exited");
    expect(spy).toHaveBeenLastCalledWith(false);

    bus.triggerSessionStateChanged("a", "disposed");
    expect(spy).toHaveBeenLastCalledWith(false);

    spy.mockRestore();
  });

  it("ignores session-state events for unknown/not-yet-synced agent ids", () => {
    const { bus, world } = makeWorld();
    world.syncAgents([profile("a")]);
    expect(() => bus.triggerSessionStateChanged("ghost", "running")).not.toThrow();
  });

  it("applies a cached session-active state (received before the entity existed) to a newly created entity", () => {
    const { bus, world } = makeWorld();
    const spy = vi.spyOn(CharacterEntity.prototype, "setSessionActive");
    spy.mockClear();

    bus.triggerSessionStateChanged("a", "running"); // arrives before "a" has a synced entity
    world.syncAgents([profile("a")]);

    expect(spy).toHaveBeenCalledWith(true);
    spy.mockRestore();
  });

  it("defaults a newly created entity to inactive when no session-state event was ever seen for it", () => {
    const { world } = makeWorld();
    const spy = vi.spyOn(CharacterEntity.prototype, "setSessionActive");
    spy.mockClear();

    world.syncAgents([profile("a")]);

    expect(spy).toHaveBeenCalledWith(false);
    spy.mockRestore();
  });

  it("cleans the cached session-active state when an agent is removed, so a later re-add starts fresh (inactive)", () => {
    const { bus, world } = makeWorld();
    bus.triggerSessionStateChanged("a", "running");
    world.syncAgents([profile("a")]);
    world.syncAgents([]); // remove "a" -> cache entry should be dropped

    const spy = vi.spyOn(CharacterEntity.prototype, "setSessionActive");
    spy.mockClear();
    world.syncAgents([profile("a")]); // re-add the same id with no fresh session-state event

    expect(spy).toHaveBeenCalledWith(false);
    spy.mockRestore();
  });
});

describe("OfficeWorld: entity -> bus (emitAgentClicked)", () => {
  it("relays a clicked entity's agentId out through bus.emitAgentClicked", () => {
    const { bus, characterLayer, world } = makeWorld();
    world.syncAgents([profile("agent-42")]);
    const sprite = characterLayer.children[0].children[0] as unknown as {
      emit(event: string, ...args: unknown[]): boolean;
    };

    tap(sprite);

    expect(bus.clickedAgentIds).toEqual(["agent-42"]);
  });

  it("wires onClicked exactly once per entity across repeated syncs (no duplicate emits)", () => {
    const { bus, characterLayer, world } = makeWorld();
    world.syncAgents([profile("agent-1")]);
    world.syncAgents([profile("agent-1")]); // unchanged -> must not re-wire
    const sprite = characterLayer.children[0].children[0] as unknown as {
      emit(event: string, ...args: unknown[]): boolean;
    };

    tap(sprite);

    expect(bus.clickedAgentIds).toEqual(["agent-1"]);
  });
});

describe("OfficeWorld.update", () => {
  it("forwards dt to every live entity", () => {
    const { characterLayer, world } = makeWorld();
    world.syncAgents([profile("a")]);
    const zBefore = characterLayer.children[0].zIndex;

    // Large dt with default entity rand (Math-free, seeded from agentId) may
    // or may not move the entity, but zIndex is always refreshed from y on
    // every update() call, so re-asserting equality after a no-op tick still
    // proves update() ran without throwing across the whole entity set.
    expect(() => world.update(16)).not.toThrow();
    expect(characterLayer.children[0].zIndex).toBe(characterLayer.children[0].y);
    expect(typeof zBefore).toBe("number");
  });
});

describe("외형 키 변경 시 엔티티 재생성", () => {
  it("seed가 바뀐 기존 에이전트는 파괴 후 재생성한다", () => {
    const { world } = makeWorld();
    const p = mkProfile({ id: "a1", seed: "s1" });
    world.syncAgents([p]);
    expect(hoisted.createCharacterAssetsSpy).toHaveBeenCalledTimes(1);

    world.syncAgents([{ ...p, seed: "s2" }]);
    expect(hoisted.createCharacterAssetsSpy).toHaveBeenCalledTimes(2);
  });

  it("spriteUpdatedAt 변경도 재생성을 유발한다", () => {
    const { world } = makeWorld();
    const p = mkProfile({ id: "a1", seed: "s1" });
    world.syncAgents([p]);
    world.syncAgents([{ ...p, spriteUpdatedAt: 123 }]);
    expect(hoisted.createCharacterAssetsSpy).toHaveBeenCalledTimes(2);
  });

  it("오버라이드 등록 자체(프로필 불변)도 키에 반영되어 재생성한다", () => {
    const { world } = makeWorld();
    const p = mkProfile({ id: "a1", seed: "s1", spriteUpdatedAt: 123 });
    world.syncAgents([p]);
    setSpriteOverride("a1", {} as CanvasImageSource);
    world.syncAgents([p]); // 프로필은 동일하지만 override 유무가 바뀜
    expect(hoisted.createCharacterAssetsSpy).toHaveBeenCalledTimes(2);
  });

  it("변화가 없으면 재생성하지 않는다", () => {
    const { world } = makeWorld();
    const p = mkProfile({ id: "a1", seed: "s1" });
    world.syncAgents([p]);
    world.syncAgents([p]);
    expect(hoisted.createCharacterAssetsSpy).toHaveBeenCalledTimes(1);
  });

  it("archetype이 바뀐 기존 에이전트는 파괴 후 재생성한다", () => {
    const { world } = makeWorld();
    const p = mkProfile({ id: "a1", seed: "s1", archetype: "human" });
    world.syncAgents([p]);
    expect(hoisted.createCharacterAssetsSpy).toHaveBeenCalledTimes(1);

    world.syncAgents([{ ...p, archetype: "orc" }]);
    expect(hoisted.createCharacterAssetsSpy).toHaveBeenCalledTimes(2);
  });
});

describe("appearanceKey: archetype 포함", () => {
  const base = mkProfile({ id: "a1", seed: "s1", archetype: "human" });

  it("archetype이 바뀌면 다른 키를 반환한다 (엔티티 재생성 강제)", () => {
    const human = appearanceKey(base);
    const orc = appearanceKey({ ...base, archetype: "orc" });
    expect(human).not.toBe(orc);
  });

  it("archetype 미지정은 'human'과 동일한 키를 반환한다", () => {
    const { archetype: _omit, ...withoutArchetype } = base;
    const missing = appearanceKey(withoutArchetype as AgentProfile);
    expect(missing).toBe(appearanceKey(base));
  });

  it("seed / spriteUpdatedAt 변화에도 여전히 반응한다", () => {
    expect(appearanceKey(base)).not.toBe(appearanceKey({ ...base, seed: "s2" }));
    expect(appearanceKey(base)).not.toBe(appearanceKey({ ...base, spriteUpdatedAt: 5 }));
  });

  it("커스텀 스프라이트 오버라이드 등록 여부도 키에 반영된다", () => {
    const before = appearanceKey(base);
    setSpriteOverride(base.id, {} as CanvasImageSource);
    const after = appearanceKey(base);
    expect(before).not.toBe(after);
  });
});

describe("OfficeWorld.destroy", () => {
  it("destroys all live entities and empties the character layer", () => {
    const { characterLayer, world } = makeWorld();
    world.syncAgents([profile("a"), profile("b")]);
    const entities = [...characterLayer.children];

    world.destroy();

    expect(characterLayer.children.length).toBe(0);
    for (const e of entities) expect(e.destroyed).toBe(true);
  });

  it("unsubscribes from the bus so later notifications are inert", () => {
    const { bus, characterLayer, world } = makeWorld();
    world.syncAgents([profile("a")]);
    world.destroy();
    expect(() => bus.triggerNotificationChanged("a", true)).not.toThrow();
    expect(characterLayer.children.length).toBe(0);
  });

  it("is safe to call twice", () => {
    const { world } = makeWorld();
    world.syncAgents([profile("a")]);
    world.destroy();
    expect(() => world.destroy()).not.toThrow();
  });
});
