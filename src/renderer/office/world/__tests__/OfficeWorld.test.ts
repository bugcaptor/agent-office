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
import { OFFICE_MAP, QUEUE_SLOTS, Tile, TILE_SIZE, type OfficeMap } from "../../map/mapData";
import { tileCenterPx } from "../pathing";
import { createMockOfficeBus } from "../../bus";
import type { AgentProfile } from "../../types";
import { CharacterEntity } from "../../entities/CharacterEntity";

const hoisted = vi.hoisted(() => ({ createCharacterAssetsSpy: vi.fn() }));

vi.mock("../../gen/characterFactory", () => ({
  createCharacterAssets: hoisted.createCharacterAssetsSpy,
}));

const { OfficeWorld, appearanceKey, LABEL_ANCHOR_OFFSET_Y } = await import("../OfficeWorld");

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

/** id -> current world position, via the only public per-id position accessor
 * (`collectLabelAnchors`); undoes its head-offset so callers get raw root x/y. */
function posOf(world: InstanceType<typeof OfficeWorld>, id: string): { x: number; y: number } {
  const anchors = new Map<string, { x: number; y: number }>();
  world.collectLabelAnchors(anchors);
  const a = anchors.get(id);
  if (!a) throw new Error(`posOf: no live entity for id "${id}"`);
  return { x: Math.round(a.x), y: Math.round(a.y + LABEL_ANCHOR_OFFSET_Y) };
}

/** Runs enough 16ms ticks for any in-map walk (even boss-queue-slot length) to finish.
 * 가장 긴 맵 횡단도 수백 tick이면 끝난다 — 1000이면 넉넉한 여유. */
function settle(world: InstanceType<typeof OfficeWorld>): void {
  for (let i = 0; i < 1000; i++) world.update(16);
}

const queuePos = (slot: number) => {
  const t = tileCenterPx(QUEUE_SLOTS[slot]);
  return { x: Math.round(t.x), y: Math.round(t.y + TILE_SIZE / 2) };
};

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

describe("OfficeWorld.syncAgents: 수동 책상 지정(assignedDeskIndex)", () => {
  const withDesk = (id: string, deskIndex: number): AgentProfile => ({
    ...profile(id),
    assignedDeskIndex: deskIndex,
  });

  it("지정된 에이전트는 정확히 그 책상 좌석에 앉는다", () => {
    const map = makeMap(2);
    const { characterLayer, world } = makeWorld(map);
    world.syncAgents([withDesk("a", 1)]);
    const seat = map.desks[1].seat;
    const entity = characterLayer.children[0];
    expect(entity.x).toBe(seat.tx * 16 + 8);
  });

  it("지정이 바뀌면 기존 엔티티를 재생성하지 않고 setSeat으로 옮긴다", () => {
    const map = makeMap(2);
    const { characterLayer, world } = makeWorld(map);
    world.syncAgents([withDesk("a", 0)]);
    const entity = characterLayer.children[0];
    const spy = vi.spyOn(CharacterEntity.prototype, "setSeat");
    hoisted.createCharacterAssetsSpy.mockClear();

    world.syncAgents([withDesk("a", 1)]);

    expect(hoisted.createCharacterAssetsSpy).not.toHaveBeenCalled();
    expect(characterLayer.children[0]).toBe(entity);
    expect(spy).toHaveBeenCalledWith(map.desks[1].seat);
    spy.mockRestore();
  });

  it("지정된 책상은 자동 배정 풀에서 제외된다 (자리 잃은 엔티티는 파괴)", () => {
    const map = makeMap(1); // 책상 1개뿐
    const { characterLayer, world } = makeWorld(map);
    world.syncAgents([profile("a")]); // a가 자동 배정으로 유일한 책상 차지
    const aEntity = characterLayer.children[0];

    // b가 그 책상을 수동 지정받으면 a는 자리를 잃고 엔티티가 사라진다.
    world.syncAgents([profile("a"), withDesk("b", 0)]);

    expect(characterLayer.children.length).toBe(1);
    expect(aEntity.destroyed).toBe(true);
    const seat = map.desks[0].seat;
    expect(characterLayer.children[0].x).toBe(seat.tx * 16 + 8);
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

describe("OfficeWorld: 외형 재생성 시 pending 복원 (Finding 2)", () => {
  it("pending 중 외형 키가 바뀌어도 재생성된 엔티티에 pending(! 오버레이)이 복원된다", () => {
    const { bus, characterLayer, world } = makeWorld();
    const p = mkProfile({ id: "a1", seed: "s1" });
    world.syncAgents([p]);
    bus.triggerNotificationChanged("a1", true);
    const overlayBefore = characterLayer.children[0].children[1];
    expect(overlayBefore.visible).toBe(true);

    world.syncAgents([{ ...p, seed: "s2" }]); // 외형 키 변경 -> 파괴 후 재생성

    const overlayAfter = characterLayer.children[0].children[1];
    expect(overlayAfter.visible).toBe(true);
  });
});

describe("OfficeWorld: 책상 없이 대기 중이던 에이전트의 pendingIds 정리 (Finding 3)", () => {
  it("엔티티 없이 대기 중이던 에이전트가 프로필에서 사라지면 pendingIds에서도 제거된다", () => {
    const { bus, characterLayer, world } = makeWorld(makeMap(1)); // 책상 1개
    world.syncAgents([profile("a"), profile("extra")]); // "extra"는 책상 없음 → 엔티티 없음
    bus.triggerNotificationChanged("extra", true);

    world.syncAgents([profile("a")]); // extra 완전 퇴장
    world.syncAgents([profile("extra")]); // 재등장 — 이번엔 책상을 받아 엔티티 생성

    // sweep이 안 됐다면 stale pendingIds로 "!"가 잘못 복원된다.
    const overlay = characterLayer.children[0].children[1] as { visible: boolean };
    expect(overlay.visible).toBe(false);
  });
});

describe("OfficeWorld: bus -> entity (setSubagentCount)", () => {
  it("버스의 subagent count 변화를 해당 엔티티 미니 표시에 반영한다", () => {
    const bus = createMockOfficeBus();
    const characterLayer = new Container();
    const world = new OfficeWorld({ bus, characterLayer, overlayLayer: new Container(), map: makeMap() });
    world.syncAgents([profile("p1")]);

    bus.triggerSubagentCountChanged("p1", 2);

    // 엔티티 root = characterLayer의 첫 자식; 미니 루트 = 그 root의 children[3].
    const entityRoot = characterLayer.children[0] as unknown as { children: { children: { visible: boolean }[] }[] };
    const miniRoot = entityRoot.children[3];
    expect(miniRoot.children.filter((s) => s.visible).length).toBe(2);

    world.destroy();
  });

  it("카운트가 온 뒤 외형 변경으로 엔티티가 재생성돼도 카운트가 재적용된다", () => {
    const bus = createMockOfficeBus();
    const characterLayer = new Container();
    const world = new OfficeWorld({ bus, characterLayer, overlayLayer: new Container(), map: makeMap() });
    const p = profile("p1");
    world.syncAgents([p]);
    bus.triggerSubagentCountChanged("p1", 2);

    // seed 변경 → appearanceKey 변경 → 엔티티 재생성.
    world.syncAgents([{ ...p, seed: p.seed + "x" }]);

    const entityRoot = characterLayer.children[0] as unknown as { children: { children: { visible: boolean }[] }[] };
    expect(entityRoot.children[3].children.filter((s) => s.visible).length).toBe(2);

    world.destroy();
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

describe("OfficeWorld.setRenderScale (이슈 #47)", () => {
  it("커스텀 시트 엔티티만 S 변경 시 재프리필터한다(절차 생성은 불변)", () => {
    const { world } = makeWorld();
    setSpriteOverride("a", { height: 256 } as unknown as CanvasImageSource);
    world.syncAgents([profile("a"), profile("b")]); // a=커스텀, b=절차 생성
    hoisted.createCharacterAssetsSpy.mockClear();

    world.setRenderScale(5); // 기본 3 → 5

    expect(hoisted.createCharacterAssetsSpy).toHaveBeenCalledTimes(1); // a만
    expect(hoisted.createCharacterAssetsSpy).toHaveBeenCalledWith(
      expect.objectContaining({ id: "a" }),
      5,
    );
  });

  it("정수 S가 그대로면 no-op", () => {
    const { world } = makeWorld();
    setSpriteOverride("a", { height: 256 } as unknown as CanvasImageSource);
    world.syncAgents([profile("a")]);
    hoisted.createCharacterAssetsSpy.mockClear();

    world.setRenderScale(3.2); // round(3.2)=3 == 기본 3

    expect(hoisted.createCharacterAssetsSpy).not.toHaveBeenCalled();
  });

  it("목표 해상도 D가 불변이면 재생성하지 않는다", () => {
    const { world } = makeWorld();
    setSpriteOverride("a", { height: 256 } as unknown as CanvasImageSource);
    world.syncAgents([profile("a")]); // fake assets cellSize=16
    hoisted.createCharacterAssetsSpy.mockClear();

    world.setRenderScale(1); // detailCellSize(256,1)=16 == entity.cellSize(16)

    expect(hoisted.createCharacterAssetsSpy).not.toHaveBeenCalled();
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

// 실제 OFFICE_MAP 사용 — 픽스처 맵에서는 QUEUE_SLOTS 좌표가 map-rect 클램프에 걸린다.
describe("boss desk queue orchestration", () => {
  it("알림 대기 → 도착 순서대로 줄 슬롯 0,1 배정", () => {
    const { world, bus } = makeWorld(OFFICE_MAP);
    world.syncAgents([profile("a"), profile("b")]);
    bus.triggerNotificationChanged("a", true);
    bus.triggerNotificationChanged("b", true);
    settle(world);
    expect(posOf(world, "a")).toEqual(queuePos(0));
    expect(posOf(world, "b")).toEqual(queuePos(1));
  });

  it("알림 해제 → 줄에서 빠지고 뒷사람이 슬롯 0으로 당겨진다", () => {
    const { world, bus } = makeWorld(OFFICE_MAP);
    world.syncAgents([profile("a"), profile("b")]);
    bus.triggerNotificationChanged("a", true);
    bus.triggerNotificationChanged("b", true);
    settle(world);
    bus.triggerNotificationChanged("a", false);
    settle(world);
    expect(posOf(world, "b")).toEqual(queuePos(0));
    expect(posOf(world, "a")).not.toEqual(queuePos(0)); // a는 자리로 복귀
  });

  it("휴가 모드 on → 전원 줄 이탈, off → 대기 중 에이전트 재배정", () => {
    const { world, bus } = makeWorld(OFFICE_MAP);
    world.syncAgents([profile("a")]);
    bus.triggerNotificationChanged("a", true);
    settle(world);
    bus.triggerVacationModeChanged(true);
    settle(world);
    expect(posOf(world, "a")).not.toEqual(queuePos(0));
    bus.triggerVacationModeChanged(false);
    settle(world);
    expect(posOf(world, "a")).toEqual(queuePos(0));
  });

  it("syncAgents로 제거된 에이전트는 큐에서도 빠진다", () => {
    const { world, bus } = makeWorld(OFFICE_MAP);
    world.syncAgents([profile("a"), profile("b")]);
    bus.triggerNotificationChanged("a", true);
    bus.triggerNotificationChanged("b", true);
    settle(world);
    world.syncAgents([profile("b")]); // a 퇴장
    settle(world);
    expect(posOf(world, "b")).toEqual(queuePos(0));
  });

  it("외형 키 변경으로 재생성된 엔티티는 큐 멤버십을 유지한다", () => {
    const { world, bus } = makeWorld(OFFICE_MAP);
    const p = mkProfile({ id: "a", seed: "s1" });
    world.syncAgents([p]);
    bus.triggerNotificationChanged("a", true);
    settle(world);
    expect(posOf(world, "a")).toEqual(queuePos(0));

    world.syncAgents([{ ...p, seed: "s2" }]); // 외형 키 변경 -> 파괴 후 재생성
    settle(world);

    expect(posOf(world, "a")).toEqual(queuePos(0));
  });

  it("책상이 없어(엔티티 없음) 대기 중인 에이전트는 줄 슬롯을 차지하지 않는다", () => {
    const { world, bus } = makeWorld(OFFICE_MAP);
    // 책상 8개를 전부 수동 지정으로 채워 extra는 자리를 못 받게 한다.
    const desked = Array.from({ length: 8 }, (_, i) => ({ ...profile(`d${i}`), assignedDeskIndex: i }));
    const extra = profile("extra");
    world.syncAgents([...desked, extra]);

    bus.triggerNotificationChanged("extra", true);
    bus.triggerNotificationChanged("d0", true);
    settle(world);

    expect(() => posOf(world, "extra")).toThrow();
    expect(posOf(world, "d0")).toEqual(queuePos(0)); // 유령 슬롯 없음 — d0이 슬롯 0
  });

  it("대기 중 책상을 잃었다가(엔티티 파괴) 되찾으면(엔티티 재생성) 줄로 복귀한다", () => {
    const { world, bus } = makeWorld(OFFICE_MAP);
    const d = Array.from({ length: 7 }, (_, i) => ({ ...profile(`d${i + 1}`), assignedDeskIndex: i + 1 }));
    const a = { ...profile("a"), assignedDeskIndex: 0 };
    world.syncAgents([a, ...d]);

    bus.triggerNotificationChanged("a", true);
    settle(world);
    expect(posOf(world, "a")).toEqual(queuePos(0));

    // "0z" < "a" 정렬이라 blocker의 수동 지정이 desk 0을 뺏는다(폴백 책상도 전부 점유됨).
    const blocker = { ...profile("0z"), assignedDeskIndex: 0 };
    world.syncAgents([a, ...d, blocker]);
    settle(world);
    expect(() => posOf(world, "a")).toThrow();

    // blocker 제거 → 재생성. pending 유지라 recomputeQueue가 큐 멤버십을 복원해야 한다.
    world.syncAgents([a, ...d]);
    settle(world);

    expect(posOf(world, "a")).toEqual(queuePos(0));
  });
});
