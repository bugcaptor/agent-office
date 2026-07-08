// src/renderer/office/entities/__tests__/ExclamationOverlay.test.ts
//
// Tests for the pending-notification
// exclamation badge — visibility toggle + sine-wave bounce.
//
// Real `Container`/`Graphics` are used (they construct and mutate fine
// without a live Pixi `Application`/renderer, per 3E's precedent — only
// `Application.init()` needs a real canvas context). Animation time is
// entirely driven by the `dt` argument to `update()`, never a real timer,
// so the bounce phase is asserted deterministically.

import { describe, expect, it } from "vitest";
import type { Graphics } from "pixi.js";
import { ExclamationOverlay } from "../ExclamationOverlay";

// `mark` (the Graphics badge) is a private implementation detail; its
// bounce offset is observed indirectly through the one child of `root`.
const markY = (o: ExclamationOverlay): number => (o.root.children[0] as Graphics).y;

describe("ExclamationOverlay: visibility", () => {
  it("setVisible(true) shows the root and setVisible(false) hides it", () => {
    const o = new ExclamationOverlay();
    o.setVisible(true);
    expect(o.root.visible).toBe(true);
    o.setVisible(false);
    expect(o.root.visible).toBe(false);
  });
});

describe("ExclamationOverlay: bounce animation", () => {
  it("resets phase to t=0 on setVisible(true), so an immediate update(0) yields the wave's starting offset", () => {
    const o = new ExclamationOverlay();
    o.setVisible(true);
    o.update(0);
    // sin(0) * 2 - 2 == -2
    expect(markY(o)).toBe(-2);
  });

  it("bounces the mark up and down over a full period (quarter-period samples)", () => {
    const o = new ExclamationOverlay();
    o.setVisible(true);
    o.update(150); // quarter period (600ms): sin(pi/2)*2-2 == 0
    expect(markY(o)).toBe(0);
    o.update(150); // half period: sin(pi)*2-2 == -2
    expect(markY(o)).toBe(-2);
    o.update(150); // three-quarter period: sin(3pi/2)*2-2 == -4
    expect(markY(o)).toBe(-4);
  });

  it("does nothing while hidden (no phase advance, no y mutation)", () => {
    const o = new ExclamationOverlay();
    o.setVisible(true);
    o.update(150);
    const yBeforeHide = markY(o);
    o.setVisible(false);
    o.update(1000);
    expect(markY(o)).toBe(yBeforeHide);
  });
});

describe("ExclamationOverlay: destroy", () => {
  it("destroys the root container", () => {
    const o = new ExclamationOverlay();
    o.destroy();
    expect(o.root.destroyed).toBe(true);
  });
});
