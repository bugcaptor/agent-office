// src/renderer/portrait/__tests__/retroFilter.test.ts
import { describe, expect, it } from "vitest";
import {
  posterize,
  posterizeRgba,
  RETRO_LEVELS,
  RETRO_DOWNSCALE,
} from "../retroFilter";

describe("posterize", () => {
  it("keeps the extremes exact", () => {
    expect(posterize(0, 6)).toBe(0);
    expect(posterize(255, 6)).toBe(255);
  });

  it("snaps to the nearest of `levels` evenly spaced steps", () => {
    // 2단계 -> {0, 255}. 128은 255쪽으로 반올림, 120은 0쪽.
    expect(posterize(128, 2)).toBe(255);
    expect(posterize(120, 2)).toBe(0);
  });

  it("reduces the number of distinct output values", () => {
    const distinct = new Set(
      Array.from({ length: 256 }, (_, v) => posterize(v, RETRO_LEVELS))
    );
    expect(distinct.size).toBe(RETRO_LEVELS);
  });

  it("collapses to 0 when levels <= 1", () => {
    expect(posterize(200, 1)).toBe(0);
  });
});

describe("posterizeRgba", () => {
  it("posterizes RGB and preserves alpha", () => {
    const data = [10, 128, 250, 42, 0, 255, 130, 200];
    const out = posterizeRgba(data, 2);
    expect(out[0]).toBe(posterize(10, 2));
    expect(out[1]).toBe(posterize(128, 2));
    expect(out[2]).toBe(posterize(250, 2));
    expect(out[3]).toBe(42); // alpha untouched
    expect(out[7]).toBe(200); // second pixel alpha untouched
  });
});

describe("retro constants", () => {
  it("downscales to a quarter-res 3:4 grid", () => {
    expect(RETRO_DOWNSCALE).toEqual({ w: 60, h: 80 });
  });
});
