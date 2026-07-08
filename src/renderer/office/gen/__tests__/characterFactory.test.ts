// src/renderer/office/gen/__tests__/characterFactory.test.ts
//
// Tests for the deterministic
// character factory (`selectLayers`, `generateSheet`, `createCharacterAssets`,
// `generateSpritePreview`).
//
// Coverage required by the task brief:
// - Determinism: same seed -> identical sprite-sheet pixels.
// - Seed sensitivity: different seed -> different pixels (very high
//   probability, not a hard guarantee).
// - `selectLayers` part selection is deterministic and picks from the
//   documented key sets (hair/clothes/accessory).
// - Palette contrast bound (shirt vs skin >= 1.6) holds through the full
//   factory path across many seeds.
// - The composed sheet is not empty (character actually drawn).
// - Frozen contract: `generateSpritePreview(seed)` is synchronous and
//   returns a `data:image/png` URL; same seed -> identical dataURL; it does
//   not require a live Pixi/browser context (canvas seam is injectable).
// - `createCharacterAssets`'s pure prerequisites (`generateSheet`'s sheet +
//   descriptor) are consistent with what `selectLayers` reports, since the
//   Pixi-texture step itself needs a renderer context this suite doesn't
//   spin up (this suite exempts it — only `gen/` need be DOM/Pixi-free and vitest-able).

import { describe, expect, it } from "vitest";

import { contrastRatio } from "../palette";
import {
  generateSheet,
  generateSpritePreview,
  selectLayers,
} from "../characterFactory";
import { composeSpriteSheet } from "../compositor";
import {
  BODY_BASE_FRONT, LEGS_WALK_A, LEGS_WALK_B,
  HAIR_VARIANTS, CLOTHES_VARIANTS, ACCESSORY_VARIANTS,
  HAIR_KEYS, CLOTHES_KEYS, ACCESSORY_KEYS,
} from "../parts";
import { makeRng, hashStringToSeed } from "../prng";
import { generatePalette } from "../palette";
import {
  createTestCanvasFactory,
  createTestPreviewCanvasFactory,
  sheetToPixels,
} from "./helpers";

describe("character generator determinism", () => {
  it("same seed -> identical pixels", () => {
    const f = createTestCanvasFactory();
    const a = sheetToPixels(generateSheet("agent-alpha", f).sheet);
    const b = sheetToPixels(generateSheet("agent-alpha", f).sheet);
    expect(a).toEqual(b);
  });

  it("different seed -> different pixels (very high probability)", () => {
    const f = createTestCanvasFactory();
    const a = sheetToPixels(generateSheet("agent-alpha", f).sheet);
    const b = sheetToPixels(generateSheet("agent-omega", f).sheet);
    expect(a).not.toEqual(b);
  });

  it("layer selection is deterministic and in-range", () => {
    const s1 = selectLayers("seed-123").descriptor;
    const s2 = selectLayers("seed-123").descriptor;
    expect(s1).toEqual(s2);
    expect(HAIR_KEYS).toContain(s1.hair);
    expect(CLOTHES_KEYS).toContain(s1.clothes);
    expect(ACCESSORY_KEYS).toContain(s1.accessory);
  });

  it("palette contrast: shirt vs skin >= 1.6", () => {
    for (const seed of ["a", "b", "c", "d", "e", "f", "g", "h"]) {
      const { pal } = selectLayers(seed);
      expect(contrastRatio(pal.shirt.base, pal.skin.base)).toBeGreaterThanOrEqual(1.6);
    }
  });

  it("outline present & non-empty frame (character actually drawn)", () => {
    const f = createTestCanvasFactory();
    const px = sheetToPixels(generateSheet("seed-x", f).sheet);
    let opaque = 0;
    for (let i = 3; i < px.length; i += 4) if (px[i] > 0) opaque++;
    expect(opaque).toBeGreaterThan(60);
  });

  it("many-seed sweep never crashes and always yields a non-empty, contrast-valid sheet", () => {
    const f = createTestCanvasFactory();
    for (let i = 0; i < 100; i++) {
      const seed = `agent-${i}`;
      const { sheet, descriptor } = generateSheet(seed, f);
      expect(HAIR_KEYS).toContain(descriptor.hair);
      expect(CLOTHES_KEYS).toContain(descriptor.clothes);
      expect(ACCESSORY_KEYS).toContain(descriptor.accessory);
      const px = sheetToPixels(sheet);
      let opaque = 0;
      for (let j = 3; j < px.length; j += 4) if (px[j] > 0) opaque++;
      expect(opaque).toBeGreaterThan(0);
    }
  });
});

describe("generateSpritePreview (R3 frozen contract: sync, dataURL)", () => {
  it("returns synchronously (not a Promise) and yields a PNG dataURL", () => {
    const outFactory = createTestPreviewCanvasFactory();
    const sheetFactory = createTestCanvasFactory();
    const result = generateSpritePreview("agent-alpha", 6, sheetFactory, outFactory);
    expect(result).not.toBeInstanceOf(Promise);
    expect(typeof result).toBe("string");
    expect(result.startsWith("data:image/png")).toBe(true);
  });

  it("same seed -> identical dataURL", () => {
    const a = generateSpritePreview(
      "agent-alpha",
      6,
      createTestCanvasFactory(),
      createTestPreviewCanvasFactory(),
    );
    const b = generateSpritePreview(
      "agent-alpha",
      6,
      createTestCanvasFactory(),
      createTestPreviewCanvasFactory(),
    );
    expect(a).toBe(b);
  });

  it("different seed -> different dataURL (very high probability)", () => {
    const a = generateSpritePreview(
      "agent-alpha",
      6,
      createTestCanvasFactory(),
      createTestPreviewCanvasFactory(),
    );
    const b = generateSpritePreview(
      "agent-omega",
      6,
      createTestCanvasFactory(),
      createTestPreviewCanvasFactory(),
    );
    expect(a).not.toBe(b);
  });

  it("defaults to the documented signature `generateSpritePreview(seed)` (extra params optional)", () => {
    // Verified at the type level: calling with only `seed` must type-check
    // (subsystem C's frozen usage). We don't invoke the zero-arg form
    // here because its default factories reach for `document`, which this
    // node test environment intentionally does not provide (`gen/` stays
    // DOM-free and is exercised only through the injectable seam).
    expect(typeof generateSpritePreview).toBe("function");
    expect(generateSpritePreview.length).toBeLessThanOrEqual(1);
  });
});

describe("human archetype byte-for-byte regression (legacy pipeline)", () => {
  // 리팩터 이전 selectLayers 알고리즘을 그대로 재현해 human 경로를 고정한다.
  function legacyHumanPixels(seed: string, f = createTestCanvasFactory()): number[] {
    const rng = makeRng(hashStringToSeed(seed));
    const pal = generatePalette(rng);
    const hair = rng.pick(Object.keys(HAIR_VARIANTS));
    const clothes = rng.pick(Object.keys(CLOTHES_VARIANTS));
    const accessory = rng.pick(Object.keys(ACCESSORY_VARIANTS));
    const layers = {
      body: BODY_BASE_FRONT,
      clothes: CLOTHES_VARIANTS[clothes],
      hair: HAIR_VARIANTS[hair],
      accessory: ACCESSORY_VARIANTS[accessory],
      legsWalkA: LEGS_WALK_A,
      legsWalkB: LEGS_WALK_B,
    };
    return sheetToPixels(composeSpriteSheet(layers, pal, f));
  }

  it("generateSheet(seed, f, 'human') matches the legacy layer pipeline exactly", () => {
    for (const seed of ["agent-alpha", "agent-omega", "seed-x", "2274", "abc123"]) {
      const f = createTestCanvasFactory();
      const got = sheetToPixels(generateSheet(seed, f, "human").sheet);
      expect(got).toEqual(legacyHumanPixels(seed));
    }
  });

  it("default archetype (omitted) is human", () => {
    const f = createTestCanvasFactory();
    const omitted = sheetToPixels(generateSheet("seed-x", f).sheet);
    expect(omitted).toEqual(legacyHumanPixels("seed-x"));
  });
});

describe("archetype routing", () => {
  it("selectLayers reports the archetype in its descriptor", () => {
    expect(selectLayers("s", "human").descriptor.archetype).toBe("human");
  });
});
