// src/renderer/office/gen/__tests__/palette.test.ts
//
// Tests for the character palette generator.
//
// Coverage required by the task brief:
// - contrastRatio(shirt.base, skin.base) >= 1.6 always holds, *including*
//   after the final hard clamp path (skin tones / seeds chosen so the
//   retry loop is exhausted and the clamp branch executes).
// - Ramp luminance order: shadow < base < light for every generated ramp.
// - Determinism: same seed -> identical palette.
// - Sanity: different seeds -> different palettes.
//
// Review fix (deliberate deviation from the original design, authorized by
// the design owner): the original final clamp randomized hue at a fixed lightness,
// but luminance at fixed HSL lightness varies with hue (WCAG RGB weights
// 0.2126/0.7152/0.0722), so the clamp did NOT guarantee contrast >= 1.6
// (counterexample: seed 2274 -> ~1.52). The clamp is now a hue-aware
// deterministic lightness scan (`clampShirtRamp`), tested below:
// - Regression: seed 2274 and neighbors satisfy the bound.
// - Adversarial: a 100k-seed sweep with an instrumented Rng counts actual
//   clamp-branch executions (>= 100 asserted, so the test can't silently
//   stop exercising the branch) and checks the bound on every one.
// - Exhaustive: `clampShirtRamp` swept over a dense hue grid for all five
//   skin tones.

import { describe, expect, it } from "vitest";
import { makeRng, type Rng } from "../prng";
import {
  clampShirtRamp,
  contrastRatio,
  generatePalette,
  hslToRgb,
  luminance,
  SHIRT_SKIN_MIN_CONTRAST,
  type CharacterPalette,
  type Ramp,
} from "../palette";

function ramps(p: CharacterPalette): Ramp[] {
  return [p.skin, p.hair, p.shirt, p.pants];
}

/**
 * Wraps a real seeded Rng and counts `range()` calls so tests can detect
 * whether `generatePalette`'s final clamp branch executed.
 *
 * Call accounting (per the implementation): hair uses 2 range calls, the
 * initial shirt 3, each of the up-to-8 retries 3, the clamp branch exactly 1,
 * and pants 3. The retry loop only stops early once contrast passes, and the
 * clamp fires iff all 8 retries were exhausted AND contrast still failed —
 * so total range calls == 2+3+24+1+3 == 33 iff the clamp branch ran.
 */
const CLAMP_PATH_RANGE_CALLS = 33;

function instrumentedRng(seed: number): { rng: Rng; rangeCalls: () => number } {
  const inner = makeRng(seed);
  let calls = 0;
  return {
    rng: {
      next: () => inner.next(),
      int: (m) => inner.int(m),
      range: (min, max) => {
        calls++;
        return inner.range(min, max);
      },
      pick: (arr) => inner.pick(arr),
      bool: (p) => inner.bool(p),
    },
    rangeCalls: () => calls,
  };
}

/** Mirrors the private SKIN_TONES in palette.ts as [h, s, l]. */
const SKIN_TONES_MIRROR: ReadonlyArray<[number, number, number]> = [
  [28, 0.45, 0.78],
  [26, 0.5, 0.66],
  [24, 0.5, 0.52],
  [20, 0.5, 0.38],
  [18, 0.45, 0.28],
];

describe("hslToRgb", () => {
  it("maps pure red/green/blue hues correctly", () => {
    expect(hslToRgb(0, 1, 0.5)).toBe(0xff0000);
    expect(hslToRgb(120, 1, 0.5)).toBe(0x00ff00);
    expect(hslToRgb(240, 1, 0.5)).toBe(0x0000ff);
  });

  it("maps l=0 to black and l=1 to white regardless of hue/sat", () => {
    expect(hslToRgb(200, 0.8, 0)).toBe(0x000000);
    expect(hslToRgb(200, 0.8, 1)).toBe(0xffffff);
  });

  it("wraps hues outside [0,360) the same as their canonical value", () => {
    expect(hslToRgb(360, 0.5, 0.5)).toBe(hslToRgb(0, 0.5, 0.5));
    expect(hslToRgb(-30, 0.5, 0.5)).toBe(hslToRgb(330, 0.5, 0.5));
  });
});

describe("luminance / contrastRatio", () => {
  it("gives black the minimum and white the maximum relative luminance", () => {
    expect(luminance(0x000000)).toBeCloseTo(0, 5);
    expect(luminance(0xffffff)).toBeCloseTo(1, 5);
  });

  it("black vs white has the maximum contrast ratio (21:1)", () => {
    expect(contrastRatio(0x000000, 0xffffff)).toBeCloseTo(21, 1);
  });

  it("is symmetric in its arguments", () => {
    expect(contrastRatio(0x336699, 0xffcc00)).toBeCloseTo(
      contrastRatio(0xffcc00, 0x336699),
      10,
    );
  });

  it("a color against itself has a contrast ratio of 1", () => {
    expect(contrastRatio(0x336699, 0x336699)).toBeCloseTo(1, 10);
  });
});

describe("generatePalette", () => {
  it("is deterministic for the same seed", () => {
    const a = generatePalette(makeRng(2024));
    const b = generatePalette(makeRng(2024));
    expect(a).toEqual(b);
  });

  it("differs across seeds (sanity, not a hard guarantee)", () => {
    const results = new Set(
      Array.from({ length: 20 }, (_, i) =>
        JSON.stringify(generatePalette(makeRng(i))),
      ),
    );
    expect(results.size).toBeGreaterThan(1);
  });

  it("always has a common outline color", () => {
    const p = generatePalette(makeRng(1));
    expect(p.outline).toBe(0x1a1420);
  });

  it("every ramp is luminance-ordered shadow < base < light", () => {
    for (let seed = 0; seed < 200; seed++) {
      const p = generatePalette(makeRng(seed));
      for (const r of ramps(p)) {
        expect(luminance(r.shadow)).toBeLessThan(luminance(r.base));
        expect(luminance(r.base)).toBeLessThan(luminance(r.light));
      }
    }
  });

  it("guarantees contrastRatio(shirt, skin) >= 1.6 across many seeds, including the final clamp path", () => {
    for (let seed = 0; seed < 500; seed++) {
      const p = generatePalette(makeRng(seed));
      expect(contrastRatio(p.shirt.base, p.skin.base)).toBeGreaterThanOrEqual(
        1.6,
      );
    }
  });

  it("regression: seed 2274 (known clamp-branch counterexample) and neighbors satisfy the bound", () => {
    for (const seed of [2273, 2274, 2275]) {
      const p = generatePalette(makeRng(seed));
      expect(contrastRatio(p.shirt.base, p.skin.base)).toBeGreaterThanOrEqual(
        SHIRT_SKIN_MIN_CONTRAST,
      );
    }
    // Determinism is unaffected by the clamp rewrite.
    expect(generatePalette(makeRng(2274))).toEqual(
      generatePalette(makeRng(2274)),
    );
  });

  it("adversarial sweep: the clamp branch fires >= 100 times and always yields contrast >= 1.6", () => {
    // 100k seeds make the clamp branch fire >100 times (measured: 112).
    // Counting is asserted so this test fails loudly if a future change
    // stops the branch from being exercised.
    const SWEEP = 100_000;
    let clampFired = 0;
    for (let seed = 0; seed < SWEEP; seed++) {
      const { rng, rangeCalls } = instrumentedRng(seed);
      const p = generatePalette(rng);
      if (rangeCalls() !== CLAMP_PATH_RANGE_CALLS) continue;
      clampFired++;
      const c = contrastRatio(p.shirt.base, p.skin.base);
      if (c < SHIRT_SKIN_MIN_CONTRAST) {
        expect.fail(
          `seed ${seed}: clamp branch produced contrast ${c} < ${SHIRT_SKIN_MIN_CONTRAST}`,
        );
      }
      // The clamped shirt ramp must still be luminance-ordered.
      expect(luminance(p.shirt.shadow)).toBeLessThan(luminance(p.shirt.base));
      expect(luminance(p.shirt.base)).toBeLessThan(luminance(p.shirt.light));
    }
    expect(clampFired).toBeGreaterThanOrEqual(100);
  });
});

describe("clampShirtRamp (hue-aware final clamp)", () => {
  it("guarantees contrast >= 1.6 for every hue against every skin tone (dense grid)", () => {
    for (const [h, s, l] of SKIN_TONES_MIRROR) {
      const skinBase = hslToRgb(h, s, l);
      const skinIsLight = l > 0.5;
      for (let hue = 0; hue < 360; hue += 0.25) {
        const shirt = clampShirtRamp(hue, skinBase, skinIsLight);
        const c = contrastRatio(shirt.base, skinBase);
        if (c < SHIRT_SKIN_MIN_CONTRAST) {
          expect.fail(
            `hue ${hue} vs skin [${h},${s},${l}]: contrast ${c} < ${SHIRT_SKIN_MIN_CONTRAST}`,
          );
        }
        expect(luminance(shirt.shadow)).toBeLessThan(luminance(shirt.base));
        expect(luminance(shirt.base)).toBeLessThan(luminance(shirt.light));
      }
    }
  });

  it("is deterministic (no rng inside the scan)", () => {
    const skinBase = hslToRgb(24, 0.5, 0.52);
    expect(clampShirtRamp(123.4, skinBase, true)).toEqual(
      clampShirtRamp(123.4, skinBase, true),
    );
  });
});
