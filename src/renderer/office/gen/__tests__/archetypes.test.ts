import { describe, expect, it } from "vitest";
import { makeRng, hashStringToSeed } from "../prng";
import { generatePalette, contrastRatio, luminance } from "../palette";
import {
  ARCHETYPE_IDS,
  ARCHETYPE_SELECT_OPTIONS,
  getArchetype,
  pickArchetype,
  resolveArchetype,
  ARCHETYPES,
} from "../archetypes";
import { generateSheet } from "../characterFactory";
import { createTestCanvasFactory, sheetToPixels } from "./helpers";

describe("archetype registry", () => {
  it("exposes human and a stable 8-id ordering", () => {
    expect(ARCHETYPE_IDS).toEqual([
      "human", "elf", "orc", "beastfolk", "robot", "android", "slime", "ghost",
    ]);
    expect(ARCHETYPES.human).toBeDefined();
    expect(ARCHETYPES.human.label).toBe("인간");
  });

  it("select options are 자동(시드) + the 8 archetypes", () => {
    expect(ARCHETYPE_SELECT_OPTIONS[0]).toEqual({ value: "auto", label: "자동(시드)" });
    expect(ARCHETYPE_SELECT_OPTIONS).toHaveLength(9);
  });

  it("getArchetype falls back to human for unknown/undefined ids", () => {
    expect(getArchetype(undefined).id).toBe("human");
    expect(getArchetype("nope").id).toBe("human");
    expect(getArchetype("human").id).toBe("human");
  });
});

describe("pickArchetype (seed draw, separate rng stream)", () => {
  it("is deterministic per seed and uses the ':archetype' salted stream", () => {
    for (const seed of ["a", "seed-1", "agent-42", "zzz"]) {
      const expected = makeRng(hashStringToSeed(seed + ":archetype")).pick(
        ARCHETYPE_IDS as readonly string[],
      );
      expect(pickArchetype(seed)).toBe(expected);
      expect(pickArchetype(seed)).toBe(pickArchetype(seed));
      expect(ARCHETYPE_IDS as readonly string[]).toContain(pickArchetype(seed));
    }
  });

  it("does not pollute the palette rng stream (independent hash)", () => {
    // 같은 seed의 팔레트는 pickArchetype 존재와 무관하게 기존과 동일해야 한다.
    const seed = "agent-42";
    const before = generatePalette(makeRng(hashStringToSeed(seed)));
    pickArchetype(seed);
    const after = generatePalette(makeRng(hashStringToSeed(seed)));
    expect(after).toEqual(before);
    // archetype 스트림은 팔레트 스트림과 다른 시드를 쓴다.
    expect(hashStringToSeed(seed + ":archetype")).not.toBe(hashStringToSeed(seed));
  });

  it("across many seeds yields more than one archetype (distribution sanity)", () => {
    const seen = new Set(Array.from({ length: 200 }, (_, i) => pickArchetype(`s${i}`)));
    expect(seen.size).toBeGreaterThan(1);
  });
});

describe("resolveArchetype", () => {
  it("'auto' -> seed draw; known -> itself; undefined/unknown -> human", () => {
    expect(resolveArchetype("auto", "s")).toBe(pickArchetype("s"));
    expect(resolveArchetype("human", "s")).toBe("human");
    expect(resolveArchetype(undefined, "s")).toBe("human");
    expect(resolveArchetype("dragon-that-does-not-exist", "s")).toBe("human");
  });
});

describe("human archetype byte-compat surface", () => {
  it("human.generatePalette is the existing generatePalette (same output)", () => {
    const rngA = makeRng(hashStringToSeed("x"));
    const rngB = makeRng(hashStringToSeed("x"));
    expect(ARCHETYPES.human.generatePalette(rngA)).toEqual(generatePalette(rngB));
  });
});

describe("humanoid archetypes (elf/orc/beastfolk/android)", () => {
  const ids = ["elf", "orc", "beastfolk", "android"] as const;

  it("are registered with 한글 labels and are humanoid in prompts", () => {
    const labels: Record<string, string> = {
      elf: "엘프", orc: "오크", beastfolk: "수인", android: "안드로이드",
    };
    for (const id of ids) {
      expect(ARCHETYPES[id]).toBeDefined();
      expect(ARCHETYPES[id].label).toBe(labels[id]);
      const rng = makeRng(hashStringToSeed("p"));
      expect(ARCHETYPES[id].promptDescriptor(ARCHETYPES[id].generatePalette(rng)).humanoid).toBe(true);
    }
  });

  it("produce a non-empty 64x16 sheet distinct from human for the same seed", () => {
    for (const id of ids) {
      const f = createTestCanvasFactory();
      const human = sheetToPixels(generateSheet("seed-cmp", createTestCanvasFactory(), "human").sheet);
      const px = sheetToPixels(generateSheet("seed-cmp", f, id).sheet);
      let opaque = 0;
      for (let i = 3; i < px.length; i += 4) if (px[i] > 0) opaque++;
      expect(opaque).toBeGreaterThan(60);
      expect(px).not.toEqual(human); // 오버레이/팔레트로 사람과 구별
    }
  });

  it("palettes keep shirt/skin contrast >= 1.6 and luminance-ordered ramps across many seeds", () => {
    for (const id of ids) {
      for (let s = 0; s < 120; s++) {
        const pal = ARCHETYPES[id].generatePalette(makeRng(s));
        expect(contrastRatio(pal.shirt.base, pal.skin.base)).toBeGreaterThanOrEqual(1.6);
        for (const r of [pal.skin, pal.hair, pal.shirt, pal.pants]) {
          expect(luminance(r.shadow)).toBeLessThan(luminance(r.base));
          expect(luminance(r.base)).toBeLessThan(luminance(r.light));
        }
      }
    }
  });

  it("are deterministic per seed", () => {
    for (const id of ids) {
      const a = sheetToPixels(generateSheet("d", createTestCanvasFactory(), id).sheet);
      const b = sheetToPixels(generateSheet("d", createTestCanvasFactory(), id).sheet);
      expect(a).toEqual(b);
    }
  });
});

describe("non-humanoid archetypes (robot/slime/ghost)", () => {
  const ids = ["robot", "slime", "ghost"] as const;

  it("are registered with 한글 labels and are non-humanoid in prompts", () => {
    const labels: Record<string, string> = { robot: "로봇", slime: "슬라임", ghost: "유령" };
    for (const id of ids) {
      expect(ARCHETYPES[id]).toBeDefined();
      expect(ARCHETYPES[id].label).toBe(labels[id]);
      const pal = ARCHETYPES[id].generatePalette(makeRng(1));
      expect(ARCHETYPES[id].promptDescriptor(pal).humanoid).toBe(false);
    }
  });

  it("build a non-empty sheet whose 4 frames are not all identical", () => {
    for (const id of ids) {
      const f = createTestCanvasFactory();
      const res = generateSheet(`nh-${id}`, f, id).sheet;
      const canvas = res.canvas as any;
      const ctx = canvas.getContext("2d");
      const frame = (x: number) => Array.from(ctx.getImageData(x, 0, 16, 16).data);
      let opaque = 0;
      const px = sheetToPixels(res);
      for (let i = 3; i < px.length; i += 4) if (px[i] > 0) opaque++;
      expect(opaque).toBeGreaterThan(60);
      // idle0 != idle1 (slime squash / ghost float) 또는 walk0 != walk1
      const distinct = new Set([frame(0), frame(16), frame(32), frame(48)].map((a) => a.join(",")));
      expect(distinct.size).toBeGreaterThan(1);
    }
  });

  it("robot accent stays readable vs chassis; ramps luminance-ordered", () => {
    for (let s = 0; s < 80; s++) {
      const pal = ARCHETYPES.robot.generatePalette(makeRng(s));
      expect(contrastRatio(pal.shirt.base, pal.skin.base)).toBeGreaterThanOrEqual(1.6);
      for (const r of [pal.skin, pal.shirt, pal.pants]) {
        expect(luminance(r.shadow)).toBeLessThan(luminance(r.base));
        expect(luminance(r.base)).toBeLessThan(luminance(r.light));
      }
    }
  });

  it("slime/ghost ramps stay luminance-ordered (sanity on many seeds)", () => {
    for (let s = 0; s < 60; s++) {
      for (const id of ["slime", "ghost"] as const) {
        const pal = ARCHETYPES[id].generatePalette(makeRng(s));
        expect(luminance(pal.skin.shadow)).toBeLessThan(luminance(pal.skin.light));
      }
    }
  });

  it("are deterministic per seed", () => {
    for (const id of ids) {
      const a = sheetToPixels(generateSheet("nd", createTestCanvasFactory(), id).sheet);
      const b = sheetToPixels(generateSheet("nd", createTestCanvasFactory(), id).sheet);
      expect(a).toEqual(b);
    }
  });
});
