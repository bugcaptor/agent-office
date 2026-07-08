// src/renderer/office/entities/__tests__/ThinkingOverlay.test.ts
//
// TDD for the "..." thinking speech bubble, following
// ExclamationOverlay.test.ts's patterns: visibility toggle + dt-driven
// animation, asserted deterministically (no real timers / Math.random).
//
// Real `Container`/`Graphics` are used — they construct and mutate fine
// without a live Pixi `Application`/renderer (ExclamationOverlay's
// precedent).

import { describe, expect, it } from "vitest";
import type { Graphics } from "pixi.js";
import { ThinkingOverlay } from "../ThinkingOverlay";

// `bubble`/`dots` are private implementation details; root.children order is
// [bubble, dot0, dot1, dot2].
const dotY = (o: ThinkingOverlay, i: number): number => (o.root.children[i + 1] as Graphics).y;

describe("ThinkingOverlay: visibility", () => {
  it("setVisible(true) shows the root and setVisible(false) hides it", () => {
    const o = new ThinkingOverlay();
    o.setVisible(true);
    expect(o.root.visible).toBe(true);
    o.setVisible(false);
    expect(o.root.visible).toBe(false);
  });

  it("renders a bubble plus exactly three dots", () => {
    const o = new ThinkingOverlay();
    expect(o.root.children.length).toBe(4);
  });
});

describe("ThinkingOverlay: dot bounce animation", () => {
  it("resets phase to t=0 on setVisible(true), so an immediate update(0) yields dot 0's base position", () => {
    const o = new ThinkingOverlay();
    o.setVisible(true);
    o.update(0);
    // DOT_BASE_Y + sin(0) * amplitude == DOT_BASE_Y == -2
    expect(dotY(o, 0)).toBeCloseTo(-2);
  });

  it("staggers the three dots' phases so they are not all at the same y at a given time", () => {
    const o = new ThinkingOverlay();
    o.setVisible(true);
    o.update(150);
    const ys = [0, 1, 2].map((i) => dotY(o, i));
    expect(new Set(ys.map((y) => y.toFixed(4))).size).toBe(3);
  });

  it("does nothing while hidden (no phase advance, no y mutation)", () => {
    const o = new ThinkingOverlay();
    o.setVisible(true);
    o.update(150);
    const yBeforeHide = dotY(o, 0);
    o.setVisible(false);
    o.update(1000);
    // setVisible(false) itself doesn't move the dot; update() while hidden is a no-op too.
    expect(dotY(o, 0)).toBe(yBeforeHide);
  });
});

describe("ThinkingOverlay: destroy", () => {
  it("destroys the root container", () => {
    const o = new ThinkingOverlay();
    o.destroy();
    expect(o.root.destroyed).toBe(true);
  });
});
