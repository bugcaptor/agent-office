// src/renderer/office/gen/__tests__/helpers.ts
//
// Shared test-only canvas plumbing.
//
// `gen/` is pure and DOM-free; every canvas-producing function takes an
// injectable factory. Tests back that seam with `@napi-rs/canvas` (a fast,
// native, non-GPU implementation) instead of a real browser canvas or
// jsdom's unimplemented 2D context, so pixel output can be asserted against
// with plain `getImageData`.
import { createCanvas, type Canvas, type SKRSContext2D } from "@napi-rs/canvas";

import type { CanvasFactory } from "../compositor";
import type { PreviewCanvasFactory } from "../characterFactory";
import type { SpriteSheetResult } from "../compositor";

/** `CanvasFactory` seam (sprite sheet canvas) backed by @napi-rs/canvas. */
export const createTestCanvasFactory: () => CanvasFactory = () => (w, h) => {
  const canvas = createCanvas(w, h);
  const ctx = canvas.getContext("2d");
  return {
    ctx: ctx as unknown as CanvasRenderingContext2D,
    canvas: canvas as unknown as HTMLCanvasElement,
  };
};

/** `PreviewCanvasFactory` seam (enlarged preview canvas) backed by @napi-rs/canvas. */
export const createTestPreviewCanvasFactory: () => PreviewCanvasFactory = () => (w, h) => {
  const canvas: Canvas = createCanvas(w, h);
  const ctx: SKRSContext2D = canvas.getContext("2d");
  return {
    ctx: ctx as unknown as CanvasRenderingContext2D,
    canvas,
  };
};

/** Extracts the full RGBA pixel buffer of a composed sprite sheet as a plain array (for `toEqual`). */
export function sheetToPixels(sheet: SpriteSheetResult): number[] {
  const canvas = sheet.canvas as unknown as Canvas;
  const ctx = canvas.getContext("2d");
  const { data } = ctx.getImageData(0, 0, canvas.width, canvas.height);
  return Array.from(data);
}
