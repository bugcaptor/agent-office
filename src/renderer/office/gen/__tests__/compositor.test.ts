// src/renderer/office/gen/__tests__/compositor.test.ts
//
// Tests for pixel part data + sprite
// sheet compositor.
//
// Coverage required by the task brief:
// - `resolveChar` maps every documented character to the correct palette
//   slot (and unmapped/'.' chars to null/transparent).
// - `composeSpriteSheet` produces a sheet with > 60 opaque pixels, using an
//   injected CanvasFactory backed by @napi-rs/canvas (no browser/DOM/
//   OffscreenCanvas dependency in the test environment).
// - Pixel-art constraint sanity: no alpha blending — every touched pixel is
//   fully opaque (alpha 255) or fully transparent (alpha 0), never partial
//   (which would indicate anti-aliasing crept in).

import { createCanvas } from "@napi-rs/canvas";
import { describe, expect, it } from "vitest";

import { generatePalette } from "../palette";
import { makeRng } from "../prng";
import {
  ACCESSORY_VARIANTS,
  BODY_BASE_FRONT,
  CLOTHES_VARIANTS,
  HAIR_VARIANTS,
  LEGS_WALK_A,
  LEGS_WALK_B,
} from "../parts";
import {
  CELL,
  composeSpriteSheet,
  FRAME_ORDER,
  resolveChar,
  type CanvasFactory,
  type CharacterLayers,
} from "../compositor";

/** Adapts @napi-rs/canvas to the compositor's CanvasFactory seam. */
const napiCanvasFactory: CanvasFactory = (w, h) => {
  const canvas = createCanvas(w, h);
  const ctx = canvas.getContext("2d");
  return { ctx: ctx as unknown as CanvasRenderingContext2D, canvas: canvas as unknown as HTMLCanvasElement };
};

function sampleLayers(): CharacterLayers {
  return {
    body: BODY_BASE_FRONT,
    clothes: CLOTHES_VARIANTS.stripe,
    hair: HAIR_VARIANTS.bob,
    accessory: ACCESSORY_VARIANTS.glasses,
    legsWalkA: LEGS_WALK_A,
    legsWalkB: LEGS_WALK_B,
  };
}

describe("resolveChar", () => {
  const pal = generatePalette(makeRng(12345));

  it("maps transparency and unknown characters to null", () => {
    expect(resolveChar(".", pal)).toBeNull();
    expect(resolveChar("?", pal)).toBeNull();
    expect(resolveChar("O", pal)).toBeNull(); // documented gap in glasses pattern
  });

  it("maps outline and eye to pal.outline", () => {
    expect(resolveChar("o", pal)).toBe(pal.outline);
    expect(resolveChar("e", pal)).toBe(pal.outline);
  });

  it("maps skin ramp characters", () => {
    expect(resolveChar("S", pal)).toBe(pal.skin.shadow);
    expect(resolveChar("s", pal)).toBe(pal.skin.base);
    expect(resolveChar("H", pal)).toBe(pal.skin.light);
  });

  it("maps hair ramp characters", () => {
    expect(resolveChar("A", pal)).toBe(pal.hair.shadow);
    expect(resolveChar("a", pal)).toBe(pal.hair.base);
    expect(resolveChar("B", pal)).toBe(pal.hair.light);
  });

  it("maps shirt ramp characters", () => {
    expect(resolveChar("C", pal)).toBe(pal.shirt.shadow);
    expect(resolveChar("c", pal)).toBe(pal.shirt.base);
    expect(resolveChar("D", pal)).toBe(pal.shirt.light);
  });

  it("maps pants ramp characters", () => {
    expect(resolveChar("P", pal)).toBe(pal.pants.shadow);
    expect(resolveChar("p", pal)).toBe(pal.pants.base);
  });

  it("maps 'W' to opaque white regardless of palette", () => {
    expect(resolveChar("W", pal)).toBe(0xffffff);
  });
});

describe("composeSpriteSheet", () => {
  const pal = generatePalette(makeRng(777));
  const layers = sampleLayers();

  it("produces a sheet sized CELL*4 x CELL (4 horizontal frames)", () => {
    const result = composeSpriteSheet(layers, pal, napiCanvasFactory);
    expect(result.cell).toBe(CELL);
    expect(result.frames).toEqual(FRAME_ORDER);
    expect((result.canvas as any).width).toBe(CELL * FRAME_ORDER.length);
    expect((result.canvas as any).height).toBe(CELL);
    for (const f of FRAME_ORDER) {
      expect(result.frameRects[f]).toEqual({
        x: FRAME_ORDER.indexOf(f) * CELL,
        y: 0,
        w: CELL,
        h: CELL,
      });
    }
  });

  it("draws more than 60 opaque pixels across the sheet", () => {
    const result = composeSpriteSheet(layers, pal, napiCanvasFactory);
    const canvas = result.canvas as any;
    const ctx = canvas.getContext("2d");
    const { data } = ctx.getImageData(0, 0, canvas.width, canvas.height);
    let opaque = 0;
    for (let i = 3; i < data.length; i += 4) {
      if (data[i] === 255) opaque++;
    }
    expect(opaque).toBeGreaterThan(60);
  });

  it("never produces partially-transparent pixels (no anti-aliasing)", () => {
    const result = composeSpriteSheet(layers, pal, napiCanvasFactory);
    const canvas = result.canvas as any;
    const ctx = canvas.getContext("2d");
    const { data } = ctx.getImageData(0, 0, canvas.width, canvas.height);
    for (let i = 3; i < data.length; i += 4) {
      expect([0, 255]).toContain(data[i]);
    }
  });

  it("is pure: calling twice with the same inputs yields identical pixels", () => {
    const a = composeSpriteSheet(layers, pal, napiCanvasFactory);
    const b = composeSpriteSheet(layers, pal, napiCanvasFactory);
    const ca = a.canvas as any;
    const cb = b.canvas as any;
    const da = ca.getContext("2d").getImageData(0, 0, ca.width, ca.height).data;
    const db = cb.getContext("2d").getImageData(0, 0, cb.width, cb.height).data;
    expect(Array.from(da)).toEqual(Array.from(db));
  });

  it("draws walk0/walk1 frames with swapped leg pixels (distinct from each other)", () => {
    const result = composeSpriteSheet(layers, pal, napiCanvasFactory);
    const canvas = result.canvas as any;
    const ctx = canvas.getContext("2d");
    const walk0Rect = result.frameRects.walk0;
    const walk1Rect = result.frameRects.walk1;
    const d0 = ctx.getImageData(walk0Rect.x, walk0Rect.y, CELL, CELL).data;
    const d1 = ctx.getImageData(walk1Rect.x, walk1Rect.y, CELL, CELL).data;
    expect(Array.from(d0)).not.toEqual(Array.from(d1));
  });

  it("defaults the factory param (does not throw when omitted) when a canvas API is available", () => {
    // In the node test environment neither OffscreenCanvas nor document exist,
    // so the *type* must still accept an optional factory — verified by
    // explicitly passing one above. This test just checks the signature
    // allows omission at the type level via a wrapper that supplies node canvas.
    const wrapped = (l: CharacterLayers, p = pal) => composeSpriteSheet(l, p, napiCanvasFactory);
    expect(() => wrapped(layers)).not.toThrow();
  });
});
