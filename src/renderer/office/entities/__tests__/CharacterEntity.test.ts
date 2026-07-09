// src/renderer/office/entities/__tests__/CharacterEntity.test.ts
//
// TDD for CharacterEntity: wires the FSM + pathing + character assets into a
// Pixi display object — seated placement, movement execution toward a
// target, sprite flip, manual frame-swap animation, click hit test, the
// exclamation overlay toggle, and (Phase B office redesign) session-active
// gating of the sit/break-room behavior plus the "..." thinking bubble.
//
// Only `rand: () => number` and `dt` (ms) drive time/randomness — no
// `Math.random`, no real timers.
//
// Two map fixtures are used:
// - `makeMap()`: a tiny 5x5 Wall/Floor map with no break-room tiles in
//   range of `BREAK_ROOM_RECT` (a module-level constant, independent of the
//   map passed in) — used for tests that don't need an actual walk to
//   resolve (construction, click, pending-pin, idle animation, clamp) and
//   for the defensive "no reachable break tile" fallback test.
// - `OFFICE_MAP` (the real map, from `map/mapData.ts`): used for tests that
//   exercise an actual sitting <-> break-room round trip, since
//   `pickBreakTarget` targets the real `BREAK_ROOM_RECT` coordinates, which
//   only exist on the real map.
//
// Real `Container`/`Sprite`/`Graphics` are used — they construct and mutate
// fine without a live Pixi `Application`/renderer. Textures come from
// `./helpers.ts`'s `BufferImageSource`-backed stand-ins.

import { describe, expect, it, vi } from "vitest";
import { Rectangle, type Sprite } from "pixi.js";

import { CharacterEntity } from "../CharacterEntity";
import { makeTestCharacterAssets } from "./helpers";
import { OFFICE_MAP, Tile, TILE_SIZE, type OfficeMap } from "../../map/mapData";
import { tileCenterPx } from "../../world/pathing";

/** 5x5 map: Wall border, Floor interior (tx/ty in [1,3]). No break-room tiles reachable. */
const makeMap = (): OfficeMap => {
  const row = (chars: string) => [...chars].map((c) => (c === "W" ? Tile.Wall : Tile.Floor));
  const tiles = [row("WWWWW"), row("WFFFW"), row("WFFFW"), row("WFFFW"), row("WWWWW")];
  return { width: 5, height: 5, tiles, desks: [] };
};

const SEAT = { tx: 2, ty: 2 };

// A real desk seat on OFFICE_MAP (ty=2 row's first DeskTop pair -> seat below at ty=3).
const OFFICE_SEAT = { tx: 2, ty: 3 };

/** Pops values off a fixed queue; returns `fallback` once exhausted. */
const queueRand = (values: number[], fallback = 0.999): (() => number) => {
  const q = [...values];
  return () => (q.length ? q.shift()! : fallback);
};

const spriteOf = (e: CharacterEntity): Sprite => e.root.children[0] as Sprite;
const overlayRootOf = (e: CharacterEntity) => e.root.children[1];
const thinkOverlayRootOf = (e: CharacterEntity) => e.root.children[2];

/** `pointertap` is typed as requiring a `FederatedPointerEvent` payload; the
 * handler under test ignores it, so tests fire it as a plain untyped emitter
 * call instead of constructing a real federated event. */
const tap = (sprite: Sprite): boolean =>
  (sprite as unknown as { emit(event: string, ...args: unknown[]): boolean }).emit("pointertap");

describe("CharacterEntity: construction / seated placement", () => {
  it("places root at the seat's pixel center, offset to feet-at-seat-bottom, with zIndex = y", () => {
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, makeMap(), () => 0.5);
    const seatCenter = tileCenterPx(SEAT);
    expect(e.root.x).toBe(seatCenter.x);
    expect(e.root.y).toBe(seatCenter.y + TILE_SIZE / 2);
    expect(e.root.zIndex).toBe(e.root.y);
  });

  it("anchors the sprite at feet-center (0.5, 1) and starts on the first idle frame", () => {
    const assets = makeTestCharacterAssets();
    const e = new CharacterEntity("agent-1", assets, SEAT, makeMap(), () => 0.5);
    const sprite = spriteOf(e);
    expect(sprite.anchor.x).toBe(0.5);
    expect(sprite.anchor.y).toBe(1);
    expect(sprite.texture).toBe(assets.idle[0]);
  });

  it("starts with the exclamation overlay and the thinking bubble hidden", () => {
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, makeMap(), () => 0.5);
    expect(overlayRootOf(e).visible).toBe(false);
    expect(thinkOverlayRootOf(e).visible).toBe(false);
  });

  it("makes the sprite interactive with a pointer cursor and an explicit hit area", () => {
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, makeMap(), () => 0.5);
    const sprite = spriteOf(e);
    expect(sprite.eventMode).toBe("static");
    expect(sprite.cursor).toBe("pointer");
    expect(sprite.hitArea).not.toBeNull();
  });

  it("sizes the hit area to cellSize=16 for procedural assets (legacy default, unchanged)", () => {
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, makeMap(), () => 0.5);
    const hitArea = spriteOf(e).hitArea as Rectangle;
    expect(hitArea).toEqual(new Rectangle(-8, -16, 16, 16));
  });

  it("sizes the hit area to the full local-space sprite (cellSize, not the apparent 16px) for a high-res custom sheet — Pixi hitArea is evaluated in local (pre-scale) coordinates, so sizing it to the apparent 16px would shrink the on-screen click/hover target to ~1x1px once `sprite.scale` = 16/cellSize is applied", () => {
    const assets = { ...makeTestCharacterAssets(), cellSize: 64 };
    const e = new CharacterEntity("agent-1", assets, SEAT, makeMap(), () => 0.5);
    const hitArea = spriteOf(e).hitArea as Rectangle;
    expect(hitArea).toEqual(new Rectangle(-32, -64, 64, 64));
  });

  it("스프라이트를 16/cellSize로 스케일해 겉보기 크기를 16px로 유지한다", () => {
    const assets = { ...makeTestCharacterAssets(), cellSize: 64 };
    const e = new CharacterEntity("agent-1", assets, SEAT, makeMap(), () => 0.5);
    const sprite = spriteOf(e);
    expect(sprite.scale.x).toBeCloseTo(16 / 64);
    expect(sprite.scale.y).toBeCloseTo(16 / 64);
  });
});

describe("CharacterEntity: click hit test", () => {
  it("invokes the onClicked callback with this entity's agentId on a pointertap", () => {
    const e = new CharacterEntity("agent-42", makeTestCharacterAssets(), SEAT, makeMap(), () => 0.5);
    const cb = vi.fn();
    e.onClicked(cb);
    tap(spriteOf(e));
    expect(cb).toHaveBeenCalledTimes(1);
    expect(cb).toHaveBeenCalledWith("agent-42");
  });
});

describe("CharacterEntity: setPending / exclamation overlay", () => {
  it("shows the overlay on setPending(true) and hides it on setPending(false)", () => {
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, makeMap(), () => 0.5);
    e.setPending(true);
    expect(overlayRootOf(e).visible).toBe(true);
    e.setPending(false);
    expect(overlayRootOf(e).visible).toBe(false);
  });

  it("keeps the character sitting at its seat forever while a notification is pending, regardless of dt/rand", () => {
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, makeMap(), () => 0);
    const seatX = e.root.x;
    const seatY = e.root.y;
    e.setPending(true);
    e.update(20_000); // way past the linger time, and would otherwise leave for the break room
    e.update(20_000);
    expect(e.root.x).toBe(seatX);
    expect(e.root.y).toBe(seatY);
  });
});

describe("CharacterEntity: setSessionActive", () => {
  it("keeps the character sitting at its seat forever while the session is active, regardless of dt/rand", () => {
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, makeMap(), () => 0);
    const seatX = e.root.x;
    const seatY = e.root.y;
    e.setSessionActive(true);
    e.update(20_000); // way past the linger time
    e.update(20_000);
    expect(e.root.x).toBe(seatX);
    expect(e.root.y).toBe(seatY);
  });

  it("does not transition out of sitting before the linger time elapses, even once inactive", () => {
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, makeMap(), () => 0);
    const seatX = e.root.x;
    const seatY = e.root.y;
    e.update(100); // well under the linger time (2000ms)
    expect(e.root.x).toBe(seatX);
    expect(e.root.y).toBe(seatY);
  });

  it("restarts the linger from the moment the session deactivates — a long-seated character does not bolt instantly", () => {
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, OFFICE_MAP, () => 0.5);
    const seatX = e.root.x;
    const seatY = e.root.y;
    e.setSessionActive(true);
    e.update(60_000); // long focused session — stateTimer accrues way past the linger
    e.setSessionActive(false);
    e.update(100); // linger must restart from deactivation, not carry the accrued timer
    expect(e.root.x).toBe(seatX);
    expect(e.root.y).toBe(seatY);
    e.update(2001); // now past a fresh linger window -> heads to the break room
    e.update(1000);
    expect(e.root.x !== seatX || e.root.y !== seatY).toBe(true);
  });

  it("restarts the linger when a pending notification clears while seated and inactive", () => {
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, OFFICE_MAP, () => 0.5);
    const seatX = e.root.x;
    const seatY = e.root.y;
    e.setPending(true);
    e.update(60_000); // pinned by the pending notification
    e.setPending(false);
    e.update(100);
    expect(e.root.x).toBe(seatX);
    expect(e.root.y).toBe(seatY);
  });

  it("stays put if no break-room tile is reachable on this map (defensive fallback when pickBreakTarget fails)", () => {
    // makeMap()'s 5x5 grid has no tiles at BREAK_ROOM_RECT's real coordinates
    // -> pickBreakTarget always returns null on it.
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, makeMap(), () => 0.5);
    const seatX = e.root.x;
    const seatY = e.root.y;
    e.update(2001); // crosses the linger threshold -> requests a break target, but none is reachable
    expect(e.root.x).toBe(seatX);
    expect(e.root.y).toBe(seatY);
  });
});

describe("CharacterEntity: FSM + movement wiring (break-room round trip, OFFICE_MAP)", () => {
  it("walks to the break room once inactive past the linger, then returns to the seat once the session activates", () => {
    // rand sequence, solved by hand against stepBehavior/pickBreakTarget:
    //   [0.5] tick1 FSM ctx draw (unused: sitting's transition is deterministic).
    //   [0]   pickBreakTarget tx: floor(0*7) = 0 -> tx = rect.x + 0 = 11.
    //   [0]   pickBreakTarget ty: floor(0*2) = 0 -> ty = rect.y + 0 = 10.
    // (11,10) is Rug (walkable) -> first attempt succeeds, target = {tx:11, ty:10}.
    const rand = queueRand([0.5, 0, 0]);
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), OFFICE_SEAT, OFFICE_MAP, rand);

    const seatPx = tileCenterPx(OFFICE_SEAT);
    const seatFeet = { x: seatPx.x, y: seatPx.y + TILE_SIZE / 2 };
    const breakPx = tileCenterPx({ tx: 11, ty: 10 });
    const breakFeet = { x: breakPx.x, y: breakPx.y + TILE_SIZE / 2 };

    // Tick 1: dt=8000ms both clears the 2000ms linger *and* covers the whole
    // seat->break-room hop distance in one step (28px/s * 8s >> the hop), so
    // the break leg both starts and arrives within this single update().
    e.update(8000);
    expect(e.root.x).toBe(breakFeet.x);
    expect(e.root.y).toBe(breakFeet.y);

    // Tick 2: activating the session while breakIdle requests an immediate,
    // deterministic return to the seat (no timeout/rand involved) — again
    // covered in one step by a large dt.
    e.setSessionActive(true);
    e.update(8000);
    expect(e.root.x).toBe(seatFeet.x);
    expect(e.root.y).toBe(seatFeet.y);
  });

  it("stays in the break room indefinitely while still inactive (no auto-return-by-timeout)", () => {
    const rand = queueRand([0.5, 0, 0], 0.999999); // arrive at break, then always-unfavorable stroll rand
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), OFFICE_SEAT, OFFICE_MAP, rand);
    e.update(8000); // arrives at the break room, becomes breakIdle
    const breakX = e.root.x;
    const breakY = e.root.y;

    // NOTE: dt must stay modest here — the per-tick stroll probability
    // approaches 1 as dtMs grows (it's `1 - (1 - chance)^(dtMs/1000)`), so an
    // arbitrarily large dt would trigger the stroll no matter how close to 1
    // `rand` is. A few seconds keeps the probability comfortably below the
    // fallback rand (0.999999).
    e.update(3000); // several seconds idle, still inactive, unfavorable stroll rand
    expect(e.root.x).toBe(breakX);
    expect(e.root.y).toBe(breakY);
  });

  it("retargets to the seat if the session activates mid-stroll toward the break room, instead of finishing the stroll", () => {
    const rand = queueRand([0.5, 0, 0]); // tick1 ctx (unused) + pickBreakTarget tx=11, ty=10
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), OFFICE_SEAT, OFFICE_MAP, rand);

    e.update(2001); // crosses the linger threshold with a *small* dt -> partial step toward the break target
    const seatFeetPx = tileCenterPx(OFFICE_SEAT);
    const seatFeet = { x: seatFeetPx.x, y: seatFeetPx.y + TILE_SIZE / 2 };
    expect(e.root.x).not.toBe(seatFeet.x); // moved away from the seat already

    e.setSessionActive(true);
    e.update(20_000); // large dt: whatever the (retargeted) target is, arrive at it this tick
    expect(e.root.x).toBe(seatFeet.x);
    expect(e.root.y).toBe(seatFeet.y);
  });

  it("sets scale.x to +1 (unflipped) while walking rightward toward the break room", () => {
    const rand = queueRand([0.5, 0, 0]); // pickBreakTarget -> (tx:11, ty:10), to the right of OFFICE_SEAT (tx:2)
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), OFFICE_SEAT, OFFICE_MAP, rand);
    e.update(2001); // small dt: partial step, doesn't arrive
    expect(spriteOf(e).scale.x).toBe(1);
  });

  it("sets scale.x to -1 (flipped) while walking leftward back toward the seat", () => {
    const rand = queueRand([0.5, 0, 0]); // arrive at break in one large-dt tick
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), OFFICE_SEAT, OFFICE_MAP, rand);
    e.update(8000); // arrives at the break room

    e.setSessionActive(true);
    e.update(16); // small dt: retarget to seat (to the left) + partial step
    expect(spriteOf(e).scale.x).toBe(-1);
  });

  it("고해상도(cellSize=64) 시트는 좌향 이동 시 x부호만 반전하고 배율은 유지한다", () => {
    const rand = queueRand([0.5, 0, 0]);
    const assets = { ...makeTestCharacterAssets(), cellSize: 64 };
    const e = new CharacterEntity("agent-1", assets, OFFICE_SEAT, OFFICE_MAP, rand);
    e.update(8000); // arrives at the break room
    e.setSessionActive(true);
    e.update(16); // 좌측(자리)으로 소폭 이동(도착 전)
    expect(spriteOf(e).scale.x).toBeCloseTo(-(16 / 64));
    expect(spriteOf(e).scale.y).toBeCloseTo(16 / 64);
  });
});

describe("CharacterEntity: break-room tile reservations (no overlap while resting)", () => {
  const feetOf = (t: { tx: number; ty: number }) => {
    const c = tileCenterPx(t);
    return { x: c.x, y: c.y + TILE_SIZE / 2 };
  };

  it("two characters sharing a reservation set never rest on the same tile, even with colliding rand streams", () => {
    const reservations = new Set<string>();
    // Both rand streams would pick (11, 10) first (pickBreakTarget draws 0, 0).
    const a = new CharacterEntity(
      "agent-a",
      makeTestCharacterAssets(),
      OFFICE_SEAT,
      OFFICE_MAP,
      queueRand([0.5, 0, 0]),
      reservations,
    );
    // b's first attempt (0, 0) -> (11, 10) is reserved by a; the retry draws
    // (0.2, 0) -> tx = 11 + floor(0.2 * 7) = 12 -> rests at (12, 10).
    const b = new CharacterEntity(
      "agent-b",
      makeTestCharacterAssets(),
      { tx: 3, ty: 3 },
      OFFICE_MAP,
      queueRand([0.5, 0, 0, 0.2, 0]),
      reservations,
    );
    a.update(8000); // linger + full walk in one large-dt tick -> rests at (11, 10)
    b.update(8000);
    expect({ x: a.root.x, y: a.root.y }).toEqual(feetOf({ tx: 11, ty: 10 }));
    expect({ x: b.root.x, y: b.root.y }).toEqual(feetOf({ tx: 12, ty: 10 }));
    expect(reservations).toEqual(new Set(["11,10", "12,10"]));
  });

  it("reserves the target tile as soon as it is picked (before arrival), so a walker also blocks it", () => {
    const reservations = new Set<string>();
    const e = new CharacterEntity(
      "agent-a",
      makeTestCharacterAssets(),
      OFFICE_SEAT,
      OFFICE_MAP,
      queueRand([0.5, 0, 0]),
      reservations,
    );
    e.update(2001); // crosses the linger with a small dt -> picked (11, 10), still walking
    expect(reservations.has("11,10")).toBe(true);
  });

  it("moves its reservation when strolling to another break-room tile", () => {
    const reservations = new Set<string>();
    // Tick 1: [0.5] ctx + [0, 0] pick -> rests at (11, 10).
    // Tick 2: [0] ctx draw < stroll probability for dt=8000 (~0.39) -> stroll;
    //         [0.2, 0] pick -> (12, 10) (own tile (11, 10) is excluded as reserved).
    const e = new CharacterEntity(
      "agent-a",
      makeTestCharacterAssets(),
      OFFICE_SEAT,
      OFFICE_MAP,
      queueRand([0.5, 0, 0, 0, 0.2, 0]),
      reservations,
    );
    e.update(8000);
    expect(reservations).toEqual(new Set(["11,10"]));
    e.update(8000);
    expect(reservations).toEqual(new Set(["12,10"]));
  });

  it("releases its break tile when heading back to the desk", () => {
    const reservations = new Set<string>();
    const e = new CharacterEntity(
      "agent-a",
      makeTestCharacterAssets(),
      OFFICE_SEAT,
      OFFICE_MAP,
      queueRand([0.5, 0, 0]),
      reservations,
    );
    e.update(8000); // rests at (11, 10)
    e.setSessionActive(true);
    e.update(8000); // returns to the seat
    expect(reservations.size).toBe(0);
  });

  it("releases its break tile on destroy", () => {
    const reservations = new Set<string>();
    const e = new CharacterEntity(
      "agent-a",
      makeTestCharacterAssets(),
      OFFICE_SEAT,
      OFFICE_MAP,
      queueRand([0.5, 0, 0]),
      reservations,
    );
    e.update(8000);
    expect(reservations.size).toBe(1);
    e.destroy();
    expect(reservations.size).toBe(0);
  });

  it("stays seated (and retries later) when every break-room tile is reserved", () => {
    const reservations = new Set<string>();
    for (let ty = 10; ty < 12; ty++) for (let tx = 11; tx < 18; tx++) reservations.add(`${tx},${ty}`);
    const e = new CharacterEntity(
      "agent-a",
      makeTestCharacterAssets(),
      OFFICE_SEAT,
      OFFICE_MAP,
      () => 0.4,
      reservations,
    );
    const seatX = e.root.x;
    const seatY = e.root.y;
    e.update(8000); // wants a break, but the room is full -> stays at its own (unique) seat
    expect(e.root.x).toBe(seatX);
    expect(e.root.y).toBe(seatY);
  });
});

describe("CharacterEntity: idle animation", () => {
  it("swaps between the two idle frames over time while stationary", () => {
    const assets = makeTestCharacterAssets();
    const e = new CharacterEntity("agent-1", assets, SEAT, makeMap(), () => 0.999);
    expect(spriteOf(e).texture).toBe(assets.idle[0]);
    e.update(500); // > ANIM_IDLE_MS (480ms), well under the linger time -> keeps sitting
    expect(spriteOf(e).texture).toBe(assets.idle[1]);
    e.update(500);
    expect(spriteOf(e).texture).toBe(assets.idle[0]);
  });
});

describe("CharacterEntity: '...' thinking bubble", () => {
  it("stays hidden while sitting but the session is inactive", () => {
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, makeMap(), () => 0.5);
    e.update(100);
    expect(thinkOverlayRootOf(e).visible).toBe(false);
  });

  it("stays hidden while a notification is pending, even if the session is active", () => {
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, makeMap(), () => 0);
    e.setSessionActive(true);
    e.setPending(true);
    e.update(10_000);
    expect(thinkOverlayRootOf(e).visible).toBe(false);
  });

  it("cycles hidden -> visible -> hidden on a dt-accumulator cadence while sitting+active+idle", () => {
    // rand = 0 always -> every duration roll resolves to its MIN
    // (THINK_HIDDEN_MIN_MS=4000, THINK_VISIBLE_MIN_MS=2000).
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, makeMap(), () => 0);
    e.setSessionActive(true);

    e.update(1000); // hidden phase timer: 1000 < 4000
    expect(thinkOverlayRootOf(e).visible).toBe(false);

    e.update(3000); // hidden phase timer: 4000 >= 4000 -> flips to visible, rolls a 2000ms visible duration
    expect(thinkOverlayRootOf(e).visible).toBe(true);

    e.update(1999); // visible phase timer: 1999 < 2000
    expect(thinkOverlayRootOf(e).visible).toBe(true);

    e.update(1); // visible phase timer: 2000 >= 2000 -> flips back to hidden
    expect(thinkOverlayRootOf(e).visible).toBe(false);
  });

  it("hides immediately (mid-visible-phase) once a notification becomes pending", () => {
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, makeMap(), () => 0);
    e.setSessionActive(true);
    e.update(1000);
    e.update(3000); // now visible
    expect(thinkOverlayRootOf(e).visible).toBe(true);

    e.setPending(true);
    expect(thinkOverlayRootOf(e).visible).toBe(false); // instant, even before the next update()
    e.update(1);
    expect(thinkOverlayRootOf(e).visible).toBe(false);
  });
});

describe("CharacterEntity: defensive out-of-bounds clamp", () => {
  it("clamps the root position back into [0, mapPx] on the next update() if something pushed it out of bounds", () => {
    const map = makeMap(); // 5x5 tiles @ TILE_SIZE -> map is 80x80px
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, map, () => 0.999);
    const mapPxW = map.width * TILE_SIZE;

    // Simulate whatever camera/movement bug shoved the character way outside
    // the map rect, then let a (possibly zero-dt) update run.
    e.root.position.set(mapPxW + 500, -500);
    e.update(0);

    expect(e.root.x).toBe(mapPxW);
    expect(e.root.y).toBe(0);
  });

  it("leaves an in-bounds position untouched", () => {
    const map = makeMap();
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, map, () => 0.999);
    const seatX = e.root.x;
    const seatY = e.root.y;
    e.update(0);
    expect(e.root.x).toBe(seatX);
    expect(e.root.y).toBe(seatY);
  });
});

describe("CharacterEntity: destroy", () => {
  it("destroys the overlays and the root container (and its children)", () => {
    const e = new CharacterEntity("agent-1", makeTestCharacterAssets(), SEAT, makeMap(), () => 0.5);
    const sprite = spriteOf(e);
    e.destroy();
    expect(e.root.destroyed).toBe(true);
    expect(sprite.destroyed).toBe(true);
  });
});
